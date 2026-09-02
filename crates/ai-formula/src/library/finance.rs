//! Loan and investment maths.
//!
//! The sign convention is Excel's: money you pay out is negative, money you
//! receive is positive, which is why `PMT` of a positive loan is negative.

use crate::values::{err, first_error, num_of, Value, NUM_ERR, VALUE_ERR};

pub const NAMES: &[&str] = &[
    "DDB", "FV", "IPMT", "IRR", "NPER", "NPV", "PMT", "PPMT", "PV", "RATE", "SLN", "SYD",
];

/// Arguments by position, with a default for the ones Excel makes optional.
fn arg(args: &[Value], i: usize, default: f64) -> Result<f64, Value> {
    match args.get(i) {
        None | Some(Value::Blank) => Ok(default),
        Some(v) => num_of(v),
    }
}

/// `(1 + rate)^nper`, the factor every one of these formulas is built on.
fn growth(rate: f64, nper: f64) -> f64 {
    (1.0 + rate).powf(nper)
}

fn pmt(rate: f64, nper: f64, pv: f64, fv: f64, due: f64) -> f64 {
    if nper == 0.0 {
        return f64::NAN;
    }
    if rate == 0.0 {
        return -(pv + fv) / nper;
    }
    let g = growth(rate, nper);
    -(pv * g + fv) * rate / ((g - 1.0) * (1.0 + rate * due))
}

fn fv(rate: f64, nper: f64, pmt: f64, pv: f64, due: f64) -> f64 {
    if rate == 0.0 {
        return -(pv + pmt * nper);
    }
    let g = growth(rate, nper);
    -(pv * g + pmt * (1.0 + rate * due) * (g - 1.0) / rate)
}

fn pv(rate: f64, nper: f64, pmt: f64, fv: f64, due: f64) -> f64 {
    if rate == 0.0 {
        return -(fv + pmt * nper);
    }
    let g = growth(rate, nper);
    -(fv + pmt * (1.0 + rate * due) * (g - 1.0) / rate) / g
}

/// Net present value of `values` at times 1..n, which is what Excel's NPV does
/// (the first value is already discounted one period).
fn npv(rate: f64, values: &[f64]) -> f64 {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| v / (1.0 + rate).powi(i as i32 + 1))
        .sum()
}

/// Newton's method with a bisection fallback, for the two functions that have no
/// closed form. Excel solves these iteratively too, and stops at 1e-7.
fn solve(guess: f64, f: impl Fn(f64) -> f64) -> Option<f64> {
    let mut x = guess;
    for _ in 0..100 {
        let y = f(x);
        if !y.is_finite() {
            break;
        }
        if y.abs() < 1e-10 {
            return Some(x);
        }
        let step = (x.abs().max(1e-4)) * 1e-6;
        let slope = (f(x + step) - y) / step;
        if !slope.is_finite() || slope == 0.0 {
            break;
        }
        let next = x - y / slope;
        if !next.is_finite() {
            break;
        }
        if (next - x).abs() < 1e-12 {
            return Some(next);
        }
        x = next;
    }
    // Bisection over a wide bracket: slower, but it does not need a derivative.
    let (mut lo, mut hi) = (-0.999_999, 10.0);
    let (mut flo, fhi) = (f(lo), f(hi));
    if !flo.is_finite() || !fhi.is_finite() || flo.signum() == fhi.signum() {
        return None;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        let fmid = f(mid);
        if fmid.abs() < 1e-10 {
            return Some(mid);
        }
        if fmid.signum() == flo.signum() {
            lo = mid;
            flo = fmid;
        } else {
            hi = mid;
        }
    }
    Some((lo + hi) / 2.0)
}

pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !NAMES.contains(&name) {
        return None;
    }
    if let Some(e) = first_error(args) {
        return Some(e);
    }
    let n = |i: usize, d: f64| arg(args, i, d);
    Some(match name {
        "PMT" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0), n(3, 0.0), n(4, 0.0)) {
            (Ok(rate), Ok(nper), Ok(present), Ok(future), Ok(due)) => {
                crate::library::math::finite(pmt(rate, nper, present, future, due))
            }
            _ => err(VALUE_ERR),
        },
        "FV" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0), n(3, 0.0), n(4, 0.0)) {
            (Ok(rate), Ok(nper), Ok(payment), Ok(present), Ok(due)) => {
                crate::library::math::finite(fv(rate, nper, payment, present, due))
            }
            _ => err(VALUE_ERR),
        },
        "PV" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0), n(3, 0.0), n(4, 0.0)) {
            (Ok(rate), Ok(nper), Ok(payment), Ok(future), Ok(due)) => {
                crate::library::math::finite(pv(rate, nper, payment, future, due))
            }
            _ => err(VALUE_ERR),
        },
        "NPER" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0), n(3, 0.0), n(4, 0.0)) {
            (Ok(rate), Ok(payment), Ok(present), Ok(future), Ok(due)) => {
                if rate == 0.0 {
                    if payment == 0.0 {
                        return Some(err(NUM_ERR));
                    }
                    Value::Number(-(present + future) / payment)
                } else {
                    let adjusted = payment * (1.0 + rate * due);
                    let top = adjusted - future * rate;
                    let bottom = adjusted + present * rate;
                    // Both sides are negative for an ordinary loan (money out),
                    // so what has to be positive is the ratio, not the parts.
                    let ratio = top / bottom;
                    if ratio.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                        return Some(err(NUM_ERR));
                    }
                    crate::library::math::finite(ratio.ln() / (1.0 + rate).ln())
                }
            }
            _ => err(VALUE_ERR),
        },
        "RATE" => match (
            n(0, 0.0),
            n(1, 0.0),
            n(2, 0.0),
            n(3, 0.0),
            n(4, 0.0),
            n(5, 0.1),
        ) {
            (Ok(nper), Ok(payment), Ok(present), Ok(future), Ok(due), Ok(guess)) => {
                match solve(guess, |r| {
                    if r == 0.0 {
                        present + payment * nper + future
                    } else {
                        let g = growth(r, nper);
                        present * g + payment * (1.0 + r * due) * (g - 1.0) / r + future
                    }
                }) {
                    Some(r) => Value::Number(r),
                    None => err(NUM_ERR),
                }
            }
            _ => err(VALUE_ERR),
        },
        "NPV" => {
            let Ok(rate) = n(0, 0.0) else {
                return Some(err(VALUE_ERR));
            };
            match crate::values::numeric_values(&args[1.min(args.len())..]) {
                Err(e) => e,
                Ok(v) => crate::library::math::finite(npv(rate, &v)),
            }
        }
        "IRR" => match crate::values::numeric_values(&args[..1.min(args.len())]) {
            Err(e) => e,
            Ok(v) if v.len() < 2 => err(NUM_ERR),
            Ok(v) => {
                let guess = arg(args, 1, 0.1).unwrap_or(0.1);
                // IRR is the rate where the cash flows, discounted from time 0,
                // sum to nothing.
                match solve(guess, |r| {
                    v.iter()
                        .enumerate()
                        .map(|(i, x)| x / (1.0 + r).powi(i as i32))
                        .sum::<f64>()
                }) {
                    Some(r) => Value::Number(r),
                    None => err(NUM_ERR),
                }
            }
        },
        "IPMT" | "PPMT" => {
            match (
                n(0, 0.0),
                n(1, 0.0),
                n(2, 0.0),
                n(3, 0.0),
                n(4, 0.0),
                n(5, 0.0),
            ) {
                (Ok(rate), Ok(period), Ok(nper), Ok(present), Ok(future), Ok(due)) => {
                    if period < 1.0 || period > nper {
                        return Some(err(NUM_ERR));
                    }
                    let payment = pmt(rate, nper, present, future, due);
                    // The balance at the start of the period, which is what the
                    // interest is charged on.
                    let balance = fv(rate, period - 1.0, payment, present, due);
                    let interest = if due != 0.0 && period == 1.0 {
                        0.0
                    } else if due != 0.0 {
                        balance * rate / (1.0 + rate)
                    } else {
                        balance * rate
                    };
                    Value::Number(if name == "IPMT" {
                        interest
                    } else {
                        payment - interest
                    })
                }
                _ => err(VALUE_ERR),
            }
        }
        "SLN" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0)) {
            (Ok(cost), Ok(salvage), Ok(life)) if life != 0.0 => {
                Value::Number((cost - salvage) / life)
            }
            (Ok(_), Ok(_), Ok(_)) => err(crate::values::DIV0_ERR),
            _ => err(VALUE_ERR),
        },
        "SYD" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0), n(3, 0.0)) {
            (Ok(cost), Ok(salvage), Ok(life), Ok(period)) => {
                if life <= 0.0 || period < 1.0 || period > life {
                    return Some(err(NUM_ERR));
                }
                Value::Number(
                    (cost - salvage) * (life - period + 1.0) * 2.0 / (life * (life + 1.0)),
                )
            }
            _ => err(VALUE_ERR),
        },
        "DDB" => match (n(0, 0.0), n(1, 0.0), n(2, 0.0), n(3, 0.0), n(4, 2.0)) {
            (Ok(cost), Ok(salvage), Ok(life), Ok(period), Ok(factor)) => {
                if life <= 0.0 || period < 1.0 {
                    return Some(err(NUM_ERR));
                }
                // Declining balance, never depreciating past the salvage value.
                let rate = factor / life;
                let mut book = cost;
                let mut charge = 0.0;
                let mut left = period;
                while left > 0.0 {
                    charge = (book * rate).min(book - salvage).max(0.0);
                    book -= charge;
                    left -= 1.0;
                }
                Value::Number(charge)
            }
            _ => err(VALUE_ERR),
        },
        _ => return None,
    })
}
