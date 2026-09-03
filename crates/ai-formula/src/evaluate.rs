//! Evaluating one formula, and recalculating a whole sheet.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::functions;
use crate::jsnum;
use crate::numfmt::{apply_num_fmt, ymd_to_serial};
use crate::parse::{collect_refs, parse, Node, Op};
use crate::refs::{bare_ref, expand_range, parse_range, parse_ref};
use crate::values::{
    compare_values, err, format_plain_number, num_of, to_text, RangeValue, Value, CIRC_ERR,
    NAME_ERR, NUM_ERR, REF_ERR, VALUE_ERR,
};

/// One stored cell.
///
/// `extra` keeps whatever the editor put there — border, fill, alignment,
/// merge span — so a recalculation never drops a cell's styling and the JSON
/// on disk round-trips unchanged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// The formula, including the leading `=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f: Option<String>,
    /// The value: `null`, a number, a string or a bool.
    #[serde(default)]
    pub v: Json,
    /// Type tag: `n`umber, `s`tring, `b`ool, `d`ate, `e`rror, `z` for empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
    /// Number format pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmt: Option<String>,
    /// On a dynamic-array anchor: the range its result spills over, itself
    /// included (`"E2:E4"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spill: Option<String>,
    /// On a spilled cell: the anchor whose formula produced this value. Such a
    /// cell is derived data — recalculation clears and rewrites it.
    #[serde(default, rename = "spillFrom", skip_serializing_if = "Option::is_none")]
    pub spill_from: Option<String>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

impl Cell {
    pub fn value(&self) -> Value {
        json_to_value(&self.v)
    }

    /// A formula cell is one whose `f` starts with `=`.
    pub fn formula(&self) -> Option<&str> {
        self.f
            .as_deref()
            .map(str::trim)
            .filter(|f| f.starts_with('='))
    }

    pub fn is_empty(&self) -> bool {
        self.f.is_none()
            && self.v.is_null()
            && self.fmt.is_none()
            && self.spill.is_none()
            && self.spill_from.is_none()
            && self.extra.is_empty()
    }

    /// True for a cell whose own content would block a spill: a formula or a
    /// value. Styling alone does not block — Excel spills into formatted empty
    /// cells and keeps the formatting.
    fn blocks_spill(&self) -> bool {
        self.f.is_some() || !self.v.is_null()
    }
}

pub fn json_to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Blank,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        Json::String(s) => Value::Text(s.clone()),
        // Arrays/objects never appear in a cell value; treat them as text.
        other => Value::Text(other.to_string()),
    }
}

pub fn value_to_json(v: &Value) -> Json {
    match v {
        Value::Blank => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => serde_json::Number::from_f64(*n)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Text(s) => Json::String(s.clone()),
        Value::Error(c) => Json::String(c.clone()),
        other => Json::String(to_text(other)),
    }
}

/// Named ranges: a name maps to a reference string, a range string, or a literal.
pub type Names = IndexMap<String, Json>;

/// The other sheets a formula can reach, by lower-cased name.
///
/// Borrowed rather than owned: recalculation reads the sibling sheets and never
/// writes to them.
pub type Book<'a> = HashMap<String, &'a IndexMap<String, Cell>>;

/// A workbook's sheets as a `Book`, keyed for case-insensitive lookup.
pub fn book_of<'a, I>(sheets: I) -> Book<'a>
where
    I: IntoIterator<Item = (&'a str, &'a IndexMap<String, Cell>)>,
{
    sheets
        .into_iter()
        .map(|(name, cells)| (name.to_lowercase(), cells))
        .collect()
}

/// What the evaluator needs from its surroundings.
pub trait Context {
    fn get_value(&self, reference: &str) -> Value;

    /// The address of the cell being evaluated, for `ROW()` and `COLUMN()` with
    /// no argument. `None` when the caller is evaluating a loose formula.
    fn current_ref(&self) -> Option<String> {
        None
    }

    /// Called when something in the formula could not be resolved: a sheet this
    /// context does not have, or a function this build does not implement.
    ///
    /// The point is not the error — that is already the returned value — but that
    /// the *caller* can decide not to overwrite a cached value it cannot beat.
    fn note_unresolved(&self) {}

    /// The range the formula at `anchor` spilled over, for `E2#`. `None` when
    /// the anchor does not spill — which Excel answers with `#REF!`.
    fn get_spill(&self, _anchor: &str) -> Option<String> {
        None
    }

    fn get_range(&self, range: &str) -> Value {
        let (sheet, local) = crate::refs::split_sheet(range);
        let Some(r) = parse_range(local) else {
            return err(REF_ERR);
        };
        let cells: Vec<(String, Value)> = expand_range(&r)
            .into_iter()
            .map(|reference| {
                let v = self.get_value(&crate::refs::with_sheet(sheet, &reference));
                (reference, v)
            })
            .collect();
        Value::Range(Rc::new(RangeValue {
            cells,
            rows: r.rows(),
            cols: r.cols(),
        }))
    }

    fn get_name(&self, _name: &str) -> Option<Json> {
        None
    }
}

/// A context over a plain map, for one-off evaluation.
pub struct MapContext<'a> {
    pub cells: &'a IndexMap<String, Cell>,
    pub names: &'a Names,
    /// The sheet `cells` belongs to, so `Q3!A1` written on sheet Q3 resolves
    /// locally. Empty when the caller has only one anonymous sheet.
    pub sheet: &'a str,
    pub book: &'a Book<'a>,
}

impl<'a> MapContext<'a> {
    /// A context with no sibling sheets, which is what most callers have.
    pub fn new(cells: &'a IndexMap<String, Cell>, names: &'a Names) -> MapContext<'a> {
        static EMPTY: std::sync::LazyLock<Book<'static>> = std::sync::LazyLock::new(Book::new);
        MapContext {
            cells,
            names,
            sheet: "",
            book: &EMPTY,
        }
    }
}

impl Context for MapContext<'_> {
    fn get_value(&self, reference: &str) -> Value {
        let key = bare_ref(reference);
        let (sheet, local) = crate::refs::split_sheet(&key);
        let cells = match sheet {
            None => Some(self.cells),
            Some(name) if name.eq_ignore_ascii_case(self.sheet) => Some(self.cells),
            Some(name) => self.book.get(&name.to_lowercase()).copied(),
        };
        let Some(cells) = cells else {
            self.note_unresolved();
            return err(REF_ERR);
        };
        cells.get(local).map(|c| c.value()).unwrap_or(Value::Blank)
    }

    fn get_name(&self, name: &str) -> Option<Json> {
        self.names
            .get(name)
            .or_else(|| self.names.get(&name.to_uppercase()))
            .cloned()
    }

    fn get_spill(&self, anchor: &str) -> Option<String> {
        let (sheet, local) = crate::refs::split_sheet(anchor);
        let cells = match sheet {
            None => Some(self.cells),
            Some(name) if name.eq_ignore_ascii_case(self.sheet) => Some(self.cells),
            Some(name) => self.book.get(&name.to_lowercase()).copied(),
        }?;
        cells.get(local)?.spill.clone()
    }
}

