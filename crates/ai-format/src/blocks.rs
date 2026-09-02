//! `<!-- block:id -->` markers: splitting a body on them and rebuilding it.

use std::collections::HashSet;

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::ids::new_block_id;

static BLOCK_MARKER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[ \t]*<!--[ \t]*block:[ \t]*([A-Za-z0-9_-]+)[ \t]*(?:\|([^>]*?))?-->[ \t]*$")
        .unwrap()
});
static FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*(`{3,}|~{3,})").unwrap());

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub id: String,
    pub md: String,
    /// True for content that appeared without a marker.
    pub implicit: bool,
    /// `<!-- block:b1 | kind=image -->` sugar.
    pub hint: IndexMap<String, String>,
}

/// Split a markdown body on `<!-- block:id -->` markers.
///
/// Fenced code blocks are skipped so a marker inside a ``` fence stays content.
/// Text appearing before the first marker becomes an implicit block, which is
/// what makes hand-written markdown (no markers at all) openable.
pub fn split_blocks(body: &str) -> Vec<Region> {
    struct Pending {
        id: String,
        lines: Vec<String>,
        implicit: bool,
        hint: IndexMap<String, String>,
    }

    let mut blocks: Vec<Region> = Vec::new();
    let mut current: Option<Pending> = None;
    let mut fence: Option<String> = None;
    let mut seen: HashSet<String> = HashSet::new();

    fn push(current: Option<Pending>, blocks: &mut Vec<Region>) {
        let Some(p) = current else { return };
        let joined = p.lines.join("\n");
        let md = trim_leading_blank_lines(&joined).trim_end().to_string();
        if !p.implicit || !md.is_empty() {
            blocks.push(Region {
                id: p.id,
                md,
                implicit: p.implicit,
                hint: p.hint,
            });
        }
    }

    for line in body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if let Some(f) = FENCE.captures(line) {
            let marker = f.get(1).unwrap().as_str();
            match &fence {
                None => fence = Some(marker.chars().next().unwrap().to_string().repeat(3)),
                Some(open) => {
                    if line.trim_start().starts_with(open.as_str()) {
                        fence = None;
                    }
                }
            }
        }

        let marker = if fence.is_none() {
            BLOCK_MARKER.captures(line)
        } else {
            None
        };
        if let Some(m) = marker {
            push(current.take(), &mut blocks);
            let mut id = m.get(1).unwrap().as_str().to_string();
            // Duplicate ids in a hand-edited file would collapse two blocks into one.
            while seen.contains(&id) {
                id = format!("{id}-{}", &new_block_id()[2..5]);
            }
            seen.insert(id.clone());
            current = Some(Pending {
                id,
                lines: Vec::new(),
                implicit: false,
                hint: parse_hint(m.get(2).map(|g| g.as_str())),
            });
            continue;
        }

        if current.is_none() {
            let id = new_block_id();
            seen.insert(id.clone());
            current = Some(Pending {
                id,
                lines: Vec::new(),
                implicit: true,
                hint: IndexMap::new(),
            });
        }
        current
            .as_mut()
            .expect("just created")
            .lines
            .push(line.to_string());
    }
    push(current.take(), &mut blocks);
    blocks
}

fn trim_leading_blank_lines(s: &str) -> &str {
    let mut rest = s;
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        match trimmed.strip_prefix('\n') {
            Some(next) => rest = next,
            None => return rest,
        }
    }
}

/// `<!-- block:b1 | kind=image -->` -> `{ kind: "image" }`. Optional sugar.
fn parse_hint(raw: Option<&str>) -> IndexMap<String, String> {
    let mut hint = IndexMap::new();
    let Some(raw) = raw else { return hint };
    for pair in raw.split([',', ';']) {
        if let Some((k, v)) = pair.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if !k.is_empty() && !v.is_empty() {
                hint.insert(k.to_string(), v.to_string());
            }
        }
    }
    hint
}

/// A block as the join step sees it.
pub struct Joinable<'a> {
    pub id: &'a str,
    pub md: &'a str,
    /// True to write the content without its `<!-- block:id -->` anchor.
    pub omit_marker: bool,
}

/// Rebuild a markdown body from blocks, emitting a marker for each.
pub fn join_blocks(blocks: &[Joinable]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for b in blocks {
        let md = b.md.trim_end();
        // Marker and its content stay on adjacent lines so the file reads as prose.
        if b.omit_marker {
            if !md.is_empty() {
                parts.push(md.to_string());
            }
        } else if md.is_empty() {
            parts.push(format!("<!-- block:{} -->", b.id));
        } else {
            parts.push(format!("<!-- block:{} -->\n{md}", b.id));
        }
    }
    format!("{}\n", parts.join("\n\n").trim_end())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image,
    Shape,
    Table,
    Chart,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Shape => "shape",
            Kind::Table => "table",
            Kind::Chart => "chart",
        }
    }

    pub fn from_name(s: &str) -> Option<Kind> {
        Some(match s {
            "text" => Kind::Text,
            "image" => Kind::Image,
            "shape" => Kind::Shape,
            "table" => Kind::Table,
            "chart" => Kind::Chart,
            _ => return None,
        })
    }

    /// The Korean label the digest prints.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Text => "텍스트",
            Kind::Image => "이미지",
            Kind::Shape => "도형",
            Kind::Table => "표",
            Kind::Chart => "차트",
        }
    }
}

