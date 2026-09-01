//! Ids, slugs and the small naming helpers the format leans on.

use std::cell::Cell as StdCell;
use std::collections::HashSet;

const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Short, url-safe, human-typeable id.
pub fn short_id(len: usize) -> String {
    (0..len)
        .map(|_| ALPHABET[next_random(ALPHABET.len())] as char)
        .collect()
}

fn next_random(bound: usize) -> usize {
    thread_local! {
        static STATE: StdCell<u64> = const { StdCell::new(0) };
    }
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            // `SystemTime::now()` aborts on wasm32-unknown-unknown; chrono reads
            // the platform clock on native and the JS clock in the browser.
            x = chrono::Utc::now()
                .timestamp_nanos_opt()
                .map(|n| n as u64)
                .unwrap_or(0x9E3779B97F4A7C15)
                | 1;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 8) as usize % bound
    })
}

pub fn new_project_id() -> String {
    format!("prj_{}", short_id(7))
}
pub fn new_slide_id() -> String {
    format!("s_{}", short_id(5))
}
pub fn new_section_id() -> String {
    format!("sec_{}", short_id(5))
}
pub fn new_sheet_id() -> String {
    format!("sh_{}", short_id(5))
}
pub fn new_block_id() -> String {
    format!("b_{}", short_id(5))
}

/// Turn a title into a filesystem-safe slug, keeping unicode letters (한글 등).
pub fn slugify(title: &str, fallback: &str) -> String {
    // NFC normalization is what the JS `String.normalize('NFC')` did; Hangul
    // typed on macOS arrives decomposed and would otherwise produce a different
    // folder name than the same title typed on Windows.
    let normalized = nfc(title);
    let stripped: String = normalized
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '/' | '\\' | '?' | '%' | '*' | ':' | '|' | '"' | '<' | '>' | '.'
            )
        })
        .collect();

    let mut out = String::with_capacity(stripped.len());
    let mut pending_dash = false;
    for c in stripped.chars() {
        if c.is_whitespace() || c == '-' {
            pending_dash = true;
            continue;
        }
        if pending_dash && !out.is_empty() {
            out.push('-');
        }
        pending_dash = false;
        out.extend(c.to_lowercase());
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

/// Compose Hangul jamo sequences into syllables (Unicode NFC, Hangul subset).
///
/// A full NFC implementation would mean pulling in a Unicode tables crate; the
/// only decomposed input this format realistically sees is Hangul from macOS,
/// and the algorithmic Hangul composition is exact.
fn nfc(input: &str) -> String {
    const S_BASE: u32 = 0xAC00;
    const L_BASE: u32 = 0x1100;
    const V_BASE: u32 = 0x1161;
    const T_BASE: u32 = 0x11A7;
    const V_COUNT: u32 = 21;
    const T_COUNT: u32 = 28;
    const N_COUNT: u32 = V_COUNT * T_COUNT;

    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i] as u32;
        // L + V (+ T)
        if (L_BASE..L_BASE + 19).contains(&c) && i + 1 < chars.len() {
            let v = chars[i + 1] as u32;
            if (V_BASE..V_BASE + V_COUNT).contains(&v) {
                let mut syllable = S_BASE + (c - L_BASE) * N_COUNT + (v - V_BASE) * T_COUNT;
                let mut used = 2;
                if i + 2 < chars.len() {
                    let t = chars[i + 2] as u32;
                    if (T_BASE + 1..T_BASE + T_COUNT).contains(&t) {
                        syllable += t - T_BASE;
                        used = 3;
                    }
                }
                out.push(char::from_u32(syllable).expect("valid Hangul syllable"));
                i += used;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Zero-padded ordinal prefix for stable file sort: 1 -> "01".
pub fn pad(n: usize, width: usize) -> String {
    format!("{n:0width$}")
}

/// Ensure `id` is unique within `taken`, suffixing -2, -3 …
pub fn unique_id(id: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(id) {
        return id.to_string();
    }
    let mut i = 2;
    loop {
        let candidate = format!("{id}-{i}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_keep_hangul_and_drop_path_characters() {
        assert_eq!(slugify("2026 3분기 사업 리뷰", "x"), "2026-3분기-사업-리뷰");
        assert_eq!(slugify("A/B: Test?", "x"), "ab-test");
        assert_eq!(slugify("  ", "untitled"), "untitled");
        assert_eq!(slugify("Hello  World", "x"), "hello-world");
        assert_eq!(slugify("--a--b--", "x"), "a-b");
        assert_eq!(slugify("Report.v2", "x"), "reportv2");
    }

    #[test]
    fn decomposed_hangul_composes_to_the_same_slug() {
        // "한글" typed on macOS arrives as jamo.
        let decomposed = "\u{1112}\u{1161}\u{11AB}\u{1100}\u{1173}\u{11AF}";
        assert_eq!(slugify(decomposed, "x"), "한글");
        assert_eq!(slugify("한글", "x"), "한글");
    }

    #[test]
    fn ids_have_the_documented_shape() {
        assert!(new_slide_id().starts_with("s_"));
        assert_eq!(new_slide_id().len(), 7);
        assert_eq!(new_project_id().len(), 11);
        assert_eq!(pad(1, 2), "01");
        assert_eq!(pad(12, 2), "12");
        assert_eq!(pad(123, 2), "123");
    }

    #[test]
    fn unique_id_suffixes_collisions() {
        let taken: HashSet<String> = ["a".into(), "a-2".into()].into_iter().collect();
        assert_eq!(unique_id("b", &taken), "b");
        assert_eq!(unique_id("a", &taken), "a-3");
    }
}
