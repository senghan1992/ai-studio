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

/* --------------------------------------------------------- cross-sheet refs */

#[test]
fn a_formula_reads_another_sheet() {
    // What a summary sheet is made of. Without this the whole formula fails to
    // parse — `!` is not an operator — and the cell shows an error.
    let source = sheet(&[("A1", literal(json!(100))), ("B1", literal(json!(200)))]);
    let summary = sheet(&[
        ("A1", formula_cell("=실적!A1+실적!B1")),
        ("A2", formula_cell("=SUM('실적'!A1:B1)")),
        ("A3", formula_cell("=요약!A1*2")),
    ]);
    let book = ai_formula::evaluate::book_of([("실적", &source)]);
    let out = ai_formula::evaluate::recalc_sheet_in(&summary, &no_names(), "요약", &book);

    assert_eq!(out.cells["A1"].v, json!(300.0));
    assert_eq!(out.cells["A2"].v, json!(300.0), "a cross-sheet range");
    assert_eq!(
        out.cells["A3"].v,
        json!(600.0),
        "a sheet naming itself resolves locally"
    );
    assert!(out.unresolved.is_empty());
}

#[test]
fn a_sheet_name_is_matched_without_case() {
    let source = sheet(&[("A1", literal(json!(7)))]);
    let book = ai_formula::evaluate::book_of([("Data", &source)]);
    let out = ai_formula::evaluate::recalc_sheet_in(
        &sheet(&[("A1", formula_cell("=data!A1"))]),
        &no_names(),
        "요약",
        &book,
    );
    assert_eq!(out.cells["A1"].v, json!(7.0));
}

#[test]
fn a_reference_to_a_sheet_we_were_not_given_keeps_the_stored_value() {
    // The rule that makes opening someone else's workbook safe: a value we
    // cannot re-derive is left as the file stated it, rather than replaced with
    // `#REF!`.
    let mut cell = formula_cell("=다른시트!B4");
    cell.v = json!(1234);
    cell.t = Some("n".into());
    let out = recalc_sheet(&sheet(&[("A1", cell)]), &no_names());

    assert_eq!(out.cells["A1"].v, json!(1234));
    assert_eq!(out.cells["A1"].f.as_deref(), Some("=다른시트!B4"));
    assert_eq!(out.unresolved, ["A1"]);
    assert!(out.errors.is_empty(), "not reported as a formula error");
}

#[test]
fn a_function_this_build_lacks_keeps_the_stored_value() {
    let mut cell = formula_cell("=SOMEFUTUREFUNC(A2)");
    cell.v = json!(42);
    cell.t = Some("n".into());
    let out = recalc_sheet(&sheet(&[("A1", cell)]), &no_names());
    assert_eq!(out.cells["A1"].v, json!(42));
    assert_eq!(out.unresolved, ["A1"]);
}

#[test]
fn an_unknown_function_with_nothing_to_keep_still_reports_the_error() {
    // A formula the user just typed has no stored value to protect, and hiding
    // the error would leave them with a blank cell and no explanation.
    let out = recalc_sheet(&sheet(&[("A1", formula_cell("=NOPE(1)"))]), &no_names());
    assert_eq!(display_value(&out.cells["A1"]), "#NAME?");
}

#[test]
fn a_cross_sheet_reference_updates_when_the_other_sheet_changes() {
    let summary = sheet(&[("A1", formula_cell("=실적!A1*2"))]);
    for (input, expected) in [(10.0, 20.0), (25.0, 50.0)] {
        let source = sheet(&[("A1", literal(json!(input)))]);
        let book = ai_formula::evaluate::book_of([("실적", &source)]);
        let out = ai_formula::evaluate::recalc_sheet_in(&summary, &no_names(), "요약", &book);
        assert_eq!(out.cells["A1"].v, json!(expected));
    }
}

#[test]
fn a_sheet_qualified_reference_lexes_as_one_token() {
    use ai_formula::parse::{tokenize, Token};
    assert_eq!(
        tokenize("=Sheet2!A1").unwrap(),
        vec![
            Token::Op(ai_formula::parse::Op::Eq),
            Token::Ref("Sheet2!A1".into())
        ]
    );
    assert_eq!(
        tokenize("='2분기 실적'!$A$1:$B$4").unwrap(),
        vec![
            Token::Op(ai_formula::parse::Op::Eq),
            Token::Range("2분기 실적!$A$1:$B$4".into())
        ]
    );
    // A bare name is still a name, and a bare range still a range.
    assert_eq!(
        tokenize("=매출데이터").unwrap(),
        vec![
            Token::Op(ai_formula::parse::Op::Eq),
            Token::Name("매출데이터".into())
        ]
    );
}

