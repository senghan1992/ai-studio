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
        self.f.is_none() && self.v.is_null() && self.fmt.is_none() && self.extra.is_empty()
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
            if let Value::Range(r) = &value {
                value = r.first();
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
pub fn recalc_sheet_in(
    cells: &IndexMap<String, Cell>,
    names: &Names,
    sheet: &str,
    book: &Book<'_>,
) -> Recalc {
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
    };

    let mut out = Recalc::default();
    for (reference, cell) in cells {
        let key = reference.to_uppercase();
        if cell.formula().is_some() {
            ctx.unresolved.set(false);
            let value = ctx.get_value(&key);
            if ctx.unresolved.get() && !cell.v.is_null() {
                // Cannot be recalculated here; leave the file's own value and
                // report which cell it was.
                out.unresolved.push(key.clone());
                out.cells.insert(key, cell.clone());
                continue;
            }
            let (v, t) = typed(&value);
            let t = keep_date_tag(cell, t);
            let mut next = cell.clone();
            let changed = next.v != v || next.t.as_deref() != Some(t);
            next.v = v;
            next.t = Some(t.to_string());
            if changed {
                out.changed.push(key.clone());
            }
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
    out
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
