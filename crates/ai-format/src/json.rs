//! JSON shaping so the files on disk look like the format's documentation.

use serde_json::Value as Json;

/// Write integral floats as integers, the way `JSON.stringify` does.
///
/// Rust's f64 serializer always emits a decimal point, so a coordinate of 72
/// would land in the file as `72.0`. The format's geometry and cell values are
/// conceptually integers most of the time, and `git diff` on a project should
/// not churn just because the writer changed language.
pub fn compact_numbers(value: &mut Json) {
    match value {
        Json::Number(n) => {
            if let Some(f) = n.as_f64() {
                // Anything integral and inside `i64` converts exactly. Beyond
                // that serde keeps the float, which JSON allows and JS reads
                // back to the same value.
                if f.fract() == 0.0 && f.abs() < 9.0e18 {
                    *n = serde_json::Number::from(f as i64);
                }
            }
        }
        Json::Array(items) => items.iter_mut().for_each(compact_numbers),
        Json::Object(map) => map.iter_mut().for_each(|(_, v)| compact_numbers(v)),
        _ => {}
    }
}

/// `serde_json::to_string_pretty` with integral floats compacted.
pub fn to_pretty(value: &Json) -> String {
    let mut copy = value.clone();
    compact_numbers(&mut copy);
    serde_json::to_string_pretty(&copy).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integral_floats_lose_their_decimal_point() {
        let mut v = json!({ "x": 72.0, "y": 0.3, "z": [1.0, -5.0], "s": "72.0" });
        compact_numbers(&mut v);
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"x":72,"y":0.3,"z":[1,-5],"s":"72.0"}"#
        );
    }

    #[test]
    fn large_integral_floats_still_compact() {
        let mut v = json!(1e17);
        compact_numbers(&mut v);
        assert_eq!(serde_json::to_string(&v).unwrap(), "100000000000000000");
    }

    #[test]
    fn a_float_beyond_i64_is_left_alone() {
        let mut v = json!(1e20);
        compact_numbers(&mut v);
        assert_eq!(serde_json::to_string(&v).unwrap(), "1e+20");
    }
}
