//! RLM — a recursive, addressable view of a document.
//!
//! `AI.md` flattens a project into one file. That shape is right for a RAG
//! chunker and wrong for a recursive language model: the model sees a wall of
//! text with no way to decide what it does *not* need to read, and a document
//! larger than the context window cannot be read at all.
//!
//! This module is the other half. Every part of a document gets a stable
//! **address** and a bounded **summary**, arranged as a tree:
//!
//! * [`outline`] is the map — summaries only, small enough to always fit.
//! * [`resolve`] takes one address and returns exactly that node's content plus
//!   its children's summaries, so a caller descends only where it must.
//! * [`verify`] walks the same tree and reports what contradicts the document's
//!   own structure — the verification half of an RLM loop.
//!
//! The addresses are stable across saves (they are built from the model's own
//! ids), so an answer produced from a resolved node can be checked against the
//! same node later. That is what lets an agent say *where* it read something.
//!
//! # Addresses
//!
//! | Document | Address |
//! |---|---|
//! | the whole project | `project` |
//! | a slide / section / sheet | `slide/s_ab12`, `section/sec_ab12`, `sheet/sh_ab12` |
//! | a block in a slide or section | `slide/s_ab12/block/b_7k2mx` |
//! | a data region of a sheet | `sheet/sh_ab12/region/A1:F51` |
//! | one cell | `sheet/sh_ab12/cell/B4` |
//! | a chart / a named range | `sheet/sh_ab12/chart/ch_1`, `sheet/sh_ab12/name/매출` |
//!
//! A numeric key is accepted as a 1-based position (`slide/3`), which is how a
//! person counts a deck; an id is accepted because that is what survives a
//! reorder. Both name the same node.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

use ai_formula::evaluate::display_value;
use ai_formula::refs::{index_to_col, parse_range, to_ref};

use crate::blocks::Kind;
use crate::chart::{describe_chart, parse_chart_block, resolve_spec, CellSource};
use crate::geometry::{position_phrase, reading_order};
use crate::grid::{render_sheet_markdown, used_range};
use crate::mdblocks::{heading_level, plain_text, BlockType};
use crate::model::{DocBlock, Items, Project, Section, Sheet, Slide, SlideBlock};
use crate::shape::preset_label;
use crate::table::parse_markdown_table;

/// The address of the project root.
pub const ROOT_PATH: &str = "project";

/// Longest summary kept for one node. Long enough to answer "what is this?",
/// short enough that a few hundred of them still fit a context window.
const SUMMARY_CHARS: usize = 180;

/// Cap on nodes read by [`verify`], so a hostile or huge file cannot make a
/// save spend unbounded time reporting problems.
const VERIFY_LIMIT: usize = 20_000;

/// One node of the recursive document tree.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Stable address, e.g. `slide/s_ab12/block/b_7k2mx`.
    pub path: String,
    /// `project` · `slide` · `block` · `section` · `sheet` · `region` · `cell` ·
    /// `chart` · `name`.
    pub kind: String,
    /// A short human title: a slide title, a shape's label, a sheet's name.
    pub title: String,
    /// A bounded one-paragraph summary. Present on every node, always safe to
    /// put in context.
    pub summary: String,
    /// Characters of the full content, so a caller can budget before asking.
    pub chars: usize,
    /// How many children this node has, even when they were not expanded.
    pub child_count: usize,
    /// Child nodes, present only as deep as the caller asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    /// The full text at this node. `None` in an outline, filled by `resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The file that holds the full content, when the node came from disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Structured facts a model can rely on without parsing prose: position,
    /// cell range, series count, group membership.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub fields: IndexMap<String, Json>,
}

/// A problem found by [`verify`], addressed so it can be fixed in place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub path: String,
    /// `error` contradicts the format; `warning` is a likely mistake.
    pub level: String,
    pub message: String,
}

/* ------------------------------------------------------------------ helpers */

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> String {
    let text = collapse(text);
    if text.chars().count() <= max {
        return text;
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

fn locate<'a, T>(items: &'a [T], key: &str, id_of: impl Fn(&T) -> &str) -> Option<(usize, &'a T)> {
    if let Ok(n) = key.parse::<usize>() {
        if n == 0 {
            return None;
        }
        return items.get(n - 1).map(|t| (n - 1, t));
    }
    items.iter().enumerate().find(|(_, t)| id_of(t) == key)
}

fn kind_counts(blocks: &[SlideBlock]) -> String {
    const KINDS: [Kind; 5] = [
        Kind::Text,
        Kind::Shape,
        Kind::Table,
        Kind::Chart,
        Kind::Image,
    ];
    let mut parts: Vec<String> = Vec::new();
    for kind in KINDS {
        let n = blocks.iter().filter(|b| b.kind == kind).count();
        if n > 0 {
            parts.push(format!("{} {n}", kind.label()));
        }
    }
    parts.join(", ")
}

/// The Korean label a doc block type prints.
fn doc_type_label(t: BlockType) -> &'static str {
    match t {
        BlockType::Heading => "제목",
        BlockType::Paragraph => "문단",
        BlockType::List => "목록",
        BlockType::Table => "표",
        BlockType::Quote => "인용",
        BlockType::Code => "코드",
        BlockType::Image => "이미지",
        BlockType::Hr => "구분선",
    }
}

/// A table's shape and header, read out of its markdown.
fn table_shape(md: &str) -> (usize, usize, String) {
    let rows = parse_markdown_table(md);
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let header = rows.first().map(|r| r.join(" | ")).unwrap_or_default();
    (rows.len(), cols, header)
}

/// A chart's one-line description, when the block holds a chart.
fn chart_description(md: &str) -> Option<String> {
    parse_chart_block(md).map(|spec| describe_chart(&spec))
}

