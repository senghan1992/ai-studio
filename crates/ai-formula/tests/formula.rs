//! The JavaScript suite from `packages/formula/test/formula.test.js`, ported.
//! Every assertion here was passing against the original implementation.

use indexmap::IndexMap;
use serde_json::json;

use ai_formula::evaluate::{
    dependencies, display_value, evaluate, parse_cell_input, recalc_sheet, Cell, Context, Names,
};
use ai_formula::numfmt::apply_num_fmt;
use ai_formula::parse::parse;
use ai_formula::refs::{
    adjust_refs, col_to_index, expand_range_str, index_to_col, parse_range, parse_ref,
    shift_formula, shift_ref, to_ref, Axis, CellRef,
};
use ai_formula::values::Value;

/* ------------------------------------------------------------------- refs */

#[test]
fn column_letters_round_trip() {
    assert_eq!(index_to_col(0), "A");
    assert_eq!(index_to_col(25), "Z");
    assert_eq!(index_to_col(26), "AA");
    assert_eq!(index_to_col(701), "ZZ");
    for i in [0usize, 5, 25, 26, 51, 52, 700, 701] {
        assert_eq!(col_to_index(&index_to_col(i)), Some(i), "index {i}");
    }
}

#[test]
fn cell_references_parse_and_print() {
    assert_eq!(parse_ref("B12"), Some(CellRef { col: 1, row: 11 }));
    assert_eq!(parse_ref("$C$3"), Some(CellRef { col: 2, row: 2 }));
    assert_eq!(parse_ref("B0"), None);
    assert_eq!(parse_ref("hello"), None);
    assert_eq!(to_ref(1, 11), "B12");
}

#[test]
fn ranges_normalise_regardless_of_corner_order() {
    let a = parse_range("C3:A1").unwrap();
    assert_eq!(a.start, CellRef { col: 0, row: 0 });
    assert_eq!(a.end, CellRef { col: 2, row: 2 });
    assert_eq!(expand_range_str("A1:B2"), ["A1", "B1", "A2", "B2"]);
}

#[test]
fn shift_ref_respects_absolute_anchors() {
    assert_eq!(shift_ref("A1", 1, 1), "B2");
    assert_eq!(shift_ref("$A1", 1, 1), "$A2");
    assert_eq!(shift_ref("A$1", 1, 1), "B$1");
    assert_eq!(shift_ref("A1", -1, 0), "#REF!");
}

/* -------------------------------------------------------------- evaluation */

/// The bare `{ getValue }` context the JS tests used.
struct Cells(IndexMap<String, Value>);

impl Context for Cells {
    fn get_value(&self, reference: &str) -> Value {
        self.0.get(reference).cloned().unwrap_or(Value::Blank)
    }
}

