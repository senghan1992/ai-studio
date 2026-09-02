//! The built-in function library.

use std::cmp::Ordering;
use std::rc::Rc;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::numfmt::{apply_num_fmt, date_arg, date_serial, serial_to_ymd};
use crate::values::{
    bool_of, compare_values, err, first_error, flatten, num_of, numeric_values, to_boolean,
    to_number, to_text, RangeValue, Value, DIV0_ERR, NA_ERR, NUM_ERR, REF_ERR, VALUE_ERR,
};

/// Every function name, sorted — what the formula bar autocompletes from.
pub static FUNCTION_NAMES: Lazy<Vec<&'static str>> = Lazy::new(|| {
    let mut names = CORE_NAMES.to_vec();
    // The families in `crate::library` — statistics, finance, dates, the newer
    // lookups — register themselves rather than being listed twice.
    names.extend(crate::library::names());
    names.sort_unstable();
    names.dedup();
    names
});

/// The functions this file answers to. `crate::library` holds the rest.
static CORE_NAMES: &[&str] = &[
    "ABS",
    "AND",
    "AVERAGE",
    "AVERAGEIF",
    "CEILING",
    "COLUMNS",
    "CONCAT",
    "CONCATENATE",
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "COUNTIF",
    "DATE",
    "DAY",
    "DAYS",
    "EXP",
    "FALSE",
    "FIND",
    "FLOOR",
    "HLOOKUP",
    "IF",
    "IFERROR",
    "IFNA",
    "IFS",
    "INDEX",
    "INT",
    "ISBLANK",
    "ISERROR",
    "ISNUMBER",
    "ISTEXT",
    "LEFT",
    "LEN",
    "LN",
    "LOG10",
    "LOWER",
    "MATCH",
    "MAX",
    "MEDIAN",
    "MID",
    "MIN",
    "MOD",
    "MONTH",
    "NA",
    "NOT",
    "NOW",
    "OR",
    "PI",
    "POWER",
    "PRODUCT",
    "PROPER",
    "RAND",
    "RIGHT",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "ROWS",
    "SIGN",
    "SQRT",
    "STDEV",
    "SUBSTITUTE",
    "SUM",
    "SUMIF",
    "SUMPRODUCT",
    "TEXT",
    "TEXTJOIN",
    "TODAY",
    "TRIM",
    "TRUE",
    "TRUNC",
    "UPPER",
    "VALUE",
    "VLOOKUP",
    "WEEKDAY",
    "YEAR",
];

pub fn is_function(name: &str) -> bool {
    FUNCTION_NAMES.binary_search(&name).is_ok()
}

/// The function names a formula calls, uppercased and in order of appearance.
///
/// Lexed rather than pattern-matched, so a cell containing the text `"SUM(x)"`
/// is not mistaken for a call.
pub fn called_functions(formula: &str) -> Vec<String> {
    use crate::parse::{tokenize, Token};
    let Ok(tokens) = tokenize(formula.trim_start_matches('=')) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for pair in tokens.windows(2) {
        if let (Token::Name(name), Token::LParen) = (&pair[0], &pair[1]) {
            let upper = name.to_uppercase();
            if !out.contains(&upper) {
                out.push(upper);
            }
        }
    }
    out
}

/// The functions in a formula that this build cannot evaluate.
pub fn unsupported_functions(formula: &str) -> Vec<String> {
    called_functions(formula)
        .into_iter()
        .filter(|name| !is_function(name))
        .collect()
}

/* --------------------------------------------------------- criteria match */

