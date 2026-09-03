//! Segmenting marker-free markdown into block elements.

use once_cell::sync::Lazy;
use regex::Regex;

static FENCE_OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*(`{3,}|~{3,})").unwrap());
static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"^#{1,6}[ \t]+").unwrap());
static HR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*(?:---+|\*\*\*+|___+)[ \t]*$").unwrap());
static LIST_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[ \t]*(?:[-+*]|\d+[.)])[ \t]+").unwrap());
static QUOTE_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*>").unwrap());
static TABLE_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*\|").unwrap());
static LIST_CONTINUATION: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]{2,}\S").unwrap());

/// The block types a Doc paragraph can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockType {
    Heading,
    Paragraph,
    List,
    Table,
    Quote,
    Code,
    Image,
    Hr,
}

/// How much larger a markdown heading draws than the text around it.
///
/// These are the stylesheet's `em` factors for `.md h1`…`h4`. Export reads them
/// so a heading leaves the app the size it was on screen: a 54px block whose
/// markdown is `# 제목` draws at 103px, and writing 54pt into the `.pptx` would
/// halve every title on the way out.
pub const HEADING_EM: [f64; 6] = [1.9, 1.5, 1.2, 1.05, 1.0, 1.0];

/// How much smaller each nested list level draws on a slide.
///
/// PowerPoint's own master steps a body placeholder 32 · 28 · 24 · 20pt, which is
/// close to seven eighths a level. Word does not do this, so it applies to slides
/// only — a Word list stays one size however deep it goes.
pub const NESTED_LIST_EM: f64 = 0.875;

pub fn heading_em(level: usize) -> f64 {
    HEADING_EM
        .get(level.saturating_sub(1))
        .copied()
        .unwrap_or(1.0)
}

impl BlockType {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockType::Heading => "heading",
            BlockType::Paragraph => "paragraph",
            BlockType::List => "list",
            BlockType::Table => "table",
            BlockType::Quote => "quote",
            BlockType::Code => "code",
            BlockType::Image => "image",
            BlockType::Hr => "hr",
        }
    }

    pub fn from_name(s: &str) -> Option<BlockType> {
        Some(match s {
            "heading" => BlockType::Heading,
            "paragraph" => BlockType::Paragraph,
            "list" => BlockType::List,
            "table" => BlockType::Table,
            "quote" => BlockType::Quote,
            "code" => BlockType::Code,
            "image" => BlockType::Image,
            "hr" => BlockType::Hr,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MdBlock {
    pub md: String,
    pub block_type: BlockType,
}

/// Segment plain markdown into top-level block elements.
///
/// A flow document (Doc) needs paragraph-level blocks for the editor, but its
/// markdown is deliberately marker-free. So we recover block boundaries the way
/// a markdown parser would: blank lines separate, and runs of list items / table
/// rows / quote lines stay together as one logical block.
pub fn split_markdown_blocks(md: &str) -> Vec<MdBlock> {
    let mut out: Vec<MdBlock> = Vec::new();
    let mut buf: Vec<&str> = Vec::new();
    let mut fence: Option<String> = None;
    let mut run: Option<BlockType> = None;

    macro_rules! flush {
        () => {{
            let text = buf.join("\n");
            let text = text.trim_end_matches([' ', '\t', '\n', '\r']);
            if !is_blank(text) {
                out.push(MdBlock {
                    md: text.to_string(),
                    block_type: run.unwrap_or_else(|| classify(text)),
                });
            }
            buf.clear();
            #[allow(unused_assignments)]
            {
                run = None;
            }
        }};
    }

    for line in md.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if let Some(open) = fence.clone() {
            buf.push(line);
            if line.trim_start().starts_with(&open) {
                fence = None;
                flush!();
            }
            continue;
        }

        if let Some(f) = FENCE_OPEN.captures(line) {
            flush!();
            let marker = f.get(1).unwrap().as_str();
            fence = Some(marker.chars().next().unwrap().to_string().repeat(3));
            buf.push(line);
            continue;
        }

        if is_blank(line) {
            flush!();
            continue;
        }

        if HEADING.is_match(line) {
            flush!();
            buf.push(line);
            flush!();
            continue;
        }

        if HR.is_match(line) {
            flush!();
            buf.push(line);
            run = Some(BlockType::Hr);
            flush!();
            continue;
        }

        if let Some(kind) = line_run(line) {
            // A different run type starting mid-block begins a new block.
            if run.is_some() && run != Some(kind) {
                flush!();
            }
            run = Some(kind);
            buf.push(line);
            continue;
        }

        // Indented continuation of a list item belongs to the list.
        if run == Some(BlockType::List) && LIST_CONTINUATION.is_match(line) {
            buf.push(line);
            continue;
        }
        if run.is_some() {
            flush!();
        }
        buf.push(line);
    }
    flush!();
    out
}

fn line_run(line: &str) -> Option<BlockType> {
    if LIST_LINE.is_match(line) {
        return Some(BlockType::List);
    }
    if TABLE_LINE.is_match(line) {
        return Some(BlockType::Table);
    }
    if QUOTE_LINE.is_match(line) {
        return Some(BlockType::Quote);
    }
    None
}

static LONE_IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^!\[[^\]]*\]\([^)]*\)$").unwrap());