/// Evaluate one AST against a context.
pub fn evaluate(ast: &Node, ctx: &dyn Context) -> Value {
    match ast {
        Node::Number(n) => Value::Number(*n),
        Node::Text(s) => Value::Text(s.clone()),
        Node::Bool(b) => Value::Bool(*b),
        Node::Blank => Value::Blank,
        Node::Error(code) => err(code),
        // Strip $ anchors here rather than in every Context: $E$7 and E7 are the
        // same cell, and only rewriting cares about anchors.
        Node::Ref(r) => ctx.get_value(&bare_ref(r)),
        Node::Range(r) => ctx.get_range(r),
        // `E2#`: the whole range E2's formula spilled over. When E2 does not
        // spill — a scalar formula, a plain value, nothing at all — Excel says
        // #REF!, and so does this.
        Node::SpillRef(anchor) => {
            let key = bare_ref(anchor);
            match ctx.get_spill(&key) {
                Some(range) => {
                    let (sheet, _) = crate::refs::split_sheet(&key);
                    ctx.get_range(&crate::refs::with_sheet(sheet, &range))
                }
                None => err(REF_ERR),
            }
        }
        Node::Array(items) => Value::Array(items.iter().map(|i| evaluate(i, ctx)).collect()),
        Node::Name(name) => match ctx.get_name(name) {
            None => err(NAME_ERR),
            Some(Json::String(target)) => {
                // A name can point into another sheet — `데이터!A1:D5` — so the
                // sheet part is set aside before the shape is recognised and put
                // back before the lookup.
                let (sheet, local) = crate::refs::split_sheet(&target);
                if parse_range(local).is_some() {
                    ctx.get_range(&bare_ref(&target))
                } else if parse_ref(local).is_some() {
                    ctx.get_value(&bare_ref(&target))
                } else {
                    let _ = sheet;
                    Value::Text(target)
                }
            }
            Some(other) => json_to_value(&other),
        },
        Node::Percent(arg) => match num_of(&evaluate(arg, ctx)) {
            Ok(n) => Value::Number(n / 100.0),
            Err(e) => e,
        },
        Node::Unary { negate, arg } => match num_of(&evaluate(arg, ctx)) {
            Ok(n) => Value::Number(if *negate { -n } else { n }),
            Err(e) => e,
        },
        Node::Binary { op, left, right } => {
            binary(*op, &evaluate(left, ctx), &evaluate(right, ctx))
        }
        Node::Call { name, args } => {
            // A few functions need the *reference* an argument is, not the value
            // it holds: `ROW(A5)` is 5 whatever A5 contains. Those are answered
            // here, where the syntax tree is still in hand.
            if let Some(value) = reference_call(name, args, ctx) {
                return value;
            }
            let args: Vec<Value> = args.iter().map(|a| evaluate(a, ctx)).collect();
            functions::call(name, &args).unwrap_or_else(|| {
                // A function this build does not have. The formula's own text is
                // kept either way; what the context decides is whether the value
                // beside it is worth overwriting with `#NAME?`.
                ctx.note_unresolved();
                err(NAME_ERR)
            })
        }
    }
}

/// The functions whose arguments are references rather than values.
///
/// `None` means this is not one of them. Everything here would be impossible in
/// [`functions`], which sees only the evaluated arguments — by then `A5` is the
/// number 42 and its address is gone.
fn reference_call(name: &str, args: &[Node], ctx: &dyn Context) -> Option<Value> {
    /// The address an argument names, if it names one.
    fn address(node: &Node) -> Option<String> {
        match node {
            Node::Ref(r) => Some(bare_ref(r)),
            Node::Range(r) => Some(bare_ref(r)),
            _ => None,
        }
    }

    match name {
        "ROW" | "COLUMN" => {
            let reference = match args.first() {
                None => ctx.current_ref()?,
                Some(node) => address(node)?,
            };
            let (_, local) = crate::refs::split_sheet(&reference);
            // A range answers with its top-left, which is what Excel does when
            // the result cannot spill.
            let first = local.split(':').next().unwrap_or(local);
            let cell = parse_ref(first)?;
            Some(Value::Number(if name == "ROW" {
                cell.row as f64 + 1.0
            } else {
                cell.col as f64 + 1.0
            }))
        }
        "INDIRECT" => {
            let target = crate::values::to_text(&evaluate(args.first()?, ctx));
            let (_, local) = crate::refs::split_sheet(&target);
            let bare: String = local.chars().filter(|c| *c != '$').collect();
            Some(if bare.contains(':') {
                ctx.get_range(&bare_ref(&target))
            } else if parse_ref(&bare).is_some() {
                ctx.get_value(&bare_ref(&target))
            } else {
                err(REF_ERR)
            })
        }
        "OFFSET" => {
            let base = address(args.first()?)?;
            let (sheet, local) = crate::refs::split_sheet(&base);
            let start = parse_ref(local.split(':').next().unwrap_or(local))?;
            let number = |i: usize, default: f64| -> Option<f64> {
                match args.get(i) {
                    None => Some(default),
                    Some(node) => match crate::values::num_of(&evaluate(node, ctx)) {
                        Ok(n) => Some(n.trunc()),
                        Err(_) => None,
                    },
                }
            };
            let (dr, dc) = (number(1, 0.0)?, number(2, 0.0)?);
            // Height and width default to the shape of the reference, which for
            // a single cell is 1x1.
            let (height, width) = (number(3, 1.0)?.max(1.0), number(4, 1.0)?.max(1.0));
            let row = start.row as f64 + dr;
            let col = start.col as f64 + dc;
            if row < 0.0 || col < 0.0 {
                return Some(err(REF_ERR));
            }
            let top = crate::refs::to_ref(col as usize, row as usize);
            Some(if height == 1.0 && width == 1.0 {
                ctx.get_value(&crate::refs::with_sheet(sheet, &top))
            } else {
                let bottom = crate::refs::to_ref(
                    (col + width - 1.0) as usize,
                    (row + height - 1.0) as usize,
                );
                ctx.get_range(&crate::refs::with_sheet(sheet, &format!("{top}:{bottom}")))
            })
        }
        _ => None,
    }
}