/// Parse a `SUMIF`/`COUNTIF` criterion: `42`, `">10"`, `"<>x"`, `"seoul"`, `"a*"`.
pub fn make_criteria(raw: &Value) -> Box<dyn Fn(&Value) -> bool> {
    let value = raw.scalar();
    if matches!(value, Value::Number(_) | Value::Bool(_)) {
        return Box::new(move |v| compare_values(v, &value) == Ordering::Equal);
    }
    let s = to_text(&value);

    static OP_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^(<=|>=|<>|<|>|=)\s*([\s\S]*)$").unwrap());
    if let Some(caps) = OP_RE.captures(&s) {
        let op = caps.get(1).unwrap().as_str().to_string();
        let rest = caps.get(2).unwrap().as_str();
        let target = criteria_target(rest, true);
        return Box::new(move |v| {
            let c = compare_values(v, &target);
            match op.as_str() {
                ">" => c == Ordering::Greater,
                "<" => c == Ordering::Less,
                ">=" => c != Ordering::Less,
                "<=" => c != Ordering::Greater,
                "<>" => c != Ordering::Equal,
                _ => c == Ordering::Equal,
            }
        });
    }

    if s.contains('*') || s.contains('?') {
        let pattern = wildcard_to_regex(&s);
        return match Regex::new(&pattern) {
            Ok(re) => Box::new(move |v| re.is_match(&to_text(v))),
            Err(_) => Box::new(|_| false),
        };
    }

    let target = criteria_target(&s, false);
    Box::new(move |v| compare_values(v, &target) == Ordering::Equal)
}

/// A criterion's right-hand side, as a number when it parses as one.
fn criteria_target(rest: &str, blank_when_empty: bool) -> Value {
    if rest.is_empty() && blank_when_empty {
        return Value::Blank;
    }
    if rest.trim().is_empty() {
        return Value::Text(rest.to_string());
    }
    match to_number(&Value::Text(rest.to_string())) {
        // `to_number` treats "abc" as an error and "12%" as 0.12; only a plain
        // numeric literal should become a number here, matching `Number(rest)`.
        Value::Number(n) if rest.trim().parse::<f64>() == Ok(n) => Value::Number(n),
        _ => Value::Text(rest.to_string()),
    }
}

fn wildcard_to_regex(s: &str) -> String {
    let mut out = String::from("(?i)^");
    for c in s.chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c if ".+^$(){}|[]\\".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('$');
    out
}

/// The values in `range` (or the parallel `sum_range`) whose key passes `criteria`.
fn paired_if(
    range_arg: &Value,
    criteria_arg: &Value,
    sum_arg: Option<&Value>,
) -> Result<Vec<Value>, Value> {
    let Value::Range(range) = range_arg else {
        return Err(err(VALUE_ERR));
    };
    let test = make_criteria(criteria_arg);
    let target: &RangeValue = match sum_arg {
        Some(Value::Range(r)) => r,
        _ => range,
    };
    let mut picked = Vec::new();
    for (i, v) in range.values().enumerate() {
        if test(v) {
            picked.push(
                target
                    .cells
                    .get(i)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Blank),
            );
        }
    }
    Ok(picked)
}

/* --------------------------------------------------------------- lookups */

fn lookup_in(
    needle: &Value,
    haystack: &Value,
    result_index: &Value,
    approximate: bool,
    horizontal: bool,
) -> Value {
    let Value::Range(range) = haystack else {
        return err(VALUE_ERR);
    };
    let grid = range.grid();
    let idx = match num_of(result_index) {
        Ok(n) if n.is_finite() => n.trunc() as i64,
        Ok(_) => return err(VALUE_ERR),
        Err(e) => return e,
    };

    let keys: Vec<Value> = if horizontal {
        grid.first().cloned().unwrap_or_default()
    } else {
        grid.iter()
            .map(|row| row.first().cloned().unwrap_or(Value::Blank))
            .collect()
    };
    let bound = if horizontal {
        grid.len()
    } else {
        grid.first().map(|r| r.len()).unwrap_or(0)
    } as i64;
    if idx < 1 || idx > bound {
        return err(REF_ERR);
    }

    let mut found: i64 = -1;
    if approximate {
        for (i, k) in keys.iter().enumerate() {
            if compare_values(k, needle) != Ordering::Greater {
                found = i as i64;
            } else {
                break;
            }
        }
    } else {
        let test = make_criteria(needle);
        found = keys.iter().position(&*test).map(|i| i as i64).unwrap_or(-1);
    }
    if found < 0 {
        return err(NA_ERR);
    }
    let (r, c) = if horizontal {
        ((idx - 1) as usize, found as usize)
    } else {
        (found as usize, (idx - 1) as usize)
    };
    grid.get(r)
        .and_then(|row| row.get(c))
        .cloned()
        .unwrap_or(Value::Blank)
}