// /* --------------------------------------------------------------- outline */
/// The full tree, summaries only. This is the map an agent reads first.
///
/// Cheap to hold and small enough to put in context: content stays on disk
/// until [`resolve`] is asked for a specific address.
pub fn outline(project: &Project) -> Node {
    project_node(project, false, usize::MAX)
}

/// Resolve one address to its content and immediate children's summaries.
///
/// `depth` is how many levels below the target to expand. `0` returns just the
/// node with its content; `1` adds its children's summaries; and so on. Nodes
/// at the boundary report `child_count` so the caller knows there is more.
pub fn resolve(project: &Project, path: &str, depth: usize) -> Option<Node> {
    let path = path.trim_matches('/');
    if path.is_empty() || path == ROOT_PATH {
        return Some(project_node(project, true, depth));
    }
    let seg: Vec<&str> = path.split('/').collect();
    match (project, seg.as_slice()) {
        (project, ["slide", key]) => {
            let slides = project.slides();
            let (i, slide) = locate(slides, key, |s| &s.id)?;
            Some(slide_node(slide, i, true, depth))
        }
        (project, ["slide", key, "block", block_key]) => {
            let slides = project.slides();
            let (_, slide) = locate(slides, key, |s| &s.id)?;
            let (_, block) = locate(&slide.blocks, block_key, |b| &b.id)?;
            Some(block_node(slide, block, true))
        }
        (project, ["slide", key, "group", group]) => {
            let slides = project.slides();
            let (i, slide) = locate(slides, key, |s| &s.id)?;
            let full = slide_node(slide, i, true, usize::MAX);
            let target = format!("slide/{}/group/{group}", slide.id);
            find_by_path(&full, &target)
        }
        (project, ["section", key]) => {
            let sections = project.sections();
            let (i, section) = locate(sections, key, |s| &s.id)?;
            Some(section_node(section, i, true, depth))
        }
        (project, ["section", key, "block", block_key]) => {
            let sections = project.sections();
            let (_, section) = locate(sections, key, |s| &s.id)?;
            let keys = doc_block_keys(&section.blocks);
            let index = keys.iter().position(|k| k == block_key)?;
            Some(doc_block_node(
                section,
                &section.blocks[index],
                block_key,
                true,
            ))
        }
        (project, ["sheet", key]) => {
            let sheets = project.sheets();
            let (i, sheet) = locate(sheets, key, |s| &s.id)?;
            Some(sheet_node(sheet, i, true, depth))
        }
        (project, ["sheet", key, "region", range]) => {
            let sheets = project.sheets();
            let (_, sheet) = locate(sheets, key, |s| &s.id)?;
            let r = parse_range(range)?;
            Some(region_node(
                sheet,
                r.start.col,
                r.start.row,
                r.end.col,
                r.end.row,
                true,
                depth,
            ))
        }
        (project, ["sheet", key, "cell", reference]) => {
            let sheets = project.sheets();
            let (_, sheet) = locate(sheets, key, |s| &s.id)?;
            cell_node(sheet, reference)
        }
        (project, ["sheet", key, "chart", chart_key]) => {
            let sheets = project.sheets();
            let (_, sheet) = locate(sheets, key, |s| &s.id)?;
            let (_, chart) = locate(&sheet.charts, chart_key, |c| &c.id)?;
            Some(chart_node(sheet, chart, true))
        }
        (project, ["sheet", key, "name", name]) => {
            let sheets = project.sheets();
            let (_, sheet) = locate(sheets, key, |s| &s.id)?;
            let target = sheet.names.get(*name)?;
            let value = match target {
                Json::String(s) => s.clone(),
                other => other.to_string(),
            };
            Some(Node {
                path: format!("sheet/{}/name/{name}", sheet.id),
                kind: "name".into(),
                title: name.to_string(),
                summary: format!("이름 범위 `{value}`"),
                chars: value.chars().count(),
                child_count: 0,
                children: Vec::new(),
                content: Some(value),
                source: sheet.file.clone(),
                fields: IndexMap::new(),
            })
        }
        _ => None,
    }
}

/* -------------------------------------------------------------- project */

fn project_node(project: &Project, with_content: bool, depth: usize) -> Node {
    let (kind, title, summary, children): (&str, String, String, Vec<Node>) = match &project.items {
        Items::Slides(slides) => {
            let elements: usize = slides.iter().map(|s| s.blocks.len()).sum();
            let noted = slides.iter().filter(|s| !s.notes.trim().is_empty()).count();
            let mut summary = format!(
                "프레젠테이션 · 슬라이드 {}장 · 요소 {}개",
                slides.len(),
                elements
            );
            if noted > 0 {
                summary.push_str(&format!(" · 발표자 노트 {}장", noted));
            }
            let children = if depth > 0 {
                slides
                    .iter()
                    .enumerate()
                    .map(|(i, s)| slide_node(s, i, with_content, depth - 1))
                    .collect()
            } else {
                Vec::new()
            };
            ("project", project.manifest.title.clone(), summary, children)
        }
        Items::Sections(sections) => {
            let words: usize = sections
                .iter()
                .flat_map(|s| s.blocks.iter())
                .map(|b| crate::mdblocks::count_words(&b.md))
                .sum();
            let headings = sections
                .iter()
                .flat_map(|s| s.blocks.iter())
                .filter(|b| heading_level(&b.md) > 0)
                .count();
            let children = if depth > 0 {
                sections
                    .iter()
                    .enumerate()
                    .map(|(i, s)| section_node(s, i, with_content, depth - 1))
                    .collect()
            } else {
                Vec::new()
            };
            (
                "project",
                project.manifest.title.clone(),
                format!(
                    "문서 · 섹션 {}개 · 단어 약 {}개 · 제목 {}개",
                    sections.len(),
                    words,
                    headings
                ),
                children,
            )
        }
        Items::Sheets(sheets) => {
            let cells: usize = sheets.iter().map(|s| s.cells.len()).sum();
            let formulas: usize = sheets
                .iter()
                .flat_map(|s| s.cells.values())
                .filter(|c| c.f.is_some())
                .count();
            let children = if depth > 0 {
                sheets
                    .iter()
                    .enumerate()
                    .map(|(i, s)| sheet_node(s, i, with_content, depth - 1))
                    .collect()
            } else {
                Vec::new()
            };
            (
                "project",
                project.manifest.title.clone(),
                format!(
                    "스프레드시트 · 시트 {}개 · 값 {}개 · 수식 {}개",
                    sheets.len(),
                    cells,
                    formulas
                ),
                children,
            )
        }
    };
    Node {
        path: ROOT_PATH.into(),
        kind: kind.into(),
        title,
        summary,
        chars: 0,
        child_count: children.len(),
        children,
        content: None,
        source: None,
        fields: IndexMap::new(),
    }
}

