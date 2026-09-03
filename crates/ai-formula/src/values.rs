//! Cell values and Excel's coercion rules.

use std::rc::Rc;

use crate::jsnum;

/// Every error code the engine can produce, in the order the lexer scans them.
pub const ERROR_CODES: [&str; 8] = [
    "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#N/A", "#NUM!", "#CIRC!", "#SPILL!",
];

pub const VALUE_ERR: &str = "#VALUE!";
pub const NUM_ERR: &str = "#NUM!";
pub const DIV0_ERR: &str = "#DIV/0!";
pub const NA_ERR: &str = "#N/A";
pub const REF_ERR: &str = "#REF!";
pub const NAME_ERR: &str = "#NAME?";
pub const CIRC_ERR: &str = "#CIRC!";
/// A dynamic array could not spill: something occupies its range.
pub const SPILL_ERR: &str = "#SPILL!";

/// The result of evaluating a range reference.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeValue {
    pub cells: Vec<(String, Value)>,
    pub rows: usize,
    pub cols: usize,
}

impl RangeValue {
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.cells.iter().map(|(_, v)| v)
    }

    pub fn first(&self) -> Value {
        self.cells
            .first()
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Blank)
    }

    /// Row-major 2D view, for `INDEX`/`VLOOKUP`.
    pub fn grid(&self) -> Vec<Vec<Value>> {
        if self.cols == 0 {
            return Vec::new();
        }
        self.cells
            .chunks(self.cols)
            .map(|row| row.iter().map(|(_, v)| v.clone()).collect())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// JS `null` — an empty cell.
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Range(Rc<RangeValue>),
    /// An inline array literal, `{1,2,3}`.
    Array(Vec<Value>),
}

pub fn err(code: &str) -> Value {
    Value::Error(code.to_string())
}

impl Value {
    pub fn is_err(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    pub fn err_code(&self) -> Option<&str> {
        match self {
            Value::Error(c) => Some(c),
            _ => None,
        }
    }

    pub fn is_range(&self) -> bool {
        matches!(self, Value::Range(_))
    }

    /// True where JS wrote `v === null || v === undefined || v === ''`.
    pub fn is_blankish(&self) -> bool {
        match self {
            Value::Blank => true,
            Value::Text(s) => s.is_empty(),
            _ => false,
        }
    }

    /// A range collapses to its first cell in scalar position.
    pub fn scalar(&self) -> Value {
        match self {
            Value::Range(r) => r.first(),
            Value::Array(items) => items.first().cloned().unwrap_or(Value::Blank),
            other => other.clone(),
        }
    }
}

/// Flatten args, expanding ranges and arrays, dropping nothing.
/// A value as a range, so a function written for ranges also accepts the array
/// a `SORT` or `UNIQUE` produced.
pub fn as_range(v: &Value) -> Option<Rc<RangeValue>> {
    match v {
        Value::Range(r) => Some(r.clone()),
        Value::Array(items) => Some(Rc::new(RangeValue {
            cells: items
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("A{}", i + 1), v.clone()))
                .collect(),
            rows: items.len(),
            cols: 1,
        })),
        _ => None,
    }
}

pub fn flatten(args: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Range(r) => r.values().for_each(|x| walk(x, out)),
            Value::Array(items) => items.iter().for_each(|x| walk(x, out)),
            other => out.push(other.clone()),
        }
    }
    args.iter().for_each(|v| walk(v, &mut out));
    out
}

/// First error found in args, or `None`.
pub fn first_error(args: &[Value]) -> Option<Value> {
    flatten(args).into_iter().find(|v| v.is_err())
}

/// JS `Number(string)` for the forms a spreadsheet realistically contains.
fn js_string_to_number(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.is_empty() {
        return Some(0.0);
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok().map(|v| v as f64);
    }
    if let Some(bin) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        return u64::from_str_radix(bin, 2).ok().map(|v| v as f64);
    }
    if let Some(oct) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
        return u64::from_str_radix(oct, 8).ok().map(|v| v as f64);
    }
    match s {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    // Rust's f64 parser accepts "inf"/"nan"; JS does not.
    let lower = s.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return None;
    }
    s.parse::<f64>().ok()
}

/// Excel-style numeric coercion. Blanks are 0; unparseable text is an error.
pub fn to_number(v: &Value) -> Value {
    match v {
        Value::Blank => Value::Number(0.0),
        Value::Number(n) => {
            if n.is_finite() {
                Value::Number(*n)
            } else {
                err(NUM_ERR)
            }
        }
        Value::Bool(b) => Value::Number(if *b { 1.0 } else { 0.0 }),
        Value::Error(_) => v.clone(),
        Value::Range(r) => {
            let first = r
                .cells
                .first()
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Number(0.0));
            to_number(&first)
        }
        Value::Array(items) => {
            let first = items.first().cloned().unwrap_or(Value::Number(0.0));
            to_number(&first)
        }
        Value::Text(t) => {
            let s: String = t.trim().chars().filter(|c| *c != ',').collect();
            if s.is_empty() {
                return Value::Number(0.0);
            }
            if let Some(n) = js_string_to_number(&s) {
                if n.is_finite() {
                    return Value::Number(n);
                }
            }
            if let Some(pct) = s.strip_suffix('%') {
                if pct
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
                    && !pct.is_empty()
                {
                    if let Ok(n) = pct.parse::<f64>() {
                        return Value::Number(n / 100.0);
                    }
                }
            }
            err(VALUE_ERR)
        }
    }
}

