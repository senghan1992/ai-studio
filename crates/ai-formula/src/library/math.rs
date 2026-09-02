//! Arithmetic, rounding and trigonometry.

use crate::values::{
    err, first_error, num_of, numeric_values, to_number, Value, NUM_ERR, VALUE_ERR,
};

pub const NAMES: &[&str] = &[
    "ACOS",
    "ACOSH",
    "ASIN",
    "ASINH",
    "ATAN",
    "ATAN2",
    "ATANH",
    "BASE",
    "CEILING.MATH",
    "COMBIN",
    "COS",
    "COSH",
    "COT",
    "CSC",
    "DECIMAL",
    "DEGREES",
    "EVEN",
    "FACT",
    "FACTDOUBLE",
    "FLOOR.MATH",
    "GCD",
    "LCM",
    "LOG",
    "MROUND",
    "ODD",
    "PERMUT",
    "QUOTIENT",
    "RADIANS",
    "RANDBETWEEN",
    "SEC",
    "SIN",
    "SINH",
    "SQRTPI",
    "SUMSQ",
    "TAN",
    "TANH",
    "XOR",
];

/// One numeric argument in, one out — the shape most of this file has.
fn unary(args: &[Value], f: impl Fn(f64) -> f64) -> Value {
    match num_of(args.first().unwrap_or(&Value::Blank)) {
        Err(e) => e,
        Ok(x) => finite(f(x)),
    }
}

/// A result outside the reals is `#NUM!`, which is what Excel says for
/// `SQRT(-1)` and `ASIN(2)`.
pub fn finite(x: f64) -> Value {
    if x.is_finite() {
        Value::Number(x)
    } else {
        err(NUM_ERR)
    }
}

fn nums(args: &[Value], count: usize) -> Result<Vec<f64>, Value> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(num_of(args.get(i).unwrap_or(&Value::Blank))?);
    }
    Ok(out)
}