/* ---------------------------------------------------------- number helpers */

#[derive(Clone, Copy)]
enum RoundMode {
    Half,
    Up,
    Down,
}

/// `ROUND`'s rounding, for the parts of the library that format numbers.
pub fn round_half_away(value: f64, digits: f64) -> f64 {
    round_to(value, digits, RoundMode::Half)
}

fn round_to(value: f64, digits: f64, mode: RoundMode) -> f64 {
    let d = digits.trunc();
    let f = 10f64.powf(d);
    let x = value * f;
    let mag = x.abs();
    let r = match mode {
        RoundMode::Up => mag.ceil(),
        RoundMode::Down => mag.floor(),
        // The epsilon nudge is the JS original's, and it is what makes
        // ROUND(1.005, 2) give 1.01 instead of 1.
        RoundMode::Half => (mag + f64::EPSILON * mag).round(),
    };
    x.signum() * r / f
}

fn num_at(args: &[Value], i: usize) -> Result<f64, Value> {
    num_of(args.get(i).unwrap_or(&Value::Blank))
}

fn text_at(args: &[Value], i: usize) -> String {
    to_text(args.get(i).unwrap_or(&Value::Blank))
}

/* ---------------------------------------------------------------- dispatch */

/// Call a built-in. `None` means there is no such function (`#NAME?`).
pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !is_function(name) {
        return None;
    }
    // The core dispatch first, so a name in both places resolves here.
    if let Some(value) = dispatch_core(name, args) {
        return Some(value);
    }
    crate::library::call(name, args)
}

/// Short-circuit on the first error anywhere in the arguments — the `guard`
/// wrapper in the original.
fn guarded(args: &[Value], f: impl FnOnce(&[Value]) -> Value) -> Value {
    match first_error(args) {
        Some(e) => e,
        None => f(args),
    }
}

fn sum_of(nums: Vec<f64>) -> f64 {
    nums.iter().sum()
}

/// The core library. `None` hands the name on to [`crate::library`].
fn dispatch_core(name: &str, args: &[Value]) -> Option<Value> {
    CORE_NAMES.contains(&name).then(|| dispatch(name, args))
}