/* ------------------------------------------------------- library completeness */

#[test]
fn every_registered_name_is_answerable() {
    // The formula bar offers whatever `FUNCTION_NAMES` lists, so a name that is
    // listed but not implemented is a promise the engine breaks. `#NAME?` is the
    // one answer no registered function may give.
    for name in ai_formula::functions::FUNCTION_NAMES.iter() {
        let value = ai_formula::functions::call(name, &[Value::Number(1.0), Value::Number(1.0)]);
        let value = value.unwrap_or_else(|| panic!("{name} is listed but not dispatched"));
        assert_ne!(
            value.err_code(),
            Some("#NAME?"),
            "{name} answered #NAME?, so it is listed but unimplemented"
        );
    }
}

#[test]
fn the_library_covers_the_functions_a_real_workbook_uses() {
    // Not a taste list: these are the names that appear in ordinary business
    // spreadsheets, and every one of them used to make a file open wrong.
    for name in [
        "VLOOKUP",
        "HLOOKUP",
        "XLOOKUP",
        "XMATCH",
        "INDEX",
        "MATCH",
        "CHOOSE",
        "LOOKUP",
        "OFFSET",
        "INDIRECT",
        "ROW",
        "COLUMN",
        "SUMIFS",
        "COUNTIFS",
        "AVERAGEIFS",
        "MAXIFS",
        "MINIFS",
        "SUBTOTAL",
        "AGGREGATE",
        "LARGE",
        "SMALL",
        "RANK",
        "PERCENTILE",
        "QUARTILE",
        "STDEV.S",
        "STDEV.P",
        "VAR.S",
        "VAR.P",
        "MEDIAN",
        "MODE",
        "CORREL",
        "SLOPE",
        "INTERCEPT",
        "FORECAST",
        "PMT",
        "FV",
        "PV",
        "NPER",
        "RATE",
        "NPV",
        "IRR",
        "IPMT",
        "PPMT",
        "SLN",
        "SYD",
        "DDB",
        "EDATE",
        "EOMONTH",
        "DATEDIF",
        "YEARFRAC",
        "NETWORKDAYS",
        "WORKDAY",
        "WEEKNUM",
        "TIME",
        "HOUR",
        "MINUTE",
        "SECOND",
        "SEARCH",
        "REPLACE",
        "REPT",
        "CHAR",
        "CODE",
        "EXACT",
        "TEXTBEFORE",
        "TEXTAFTER",
        "SWITCH",
        "XOR",
        "ISEVEN",
        "ISODD",
        "ISNA",
        "ISERR",
        "N",
        "TYPE",
        "LOG",
        "SIN",
        "COS",
        "TAN",
        "ATAN2",
        "DEGREES",
        "RADIANS",
        "GCD",
        "LCM",
        "FACT",
        "COMBIN",
        "MROUND",
        "EVEN",
        "ODD",
        "QUOTIENT",
        "RANDBETWEEN",
        "UNIQUE",
        "SORT",
        "FILTER",
        "SEQUENCE",
        "TRANSPOSE",
        "ADDRESS",
    ] {
        assert!(
            ai_formula::functions::is_function(name),
            "{name} is missing from the library"
        );
    }
}

/* ------------------------------------------------------------ the library */

/// A sheet shaped like a small report, for the table-driven checks below.
fn report() -> IndexMap<String, Cell> {
    let mut cells = IndexMap::new();
    let rows = [
        ("서울", 1200.0, 800.0),
        ("부산", 800.0, 500.0),
        ("서울", 1500.0, 900.0),
        ("대구", 600.0, 400.0),
        ("서울", 1800.0, 1100.0),
        ("부산", 900.0, 600.0),
    ];
    for (i, (region, revenue, cost)) in rows.iter().enumerate() {
        let row = i + 1;
        cells.insert(format!("A{row}"), literal(json!(region)));
        cells.insert(format!("B{row}"), literal(json!(revenue)));
        cells.insert(format!("C{row}"), literal(json!(cost)));
    }
    cells
}

fn value_of(formula: &str) -> Value {
    ai_formula::evaluate::evaluate_in(formula, &report(), &no_names())
}

fn text_of(formula: &str) -> String {
    ai_formula::values::to_text(&value_of(formula))
}

