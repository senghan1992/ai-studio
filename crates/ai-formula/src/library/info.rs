//! The `IS*` family and the other questions a formula asks about a value.

use crate::values::{err, to_number, to_text, Value, NA_ERR, VALUE_ERR};

pub const NAMES: &[&str] = &[
    "ERROR.TYPE",
    "ISERR",
    "ISEVEN",
    "ISLOGICAL",
    "ISNA",
    "ISNONTEXT",
    "ISODD",
    "N",
    "TYPE",
];

/// These deliberately do **not** propagate errors — inspecting an error is the
/// whole point of half of them.
pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !NAMES.contains(&name) {
        return None;
    }
    let first = args.first().map(Value::scalar).unwrap_or(Value::Blank);
    Some(match name {
        "ISERR" => Value::Bool(matches!(&first, Value::Error(c) if c != NA_ERR)),
        "ISNA" => Value::Bool(matches!(&first, Value::Error(c) if c == NA_ERR)),
        "ISLOGICAL" => Value::Bool(matches!(first, Value::Bool(_))),
        "ISNONTEXT" => Value::Bool(!matches!(first, Value::Text(_))),
        "ISEVEN" | "ISODD" => match to_number(&first) {
            Value::Number(n) => {
                let even = (n.trunc() as i64).rem_euclid(2) == 0;
                Value::Bool(if name == "ISEVEN" { even } else { !even })
            }
            _ => err(VALUE_ERR),
        },
        "N" => match &first {
            Value::Error(_) => first.clone(),
            Value::Number(n) => Value::Number(*n),
            Value::Bool(b) => Value::Number(if *b { 1.0 } else { 0.0 }),
            // Text is 0 rather than an error, which is what N is for.
            _ => Value::Number(0.0),
        },
        "TYPE" => Value::Number(match &first {
            Value::Number(_) | Value::Blank => 1.0,
            Value::Text(_) => 2.0,
            Value::Bool(_) => 4.0,
            Value::Error(_) => 16.0,
            Value::Range(_) | Value::Array(_) => 64.0,
        }),
        "ERROR.TYPE" => match &first {
            Value::Error(code) => {
                // Excel's own numbering, in its own order.
                let codes = [
                    "#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A",
                ];
                match codes.iter().position(|c| c == code) {
                    Some(i) => Value::Number(i as f64 + 1.0),
                    None => Value::Number(codes.len() as f64 + 1.0),
                }
            }
            _ => err(NA_ERR),
        },
        _ => {
            let _ = to_text(&first);
            return None;
        }
    })
}