fn dispatch(name: &str, args: &[Value]) -> Value {
    match name {
        /* ------------------------------------------ math & aggregation */
        "SUM" => guarded(args, |a| match numeric_values(a) {
            Ok(n) => Value::Number(sum_of(n)),
            Err(e) => e,
        }),
        "PRODUCT" => guarded(args, |a| match numeric_values(a) {
            Ok(n) if n.is_empty() => Value::Number(0.0),
            Ok(n) => Value::Number(n.iter().product()),
            Err(e) => e,
        }),
        "AVERAGE" => guarded(args, |a| match numeric_values(a) {
            Ok(n) if n.is_empty() => err(DIV0_ERR),
            Ok(n) => Value::Number(sum_of(n.clone()) / n.len() as f64),
            Err(e) => e,
        }),
        "MEDIAN" => guarded(args, |a| match numeric_values(a) {
            Ok(n) if n.is_empty() => err(NUM_ERR),
            Ok(mut n) => {
                n.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
                let mid = n.len() / 2;
                Value::Number(if n.len() % 2 == 1 {
                    n[mid]
                } else {
                    (n[mid - 1] + n[mid]) / 2.0
                })
            }
            Err(e) => e,
        }),
        "MIN" => guarded(args, |a| match numeric_values(a) {
            Ok(n) => Value::Number(n.into_iter().fold(f64::INFINITY, f64::min)).finite_or(0.0),
            Err(e) => e,
        }),
        "MAX" => guarded(args, |a| match numeric_values(a) {
            Ok(n) => Value::Number(n.into_iter().fold(f64::NEG_INFINITY, f64::max)).finite_or(0.0),
            Err(e) => e,
        }),
        "COUNT" => Value::Number(numeric_values(args).map(|n| n.len()).unwrap_or(0) as f64),
        "COUNTA" => Value::Number(flatten(args).iter().filter(|v| !v.is_blankish()).count() as f64),
        "COUNTBLANK" => {
            Value::Number(flatten(args).iter().filter(|v| v.is_blankish()).count() as f64)
        }
        "ABS" => guarded(args, |a| wrap1(a, f64::abs)),
        "SQRT" => guarded(args, |a| match num_at(a, 0) {
            Ok(x) if x < 0.0 => err(NUM_ERR),
            Ok(x) => Value::Number(x.sqrt()),
            Err(e) => e,
        }),
        "POWER" => guarded(args, |a| bin(a, |x, y| Value::Number(x.powf(y)))),
        "MOD" => guarded(args, |a| {
            bin(a, |x, y| {
                if y == 0.0 {
                    err(DIV0_ERR)
                } else {
                    Value::Number(x - y * (x / y).floor())
                }
            })
        }),
        "INT" => guarded(args, |a| wrap1(a, f64::floor)),
        "TRUNC" => guarded(args, |a| round_arg(a, RoundMode::Down)),
        "ROUND" => guarded(args, |a| round_arg(a, RoundMode::Half)),
        "ROUNDUP" => guarded(args, |a| round_arg(a, RoundMode::Up)),
        "ROUNDDOWN" => guarded(args, |a| round_arg(a, RoundMode::Down)),
        "CEILING" => guarded(args, |a| {
            bin(a, |x, y| {
                if y == 0.0 {
                    Value::Number(0.0)
                } else {
                    Value::Number((x / y).ceil() * y)
                }
            })
        }),
        "FLOOR" => guarded(args, |a| {
            bin(a, |x, y| {
                if y == 0.0 {
                    err(DIV0_ERR)
                } else {
                    Value::Number((x / y).floor() * y)
                }
            })
        }),
        // Math.sign, which is 0 for both zeros and never NaN for finite input.
        "SIGN" => guarded(args, |a| {
            wrap1(a, |x| {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            })
        }),
        "EXP" => guarded(args, |a| wrap1(a, f64::exp)),
        "LN" => guarded(args, |a| match num_at(a, 0) {
            Ok(x) if x <= 0.0 => err(NUM_ERR),
            Ok(x) => Value::Number(x.ln()),
            Err(e) => e,
        }),
        "LOG10" => guarded(args, |a| match num_at(a, 0) {
            Ok(x) if x <= 0.0 => err(NUM_ERR),
            Ok(x) => Value::Number(x.log10()),
            Err(e) => e,
        }),
        "RAND" => Value::Number(pseudo_random()),
        "PI" => Value::Number(std::f64::consts::PI),
        "SUMPRODUCT" => guarded(args, |a| {
            let arrays: Vec<Vec<Value>> = a
                .iter()
                .map(|v| match v {
                    Value::Range(r) => r.values().cloned().collect(),
                    // An array counts as a column too: `(A:A="서울")*B:B` is how
                    // conditional sums were written before SUMIFS existed.
                    Value::Array(items) => items.clone(),
                    other => vec![other.clone()],
                })
                .collect();
            let len = arrays.iter().map(|v| v.len()).max().unwrap_or(0);
            let mut total = 0.0;
            for i in 0..len {
                let mut p = 1.0;
                for arr in &arrays {
                    let v = arr.get(i).cloned().unwrap_or(Value::Number(0.0));
                    match num_of(&v) {
                        Ok(n) => p *= n,
                        Err(e) => return e,
                    }
                }
                total += p;
            }
            Value::Number(total)
        }),
        "STDEV" => guarded(args, |a| match numeric_values(a) {
            Ok(n) if n.len() < 2 => err(DIV0_ERR),
            Ok(n) => {
                let mean = sum_of(n.clone()) / n.len() as f64;
                let var: f64 =
                    n.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n.len() - 1) as f64;
                Value::Number(var.sqrt())
            }
            Err(e) => e,
        }),

        /* --------------------------------------- conditional aggregation */
        "SUMIF" => match paired_if(
            args.first().unwrap_or(&Value::Blank),
            args.get(1).unwrap_or(&Value::Blank),
            args.get(2),
        ) {
            Err(e) => e,
            Ok(picked) => match numeric_values(&[Value::Array(picked)]) {
                Ok(n) => Value::Number(sum_of(n)),
                Err(e) => e,
            },
        },
        "COUNTIF" => match args.first() {
            Some(Value::Range(r)) => {
                let test = make_criteria(args.get(1).unwrap_or(&Value::Blank));
                Value::Number(r.values().filter(|v| test(v)).count() as f64)
            }
            _ => err(VALUE_ERR),
        },
        "AVERAGEIF" => match paired_if(
            args.first().unwrap_or(&Value::Blank),
            args.get(1).unwrap_or(&Value::Blank),
            args.get(2),
        ) {
            Err(e) => e,
            Ok(picked) => match numeric_values(&[Value::Array(picked)]) {
                Ok(n) if n.is_empty() => err(DIV0_ERR),
                Ok(n) => Value::Number(sum_of(n.clone()) / n.len() as f64),
                Err(e) => e,
            },
        },

        /* ------------------------------------------------------- logic */
        "IF" => match bool_of(args.first().unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(cond) => {
                let branch = if cond { args.get(1) } else { args.get(2) };
                match branch {
                    None => Value::Bool(cond),
                    Some(v) => v.scalar(),
                }
            }
        },
        "IFERROR" => {
            let first = args.first().unwrap_or(&Value::Blank);
            if first.is_err() || first.scalar().is_err() {
                args.get(1).cloned().unwrap_or(Value::Blank)
            } else {
                first.clone()
            }
        }
        "IFNA" => {
            let first = args.first().unwrap_or(&Value::Blank);
            if first.err_code() == Some(NA_ERR) {
                args.get(1).cloned().unwrap_or(Value::Blank)
            } else {
                first.clone()
            }
        }
        "IFS" => {
            let mut i = 0;
            while i + 1 < args.len() {
                match bool_of(&args[i]) {
                    Err(e) => return e,
                    Ok(true) => return args[i + 1].clone(),
                    Ok(false) => {}
                }
                i += 2;
            }
            err(NA_ERR)
        }
        "AND" => guarded(args, |a| {
            for v in flatten(a).iter().filter(|v| !v.is_blankish()) {
                match bool_of(v) {
                    Err(e) => return e,
                    Ok(false) => return Value::Bool(false),
                    Ok(true) => {}
                }
            }
            Value::Bool(true)
        }),
        "OR" => guarded(args, |a| {
            for v in flatten(a).iter().filter(|v| !v.is_blankish()) {
                match bool_of(v) {
                    Err(e) => return e,
                    Ok(true) => return Value::Bool(true),
                    Ok(false) => {}
                }
            }
            Value::Bool(false)
        }),
        "NOT" => guarded(args, |a| {
            match bool_of(a.first().unwrap_or(&Value::Blank)) {
                Ok(b) => Value::Bool(!b),
                Err(e) => e,
            }
        }),
        "TRUE" => Value::Bool(true),
        "FALSE" => Value::Bool(false),
        "ISBLANK" => Value::Bool(args.first().map(|v| v.is_blankish()).unwrap_or(true)),
        "ISNUMBER" => Value::Bool(matches!(
            args.first().unwrap_or(&Value::Blank).scalar(),
            Value::Number(_)
        )),
        "ISTEXT" => Value::Bool(matches!(
            args.first().unwrap_or(&Value::Blank).scalar(),
            Value::Text(_)
        )),
        "ISERROR" => {
            let v = args.first().unwrap_or(&Value::Blank);
            Value::Bool(v.is_err() || (v.is_range() && v.scalar().is_err()))
        }
        "NA" => err(NA_ERR),

        /* -------------------------------------------------------- text */
        "CONCAT" | "CONCATENATE" => Value::Text(
            flatten(args)
                .iter()
                .map(to_text)
                .collect::<Vec<_>>()
                .join(""),
        ),
        "TEXTJOIN" => {
            let sep = text_at(args, 0);
            let skip_empty = matches!(
                to_boolean(args.get(1).unwrap_or(&Value::Blank)),
                Value::Bool(true)
            );
            let rest = if args.len() > 2 { &args[2..] } else { &[] };
            let vals: Vec<String> = flatten(rest)
                .into_iter()
                .filter(|v| !skip_empty || !v.is_blankish())
                .map(|v| to_text(&v))
                .collect();
            Value::Text(vals.join(&sep))
        }
        "LEN" => guarded(
            args,
            |a| Value::Number(text_at(a, 0).chars().count() as f64),
        ),
        "LEFT" => guarded(args, |a| slice_text(a, true)),
        "RIGHT" => guarded(args, |a| slice_text(a, false)),
        "MID" => guarded(args, |a| {
            let s: Vec<char> = text_at(a, 0).chars().collect();
            let start = match num_at(a, 1) {
                Ok(n) => n,
                Err(e) => return e,
            };
            let len = match num_at(a, 2) {
                Ok(n) => n,
                Err(e) => return e,
            };
            if start < 1.0 || len < 0.0 {
                return err(VALUE_ERR);
            }
            let from = (start as usize).saturating_sub(1).min(s.len());
            let to = (from + len.trunc() as usize).min(s.len());
            Value::Text(s[from..to].iter().collect())
        }),
        "UPPER" => guarded(args, |a| Value::Text(text_at(a, 0).to_uppercase())),
        "LOWER" => guarded(args, |a| Value::Text(text_at(a, 0).to_lowercase())),
        "PROPER" => guarded(args, |a| {
            let text = text_at(a, 0);
            let out: Vec<String> = text
                .split_inclusive(char::is_whitespace)
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => {
                            first.to_uppercase().collect::<String>()
                                + &chars.as_str().to_lowercase()
                        }
                    }
                })
                .collect();
            Value::Text(out.join(""))
        }),
        "TRIM" => guarded(args, |a| {
            let text = text_at(a, 0);
            Value::Text(text.split_whitespace().collect::<Vec<_>>().join(" "))
        }),
        "SUBSTITUTE" => guarded(args, |a| {
            let s = text_at(a, 0);
            let find = text_at(a, 1);
            let repl = text_at(a, 2);
            if find.is_empty() {
                return Value::Text(s);
            }
            match a.get(3) {
                None => Value::Text(s.replace(&find, &repl)),
                Some(nth_arg) => {
                    let nth = match num_of(nth_arg) {
                        Ok(n) => n.trunc() as i64,
                        Err(e) => return e,
                    };
                    let mut count = 0i64;
                    let mut from = 0usize;
                    while let Some(at) = s[from..].find(&find) {
                        let at = from + at;
                        count += 1;
                        if count == nth {
                            return Value::Text(format!(
                                "{}{repl}{}",
                                &s[..at],
                                &s[at + find.len()..]
                            ));
                        }
                        from = at + find.len().max(1);
                    }
                    Value::Text(s)
                }
            }
        }),
        "FIND" => guarded(args, |a| {
            let needle = text_at(a, 0);
            let haystack = text_at(a, 1);
            let start = match a.get(2) {
                None => 1.0,
                Some(v) => match num_of(v) {
                    Ok(n) => n,
                    Err(e) => return e,
                },
            };
            let skip = (start.trunc() as i64 - 1).max(0) as usize;
            // FIND is 1-based in characters, not bytes.
            let chars: Vec<char> = haystack.chars().collect();
            if skip > chars.len() {
                return err(VALUE_ERR);
            }
            let tail: String = chars[skip..].iter().collect();
            match tail.find(&needle) {
                None => err(VALUE_ERR),
                Some(byte_at) => {
                    let char_at = tail[..byte_at].chars().count();
                    Value::Number((skip + char_at + 1) as f64)
                }
            }
        }),
        "VALUE" => guarded(args, |a| to_number(a.first().unwrap_or(&Value::Blank))),
        "TEXT" => guarded(args, |a| {
            let v = a.first().unwrap_or(&Value::Blank).scalar();
            Value::Text(apply_num_fmt(&v, &text_at(a, 1)))
        }),

        /* -------------------------------------------- lookup & reference */
        "VLOOKUP" | "HLOOKUP" => {
            let approximate = match args.get(3) {
                None => false,
                Some(v) => matches!(to_boolean(v), Value::Bool(true)),
            };
            lookup_in(
                args.first().unwrap_or(&Value::Blank),
                args.get(1).unwrap_or(&Value::Blank),
                args.get(2).unwrap_or(&Value::Blank),
                approximate,
                name == "HLOOKUP",
            )
        }
        "INDEX" => {
            // An array works as a one-column range, so `INDEX(SORT(...), 1)`
            // means what it looks like.
            let Some(range) = args.first().and_then(crate::values::as_range) else {
                return err(VALUE_ERR);
            };
            let grid = range.grid();
            let r = match args.get(1) {
                None => Ok(1.0),
                Some(v) => num_of(v),
            };
            let c = match args.get(2) {
                None => Ok(1.0),
                Some(v) => num_of(v),
            };
            let (Ok(r), Ok(c)) = (r, c) else {
                return err(VALUE_ERR);
            };
            let (r, c) = (r.trunc() as i64, c.trunc() as i64);
            let pick = |row: i64, col: i64| -> Option<Value> {
                let row = usize::try_from(row - 1).ok()?;
                let col = usize::try_from(col - 1).ok()?;
                grid.get(row)?.get(col).cloned()
            };
            // A single-row or single-column range accepts one index.
            if args.get(2).is_none() && grid.len() == 1 {
                return pick(1, r).unwrap_or_else(|| err(REF_ERR));
            }
            if args.get(2).is_none() && grid.first().map(|r| r.len()) == Some(1) {
                return pick(r, 1).unwrap_or_else(|| err(REF_ERR));
            }
            pick(r, c).unwrap_or_else(|| err(REF_ERR))
        }
        "MATCH" => {
            let Some(Value::Range(range)) = args.get(1) else {
                return err(VALUE_ERR);
            };
            let values: Vec<&Value> = range.values().collect();
            let needle = args.first().unwrap_or(&Value::Blank);
            let match_type = match args.get(2) {
                None => 1i64,
                Some(v) => match num_of(v) {
                    Ok(n) => n.trunc() as i64,
                    Err(e) => return e,
                },
            };
            if match_type == 0 {
                let test = make_criteria(needle);
                return match values.iter().position(|v| test(v)) {
                    Some(i) => Value::Number((i + 1) as f64),
                    None => err(NA_ERR),
                };
            }
            let mut best: i64 = -1;
            for (i, v) in values.iter().enumerate() {
                let c = compare_values(v, needle);
                if match_type == 1 && c != Ordering::Greater {
                    best = i as i64;
                }
                if match_type == -1 && c != Ordering::Less {
                    best = i as i64;
                }
            }
            if best < 0 {
                err(NA_ERR)
            } else {
                Value::Number((best + 1) as f64)
            }
        }
        "ROWS" => match args.first() {
            Some(Value::Range(r)) => Value::Number(r.rows as f64),
            _ => Value::Number(1.0),
        },
        "COLUMNS" => match args.first() {
            Some(Value::Range(r)) => Value::Number(r.cols as f64),
            _ => Value::Number(1.0),
        },

        /* --------------------------------------------------- date & time */
        "TODAY" => Value::Number(today_serial()),
        "NOW" => Value::Number(now_serial()),
        "DATE" => guarded(args, |a| {
            let (Ok(y), Ok(m), Ok(d)) = (num_at(a, 0), num_at(a, 1), num_at(a, 2)) else {
                return err(VALUE_ERR);
            };
            match date_serial(y.trunc() as i64, m.trunc() as i64, d.trunc() as i64) {
                Some(s) => Value::Number(s),
                None => err(NUM_ERR),
            }
        }),
        "YEAR" => guarded(args, |a| date_part(a, |y, _, _, _| y as f64)),
        "MONTH" => guarded(args, |a| date_part(a, |_, m, _, _| m as f64)),
        "DAY" => guarded(args, |a| date_part(a, |_, _, d, _| d as f64)),
        "WEEKDAY" => guarded(args, |a| date_part(a, |_, _, _, w| w as f64)),
        "DAYS" => guarded(args, |a| {
            let a0 = date_arg(a.first().unwrap_or(&Value::Blank));
            let a1 = date_arg(a.get(1).unwrap_or(&Value::Blank));
            match (a0, a1) {
                (Err(e), _) | (_, Err(e)) => e,
                (Ok(x), Ok(y)) => Value::Number(x - y),
            }
        }),

        _ => err(crate::values::NAME_ERR),
    }
}

