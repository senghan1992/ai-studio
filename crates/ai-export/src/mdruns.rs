//! Markdown into a small block/run tree that both the pptx and docx exporters
//! walk.
//!
//! This is deliberately not a full markdown implementation — it covers exactly
//! the constructs the editors can produce, and everything else falls through as
//! plain text rather than being dropped.

use once_cell::sync::Lazy;
use regex::Regex;

use ai_format::mdblocks::split_markdown_blocks;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
    /// From an inline `<span style="color:#…">`: a run coloured unlike its block.
    pub color: Option<String>,
    /// From `font-family:` in the same span.
    pub font: Option<String>,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            ..Run::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// Nesting depth, from the source indentation.
    pub level: usize,
    /// This item's own marker: `1.` numbered, `-` a bullet. A slide mixes
    /// the two in one block routinely, so the kind is not the list's.
    pub ordered: bool,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: usize,
        runs: Vec<Run>,
    },
    Paragraph {
        runs: Vec<Run>,
    },
    Quote {
        runs: Vec<Run>,
    },
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },
    Code {
        lang: String,
        text: String,
    },
    Table {
        rows: Vec<Vec<Vec<Run>>>,
    },
    Image {
        alt: String,
        src: String,
    },
    Hr,
    /// `<!-- page-break -->` — the editor's Ctrl+Enter.
    PageBreak,
}

static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(#{1,6})[ \t]+(.*)$").unwrap());
static FENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)^(?:```|~~~)([^\n]*)\n(.*?)\n?(?:```|~~~)?$").unwrap());
static HR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(?:---+|\*\*\*+|___+)$").unwrap());
static BLOCK_IMAGE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)$"#).unwrap());
static TABLE_RULE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\|[\s:|-]*-[\s:|-]*\|?$").unwrap());
static QUOTE_PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*>[ \t]?").unwrap());
static LIST_START: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([-+*]|\d+[.)])[ \t]+").unwrap());
/// One space after the marker belongs to the syntax; any more are the item's
/// own text — an author's manual indentation, kept as typed.
static LIST_ITEM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([ \t]*)([-+*]|\d+[.)])[ \t](.*)$").unwrap());

pub fn parse_markdown(md: &str) -> Vec<Block> {
    split_markdown_blocks(md)
        .into_iter()
        .flat_map(|part| split_leading_paragraph(&part.md))
        .filter_map(|piece| to_block(&piece))
        .collect()
}

/// A block whose first lines are prose and whose later lines are list items —
/// a heading-like line with its bullets right under it, as a slide writes it —
/// is a paragraph followed by a list, the way every renderer already draws it.
/// Reading the whole block as a paragraph turned the bullets into literal text.
fn split_leading_paragraph(md: &str) -> Vec<String> {
    let trimmed = md.trim_start();
    if trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || trimmed.starts_with('|')
        || trimmed.starts_with('>')
        || LIST_START.is_match(trimmed)
    {
        return vec![md.to_string()];
    }
    let lines: Vec<&str> = md.lines().collect();
    match lines.iter().position(|l| LIST_ITEM.is_match(l)) {
        Some(at) if at > 0 => vec![lines[..at].join("\n"), lines[at..].join("\n")],
        _ => vec![md.to_string()],
    }
}

/// The row's closing `|` — but a trailing `\|` is the last cell's own text.
fn strip_unescaped_pipe_suffix(body: &str) -> &str {
    if body.ends_with('|') && !body.ends_with("\\|") {
        &body[..body.len() - 1]
    } else {
        body
    }
}

/// A table row's cells, honouring the `\|` escape a cell uses to keep a pipe.
fn split_cells(body: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }
    cells.push(cell);
    cells
}