/* ------------------------------------------------------------------ deck */

fn slide_node(slide: &Slide, index: usize, with_content: bool, depth: usize) -> Node {
    let counts = kind_counts(&slide.blocks);
    let mut summary = if counts.is_empty() {
        "빈 슬라이드".to_string()
    } else {
        format!("{} — {counts}", or_title(&slide.title))
    };
    if !slide.notes.trim().is_empty() {
        summary.push_str(&format!(" · 노트: {}", clip(&slide.notes, 80)));
    }
    let mut fields = IndexMap::new();
    fields.insert(
        "canvas".into(),
        json!(format!(
            "{}×{}",
            slide.canvas.w as i64, slide.canvas.h as i64
        )),
    );
    fields.insert("hidden".into(), json!(slide.hidden));
    if let Some(layout) = &slide.layout_part {
        fields.insert("layout".into(), json!(layout));
    }

    // Build the whole block tree (with content only when asked), then drop the
    // children at depth 0. Group blocks are nested, so a slide reads as the
    // structure the author drew rather than a flat list.
    let grouped = slide_children(slide, with_content);
    let child_total = grouped.len();
    let children = if depth > 0 { grouped } else { Vec::new() };

    Node {
        path: format!("slide/{}", slide.id),
        kind: "slide".into(),
        title: format!("{} — {}", index + 1, or_title(&slide.title)),
        summary,
        chars: slide_content(slide).chars().count(),
        child_count: child_total,
        children,
        content: with_content.then(|| slide_content(slide)),
        source: slide.file.clone(),
        fields,
    }
}

/// A slide's blocks, nested under the groups the author drew.
///
/// Reading order decides where a node appears, and a block's `style.group` (an
/// ordered path set by the importer) decides how deep it sits. Group nodes get
/// their own address (`slide/…/group/1`) so an agent can read a card or a flow
/// chart node as a unit instead of as loose shapes.
fn slide_children(slide: &Slide, with_content: bool) -> Vec<Node> {
    let ordered = reading_order(&slide.blocks, |b| b.geometry(), 40.0);
    let mut roots: Vec<Node> = Vec::new();
    let mut ordinals: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut next = 1usize;
    for block in &ordered {
        let groups = block_groups(block);
        let node = block_node(slide, block, with_content);
        insert_group(
            &mut roots,
            &groups,
            node,
            &slide.id,
            &mut ordinals,
            &mut next,
        );
    }
    roots
}

fn block_groups(block: &SlideBlock) -> Vec<String> {
    block
        .style
        .get("group")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn insert_group(
    siblings: &mut Vec<Node>,
    remaining: &[String],
    block: Node,
    slide_id: &str,
    ordinals: &mut std::collections::HashMap<String, usize>,
    next: &mut usize,
) {
    if remaining.is_empty() {
        siblings.push(block);
        return;
    }
    let key = remaining.join("\u{1f}");
    let existing = siblings.iter().position(|n| {
        n.kind == "group" && n.fields.get("groupKey").and_then(Json::as_str) == Some(key.as_str())
    });
    let idx = match existing {
        Some(i) => i,
        None => {
            let ordinal = *ordinals.entry(key).or_insert_with(|| {
                let n = *next;
                *next += 1;
                n
            });
            let mut fields = IndexMap::new();
            fields.insert("groupKey".into(), json!(remaining.join("\u{1f}")));
            siblings.push(Node {
                path: format!("slide/{slide_id}/group/{ordinal}"),
                kind: "group".into(),
                title: remaining[0].clone(),
                summary: String::new(),
                chars: 0,
                child_count: 0,
                children: Vec::new(),
                content: None,
                source: None,
                fields,
            });
            siblings.len() - 1
        }
    };
    insert_group(
        &mut siblings[idx].children,
        &remaining[1..],
        block,
        slide_id,
        ordinals,
        next,
    );
    siblings[idx].child_count = siblings[idx].children.len();
    siblings[idx].summary = format!(
        "그룹 · 요소 {}개{}",
        siblings[idx].child_count,
        if with_more(&siblings[idx]) {
            " · 하위 그룹 포함"
        } else {
            ""
        }
    );
}

/// True when a group holds another group, so its summary can say the count is
/// not all blocks.
fn with_more(group: &Node) -> bool {
    group.children.iter().any(|c| c.kind == "group")
}

fn or_title(title: &str) -> &str {
    if title.trim().is_empty() {
        "(제목 없음)"
    } else {
        title
    }
}

fn slide_content(slide: &Slide) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !slide.title.trim().is_empty() {
        parts.push(format!("# {}", slide.title));
    }
    for block in &slide.blocks {
        if !block.md.trim().is_empty() {
            parts.push(block.md.trim().to_string());
        }
    }
    if !slide.notes.trim().is_empty() {
        parts.push(format!("> 발표자 노트: {}", slide.notes.trim()));
    }
    parts.join("\n\n")
}

