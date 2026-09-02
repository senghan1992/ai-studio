//! Statistics, and the multi-criteria aggregates.

use std::cmp::Ordering;

use crate::functions::make_criteria;
use crate::values::{
    err, first_error, flatten, num_of, numeric_values, to_number, Value, DIV0_ERR, NA_ERR, NUM_ERR,
    VALUE_ERR,
};

pub const NAMES: &[&str] = &[
    "AGGREGATE",
    "AVEDEV",
    "AVERAGEIFS",
    "CORREL",
    "COUNTIFS",
    "DEVSQ",
    "FORECAST",
    "FORECAST.LINEAR",
    "GEOMEAN",
    "HARMEAN",
    "INTERCEPT",
    "LARGE",
    "MAXIFS",
    "MINIFS",
    "MODE",
    "MODE.SNGL",
    "PEARSON",
    "PERCENTILE",
    "PERCENTILE.INC",
    "PERCENTRANK",
    "QUARTILE",
    "QUARTILE.INC",
    "RANK",
    "RANK.EQ",
    "RSQ",
    "SLOPE",
    "SMALL",
    "STDEV.P",
    "STDEV.S",
    "STDEVP",
    "SUBTOTAL",
    "SUMIFS",
    "TRIMMEAN",
    "VAR",
    "VAR.P",
    "VAR.S",
    "VARP",
];

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    v
}

/// Sum of squared deviations from the mean, the shared core of the variance
/// family.
fn devsq(v: &[f64]) -> (f64, f64) {
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    (v.iter().map(|x| (x - mean) * (x - mean)).sum(), mean)
}

/// Paired numeric values from two ranges of the same shape.
fn pairs(args: &[Value]) -> Result<Vec<(f64, f64)>, Value> {
    let ys = flatten(&args[..1]);
    let xs = flatten(&args[1..2]);
    if ys.len() != xs.len() {
        return Err(err(NA_ERR));
    }
    let mut out = Vec::new();
    for (y, x) in ys.iter().zip(&xs) {
        // A pair is used only when both sides are numbers, as Excel does.
        if let (Value::Number(a), Value::Number(b)) = (to_number(y), to_number(x)) {
            if y.is_blankish() || x.is_blankish() {
                continue;
            }
            out.push((a, b));
        }
    }
    Ok(out)
}

/// `SUMIFS`-style argument lists: a target range, then (range, criteria) pairs.
///
/// The rows that pass every pair, as indices into the first range.
fn matching_rows(ranges: &[(&Value, &Value)], length: usize) -> Result<Vec<usize>, Value> {
    let mut keep: Vec<bool> = vec![true; length];
    for (range, criterion) in ranges {
        let Value::Range(r) = range else {
            return Err(err(VALUE_ERR));
        };
        if r.cells.len() != length {
            // Excel requires every criteria range to be the same shape.
            return Err(err(VALUE_ERR));
        }
        let test = make_criteria(criterion);
        for (i, v) in r.values().enumerate() {
            if !test(v) {
                keep[i] = false;
            }
        }
    }
    Ok(keep
        .into_iter()
        .enumerate()
        .filter(|(_, k)| *k)
        .map(|(i, _)| i)
        .collect())
}

