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

/// Render a value through an Excel number-format pattern.
///
/// Not a full implementation of the format language — nobody needs `[Blue]`
/// text or fill characters on screen — but a real implementation of the parts a
/// workbook actually carries: sections for negatives and zero, quoted literals,
/// currency and locale brackets, month and day names, times, and scaling by
/// trailing commas. A file whose amounts are formatted `#,##0_);[Red](#,##0)`
/// has to show `(1,234)`, not `-1234`, or the sheet does not look like itself.
pub fn apply_num_fmt(value: &Value, fmt: &str) -> String {
    if let Value::Error(code) = value {
        return code.clone();
    }
    if fmt.is_empty() || fmt.eq_ignore_ascii_case("general") {
        return match value {
            Value::Number(n) => format_plain_number(*n),
            other => to_text(other),
        };
    }

    let sections = split_sections(fmt);
    // Text goes through the fourth section, or through the first when that one
    // is itself a text pattern. Otherwise it comes straight out — a number
    // format has nothing to say about a word.
    if !matches!(value, Value::Number(_) | Value::Blank) && !value.is_blankish() {
        let section = sections
            .get(3)
            .or_else(|| sections.first().filter(|s| s.contains('@')));
        return match section {
            Some(section) => render_text(&to_text(value), section),
            None => to_text(value),
        };
    }
    if value.is_blankish() {
        // An empty cell shows nothing, whatever the format says.
        return String::new();
    }
    let Value::Number(num) = to_number(value) else {
        return to_text(value);
    };

    // Positive; negative; zero. A negative with its own section is rendered from
    // its absolute value, because the section carries the sign itself.
    let (section, magnitude) = match (num, sections.len()) {
        (n, len) if n < 0.0 && len >= 2 => (&sections[1], n.abs()),
        (n, len) if n == 0.0 && len >= 3 => (&sections[2], n),
        // One section for everything: the sign is rendered in front of it, which
        // is what Excel does — `-₩1,235`, not `₩-1,235`.
        (n, _) => (&sections[0], n.abs()),
    };
    if section.trim().is_empty() {
        // `0;;` hides zero, and `;;;` hides everything.
        return String::new();
    }

    // `@` is the text placeholder; a number formatted through it is shown as
    // the text it would be, which is what Excel's "Text" format does.
    if section.contains('@') {
        return render_text(&format_plain_number(num), section);
    }
    if is_date_pattern(section) {
        return render_datetime(num, section);
    }
    render_number(magnitude, num < 0.0 && sections.len() < 2, section)
}

/// Split on `;`, ignoring separators inside quotes or brackets.
fn split_sections(fmt: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut bracketed = false;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                out.last_mut().unwrap().push(c);
            }
            '[' if !quoted => {
                bracketed = true;
                out.last_mut().unwrap().push(c);
            }
            ']' if !quoted => {
                bracketed = false;
                out.last_mut().unwrap().push(c);
            }
            '\\' if !quoted => {
                out.last_mut().unwrap().push(c);
                if let Some(next) = chars.next() {
                    out.last_mut().unwrap().push(next);
                }
            }
            ';' if !quoted && !bracketed => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

/// One piece of a pattern: a literal to emit, or a slot to fill.
#[derive(Debug, PartialEq)]
enum Piece {
    Literal(String),
    /// A run of `#`, `0` and `?` with its decimal separator, if any.
    Number,
    Percent,
    Token(String),
}