#[test]
fn the_multi_criteria_aggregates_match_excel() {
    for (formula, expected) in [
        ("=SUMIFS(B1:B6,A1:A6,\"서울\")", 4500.0),
        ("=SUMIFS(B1:B6,A1:A6,\"서울\",B1:B6,\">1200\")", 3300.0),
        ("=COUNTIFS(A1:A6,\"서울\")", 3.0),
        ("=COUNTIFS(A1:A6,\"서울\",B1:B6,\">1200\")", 2.0),
        ("=AVERAGEIFS(B1:B6,A1:A6,\"서울\")", 1500.0),
        ("=MAXIFS(B1:B6,A1:A6,\"부산\")", 900.0),
        ("=MINIFS(B1:B6,A1:A6,\"부산\")", 800.0),
        // The idiom that predates SUMIFS, and needs array-wide comparison.
        ("=SUMPRODUCT((A1:A6=\"서울\")*B1:B6)", 4500.0),
        ("=SUM(FILTER(B1:B6,A1:A6=\"서울\"))", 4500.0),
    ] {
        assert_eq!(value_of(formula), Value::Number(expected), "{formula}");
    }
}

#[test]
fn the_lookup_family_matches_excel() {
    for (formula, expected) in [
        ("=VLOOKUP(\"대구\",A1:C6,2,FALSE)", Value::Number(600.0)),
        ("=XLOOKUP(\"부산\",A1:A6,B1:B6)", Value::Number(800.0)),
        ("=XLOOKUP(\"광주\",A1:A6,B1:B6,-1)", Value::Number(-1.0)),
        ("=XMATCH(\"대구\",A1:A6)", Value::Number(4.0)),
        ("=CHOOSE(2,10,20,30)", Value::Number(20.0)),
        // LOOKUP assumes its range is sorted, so it is given a sorted one.
        (
            "=LOOKUP(1000,{600,800,900,1200,1500,1800})",
            Value::Number(900.0),
        ),
        (
            "=INDEX(B1:B6,MATCH(\"대구\",A1:A6,0))",
            Value::Number(600.0),
        ),
        // Reference functions: the address matters, not the value.
        ("=ROW(B4)", Value::Number(4.0)),
        ("=COLUMN(C1)", Value::Number(3.0)),
        ("=OFFSET(A1,3,1)", Value::Number(600.0)),
        ("=SUM(OFFSET(B1,0,0,6,1))", Value::Number(6800.0)),
        ("=INDIRECT(\"B3\")", Value::Number(1500.0)),
        ("=SUM(INDIRECT(\"B1:B6\"))", Value::Number(6800.0)),
        ("=INDEX(SORT(B1:B6),1)", Value::Number(600.0)),
        ("=COUNTA(UNIQUE(A1:A6))", Value::Number(3.0)),
        ("=SUM(SEQUENCE(5))", Value::Number(15.0)),
        ("=ADDRESS(3,2)", Value::Text("$B$3".into())),
        (
            "=SWITCH(2,1,\"하나\",2,\"둘\",\"기타\")",
            Value::Text("둘".into()),
        ),
    ] {
        assert_eq!(value_of(formula), expected, "{formula}");
    }
}

#[test]
fn the_statistics_match_excel() {
    for (formula, expected) in [
        ("=ROUND(STDEV.S(B1:B6),4)", 454.6061),
        ("=ROUND(STDEV.P(B1:B6),4)", 414.9967),
        ("=ROUND(VAR.S(B1:B6),4)", 206666.6667),
        ("=ROUND(VAR.P(B1:B6),4)", 172222.2222),
        ("=MEDIAN(B1:B6)", 1050.0),
        ("=LARGE(B1:B6,2)", 1500.0),
        ("=SMALL(B1:B6,2)", 800.0),
        ("=RANK(1500,B1:B6)", 2.0),
        ("=PERCENTILE(B1:B6,0.75)", 1425.0),
        ("=QUARTILE(B1:B6,3)", 1425.0),
        ("=SUBTOTAL(9,B1:B6)", 6800.0),
        ("=AGGREGATE(9,6,B1:B6)", 6800.0),
        ("=ROUND(CORREL(B1:B6,C1:C6),6)", 0.994521),
        ("=ROUND(SLOPE(B1:B6,C1:C6),6)", 1.712919),
        ("=MODE({1,2,2,3})", 2.0),
    ] {
        let Value::Number(got) = value_of(formula) else {
            panic!(
                "{formula} did not produce a number: {:?}",
                value_of(formula)
            );
        };
        assert!(
            (got - expected).abs() < 1e-4,
            "{formula}: {got} vs {expected}"
        );
    }
}