/// The `*IFS` family: an optional aggregate range, then criteria pairs.
fn ifs(name: &str, args: &[Value]) -> Value {
    // COUNTIFS starts with a criteria range; the others start with the range to
    // aggregate.
    let counting = name == "COUNTIFS";
    let (target, rest) = if counting {
        (None, args)
    } else {
        (args.first(), &args[1.min(args.len())..])
    };
    if rest.len() < 2 || rest.len() % 2 != 0 {
        return err(VALUE_ERR);
    }
    let pairs: Vec<(&Value, &Value)> = rest.chunks(2).map(|c| (&c[0], &c[1])).collect();
    let length = match pairs.first().map(|(r, _)| r) {
        Some(Value::Range(r)) => r.cells.len(),
        _ => return err(VALUE_ERR),
    };
    let rows = match matching_rows(&pairs, length) {
        Ok(rows) => rows,
        Err(e) => return e,
    };
    if counting {
        return Value::Number(rows.len() as f64);
    }
    let Some(Value::Range(source)) = target else {
        return err(VALUE_ERR);
    };
    if source.cells.len() != length {
        return err(VALUE_ERR);
    }
    let picked: Vec<f64> = rows
        .iter()
        .filter_map(|i| source.cells.get(*i))
        .filter_map(|(_, v)| match to_number(v) {
            Value::Number(n) if !v.is_blankish() => Some(n),
            _ => None,
        })
        .collect();
    match name {
        "SUMIFS" => Value::Number(picked.iter().sum()),
        "AVERAGEIFS" if picked.is_empty() => err(DIV0_ERR),
        "AVERAGEIFS" => Value::Number(picked.iter().sum::<f64>() / picked.len() as f64),
        "MAXIFS" => {
            Value::Number(picked.iter().cloned().fold(f64::NEG_INFINITY, f64::max)).or_zero()
        }
        "MINIFS" => Value::Number(picked.iter().cloned().fold(f64::INFINITY, f64::min)).or_zero(),
        _ => err(VALUE_ERR),
    }
}

trait OrZero {
    fn or_zero(self) -> Value;
}

impl OrZero for Value {
    /// `MAXIFS` with nothing matching is 0 in Excel, not an error.
    fn or_zero(self) -> Value {
        match self {
            Value::Number(n) if !n.is_finite() => Value::Number(0.0),
            other => other,
        }
    }
}

/// The percentile of a sorted sample, interpolating between neighbours.
fn percentile(sorted: &[f64], p: f64) -> Value {
    if sorted.is_empty() || !(0.0..=1.0).contains(&p) {
        return err(NUM_ERR);
    }
    let rank = p * (sorted.len() as f64 - 1.0);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let weight = rank - lower as f64;
    Value::Number(sorted[lower] + (sorted[upper] - sorted[lower]) * weight)
}

/// `SUBTOTAL`'s function numbers. 101-111 mean "ignore hidden rows", which this
/// format has no concept of, so they behave as their 1-11 twins.
fn subtotal(code: i64, args: &[Value]) -> Value {
    let name = match code % 100 {
        1 => "AVERAGE",
        2 => "COUNT",
        3 => "COUNTA",
        4 => "MAX",
        5 => "MIN",
        6 => "PRODUCT",
        7 => "STDEV",
        8 => "STDEV.P",
        9 => "SUM",
        10 => "VAR",
        11 => "VAR.P",
        _ => return err(VALUE_ERR),
    };
    crate::functions::call(name, args).unwrap_or_else(|| err(VALUE_ERR))
}

pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !NAMES.contains(&name) {
        return None;
    }
    // The `*IFS` family reads criteria that may legitimately be text like ">10";
    // everything else propagates an error in any argument.
    if !name.ends_with("IFS") {
        if let Some(e) = first_error(args) {
            return Some(e);
        }
    }
    Some(match name {
        "SUMIFS" | "COUNTIFS" | "AVERAGEIFS" | "MAXIFS" | "MINIFS" => ifs(name, args),
        "VAR" | "VAR.S" | "VARP" | "VAR.P" | "STDEV.S" | "STDEV.P" | "STDEVP" => {
            match numeric_values(args) {
                Err(e) => e,
                Ok(v) => {
                    let population = name.ends_with('P');
                    let n = v.len();
                    if n < 2 && !population || n == 0 {
                        return Some(err(DIV0_ERR));
                    }
                    let (ss, _) = devsq(&v);
                    let divisor = if population { n as f64 } else { n as f64 - 1.0 };
                    let variance = ss / divisor;
                    Value::Number(if name.starts_with("STDEV") {
                        variance.sqrt()
                    } else {
                        variance
                    })
                }
            }
        }
        "DEVSQ" => match numeric_values(args) {
            Err(e) => e,
            Ok(v) if v.is_empty() => err(NUM_ERR),
            Ok(v) => Value::Number(devsq(&v).0),
        },
        "AVEDEV" => match numeric_values(args) {
            Err(e) => e,
            Ok(v) if v.is_empty() => err(NUM_ERR),
            Ok(v) => {
                let mean = v.iter().sum::<f64>() / v.len() as f64;
                Value::Number(v.iter().map(|x| (x - mean).abs()).sum::<f64>() / v.len() as f64)
            }
        },
        "GEOMEAN" => match numeric_values(args) {
            Err(e) => e,
            Ok(v) if v.is_empty() || v.iter().any(|x| *x <= 0.0) => err(NUM_ERR),
            Ok(v) => Value::Number((v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp()),
        },
        "HARMEAN" => match numeric_values(args) {
            Err(e) => e,
            Ok(v) if v.is_empty() || v.iter().any(|x| *x <= 0.0) => err(NUM_ERR),
            Ok(v) => Value::Number(v.len() as f64 / v.iter().map(|x| 1.0 / x).sum::<f64>()),
        },
        "MODE" | "MODE.SNGL" => match numeric_values(args) {
            Err(e) => e,
            Ok(v) => {
                let mut best: Option<(f64, usize)> = None;
                for x in &v {
                    let count = v.iter().filter(|y| *y == x).count();
                    if count > 1 && best.map(|(_, c)| count > c).unwrap_or(true) {
                        best = Some((*x, count));
                    }
                }
                match best {
                    Some((x, _)) => Value::Number(x),
                    None => err(NA_ERR),
                }
            }
        },
        "LARGE" | "SMALL" => {
            let (Ok(v), Ok(k)) = (
                numeric_values(&args[..1.min(args.len())]),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            let k = k.trunc();
            if v.is_empty() || k < 1.0 || k > v.len() as f64 {
                return Some(err(NUM_ERR));
            }
            let s = sorted(v);
            let i = if name == "LARGE" {
                s.len() - k as usize
            } else {
                k as usize - 1
            };
            Value::Number(s[i])
        }
        "PERCENTILE" | "PERCENTILE.INC" => {
            let (Ok(v), Ok(p)) = (
                numeric_values(&args[..1.min(args.len())]),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            percentile(&sorted(v), p)
        }
        "QUARTILE" | "QUARTILE.INC" => {
            let (Ok(v), Ok(q)) = (
                numeric_values(&args[..1.min(args.len())]),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            if !(0.0..=4.0).contains(&q.trunc()) {
                return Some(err(NUM_ERR));
            }
            percentile(&sorted(v), q.trunc() / 4.0)
        }
        "TRIMMEAN" => {
            let (Ok(v), Ok(share)) = (
                numeric_values(&args[..1.min(args.len())]),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            if v.is_empty() || !(0.0..1.0).contains(&share) {
                return Some(err(NUM_ERR));
            }
            let s = sorted(v);
            // Excel trims an even number of points, half from each end.
            let cut = ((s.len() as f64 * share) / 2.0).floor() as usize;
            let kept = &s[cut..s.len() - cut];
            if kept.is_empty() {
                return Some(err(NUM_ERR));
            }
            Value::Number(kept.iter().sum::<f64>() / kept.len() as f64)
        }
        "RANK" | "RANK.EQ" => {
            let Ok(target) = num_of(args.first().unwrap_or(&Value::Blank)) else {
                return Some(err(VALUE_ERR));
            };
            let Ok(v) = numeric_values(&args[1..2.min(args.len())]) else {
                return Some(err(VALUE_ERR));
            };
            let ascending = args
                .get(2)
                .map(|a| num_of(a).unwrap_or(0.0) != 0.0)
                .unwrap_or(false);
            if !v.contains(&target) {
                return Some(err(NA_ERR));
            }
            let better = v
                .iter()
                .filter(|x| {
                    if ascending {
                        **x < target
                    } else {
                        **x > target
                    }
                })
                .count();
            Value::Number(better as f64 + 1.0)
        }
        "PERCENTRANK" => {
            let Ok(v) = numeric_values(&args[..1.min(args.len())]) else {
                return Some(err(VALUE_ERR));
            };
            let Ok(target) = num_of(args.get(1).unwrap_or(&Value::Blank)) else {
                return Some(err(VALUE_ERR));
            };
            if v.len() < 2 {
                return Some(err(NUM_ERR));
            }
            let below = v.iter().filter(|x| **x < target).count() as f64;
            Value::Number(below / (v.len() as f64 - 1.0))
        }
        "CORREL" | "PEARSON" | "RSQ" | "SLOPE" | "INTERCEPT" => {
            if args.len() < 2 {
                return Some(err(VALUE_ERR));
            }
            let data = match pairs(args) {
                Ok(d) => d,
                Err(e) => return Some(e),
            };
            if data.len() < 2 {
                return Some(err(DIV0_ERR));
            }
            let n = data.len() as f64;
            let (sy, sx) = (
                data.iter().map(|(y, _)| y).sum::<f64>(),
                data.iter().map(|(_, x)| x).sum::<f64>(),
            );
            let (my, mx) = (sy / n, sx / n);
            let sxy: f64 = data.iter().map(|(y, x)| (y - my) * (x - mx)).sum();
            let sxx: f64 = data.iter().map(|(_, x)| (x - mx) * (x - mx)).sum();
            let syy: f64 = data.iter().map(|(y, _)| (y - my) * (y - my)).sum();
            if sxx == 0.0 || syy == 0.0 {
                return Some(err(DIV0_ERR));
            }
            let r = sxy / (sxx * syy).sqrt();
            Value::Number(match name {
                "RSQ" => r * r,
                "SLOPE" => sxy / sxx,
                "INTERCEPT" => my - (sxy / sxx) * mx,
                _ => r,
            })
        }
        "FORECAST" | "FORECAST.LINEAR" => {
            if args.len() < 3 {
                return Some(err(VALUE_ERR));
            }
            let Ok(x) = num_of(&args[0]) else {
                return Some(err(VALUE_ERR));
            };
            let data = match pairs(&args[1..]) {
                Ok(d) => d,
                Err(e) => return Some(e),
            };
            if data.len() < 2 {
                return Some(err(DIV0_ERR));
            }
            let n = data.len() as f64;
            let (my, mx) = (
                data.iter().map(|(y, _)| y).sum::<f64>() / n,
                data.iter().map(|(_, x)| x).sum::<f64>() / n,
            );
            let sxy: f64 = data.iter().map(|(y, x)| (y - my) * (x - mx)).sum();
            let sxx: f64 = data.iter().map(|(_, x)| (x - mx) * (x - mx)).sum();
            if sxx == 0.0 {
                return Some(err(DIV0_ERR));
            }
            let slope = sxy / sxx;
            Value::Number(my - slope * mx + slope * x)
        }
        "SUBTOTAL" => match num_of(args.first().unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(code) => subtotal(code.trunc() as i64, &args[1.min(args.len())..]),
        },
        "AGGREGATE" => {
            // `options` says which values to ignore; this format has no hidden
            // rows, and errors are skipped when asked.
            let (Ok(code), Ok(options)) = (
                num_of(args.first().unwrap_or(&Value::Blank)),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            let rest = &args[2.min(args.len())..];
            let cleaned: Vec<Value> = if matches!(options.trunc() as i64, 2 | 3 | 6 | 7) {
                vec![Value::Array(
                    flatten(rest).into_iter().filter(|v| !v.is_err()).collect(),
                )]
            } else {
                rest.to_vec()
            };
            match code.trunc() as i64 {
                code @ 1..=11 => subtotal(code, &cleaned),
                12 => call(
                    "PERCENTILE.INC",
                    &[
                        cleaned[0].clone(),
                        cleaned.get(1).cloned().unwrap_or(Value::Blank),
                    ],
                )
                .unwrap_or_else(|| err(VALUE_ERR)),
                13 => call("MODE.SNGL", &cleaned).unwrap_or_else(|| err(VALUE_ERR)),
                14 => call("LARGE", &cleaned).unwrap_or_else(|| err(VALUE_ERR)),
                15 => call("SMALL", &cleaned).unwrap_or_else(|| err(VALUE_ERR)),
                16 => call("PERCENTILE.INC", &cleaned).unwrap_or_else(|| err(VALUE_ERR)),
                17 => call("QUARTILE.INC", &cleaned).unwrap_or_else(|| err(VALUE_ERR)),
                _ => err(VALUE_ERR),
            }
        }
        _ => return None,
    })
}