fn binary(op: Op, left: &Value, right: &Value) -> Value {
    // A range or array on either side applies the operator element by element.
    // `SUMPRODUCT((A2:A7="서울")*C2:C7)` is how spreadsheets did conditional sums
    // for twenty years, and it needs exactly this.
    let width = |v: &Value| match v {
        Value::Range(r) => Some(r.cells.len()),
        Value::Array(items) => Some(items.len()),
        _ => None,
    };
    if let (true, Some(len)) = (
        !matches!(op, Op::Percent),
        width(left).or_else(|| width(right)),
    ) {
        if width(left).zip(width(right)).map(|(a, b)| a != b) == Some(true) {
            // Excel pads mismatched shapes with #N/A; this engine has no spilling
            // to show that in, so the mismatch itself is the answer.
            return err(crate::values::NA_ERR);
        }
        let at = |v: &Value, i: usize| match v {
            Value::Range(r) => r
                .cells
                .get(i)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Blank),
            Value::Array(items) => items.get(i).cloned().unwrap_or(Value::Blank),
            other => other.clone(),
        };
        return Value::Array(
            (0..len)
                .map(|i| binary_scalar(op, &at(left, i), &at(right, i)))
                .collect(),
        );
    }
    binary_scalar(op, left, right)
}

fn binary_scalar(op: Op, left: &Value, right: &Value) -> Value {
    use std::cmp::Ordering;

    let l = left.scalar();
    let r = right.scalar();
    if l.is_err() {
        return l;
    }
    if r.is_err() {
        return r;
    }

    if op == Op::Concat {
        return Value::Text(format!("{}{}", to_text(&l), to_text(&r)));
    }

    if matches!(op, Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Le | Op::Ge) {
        let c = compare_values(&l, &r);
        return Value::Bool(match op {
            Op::Eq => c == Ordering::Equal,
            Op::Ne => c != Ordering::Equal,
            Op::Lt => c == Ordering::Less,
            Op::Gt => c == Ordering::Greater,
            Op::Le => c != Ordering::Greater,
            _ => c != Ordering::Less,
        });
    }

    let (a, b) = match (num_of(&l), num_of(&r)) {
        (Err(e), _) => return e,
        (_, Err(e)) => return e,
        (Ok(a), Ok(b)) => (a, b),
    };
    match op {
        Op::Add => Value::Number(a + b),
        Op::Sub => Value::Number(a - b),
        Op::Mul => Value::Number(a * b),
        Op::Div => {
            if b == 0.0 {
                err(crate::values::DIV0_ERR)
            } else {
                Value::Number(a / b)
            }
        }
        Op::Pow => {
            let p = a.powf(b);
            if p.is_finite() {
                Value::Number(p)
            } else {
                err(NUM_ERR)
            }
        }
        _ => err(VALUE_ERR),
    }
}

/* --------------------------------------------------------- sheet recalc */

/// A sheet with more than this many evaluation steps is assumed pathological.
const MAX_STEPS: usize = 200_000;

#[derive(Debug, Default)]
pub struct Recalc {
    pub cells: IndexMap<String, Cell>,
    pub changed: Vec<String>,
    pub errors: IndexMap<String, String>,
    /// Cells whose formula could not be recalculated here, and whose stored
    /// value was therefore left alone.
    pub unresolved: Vec<String>,
}

struct RecalcCtx<'a> {
    source: &'a IndexMap<String, Cell>,
    names: &'a Names,
    /// The name of the sheet being recalculated, so a formula that names its own
    /// sheet resolves locally.
    sheet: &'a str,
    book: &'a Book<'a>,
    memo: RefCell<HashMap<String, Value>>,
    visiting: RefCell<HashSet<String>>,
    asts: RefCell<HashMap<String, Rc<Node>>>,
    steps: RefCell<usize>,
    /// Set while evaluating a cell whose formula reached something this build
    /// cannot resolve.
    unresolved: std::cell::Cell<bool>,
    /// The cell currently being evaluated, for `ROW()` with no argument.
    current: RefCell<Option<String>>,
    /// The full array a formula cell produced, before `get_value` reduced it to
    /// its top-left for dependents. This is what the spill pass reads.
    arrays: RefCell<HashMap<String, Value>>,
}

impl<'a> RecalcCtx<'a> {
    /// The cells a reference points into, and the key inside them.
    ///
    /// `None` for the sheet means "this one". A named sheet we do not have is
    /// not an error the caller should act on by writing `#REF!` over the file's
    /// own value — see `note_unresolved`.
    fn locate(&self, key: &'a str) -> Option<(&'a IndexMap<String, Cell>, &'a str)> {
        let (sheet, local) = crate::refs::split_sheet(key);
        match sheet {
            None => Some((self.source, local)),
            Some(name) if name.eq_ignore_ascii_case(self.sheet) => Some((self.source, local)),
            Some(name) => self
                .book
                .get(&name.to_lowercase())
                .map(|cells| (*cells, local)),
        }
    }
}

