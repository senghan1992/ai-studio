//! YAML frontmatter, split from and re-attached to a markdown body.

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value as Json;

/// The frontmatter block is a flat map in this format, so an ordered map of
/// JSON scalars is both sufficient and stable to re-emit.
pub type Meta = IndexMap<String, Json>;

static FM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---[ \t]*\r?\n?").unwrap());
static LEADING_BLANK: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\n").unwrap());

const BOM: char = '\u{FEFF}';

pub struct Split {
    pub meta: Meta,
    pub body: String,
}

/// Split a markdown file into YAML frontmatter and body.
///
/// Missing or malformed frontmatter yields an empty map rather than an error —
/// hand-edited files should still open.
pub fn parse_frontmatter(text: &str) -> Split {
    let src = text.strip_prefix(BOM).unwrap_or(text);
    let Some(m) = FM_RE.captures(src) else {
        return Split {
            meta: Meta::new(),
            body: LEADING_BLANK.replace(src, "").into_owned(),
        };
    };
    let yaml = m.get(1).unwrap().as_str();
    let meta = parse_yaml_map(yaml).unwrap_or_default();
    Split {
        meta,
        body: src[m.get(0).unwrap().end()..].to_string(),
    }
}

fn parse_yaml_map(yaml: &str) -> Option<Meta> {
    let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).ok()?;
    let map = parsed.as_mapping()?;
    let mut out = Meta::new();
    for (k, v) in map {
        let key = match k {
            serde_yaml_ng::Value::String(s) => s.clone(),
            other => serde_yaml_ng::to_string(other).ok()?.trim().to_string(),
        };
        out.insert(key, yaml_to_json(v));
    }
    Some(out)
}

fn yaml_to_json(v: &serde_yaml_ng::Value) -> Json {
    use serde_yaml_ng::Value as Y;
    match v {
        Y::Null => Json::Null,
        Y::Bool(b) => Json::Bool(*b),
        Y::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Y::String(s) => Json::String(s.clone()),
        Y::Sequence(items) => Json::Array(items.iter().map(yaml_to_json).collect()),
        Y::Mapping(map) => Json::Object(
            map.iter()
                .map(|(k, v)| (k.as_str().unwrap_or_default().to_string(), yaml_to_json(v)))
                .collect(),
        ),
        Y::Tagged(t) => yaml_to_json(&t.value),
    }
}

/// Re-attach frontmatter. Empty meta emits no fence.
pub fn serialize_frontmatter(meta: &Meta, body: &str) -> String {
    let text = body.trim_end();
    let entries: Vec<(&String, &Json)> = meta
        .iter()
        .filter(|(_, v)| !matches!(v, Json::Null) && !matches!(v, Json::String(s) if s.is_empty()))
        .collect();

    if entries.is_empty() {
        return if text.is_empty() {
            String::new()
        } else {
            format!("{text}\n")
        };
    }

    let mut yaml = String::new();
    for (key, value) in entries {
        yaml.push_str(&emit_entry(key, value));
    }
    format!("---\n{}\n---\n\n{text}\n", yaml.trim_end())
}

fn emit_entry(key: &str, value: &Json) -> String {
    match value {
        Json::String(s) => format!("{key}: {}\n", scalar(s)),
        Json::Bool(b) => format!("{key}: {b}\n"),
        Json::Number(n) => format!("{key}: {n}\n"),
        Json::Array(items) => {
            let mut out = format!("{key}:\n");
            for item in items {
                out.push_str(&format!("  - {}\n", inline(item)));
            }
            out
        }
        Json::Object(map) => {
            let mut out = format!("{key}:\n");
            for (k, v) in map {
                out.push_str(&format!("  {k}: {}\n", inline(v)));
            }
            out
        }
        Json::Null => String::new(),
    }
}