/// Split a section into literals and slots.
///
/// Quotes, `\x`, `[...]` and `_x` all mean "this is text, not a slot", and
/// getting that wrong turns `"년"` into a fill of digits.
fn pieces(section: &str, dates: bool) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut literal = String::new();
    let chars: Vec<char> = section.chars().collect();
    let mut i = 0;

    let flush = |literal: &mut String, out: &mut Vec<Piece>| {
        if !literal.is_empty() {
            out.push(Piece::Literal(std::mem::take(literal)));
        }
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    literal.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '\\' => {
                if i + 1 < chars.len() {
                    literal.push(chars[i + 1]);
                }
                i += 2;
            }
            // `_x` reserves the width of a character; on screen a space is the
            // honest rendering of that.
            '_' => {
                literal.push(' ');
                i += 2;
            }
            // `*x` repeats a character to fill the column; nothing to fill here.
            '*' => i += 2,
            '[' => {
                let mut inner = String::new();
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    inner.push(chars[i]);
                    i += 1;
                }
                i += 1;
                // `[$₩-412]` carries a currency symbol; `[Red]` and `[<100]`
                // carry nothing to show.
                if let Some(rest) = inner.strip_prefix('$') {
                    let symbol = rest.split('-').next().unwrap_or("");
                    literal.push_str(symbol);
                } else if dates
                    && matches!(
                        inner.to_lowercase().as_str(),
                        "h" | "hh" | "m" | "mm" | "s" | "ss"
                    )
                {
                    flush(&mut literal, &mut out);
                    // Elapsed time, e.g. `[h]:mm`.
                    out.push(Piece::Token(format!("[{}]", inner.to_lowercase())));
                }
            }
            'e' | 'E' if !dates && matches!(chars.get(i + 1), Some('+') | Some('-')) => {
                flush(&mut literal, &mut out);
                out.push(Piece::Token("E".into()));
                i += 2;
                // The exponent's own digit slots.
                while matches!(chars.get(i), Some('0') | Some('#')) {
                    i += 1;
                }
            }
            '#' | '0' | '?' | '.' | ',' if !dates => {
                flush(&mut literal, &mut out);
                while i < chars.len() && matches!(chars[i], '#' | '0' | '?' | '.' | ',') {
                    i += 1;
                }
                out.push(Piece::Number);
            }
            '%' => {
                flush(&mut literal, &mut out);
                out.push(Piece::Percent);
                i += 1;
            }
            c if dates && "yYmMdDhHsS".contains(c) => {
                flush(&mut literal, &mut out);
                let mut token = String::new();
                while i < chars.len() && chars[i].eq_ignore_ascii_case(&c) {
                    token.push(chars[i]);
                    i += 1;
                }
                out.push(Piece::Token(token.to_lowercase()));
            }
            c if dates
                && (c == 'A' || c == 'a')
                && section[i..].to_uppercase().starts_with("AM/PM") =>
            {
                flush(&mut literal, &mut out);
                out.push(Piece::Token("am/pm".into()));
                i += 5;
            }
            _ => {
                literal.push(c);
                i += 1;
            }
        }
    }
    flush(&mut literal, &mut out);
    out
}

/// Whether a section formats a date or a time rather than a number.
fn is_date_pattern(section: &str) -> bool {
    let mut date_token = false;
    let mut digit_slot = false;
    for piece in pieces(section, false) {
        match piece {
            Piece::Number => digit_slot = true,
            Piece::Literal(_) | Piece::Percent | Piece::Token(_) => {}
        }
    }
    // The date letters have to be found outside quotes, which is what `pieces`
    // establishes: anything it kept as a literal is not a token.
    for piece in pieces(section, true) {
        if let Piece::Token(token) = piece {
            if token.starts_with(['y', 'm', 'd', 'h', 's']) || token.starts_with('[') {
                date_token = true;
            }
        }
    }
    date_token && !digit_slot
}

const MONTHS_SHORT: [&str; 12] = [
    "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
];
const DAYS_SHORT: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];

fn render_datetime(serial: f64, section: &str) -> String {
    let Some((y, m, d)) = serial_to_ymd(serial) else {
        return format_plain_number(serial);
    };
    let day_seconds = {
        let frac = serial - serial.floor();
        (frac * 86_400.0).round() as i64 % 86_400
    };
    let (hour24, minute, second) = (
        day_seconds / 3600,
        (day_seconds % 3600) / 60,
        day_seconds % 60,
    );
    // Serial 1 is a Monday in this calendar.
    let weekday = ((serial.floor() as i64) + 5).rem_euclid(7) as usize;
    let parts = pieces(section, true);
    let twelve_hour = parts
        .iter()
        .any(|p| matches!(p, Piece::Token(t) if t == "am/pm"));

    let mut out = String::new();
    for (index, piece) in parts.iter().enumerate() {
        match piece {
            Piece::Literal(text) => out.push_str(text),
            Piece::Percent => out.push('%'),
            Piece::Number => {}
            Piece::Token(token) => {
                // `m` is minutes when it follows an hour or precedes seconds,
                // and months otherwise — the one genuinely ambiguous token in
                // the whole format language.
                let neighbour_is_time = |offset: isize| {
                    let i = index as isize + offset;
                    usize::try_from(i)
                        .ok()
                        .and_then(|i| parts.get(i))
                        .map(|p| matches!(p, Piece::Token(t) if t.starts_with('h') || t.starts_with('s')))
                        .unwrap_or(false)
                };
                let text = match token.as_str() {
                    "yyyy" | "yyy" => y.to_string(),
                    "yy" | "y" => format!("{:02}", y.rem_euclid(100)),
                    "mmmmm" => MONTHS_SHORT[(m - 1) as usize]
                        .chars()
                        .take(1)
                        .collect::<String>(),
                    "mmmm" | "mmm" => MONTHS_SHORT[(m - 1) as usize].to_string(),
                    "mm" | "m" if neighbour_is_time(-1) || neighbour_is_time(1) => {
                        if token.len() == 2 {
                            pad2(minute as u32)
                        } else {
                            minute.to_string()
                        }
                    }
                    "mm" => pad2(m),
                    "m" => m.to_string(),
                    "dddd" => format!("{}요일", DAYS_SHORT[weekday]),
                    "ddd" => DAYS_SHORT[weekday].to_string(),
                    "dd" => pad2(d),
                    "d" => d.to_string(),
                    "hh" | "h" => {
                        let hour = if twelve_hour {
                            match hour24 % 12 {
                                0 => 12,
                                h => h,
                            }
                        } else {
                            hour24
                        };
                        if token.len() == 2 {
                            pad2(hour as u32)
                        } else {
                            hour.to_string()
                        }
                    }
                    "ss" => pad2(second as u32),
                    "s" => second.to_string(),
                    "am/pm" => if hour24 < 12 { "오전" } else { "오후" }.to_string(),
                    "[h]" => ((serial * 24.0).floor() as i64).to_string(),
                    "[m]" => ((serial * 1440.0).floor() as i64).to_string(),
                    "[s]" => ((serial * 86400.0).floor() as i64).to_string(),
                    other => other.to_string(),
                };
                out.push_str(&text);
            }
        }
    }
    out
}