impl Context for RecalcCtx<'_> {
    fn note_unresolved(&self) {
        self.unresolved.set(true);
    }

    fn current_ref(&self) -> Option<String> {
        self.current.borrow().clone()
    }

    fn get_value(&self, reference: &str) -> Value {
        let key = bare_ref(reference);
        if let Some(v) = self.memo.borrow().get(&key) {
            return v.clone();
        }
        {
            let mut steps = self.steps.borrow_mut();
            *steps += 1;
            if *steps > MAX_STEPS {
                return err(NUM_ERR);
            }
        }

        // A reference into a sheet this context was not given: the formula
        // cannot be recalculated, and the value already in the cell is the best
        // answer available.
        let Some((cells, local)) = self.locate(&key) else {
            self.note_unresolved();
            return err(REF_ERR);
        };
        // Another sheet's cell is read at its cached value rather than
        // recursively recalculated: each sheet is recalculated on its own, and
        // cross-sheet recursion would need the whole workbook's dependency graph
        // to stay cycle-free.
        if !std::ptr::eq(cells, self.source) {
            let value = cells.get(local).map(|c| c.value()).unwrap_or(Value::Blank);
            self.memo.borrow_mut().insert(key, value.clone());
            return value;
        }
        let key = local.to_string();
        if let Some(v) = self.memo.borrow().get(&key) {
            return v.clone();
        }

        let Some(cell) = self.source.get(&key) else {
            return Value::Blank;
        };

        if let Some(formula) = cell.formula() {
            if self.visiting.borrow().contains(&key) {
                return err(CIRC_ERR);
            }
            self.visiting.borrow_mut().insert(key.clone());

            let ast = {
                let cached = self.asts.borrow().get(&key).cloned();
                match cached {
                    Some(a) => a,
                    None => {
                        let node = Rc::new(match parse(formula) {
                            Ok(node) => node,
                            Err(e) => Node::Error(e.code),
                        });
                        self.asts.borrow_mut().insert(key.clone(), node.clone());
                        node
                    }
                }
            };

            let previous = self.current.replace(Some(key.clone()));
            let mut value = evaluate(&ast, self);
            *self.current.borrow_mut() = previous;
            self.visiting.borrow_mut().remove(&key);
            // An array result spills; the cell itself — and any plain reference
            // to it — carries the top-left, exactly as in Excel.
            if matches!(value, Value::Range(_) | Value::Array(_)) {
                self.arrays.borrow_mut().insert(key.clone(), value.clone());
                value = value.scalar();
            }
            self.memo.borrow_mut().insert(key, value.clone());
            return value;
        }

        let literal = cell.value();
        self.memo.borrow_mut().insert(key, literal.clone());
        literal
    }

    fn get_name(&self, name: &str) -> Option<Json> {
        self.names
            .get(name)
            .or_else(|| self.names.get(&name.to_uppercase()))
            .cloned()
    }

    /// The anchor's spill range as of the previous round — the fixpoint loop
    /// re-runs until this and the values behind it stop moving.
    fn get_spill(&self, anchor: &str) -> Option<String> {
        let (cells, local) = self.locate(anchor)?;
        cells.get(local)?.spill.clone()
    }
}

/// Recalculate every formula in a sheet.
///
/// Formula cells are evaluated on demand with memoisation; a cell already on the
/// evaluation stack yields `#CIRC!` instead of recursing forever. Literal cells
/// are returned untouched, so a sheet with no formulas costs one pass.
pub fn recalc_sheet(cells: &IndexMap<String, Cell>, names: &Names) -> Recalc {
    static EMPTY: std::sync::LazyLock<Book<'static>> = std::sync::LazyLock::new(Book::new);
    recalc_sheet_in(cells, names, "", &EMPTY)
}

/// The same, with the workbook's other sheets available.
///
/// A formula that reaches a sheet the book does not have — or calls a function
/// this build does not implement — **keeps the value already in the cell**.
/// Opening someone's workbook and replacing their numbers with `#REF!` is worse
/// than showing a number we could not re-derive: the file said it, and the file
/// is what the reader came for.
///
/// Dynamic arrays spill: a formula producing an array writes its values into
/// the empty cells below and to the right (marked `spillFrom`), or turns into
/// `#SPILL!` if anything is in the way. Spilled values can feed other formulas,
/// so the sheet is recalculated to a fixpoint rather than in one pass.
pub fn recalc_sheet_in(
    cells: &IndexMap<String, Cell>,
    names: &Names,
    sheet: &str,
    book: &Book<'_>,
) -> Recalc {
    static NONE: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(HashSet::new);
    recalc_sheet_blocked(cells, names, sheet, book, &NONE)
}

/// The same, refusing to spill into `blocked` cells.
///
/// The caller knows things the cell map cannot say — above all which cells a
/// merge covers. Excel refuses to spill into (or out of) a merged cell, and a
/// value written under a merge cover would exist in the file but never on the
/// screen, which is the worst kind of existing.
pub fn recalc_sheet_blocked(
    cells: &IndexMap<String, Cell>,
    names: &Names,
    sheet: &str,
    book: &Book<'_>,
    blocked: &HashSet<String>,
) -> Recalc {
    let mut current: IndexMap<String, Cell> = cells
        .iter()
        .map(|(k, c)| (k.to_uppercase(), c.clone()))
        .collect();
    let mut out = Recalc::default();
    for _ in 0..MAX_SPILL_ROUNDS {
        out = recalc_once(&current, names, sheet, book, blocked);
        if out.cells == current {
            break;
        }
        current = out.cells.clone();
    }

    // `changed` compares the final state against what the caller sent, so the
    // editor repaints spilled cells and cleared ghosts too.
    out.changed = diff_keys(cells, &out.cells);
    out
}

/// Every recalculation round the sheet needs before it stops moving. Two covers
/// a plain spill (spill, then the formulas that read the spilled cells); deeper
/// chains take one more each; a spill that feeds its own size never settles and
/// stops here.
const MAX_SPILL_ROUNDS: usize = 8;

/// Spilling more cells than this is assumed to be a mistake (`SEQUENCE(1e6)`),
/// and answers `#SPILL!` rather than flooding the file.
const MAX_SPILL_CELLS: usize = 10_000;

fn recalc_once(
    cells: &IndexMap<String, Cell>,
    names: &Names,
    sheet: &str,
    book: &Book<'_>,
    blocked: &HashSet<String>,
) -> Recalc {
    // Ghost cells are evaluated at their previous round's values — that is the
    // fixpoint mechanism: a formula reading a spilled cell sees last round's
    // spill, and the rounds repeat until nothing moves.
    let ctx = RecalcCtx {
        source: cells,
        names,
        sheet,
        book,
        memo: RefCell::new(HashMap::new()),
        visiting: RefCell::new(HashSet::new()),
        asts: RefCell::new(HashMap::new()),
        steps: RefCell::new(0),
        unresolved: std::cell::Cell::new(false),
        current: RefCell::new(None),
        arrays: RefCell::new(HashMap::new()),
    };

    let mut out = Recalc::default();
    for (reference, cell) in cells {
        let key = reference.clone();
        if cell.formula().is_some() {
            ctx.unresolved.set(false);
            let value = ctx.get_value(&key);
            if ctx.unresolved.get() && !cell.v.is_null() {
                // Cannot be recalculated here; leave the file's own value —
                // and, via the untouched `spill` field, its spilled cells.
                ctx.arrays.borrow_mut().remove(&key);
                out.unresolved.push(key.clone());
                out.cells.insert(key, cell.clone());
                continue;
            }
            let (v, t) = typed(&value);
            let t = keep_date_tag(cell, t);
            let mut next = cell.clone();
            next.v = v;
            next.t = Some(t.to_string());
            if let Value::Error(code) = &value {
                out.errors.insert(key.clone(), code.clone());
            }
            out.cells.insert(key, next);
        } else {
            let (v, t) = typed(&cell.value());
            let t = keep_date_tag(cell, t);
            let mut next = cell.clone();
            next.v = v;
            next.t = Some(t.to_string());
            out.cells.insert(key, next);
        }
    }

    let unresolved: HashSet<&String> = out.unresolved.iter().collect();
    let arrays = ctx.arrays.into_inner();

    // Anchors that no longer produce an array give up their spill claim before
    // ghosts are reconciled against it.
    for (key, cell) in out.cells.iter_mut() {
        if cell.spill.is_some() && !arrays.contains_key(key) && !unresolved.contains(key) {
            cell.spill = None;
        }
    }

    spill_arrays(&mut out, arrays, blocked);
    sweep_stale_ghosts(&mut out.cells);
    out
}