fn to_block(text: &str) -> Option<Block> {
    let trimmed = text.trim();

    if trimmed == "<!-- page-break -->" {
        return Some(Block::PageBreak);
    }

    if let Some(c) = HEADING.captures(trimmed) {
        return Some(Block::Heading {
            level: c.get(1).unwrap().as_str().len(),
            runs: inline_runs(c.get(2).unwrap().as_str()),
        });
    }

    if let Some(c) = FENCE.captures(trimmed) {
        return Some(Block::Code {
            lang: c.get(1).unwrap().as_str().trim().to_string(),
            text: c.get(2).unwrap().as_str().to_string(),
        });
    }

    if HR.is_match(trimmed) {
        return Some(Block::Hr);
    }

    if let Some(c) = BLOCK_IMAGE.captures(trimmed) {
        return Some(Block::Image {
            alt: c.get(1).unwrap().as_str().to_string(),
            src: c.get(2).unwrap().as_str().to_string(),
        });
    }

    if trimmed.starts_with('|') {
        let rows: Vec<Vec<Vec<Run>>> = trimmed
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('|') && !TABLE_RULE.is_match(l))
            .map(|l| {
                let body = l.strip_prefix('|').unwrap_or(l);
                let body = strip_unescaped_pipe_suffix(body);
                split_cells(body)
                    .iter()
                    .map(|cell| inline_runs(cell.trim()))
                    .collect()
            })
            .collect();
        if !rows.is_empty() {
            return Some(Block::Table { rows });
        }
    }

    if trimmed.starts_with('>') {
        let body: Vec<String> = trimmed
            .lines()
            .map(|l| QUOTE_PREFIX.replace(l, "").into_owned())
            .collect();
        return Some(Block::Quote {
            runs: inline_runs(&body.join("\n")),
        });
    }

    if let Some(start) = LIST_START.captures(trimmed) {
        let ordered = start
            .get(1)
            .unwrap()
            .as_str()
            .chars()
            .any(|c| c.is_ascii_digit());
        let mut items: Vec<ListItem> = Vec::new();
        // The original lines, not the trimmed block: a block holding only
        // sub-bullets starts indented, and trimming it dropped the first
        // item to level 0 while its siblings stayed at level 1.
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match LIST_ITEM.captures(line) {
                Some(c) => {
                    let indent = c.get(1).unwrap().as_str().replace('\t', "  ").len();
                    items.push(ListItem {
                        level: indent / 2,
                        ordered: c
                            .get(2)
                            .unwrap()
                            .as_str()
                            .starts_with(|ch: char| ch.is_ascii_digit()),
                        runs: inline_runs(c.get(3).unwrap().as_str()),
                    });
                }
                None => {
                    // A continuation line belongs to the previous item, as its
                    // own line: the author broke it there (Shift+Enter), and
                    // the spaces in front of it are theirs too.
                    if let Some(last) = items.last_mut() {
                        last.runs.push(Run::plain(format!("\n{}", line.trim_end())));
                    }
                }
            }
        }
        if !items.is_empty() {
            return Some(Block::List { ordered, items });
        }
    }

    if ai_format::mdblocks::is_blank(text) {
        return None;
    }
    // A newline inside a paragraph is a line break, not a space: the editor
    // renders markdown with hard breaks on, and an imported slide keeps each of
    // its original paragraphs on its own line this way. Both exporters turn the
    // `\n` into `<a:br>`/`<w:br/>`; folding it to a space merged a text box's
    // eight bullet lines into one run-on paragraph on export.
    // Spaces stay where the author put them — before a line, after it, and
    // an empty first line: PowerPoint shows all three, and trimming any of
    // them made the next re-import differ from this one.
    let lines: Vec<&str> = text.lines().collect();
    Some(Block::Paragraph {
        runs: inline_runs(&lines.join("\n")),
    })
}

/// Inline emphasis rules, longest delimiter first so `***x***` is not read as
/// `**` followed by a stray `*`.
struct InlineRule {
    re: Regex,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    /// `_` emphasis, which unlike `*` never applies inside a word.
    underscore: bool,
}

static INLINE_LINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());
static INLINE_IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^!\[([^\]]*)\]\([^)]*\)").unwrap());

static RULES: Lazy<Vec<InlineRule>> = Lazy::new(|| {
    let rule = |pattern: &str, bold, italic, strike, code, underscore| InlineRule {
        re: Regex::new(pattern).unwrap(),
        bold,
        italic,
        strike,
        code,
        underscore,
    };
    vec![
        rule(r"^`([^`]+)`", false, false, false, true, false),
        rule(r"^\*\*\*([^*]+)\*\*\*", true, true, false, false, false),
        rule(r"^\*\*([^*]+)\*\*", true, false, false, false, false),
        rule(r"^__([^_]+)__", true, false, false, false, true),
        rule(r"^~~([^~]+)~~", false, false, true, false, false),
        rule(r"^\*([^*]+)\*", false, true, false, false, false),
        rule(r"^_([^_]+)_", false, true, false, false, true),
    ]
});

