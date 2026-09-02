//! Dates and times, on the 1900 serial the rest of the engine uses.

use crate::numfmt::{date_arg, date_serial, serial_to_ymd};
use crate::values::{err, first_error, num_of, to_text, Value, NUM_ERR, VALUE_ERR};

pub const NAMES: &[&str] = &[
    "DATEDIF",
    "DATEVALUE",
    "DAYS360",
    "EDATE",
    "EOMONTH",
    "HOUR",
    "ISOWEEKNUM",
    "MINUTE",
    "NETWORKDAYS",
    "SECOND",
    "TIME",
    "TIMEVALUE",
    "WEEKNUM",
    "WORKDAY",
    "YEARFRAC",
];

fn serial_at(args: &[Value], i: usize) -> Result<f64, Value> {
    date_arg(args.get(i).unwrap_or(&Value::Blank))
}

/// The clock part of a serial, in seconds, rounded to the nearest second.
///
/// A time is the fractional day, so 0.5 is noon. Rounding matters: 3/24 stored
/// as a float is 0.12499999999999999, and `HOUR` of that must be 3.
fn seconds_of(serial: f64) -> i64 {
    let frac = serial - serial.floor();
    (frac * 86_400.0).round() as i64 % 86_400
}

/// Whether a serial falls on a Saturday or Sunday. Serial 1 is 1900-01-01, a
/// Monday in Excel's calendar.
fn is_weekend(serial: f64) -> bool {
    let dow = (serial.floor() as i64).rem_euclid(7);
    // 0 = Saturday in this arithmetic, 1 = Sunday.
    dow == 0 || dow == 1
}

fn holidays(arg: Option<&Value>) -> Vec<f64> {
    let Some(arg) = arg else { return Vec::new() };
    crate::values::flatten(std::slice::from_ref(arg))
        .iter()
        .filter_map(|v| date_arg(v).ok())
        .map(f64::floor)
        .collect()
}

/// Months added to a date, clamped to the end of the target month — Excel's
/// EDATE of 31 January plus one month is 28 February, not 3 March.
fn shift_months(serial: f64, months: i64) -> Option<f64> {
    let (y, m, d) = serial_to_ymd(serial)?;
    let total = y as i64 * 12 + (m as i64 - 1) + months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let last = last_day(year as i32, month as u32);
    date_serial(year, month, d.min(last) as i64)
}