/// Write each anchor's array into the cells below and to the right of it, or
/// mark the anchor `#SPILL!` when something is in the way.
fn spill_arrays(out: &mut Recalc, arrays: HashMap<String, Value>, blocked: &HashSet<String>) {
    use crate::refs::{parse_ref, to_ref};
    use crate::values::SPILL_ERR;

    // Address order, so two anchors contending for the same cells resolve the
    // same way every time: the earlier anchor wins, the later one errors.
    let mut anchors: Vec<(String, Value)> = arrays.into_iter().collect();
    anchors.sort_by_key(|(key, _)| parse_ref(key).map(|r| (r.row, r.col)));

    for (key, full) in anchors {
        let Some(origin) = parse_ref(&key) else {
            continue;
        };
        let (rows, cols, elements) = array_shape(&full);
        if rows * cols <= 1 {
            if let Some(anchor) = out.cells.get_mut(&key) {
                anchor.spill = None;
            }
            continue;
        }

        // Anything with content blocks the spill — a user's cell, another
        // formula, another anchor's ghost (overlapping spills: the earlier
        // anchor wins, the later errors, as in Excel). This anchor's own
        // ghosts from the previous round do not block; they are its to rewrite.
        // A merged cell blocks even when empty, the anchor's own included —
        // Excel refuses to spill out of or into a merge.
        let refused = rows * cols > MAX_SPILL_CELLS
            || targets(&origin, rows, cols).any(|reference| blocked.contains(&reference))
            || targets(&origin, rows, cols).skip(1).any(|reference| {
                out.cells.get(&reference).is_some_and(|c| {
                    c.blocks_spill() && c.spill_from.as_deref() != Some(key.as_str())
                })
            });
        if refused {
            if let Some(anchor) = out.cells.get_mut(&key) {
                anchor.v = Json::String(SPILL_ERR.to_string());
                anchor.t = Some("e".to_string());
                anchor.spill = None;
            }
            out.errors.insert(key.clone(), SPILL_ERR.to_string());
            continue;
        }

        let last = to_ref(origin.col + cols - 1, origin.row + rows - 1);
        for (i, reference) in targets(&origin, rows, cols).enumerate() {
            let element = elements.get(i).cloned().unwrap_or(Value::Blank).scalar();
            let (v, t) = typed(&element);
            if let Value::Error(code) = &element {
                out.errors.insert(reference.clone(), code.clone());
            }
            if i == 0 {
                let anchor = out.cells.get_mut(&key).expect("anchor was evaluated");
                anchor.v = v;
                anchor.t = Some(t.to_string());
                anchor.spill = Some(format!("{key}:{last}"));
                continue;
            }
            let ghost = out.cells.entry(reference).or_default();
            ghost.v = v;
            ghost.t = Some(t.to_string());
            ghost.spill_from = Some(key.clone());
        }
    }
}

/// Clear ghosts whose anchor no longer spills over them — the formula was
/// edited, errored, or now produces a smaller array. Their formatting stays,
/// as it does when Excel retracts a spill.
fn sweep_stale_ghosts(cells: &mut IndexMap<String, Cell>) {
    use crate::refs::{parse_range, parse_ref};

    let covered = |anchor: &str, reference: &str| -> bool {
        let Some(spill) = cells.get(anchor).and_then(|c| c.spill.as_deref()) else {
            return false;
        };
        let (Some(range), Some(at)) = (parse_range(spill), parse_ref(reference)) else {
            return false;
        };
        (range.start.row..=range.end.row).contains(&at.row)
            && (range.start.col..=range.end.col).contains(&at.col)
    };

    let stale: Vec<String> = cells
        .iter()
        .filter(|(reference, cell)| {
            cell.spill_from
                .as_deref()
                .is_some_and(|anchor| !covered(anchor, reference))
        })
        .map(|(reference, _)| reference.clone())
        .collect();
    for reference in stale {
        let cell = cells.get_mut(&reference).expect("listed above");
        cell.v = Json::Null;
        cell.t = None;
        cell.spill_from = None;
        if cell.is_empty() {
            cells.shift_remove(&reference);
        }
    }
}

/// The spill range's cell addresses in row-major order, anchor first.
fn targets(
    origin: &crate::refs::CellRef,
    rows: usize,
    cols: usize,
) -> impl Iterator<Item = String> + '_ {
    (0..rows).flat_map(move |r| {
        (0..cols).map(move |c| crate::refs::to_ref(origin.col + c, origin.row + r))
    })
}

/// An evaluated array's shape and row-major elements. A shapeless `Array` — a
/// `UNIQUE`, a `FILTER` — is a column, which is what those functions return.
fn array_shape(value: &Value) -> (usize, usize, Vec<Value>) {
    match value {
        Value::Range(r) => (r.rows, r.cols, r.values().cloned().collect()),
        Value::Array(items) => (items.len(), 1, items.clone()),
        other => (1, 1, vec![other.clone()]),
    }
}