static INLINE_SPAN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?s)<span style="([^"]*)">(.*?)</span>"#).unwrap());

/// Split inline markdown into styled runs.
///
/// An inline `<span style="color:#…;font-family:…">` — the one piece of HTML
/// the importers write, for a run whose colour or family differs from its
/// block — colours the runs inside it; everything else is plain markdown.
pub fn inline_runs(text: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut last = 0;
    for span in INLINE_SPAN.captures_iter(text) {
        let whole = span.get(0).unwrap();
        let before = &text[last..whole.start()];
        if !before.is_empty() {
            runs.extend(inline_runs_plain(before));
        }
        let (mut color, mut font) = (None, None);
        for declaration in span[1].split(';') {
            let Some((key, value)) = declaration.split_once(':') else {
                continue;
            };
            let value = value.trim().trim_matches(['"', '\'']).to_string();
            match key.trim() {
                "color" if !value.is_empty() => color = Some(value),
                "font-family" if !value.is_empty() => font = Some(value),
                _ => {}
            }
        }
        for mut run in inline_runs_plain(&span[2]) {
            run.color = run.color.or_else(|| color.clone());
            run.font = run.font.or_else(|| font.clone());
            runs.push(run);
        }
        last = whole.end();
    }
    let after = &text[last..];
    if last == 0 || !after.is_empty() {
        runs.extend(inline_runs_plain(after));
    }
    runs
}

/// Split inline markdown into styled runs — the markdown part alone.
///
/// Unmatched syntax characters stay as literal text — an exporter should never
/// silently swallow a stray asterisk.
fn inline_runs_plain(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;

    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                runs.push(Run::plain(std::mem::take(&mut plain)));
            }
        };
    }

    while i < text.len() {
        let rest = &text[i..];

        if let Some(c) = INLINE_LINK.captures(rest) {
            flush!();
            let label = c.get(1).unwrap().as_str();
            let url = c.get(2).unwrap().as_str();
            runs.push(Run {
                text: if label.is_empty() {
                    url.to_string()
                } else {
                    label.to_string()
                },
                link: Some(url.to_string()),
                ..Run::default()
            });
            i += c.get(0).unwrap().len();
            continue;
        }

        if let Some(c) = INLINE_IMAGE.captures(rest) {
            flush!();
            // Inline images become their alt text; block images are handled separately.
            let alt = c.get(1).unwrap().as_str();
            if !alt.is_empty() {
                runs.push(Run {
                    text: alt.to_string(),
                    italic: true,
                    ..Run::default()
                });
            }
            i += c.get(0).unwrap().len();
            continue;
        }

        // `\*`, `\|`, `\_`… — a backslash keeps the next punctuation literal,
        // exactly as the on-screen renderer treats it. Without this a salary
        // formula like `기본급\*1.5` loses its asterisk to emphasis.
        if let Some(after) = rest.strip_prefix('\\') {
            if let Some(next) = after.chars().next() {
                if next.is_ascii_punctuation() {
                    plain.push(next);
                    i += 1 + next.len_utf8();
                    continue;
                }
            }
        }

        // `<br>` inside a paragraph or a table cell is the format's own line
        // break; it becomes a newline the writers turn into a real break.
        if let Some(len) = ["<br />", "<br/>", "<br>"]
            .iter()
            .find(|br| {
                rest.get(..br.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(br))
            })
            .map(|br| br.len())
        {
            plain.push('\n');
            i += len;
            continue;
        }

        let mut matched = false;
        for rule in RULES.iter() {
            let Some(c) = rule.re.captures(rest) else {
                continue;
            };
            // CommonMark: `_` does not open or close emphasis inside a word,
            // so `인사_규정_v2` stays literal — as it does on screen.
            if rule.underscore {
                let before_ok = text[..i]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !ch.is_alphanumeric());
                let after = &text[i + c.get(0).unwrap().len()..];
                let after_ok = after.chars().next().is_none_or(|ch| !ch.is_alphanumeric());
                if !before_ok || !after_ok {
                    let ch = rest.chars().next().expect("non-empty");
                    plain.push(ch);
                    i += ch.len_utf8();
                    matched = true;
                    break;
                }
            }
            flush!();
            runs.push(Run {
                text: c.get(1).unwrap().as_str().to_string(),
                bold: rule.bold,
                italic: rule.italic,
                strike: rule.strike,
                code: rule.code,
                link: None,
                color: None,
                font: None,
            });
            i += c.get(0).unwrap().len();
            matched = true;
            break;
        }
        if matched {
            continue;
        }

        let c = rest.chars().next().expect("non-empty");
        plain.push(c);
        i += c.len_utf8();
    }
    flush!();

    if runs.is_empty() {
        vec![Run::plain("")]
    } else {
        runs
    }
}