fn classify(text: &str) -> BlockType {
    let t = text.trim();
    if HEADING.is_match(t) {
        return BlockType::Heading;
    }
    if t.starts_with("```") || t.starts_with("~~~") {
        return BlockType::Code;
    }
    if t.starts_with('|') {
        return BlockType::Table;
    }
    if t.starts_with('>') {
        return BlockType::Quote;
    }
    if LIST_LINE.is_match(t) {
        return BlockType::List;
    }
    if LONE_IMAGE.is_match(t) {
        return BlockType::Image;
    }
    BlockType::Paragraph
}

static HEADING_MARK: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(#{1,6})[ \t]+").unwrap());

/// Heading level, or 0 for non-headings.
pub fn heading_level(md: &str) -> usize {
    HEADING_MARK
        .captures(md.trim())
        .map(|c| c.get(1).unwrap().as_str().len())
        .unwrap_or(0)
}

static RE_CODE_FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static RE_LITERAL_MARKER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\s*)(\d{1,3}[.)]|[-+*]|#{1,6}|>)(\s)").unwrap());
static RE_ESCAPED_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\([!-/:-@\[-`{-~])").unwrap());

/// Blank in the markdown sense: only ASCII spaces and tabs. A no-break space
/// (U+00A0) is content — it is how an imported slide keeps an empty line.
pub fn is_blank(text: &str) -> bool {
    text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// Keep a line that merely *looks* like markdown structure literal.
///
/// A slide title typed as `2. 클라우드 전환` or a step label `0. 배경` is plain
/// text in PowerPoint, but as markdown it is an ordered list — and an export
/// would renumber it from 1. Escaping the marker (`2\. 클라우드 전환`) keeps the
/// renderer, the exporters and the file all reading it as the author typed it.
/// Real bullets never come through here: the importers mark those with `- `.
pub fn escape_literal_marker(text: &str) -> String {
    // Line by line: a paragraph with Shift+Enter breaks has several starts.
    text.split('\n')
        .map(escape_line_marker)
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_line_marker(line: &str) -> String {
    RE_LITERAL_MARKER
        .replace(line, |c: &regex::Captures| {
            let marker = &c[2];
            // `2.` escapes its dot; a single-character marker escapes itself.
            let escaped = if marker.len() > 1
                && marker.chars().next().is_some_and(|ch| ch.is_ascii_digit())
            {
                let (digits, punct) = marker.split_at(marker.len() - 1);
                format!("{digits}\\{punct}")
            } else {
                format!("\\{marker}")
            };
            format!("{}{escaped}{}", &c[1], &c[3])
        })
        .into_owned()
}
static RE_CODE_SPAN: Lazy<Regex> = Lazy::new(|| Regex::new(r"`([^`]*)`").unwrap());
static RE_IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"!\[([^\]]*)\]\([^)]*\)").unwrap());
static RE_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());
static RE_HEADING_PREFIX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^[ \t]*#{1,6}[ \t]+").unwrap());
static RE_QUOTE_PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^[ \t]*>[ \t]?").unwrap());
static RE_LIST_PREFIX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^[ \t]*(?:[-+*]|\d+[.)])[ \t]+").unwrap());
static RE_STRONG: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*([^*]*)\*\*").unwrap());
static RE_EM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*([^*]*)\*").unwrap());
static RE_STRIKE: Lazy<Regex> = Lazy::new(|| Regex::new(r"~~([^~]*)~~").unwrap());
/// The one inline HTML the importers write: a coloured run.
static RE_SPAN_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"</?(?:span|u)\b[^>]*>").unwrap());
static RE_TABLE_EDGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^[ \t]*\|").unwrap());
static RE_SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]+").unwrap());

/// Strip markdown syntax to plain text, for word counts and outlines.
pub fn plain_text(md: &str) -> String {
    let s = RE_CODE_FENCE.replace_all(md, " ");
    let s = RE_CODE_SPAN.replace_all(&s, "$1");
    let s = RE_IMAGE.replace_all(&s, "$1");
    let s = RE_LINK.replace_all(&s, "$1");
    let s = RE_HEADING_PREFIX.replace_all(&s, "");
    let s = RE_QUOTE_PREFIX.replace_all(&s, "");
    let s = RE_LIST_PREFIX.replace_all(&s, "");
    let s = RE_STRONG.replace_all(&s, "$1");
    let s = RE_EM.replace_all(&s, "$1");
    let s = RE_STRIKE.replace_all(&s, "$1");
    let s = RE_SPAN_TAG.replace_all(&s, "");
    let s = RE_TABLE_EDGE.replace_all(&s, "");
    let s = RE_ESCAPED_PUNCT.replace_all(&s, "$1");
    let s = s.replace('|', " ");
    let s = RE_SPACES.replace_all(&s, " ");
    s.trim().to_string()
}

