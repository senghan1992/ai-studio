//! Number-format rendering.

use crate::jsnum;
use crate::values::{format_plain_number, to_number, to_text, Value};

/// Excel serial 0 is 1899-12-30.
const EPOCH_DAYS_FROM_CE: i32 = 693594;

pub fn serial_to_ymd(serial: f64) -> Option<(i32, u32, u32)> {
    use chrono::{Datelike, NaiveDate};
    let days = serial.round();
    if !days.is_finite() || days.abs() > 4_000_000.0 {
        return None;
    }
    let d = NaiveDate::from_num_days_from_ce_opt(EPOCH_DAYS_FROM_CE + days as i32)?;
    Some((d.year(), d.month(), d.day()))
}

pub fn ymd_to_serial(year: i32, month: u32, day: u32) -> Option<f64> {
    use chrono::{Datelike, NaiveDate};
    let d = NaiveDate::from_ymd_opt(year, month, day)?;
    Some((d.num_days_from_ce() - EPOCH_DAYS_FROM_CE) as f64)
}

/// `Date.UTC(y, m - 1, d)` expressed as an Excel serial, including the
/// month/day overflow that makes `DATE(2026, 13, 1)` mean January 2027.
pub fn date_serial(year: i64, month: i64, day: i64) -> Option<f64> {
    use chrono::{Duration, NaiveDate};
    // JS MakeFullYear: a two-digit year means 19xx.
    let year = if (0..=99).contains(&year) {
        year + 1900
    } else {
        year
    };
    let month0 = month - 1;
    let year = year + month0.div_euclid(12);
    let month = month0.rem_euclid(12) + 1;
    let year: i32 = year.try_into().ok()?;
    let first = NaiveDate::from_ymd_opt(year, month as u32, 1)?;
    let date = first.checked_add_signed(Duration::days(day - 1))?;
    use chrono::Datelike;
    Some((date.num_days_from_ce() - EPOCH_DAYS_FROM_CE) as f64)
}

/// `Date.parse` for the ISO-ish shapes a spreadsheet actually stores.
pub fn parse_date_text(text: &str) -> Option<f64> {
    let s = text.trim();
    let date_part = s.split(['T', ' ']).next()?;
    let sep = if date_part.contains('-') { '-' } else { '/' };
    let mut parts = date_part.split(sep);
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    ymd_to_serial(y, m, d)
}

/// A value read as a date serial, accepting both numbers and date text.
pub fn date_arg(v: &Value) -> Result<f64, Value> {
    match to_number(v) {
        Value::Number(n) => Ok(n),
        _ => {
            parse_date_text(&to_text(v)).ok_or_else(|| crate::values::err(crate::values::VALUE_ERR))
        }
    }
}

fn pad2(n: u32) -> String {
    format!("{n:02}")
}

/// Minimal number-format renderer covering the patterns the UI offers:
/// `#,##0`, `#,##0.00`, `0%`, `0.0%`, `₩#,##0`, `$#,##0.00`, `yyyy-mm-dd`, `@`.
pub fn apply_num_fmt(value: &Value, fmt: &str) -> String {
    if value.is_blankish() {
        return String::new();
    }
    if let Value::Error(code) = value {
        return code.clone();
    }
    if fmt.is_empty() || fmt == "General" || fmt == "@" {
        return match value {
            Value::Number(n) => format_plain_number(*n),
            other => to_text(other),
        };
    }

    let has_date_letter = fmt
        .chars()
        .any(|c| matches!(c, 'y' | 'Y' | 'm' | 'M' | 'd' | 'D'));
    let has_digit_slot = fmt.contains('#') || fmt.contains('0');
    if has_date_letter && !has_digit_slot {
        let Ok(serial) = date_arg(value) else {
            return to_text(value);
        };
        let Some((y, m, d)) = serial_to_ymd(serial) else {
            return to_text(value);
        };
        // Order matters: yyyy before yy, and `mm` is case-sensitive while the
        // others are not — same as the original's regex flags.
        let out = replace_ci(fmt, "yyyy", &y.to_string());
        let short = y.to_string();
        let short = short
            .get(short.len().saturating_sub(2)..)
            .unwrap_or(&short)
            .to_string();
        let out = replace_ci(&out, "yy", &short);
        let out = out.replace("mm", &pad2(m));
        return replace_ci(&out, "dd", &pad2(d));
    }

    let num = match to_number(value) {
        Value::Number(n) => n,
        _ => return to_text(value),
    };

    let is_percent = fmt.contains('%');
    let scaled = if is_percent { num * 100.0 } else { num };
    let decimals = decimal_slots(fmt);
    let grouping = fmt.contains("#,##");
    let prefix: String = fmt.chars().take_while(|c| !is_fmt_slot(*c)).collect();
    let suffix: String = {
        let tail: String = fmt.chars().rev().take_while(|c| !is_fmt_slot(*c)).collect();
        tail.chars().rev().collect()
    };

    let body = jsnum::to_locale_string(scaled.abs(), decimals, grouping);
    let sign = if scaled < 0.0 { "-" } else { "" };
    let pct = if is_percent { "%" } else { "" };
    format!("{sign}{prefix}{body}{pct}{suffix}")
}