/// The digit slots of a numeric section: decimals, grouping, and scaling.
struct Slots {
    decimals: usize,
    /// Trailing zeros the pattern does not require, as in `0.##`.
    optional: usize,
    grouping: bool,
    /// Each trailing comma divides by a thousand: `#,##0,` shows thousands.
    scale: f64,
}

fn slots_of(section: &str) -> Slots {
    // Only the digit runs matter, so the literals are stripped first.
    let cleaned: String = {
        let mut out = String::new();
        let mut quoted = false;
        let mut bracket = false;
        let mut chars = section.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => quoted = !quoted,
                '[' => bracket = true,
                ']' => bracket = false,
                '\\' | '_' | '*' => {
                    chars.next();
                }
                c if !quoted && !bracket => out.push(c),
                _ => {}
            }
        }
        out
    };

    let (integer_part, decimal_part) = match cleaned.split_once('.') {
        Some((a, b)) => (a, b),
        None => (cleaned.as_str(), ""),
    };
    // `0.00E+00` has two decimals, not four: the exponent's own slots are not
    // part of the mantissa.
    let decimal_part = decimal_part
        .split(['e', 'E'])
        .next()
        .unwrap_or(decimal_part);
    let decimals = decimal_part.chars().filter(|c| *c == '0').count();
    let optional = decimal_part
        .chars()
        .filter(|c| matches!(c, '#' | '?'))
        .count();
    let grouping = integer_part.contains(",");
    // A comma only scales when it sits at the end of the digit run.
    let trailing = integer_part
        .trim_end_matches(|c: char| !matches!(c, '#' | '0' | '?' | ','))
        .chars()
        .rev()
        .take_while(|c| *c == ',')
        .count();
    Slots {
        decimals,
        optional,
        grouping,
        scale: 1000f64.powi(trailing as i32),
    }
}

fn render_number(magnitude: f64, negative: bool, section: &str) -> String {
    let parts = pieces(section, false);
    let percent = parts.iter().any(|p| matches!(p, Piece::Percent));
    let scientific = parts
        .iter()
        .any(|p| matches!(p, Piece::Token(t) if t == "E"));
    let slots = slots_of(section);

    let mut scaled = magnitude;
    if percent {
        scaled *= 100.0;
    }
    if slots.scale > 1.0 {
        scaled /= slots.scale;
    }

    let body = if scientific {
        let formatted = format!("{:.*e}", slots.decimals, scaled);
        // Rust writes `1.2e5`; Excel writes `1.2E+05`.
        match formatted.split_once('e') {
            Some((mantissa, exponent)) => {
                let value: i32 = exponent.parse().unwrap_or(0);
                format!(
                    "{mantissa}E{}{:02}",
                    if value < 0 { '-' } else { '+' },
                    value.abs()
                )
            }
            None => formatted,
        }
    } else if slots.optional > 0 && slots.decimals == 0 {
        // `0.##` shows up to two decimals and drops the trailing zeros. The
        // integer part is the truncated value, not the rounded one, or 3.5 would
        // render as "4.5".
        let rounded = crate::functions::round_half_away(scaled, slots.optional as f64);
        let whole = jsnum::to_locale_string(rounded.trunc(), 0, slots.grouping);
        let frac = (rounded - rounded.trunc()).abs();
        if frac == 0.0 {
            whole
        } else {
            let text = format!("{:.*}", slots.optional, frac);
            format!("{whole}.{}", text[2..].trim_end_matches('0'))
        }
    } else {
        jsnum::to_locale_string(scaled, slots.decimals, slots.grouping)
    };

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    for piece in &parts {
        match piece {
            Piece::Literal(text) => out.push_str(text),
            Piece::Number => out.push_str(&body),
            Piece::Percent => out.push('%'),
            Piece::Token(_) => {}
        }
    }
    out
}

