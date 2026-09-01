//! Doc: a flow document whose markdown stays marker-free wherever it can.

use serde_json::{json, Value as Json};

use crate::blocks::{join_blocks, split_blocks, Joinable};
use crate::frontmatter::{meta_str, parse_frontmatter, serialize_frontmatter, Meta};
use crate::ids::{new_block_id, new_section_id};
use crate::mdblocks::{count_words, heading_level, plain_text, split_markdown_blocks, BlockType};
use crate::model::{DocBlock, Override, Page, Section, Spacing};

/// Read an override out of meta JSON, dropping anything that equals the default.
///
/// An override that matches the document default carries no information, and
/// keeping it would put a `<!-- block:id -->` anchor in the markdown for a
/// paragraph that has no formatting — exactly what this format avoids.
pub fn normalize_override(raw: Option<&Json>) -> Option<Override> {
    let obj = raw?.as_object()?;
    let mut out = Override::default();

    if let Some(align) = obj.get("align").and_then(|v| v.as_str()) {
        if align != "left" {
            out.align = Some(align.to_string());
        }
    }
    if let Some(indent) = obj.get("indent").and_then(number_of) {
        if indent > 0.0 {
            out.indent = Some(indent);
        }
    }
    if let Some(spacing) = obj.get("spacing").and_then(|v| v.as_object()) {
        let before = spacing
            .get("before")
            .and_then(number_of)
            .filter(|v| *v != 0.0);
        let after = spacing
            .get("after")
            .and_then(number_of)
            .filter(|v| *v != 0.0);
        if before.is_some() || after.is_some() {
            out.spacing = Some(Spacing { before, after });
        }
    }
    if let Some(style) = obj.get("style").and_then(|v| v.as_object()) {
        for key in ["italic", "bold", "underline"] {
            if style.get(key).and_then(|v| v.as_bool()) == Some(true) {
                out.style.insert(key.to_string(), json!(true));
            }
        }
        for key in ["color", "bg", "fontSize", "font", "lineHeight"] {
            match style.get(key) {
                None | Some(Json::Null) => {}
                Some(Json::String(s)) if s.is_empty() => {}
                Some(v) => {
                    out.style.insert(key.to_string(), v.clone());
                }
            }
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn number_of(v: &Json) -> Option<f64> {
    match v {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Merge a Doc section's markdown with its meta JSON.
///
/// Marker-free regions are re-segmented into paragraph blocks so the editor has
/// something to address; anchored regions keep their id and their override.
pub fn read_section(md: &str, meta_json: Option<&Json>) -> Section {
    let split = parse_frontmatter(md);
    let regions = split_blocks(&split.body);
    let overrides = meta_json
        .and_then(|m| m.get("blocks"))
        .and_then(|b| b.as_object())
        .cloned()
        .unwrap_or_default();

    let mut blocks: Vec<DocBlock> = Vec::new();
    for region in &regions {
        let parts = split_markdown_blocks(&region.md);
        if region.implicit {
            for part in parts {
                blocks.push(DocBlock {
                    id: new_block_id(),
                    md: part.md,
                    block_type: part.block_type,
                    format_override: None,
                });
            }
            continue;
        }

        let saved = normalize_override(overrides.get(&region.id));
        if parts.is_empty() {
            blocks.push(DocBlock {
                id: region.id.clone(),
                md: String::new(),
                block_type: BlockType::Paragraph,
                format_override: saved,
            });
            continue;
        }
        // An anchored region should stay one block; if markdown made it several,
        // the anchor applies to the first and the rest become plain blocks.
        for (i, part) in parts.into_iter().enumerate() {
            blocks.push(DocBlock {
                id: if i == 0 {
                    region.id.clone()
                } else {
                    new_block_id()
                },
                md: part.md,
                block_type: part.block_type,
                format_override: if i == 0 { saved.clone() } else { None },
            });
        }
    }

    Section {
        id: meta_str(&split.meta, "id")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                meta_json
                    .and_then(|m| m.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(new_section_id),
        name: meta_str(&split.meta, "name")
            .or_else(|| meta_str(&split.meta, "title"))
            .unwrap_or_else(|| "섹션".to_string()),
        page: read_page(meta_json.and_then(|m| m.get("page"))),
        blocks,
        file: None,
    }
}

fn read_page(value: Option<&Json>) -> Page {
    match value {
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_default(),
        None => Page::default(),
    }
}

pub struct SectionFiles {
    pub md: String,
    pub meta: Json,
}

/// Split a section back into the markdown + meta JSON pair.
pub fn write_section(section: &Section) -> SectionFiles {
    let s = normalize_section(section);

    let joinable: Vec<Joinable> = s
        .blocks
        .iter()
        .map(|b| Joinable {
            id: &b.id,
            md: &b.md,
            omit_marker: b.format_override.is_none(),
        })
        .collect();
    let body = join_blocks(&joinable);

    let mut meta = Meta::new();
    meta.insert("id".into(), json!(s.id));
    meta.insert("name".into(), json!(s.name));
    let md = serialize_frontmatter(&meta, &body);

    let mut blocks = serde_json::Map::new();
    for b in &s.blocks {
        if let Some(o) = &b.format_override {
            blocks.insert(b.id.clone(), serde_json::to_value(o).unwrap_or(Json::Null));
        }
    }

    SectionFiles {
        md,
        meta: json!({
            "id": s.id,
            "page": s.page,
            "blocks": Json::Object(blocks),
            "outline": build_outline(&s.blocks),
            "stats": section_stats(&s.blocks),
        }),
    }
}

pub fn normalize_section(section: &Section) -> Section {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut blocks: Vec<DocBlock> = section
        .blocks
        .iter()
        .map(|b| {
            let mut id = if b.id.is_empty() {
                new_block_id()
            } else {
                b.id.clone()
            };
            while seen.contains(&id) {
                id = new_block_id();
            }
            seen.insert(id.clone());
            DocBlock {
                id,
                md: b.md.clone(),
                block_type: b.block_type,
                format_override: b
                    .format_override
                    .as_ref()
                    .and_then(|o| normalize_override(serde_json::to_value(o).ok().as_ref())),
            }
        })
        .collect();

    if blocks.is_empty() {
        blocks.push(DocBlock {
            id: new_block_id(),
            md: String::new(),
            block_type: BlockType::Paragraph,
            format_override: None,
        });
    }

    Section {
        id: if section.id.is_empty() {
            new_section_id()
        } else {
            section.id.clone()
        },
        name: if section.name.is_empty() {
            "섹션".to_string()
        } else {
            section.name.clone()
        },
        page: section.page.clone(),
        blocks,
        file: section.file.clone(),
    }
}

#[derive(serde::Serialize)]
pub struct OutlineItem {
    pub level: usize,
    pub text: String,
    pub anchor: String,
}

pub fn build_outline(blocks: &[DocBlock]) -> Vec<OutlineItem> {
    blocks
        .iter()
        .filter_map(|b| {
            let level = heading_level(&b.md);
            if level == 0 {
                return None;
            }
            let text = plain_text(&b.md);
            if text.is_empty() {
                return None;
            }
            Some(OutlineItem {
                level,
                text,
                anchor: b.id.clone(),
            })
        })
        .collect()
}

#[derive(serde::Serialize)]
pub struct SectionStats {
    pub words: usize,
    pub chars: usize,
    pub blocks: usize,
}

pub fn section_stats(blocks: &[DocBlock]) -> SectionStats {
    let text = blocks
        .iter()
        .map(|b| plain_text(&b.md))
        .collect::<Vec<_>>()
        .join("\n");
    SectionStats {
        words: count_words(&text),
        chars: text.chars().filter(|c| !c.is_whitespace()).count(),
        blocks: blocks.len(),
    }
}

/// A one-line summary of an override, for the digest's `<!-- 서식: … -->` note.
pub fn describe_override(o: &Override) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(align) = &o.align {
        parts.push(format!("정렬 {align}"));
    }
    if let Some(indent) = o.indent {
        parts.push(format!(
            "들여쓰기 {}px",
            crate::geometry::js_round(indent) as i64
        ));
    }
    if o.style.get("italic") == Some(&json!(true)) {
        parts.push("기울임".into());
    }
    if o.style.get("bold") == Some(&json!(true)) {
        parts.push("굵게".into());
    }
    if let Some(Json::String(color)) = o.style.get("color") {
        parts.push(format!("색 {color}"));
    }
    if let Some(size) = o.style.get("fontSize").and_then(number_of) {
        parts.push(format!("크기 {}px", crate::geometry::js_round(size) as i64));
    }
    if parts.is_empty() {
        "기본".to_string()
    } else {
        parts.join(", ")
    }
}

pub fn make_section(name: &str, heading: bool) -> Section {
    let mut blocks = Vec::new();
    if heading {
        blocks.push(DocBlock {
            id: new_block_id(),
            md: format!("# {name}"),
            block_type: BlockType::Heading,
            format_override: None,
        });
    }
    blocks.push(DocBlock {
        id: new_block_id(),
        md: "내용을 입력하세요.".to_string(),
        block_type: BlockType::Paragraph,
        format_override: None,
    });
    Section {
        id: new_section_id(),
        name: name.to_string(),
        page: Page::default(),
        blocks,
        file: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_formatting_leaves_the_markdown_pure() {
        let section = make_section("요약", true);
        let files = write_section(&section);
        assert!(!files.md.contains("<!-- block:"), "{}", files.md);
        assert!(files.md.contains("# 요약"));
        assert_eq!(files.meta["blocks"], json!({}));
    }

    #[test]
    fn only_formatted_paragraphs_get_an_anchor() {
        let mut section = make_section("요약", true);
        section.blocks[1].format_override = Some(Override {
            align: Some("center".into()),
            ..Override::default()
        });
        let files = write_section(&section);
        let anchors = files.md.matches("<!-- block:").count();
        assert_eq!(anchors, 1, "{}", files.md);
        assert!(files
            .md
            .contains(&format!("<!-- block:{} -->", section.blocks[1].id)));
        assert_eq!(
            files.meta["blocks"][&section.blocks[1].id]["align"],
            json!("center")
        );
    }

    #[test]
    fn reverting_formatting_removes_the_anchor() {
        let mut section = make_section("요약", true);
        section.blocks[1].format_override = Some(Override {
            align: Some("left".into()),
            ..Override::default()
        });
        let files = write_section(&section);
        assert!(!files.md.contains("<!-- block:"), "{}", files.md);
    }

    #[test]
    fn a_section_round_trips() {
        let mut section = make_section("설계 원칙", true);
        section.blocks.push(DocBlock {
            id: "b_list".into(),
            md: "- 하나\n- 둘".into(),
            block_type: BlockType::List,
            format_override: Some(Override {
                indent: Some(24.0),
                ..Override::default()
            }),
        });
        let files = write_section(&section);
        let back = read_section(&files.md, Some(&files.meta));

        assert_eq!(back.id, section.id);
        assert_eq!(back.name, section.name);
        assert_eq!(back.blocks.len(), 3);
        assert_eq!(back.blocks[2].md, "- 하나\n- 둘");
        assert_eq!(back.blocks[2].block_type, BlockType::List);
        assert_eq!(
            back.blocks[2].format_override.as_ref().unwrap().indent,
            Some(24.0)
        );
    }

    #[test]
    fn hand_written_markdown_is_segmented_into_paragraphs() {
        let section = read_section(
            "---\nid: sec_1\nname: 손글씨\n---\n\n# 제목\n\n첫 문단\n\n둘째 문단\n",
            None,
        );
        assert_eq!(section.blocks.len(), 3);
        assert_eq!(section.blocks[0].block_type, BlockType::Heading);
        assert_eq!(section.blocks[1].md, "첫 문단");
        assert!(section.blocks.iter().all(|b| b.format_override.is_none()));
    }

    #[test]
    fn an_anchored_region_split_by_markdown_keeps_the_anchor_on_the_first_part() {
        let md = "<!-- block:b_a -->\n첫 문단\n\n둘째 문단\n";
        let meta = json!({ "blocks": { "b_a": { "align": "center" } } });
        let section = read_section(md, Some(&meta));
        assert_eq!(section.blocks.len(), 2);
        assert_eq!(section.blocks[0].id, "b_a");
        assert!(section.blocks[0].format_override.is_some());
        assert!(section.blocks[1].format_override.is_none());
    }

    #[test]
    fn outline_and_stats_land_in_the_meta() {
        let section = read_section("# 제목\n\n본문 두 단어\n\n## 하위\n", None);
        let files = write_section(&section);
        assert_eq!(files.meta["outline"][0]["level"], json!(1));
        assert_eq!(files.meta["outline"][0]["text"], json!("제목"));
        assert_eq!(files.meta["outline"][1]["level"], json!(2));
        assert_eq!(files.meta["stats"]["blocks"], json!(3));
        assert!(files.meta["stats"]["words"].as_u64().unwrap() > 0);
    }

    #[test]
    fn page_setup_survives() {
        let meta = json!({ "page": { "size": "Letter", "margin": { "top": 40, "right": 40, "bottom": 40, "left": 40 }, "columns": 2 } });
        let section = read_section("body", Some(&meta));
        assert_eq!(section.page.size, "Letter");
        assert_eq!(section.page.columns, 2);
        assert_eq!(section.page.margin.top, 40.0);
        assert_eq!(section.page.dimensions(), (816.0, 1056.0));
    }

    #[test]
    fn an_empty_section_gets_one_empty_paragraph() {
        let section = normalize_section(&Section {
            id: "s".into(),
            name: "n".into(),
            page: Page::default(),
            blocks: vec![],
            file: None,
        });
        assert_eq!(section.blocks.len(), 1);
    }

    #[test]
    fn overrides_describe_themselves() {
        let o = Override {
            align: Some("center".into()),
            indent: Some(24.0),
            spacing: None,
            style: [
                ("bold".to_string(), json!(true)),
                ("fontSize".to_string(), json!(18)),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            describe_override(&o),
            "정렬 center, 들여쓰기 24px, 굵게, 크기 18px"
        );
        assert_eq!(describe_override(&Override::default()), "기본");
    }
}
