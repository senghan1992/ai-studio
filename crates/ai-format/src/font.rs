//! The one font this format draws with.
//!
//! An imported `.docx` asks for Calibri, 맑은 고딕, Batang and whatever else its
//! author had installed; a `.pptx` asks per run. Honouring those would mean
//! carrying a font name on every block and still drawing the wrong thing,
//! because the reader's machine has a different set installed than the author's.
//!
//! So the format *draws* exactly one family. It ships with the app, so a
//! document looks the same on Windows, macOS and Linux. Import records which
//! families it replaced rather than dropping them silently — and carries the
//! author's family name along as `style.font`, so an export asks Office for the
//! font the document was written in rather than for one the reader's PC lacks.

/// The family name, as written into exported OOXML and into the manifest.
pub const FAMILY: &str = "Pretendard";

/// What CSS asks for: the bundled variable font first, then the closest thing
/// each OS ships, so text is still readable if the font file fails to load.
pub const STACK: &str = "\"Pretendard Variable\", Pretendard, -apple-system, \"Apple SD Gothic Neo\", \"Malgun Gothic\", \"Noto Sans KR\", system-ui, sans-serif";

/// Monospace, for code blocks and formulas. Not bundled: every OS has one.
pub const MONO: &str =
    "\"Cascadia Mono\", Consolas, \"SF Mono\", \"D2Coding\", ui-monospace, monospace";

/// True when a font name is one the app draws as monospace.
///
/// A code run keeps its meaning — it becomes `` `code` `` in the markdown — so
/// the one thing worth reading off a font name is whether it was monospaced.
pub fn is_mono(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "consol", "courier", "mono", "d2coding", "menlo", "monaco", "cascadia",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// True when a font name is a glyph font rather than a text font.
///
/// `Symbol` and `Wingdings` appear in a Word file as the font of a bullet
/// character, never of prose. The bullet becomes a markdown list marker here, so
/// there is nothing to substitute and nothing to report.
pub fn is_symbol(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["symbol", "wingding", "webding", "dingbat"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Points → the px the editor stores, chosen so the size survives a round trip.
///
/// The editor works in whole px. 10pt is 13.33px and rounds to 13, which
/// [`pt_for_px`] maps back to 10 — so a whole number is kept whenever it maps
/// back to the size it came from. 9.5pt is 12.67px, and 13px would come back as
/// 10pt; for such half-point sizes the px is kept to two decimals instead, which
/// the grid and the canvas draw just as well.
pub fn px_for_pt(pt: f64) -> f64 {
    let px = pt / 0.75;
    let whole = px.round();
    if (pt_for_px(whole) - pt).abs() < 1e-6 {
        whole
    } else {
        (px * 100.0).round() / 100.0
    }
}

/// px → points for a font size, snapped to what Office offers.
///
/// A whole-px size is off by at most 0.375pt from the point size it was made
/// from, so the nearest whole point is that size; anything further away is a
/// genuine half-point size and is kept to 0.5.
pub fn pt_for_px(px: f64) -> f64 {
    let points = px * 0.75;
    let whole = points.round();
    if (points - whole).abs() <= 0.375 + 1e-9 {
        whole
    } else {
        (points * 2.0).round() / 2.0
    }
}

/// Whether a font name needs reporting when an import replaces it.
///
/// A theme placeholder (`+mn-lt`), an empty name and the family we were going to
/// use anyway are all no-ops, and listing them would bury the real substitutions
/// under noise.
pub fn is_substitution(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('+')
        && !trimmed.eq_ignore_ascii_case(FAMILY)
        && !is_mono(trimmed)
        && !is_symbol(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_names_are_recognised() {
        assert!(is_mono("Consolas"));
        assert!(is_mono("Courier New"));
        assert!(is_mono("D2Coding"));
        assert!(!is_mono("맑은 고딕"));
    }

    #[test]
    fn only_real_substitutions_are_reported() {
        assert!(is_substitution("Calibri"));
        assert!(is_substitution("맑은 고딕"));
        assert!(
            !is_substitution("+mn-lt"),
            "a theme reference names no font"
        );
        assert!(!is_substitution("  "));
        assert!(!is_substitution("pretendard"), "already the family we use");
        assert!(!is_substitution("Consolas"), "monospace survives as `code`");
        assert!(
            !is_substitution("Symbol"),
            "a bullet glyph is not the text font"
        );
    }
}