/// Flatten runs back to plain text, for places that cannot carry formatting.
pub fn runs_to_text(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

/// Plain text of a whole block tree, one line per block.
pub fn blocks_to_text(blocks: &[Block]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { runs, .. } | Block::Paragraph { runs } | Block::Quote { runs } => {
                lines.push(runs_to_text(runs))
            }
            Block::PageBreak => {}
            Block::List { ordered, items } => {
                for (i, item) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}.", i + 1)
                    } else {
                        "•".to_string()
                    };
                    lines.push(format!("{marker} {}", runs_to_text(&item.runs)));
                }
            }
            Block::Code { text, .. } => lines.push(text.clone()),
            Block::Table { rows } => {
                for row in rows {
                    let cells: Vec<String> = row.iter().map(|c| runs_to_text(c)).collect();
                    lines.push(cells.join("\t"));
                }
            }
            Block::Image { alt, .. } => lines.push(if alt.is_empty() {
                "(이미지)".to_string()
            } else {
                alt.clone()
            }),
            Block::Hr => {}
        }
    }
    lines.join("\n")
}

/// The markdown rows of a table, flattened — what the pptx table export needs.
pub fn table_rows(md: &str) -> Vec<Vec<String>> {
    md.lines()
        .map(str::trim)
        .filter(|l| l.starts_with('|') && !TABLE_RULE.is_match(l))
        .map(|l| {
            let body = l.strip_prefix('|').unwrap_or(l);
            let body = strip_unescaped_pipe_suffix(body);
            split_cells(body)
                .iter()
                .map(|c| c.trim().replace("**", ""))
                .collect()
        })
        .collect()
}

static IMAGE_SRC: Lazy<Regex> = Lazy::new(|| Regex::new(r"!\[[^\]]*\]\(([^)\s]+)").unwrap());
static IMAGE_ALT: Lazy<Regex> = Lazy::new(|| Regex::new(r"!\[([^\]]*)\]").unwrap());

pub fn image_source(md: &str) -> Option<String> {
    IMAGE_SRC
        .captures(md)
        .map(|c| c.get(1).unwrap().as_str().to_string())
}

