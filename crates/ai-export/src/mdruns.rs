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
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading { level: usize, runs: Vec<Run> },
    Paragraph { runs: Vec<Run> },
    Quote { runs: Vec<Run> },
    List { ordered: bool, items: Vec<ListItem> },
    Code { lang: String, text: String },
    Table { rows: Vec<Vec<Vec<Run>>> },
    Image { alt: String, src: String },
    Hr,
}

static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(#{1,6})[ \t]+(.*)$").unwrap());
static FENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)^(?:```|~~~)([^\n]*)\n(.*?)\n?(?:```|~~~)?$").unwrap());
static HR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(?:---+|\*\*\*+|___+)$").unwrap());
static BLOCK_IMAGE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)$"#).unwrap());
static TABLE_RULE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\|[\s:|-]+\|?$").unwrap());
static QUOTE_PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*>[ \t]?").unwrap());
static LIST_START: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([-+*]|\d+[.)])[ \t]+").unwrap());
static LIST_ITEM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([ \t]*)(?:[-+*]|\d+[.)])[ \t]+(.*)$").unwrap());

pub fn parse_markdown(md: &str) -> Vec<Block> {
    split_markdown_blocks(md)
        .into_iter()
        .filter_map(|part| to_block(&part.md))
        .collect()
}

fn to_block(text: &str) -> Option<Block> {
    let trimmed = text.trim();

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
                let body = body.strip_suffix('|').unwrap_or(body);
                body.split('|')
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
        for line in trimmed.lines() {
            match LIST_ITEM.captures(line) {
                Some(c) => {
                    let indent = c.get(1).unwrap().as_str().replace('\t', "  ").len();
                    items.push(ListItem {
                        level: indent / 2,
                        runs: inline_runs(c.get(2).unwrap().as_str()),
                    });
                }
                None => {
                    // A wrapped continuation line belongs to the previous item.
                    if let Some(last) = items.last_mut() {
                        last.runs.push(Run::plain(format!(" {}", line.trim())));
                    }
                }
            }
        }
        if !items.is_empty() {
            return Some(Block::List { ordered, items });
        }
    }

    if trimmed.is_empty() {
        return None;
    }
    Some(Block::Paragraph {
        runs: inline_runs(&text.replace('\n', " ")),
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
}

static INLINE_LINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());
static INLINE_IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^!\[([^\]]*)\]\([^)]*\)").unwrap());

static RULES: Lazy<Vec<InlineRule>> = Lazy::new(|| {
    let rule = |pattern: &str, bold, italic, strike, code| InlineRule {
        re: Regex::new(pattern).unwrap(),
        bold,
        italic,
        strike,
        code,
    };
    vec![
        rule(r"^`([^`]+)`", false, false, false, true),
        rule(r"^\*\*\*([^*]+)\*\*\*", true, true, false, false),
        rule(r"^\*\*([^*]+)\*\*", true, false, false, false),
        rule(r"^__([^_]+)__", true, false, false, false),
        rule(r"^~~([^~]+)~~", false, false, true, false),
        rule(r"^\*([^*]+)\*", false, true, false, false),
        rule(r"^_([^_]+)_", false, true, false, false),
    ]
});

/// Split inline markdown into styled runs.
///
/// Unmatched syntax characters stay as literal text — an exporter should never
/// silently swallow a stray asterisk.
pub fn inline_runs(text: &str) -> Vec<Run> {
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

        let mut matched = false;
        for rule in RULES.iter() {
            let Some(c) = rule.re.captures(rest) else {
                continue;
            };
            flush!();
            runs.push(Run {
                text: c.get(1).unwrap().as_str().to_string(),
                bold: rule.bold,
                italic: rule.italic,
                strike: rule.strike,
                code: rule.code,
                link: None,
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
            let body = body.strip_suffix('|').unwrap_or(body);
            body.split('|')
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
        assert_eq!(runs_to_text(&items[0].runs), "첫 항목 계속되는 줄");
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