fn block_node(slide: &Slide, block: &SlideBlock, with_content: bool) -> Node {
    let path = format!("slide/{}/block/{}", slide.id, block.id);
    let where_ = position_phrase(&block.geometry(), &slide.canvas);
    let label = match (&block.kind, &block.shape) {
        (Kind::Shape, Some(shape)) => format!("도형({})", preset_label(&shape.preset)),
        _ => block.kind.label().to_string(),
    };
    let text = plain_text(&block.md);
    let title = {
        let t = crate::blocks::block_label(&block.md, 60);
        if t.is_empty() {
            label.clone()
        } else {
            t
        }
    };
    let (summary, child_count) = match block.kind {
        Kind::Table => {
            let (rows, cols, header) = table_shape(&block.md);
            (
                format!("{where_} · 표 {rows}행 × {cols}열 — {}", clip(&header, 90)),
                rows,
            )
        }
        Kind::Chart => (
            format!(
                "{where_} · {}",
                chart_description(&block.md).unwrap_or_else(|| "차트".into())
            ),
            0,
        ),
        _ => (format!("{where_} · {}", clip(&text, SUMMARY_CHARS)), 0),
    };

    let mut fields = IndexMap::new();
    fields.insert("position".into(), json!(where_));
    fields.insert("z".into(), json!(block.z));
    if block.kind == Kind::Shape {
        if let Some(shape) = &block.shape {
            fields.insert("preset".into(), json!(shape.preset));
        }
    }
    if let Some(group) = block.style.get("group") {
        fields.insert("group".into(), group.clone());
    }

    Node {
        path,
        kind: format!("block/{}", block.kind.as_str()),
        title,
        summary,
        chars: block.md.chars().count(),
        child_count,
        children: Vec::new(),
        content: with_content.then(|| block.md.trim().to_string()),
        source: slide.file.clone(),
        fields,
    }
}

/* ------------------------------------------------------------------- doc */

fn section_node(section: &Section, index: usize, with_content: bool, depth: usize) -> Node {
    let words: usize = section
        .blocks
        .iter()
        .map(|b| crate::mdblocks::count_words(&b.md))
        .sum();
    let headings = section
        .blocks
        .iter()
        .filter(|b| heading_level(&b.md) > 0)
        .count();
    let child_total = section.blocks.len();

    // Headings make the document recursive: a heading is a branch, the plain
    // blocks under it are its children. Reading `section/…` with depth 1 gives
    // the chapter list; resolving a heading gives that chapter.
    let children = if depth > 0 {
        heading_tree(section, with_content)
    } else {
        Vec::new()
    };

    Node {
        path: format!("section/{}", section.id),
        kind: "section".into(),
        title: format!("{} — {}", index + 1, or_title(&section.name)),
        summary: format!(
            "단어 약 {words}개 · 블록 {}개 · 제목 {headings}개",
            section.blocks.len()
        ),
        chars: section_content(section).chars().count(),
        child_count: child_total,
        children,
        content: with_content.then(|| section_content(section)),
        source: section.file.clone(),
        fields: IndexMap::new(),
    }
}