trait FiniteOr {
    fn finite_or(self, fallback: f64) -> Value;
}

impl FiniteOr for Value {
    /// `Math.min()` of nothing is Infinity in JS but the original returned 0
    /// for an empty set, so collapse the sentinel here.
    fn finite_or(self, fallback: f64) -> Value {
        match self {
            Value::Number(n) if !n.is_finite() => Value::Number(fallback),
            other => other,
        }
    }
}

fn wrap1(args: &[Value], f: impl Fn(f64) -> f64) -> Value {
    match num_at(args, 0) {
        Ok(x) => Value::Number(f(x)),
        Err(e) => e,
    }
}

fn bin(args: &[Value], f: impl Fn(f64, f64) -> Value) -> Value {
    match (num_at(args, 0), num_at(args, 1)) {
        (Err(e), _) => e,
        (_, Err(e)) => e,
        (Ok(x), Ok(y)) => f(x, y),
    }
}

fn round_arg(args: &[Value], mode: RoundMode) -> Value {
    let x = match num_at(args, 0) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let d = if args.len() > 1 {
        match num_at(args, 1) {
            Ok(n) => n,
            Err(e) => return e,
        }
    } else {
        0.0
    };
    Value::Number(round_to(x, d, mode))
}

fn slice_text(args: &[Value], from_left: bool) -> Value {
    let s: Vec<char> = text_at(args, 0).chars().collect();
    let count = if args.len() > 1 {
        match num_at(args, 1) {
            Ok(n) => n,
            Err(e) => return e,
        }
    } else {
        1.0
    };
    let c = count.trunc();
    if c < 0.0 {
        return err(VALUE_ERR);
    }
    let c = (c as usize).min(s.len());
    let picked: String = if from_left {
        s[..c].iter().collect()
    } else if c == 0 {
        String::new()
    } else {
        s[s.len() - c..].iter().collect()
    };
    Value::Text(picked)
}