/// True for a character that is a word on its own — CJK carries no spaces.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF   // Hangul Jamo
        | 0x3000..=0x303F // CJK punctuation
        | 0x3040..=0x30FF // Hiragana, Katakana
        | 0x3130..=0x318F // Hangul compatibility jamo
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0x4E00..=0x9FFF // CJK unified ideographs
    )
}

pub fn count_words(text: &str) -> usize {
    let t = plain_text(text);
    if t.is_empty() {
        return 0;
    }
    let cjk = t.chars().filter(|c| is_cjk(*c)).count();
    // Latin runs, with CJK replaced by separators so the two counts do not overlap.
    let latin_source: String = t.chars().map(|c| if is_cjk(c) { ' ' } else { c }).collect();
    let latin = latin_source
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '\'' || c == '\u{2019}' || c == '-'))
        .filter(|w| !w.is_empty())
        .count();
    cjk + latin
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_literal_marker_is_escaped_and_reads_back_plain() {
        assert_eq!(
            escape_literal_marker("2. 클라우드 전환"),
            "2\\. 클라우드 전환"
        );
        assert_eq!(escape_literal_marker("0) 배경"), "0\\) 배경");
        assert_eq!(escape_literal_marker("- 항목"), "\\- 항목");
        assert_eq!(escape_literal_marker("# 제목"), "\\# 제목");
        assert_eq!(
            escape_literal_marker("2026. 계획"),
            "2026. 계획",
            "years are not markers"
        );
        assert_eq!(escape_literal_marker("보통 문장"), "보통 문장");
        assert_eq!(plain_text("2\\. 클라우드 전환"), "2. 클라우드 전환");
    }

    use super::*;

    fn types(md: &str) -> Vec<&'static str> {
        split_markdown_blocks(md)
            .iter()
            .map(|b| b.block_type.as_str())
            .collect()
    }

    #[test]
    fn a_blank_line_separates_paragraphs() {
        let blocks = split_markdown_blocks("첫 문단\n\n둘째 문단\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].md, "첫 문단");
        assert_eq!(blocks[1].md, "둘째 문단");
    }

    #[test]
    fn a_heading_is_always_its_own_block() {
        assert_eq!(
            types("본문\n# 제목\n다음 본문"),
            ["paragraph", "heading", "paragraph"]
        );
    }

    #[test]
    fn runs_of_list_items_stay_one_block() {
        let blocks = split_markdown_blocks("- a\n- b\n- c\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, BlockType::List);
        assert_eq!(blocks[0].md, "- a\n- b\n- c");
    }

    #[test]
    fn an_indented_continuation_stays_with_its_list_item() {
        let blocks = split_markdown_blocks("- a\n  이어지는 줄\n- b\n");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].md.contains("이어지는 줄"));
    }

    #[test]
    fn a_table_and_a_list_touching_split_apart() {
        assert_eq!(types("- a\n| A |\n|---|\n"), ["list", "table"]);
    }

    #[test]
    fn a_fenced_block_survives_intact() {
        let blocks = split_markdown_blocks("```js\nconst a = 1;\n\nconst b = 2;\n```\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, BlockType::Code);
        assert!(blocks[0].md.contains("const b"));
    }

    #[test]
    fn a_horizontal_rule_is_its_own_block() {
        assert_eq!(types("a\n\n---\n\nb"), ["paragraph", "hr", "paragraph"]);
    }

    #[test]
    fn heading_levels_are_counted() {
        assert_eq!(heading_level("### 제목"), 3);
        assert_eq!(heading_level("본문"), 0);
        assert_eq!(heading_level("#제목"), 0); // no space: not a heading
    }

    #[test]
    fn plain_text_strips_markdown() {
        assert_eq!(plain_text("# 제목"), "제목");
        assert_eq!(plain_text("**굵게** 그리고 *기울임*"), "굵게 그리고 기울임");
        assert_eq!(plain_text("- 항목 `코드`"), "항목 코드");
        assert_eq!(plain_text("![alt](a.png)"), "alt");
        assert_eq!(plain_text("| A | B |"), "A B");
        assert_eq!(plain_text("> 인용"), "인용");
    }

    #[test]
    fn word_counts_treat_cjk_per_character() {
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("안녕하세요"), 5);
        // Mixed: 4 Hangul characters + 1 latin word.
        assert_eq!(count_words("매출 142억 growth"), 5);
        assert_eq!(count_words(""), 0);
    }
}