#[test]
fn the_loan_maths_matches_excel() {
    // A 300,000,000 won loan at 5% over 30 years, which is what these functions
    // exist for. The expected values are Excel's own, to four decimals.
    for (formula, expected) in [
        ("=ROUND(PMT(0.05/12,360,300000000),4)", -1610464.8690),
        ("=ROUND(FV(0.03/12,120,-500000),4)", 69870709.4382),
        ("=ROUND(NPER(0.05/12,-1610000,300000000),4)", 360.2409),
        ("=ROUND(RATE(360,-1610464.869,300000000)*12,6)", 0.05),
        ("=ROUND(IPMT(0.05/12,1,360,300000000),4)", -1250000.0),
        ("=ROUND(PPMT(0.05/12,1,360,300000000),4)", -360464.8690),
        ("=ROUND(NPV(0.1,{-1000,300,420,680}),6)", 118.844341),
        ("=ROUND(IRR({-1000,300,420,680}),6)", 0.163406),
        ("=SLN(10000,1000,5)", 1800.0),
        ("=SYD(10000,1000,5,2)", 2400.0),
        ("=DDB(10000,1000,5,1)", 4000.0),
    ] {
        let Value::Number(got) = value_of(formula) else {
            panic!("{formula} did not produce a number");
        };
        assert!(
            (got - expected).abs() < 1e-3,
            "{formula}: {got} vs {expected}"
        );
    }
}

#[test]
fn the_date_maths_matches_excel() {
    for (formula, expected) in [
        (
            "=TEXT(EDATE(DATE(2026,1,31),1),\"yyyy-mm-dd\")",
            "2026-02-28",
        ),
        (
            "=TEXT(EOMONTH(DATE(2026,2,5),0),\"yyyy-mm-dd\")",
            "2026-02-28",
        ),
        ("=DATEDIF(DATE(2020,3,15),DATE(2026,9,2),\"Y\")", "6"),
        ("=DATEDIF(DATE(2020,3,15),DATE(2026,9,2),\"M\")", "77"),
        ("=DATEDIF(DATE(2020,3,15),DATE(2026,9,2),\"MD\")", "18"),
        ("=NETWORKDAYS(DATE(2026,9,1),DATE(2026,9,30))", "22"),
        (
            "=TEXT(WORKDAY(DATE(2026,9,1),5),\"yyyy-mm-dd\")",
            "2026-09-08",
        ),
        ("=YEARFRAC(DATE(2026,1,1),DATE(2026,7,1))", "0.5"),
        ("=WEEKNUM(DATE(2026,9,2))", "36"),
        ("=ISOWEEKNUM(DATE(2026,9,2))", "36"),
        ("=HOUR(TIME(13,45,30))", "13"),
        ("=MINUTE(TIME(13,45,30))", "45"),
        ("=SECOND(TIME(13,45,30))", "30"),
        ("=DAYS360(DATE(2026,1,31),DATE(2026,3,31))", "60"),
    ] {
        assert_eq!(text_of(formula), expected, "{formula}");
    }
}

#[test]
fn the_text_and_logic_helpers_match_excel() {
    for (formula, expected) in [
        ("=SEARCH(\"서\",\"강남 서초\")", "4"),
        ("=SEARCH(\"A\",\"banana\")", "2"),
        ("=REPLACE(\"2026-09\",6,2,\"12\")", "2026-12"),
        ("=REPT(\"■\",3)", "■■■"),
        ("=TEXTBEFORE(\"서울-강남\",\"-\")", "서울"),
        ("=TEXTAFTER(\"서울-강남\",\"-\")", "강남"),
        ("=EXACT(\"a\",\"A\")", "FALSE"),
        ("=CHAR(65)", "A"),
        ("=CODE(\"A\")", "65"),
        ("=XOR(TRUE,FALSE,TRUE)", "FALSE"),
        ("=ISODD(7)", "TRUE"),
        ("=ISEVEN(7)", "FALSE"),
        ("=TYPE(\"x\")", "2"),
        ("=N(TRUE)", "1"),
        ("=MROUND(17,5)", "15"),
        ("=EVEN(3)", "4"),
        ("=ODD(4)", "5"),
        ("=GCD(24,36)", "12"),
        ("=LCM(4,6)", "12"),
        ("=COMBIN(10,3)", "120"),
        ("=FACT(5)", "120"),
        ("=QUOTIENT(17,5)", "3"),
        ("=ROUND(DEGREES(PI()),4)", "180"),
        ("=ROUND(ATAN2(1,1),6)", "0.785398"),
        ("=LOG(8,2)", "3"),
    ] {
        assert_eq!(text_of(formula), expected, "{formula}");
    }
}