fn cells(pairs: &[(&str, Value)]) -> Cells {
    Cells(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn n(x: f64) -> Value {
    Value::Number(x)
}
fn s(x: &str) -> Value {
    Value::Text(x.to_string())
}

fn eval_in(formula: &str, ctx: &Cells) -> Value {
    evaluate(&parse(formula).expect("parses"), ctx)
}

fn eval_bare(formula: &str) -> Value {
    eval_in(formula, &cells(&[]))
}

#[test]
fn arithmetic_honours_precedence_and_associativity() {
    assert_eq!(eval_bare("=1+2*3"), n(7.0));
    assert_eq!(eval_bare("=(1+2)*3"), n(9.0));
    assert_eq!(eval_bare("=2^3^2"), n(512.0)); // right-associative
    assert_eq!(eval_bare("=-2^2"), n(4.0)); // unary binds tighter than ^ here
    assert_eq!(eval_bare("=10/4"), n(2.5));
    assert_eq!(eval_bare("=50%"), n(0.5));
    assert_eq!(eval_bare("=10+5%"), n(10.05));
}

#[test]
fn division_by_zero_yields_an_error_value() {
    let v = eval_bare("=1/0");
    assert_eq!(v.err_code(), Some("#DIV/0!"));
}

#[test]
fn string_concatenation_and_comparison() {
    assert_eq!(eval_bare("=\"ab\"&\"cd\""), s("abcd"));
    assert_eq!(eval_bare("=\"a\"&1"), s("a1"));
    assert_eq!(eval_bare("=1<2"), Value::Bool(true));
    assert_eq!(eval_bare("=2<>2"), Value::Bool(false));
    // text comparison is case-insensitive
    assert_eq!(eval_bare("=\"a\"=\"A\""), Value::Bool(true));
    // numbers sort before text
    assert_eq!(eval_bare("=1<\"a\""), Value::Bool(true));
}

#[test]
fn refs_and_ranges_resolve_through_the_context() {
    let c = cells(&[
        ("A1", n(1.0)),
        ("A2", n(2.0)),
        ("A3", n(3.0)),
        ("B1", s("x")),
    ]);
    assert_eq!(eval_in("=A1+A2", &c), n(3.0));
    assert_eq!(eval_in("=SUM(A1:A3)", &c), n(6.0));
    assert_eq!(eval_in("=AVERAGE(A1:A3)", &c), n(2.0));
    assert_eq!(eval_in("=COUNTA(A1:B1)", &c), n(2.0));
    assert_eq!(eval_in("=MAX(A1:A3)", &c), n(3.0));
}

#[test]
fn sum_ignores_text_count_counts_only_numbers() {
    let c = cells(&[("A1", n(1.0)), ("A2", s("hello")), ("A3", n(3.0))]);
    assert_eq!(eval_in("=SUM(A1:A3)", &c), n(4.0));
    assert_eq!(eval_in("=COUNT(A1:A3)", &c), n(2.0));
    assert_eq!(eval_in("=COUNTA(A1:A3)", &c), n(3.0));
}

#[test]
fn if_and_iferror() {
    assert_eq!(eval_bare("=IF(1>0,\"yes\",\"no\")"), s("yes"));
    assert_eq!(eval_bare("=IF(1<0,\"yes\",\"no\")"), s("no"));
    assert_eq!(eval_bare("=IFERROR(1/0,\"safe\")"), s("safe"));
    assert_eq!(eval_bare("=IFERROR(4/2,\"safe\")"), n(2.0));
}

#[test]
fn conditional_aggregation() {
    let c = cells(&[
        ("A1", s("seoul")),
        ("A2", s("busan")),
        ("A3", s("seoul")),
        ("B1", n(10.0)),
        ("B2", n(20.0)),
        ("B3", n(30.0)),
    ]);
    assert_eq!(eval_in("=SUMIF(A1:A3,\"seoul\",B1:B3)", &c), n(40.0));
    assert_eq!(eval_in("=COUNTIF(A1:A3,\"seoul\")", &c), n(2.0));
    assert_eq!(eval_in("=COUNTIF(B1:B3,\">15\")", &c), n(2.0));
    assert_eq!(eval_in("=AVERAGEIF(A1:A3,\"seoul\",B1:B3)", &c), n(20.0));
}

#[test]
fn lookup_functions() {
    let c = cells(&[
        ("A1", s("a")),
        ("B1", n(1.0)),
        ("A2", s("b")),
        ("B2", n(2.0)),
        ("A3", s("c")),
        ("B3", n(3.0)),
    ]);
    assert_eq!(eval_in("=VLOOKUP(\"b\",A1:B3,2,FALSE)", &c), n(2.0));
    assert_eq!(eval_in("=MATCH(\"c\",A1:A3,0)", &c), n(3.0));
    assert_eq!(eval_in("=INDEX(B1:B3,2)", &c), n(2.0));
    assert_eq!(eval_in("=INDEX(A1:B3,3,2)", &c), n(3.0));
    assert_eq!(
        eval_in("=VLOOKUP(\"zz\",A1:B3,2,FALSE)", &c).err_code(),
        Some("#N/A")
    );
}

#[test]
fn text_functions() {
    assert_eq!(eval_bare("=LEFT(\"hello\",2)"), s("he"));
    assert_eq!(eval_bare("=RIGHT(\"hello\",2)"), s("lo"));
    assert_eq!(eval_bare("=MID(\"hello\",2,3)"), s("ell"));
    assert_eq!(eval_bare("=LEN(\"한글\")"), n(2.0));
    assert_eq!(eval_bare("=UPPER(\"abc\")"), s("ABC"));
    assert_eq!(eval_bare("=TRIM(\"  a  b  \")"), s("a b"));
    assert_eq!(eval_bare("=SUBSTITUTE(\"a-b-c\",\"-\",\"+\")"), s("a+b+c"));
    assert_eq!(eval_bare("=TEXTJOIN(\", \",TRUE,\"a\",\"b\")"), s("a, b"));
}

#[test]
fn unknown_function_names_surface_as_name_error() {
    assert_eq!(eval_bare("=NOPE(1)").err_code(), Some("#NAME?"));
}

#[test]
fn sumproduct_multiplies_element_wise() {
    let c = cells(&[
        ("A1", n(2.0)),
        ("A2", n(3.0)),
        ("B1", n(4.0)),
        ("B2", n(5.0)),
    ]);
    assert_eq!(eval_in("=SUMPRODUCT(A1:A2,B1:B2)", &c), n(23.0));
}

/* ----------------------------------------------------------- sheet recalc */

fn sheet(pairs: &[(&str, Cell)]) -> IndexMap<String, Cell> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn formula_cell(f: &str) -> Cell {
    Cell {
        f: Some(f.to_string()),
        ..Cell::default()
    }
}

fn literal(v: serde_json::Value) -> Cell {
    Cell {
        v,
        ..Cell::default()
    }
}

fn no_names() -> Names {
    Names::new()
}

#[test]
fn recalc_resolves_formula_chains_in_any_declaration_order() {
    let out = recalc_sheet(
        &sheet(&[
            ("C1", formula_cell("=B1*2")),
            ("B1", formula_cell("=A1+1")),
            (
                "A1",
                Cell {
                    v: json!(5),
                    t: Some("n".into()),
                    ..Cell::default()
                },
            ),
        ]),
        &no_names(),
    );
    assert_eq!(out.cells["A1"].v, json!(5.0));
    assert_eq!(out.cells["B1"].v, json!(6.0));
    assert_eq!(out.cells["C1"].v, json!(12.0));
}

#[test]
fn recalc_reports_circular_references_instead_of_hanging() {
    let out = recalc_sheet(
        &sheet(&[("A1", formula_cell("=B1")), ("B1", formula_cell("=A1"))]),
        &no_names(),
    );
    assert_eq!(out.cells["A1"].t.as_deref(), Some("e"));
    assert_eq!(out.errors["A1"], "#CIRC!");
}

#[test]
fn recalc_propagates_errors_downstream() {
    let out = recalc_sheet(
        &sheet(&[("A1", formula_cell("=1/0")), ("B1", formula_cell("=A1+1"))]),
        &no_names(),
    );
    assert_eq!(out.cells["A1"].v, json!("#DIV/0!"));
    assert_eq!(out.cells["B1"].v, json!("#DIV/0!"));
}

#[test]
fn named_ranges_resolve_in_formulas() {
    let mut names = Names::new();
    names.insert("sales".into(), json!("A1:A3"));
    let out = recalc_sheet(
        &sheet(&[
            ("A1", literal(json!(1))),
            ("A2", literal(json!(2))),
            ("A3", literal(json!(3))),
            ("B1", formula_cell("=SUM(sales)")),
        ]),
        &names,
    );
    assert_eq!(out.cells["B1"].v, json!(6.0));
}

#[test]
fn a_formula_referencing_an_empty_cell_treats_it_as_zero() {
    let out = recalc_sheet(&sheet(&[("A1", formula_cell("=Z99+5"))]), &no_names());
    assert_eq!(out.cells["A1"].v, json!(5.0));
}

/* ------------------------------------------------------------ cell input */

#[test]
fn parse_cell_input_classifies_what_the_user_typed() {
    let c = parse_cell_input("42").unwrap();
    assert_eq!((c.v, c.t.as_deref(), c.fmt), (json!(42.0), Some("n"), None));

    let c = parse_cell_input("1,200").unwrap();
    assert_eq!(
        (c.v, c.t.as_deref(), c.fmt.as_deref()),
        (json!(1200.0), Some("n"), Some("#,##0"))
    );

    let c = parse_cell_input("-3.5").unwrap();
    assert_eq!((c.v, c.t.as_deref(), c.fmt), (json!(-3.5), Some("n"), None));

    let c = parse_cell_input("12%").unwrap();
    assert_eq!(
        (c.v, c.t.as_deref(), c.fmt.as_deref()),
        (json!(0.12), Some("n"), Some("0.0%"))
    );

    let c = parse_cell_input("TRUE").unwrap();
    assert_eq!((c.v, c.t.as_deref()), (json!(true), Some("b")));

    let c = parse_cell_input("=SUM(A1:A2)").unwrap();
    assert_eq!(
        (c.f.as_deref(), c.v, c.t.as_deref()),
        (Some("=SUM(A1:A2)"), json!(null), Some("n"))
    );

    let c = parse_cell_input("안녕하세요").unwrap();
    assert_eq!((c.v, c.t.as_deref()), (json!("안녕하세요"), Some("s")));

    assert!(parse_cell_input("   ").is_none());

    let won = parse_cell_input("₩1,500").unwrap();
    assert_eq!(won.v, json!(1500.0));
    assert_eq!(won.fmt.as_deref(), Some("₩#,##0"));

    let date = parse_cell_input("2026-09-01").unwrap();
    assert_eq!(date.t.as_deref(), Some("d"));
    assert_eq!(date.fmt.as_deref(), Some("yyyy-mm-dd"));
}

#[test]
fn number_formats_render_as_the_ui_promises() {
    assert_eq!(apply_num_fmt(&n(1234.5), "#,##0"), "1,235");
    assert_eq!(apply_num_fmt(&n(1234.5), "#,##0.00"), "1,234.50");
    assert_eq!(apply_num_fmt(&n(0.1234), "0%"), "12%");
    assert_eq!(apply_num_fmt(&n(0.1234), "0.0%"), "12.3%");
    assert_eq!(apply_num_fmt(&n(1500.0), "₩#,##0"), "₩1,500");
    assert_eq!(apply_num_fmt(&n(-42.0), "#,##0"), "-42");
    assert_eq!(apply_num_fmt(&Value::Blank, "#,##0"), "");
}

#[test]
fn display_value_shows_error_codes_not_raw_objects() {
    let e = Cell {
        v: json!("#DIV/0!"),
        t: Some("e".into()),
        ..Cell::default()
    };
    assert_eq!(display_value(&e), "#DIV/0!");
    let noisy = Cell {
        v: json!(0.30000000000000004),
        t: Some("n".into()),
        ..Cell::default()
    };
    assert_eq!(display_value(&noisy), "0.3");
    let b = Cell {
        v: json!(true),
        t: Some("b".into()),
        ..Cell::default()
    };
    assert_eq!(display_value(&b), "TRUE");
    assert_eq!(display_value(&Cell::default()), "");
}

#[test]
fn dependencies_lists_every_cell_a_formula_reads() {
    let mut deps = dependencies("=SUM(A1:A3)+B7");
    deps.sort();
    assert_eq!(deps, ["A1", "A2", "A3", "B7"]);
}

#[test]
fn malformed_formulas_do_not_throw() {
    let out = recalc_sheet(&sheet(&[("A1", formula_cell("=SUM("))]), &no_names());
    assert_eq!(out.cells["A1"].t.as_deref(), Some("e"));
}

/* ---------------------------------------------------- structural edits */

#[test]
fn adjust_refs_shifts_references_when_a_row_is_inserted() {
    assert_eq!(adjust_refs("=SUM(B2:B4)", Axis::Row, 1, 1), "=SUM(B3:B5)");
    // refs above the insertion are untouched
    assert_eq!(adjust_refs("=A1+B2", Axis::Row, 5, 1), "=A1+B2");
    // absolute refs still move
    assert_eq!(adjust_refs("=$B$2", Axis::Row, 0, 1), "=$B$3");
    assert_eq!(adjust_refs("=B2", Axis::Col, 0, 1), "=C2");
}

#[test]
fn adjust_refs_turns_deleted_references_into_ref_error() {
    assert_eq!(adjust_refs("=B3", Axis::Row, 2, -1), "=#REF!");
    assert_eq!(adjust_refs("=B5", Axis::Row, 2, -1), "=B4");
    assert_eq!(adjust_refs("=B1", Axis::Row, 2, -1), "=B1");
}

#[test]
fn adjust_refs_leaves_string_literals_alone() {
    assert_eq!(
        adjust_refs("=IF(A1>0,\"B2 is fine\",\"B2\")", Axis::Row, 0, 1),
        "=IF(A2>0,\"B2 is fine\",\"B2\")"
    );
}

#[test]
fn adjust_refs_ignores_literal_values() {
    assert_eq!(adjust_refs("hello", Axis::Row, 0, 1), "hello");
    assert_eq!(adjust_refs("=A1", Axis::Row, 0, 0), "=A1");
}

/* -------------------------------------------------- absolute references */

#[test]
fn absolute_references_resolve_to_the_same_cell_as_relative_ones() {
    let c = cells(&[("A1", n(5.0)), ("B2", n(10.0))]);
    assert_eq!(eval_in("=$A$1", &c), n(5.0));
    assert_eq!(eval_in("=$A1", &c), n(5.0));
    assert_eq!(eval_in("=A$1", &c), n(5.0));
    assert_eq!(eval_in("=$B$2/$A$1", &c), n(2.0));
    assert_eq!(eval_in("=SUM($A$1:$B$2)", &c), n(15.0));
}

#[test]
fn a_percentage_of_total_formula_anchored_with_dollar_computes_correctly() {
    // The shape every budget sheet uses: each row divided by an absolute total.
    let out = recalc_sheet(
        &sheet(&[
            ("A1", literal(json!(30))),
            ("A2", literal(json!(70))),
            ("A3", formula_cell("=SUM(A1:A2)")),
            ("B1", formula_cell("=A1/$A$3")),
            ("B2", formula_cell("=A2/$A$3")),
        ]),
        &no_names(),
    );
    assert_eq!(out.cells["A3"].v, json!(100.0));
    assert_eq!(out.cells["B1"].v, json!(0.3));
    assert_eq!(out.cells["B2"].v, json!(0.7));
    assert_ne!(out.cells["B1"].t.as_deref(), Some("e"));
}

/* --------------------------------------------------- relative ref shifting */

#[test]
fn shift_formula_translates_relative_references_and_keeps_anchors() {
    assert_eq!(shift_formula("=B2+C2", 0, 1), "=B3+C3");
    assert_eq!(shift_formula("=SUM(B2:B4)", 1, 0), "=SUM(C2:C4)");
    assert_eq!(shift_formula("=B2*$D$1", 0, 2), "=B4*$D$1");
    assert_eq!(shift_formula("=B$2+$C3", 0, 1), "=B$2+$C4");
    assert_eq!(shift_formula("=A1", 0, 0), "=A1");
    assert_eq!(shift_formula("42", 0, 1), "42");
}

#[test]
fn shift_formula_leaves_string_literals_alone() {
    assert_eq!(
        shift_formula("=IF(A1>0,\"A1 ok\",\"B2 bad\")", 0, 1),
        "=IF(A2>0,\"A1 ok\",\"B2 bad\")"
    );
}

#[test]
fn shift_formula_marks_references_pushed_off_the_sheet_as_ref_error() {
    assert_eq!(shift_formula("=A1", 0, -1), "=#REF!");
}