fn date_part(args: &[Value], pick: impl Fn(i32, u32, u32, u32) -> f64) -> Value {
    let serial = match date_arg(args.first().unwrap_or(&Value::Blank)) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let Some((y, m, d)) = serial_to_ymd(serial) else {
        return err(NUM_ERR);
    };
    // JS getUTCDay() is 0 for Sunday; WEEKDAY adds one.
    use chrono::{Datelike, NaiveDate};
    let weekday = NaiveDate::from_ymd_opt(y, m, d)
        .map(|dt| dt.weekday().num_days_from_sunday() + 1)
        .unwrap_or(0);
    Value::Number(pick(y, m, d, weekday))
}

/// Today's date as a serial.
///
/// The JS original wrote `toSerial(new Date())`, and `toSerial` rounds — so
/// after noon UTC its `TODAY()` returned tomorrow. Truncating is what the
/// function is supposed to mean.
fn today_serial() -> f64 {
    now_serial().floor()
}

fn now_serial() -> f64 {
    use chrono::Utc;
    // (Date.now() - Date.UTC(1899, 11, 30)) / DAY_MS
    (Utc::now().timestamp_millis() as f64 + 2_209_161_600_000.0) / 86_400_000.0
}

/// `Math.random()`. A tiny xorshift keeps the crate dependency-free; RAND is
/// not used for anything that needs cryptographic quality.
/// A uniform number in `[0, 1)`, shared with the rest of the library.
pub fn random() -> f64 {
    pseudo_random()
}

fn pseudo_random() -> f64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = const { Cell::new(0) };
    }
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            // `SystemTime::now()` aborts on wasm32-unknown-unknown.
            x = chrono::Utc::now()
                .timestamp_nanos_opt()
                .map(|n| n as u64)
                .unwrap_or(0x2545F4914F6CDD1D)
                | 1;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Build a range value from a list of (ref, value) pairs — used by the
/// evaluator and by tests.
pub fn range_value(cells: Vec<(String, Value)>, rows: usize, cols: usize) -> Value {
    Value::Range(Rc::new(RangeValue { cells, rows, cols }))
}