pub fn image_alt(md: &str) -> String {
    IMAGE_ALT
        .captures(md)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_becomes_runs() {
        let runs = inline_runs("일반 **굵게** 그리고 *기울임* 끝");
        assert_eq!(runs.len(), 5);
        assert_eq!(
            runs[1],
            Run {
                text: "굵게".into(),
                bold: true,
                ..Run::default()
            }
        );
        assert_eq!(
            runs[3],
            Run {
                text: "기울임".into(),
                italic: true,
                ..Run::default()
            }
        );
    }

    #[test]
    fn triple_asterisks_are_bold_italic() {
        let runs = inline_runs("***둘 다***");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].bold && runs[0].italic);
    }

    #[test]
    fn escapes_and_word_internal_underscores_stay_literal() {
        // The on-screen renderer's rules, mirrored: `\*` is an asterisk,
        // and `_` inside a word never becomes emphasis.
        let runs = inline_runs("통상임금\\*0.5\\*시간");
        assert_eq!(runs_to_text(&runs), "통상임금*0.5*시간");
        assert!(runs.iter().all(|r| !r.italic && !r.bold));

        let runs = inline_runs("인사_규정_v2와 emp_id_no");
        assert_eq!(runs_to_text(&runs), "인사_규정_v2와 emp_id_no");
        assert!(runs.iter().all(|r| !r.italic));

        // Boundary `_` still works.
        let runs = inline_runs("자 _기울임_ 끝");
        assert!(runs.iter().any(|r| r.italic && r.text == "기울임"));
    }

    #[test]
    fn a_br_becomes_a_newline_in_the_run() {
        let runs = inline_runs("1년 미만<br>월 1일 발생");
        assert_eq!(runs_to_text(&runs), "1년 미만\n월 1일 발생");
    }

    #[test]
    fn escaped_pipes_stay_inside_their_cell() {
        let rows = table_rows("| 직급 | 사원\\|대리\\|과장 |\n|---|---|\n| a | b |");
        assert_eq!(rows[0], vec!["직급", "사원|대리|과장"]);
        assert_eq!(rows[0].len(), 2);
    }

    #[test]
    fn a_page_break_comment_is_its_own_block() {
        let blocks = parse_markdown("<!-- page-break -->");
        assert!(matches!(blocks.as_slice(), [Block::PageBreak]));
    }

    #[test]
    fn a_stray_asterisk_survives_as_text() {
        assert_eq!(runs_to_text(&inline_runs("2 * 3 = 6")), "2 * 3 = 6");
        assert_eq!(runs_to_text(&inline_runs("**unclosed")), "**unclosed");
    }

    #[test]
    fn links_and_code_and_strike() {
        let runs = inline_runs("`code` ~~gone~~ [라벨](https://x.test)");
        assert!(runs[0].code);
        assert!(runs[2].strike);
        let link = runs.last().unwrap();
        assert_eq!(link.text, "라벨");
        assert_eq!(link.link.as_deref(), Some("https://x.test"));
    }

    #[test]
    fn a_bare_link_shows_its_url() {
        let runs = inline_runs("[](https://x.test)");
        assert_eq!(runs[0].text, "https://x.test");
    }

    #[test]
    fn headings_carry_their_level() {
        let blocks = parse_markdown("### 소제목");
        assert_eq!(
            blocks,
            vec![Block::Heading {
                level: 3,
                runs: inline_runs("소제목")
            }]
        );
    }

    #[test]
    fn lists_keep_nesting_and_ordering() {
        let blocks = parse_markdown("- 하나\n  - 중첩\n- 둘");
        let Block::List { ordered, items } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert!(!ordered);
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].level, 1);

        let blocks = parse_markdown("1. 하나\n2. 둘");
        let Block::List { ordered, .. } = &blocks[0] else {
            panic!()
        };
        assert!(ordered);
    }

    #[test]
    fn an_indented_continuation_joins_its_list_item() {
        // `split_markdown_blocks` keeps the indented line with the list, and the
        // run parser folds it into the item it continues.
        let blocks = parse_markdown("- 첫 항목\n  계속되는 줄");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(items.len(), 1);
        // A line break inside the item, with the author's indentation kept.
        assert_eq!(runs_to_text(&items[0].runs), "첫 항목\n  계속되는 줄");
    }

    #[test]
    fn a_newline_inside_a_paragraph_stays_a_line_break() {
        // An imported text box keeps each original paragraph on its own line;
        // the exporters turn `\n` into a break. Folding it to a space merged
        // eight bullet lines of a real slide into one run-on paragraph.
        let blocks = parse_markdown("첫 줄\n둘째 줄\n  셋째 줄");
        assert_eq!(blocks.len(), 1);
        let Block::Paragraph { runs } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(runs_to_text(runs), "첫 줄\n둘째 줄\n  셋째 줄");
    }

    #[test]
    fn a_coloured_span_colours_the_runs_inside_it() {
        // The importers mark a run coloured unlike its block this way; the
        // exporters need it back as a run colour, and the text stays plain.
        let runs = inline_runs(
            "<span style=\"color:#a50034;font-family:맑은 고딕\">**핵심** Pain</span> Point",
        );
        assert_eq!(runs_to_text(&runs), "핵심 Pain Point", "{runs:?}");
        assert_eq!(runs[0].color.as_deref(), Some("#a50034"));
        assert_eq!(runs[0].font.as_deref(), Some("맑은 고딕"));
        assert!(runs[0].bold, "markdown inside the span still applies");
        assert_eq!(runs[1].color.as_deref(), Some("#a50034"));
        let last = runs.last().unwrap();
        assert_eq!(
            last.color, None,
            "outside the span the block's colour rules"
        );
    }

    #[test]
    fn a_nested_bullet_keeps_its_level() {
        // An imported deck writes sub-bullets with two spaces; the export must
        // put them back on level 1, or every hierarchy flattens on the way out.
        let blocks =
            parse_markdown("- 정량적 성과\n  - 분석 단계 절감\n  - 데이터 취득시간\n- 정성적 성과");
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        let levels: Vec<usize> = items.iter().map(|i| i.level).collect();
        assert_eq!(levels, vec![0, 1, 1, 0]);

        // A block that is nothing but sub-bullets starts indented.
        let blocks = parse_markdown("  - 하위 하나\n  - 하위 둘");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(
            items.iter().map(|i| i.level).collect::<Vec<_>>(),
            vec![1, 1]
        );
    }

    #[test]
    fn a_heading_line_with_bullets_right_under_it_is_a_paragraph_then_a_list() {
        let blocks = parse_markdown(
            "<span style=\"color:#6b1f2a\">**Phase 1**</span>\n- 첫 항목\n- 둘째 항목",
        );
        assert_eq!(blocks.len(), 2, "{blocks:?}");
        assert!(matches!(blocks[0], Block::Paragraph { .. }));
        let Block::List { items, .. } = &blocks[1] else {
            panic!("{blocks:?}")
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn numbered_and_bulleted_items_keep_their_own_kind_in_one_list() {
        let blocks = parse_markdown("1. 안내\n- 세부 하나\n- 세부 둘");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(
            items.iter().map(|i| i.ordered).collect::<Vec<_>>(),
            vec![true, false, false]
        );
    }

    #[test]
    fn an_unindented_line_after_a_list_is_its_own_paragraph() {
        let blocks = parse_markdown("- 첫 항목\n계속되는 줄");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], Block::List { .. }));
        assert!(matches!(blocks[1], Block::Paragraph { .. }));
    }

    #[test]
    fn fenced_code_keeps_its_language_and_body() {
        let blocks = parse_markdown("```rust\nfn main() {}\n```");
        assert_eq!(
            blocks,
            vec![Block::Code {
                lang: "rust".into(),
                text: "fn main() {}".into()
            }]
        );
    }

    #[test]
    fn tables_become_run_grids() {
        let blocks = parse_markdown("| A | B |\n|---|---|\n| **1** | 2 |");
        let Block::Table { rows } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(rows.len(), 2, "the rule row is dropped");
        assert_eq!(runs_to_text(&rows[0][0]), "A");
        assert!(rows[1][0][0].bold);
    }

    #[test]
    fn quotes_and_rules_and_images() {
        assert_eq!(
            parse_markdown("> 인용문"),
            vec![Block::Quote {
                runs: inline_runs("인용문")
            }]
        );
        assert_eq!(parse_markdown("---"), vec![Block::Hr]);
        assert_eq!(
            parse_markdown("![대체 텍스트](../assets/a.png)"),
            vec![Block::Image {
                alt: "대체 텍스트".into(),
                src: "../assets/a.png".into()
            }]
        );
    }

    #[test]
    fn image_helpers_read_the_reference() {
        assert_eq!(
            image_source("![a](../assets/x.png)").as_deref(),
            Some("../assets/x.png")
        );
        assert_eq!(image_alt("![a](x.png)"), "a");
        assert_eq!(image_source("no image"), None);
    }

    #[test]
    fn plain_text_of_a_tree_reads_as_prose() {
        let blocks = parse_markdown("# 제목\n\n- 하나\n- 둘\n\n| A |\n|---|\n| 1 |");
        assert_eq!(blocks_to_text(&blocks), "제목\n• 하나\n• 둘\nA\n1");
    }
}