fn section_content(section: &Section) -> String {
    section
        .blocks
        .iter()
        .map(|b| b.md.trim())
        .filter(|m| !m.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Depth-first search by address, for resolving a nested group.
fn find_by_path(node: &Node, path: &str) -> Option<Node> {
    if node.path == path {
        return Some(node.clone());
    }
    for child in &node.children {
        if let Some(found) = find_by_path(child, path) {
            return Some(found);
        }
    }
    None
}

/// A stable address key for every block in a section.
///
/// A Doc paragraph with no `<!-- block:id -->` anchor gets a fresh id on every
/// read, so a node id cannot be the address — it would change on each save and
/// every answer would point at a block that no longer exists. A heading's
/// address is the same slug the outline uses; anything else is `p` plus its
/// 1-based position. Both are stable across an unchanged save.
fn doc_block_keys(blocks: &[DocBlock]) -> Vec<String> {
    let mut taken: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut keys = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        let level = heading_level(&block.md);
        let text = plain_text(&block.md);
        let key = if level > 0 && !text.is_empty() {
            let slug = crate::doc::anchor_slug(&text);
            let n = taken.entry(slug.clone()).or_insert(0);
            *n += 1;
            if *n == 1 {
                slug
            } else {
                format!("{slug}-{n}")
            }
        } else {
            format!("p{}", i + 1)
        };
        keys.push(key);
    }
    keys
}

/// Nest a section's blocks under their headings.
///
/// Every block keeps its own global address (`section/…/block/…`) so a caller
/// can resolve it directly; the tree only says where it sits in the outline.
fn heading_tree(section: &Section, with_content: bool) -> Vec<Node> {
    let mut roots: Vec<Node> = Vec::new();
    // Stack of (level, index-into-roots-or-children) is awkward with owned
    // nodes, so build a flat list of (level, node) and fold it into a tree.
    let mut flat: Vec<(usize, Node)> = Vec::new();
    let keys = doc_block_keys(&section.blocks);
    for (i, block) in section.blocks.iter().enumerate() {
        let level = heading_level(&block.md);
        let node = doc_block_node(section, block, &keys[i], with_content);
        if level > 0 {
            flat.push((level, node));
        } else if let Some((_, parent)) = flat.last_mut() {
            // A plain block belongs to the nearest heading above it.
            parent.children.push(node);
            parent.child_count = parent.children.len();
        } else {
            // Text before the first heading stays at the section's top level.
            roots.push(node);
        }
    }
    // Fold headings into their parents, level by level.
    for (level, node) in flat {
        insert_by_level(&mut roots, level, node);
    }
    roots
}

/// Insert a heading node at the right place in an outline, nesting it under the
/// nearest shallower heading.
fn insert_by_level(siblings: &mut Vec<Node>, level: usize, node: Node) {
    // The last sibling with a strictly smaller level is the parent.
    if let Some(parent) = siblings.iter_mut().rev().find(|s| level_of(s) < level) {
        insert_by_level(&mut parent.children, level, node);
        parent.child_count = parent.children.len();
        return;
    }
    siblings.push(node);
}

fn level_of(node: &Node) -> usize {
    node.fields
        .get("level")
        .and_then(Json::as_u64)
        .map(|n| n as usize)
        .unwrap_or(usize::MAX)
}

fn doc_block_node(section: &Section, block: &DocBlock, key: &str, with_content: bool) -> Node {
    let path = format!("section/{}/block/{key}", section.id);
    let level = heading_level(&block.md);
    let text = plain_text(&block.md);
    let title = {
        let t = crate::blocks::block_label(&block.md, 80);
        if t.is_empty() {
            doc_type_label(block.block_type).to_string()
        } else {
            t
        }
    };
    let (summary, child_count) = if block.block_type == BlockType::Table {
        let (rows, cols, header) = table_shape(&block.md);
        (
            format!("표 {rows}행 × {cols}열 — {}", clip(&header, 90)),
            rows,
        )
    } else if let Some(desc) = chart_description(&block.md) {
        (desc, 0)
    } else if level > 0 {
        (
            format!("제목 {}단계 · {}", level, clip(&text, SUMMARY_CHARS)),
            0,
        )
    } else {
        (clip(&text, SUMMARY_CHARS), 0)
    };

    let mut fields = IndexMap::new();
    if level > 0 {
        fields.insert("level".into(), json!(level));
    }
    Node {
        path,
        kind: format!("block/{}", block.block_type.as_str()),
        title,
        summary,
        chars: block.md.chars().count(),
        child_count,
        children: Vec::new(),
        content: with_content.then(|| block.md.trim().to_string()),
        source: section.file.clone(),
        fields,
    }
}

/* ------------------------------------------------------------------ grid */

fn sheet_node(sheet: &Sheet, index: usize, with_content: bool, depth: usize) -> Node {
    let range_text = used_range(&sheet.cells)
        .map(|r| format!("`A1:{}`", to_ref(r.max_col, r.max_row)))
        .unwrap_or_else(|| "빈 시트".to_string());
    let formulas = sheet.cells.values().filter(|c| c.f.is_some()).count();
    let regions = sheet_regions(sheet);
    let summary = format!(
        "{range_text} · 값 {}개 · 수식 {formulas}개 · 데이터 영역 {}개",
        sheet.cells.len(),
        regions.len()
    );

    let children = if depth > 0 {
        let mut out: Vec<Node> = Vec::new();
        for (c0, r0, c1, r1) in &regions {
            out.push(region_node(
                sheet,
                *c0,
                *r0,
                *c1,
                *r1,
                with_content,
                depth - 1,
            ));
        }
        for chart in &sheet.charts {
            out.push(chart_node(sheet, chart, with_content));
        }
        for (name, target) in &sheet.names {
            let value = match target {
                Json::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.push(Node {
                path: format!("sheet/{}/name/{name}", sheet.id),
                kind: "name".into(),
                title: name.clone(),
                summary: format!("이름 범위 `{value}`"),
                chars: value.chars().count(),
                child_count: 0,
                children: Vec::new(),
                content: with_content.then_some(value),
                source: sheet.file.clone(),
                fields: IndexMap::new(),
            });
        }
        out
    } else {
        Vec::new()
    };
    let child_count = regions.len() + sheet.charts.len() + sheet.names.len();
    let content = render_sheet_markdown(sheet);

    Node {
        path: format!("sheet/{}", sheet.id),
        kind: "sheet".into(),
        title: format!("{} — {}", index + 1, or_title(&sheet.name)),
        summary,
        chars: content.chars().count(),
        child_count,
        children,
        content: with_content.then_some(content),
        source: sheet.file.clone(),
        fields: IndexMap::new(),
    }
}

/// Contiguous bands of non-blank rows inside the used range.
///
/// A workbook is usually several tables stacked with a blank row between them.
/// Splitting on those blanks gives each table its own address, which is what
/// lets an agent read one table instead of the whole sheet.
fn sheet_regions(sheet: &Sheet) -> Vec<(usize, usize, usize, usize)> {
    let Some(range) = used_range(&sheet.cells) else {
        return Vec::new();
    };
    let mut out: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for row in range.first_row..=range.max_row {
        let blank = (range.first_col..=range.max_col).all(|col| {
            sheet
                .cells
                .get(&to_ref(col, row))
                .map(|c| c.v.is_null() && c.f.is_none())
                .unwrap_or(true)
        });
        match (blank, start) {
            (false, None) => start = Some(row),
            (true, Some(r0)) => {
                out.push((range.first_col, r0, range.max_col, row - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(r0) = start {
        out.push((range.first_col, r0, range.max_col, range.max_row));
    }
    out
}

fn region_node(
    sheet: &Sheet,
    c0: usize,
    r0: usize,
    c1: usize,
    r1: usize,
    with_content: bool,
    depth: usize,
) -> Node {
    let range = format!("{}:{}", to_ref(c0, r0), to_ref(c1, r1));
    let rows = r1 - r0 + 1;
    let cols = c1 - c0 + 1;
    let header: Vec<String> = (c0..=c1)
        .map(|c| {
            sheet
                .cells
                .get(&to_ref(c, r0))
                .map(display_value)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| index_to_col(c))
        })
        .collect();
    let summary = format!("{rows}행 × {cols}열 — {}", clip(&header.join(" | "), 120));

    // A sheet's own column summary describes its leading table; a second table
    // below a blank row has different rows, so each region gets its own numbers.
    let all_summary = region_columns(sheet, c0, r0, c1, r1);
    let child_total = cols;
    let children = if depth > 0 {
        let mut out: Vec<Node> = Vec::new();
        for col in &all_summary {
            let stats = match (&col.sum, &col.avg, &col.min, &col.max) {
                (Some(sum), Some(avg), Some(min), Some(max)) => {
                    format!(" (합계 {sum}, 평균 {avg}, 범위 {min}~{max})")
                }
                _ => String::new(),
            };
            out.push(Node {
                path: format!("sheet/{}/region/{range}/col/{}", sheet.id, col.col),
                kind: "col".into(),
                title: col.col.clone(),
                summary: format!(
                    "{} — {}, 값 {}개{stats}",
                    col.header, col.col_type, col.count
                ),
                chars: 0,
                child_count: rows,
                children: Vec::new(),
                content: None,
                source: sheet.file.clone(),
                fields: IndexMap::new(),
            });
        }
        out
    } else {
        Vec::new()
    };

    let content = region_markdown(sheet, c0, r0, c1, r1);
    Node {
        path: format!("sheet/{}/region/{range}", sheet.id),
        kind: "region".into(),
        title: format!("데이터 영역 {range}"),
        summary,
        chars: content.chars().count(),
        child_count: child_total,
        children,
        content: with_content.then_some(content),
        source: sheet.file.clone(),
        fields: IndexMap::new(),
    }
}

/// A region as a markdown table, capped so one region cannot blow the context.
/// Per-column statistics for one region, using the region's own header row.
fn region_columns(
    sheet: &Sheet,
    c0: usize,
    r0: usize,
    c1: usize,
    r1: usize,
) -> Vec<crate::grid::ColumnSummary> {
    let mut out = Vec::new();
    for col in c0..=c1 {
        let letter = index_to_col(col);
        let header_text = sheet
            .cells
            .get(&to_ref(col, r0))
            .map(display_value)
            .unwrap_or_default();
        let header = if header_text.is_empty() {
            letter.clone()
        } else {
            header_text
        };
        let mut non_empty = 0usize;
        let mut numeric = 0usize;
        let mut text = 0usize;
        let mut sum = 0.0f64;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut fmt: Option<String> = None;
        let first_data_row = if r1 > r0 { r0 + 1 } else { r0 };
        for row in first_data_row..=r1 {
            // A total row would be counted twice — in the region's sum and in
            // the sheet's own total — so it is left out, exactly as the sheet
            // summary does.
            if crate::grid::is_aggregate_row(&sheet.cells, row, c1) {
                continue;
            }
            let Some(cell) = sheet.cells.get(&to_ref(col, row)) else {
                continue;
            };
            let shown = display_value(cell);
            if shown.is_empty() {
                continue;
            }
            non_empty += 1;
            if fmt.is_none() {
                fmt = cell.fmt.clone();
            }
            if let Some(n) = cell.v.as_f64() {
                numeric += 1;
                sum += n;
                min = min.min(n);
                max = max.max(n);
            } else {
                text += 1;
            }
        }
        let is_numeric = numeric > 0 && text == 0;
        let col_type = if non_empty == 0 {
            "빈 열"
        } else if is_numeric {
            "숫자"
        } else if numeric == 0 {
            "텍스트"
        } else {
            "혼합"
        };
        let f = fmt.as_deref();
        out.push(crate::grid::ColumnSummary {
            col: letter,
            header,
            col_type: col_type.to_string(),
            count: non_empty,
            sum: is_numeric.then(|| crate::grid::format_stat(sum, f)),
            avg: (is_numeric && numeric > 0)
                .then(|| crate::grid::format_stat(sum / numeric as f64, f)),
            min: (is_numeric && numeric > 0).then(|| crate::grid::format_stat(min, f)),
            max: (is_numeric && numeric > 0).then(|| crate::grid::format_stat(max, f)),
        });
    }
    out
}

fn region_markdown(sheet: &Sheet, c0: usize, r0: usize, c1: usize, r1: usize) -> String {
    const MAX_ROWS: usize = 200;
    let mut lines = vec![format!(
        "데이터 영역 `{}`",
        to_ref(c0, r0) + ":" + &to_ref(c1, r1)
    )];
    let mut md: Vec<Vec<String>> = Vec::new();
    for row in r0..=r1.min(r0 + MAX_ROWS) {
        let mut line: Vec<String> = Vec::new();
        for col in c0..=c1 {
            line.push(
                sheet
                    .cells
                    .get(&to_ref(col, row))
                    .map(display_value)
                    .unwrap_or_default(),
            );
        }
        md.push(line);
    }
    lines.push(crate::table::to_markdown_table(&md, &Default::default()));
    if r1 > r0 + MAX_ROWS {
        lines.push(format!("_… {}개 행 생략_", r1 - r0 - MAX_ROWS));
    }
    lines.join("\n\n")
}

fn cell_node(sheet: &Sheet, reference: &str) -> Option<Node> {
    let reference = reference.to_uppercase();
    let cell = sheet.cells.get(&reference)?;
    let shown = display_value(cell);
    let formula = cell.f.as_deref().unwrap_or("");
    let summary = if formula.is_empty() {
        clip(&shown, SUMMARY_CHARS)
    } else {
        format!("`{formula}` → {}", clip(&shown, SUMMARY_CHARS))
    };
    let mut fields = IndexMap::new();
    if !formula.is_empty() {
        fields.insert("formula".into(), json!(formula));
    }
    if let Some(t) = &cell.t {
        fields.insert("type".into(), json!(t));
    }
    if let Some(fmt) = &cell.fmt {
        fields.insert("format".into(), json!(fmt));
    }
    Some(Node {
        path: format!("sheet/{}/cell/{reference}", sheet.id),
        kind: "cell".into(),
        title: reference.clone(),
        summary: if summary.is_empty() {
            "(빈 값)".into()
        } else {
            summary
        },
        chars: shown.chars().count(),
        child_count: 0,
        children: Vec::new(),
        content: Some(shown),
        source: sheet.file.clone(),
        fields,
    })
}

fn chart_node(sheet: &Sheet, chart: &crate::model::SheetChart, with_content: bool) -> Node {
    let resolved = resolve_spec(&chart.spec, Some(sheet as &dyn CellSource));
    let child_total = resolved.series.len();
    Node {
        path: format!("sheet/{}/chart/{}", sheet.id, chart.id),
        kind: "chart".into(),
        title: chart.spec.title.clone(),
        summary: describe_chart(&resolved),
        chars: 0,
        child_count: child_total,
        children: Vec::new(),
        content: with_content.then(|| {
            format!(
                "{}\n\n{}",
                describe_chart(&resolved),
                crate::chart::chart_to_markdown_table(&resolved)
            )
        }),
        source: sheet.file.clone(),
        fields: IndexMap::new(),
    }
}

/* ------------------------------------------------------------------- map */

/// Render an outline as the compact markdown map `AI.md` carries.
///
/// Summaries only; the full digest follows below. A node budget keeps the map
/// bounded even for a workbook with thousands of rows.
pub fn map_markdown(root: &Node) -> String {
    const MAX_NODES: usize = 400;
    let mut lines: Vec<String> = vec![
        "## 문서 지도 (RLM)".into(),
        String::new(),
        "> 이 문서는 주소로 재귀 탐색할 수 있습니다. 먼저 이 지도를 읽고, 필요한 주소만 \
         `read_node(path, depth)`로 내려받으세요. 주소는 저장해도 바뀌지 않습니다."
            .into(),
        String::new(),
        "> 예: `project` · `slide/s_ab12` · `slide/s_ab12/block/b_7k2mx` · \
         `section/sec_ab12` · `sheet/sh_ab12/region/A1:F51` · `sheet/sh_ab12/cell/B4`"
            .into(),
        String::new(),
    ];
    let mut count = 0usize;
    render_map_line(root, 0, MAX_NODES, &mut count, &mut lines);
    if count >= MAX_NODES {
        lines.push(String::new());
        lines.push(format!(
            "_… 지도가 {MAX_NODES}개 노드에서 잘렸습니다. `read_node`로 하위를 이어서 읽으세요._"
        ));
    }
    lines.join("\n")
}

fn render_map_line(
    node: &Node,
    indent: usize,
    max_nodes: usize,
    count: &mut usize,
    lines: &mut Vec<String>,
) {
    if *count >= max_nodes {
        return;
    }
    *count += 1;
    let pad = "  ".repeat(indent);
    lines.push(format!(
        "{pad}- `{}` — {}",
        node.path,
        clip(&node.summary, 110)
    ));
    for child in &node.children {
        render_map_line(child, indent + 1, max_nodes, count, lines);
    }
}

/* ---------------------------------------------------------------- verify */

/// Walk the tree and report what contradicts the document's own structure.
///
/// This is the verification half of an RLM loop: after a model edits by
/// address, the same addresses are checked so a wrong change is named rather
/// than silently saved.
pub fn verify(project: &Project) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    match &project.items {
        Items::Slides(slides) => verify_deck(slides, &mut out),
        Items::Sections(sections) => verify_doc(sections, &mut out),
        Items::Sheets(sheets) => verify_grid(sheets, &mut out),
    }
    out
}

fn finding(out: &mut Vec<Finding>, path: String, level: &str, message: String) {
    if out.len() < VERIFY_LIMIT {
        out.push(Finding {
            path,
            level: level.into(),
            message,
        });
    }
}

fn verify_deck(slides: &[Slide], out: &mut Vec<Finding>) {
    for slide in slides {
        let base = format!("slide/{}", slide.id);
        if slide.blocks.is_empty() {
            finding(out, base.clone(), "warning", "빈 슬라이드입니다".into());
        }
        for block in &slide.blocks {
            let path = format!("{base}/block/{}", block.id);
            let g = block.geometry();
            let c = &slide.canvas;
            // A shape half off the slide is a design choice, and PowerPoint files
            // are full of them (entrances, bleeds). Only a shape that misses the
            // canvas entirely is certainly invisible, so only that is reported.
            let misses = g.x >= c.w || g.y >= c.h || g.x + g.w <= 0.0 || g.y + g.h <= 0.0;
            if misses {
                finding(
                    out,
                    path.clone(),
                    "warning",
                    format!(
                        "캔버스({}×{}) 밖에 있어 보이지 않습니다 — [{:.0},{:.0}]+{:.0}×{:.0}",
                        c.w as i64, c.h as i64, g.x, g.y, g.w, g.h
                    ),
                );
            }
            if g.w < 1.0 || g.h < 1.0 {
                finding(out, path.clone(), "error", "크기가 0입니다".into());
            }
            match block.kind {
                Kind::Table => {
                    if block.md.trim().is_empty() {
                        finding(out, path.clone(), "error", "표에 내용이 없습니다".into());
                    }
                    if let Some(spec) = &block.table {
                        let (rows, cols, _) = table_shape(&block.md);
                        for merge in &spec.merges {
                            let ok = parse_range(&merge.to_uppercase())
                                .map(|r| r.end.col < cols && r.end.row < rows)
                                .unwrap_or(false);
                            if !ok {
                                finding(
                                    out,
                                    path.clone(),
                                    "error",
                                    format!("병합 `{merge}`가 표({rows}×{cols})를 벗어납니다"),
                                );
                            }
                        }
                    }
                }
                Kind::Chart => {
                    if chart_description(&block.md).is_none() {
                        finding(
                            out,
                            path.clone(),
                            "error",
                            "차트 블록을 해석할 수 없습니다".into(),
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

fn verify_doc(sections: &[Section], out: &mut Vec<Finding>) {
    for section in sections {
        let base = format!("section/{}", section.id);
        if section.blocks.is_empty() {
            finding(out, base.clone(), "warning", "빈 섹션입니다".into());
        }
        // A jump from h1 to h3 loses a level in every converter that reads the
        // outline; report it with the heading's own address.
        let keys = doc_block_keys(&section.blocks);
        let mut previous = 0usize;
        for (i, block) in section.blocks.iter().enumerate() {
            let path = format!("{base}/block/{}", keys[i]);
            let level = heading_level(&block.md);
            if level > 0 {
                if previous > 0 && level > previous + 1 {
                    finding(
                        out,
                        path.clone(),
                        "warning",
                        format!("제목 단계가 {previous}에서 {level}로 건너뜁니다"),
                    );
                }
                previous = level;
            }
            if block.block_type == BlockType::Table {
                let (rows, cols, _) = table_shape(&block.md);
                if rows == 0 || cols == 0 {
                    finding(out, path, "error", "표를 해석할 수 없습니다".into());
                }
            }
        }
    }
}

/// The error display Excel and the engine both produce.
fn is_error_value(shown: &str) -> bool {
    matches!(
        shown,
        "#REF!"
            | "#DIV/0!"
            | "#VALUE!"
            | "#NAME?"
            | "#NUM!"
            | "#NULL!"
            | "#N/A"
            | "#SPILL!"
            | "#CALC!"
    )
}

fn verify_grid(sheets: &[Sheet], out: &mut Vec<Finding>) {
    for sheet in sheets {
        let base = format!("sheet/{}", sheet.id);
        for (reference, cell) in &sheet.cells {
            let shown = display_value(cell);
            if is_error_value(&shown) {
                finding(
                    out,
                    format!("{base}/cell/{reference}"),
                    "warning",
                    format!("수식 오류 {shown}"),
                );
            }
        }
        for merge in &sheet.merges {
            let spec = merge.as_str().unwrap_or_default().to_uppercase();
            if parse_range(&spec).is_none() {
                finding(
                    out,
                    base.clone(),
                    "error",
                    format!("병합 범위 `{spec}`를 해석할 수 없습니다"),
                );
            }
        }
        for (name, target) in &sheet.names {
            let value = match target {
                Json::String(s) => s.clone(),
                other => other.to_string(),
            };
            if !value.contains('!') && parse_range(&value.to_uppercase()).is_none() {
                finding(
                    out,
                    format!("{base}/name/{name}"),
                    "warning",
                    format!("이름 범위 대상 `{value}`를 해석할 수 없습니다"),
                );
            }
        }
        for chart in &sheet.charts {
            if let Some(range) = &chart.spec.range {
                if used_range(&sheet.cells).is_none() {
                    finding(
                        out,
                        format!("{base}/chart/{}", chart.id),
                        "warning",
                        format!("차트가 가리키는 `{range}`의 시트가 비어 있습니다"),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Manifest, ProjectType, Theme};

    fn deck() -> Project {
        let slides = vec![
            {
                let mut s = crate::deck::make_slide("title", Some("표지"), 1);
                s.id = "s_a".into();
                s.blocks[0].id = "b_a".into();
                s
            },
            {
                let mut s = crate::deck::make_slide("title-content", Some("개요"), 2);
                s.id = "s_b".into();
                s.blocks[0].id = "b_b".into();
                s
            },
        ];
        Project {
            project_type: ProjectType::Deck,
            dir: std::path::PathBuf::from("/tmp"),
            manifest: Manifest {
                format: ProjectType::Deck.format_id().into(),
                format_version: 1,
                id: "prj_test".into(),
                title: "테스트".into(),
                created: "2026-01-01".into(),
                modified: "2026-01-01".into(),
                theme: Theme::default(),
                entries: Vec::new(),
            },
            items: Items::Slides(slides),
        }
    }

    #[test]
    fn an_outline_carries_summaries_and_no_content() {
        let outline = outline(&deck());
        assert_eq!(outline.path, ROOT_PATH);
        assert_eq!(outline.children.len(), 2);
        assert!(outline.content.is_none());
        assert!(outline.children.iter().all(|c| c.content.is_none()));
        assert!(outline.children[0].summary.contains("표지"));
    }

    #[test]
    fn resolve_by_id_and_by_position_agree() {
        let project = deck();
        let by_id = resolve(&project, "slide/s_a", 1).unwrap();
        let by_position = resolve(&project, "slide/1", 1).unwrap();
        assert_eq!(by_id.path, by_position.path);
        assert!(by_id.content.as_deref().unwrap().contains("표지"));
        assert!(!by_id.children.is_empty());
        assert_eq!(by_id.children.len(), by_position.children.len());
    }

    #[test]
    fn resolve_reaches_a_block_and_a_missing_path_is_none() {
        let project = deck();
        let block = resolve(&project, "slide/s_a/block/b_a", 0).unwrap();
        assert_eq!(block.kind, "block/text");
        assert!(block.content.is_some());
        assert!(block.summary.contains("정중앙") || block.summary.contains("상단"));
        assert!(resolve(&project, "slide/nope", 0).is_none());
    }

    #[test]
    fn the_map_lists_addresses_and_stays_bounded() {
        let map = map_markdown(&outline(&deck()));
        assert!(map.contains("## 문서 지도 (RLM)"));
        assert!(map.contains("`slide/s_a`"));
        assert!(map.contains("`slide/s_a/block/b_a`"));
        assert!(map.contains("read_node"));
    }

    #[test]
    fn verify_names_an_off_canvas_block() {
        let mut project = deck();
        let Items::Slides(slides) = &mut project.items else {
            panic!()
        };
        slides[0].blocks[0].x = 5000.0;
        let findings = verify(&project);
        assert!(
            findings
                .iter()
                .any(|f| f.path == "slide/s_a/block/b_a" && f.message.contains("보이지 않습니다")),
            "{findings:?}"
        );
    }
}
