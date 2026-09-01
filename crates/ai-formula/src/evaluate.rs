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

/// What the evaluator needs from its surroundings.
pub trait Context {
    fn get_value(&self, reference: &str) -> Value;

    fn get_range(&self, range: &str) -> Value {
        let Some(r) = parse_range(range) else {
            return err(REF_ERR);
        };
        let cells: Vec<(String, Value)> = expand_range(&r)
            .into_iter()
            .map(|reference| {
                let v = self.get_value(&reference);
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
}

impl Context for MapContext<'_> {
    fn get_value(&self, reference: &str) -> Value {
        self.cells
            .get(&bare_ref(reference))
            .map(|c| c.value())
            .unwrap_or(Value::Blank)
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
                if parse_range(&target).is_some() {
                    ctx.get_range(&target)
                } else if parse_ref(&target).is_some() {
                    ctx.get_value(&bare_ref(&target))
                } else {
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
            let args: Vec<Value> = args.iter().map(|a| evaluate(a, ctx)).collect();
            functions::call(name, &args).unwrap_or_else(|| err(NAME_ERR))
        }
    }
}

fn binary(op: Op, left: &Value, right: &Value) -> Value {
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
}

struct RecalcCtx<'a> {
    source: &'a IndexMap<String, Cell>,
    names: &'a Names,
    memo: RefCell<HashMap<String, Value>>,
    visiting: RefCell<HashSet<String>>,
    asts: RefCell<HashMap<String, Rc<Node>>>,
    steps: RefCell<usize>,
}

impl Context for RecalcCtx<'_> {
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

            let mut value = evaluate(&ast, self);
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
    let ctx = RecalcCtx {
        source: cells,
        names,
        memo: RefCell::new(HashMap::new()),
        visiting: RefCell::new(HashSet::new()),
        asts: RefCell::new(HashMap::new()),
        steps: RefCell::new(0),
    };

    let mut out = Recalc::default();
    for (reference, cell) in cells {
        let key = reference.to_uppercase();
        if cell.formula().is_some() {
            let value = ctx.get_value(&key);
            let (v, t) = typed(&value);
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
            let mut next = cell.clone();
            next.v = v;
            next.t = Some(t.to_string());
            out.cells.insert(key, next);
        }
    }
    out
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
        Ok(ast) => evaluate(&ast, &MapContext { cells, names }),
    }
}