static CHART_FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^```chart\b").unwrap());
static LONE_IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^!\[[^\]]*\]\([^)]+\)\s*$").unwrap());
static TABLE_ROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^\|.*\|\s*$").unwrap());
static TABLE_RULE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\|[\s:|-]*-[\s:|-]*\|\s*$").unwrap());

/// Guess a block's kind from its markdown, used when layout JSON has no entry
/// (a hand-added block) or when the user pastes content.
pub fn infer_kind(md: &str) -> Kind {
    let t = md.trim();
    if t.is_empty() {
        return Kind::Text;
    }
    if CHART_FENCE.is_match(t) {
        return Kind::Chart;
    }
    if LONE_IMAGE.is_match(t) {
        return Kind::Image;
    }
    if TABLE_ROW.is_match(t) && TABLE_RULE.is_match(t) {
        return Kind::Table;
    }
    Kind::Text
}

static HEADING_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^#{1,6}[ \t]+(.+)$").unwrap());
static MD_IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"!\[([^\]]*)\]\([^)]*\)").unwrap());
static MD_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());
static LIST_BULLET: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[-+*]\s+").unwrap());

/// Extract the first heading or line of a block — used for outlines and digests.
pub fn block_label(md: &str, max: usize) -> String {
    let t = md.trim();
    if t.is_empty() {
        return String::new();
    }
    let raw = match HEADING_LINE.captures(t) {
        Some(c) => c.get(1).unwrap().as_str().to_string(),
        None => t
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string(),
    };
    let plain = MD_IMAGE.replace_all(&raw, "$1");
    let plain = MD_LINK.replace_all(&plain, "$1");
    let plain: String = plain.chars().filter(|c| !"*_`>#".contains(*c)).collect();
    let plain = LIST_BULLET.replace(&plain, "");
    let plain = plain.trim();

    if plain.chars().count() > max {
        let kept: String = plain.chars().take(max - 1).collect();
        format!("{kept}…")
    } else {
        plain.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_markers() {
        let body = "<!-- block:b1 -->\n# 제목\n\n<!-- block:b2 -->\n- 항목\n";
        let blocks = split_blocks(body);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id, "b1");
        assert_eq!(blocks[0].md, "# 제목");
        assert_eq!(blocks[1].md, "- 항목");
        assert!(!blocks[0].implicit);
    }

    #[test]
    fn marker_free_markdown_becomes_one_implicit_block() {
        let blocks = split_blocks("# 손으로 쓴 제목\n\n본문\n");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].implicit);
        assert_eq!(blocks[0].md, "# 손으로 쓴 제목\n\n본문");
    }

    #[test]
    fn a_marker_inside_a_fence_stays_content() {
        let body = "<!-- block:b1 -->\n```\n<!-- block:fake -->\n```\n";
        let blocks = split_blocks(body);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].md.contains("block:fake"));
    }

    #[test]
    fn duplicate_ids_do_not_collapse_blocks() {
        let blocks = split_blocks("<!-- block:b1 -->\nA\n\n<!-- block:b1 -->\nB\n");
        assert_eq!(blocks.len(), 2);
        assert_ne!(blocks[0].id, blocks[1].id);
        assert_eq!(blocks[0].md, "A");
        assert_eq!(blocks[1].md, "B");
    }

    #[test]
    fn hints_are_parsed() {
        let blocks = split_blocks("<!-- block:b1 | kind=image, alt=x -->\n![](a.png)\n");
        assert_eq!(blocks[0].hint["kind"], "image");
        assert_eq!(blocks[0].hint["alt"], "x");
    }

    #[test]
    fn join_round_trips_a_split() {
        let body = "<!-- block:b1 -->\n# A\n\n<!-- block:b2 -->\n- B\n";
        let blocks = split_blocks(body);
        let joinable: Vec<Joinable> = blocks
            .iter()
            .map(|b| Joinable {
                id: &b.id,
                md: &b.md,
                omit_marker: false,
            })
            .collect();
        assert_eq!(join_blocks(&joinable), body);
    }

    #[test]
    fn omitting_markers_yields_plain_markdown() {
        let joined = join_blocks(&[
            Joinable {
                id: "b1",
                md: "# A",
                omit_marker: true,
            },
            Joinable {
                id: "b2",
                md: "본문",
                omit_marker: true,
            },
        ]);
        assert_eq!(joined, "# A\n\n본문\n");
    }

    #[test]
    fn kinds_are_inferred_from_content() {
        assert_eq!(infer_kind("# 제목"), Kind::Text);
        assert_eq!(infer_kind("```chart\n{}\n```"), Kind::Chart);
        assert_eq!(infer_kind("![alt](../assets/a.png)"), Kind::Image);
        assert_eq!(infer_kind("| A | B |\n|---|---|\n| 1 | 2 |"), Kind::Table);
        // An image with a caption below it is not an image block.
        assert_eq!(infer_kind("![a](b.png)\n\n설명"), Kind::Text);
        assert_eq!(infer_kind(""), Kind::Text);
    }

    #[test]
    fn labels_prefer_the_first_heading() {
        assert_eq!(block_label("본문\n\n# 제목", 80), "제목");
        assert_eq!(
            block_label("- **매출 142억** — +24%", 80),
            "매출 142억 — +24%"
        );
        assert_eq!(block_label("[링크](http://x)", 80), "링크");
        assert_eq!(block_label("", 80), "");
        assert_eq!(block_label("abcdef", 4), "abc…");
    }
}