fn is_fmt_slot(c: char) -> bool {
    matches!(c, '#' | '0' | '.' | ',' | '%')
}

/// Digits after the decimal point in the pattern, i.e. `.00` -> 2.
fn decimal_slots(fmt: &str) -> usize {
    let Some(idx) = fmt.find(".0") else { return 0 };
    fmt[idx + 1..].chars().take_while(|c| *c == '0').count()
}

fn replace_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    let lower_needle = needle.to_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut rest = haystack;
    loop {
        let lower = rest.to_lowercase();
        match lower.find(&lower_needle) {
            Some(at) => {
                out.push_str(&rest[..at]);
                out.push_str(replacement);
                rest = &rest[at + needle.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::Value;

    #[test]
    fn renders_the_formats_the_ui_offers() {
        let n = |x: f64| Value::Number(x);
        assert_eq!(apply_num_fmt(&n(1470000000.0), "₩#,##0"), "₩1,470,000,000");
        assert_eq!(apply_num_fmt(&n(0.48), "0.0%"), "48.0%");
        assert_eq!(apply_num_fmt(&n(0.48), "0%"), "48%");
        assert_eq!(apply_num_fmt(&n(1234.5), "#,##0.00"), "1,234.50");
        assert_eq!(apply_num_fmt(&n(-1234.5), "#,##0"), "-1,235");
        assert_eq!(apply_num_fmt(&n(1234.5), "$#,##0.00"), "$1,234.50");
        assert_eq!(apply_num_fmt(&n(42.0), "General"), "42");
        assert_eq!(apply_num_fmt(&Value::Blank, "#,##0"), "");
        assert_eq!(apply_num_fmt(&Value::Error("#N/A".into()), "#,##0"), "#N/A");
    }

    #[test]
    fn renders_dates() {
        let serial = ymd_to_serial(2026, 9, 1).unwrap();
        assert_eq!(
            apply_num_fmt(&Value::Number(serial), "yyyy-mm-dd"),
            "2026-09-01"
        );
        assert_eq!(
            apply_num_fmt(&Value::Number(serial), "yy-mm-dd"),
            "26-09-01"
        );
        // Serial 0 is 1899-12-30, as in the JS original. That is one day off
        // from Excel's own epoch, which is how both agree on every date from
        // 1900-03-01 on despite Excel's phantom 1900-02-29.
        assert_eq!(serial_to_ymd(0.0), Some((1899, 12, 30)));
        assert_eq!(ymd_to_serial(2026, 9, 1), Some(46266.0));
    }

    #[test]
    fn date_arithmetic_overflows_like_js() {
        // DATE(2026, 13, 1) is January 2027.
        assert_eq!(
            serial_to_ymd(date_serial(2026, 13, 1).unwrap()),
            Some((2027, 1, 1))
        );
        // Day 0 is the last day of the previous month.
        assert_eq!(
            serial_to_ymd(date_serial(2026, 3, 0).unwrap()),
            Some((2026, 2, 28))
        );
        // A two-digit year means 19xx.
        assert_eq!(
            serial_to_ymd(date_serial(99, 1, 1).unwrap()),
            Some((1999, 1, 1))
        );
    }
}