fn last_day(year: i32, month: u32) -> u32 {
    let (next_y, next_m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = date_serial(next_y as i64, next_m as i64, 1).unwrap_or(0.0);
    let this = date_serial(year as i64, month as i64, 1).unwrap_or(0.0);
    (first_next - this) as u32
}

pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !NAMES.contains(&name) {
        return None;
    }
    if let Some(e) = first_error(args) {
        return Some(e);
    }
    Some(match name {
        "TIME" => {
            let mut parts = [0.0f64; 3];
            for (i, slot) in parts.iter_mut().enumerate() {
                match num_of(args.get(i).unwrap_or(&Value::Blank)) {
                    Err(e) => return Some(e),
                    Ok(n) => *slot = n.trunc(),
                }
            }
            let seconds = parts[0] * 3600.0 + parts[1] * 60.0 + parts[2];
            // Excel wraps: TIME(25,0,0) is 01:00.
            let frac = (seconds / 86_400.0).rem_euclid(1.0);
            Value::Number(frac)
        }
        "HOUR" | "MINUTE" | "SECOND" => match serial_at(args, 0) {
            Err(e) => e,
            Ok(serial) => {
                let s = seconds_of(serial);
                Value::Number(match name {
                    "HOUR" => (s / 3600) as f64,
                    "MINUTE" => ((s % 3600) / 60) as f64,
                    _ => (s % 60) as f64,
                })
            }
        },
        "DATEVALUE" | "TIMEVALUE" => match date_arg(args.first().unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(serial) => Value::Number(if name == "DATEVALUE" {
                serial.floor()
            } else {
                serial - serial.floor()
            }),
        },
        "EDATE" | "EOMONTH" => {
            let (Ok(serial), Ok(months)) = (
                serial_at(args, 0),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            let shifted = shift_months(serial, months.trunc() as i64);
            match (name, shifted) {
                (_, None) => err(NUM_ERR),
                ("EDATE", Some(s)) => Value::Number(s),
                (_, Some(s)) => match serial_to_ymd(s) {
                    None => err(NUM_ERR),
                    Some((y, m, _)) => match date_serial(y as i64, m as i64, last_day(y, m) as i64)
                    {
                        Some(end) => Value::Number(end),
                        None => err(NUM_ERR),
                    },
                },
            }
        }
        "DATEDIF" => {
            let (Ok(from), Ok(to)) = (serial_at(args, 0), serial_at(args, 1)) else {
                return Some(err(VALUE_ERR));
            };
            let unit = to_text(args.get(2).unwrap_or(&Value::Blank)).to_uppercase();
            if to < from {
                return Some(err(NUM_ERR));
            }
            let (Some((y1, m1, d1)), Some((y2, m2, d2))) = (serial_to_ymd(from), serial_to_ymd(to))
            else {
                return Some(err(NUM_ERR));
            };
            let whole_months = |partial: bool| {
                let mut months = (y2 as i64 - y1 as i64) * 12 + (m2 as i64 - m1 as i64);
                if d2 < d1 {
                    months -= 1;
                }
                if partial {
                    months.rem_euclid(12)
                } else {
                    months
                }
            };
            Value::Number(match unit.as_str() {
                "D" => (to.floor() - from.floor()).abs(),
                "M" => whole_months(false) as f64,
                "Y" => (whole_months(false) / 12) as f64,
                "YM" => whole_months(true) as f64,
                "MD" => {
                    // Days ignoring months and years.
                    let anchor = if d2 >= d1 {
                        date_serial(y2 as i64, m2 as i64, d1 as i64)
                    } else {
                        let prev = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
                        date_serial(prev.0 as i64, prev.1 as i64, d1 as i64)
                    };
                    match anchor {
                        Some(a) => to.floor() - a,
                        None => return Some(err(NUM_ERR)),
                    }
                }
                "YD" => {
                    // Days ignoring years.
                    let anchor = if (m2, d2) >= (m1, d1) {
                        date_serial(y2 as i64, m1 as i64, d1 as i64)
                    } else {
                        date_serial(y2 as i64 - 1, m1 as i64, d1 as i64)
                    };
                    match anchor {
                        Some(a) => to.floor() - a,
                        None => return Some(err(NUM_ERR)),
                    }
                }
                _ => return Some(err(NUM_ERR)),
            })
        }
        "DAYS360" => {
            let (Ok(from), Ok(to)) = (serial_at(args, 0), serial_at(args, 1)) else {
                return Some(err(VALUE_ERR));
            };
            let european = args
                .get(2)
                .map(|v| matches!(crate::values::to_boolean(v), Value::Bool(true)))
                .unwrap_or(false);
            let (Some((y1, m1, d1)), Some((y2, m2, d2))) = (serial_to_ymd(from), serial_to_ymd(to))
            else {
                return Some(err(NUM_ERR));
            };
            let (mut a, mut b) = (d1 as i64, d2 as i64);
            if european {
                a = a.min(30);
                b = b.min(30);
            } else {
                if a == 31 {
                    a = 30;
                }
                if b == 31 {
                    b = if a == 30 { 30 } else { 31 };
                }
            }
            Value::Number(
                ((y2 as i64 - y1 as i64) * 360 + (m2 as i64 - m1 as i64) * 30 + (b - a)) as f64,
            )
        }
        "YEARFRAC" => {
            let (Ok(from), Ok(to)) = (serial_at(args, 0), serial_at(args, 1)) else {
                return Some(err(VALUE_ERR));
            };
            let basis = args
                .get(2)
                .map(|v| num_of(v).unwrap_or(0.0).trunc())
                .unwrap_or(0.0);
            let (start, end) = (from.min(to).floor(), from.max(to).floor());
            let days = end - start;
            Value::Number(match basis as i64 {
                // Actual/actual, approximated by the average year length over the
                // span, which is what makes 2000-02-29 come out right.
                1 => days / average_year(start, end),
                2 => days / 360.0,
                3 => days / 365.0,
                4 => {
                    let d = match call(
                        "DAYS360",
                        &[Value::Number(start), Value::Number(end), Value::Bool(true)],
                    ) {
                        Some(Value::Number(n)) => n,
                        _ => days,
                    };
                    d / 360.0
                }
                // 30/360 US.
                _ => {
                    let d = match call("DAYS360", &[Value::Number(start), Value::Number(end)]) {
                        Some(Value::Number(n)) => n,
                        _ => days,
                    };
                    d / 360.0
                }
            })
        }
        "NETWORKDAYS" | "WORKDAY" => {
            let Ok(start) = serial_at(args, 0) else {
                return Some(err(VALUE_ERR));
            };
            let skip = holidays(args.get(2));
            let counts = |day: f64| !is_weekend(day) && !skip.contains(&day.floor());
            if name == "NETWORKDAYS" {
                let Ok(end) = serial_at(args, 1) else {
                    return Some(err(VALUE_ERR));
                };
                let (a, b) = (
                    start.floor().min(end.floor()),
                    start.floor().max(end.floor()),
                );
                let mut days = 0i64;
                let mut day = a;
                while day <= b {
                    if counts(day) {
                        days += 1;
                    }
                    day += 1.0;
                }
                let signed = if end < start { -days } else { days };
                Value::Number(signed as f64)
            } else {
                let Ok(count) = num_of(args.get(1).unwrap_or(&Value::Blank)) else {
                    return Some(err(VALUE_ERR));
                };
                let step = if count < 0.0 { -1.0 } else { 1.0 };
                let mut left = count.trunc().abs();
                let mut day = start.floor();
                while left > 0.0 {
                    day += step;
                    if counts(day) {
                        left -= 1.0;
                    }
                }
                Value::Number(day)
            }
        }
        "WEEKNUM" | "ISOWEEKNUM" => match serial_at(args, 0) {
            Err(e) => e,
            Ok(serial) => {
                let Some((y, _, _)) = serial_to_ymd(serial) else {
                    return Some(err(NUM_ERR));
                };
                if name == "ISOWEEKNUM" {
                    use chrono::{Datelike, NaiveDate};
                    let Some((y, m, d)) = serial_to_ymd(serial) else {
                        return Some(err(NUM_ERR));
                    };
                    match NaiveDate::from_ymd_opt(y, m, d) {
                        Some(date) => Value::Number(date.iso_week().week() as f64),
                        None => err(NUM_ERR),
                    }
                } else {
                    // Week 1 is the week containing 1 January; `mode` says which
                    // day starts a week (1 = Sunday, 2 = Monday).
                    let mode = args
                        .get(1)
                        .map(|v| num_of(v).unwrap_or(1.0).trunc())
                        .unwrap_or(1.0);
                    let jan1 = date_serial(y as i64, 1, 1).unwrap_or(serial);
                    let offset = if mode == 2.0 { 1 } else { 0 };
                    let start_dow = ((jan1.floor() as i64 + 5 - offset).rem_euclid(7)) as f64;
                    Value::Number(((serial.floor() - jan1 + start_dow) / 7.0).floor() + 1.0)
                }
            }
        },
        _ => return None,
    })
}

/// Days per year across a span, for `YEARFRAC` basis 1.
fn average_year(start: f64, end: f64) -> f64 {
    let (Some((y1, ..)), Some((y2, ..))) = (serial_to_ymd(start), serial_to_ymd(end)) else {
        return 365.0;
    };
    let years = (y2 - y1 + 1) as f64;
    let first = date_serial(y1 as i64, 1, 1).unwrap_or(0.0);
    let after = date_serial(y2 as i64 + 1, 1, 1).unwrap_or(first + 365.0 * years);
    (after - first) / years
}