/// The `f64` behind a value, or the error that stopped us.
pub fn num_of(v: &Value) -> Result<f64, Value> {
    match to_number(v) {
        Value::Number(n) => Ok(n),
        other => Err(other),
    }
}

pub fn to_text(v: &Value) -> String {
    match v {
        Value::Blank => String::new(),
        Value::Error(c) => c.clone(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Range(r) => to_text(&r.first()),
        Value::Array(items) => to_text(items.first().unwrap_or(&Value::Blank)),
        Value::Number(n) => format_plain_number(*n),
        Value::Text(s) => s.clone(),
    }
}

pub fn to_boolean(v: &Value) -> Value {
    match v {
        Value::Error(_) => v.clone(),
        Value::Bool(b) => Value::Bool(*b),
        Value::Blank => Value::Bool(false),
        Value::Number(n) => Value::Bool(*n != 0.0),
        Value::Range(r) => to_boolean(&r.first()),
        Value::Array(items) => to_boolean(items.first().unwrap_or(&Value::Blank)),
        Value::Text(s) => {
            if s.is_empty() {
                return Value::Bool(false);
            }
            match s.trim().to_ascii_uppercase().as_str() {
                "TRUE" => Value::Bool(true),
                "FALSE" => Value::Bool(false),
                other => match js_string_to_number(other) {
                    Some(n) if n.is_finite() => Value::Bool(n != 0.0),
                    _ => err(VALUE_ERR),
                },
            }
        }
    }
}

pub fn bool_of(v: &Value) -> Result<bool, Value> {
    match to_boolean(v) {
        Value::Bool(b) => Ok(b),
        other => Err(other),
    }
}

/// Numbers only — the `SUM`/`AVERAGE` contract. An error anywhere aborts.
///
/// Excel draws a line the flattened stream would erase: text inside a
/// referenced range or array is *ignored* (a `SUM` down a column skips the label
/// cells and a stray `'5` typed as text), but a text argument passed *directly*
/// — `SUM("5", A1)` — is coerced. We keep that distinction by tracking whether a
/// value arrived through a range/array (`direct = false`) or as its own argument.
pub fn numeric_values(args: &[Value]) -> Result<Vec<f64>, Value> {
    let mut out = Vec::new();
    for arg in args {
        collect_numeric(arg, true, &mut out)?;
    }
    Ok(out)
}

fn collect_numeric(v: &Value, direct: bool, out: &mut Vec<f64>) -> Result<(), Value> {
    match v {
        Value::Error(_) => return Err(v.clone()),
        Value::Range(r) => {
            for x in r.values() {
                collect_numeric(x, false, out)?;
            }
        }
        Value::Array(items) => {
            for x in items {
                collect_numeric(x, false, out)?;
            }
        }
        Value::Number(n) => out.push(*n),
        Value::Blank => {}
        // Logical values are ignored here whether direct or referenced; this
        // matches the reader's long-standing behaviour and is left untouched.
        Value::Bool(_) => {}
        Value::Text(s) => {
            // Only a text value handed in as its own argument is coerced; text
            // pulled out of a range or array is a label, not a number.
            if direct && !s.is_empty() {
                let cleaned: String = s.chars().filter(|c| *c != ',').collect();
                if let Some(n) = js_string_to_number(&cleaned) {
                    if n.is_finite() {
                        out.push(n);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Shortest round-trippable decimal, avoiding `0.30000000000000004` in output.
pub fn format_plain_number(n: f64) -> String {
    if !n.is_finite() {
        return jsnum::number_to_string(n);
    }
    if n.fract() == 0.0 && n.abs() < 1e21 {
        return jsnum::number_to_string(n);
    }
    jsnum::number_to_string(jsnum::to_precision(n, 12))
}

/// Excel comparison semantics: numbers sort before text, text compares
/// case-insensitively, and a blank compares as 0 against a number.
pub fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let numeric = |v: &Value| matches!(v, Value::Number(_) | Value::Bool(_));
    let as_f64 = |v: &Value| match v {
        Value::Number(n) => *n,
        Value::Bool(t) => {
            if *t {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    };
    let cmp_f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);

    if a.is_blankish() {
        return if b.is_blankish() {
            Ordering::Equal
        } else if numeric(b) {
            cmp_f(0.0, as_f64(b))
        } else {
            Ordering::Less
        };
    }
    if b.is_blankish() {
        return if numeric(a) {
            cmp_f(as_f64(a), 0.0)
        } else {
            Ordering::Greater
        };
    }
    match (numeric(a), numeric(b)) {
        (true, true) => cmp_f(as_f64(a), as_f64(b)),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => to_text(a).to_lowercase().cmp(&to_text(b).to_lowercase()),
    }
}
