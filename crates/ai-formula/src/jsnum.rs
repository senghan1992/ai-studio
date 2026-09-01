//! ECMAScript number formatting, reproduced exactly.
//!
//! The Rust port has to write the same bytes to disk as the JavaScript original
//! did, and the JS code leans on three `Number` behaviours that Rust's
//! formatters do not share:
//!
//! * `String(x)` uses exponent notation outside `1e-7 .. 1e21`, Rust never does.
//! * `x.toPrecision(12)` rounds to significant digits, Rust has no equivalent.
//! * `toLocaleString` rounds half away from zero, Rust's `{:.n}` rounds
//!   half to even — so `(0.5).toFixed(0)` is `1` in JS and `0` in Rust.
//!
//! Getting these wrong would not crash anything; it would silently change
//! `AI.md` and `.cells.json` for a handful of values, which is worse.

/// Shortest round-trippable digits of `x`, as (digit string, `n`) where the
/// value equals `0.<digits> × 10^n` — the `s`/`n` pair of ECMA-262 6.1.6.1.20.
fn shortest_digits(x: f64) -> (String, i32) {
    debug_assert!(x.is_finite() && x != 0.0);
    // Rust's LowerExp emits the same shortest round-trip mantissa as JS, in the
    // form `d.dddde±E`, so the digits can be lifted straight out of it.
    let s = format!("{:e}", x.abs());
    let (mantissa, exp) = s.split_once('e').expect("LowerExp always has an exponent");
    let exp: i32 = exp.parse().expect("LowerExp exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    (digits.to_string(), exp + 1)
}

/// `String(x)` for a JS number.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let (s, n) = shortest_digits(x);
    let k = s.len() as i32;

    let body = if k <= n && n <= 21 {
        // 12345 with n=7 -> "1234500"
        format!("{s}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        // digits with the point inserted after n of them
        format!("{}.{}", &s[..n as usize], &s[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{s}", "0".repeat((-n) as usize))
    } else {
        // exponent notation
        let e = n - 1;
        let esign = if e >= 0 { "+" } else { "-" };
        if k == 1 {
            format!("{s}e{esign}{}", e.abs())
        } else {
            format!("{}.{}e{esign}{}", &s[..1], &s[1..], e.abs())
        }
    };
    format!("{sign}{body}")
}

/// `Number(x.toPrecision(p))` — round to `p` significant digits, then back to
/// the nearest f64. The JS code always immediately re-parses, so the
/// intermediate string never escapes.
pub fn to_precision(x: f64, p: usize) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    // `{:.*e}` rounds the mantissa to `p-1` fractional digits = `p` significant.
    format!("{:.*e}", p - 1, x).parse().unwrap_or(x)
}

/// `x.toFixed(digits)` — fixed decimals, rounding half away from zero.
pub fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() {
        return number_to_string(x);
    }
    let negative = x < 0.0;
    let mag = x.abs();

    // Round on decimal digits, not in binary: by the time Rust's `{:.2}` sees
    // 1.005 the tie is already gone, so ask for one extra digit and do the
    // half-expand step by hand.
    let full = format!("{:.*}", digits + 1, mag);
    let (kept, last) = full.split_at(full.len() - 1);
    let last = last.as_bytes()[0];
    // `kept` is "12.34" for digits=2, or "12." for digits=0.
    let mut out = kept.trim_end_matches('.').to_string();

    if last >= b'5' {
        bump_decimal(&mut out);
    }

    // JS prints "-0.00", not "0.00", only when the input was actually negative.
    if negative {
        format!("-{out}")
    } else {
        out
    }
}

/// Increment a plain decimal string by one unit in its last place.
fn bump_decimal(s: &mut String) {
    let mut bytes: Vec<u8> = s.as_bytes().to_vec();
    let mut i = bytes.len();
    loop {
        if i == 0 {
            bytes.insert(0, b'1');
            break;
        }
        i -= 1;
        match bytes[i] {
            b'.' => continue,
            b'9' => bytes[i] = b'0',
            d => {
                bytes[i] = d + 1;
                break;
            }
        }
        if i == 0 {
            bytes.insert(0, b'1');
            break;
        }
    }
    *s = String::from_utf8(bytes).expect("ascii");
}

/// `x.toLocaleString('en-US', { minimumFractionDigits: d, maximumFractionDigits: d,
/// useGrouping })` for a non-negative `x`.
pub fn to_locale_string(x: f64, decimals: usize, grouping: bool) -> String {
    let fixed = to_fixed(x, decimals);
    let (int_part, frac_part) = match fixed.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (fixed, None),
    };
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(rest) => ("-", rest.to_string()),
        None => ("", int_part),
    };
    let grouped = if grouping {
        group_thousands(&digits)
    } else {
        digits
    };
    match frac_part {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_js_string_conversion() {
        assert_eq!(number_to_string(0.0), "0");
        assert_eq!(number_to_string(-0.0), "0");
        assert_eq!(number_to_string(42.0), "42");
        assert_eq!(number_to_string(-42.0), "-42");
        assert_eq!(number_to_string(0.1), "0.1");
        assert_eq!(number_to_string(1.5), "1.5");
        assert_eq!(number_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(number_to_string(1e21), "1e+21");
        assert_eq!(number_to_string(1e-7), "1e-7");
        assert_eq!(number_to_string(1e-6), "0.000001");
        assert_eq!(number_to_string(1.23e-8), "1.23e-8");
        assert_eq!(number_to_string(1e20), "100000000000000000000");
        assert_eq!(
            number_to_string(1.7976931348623157e308),
            "1.7976931348623157e+308"
        );
    }

    #[test]
    fn to_precision_kills_float_noise() {
        // The reason the JS code calls toPrecision(12) at all.
        assert_eq!(number_to_string(to_precision(0.1 + 0.2, 12)), "0.3");
        assert_eq!(
            number_to_string(to_precision(1.0 / 3.0, 12)),
            "0.333333333333"
        );
        assert_eq!(
            number_to_string(to_precision(1470000000.0, 14)),
            "1470000000"
        );
    }

    #[test]
    fn to_fixed_rounds_half_away_from_zero() {
        assert_eq!(to_fixed(0.5, 0), "1"); // Rust's {:.0} would say "0"
        assert_eq!(to_fixed(1.5, 0), "2");
        assert_eq!(to_fixed(2.5, 0), "3"); // and "2"
        assert_eq!(to_fixed(-2.5, 0), "-3");
        assert_eq!(to_fixed(-0.001, 2), "-0.00");
        assert_eq!(to_fixed(1.005, 2), "1.01");
        assert_eq!(to_fixed(0.0, 2), "0.00");
        assert_eq!(to_fixed(9.99, 1), "10.0");
        assert_eq!(to_fixed(9.999, 0), "10");
    }

    #[test]
    fn locale_string_groups_like_en_us() {
        assert_eq!(to_locale_string(1234567.0, 0, true), "1,234,567");
        assert_eq!(to_locale_string(1234567.0, 0, false), "1234567");
        assert_eq!(to_locale_string(1470000000.0, 0, true), "1,470,000,000");
        assert_eq!(to_locale_string(1234.5, 2, true), "1,234.50");
        assert_eq!(to_locale_string(48.0, 1, false), "48.0");
        assert_eq!(to_locale_string(100.0, 0, true), "100");
        assert_eq!(to_locale_string(999.999, 2, true), "1,000.00");
    }
}