/// The keys whose stored value or type differs between two sheets, including
/// keys present on only one side.
fn diff_keys(before: &IndexMap<String, Cell>, after: &IndexMap<String, Cell>) -> Vec<String> {
    let normalized: HashMap<String, &Cell> = before
        .iter()
        .map(|(key, cell)| (key.to_uppercase(), cell))
        .collect();
    let mut changed: Vec<String> = after
        .iter()
        .filter(|(key, cell)| {
            normalized
                .get(*key)
                .is_none_or(|b| b.v != cell.v || b.t != cell.t)
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in normalized.keys() {
        if !after.contains_key(key) {
            changed.push(key.clone());
        }
    }
    changed
}

/// Keep a `d` tag that the value alone cannot carry.
///
/// A date is a number plus the knowledge that it is a date, and only `t` holds
/// that. Recomputing the tag from the value turns every date back into a plain
/// number, which made the column summary add date serials into a total — a sum
/// no reader could reconcile with the sheet.
fn keep_date_tag(cell: &Cell, computed: &'static str) -> &'static str {
    if computed == "n" && cell.t.as_deref() == Some("d") {
        "d"
    } else {
        computed
    }
}

/// Convert an evaluated value into the stored `(v, t)` pair.
pub fn typed(value: &Value) -> (Json, &'static str) {
    match value {
        Value::Error(code) => (Json::String(code.clone()), "e"),
        v if v.is_blankish() => (Json::Null, "z"),
        Value::Number(n) => {
            let rounded = jsnum::to_precision(*n, 14);
            // `SUM()` folds from -0.0; nobody wants to see the sign of nothing.
            let rounded = if rounded == 0.0 { 0.0 } else { rounded };
            (
                serde_json::Number::from_f64(rounded)
                    .map(Json::Number)
                    .unwrap_or(Json::Null),
                "n",
            )
        }
        Value::Bool(b) => (Json::Bool(*b), "b"),
        other => (Json::String(to_text(other)), "s"),
    }
}

/// Interpret what a user typed into a cell.
/// `None` clears the cell.
pub fn parse_cell_input(raw: &str) -> Option<Cell> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut cell = Cell::default();

    if trimmed.starts_with('=') {
        cell.f = Some(trimmed.to_string());
        cell.t = Some("n".into());
        return Some(cell);
    }

    let upper = trimmed.to_uppercase();
    if upper == "TRUE" || upper == "FALSE" {
        cell.v = Json::Bool(upper == "TRUE");
        cell.t = Some("b".into());
        return Some(cell);
    }

    // Percent: `12%`, `1,200.5 %`
    if let Some(body) = trimmed.strip_suffix('%') {
        if let Some(n) = plain_decimal(body.trim()) {
            cell.v = json_number(n / 100.0);
            cell.t = Some("n".into());
            cell.fmt = Some("0.0%".into());
            return Some(cell);
        }
    }

    // Currency: `₩1,200`, `$3.50`
    let mut chars = trimmed.chars();
    if let Some(symbol) = chars.next() {
        if "₩$€£¥".contains(symbol) {
            if let Some(n) = plain_decimal(chars.as_str().trim()) {
                cell.v = json_number(n);
                cell.t = Some("n".into());
                cell.fmt = Some(format!("{symbol}#,##0"));
                return Some(cell);
            }
        }
    }

    if let Some(n) = plain_number(trimmed) {
        cell.v = json_number(n);
        cell.t = Some("n".into());
        if trimmed.contains(',') {
            cell.fmt = Some("#,##0".into());
        }
        return Some(cell);
    }

    if let Some(serial) = iso_date(trimmed) {
        cell.v = json_number(serial);
        cell.t = Some("d".into());
        cell.fmt = Some("yyyy-mm-dd".into());
        return Some(cell);
    }

    cell.v = Json::String(raw.to_string());
    cell.t = Some("s".into());
    Some(cell)
}

fn json_number(n: f64) -> Json {
    serde_json::Number::from_f64(n)
        .map(Json::Number)
        .unwrap_or(Json::Null)
}

/// `-1,234.5` — digits with optional grouping commas, no exponent.
fn plain_decimal(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
        || body.matches('.').count() > 1
        || !body.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    s.chars()
        .filter(|c| *c != ',')
        .collect::<String>()
        .parse()
        .ok()
}

/// The same, plus scientific notation — what the numeric branch accepts.
fn plain_number(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|c| *c != ',').collect();
    let body = cleaned.strip_prefix('-').unwrap_or(&cleaned);
    if body.is_empty() || !body.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if !body
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-')
    {
        return None;
    }
    cleaned.parse().ok()
}

fn iso_date(s: &str) -> Option<f64> {
    let sep = if s.contains('-') { '-' } else { '/' };
    let mut parts = s.split(sep);
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    ymd_to_serial(y, m, d)
}

/// What the user sees in a cell: formatted value, or the error code.
pub fn display_value(cell: &Cell) -> String {
    if cell.t.as_deref() == Some("e") {
        return match &cell.v {
            Json::String(s) => s.clone(),
            _ => VALUE_ERR.to_string(),
        };
    }
    if cell.v.is_null() {
        return String::new();
    }
    if let Some(fmt) = &cell.fmt {
        return apply_num_fmt(&cell.value(), fmt);
    }
    match &cell.v {
        Json::Number(n) => format_plain_number(n.as_f64().unwrap_or(f64::NAN)),
        Json::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        other => to_text(&json_to_value(other)),
    }
}

/// What the user edits: the formula if there is one, else the raw value.
pub fn edit_value(cell: &Cell) -> String {
    if let Some(f) = &cell.f {
        return f.clone();
    }
    if cell.v.is_null() {
        return String::new();
    }
    if cell.t.as_deref() == Some("d") {
        if let Some(fmt) = &cell.fmt {
            return apply_num_fmt(&cell.value(), fmt);
        }
    }
    match &cell.v {
        Json::Number(n) => format_plain_number(n.as_f64().unwrap_or(f64::NAN)),
        Json::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        other => to_text(&json_to_value(other)),
    }
}

/// Direct dependencies of a formula, for the editor's dependency highlight.
pub fn dependencies(formula: &str) -> Vec<String> {
    let Ok(ast) = parse(formula) else {
        return Vec::new();
    };
    let refs = collect_refs(&ast);
    let mut seen: IndexMap<String, ()> = IndexMap::new();
    for r in &refs.refs {
        seen.insert(bare_ref(r), ());
    }
    for range in &refs.ranges {
        if let Some(r) = parse_range(range) {
            for reference in expand_range(&r) {
                seen.insert(reference, ());
            }
        }
    }
    seen.into_keys().collect()
}

/// Evaluate a single formula against a sheet, without recalculating it.
pub fn evaluate_in(formula: &str, cells: &IndexMap<String, Cell>, names: &Names) -> Value {
    match parse(formula) {
        Err(e) => err(&e.code),
        Ok(ast) => evaluate(&ast, &MapContext::new(cells, names)),
    }
}

#[cfg(test)]
mod spill_tests {
    use super::*;
    use serde_json::json;

    fn sheet(pairs: &[(&str, Cell)]) -> IndexMap<String, Cell> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn text(s: &str) -> Cell {
        Cell {
            v: json!(s),
            t: Some("s".into()),
            ..Cell::default()
        }
    }

    fn formula(f: &str) -> Cell {
        Cell {
            f: Some(f.into()),
            ..Cell::default()
        }
    }

    #[test]
    fn a_unique_spills_down_and_marks_its_cells() {
        let out = recalc_sheet(
            &sheet(&[
                ("A1", text("서울")),
                ("A2", text("부산")),
                ("A3", text("서울")),
                ("C1", formula("=UNIQUE(A1:A3)")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["C1"].v, json!("서울"));
        assert_eq!(out.cells["C1"].spill.as_deref(), Some("C1:C2"));
        assert_eq!(out.cells["C2"].v, json!("부산"));
        assert_eq!(out.cells["C2"].spill_from.as_deref(), Some("C1"));
        assert!(out.cells["C2"].f.is_none(), "a ghost carries no formula");
        assert!(out.changed.iter().any(|k| k == "C2"));
    }

    #[test]
    fn content_in_the_way_gives_spill_error_and_writes_nothing() {
        let out = recalc_sheet(
            &sheet(&[
                ("A1", text("가")),
                ("A2", text("나")),
                ("C1", formula("=SORT(A1:A2)")),
                ("C2", text("점유")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["C1"].v, json!("#SPILL!"));
        assert_eq!(out.cells["C1"].t.as_deref(), Some("e"));
        assert!(out.cells["C1"].spill.is_none());
        assert_eq!(
            out.cells["C2"].v,
            json!("점유"),
            "the occupant is untouched"
        );
        assert_eq!(out.errors.get("C1").map(String::as_str), Some("#SPILL!"));
    }

    #[test]
    fn typing_over_a_ghost_breaks_the_spill_on_the_next_recalc() {
        let first = recalc_sheet(
            &sheet(&[
                ("A1", text("가")),
                ("A2", text("나")),
                ("C1", formula("=SORT(A1:A2)")),
            ]),
            &Names::new(),
        );
        assert_eq!(first.cells["C2"].spill_from.as_deref(), Some("C1"));

        // The user types over the ghost: it becomes an ordinary cell.
        let mut edited = first.cells.clone();
        edited.insert("C2".into(), text("직접 입력"));
        let second = recalc_sheet(&edited, &Names::new());
        assert_eq!(second.cells["C1"].v, json!("#SPILL!"));
        assert_eq!(second.cells["C2"].v, json!("직접 입력"));
        assert!(second.cells["C2"].spill_from.is_none());
    }

    #[test]
    fn spilled_values_feed_other_formulas() {
        let out = recalc_sheet(
            &sheet(&[
                ("A1", formula("=SEQUENCE(3)")),
                // Reads the spilled cells, so it needs the fixpoint round.
                ("C1", formula("=SUM(A1:A3)")),
                // A plain reference to the anchor sees its top-left value.
                ("D1", formula("=A1")),
                // And a reference to a spilled cell sees the spilled value.
                ("E1", formula("=A3*10")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["A2"].v, json!(2.0));
        assert_eq!(out.cells["C1"].v, json!(6.0));
        assert_eq!(out.cells["D1"].v, json!(1.0));
        assert_eq!(out.cells["E1"].v, json!(30.0));
    }

    #[test]
    fn a_removed_formula_clears_its_ghosts() {
        let first = recalc_sheet(&sheet(&[("A1", formula("=SEQUENCE(3)"))]), &Names::new());
        assert!(first.cells.contains_key("A3"));

        let mut edited = first.cells.clone();
        edited.insert("A1".into(), text("이제 텍스트"));
        let second = recalc_sheet(&edited, &Names::new());
        assert!(
            !second.cells.contains_key("A3"),
            "stale ghosts are cleared: {:?}",
            second.cells.get("A3")
        );
        assert!(second.changed.iter().any(|k| k == "A3"));
    }

    #[test]
    fn a_two_dimensional_sequence_keeps_its_shape() {
        let out = recalc_sheet(&sheet(&[("B2", formula("=SEQUENCE(2,3)"))]), &Names::new());
        assert_eq!(out.cells["B2"].spill.as_deref(), Some("B2:D3"));
        assert_eq!(out.cells["D2"].v, json!(3.0));
        assert_eq!(out.cells["B3"].v, json!(4.0));
        assert_eq!(out.cells["D3"].v, json!(6.0));
    }

    #[test]
    fn a_transpose_spills_sideways() {
        let out = recalc_sheet(
            &sheet(&[
                ("A1", text("가")),
                ("A2", text("나")),
                ("C1", formula("=TRANSPOSE(A1:A2)")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["C1"].spill.as_deref(), Some("C1:D1"));
        assert_eq!(out.cells["D1"].v, json!("나"));
    }

    #[test]
    fn a_bare_range_formula_spills_like_excel() {
        let out = recalc_sheet(
            &sheet(&[
                ("A1", text("x")),
                ("A2", text("y")),
                ("C1", formula("=A1:A2")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["C1"].v, json!("x"));
        assert_eq!(out.cells["C2"].v, json!("y"));
    }

    #[test]
    fn an_oversized_spill_is_refused() {
        let out = recalc_sheet(
            &sheet(&[("A1", formula("=SEQUENCE(20000)"))]),
            &Names::new(),
        );
        assert_eq!(out.cells["A1"].v, json!("#SPILL!"));
        assert!(!out.cells.contains_key("A2"));
    }

    #[test]
    fn spilling_into_a_styled_empty_cell_keeps_the_styling() {
        let styled = Cell {
            fmt: Some("0.00".into()),
            extra: [("style".to_string(), json!({"bold": true}))]
                .into_iter()
                .collect(),
            ..Cell::default()
        };
        let out = recalc_sheet(
            &sheet(&[("A1", formula("=SEQUENCE(2)")), ("A2", styled)]),
            &Names::new(),
        );
        assert_eq!(out.cells["A2"].v, json!(2.0));
        assert_eq!(out.cells["A2"].fmt.as_deref(), Some("0.00"));
        assert_eq!(out.cells["A2"].extra["style"], json!({"bold": true}));
        assert_eq!(out.cells["A2"].spill_from.as_deref(), Some("A1"));
    }

    #[test]
    fn overlapping_spills_resolve_in_address_order() {
        let out = recalc_sheet(
            &sheet(&[
                ("A1", formula("=SEQUENCE(3)")),
                ("A2", formula("=SEQUENCE(3)")),
            ]),
            &Names::new(),
        );
        // A1 wins the ground; A2 is a formula cell (blocked ground for A1)…
        // actually A2 blocks A1 first: A1 spills over A2:A3 which holds a
        // formula, so A1 errors, then A2 spills freely below.
        assert_eq!(out.cells["A1"].v, json!("#SPILL!"));
        assert_eq!(out.cells["A2"].spill.as_deref(), Some("A2:A4"));
        assert_eq!(out.cells["A4"].v, json!(3.0));
    }

    #[test]
    fn a_spill_reference_reads_the_whole_spilled_range() {
        let out = recalc_sheet(
            &sheet(&[
                (
                    "A1",
                    Cell {
                        v: json!(3.0),
                        t: Some("n".into()),
                        ..Cell::default()
                    },
                ),
                (
                    "A2",
                    Cell {
                        v: json!(1.0),
                        t: Some("n".into()),
                        ..Cell::default()
                    },
                ),
                (
                    "A3",
                    Cell {
                        v: json!(3.0),
                        t: Some("n".into()),
                        ..Cell::default()
                    },
                ),
                ("C1", formula("=UNIQUE(A1:A3)")),
                // Reads C1's spill range without naming its size — the point of `#`.
                ("E1", formula("=SUM(C1#)")),
                ("E2", formula("=COUNTA(C1#)")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["E1"].v, json!(4.0), "3+1 through the spill");
        assert_eq!(out.cells["E2"].v, json!(2.0));
    }

    #[test]
    fn a_spill_reference_follows_the_spill_as_it_grows() {
        let base = sheet(&[
            (
                "A1",
                Cell {
                    v: json!(1.0),
                    t: Some("n".into()),
                    ..Cell::default()
                },
            ),
            ("B1", formula("=UNIQUE(A1:A3)")),
            ("D1", formula("=COUNTA(B1#)")),
        ]);
        let first = recalc_sheet(&base, &Names::new());
        assert_eq!(first.cells["D1"].v, json!(1.0));

        // More distinct data appears; the same `B1#` now covers more cells.
        let mut grown = first.cells.clone();
        grown.insert(
            "A2".into(),
            Cell {
                v: json!(2.0),
                t: Some("n".into()),
                ..Cell::default()
            },
        );
        grown.insert(
            "A3".into(),
            Cell {
                v: json!(5.0),
                t: Some("n".into()),
                ..Cell::default()
            },
        );
        let second = recalc_sheet(&grown, &Names::new());
        assert_eq!(second.cells["B1"].spill.as_deref(), Some("B1:B3"));
        assert_eq!(second.cells["D1"].v, json!(3.0));
    }

    #[test]
    fn a_spill_reference_to_a_non_spilling_cell_is_ref_error() {
        let out = recalc_sheet(
            &sheet(&[
                (
                    "A1",
                    Cell {
                        v: json!(7.0),
                        t: Some("n".into()),
                        ..Cell::default()
                    },
                ),
                ("B1", formula("=A1#")),
                ("C1", formula("=D9#")),
            ]),
            &Names::new(),
        );
        assert_eq!(out.cells["B1"].v, json!("#REF!"));
        assert_eq!(out.cells["C1"].v, json!("#REF!"));
    }

    #[test]
    fn a_spill_reference_reaches_across_sheets() {
        let data = sheet(&[
            ("A1", text("서울")),
            ("A2", text("부산")),
            ("C1", formula("=SORT(A1:A2)")),
        ]);
        // The data sheet is recalculated first, so its spill is materialised.
        let data = recalc_sheet(&data, &Names::new()).cells;
        let summary = sheet(&[("A1", formula("=COUNTA(데이터!C1#)"))]);
        let book = book_of([("데이터", &data)]);
        let out = recalc_sheet_in(&summary, &Names::new(), "요약", &book);
        assert_eq!(out.cells["A1"].v, json!(2.0));
    }

    #[test]
    fn a_merged_cell_blocks_a_spill_even_when_empty() {
        let cells = sheet(&[("A1", formula("=SEQUENCE(3)"))]);
        // A2:B2 is merged: empty, but Excel refuses to spill into a merge.
        let blocked: HashSet<String> = ["A2".to_string(), "B2".to_string()].into();
        let out = recalc_sheet_blocked(&cells, &Names::new(), "", &Book::new(), &blocked);
        assert_eq!(out.cells["A1"].v, json!("#SPILL!"));
        assert!(!out.cells.contains_key("A2"));

        // Without the merge the same sheet spills freely.
        let free = recalc_sheet(&cells, &Names::new());
        assert_eq!(free.cells["A3"].v, json!(3.0));
    }

    #[test]
    fn a_spill_round_trips_through_serde() {
        let out = recalc_sheet(&sheet(&[("A1", formula("=SEQUENCE(2)"))]), &Names::new());
        let json_text = serde_json::to_string(&out.cells).unwrap();
        assert!(json_text.contains("\"spill\":\"A1:A2\""), "{json_text}");
        assert!(json_text.contains("\"spillFrom\":\"A1\""), "{json_text}");
        let back: IndexMap<String, Cell> = serde_json::from_str(&json_text).unwrap();
        assert_eq!(back, out.cells);
    }
}

#[cfg(test)]
mod date_tag_tests {
    use super::*;
    use serde_json::json;

    fn sheet(pairs: &[(&str, Cell)]) -> IndexMap<String, Cell> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn recalculating_keeps_a_date_a_date() {
        let out = recalc_sheet(
            &sheet(&[(
                "A1",
                Cell {
                    v: json!(46296.0),
                    t: Some("d".into()),
                    fmt: Some("yyyy-mm-dd".into()),
                    ..Cell::default()
                },
            )]),
            &Names::new(),
        );
        assert_eq!(out.cells["A1"].t.as_deref(), Some("d"));
        assert_eq!(display_value(&out.cells["A1"]), "2026-10-01");
    }

    #[test]
    fn a_formula_that_produced_a_date_keeps_the_tag() {
        let out = recalc_sheet(
            &sheet(&[(
                "A1",
                Cell {
                    f: Some("=DATE(2026,10,1)".into()),
                    t: Some("d".into()),
                    fmt: Some("yyyy-mm-dd".into()),
                    ..Cell::default()
                },
            )]),
            &Names::new(),
        );
        assert_eq!(out.cells["A1"].t.as_deref(), Some("d"));
        assert_eq!(display_value(&out.cells["A1"]), "2026-10-01");
    }

    #[test]
    fn a_plain_number_is_not_promoted_to_a_date() {
        let out = recalc_sheet(
            &sheet(&[(
                "A1",
                Cell {
                    v: json!(42.0),
                    ..Cell::default()
                },
            )]),
            &Names::new(),
        );
        assert_eq!(out.cells["A1"].t.as_deref(), Some("n"));
    }

    #[test]
    fn a_date_cell_that_becomes_text_gives_up_the_tag() {
        let out = recalc_sheet(
            &sheet(&[(
                "A1",
                Cell {
                    v: json!("직접 입력"),
                    t: Some("d".into()),
                    ..Cell::default()
                },
            )]),
            &Names::new(),
        );
        assert_eq!(out.cells["A1"].t.as_deref(), Some("s"));
    }
}