fn gcd2(a: f64, b: f64) -> f64 {
    let (mut a, mut b) = (a.abs().trunc(), b.abs().trunc());
    while b > 0.5 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if let Some(e) = first_error(args) {
        // `XOR` of an error is still an error, and so is every function here.
        if NAMES.contains(&name) {
            return Some(e);
        }
    }
    Some(match name {
        "SIN" => unary(args, f64::sin),
        "COS" => unary(args, f64::cos),
        "TAN" => unary(args, f64::tan),
        "ASIN" => unary(args, f64::asin),
        "ACOS" => unary(args, f64::acos),
        "ATAN" => unary(args, f64::atan),
        "SINH" => unary(args, f64::sinh),
        "COSH" => unary(args, f64::cosh),
        "TANH" => unary(args, f64::tanh),
        "ASINH" => unary(args, f64::asinh),
        "ACOSH" => unary(args, f64::acosh),
        "ATANH" => unary(args, f64::atanh),
        "SEC" => unary(args, |x| 1.0 / x.cos()),
        "CSC" => unary(args, |x| 1.0 / x.sin()),
        "COT" => unary(args, |x| 1.0 / x.tan()),
        "DEGREES" => unary(args, f64::to_degrees),
        "RADIANS" => unary(args, f64::to_radians),
        "SQRTPI" => unary(args, |x| (x * std::f64::consts::PI).sqrt()),
        "ATAN2" => match nums(args, 2) {
            // Excel takes x first, unlike every programming language.
            Ok(v) => finite(v[1].atan2(v[0])),
            Err(e) => e,
        },
        "LOG" => match args.len() {
            0 => err(VALUE_ERR),
            1 => unary(args, f64::log10),
            _ => match nums(args, 2) {
                Ok(v) if v[0] > 0.0 && v[1] > 0.0 && v[1] != 1.0 => finite(v[0].log(v[1])),
                Ok(_) => err(NUM_ERR),
                Err(e) => e,
            },
        },
        "EVEN" => unary(args, |x| {
            let n = (x.abs() / 2.0).ceil() * 2.0;
            if x < 0.0 {
                -n
            } else {
                n
            }
        }),
        "ODD" => unary(args, |x| {
            let mag = x.abs();
            let n = ((mag - 1.0) / 2.0).ceil() * 2.0 + 1.0;
            let n = if mag == 0.0 { 1.0 } else { n };
            if x < 0.0 {
                -n
            } else {
                n
            }
        }),
        "MROUND" => match nums(args, 2) {
            Ok(v) if v[1] == 0.0 => Value::Number(0.0),
            // Excel refuses a number and a multiple with different signs.
            Ok(v) if v[0].signum() != v[1].signum() && v[0] != 0.0 => err(NUM_ERR),
            Ok(v) => finite((v[0] / v[1]).round() * v[1]),
            Err(e) => e,
        },
        "CEILING.MATH" | "FLOOR.MATH" => {
            let up = name.starts_with("CEILING");
            match num_of(args.first().unwrap_or(&Value::Blank)) {
                Err(e) => e,
                Ok(x) => {
                    let step = match args.get(1) {
                        None | Some(Value::Blank) => 1.0,
                        Some(v) => match num_of(v) {
                            Err(e) => return Some(e),
                            Ok(n) => n,
                        },
                    };
                    // `mode` decides which way a negative number goes: away from
                    // zero when it is non-zero, toward zero by default.
                    let away = match args.get(2) {
                        None | Some(Value::Blank) => false,
                        Some(v) => match num_of(v) {
                            Err(e) => return Some(e),
                            Ok(n) => n != 0.0,
                        },
                    };
                    if step == 0.0 {
                        return Some(Value::Number(0.0));
                    }
                    let q = x / step.abs();
                    let rounded = if up != (x < 0.0 && away) {
                        q.ceil()
                    } else {
                        q.floor()
                    };
                    finite(rounded * step.abs())
                }
            }
        }
        "QUOTIENT" => match nums(args, 2) {
            Ok(v) if v[1] == 0.0 => err(crate::values::DIV0_ERR),
            Ok(v) => finite((v[0] / v[1]).trunc()),
            Err(e) => e,
        },
        "FACT" | "FACTDOUBLE" => match num_of(args.first().unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(x) if x < 0.0 || x > 170.0 => err(NUM_ERR),
            Ok(x) => {
                let n = x.trunc() as u64;
                let step = if name == "FACT" { 1 } else { 2 };
                let mut acc = 1.0f64;
                let mut i = n;
                while i > 1 {
                    acc *= i as f64;
                    i = i.saturating_sub(step);
                }
                finite(acc)
            }
        },
        "COMBIN" | "PERMUT" => match nums(args, 2) {
            Err(e) => e,
            Ok(v) => {
                let (n, k) = (v[0].trunc(), v[1].trunc());
                if n < 0.0 || k < 0.0 || k > n {
                    return Some(err(NUM_ERR));
                }
                // Multiplicative form: 170! overflows, but C(1000, 3) does not.
                let mut acc = 1.0f64;
                for i in 0..(k as u64) {
                    acc *= n - i as f64;
                    if name == "COMBIN" {
                        acc /= i as f64 + 1.0;
                    }
                }
                finite(acc.round())
            }
        },
        "GCD" | "LCM" => match numeric_values(args) {
            Err(e) => e,
            Ok(v) if v.is_empty() => Value::Number(0.0),
            Ok(v) => {
                if v.iter().any(|x| *x < 0.0) {
                    return Some(err(NUM_ERR));
                }
                let mut acc = v[0].trunc();
                for x in &v[1..] {
                    let x = x.trunc();
                    acc = if name == "GCD" {
                        gcd2(acc, x)
                    } else if acc == 0.0 || x == 0.0 {
                        0.0
                    } else {
                        (acc / gcd2(acc, x)) * x
                    };
                }
                finite(acc)
            }
        },
        "SUMSQ" => match numeric_values(args) {
            Ok(v) => Value::Number(v.iter().map(|x| x * x).sum()),
            Err(e) => e,
        },
        "RANDBETWEEN" => match nums(args, 2) {
            Err(e) => e,
            Ok(v) => {
                let (lo, hi) = (v[0].ceil(), v[1].floor());
                if lo > hi {
                    return Some(err(NUM_ERR));
                }
                let span = hi - lo + 1.0;
                Value::Number(lo + (crate::functions::random() * span).floor().min(span - 1.0))
            }
        },
        "BASE" => match nums(args, 2) {
            Err(e) => e,
            Ok(v) if !(2.0..=36.0).contains(&v[1]) || v[0] < 0.0 => err(NUM_ERR),
            Ok(v) => {
                let pad = args
                    .get(2)
                    .map(|a| num_of(a).unwrap_or(0.0).max(0.0) as usize)
                    .unwrap_or(0);
                Value::Text(to_base(v[0].trunc() as u64, v[1] as u32, pad))
            }
        },
        "DECIMAL" => {
            let text = crate::values::to_text(args.first().unwrap_or(&Value::Blank));
            match num_of(args.get(1).unwrap_or(&Value::Blank)) {
                Err(e) => e,
                Ok(radix) if !(2.0..=36.0).contains(&radix) => err(NUM_ERR),
                Ok(radix) => match u64::from_str_radix(text.trim(), radix as u32) {
                    Ok(n) => Value::Number(n as f64),
                    Err(_) => err(NUM_ERR),
                },
            }
        }
        "XOR" => {
            let mut count = 0usize;
            for v in crate::values::flatten(args) {
                // Text that is not a boolean is skipped, as in Excel's own XOR.
                match crate::values::to_boolean(&v) {
                    Value::Bool(true) => count += 1,
                    Value::Bool(false) => {}
                    Value::Error(_) => {}
                    _ => {}
                }
                if let Value::Error(c) = to_number(&v) {
                    let _ = c;
                }
            }
            Value::Bool(count % 2 == 1)
        }
        _ => return None,
    })
}

fn to_base(mut n: u64, radix: u32, pad: usize) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if n == 0 {
        return "0".repeat(pad.max(1));
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % radix as u64) as usize]);
        n /= radix as u64;
    }
    out.reverse();
    let text = String::from_utf8(out).unwrap_or_default();
    if text.len() >= pad {
        text
    } else {
        format!("{}{text}", "0".repeat(pad - text.len()))
    }
}