/// The text section of a format, where `@` stands for the text itself.
fn render_text(text: &str, section: &str) -> String {
    let mut out = String::new();
    for piece in pieces(section, false) {
        match piece {
            Piece::Literal(literal) => out.push_str(&literal.replace('@', text)),
            Piece::Percent => out.push('%'),
            _ => {}
        }
    }
    if out.is_empty() {
        text.to_string()
    } else {
        out
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

#[cfg(test)]
mod format_tests {
    use super::*;
    use crate::values::Value;

    fn at(value: f64, fmt: &str) -> String {
        apply_num_fmt(&Value::Number(value), fmt)
    }

    #[test]
    fn negative_sections_are_used() {
        // The accounting format every finance workbook is full of.
        assert_eq!(at(-45900.0, "#,##0_);[Red](#,##0)"), "(45,900)");
        assert_eq!(at(45900.0, "#,##0_);[Red](#,##0)"), "45,900 ");
        // A single section renders the sign in front, as Excel does.
        assert_eq!(at(-1234.0, "₩#,##0"), "-₩1,234");
    }

    #[test]
    fn a_zero_section_can_hide_or_replace_zero() {
        assert_eq!(at(0.0, "#,##0;-#,##0;\"-\""), "-");
        assert_eq!(at(0.0, "0;;"), "");
        assert_eq!(at(1.0, ";;;"), "", "every section empty hides the value");
    }

    #[test]
    fn quoted_literals_survive() {
        let serial = ymd_to_serial(2026, 9, 2).unwrap();
        assert_eq!(
            apply_num_fmt(&Value::Number(serial), "yyyy\"년\" m\"월\" d\"일\""),
            "2026년 9월 2일"
        );
        assert_eq!(at(5.0, "0\"개\""), "5개");
    }

    #[test]
    fn month_and_day_names_render() {
        let serial = ymd_to_serial(2026, 9, 2).unwrap();
        let f = |fmt: &str| apply_num_fmt(&Value::Number(serial), fmt);
        assert_eq!(f("yyyy-mm-dd (ddd)"), "2026-09-02 (수)");
        assert_eq!(f("dddd"), "수요일");
        assert_eq!(f("mmm"), "9월");
    }

    #[test]
    fn times_render_and_m_means_minutes_next_to_an_hour() {
        let serial = ymd_to_serial(2026, 9, 2).unwrap() + 13.0 / 24.0 + 9.0 / 1440.0;
        let f = |fmt: &str| apply_num_fmt(&Value::Number(serial), fmt);
        assert_eq!(f("hh:mm"), "13:09");
        assert_eq!(f("h:mm AM/PM"), "1:09 오후");
        // The same `mm` is a month when no hour is beside it.
        assert_eq!(f("yyyy-mm"), "2026-09");
    }

    #[test]
    fn currency_and_locale_brackets_show_their_symbol() {
        assert_eq!(at(1234.5, "[$₩-412]#,##0.00"), "₩1,234.50");
        assert_eq!(at(1234.5, "[Red]#,##0"), "1,235", "a colour shows nothing");
    }

    #[test]
    fn scaling_scientific_and_optional_decimals() {
        assert_eq!(
            at(1_234_567.0, "#,##0,"),
            "1,235",
            "a trailing comma is thousands"
        );
        assert_eq!(at(0.000123, "0.00E+00"), "1.23E-04");
        assert_eq!(at(3.5, "0.##"), "3.5");
        assert_eq!(at(3.0, "0.##"), "3");
    }

    #[test]
    fn the_text_placeholder_shows_the_value_as_text() {
        assert_eq!(at(1234.5, "@"), "1234.5");
        assert_eq!(
            apply_num_fmt(&Value::Text("서울".into()), "\"[\"@\"]\""),
            "[서울]"
        );
    }
}