fn inline(value: &Json) -> String {
    match value {
        Json::String(s) => scalar(s),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        Json::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Emit a YAML scalar, quoting only when the plain form would not round-trip.
///
/// `lineWidth: 0` in the original meant "never fold", so a long value stays on
/// one line; a value containing a newline has to become a quoted scalar.
fn scalar(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = s.contains('\n')
        || s.contains(": ")
        || s.ends_with(':')
        || s.contains(" #")
        || s.starts_with([
            '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '-', '?', ',', '[', ']', '{',
            '}',
        ])
        || s.trim() != s
        || is_yaml_scalar_keyword(s)
        || looks_numeric(s);

    if !needs_quotes {
        return s.to_string();
    }
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

fn is_yaml_scalar_keyword(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off"
    )
}

fn looks_numeric(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

/// Read a string field out of frontmatter.
pub fn meta_str(meta: &Meta, key: &str) -> Option<String> {
    match meta.get(key) {
        Some(Json::String(s)) => Some(s.clone()),
        Some(Json::Number(n)) => Some(n.to_string()),
        Some(Json::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta_of(pairs: &[(&str, Json)]) -> Meta {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn splits_frontmatter_from_body() {
        let src = "---\nid: s_9k2mx\ntitle: 핵심 성과\n---\n\n# 핵심 성과\n";
        let split = parse_frontmatter(src);
        assert_eq!(meta_str(&split.meta, "id").as_deref(), Some("s_9k2mx"));
        assert_eq!(meta_str(&split.meta, "title").as_deref(), Some("핵심 성과"));
        // The fence match consumes only one trailing newline, so the blank
        // separator line stays with the body — `split_blocks` trims it.
        assert_eq!(split.body, "\n# 핵심 성과\n");
    }

    #[test]
    fn a_file_without_frontmatter_still_opens() {
        let split = parse_frontmatter("\n# Just markdown\n");
        assert!(split.meta.is_empty());
        assert_eq!(split.body, "# Just markdown\n");
    }

    #[test]
    fn malformed_yaml_yields_no_meta_rather_than_an_error() {
        let split = parse_frontmatter("---\n: : :\n---\nbody\n");
        assert!(split.meta.is_empty());
        assert_eq!(split.body, "body\n");
    }

    #[test]
    fn a_byte_order_mark_is_tolerated() {
        let split = parse_frontmatter("\u{FEFF}---\nid: a\n---\nbody\n");
        assert_eq!(meta_str(&split.meta, "id").as_deref(), Some("a"));
    }

    #[test]
    fn round_trips_through_serialize() {
        let meta = meta_of(&[("id", json!("s_1")), ("title", json!("표: 매출"))]);
        let text = serialize_frontmatter(&meta, "# 본문\n\n내용");
        let back = parse_frontmatter(&text);
        assert_eq!(meta_str(&back.meta, "title").as_deref(), Some("표: 매출"));
        assert_eq!(back.body.trim(), "# 본문\n\n내용");
    }

    #[test]
    fn empty_meta_emits_no_fence() {
        assert_eq!(serialize_frontmatter(&Meta::new(), "body"), "body\n");
        assert_eq!(serialize_frontmatter(&Meta::new(), "  \n"), "");
    }

    #[test]
    fn null_and_empty_fields_are_dropped() {
        let meta = meta_of(&[("id", json!("a")), ("notes", json!("")), ("x", json!(null))]);
        assert_eq!(serialize_frontmatter(&meta, "b"), "---\nid: a\n---\n\nb\n");
    }

    #[test]
    fn multiline_notes_survive_the_round_trip() {
        let meta = meta_of(&[("notes", json!("첫 줄\n둘째 줄"))]);
        let text = serialize_frontmatter(&meta, "body");
        let back = parse_frontmatter(&text);
        assert_eq!(
            meta_str(&back.meta, "notes").as_deref(),
            Some("첫 줄\n둘째 줄")
        );
    }

    #[test]
    fn a_numeric_looking_title_stays_a_string() {
        let meta = meta_of(&[("title", json!("2026"))]);
        let text = serialize_frontmatter(&meta, "b");
        assert!(text.contains("title: \"2026\""), "{text}");
        let back = parse_frontmatter(&text);
        assert_eq!(meta_str(&back.meta, "title").as_deref(), Some("2026"));
    }
}
