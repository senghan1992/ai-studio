//! String functions.

use crate::values::{err, first_error, num_of, to_text, Value, NA_ERR, VALUE_ERR};

pub const NAMES: &[&str] = &[
    "CHAR",
    "CLEAN",
    "CODE",
    "DOLLAR",
    "EXACT",
    "FIXED",
    "NUMBERVALUE",
    "REPLACE",
    "REPT",
    "SEARCH",
    "T",
    "TEXTAFTER",
    "TEXTBEFORE",
    "UNICHAR",
    "UNICODE",
];

fn text_at(args: &[Value], i: usize) -> String {
    to_text(args.get(i).unwrap_or(&Value::Blank))
}

/// Excel counts characters from 1, and in characters rather than bytes — a
/// Korean document would be cut mid-letter otherwise.
fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !NAMES.contains(&name) {
        return None;
    }
    // `T` and `EXACT` are the only ones that look at an error rather than
    // propagating it, and neither does anything useful with one.
    if let Some(e) = first_error(args) {
        return Some(e);
    }
    Some(match name {
        "SEARCH" => {
            // Like FIND, but case-insensitive and wildcard-aware. Most sheets use
            // it as "find, but I do not care about case".
            let needle = text_at(args, 0).to_lowercase();
            let haystack = text_at(args, 1);
            let lower = haystack.to_lowercase();
            let start = match args.get(2) {
                None | Some(Value::Blank) => 1.0,
                Some(v) => match num_of(v) {
                    Err(e) => return Some(e),
                    Ok(n) => n,
                },
            };
            let from = (start.trunc() as isize - 1).max(0) as usize;
            let hay: Vec<char> = chars(&lower);
            if from >= hay.len() && !needle.is_empty() {
                return Some(err(VALUE_ERR));
            }
            let tail: String = hay[from.min(hay.len())..].iter().collect();
            match tail.find(&needle) {
                // `find` gives a byte offset; the answer is in characters.
                Some(at) => Value::Number((from + tail[..at].chars().count() + 1) as f64),
                None => err(VALUE_ERR),
            }
        }
        "REPLACE" => {
            let source = chars(&text_at(args, 0));
            let (start, count) = match (
                num_of(args.get(1).unwrap_or(&Value::Blank)),
                num_of(args.get(2).unwrap_or(&Value::Blank)),
            ) {
                (Ok(a), Ok(b)) => (a.trunc() as isize, b.trunc().max(0.0) as usize),
                (Err(e), _) | (_, Err(e)) => return Some(e),
            };
            if start < 1 {
                return Some(err(VALUE_ERR));
            }
            let at = (start as usize - 1).min(source.len());
            let end = (at + count).min(source.len());
            let mut out: String = source[..at].iter().collect();
            out.push_str(&text_at(args, 3));
            out.extend(&source[end..]);
            Value::Text(out)
        }
        "REPT" => match num_of(args.get(1).unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(n) if n < 0.0 => err(VALUE_ERR),
            Ok(n) => {
                let times = n.trunc() as usize;
                let unit = text_at(args, 0);
                // A guard against `REPT("x", 1e9)` eating the process.
                if unit.len().saturating_mul(times) > 32_768 {
                    err(VALUE_ERR)
                } else {
                    Value::Text(unit.repeat(times))
                }
            }
        },
        "CHAR" | "UNICHAR" => match num_of(args.first().unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(n) => {
                let code = n.trunc();
                if code < 1.0 || code > 1_114_111.0 {
                    return Some(err(VALUE_ERR));
                }
                match char::from_u32(code as u32) {
                    Some(c) => Value::Text(c.to_string()),
                    None => err(VALUE_ERR),
                }
            }
        },
        "CODE" | "UNICODE" => match chars(&text_at(args, 0)).first() {
            None => err(VALUE_ERR),
            Some(c) => Value::Number(*c as u32 as f64),
        },
        "CLEAN" => Value::Text(
            text_at(args, 0)
                .chars()
                .filter(|c| !c.is_control())
                .collect(),
        ),
        "EXACT" => Value::Bool(text_at(args, 0) == text_at(args, 1)),
        "T" => match args.first() {
            Some(Value::Text(s)) => Value::Text(s.clone()),
            Some(other) => match other.scalar() {
                Value::Text(s) => Value::Text(s),
                _ => Value::Text(String::new()),
            },
            None => Value::Text(String::new()),
        },
        "FIXED" | "DOLLAR" => {
            let value = match num_of(args.first().unwrap_or(&Value::Blank)) {
                Err(e) => return Some(e),
                Ok(n) => n,
            };
            let digits = match args.get(1) {
                // Both default to two decimals, as Excel does.
                None | Some(Value::Blank) => 2.0,
                Some(v) => match num_of(v) {
                    Err(e) => return Some(e),
                    Ok(n) => n.trunc(),
                },
            };
            let no_commas = name == "FIXED"
                && args
                    .get(2)
                    .map(|v| matches!(crate::values::to_boolean(v), Value::Bool(true)))
                    .unwrap_or(false);
            let pattern = match (no_commas, digits > 0.0) {
                (true, true) => format!("0.{}", "0".repeat(digits as usize)),
                (true, false) => "0".to_string(),
                (false, true) => format!("#,##0.{}", "0".repeat(digits as usize)),
                (false, false) => "#,##0".to_string(),
            };
            let rounded = crate::functions::round_half_away(value, digits);
            let body = crate::numfmt::apply_num_fmt(&Value::Number(rounded), &pattern);
            Value::Text(if name == "DOLLAR" {
                match rounded < 0.0 {
                    // Excel writes a negative currency in parentheses.
                    true => format!("(₩{})", body.trim_start_matches('-')),
                    false => format!("₩{body}"),
                }
            } else {
                body
            })
        }
        "NUMBERVALUE" => {
            let raw = text_at(args, 0);
            let decimal = text_at(args, 1);
            let group = text_at(args, 2);
            let mut cleaned = raw.replace(char::is_whitespace, "");
            if !group.is_empty() {
                cleaned = cleaned.replace(&group, "");
            } else {
                cleaned = cleaned.replace(',', "");
            }
            if !decimal.is_empty() && decimal != "." {
                cleaned = cleaned.replace(&decimal, ".");
            }
            match cleaned.parse::<f64>() {
                Ok(n) => Value::Number(n),
                Err(_) => err(VALUE_ERR),
            }
        }
        "TEXTBEFORE" | "TEXTAFTER" => {
            let source = text_at(args, 0);
            let sep = text_at(args, 1);
            if sep.is_empty() {
                return Some(err(VALUE_ERR));
            }
            let occurrence = match args.get(2) {
                None | Some(Value::Blank) => 1.0,
                Some(v) => match num_of(v) {
                    Err(e) => return Some(e),
                    Ok(n) => n.trunc(),
                },
            };
            let hits: Vec<usize> = source.match_indices(&sep).map(|(i, _)| i).collect();
            if hits.is_empty() {
                return Some(err(NA_ERR));
            }
            // A negative occurrence counts from the end, as Excel's does.
            let index = if occurrence < 0.0 {
                hits.len() as isize + occurrence as isize
            } else {
                occurrence as isize - 1
            };
            let Some(at) = usize::try_from(index).ok().and_then(|i| hits.get(i)) else {
                return Some(err(NA_ERR));
            };
            Value::Text(if name == "TEXTBEFORE" {
                source[..*at].to_string()
            } else {
                source[at + sep.len()..].to_string()
            })
        }
        _ => return None,
    })
}
