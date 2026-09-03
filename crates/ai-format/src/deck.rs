//! Deck: merging a slide's markdown with its layout JSON, and splitting it back.

use indexmap::IndexMap;
use serde_json::{json, Value as Json};

use crate::blocks::{block_label, infer_kind, join_blocks, split_blocks, Joinable, Kind};
use crate::frontmatter::{meta_str, parse_frontmatter, serialize_frontmatter, Meta};
use crate::geometry::{auto_layout, clamp_box, Box, Canvas, DEFAULT_CANVAS};
use crate::ids::{new_block_id, new_slide_id};
use crate::model::{Slide, SlideBlock, SLIDE_LAYOUTS};
use crate::shape::ShapeSpec;
use crate::table::{apply_markdown_authority, TableSpec};

/// PowerPoint's single ("100%") line spacing, as the CSS multiplier the editor
/// draws with. A line at 100% in PowerPoint is the font's own height — about
/// 1.2× its size — not 1.0×, so `spcPct` and `lineHeight` convert through this
/// factor in both directions and a deck's spacing looks the same on both sides.
pub const SINGLE_SPACING: f64 = 1.2;

/// Default text styling, applied to text blocks that carry no style of their own.
pub fn default_text_style() -> IndexMap<String, Json> {
    [
        ("fontSize", json!(20)),
        ("weight", json!(400)),
        ("align", json!("left")),
        ("valign", json!("top")),
        ("color", json!("#1f2937")),
        ("lineHeight", json!(1.45)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

fn layout_name_or_default(candidate: Option<&str>) -> String {
    match candidate {
        Some(name) if SLIDE_LAYOUTS.contains(&name) => name.to_string(),
        _ => "title-content".to_string(),
    }
}

fn num(v: Option<&Json>, fallback: f64) -> f64 {
    match v {
        Some(Json::Number(n)) => n.as_f64().filter(|x| x.is_finite()).unwrap_or(fallback),
        Some(Json::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .unwrap_or(fallback),
        _ => fallback,
    }
}

/// Merge a slide's markdown with its layout JSON into one editable object.
///
/// Either input may be missing or stale: markdown is authoritative for *which*
/// blocks exist and what they say, JSON is authoritative for where they sit.
/// Blocks present only in JSON are dropped; blocks present only in markdown get
/// an auto layout.
pub fn read_slide(md: &str, layout: Option<&Json>) -> Slide {
    let split = parse_frontmatter(md);
    let parsed = split_blocks(&split.body);

    let geo = layout
        .and_then(|l| l.get("blocks"))
        .and_then(|b| b.as_object())
        .cloned()
        .unwrap_or_default();
    let canvas = read_canvas(layout.and_then(|l| l.get("canvas")));

    let total = parsed.len();
    let blocks: Vec<SlideBlock> = parsed
        .iter()
        .enumerate()
        .map(|(i, region)| {
            let g = geo.get(&region.id);
            let kind = g
                .and_then(|g| g.get("kind"))
                .and_then(|k| k.as_str())
                .and_then(Kind::from_name)
                .or_else(|| region.hint.get("kind").and_then(|k| Kind::from_name(k)))
                .unwrap_or_else(|| infer_kind(&region.md));

            let fallback = auto_layout(i, total, &canvas);
            let boxed = clamp_box(
                Box {
                    x: num(g.and_then(|g| g.get("x")), fallback.x),
                    y: num(g.and_then(|g| g.get("y")), fallback.y),
                    w: num(g.and_then(|g| g.get("w")), fallback.w),
                    h: num(g.and_then(|g| g.get("h")), fallback.h),
                    z: num(g.and_then(|g| g.get("z")), fallback.z),
                },
                &canvas,
            );

            let saved_style = g
                .and_then(|g| g.get("style"))
                .and_then(|s| s.as_object())
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_else(IndexMap::new);
            let style = if kind == Kind::Text {
                let mut merged = default_text_style();
                merged.extend(saved_style);
                merged
            } else {
                saved_style
            };

            SlideBlock {
                id: region.id.clone(),
                kind,
                md: region.md.clone(),
                x: boxed.x,
                y: boxed.y,
                w: boxed.w,
                h: boxed.h,
                z: boxed.z,
                style,
                shape: read_shape(kind, g),
                table: read_table(kind, g, &region.md),
                locked: g
                    .and_then(|g| g.get("locked"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }
        })
        .collect();

    Slide {
        id: meta_str(&split.meta, "id")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                layout
                    .and_then(|l| l.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(new_slide_id),
        title: meta_str(&split.meta, "title").unwrap_or_else(|| derive_title(&blocks)),
        layout_name: layout_name_or_default(meta_str(&split.meta, "layout").as_deref()),
        notes: meta_str(&split.meta, "notes").unwrap_or_default(),
        canvas,
        blocks,
        layout_part: meta_str(&split.meta, "layoutPart").filter(|s| !s.is_empty()),
        master_shapes: split
            .meta
            .get("masterShapes")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        hidden: split
            .meta
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        file: None,
    }
}

/// A shape's geometry, defaulted so a hand-written `kind: "shape"` still draws.
fn read_shape(kind: Kind, entry: Option<&Json>) -> Option<ShapeSpec> {
    if kind != Kind::Shape {
        return None;
    }
    let parsed = entry
        .and_then(|g| g.get("shape"))
        .and_then(|v| serde_json::from_value::<ShapeSpec>(v.clone()).ok());
    Some(parsed.unwrap_or_default())
}

/// A table's layout.
///
/// The markdown is the authority for anything it can express, so the alignment
/// row wins over a stale `align` in the JSON. Editing `|---:|` by hand works.
fn read_table(kind: Kind, entry: Option<&Json>, md: &str) -> Option<TableSpec> {
    if kind != Kind::Table {
        return None;
    }
    let mut spec = entry
        .and_then(|g| g.get("table"))
        .and_then(|v| serde_json::from_value::<TableSpec>(v.clone()).ok())
        .unwrap_or_default();
    apply_markdown_authority(&mut spec, md);
    Some(spec)
}

fn read_canvas(value: Option<&Json>) -> Canvas {
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return DEFAULT_CANVAS;
    };
    Canvas {
        w: num(obj.get("w"), DEFAULT_CANVAS.w),
        h: num(obj.get("h"), DEFAULT_CANVAS.h),
        bg: obj
            .get("bg")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string().into())
            .unwrap_or(DEFAULT_CANVAS.bg),
    }
}

/// A slide split into the markdown + layout JSON pair written to disk.
pub struct SlideFiles {
    pub md: String,
    pub layout: Json,
}

pub fn write_slide(slide: &Slide) -> SlideFiles {
    let s = normalize_slide(slide);

    let mut meta = Meta::new();
    meta.insert("id".into(), json!(s.id));
    meta.insert("title".into(), json!(s.title));
    meta.insert("layout".into(), json!(s.layout_name));
    if let Some(part) = &s.layout_part {
        meta.insert("layoutPart".into(), json!(part));
    }
    if !s.master_shapes {
        meta.insert("masterShapes".into(), json!(false));
    }
    if s.hidden {
        meta.insert("hidden".into(), json!(true));
    }
    if !s.notes.is_empty() {
        meta.insert("notes".into(), json!(s.notes));
    }
    let joinable: Vec<Joinable> = s
        .blocks
        .iter()
        .map(|b| Joinable {
            id: &b.id,
            md: &b.md,
            omit_marker: false,
        })
        .collect();
    let md = serialize_frontmatter(&meta, &join_blocks(&joinable));

    let mut blocks = serde_json::Map::new();
    for b in &s.blocks {
        let mut entry = serde_json::Map::new();
        entry.insert("x".into(), json!(b.x));
        entry.insert("y".into(), json!(b.y));
        entry.insert("w".into(), json!(b.w));
        entry.insert("h".into(), json!(b.h));
        entry.insert("z".into(), json!(b.z));
        entry.insert("kind".into(), json!(b.kind.as_str()));
        if b.locked {
            entry.insert("locked".into(), json!(true));
        }
        let style = written_style(b);
        if !style.is_empty() {
            entry.insert(
                "style".into(),
                serde_json::to_value(&style).unwrap_or(Json::Null),
            );
        }
        if let Some(shape) = &b.shape {
            entry.insert(
                "shape".into(),
                serde_json::to_value(shape).unwrap_or(Json::Null),
            );
        }
        if let Some(table) = &b.table {
            entry.insert(
                "table".into(),
                serde_json::to_value(table).unwrap_or(Json::Null),
            );
        }
        blocks.insert(b.id.clone(), Json::Object(entry));
    }

    SlideFiles {
        md,
        layout: json!({
            "id": s.id,
            "canvas": { "w": s.canvas.w, "h": s.canvas.h, "bg": s.canvas.bg.as_str() },
            "blocks": Json::Object(blocks),
        }),
    }
}

/// The style entries worth writing: for a text block, the ones that differ from
/// the default. `read_slide` merges the default back under whatever is saved, so
/// an entry that equals it says nothing — the same rule Doc applies to overrides.
/// Non-text blocks get no default merge on read, so their style is kept whole.
fn written_style(b: &SlideBlock) -> IndexMap<String, Json> {
    if b.kind != Kind::Text {
        return b.style.clone();
    }
    let defaults = default_text_style();
    b.style
        .iter()
        .filter(|(k, v)| defaults.get(*k).is_none_or(|d| !json_value_eq(d, v)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Equality that treats `20` and `20.0` as the same number — the editor state
/// crosses the wasm boundary, which does not preserve a number's JSON flavour.
fn json_value_eq(a: &Json, b: &Json) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

pub fn normalize_slide(slide: &Slide) -> Slide {
    let canvas = slide.canvas;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let blocks: Vec<SlideBlock> = slide
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut id = if b.id.is_empty() {
                new_block_id()
            } else {
                b.id.clone()
            };
            while seen.contains(&id) {
                id = new_block_id();
            }
            seen.insert(id.clone());

            let fallback = auto_layout(i, 1, &canvas);
            let boxed = clamp_box(
                Box {
                    x: finite_or(b.x, fallback.x),
                    y: finite_or(b.y, fallback.y),
                    w: finite_or(b.w, 400.0),
                    h: finite_or(b.h, 120.0),
                    z: finite_or(b.z, i as f64 + 1.0),
                },
                &canvas,
            );
            // A block whose kind says shape or table must carry the matching
            // spec, or the renderer has nothing to draw; one whose kind says
            // otherwise must not, or a stale spec would resurface on a re-typed
            // block.
            let shape = match b.kind {
                Kind::Shape => Some(b.shape.clone().unwrap_or_default()),
                _ => None,
            };
            let table = match b.kind {
                Kind::Table => {
                    let mut spec = b.table.clone().unwrap_or_default();
                    apply_markdown_authority(&mut spec, &b.md);
                    Some(spec)
                }
                _ => None,
            };
            SlideBlock {
                id,
                kind: b.kind,
                md: b.md.clone(),
                x: boxed.x,
                y: boxed.y,
                w: boxed.w,
                h: boxed.h,
                z: boxed.z,
                style: b.style.clone(),
                shape,
                table,
                locked: b.locked,
            }
        })
        .collect();

    Slide {
        id: if slide.id.is_empty() {
            new_slide_id()
        } else {
            slide.id.clone()
        },
        title: if slide.title.is_empty() {
            derive_title(&blocks)
        } else {
            slide.title.clone()
        },
        layout_name: layout_name_or_default(Some(&slide.layout_name)),
        notes: slide.notes.clone(),
        canvas,
        blocks,
        layout_part: slide.layout_part.clone(),
        master_shapes: slide.master_shapes,
        hidden: slide.hidden,
        file: slide.file.clone(),
    }
}

fn finite_or(v: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

fn derive_title(blocks: &[SlideBlock]) -> String {
    for b in blocks {
        let label = block_label(&b.md, 80);
        if !label.is_empty() {
            return label;
        }
    }
    "제목 없는 슬라이드".to_string()
}

fn text_block(md: String, b: Box, style: &[(&str, Json)]) -> SlideBlock {
    let mut merged = default_text_style();
    for (k, v) in style {
        merged.insert(k.to_string(), v.clone());
    }
    SlideBlock {
        id: new_block_id(),
        kind: Kind::Text,
        md,
        x: b.x,
        y: b.y,
        w: b.w,
        h: b.h,
        z: b.z,
        style: merged,
        shape: None,
        table: None,
        locked: false,
    }
}

/// A shape block, the way the gallery inserts one.
pub fn make_shape(preset: &str, b: Box) -> SlideBlock {
    SlideBlock {
        id: new_block_id(),
        kind: Kind::Shape,
        md: String::new(),
        x: b.x,
        y: b.y,
        w: b.w,
        h: b.h,
        z: b.z,
        style: [
            ("align".to_string(), json!("center")),
            ("valign".to_string(), json!("middle")),
            ("fontSize".to_string(), json!(18)),
            ("color".to_string(), json!("#1f2937")),
        ]
        .into_iter()
        .collect(),
        shape: Some(ShapeSpec {
            preset: preset.to_string(),
            ..ShapeSpec::default()
        }),
        table: None,
        locked: false,
    }
}

/// A table block of the given size, the way Office's grid picker inserts one.
pub fn make_table(columns: usize, rows: usize, b: Box) -> SlideBlock {
    let (md, table) = crate::table::blank_table(columns, rows);
    SlideBlock {
        id: new_block_id(),
        kind: Kind::Table,
        md,
        x: b.x,
        y: b.y,
        w: b.w,
        h: b.h,
        z: b.z,
        style: IndexMap::new(),
        shape: None,
        table: Some(table),
        locked: false,
    }
}

fn bx(x: f64, y: f64, w: f64, h: f64, z: f64) -> Box {
    Box { x, y, w, h, z }
}

/// Blank slide for a chosen semantic layout, pre-populated like PowerPoint does.
pub fn make_slide(layout_name: &str, title: Option<&str>, index: usize) -> Slide {
    let blocks = match layout_name {
        "title" => vec![
            text_block(
                format!("# {}", title.unwrap_or("프레젠테이션 제목")),
                bx(96.0, 260.0, 1088.0, 120.0, 1.0),
                &[
                    ("fontSize", json!(54)),
                    ("weight", json!(700)),
                    ("align", json!("center")),
                ],
            ),
            text_block(
                "부제목을 입력하세요".to_string(),
                bx(96.0, 396.0, 1088.0, 60.0, 2.0),
                &[
                    ("fontSize", json!(22)),
                    ("align", json!("center")),
                    ("color", json!("#6b7280")),
                ],
            ),
        ],
        "section" => vec![text_block(
            format!("## {}", title.unwrap_or("섹션")),
            bx(96.0, 300.0, 1088.0, 100.0, 1.0),
            &[
                ("fontSize", json!(40)),
                ("weight", json!(600)),
                ("align", json!("center")),
            ],
        )],
        "two-column" => vec![
            text_block(
                format!(
                    "# {}",
                    title
                        .map(str::to_string)
                        .unwrap_or(format!("슬라이드 {index}"))
                ),
                bx(96.0, 72.0, 1088.0, 80.0, 1.0),
                &[("fontSize", json!(36)), ("weight", json!(700))],
            ),
            text_block(
                "- 왼쪽 항목".to_string(),
                bx(96.0, 196.0, 512.0, 420.0, 2.0),
                &[],
            ),
            text_block(
                "- 오른쪽 항목".to_string(),
                bx(672.0, 196.0, 512.0, 420.0, 3.0),
                &[],
            ),
        ],
        "blank" => Vec::new(),
        _ => vec![
            text_block(
                format!(
                    "# {}",
                    title
                        .map(str::to_string)
                        .unwrap_or(format!("슬라이드 {index}"))
                ),
                bx(96.0, 72.0, 1088.0, 80.0, 1.0),
                &[("fontSize", json!(36)), ("weight", json!(700))],
            ),
            text_block(
                "- 첫 번째 항목\n- 두 번째 항목".to_string(),
                bx(96.0, 196.0, 1088.0, 420.0, 2.0),
                &[],
            ),
        ],
    };

    Slide {
        id: new_slide_id(),
        title: title
            .map(str::to_string)
            .unwrap_or(format!("슬라이드 {index}")),
        layout_name: layout_name_or_default(Some(layout_name)),
        notes: String::new(),
        canvas: DEFAULT_CANVAS,
        blocks,
        layout_part: None,
        master_shapes: true,
        hidden: false,
        file: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_readme_example() {
        let md = "---\nid: s_9k2mx\ntitle: 핵심 성과\nlayout: title-content\nnotes: 전년 동기 대비라는 점을 반드시 짚는다.\n---\n\n<!-- block:b_3fh2a -->\n# 핵심 성과\n\n<!-- block:b_7dk1p -->\n- **매출 142억**\n";
        let layout = json!({
            "id": "s_9k2mx",
            "canvas": { "w": 1280, "h": 720, "bg": "#ffffff" },
            "blocks": {
                "b_3fh2a": { "x": 96, "y": 64, "w": 1088, "h": 70, "z": 1, "kind": "text",
                             "style": { "fontSize": 38, "weight": 700 } }
            }
        });
        let slide = read_slide(md, Some(&layout));
        assert_eq!(slide.id, "s_9k2mx");
        assert_eq!(slide.title, "핵심 성과");
        assert_eq!(slide.notes, "전년 동기 대비라는 점을 반드시 짚는다.");
        assert_eq!(slide.blocks.len(), 2);
        assert_eq!((slide.blocks[0].x, slide.blocks[0].y), (96.0, 64.0));
        assert_eq!(slide.blocks[0].style["fontSize"], json!(38));
        // The default style is merged under the saved one.
        assert_eq!(slide.blocks[0].style["align"], json!("left"));
        // Block 2 has no JSON entry, so it gets an auto layout rather than 0,0.
        assert!(slide.blocks[1].y > 0.0);
    }

    #[test]
    fn coordinates_never_leak_into_markdown() {
        let slide = make_slide("title-content", Some("테스트"), 1);
        let files = write_slide(&slide);
        for needle in ["\"x\"", "x=", "\"w\"", "fontSize"] {
            assert!(
                !files.md.contains(needle),
                "md leaked {needle}: {}",
                files.md
            );
        }
    }

    #[test]
    fn text_never_leaks_into_layout_json() {
        let slide = make_slide("title-content", Some("고유한제목입니다"), 1);
        let files = write_slide(&slide);
        let json = serde_json::to_string(&files.layout).unwrap();
        assert!(!json.contains("고유한제목입니다"), "{json}");
        assert!(!json.contains("첫 번째 항목"), "{json}");
    }

    #[test]
    fn default_equal_style_entries_stay_out_of_the_layout() {
        // The body block of the default layout carries only default styling, so
        // the layout entry should say nothing about style at all — the same rule
        // Doc applies to overrides. `20.0` exercises the wasm-boundary number.
        let mut slide = make_slide("title-content", Some("제목"), 1);
        slide.blocks[1]
            .style
            .insert("fontSize".to_string(), json!(20.0));
        let files = write_slide(&slide);
        let body = &files.layout["blocks"][&slide.blocks[1].id];
        assert!(body.get("style").is_none(), "{body}");
        // The title's non-default entries survive, the default ones do not.
        let title = &files.layout["blocks"][&slide.blocks[0].id];
        assert_eq!(title["style"]["fontSize"], json!(36));
        assert!(title["style"].get("align").is_none(), "{title}");
        // Reading back restores the defaults, so nothing was lost.
        let back = read_slide(&files.md, Some(&files.layout));
        assert_eq!(back.blocks[1].style["fontSize"], json!(20));
        assert_eq!(back.blocks[0].style["valign"], json!("top"));
    }

    #[test]
    fn a_shape_style_is_written_whole() {
        // Non-text blocks get no default merge on read, so their style must
        // land on disk even where it happens to match the text default.
        let mut slide = make_slide("blank", None, 1);
        slide.blocks.push(make_shape("rect", box_dims()));
        let files = write_slide(&slide);
        let entry = &files.layout["blocks"][&slide.blocks[0].id];
        assert_eq!(entry["style"]["valign"], json!("middle"));
        assert_eq!(entry["style"]["color"], json!("#1f2937"));
    }

    fn box_dims() -> Box {
        Box {
            x: 10.0,
            y: 10.0,
            w: 200.0,
            h: 100.0,
            z: 1.0,
        }
    }

    #[test]
    fn a_slide_round_trips_through_disk_form() {
        let original = make_slide("two-column", Some("분기 리뷰"), 3);
        let files = write_slide(&original);
        let back = read_slide(&files.md, Some(&files.layout));
        assert_eq!(back.id, original.id);
        assert_eq!(back.title, original.title);
        assert_eq!(back.layout_name, original.layout_name);
        assert_eq!(back.blocks.len(), original.blocks.len());
        for (a, b) in back.blocks.iter().zip(&original.blocks) {
            assert_eq!(
                (a.id.as_str(), a.md.as_str()),
                (b.id.as_str(), b.md.as_str())
            );
            assert_eq!((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h));
            assert_eq!(a.kind, b.kind);
        }
    }

    #[test]
    fn a_hand_written_block_appears_with_an_auto_layout() {
        let md = "---\nid: s_1\ntitle: T\nlayout: blank\n---\n\n<!-- block:known -->\n# A\n\n<!-- block:added_by_hand -->\n손으로 추가\n";
        let layout =
            json!({ "blocks": { "known": { "x": 10, "y": 10, "w": 100, "h": 50, "z": 1 } } });
        let slide = read_slide(md, Some(&layout));
        assert_eq!(slide.blocks.len(), 2);
        let added = &slide.blocks[1];
        assert_eq!(added.id, "added_by_hand");
        assert!(added.w > 0.0 && added.h > 0.0);
    }

    #[test]
    fn a_block_only_in_json_is_dropped() {
        let md = "---\nid: s_1\n---\n\n<!-- block:kept -->\nA\n";
        let layout = json!({ "blocks": {
            "kept": { "x": 0, "y": 0, "w": 100, "h": 50 },
            "ghost": { "x": 0, "y": 0, "w": 100, "h": 50 },
        }});
        assert_eq!(read_slide(md, Some(&layout)).blocks.len(), 1);
    }

    #[test]
    fn markdown_alone_still_opens() {
        let slide = read_slide("# 제목만 있는 슬라이드\n\n본문", None);
        assert_eq!(slide.blocks.len(), 1);
        assert_eq!(slide.title, "제목만 있는 슬라이드");
        assert_eq!(slide.layout_name, "title-content");
        assert_eq!(slide.canvas.w, 1280.0);
    }

    #[test]
    fn an_unknown_layout_name_falls_back() {
        let slide = read_slide("---\nlayout: fancy\n---\n\nA", None);
        assert_eq!(slide.layout_name, "title-content");
    }

    #[test]
    fn blank_layout_makes_an_empty_slide() {
        let slide = make_slide("blank", None, 1);
        assert!(slide.blocks.is_empty());
        let files = write_slide(&slide);
        assert_eq!(files.layout["blocks"], json!({}));
    }
}

#[cfg(test)]
mod shape_table_tests {
    use super::*;
    use crate::shape::{Dash, Fill, Line};
    use crate::table::TableStyle;

    fn box_of() -> Box {
        Box {
            x: 100.0,
            y: 120.0,
            w: 300.0,
            h: 200.0,
            z: 3.0,
        }
    }

    #[test]
    fn a_shape_keeps_its_preset_through_disk() {
        let mut slide = make_slide("blank", Some("도형"), 1);
        let mut shape = make_shape("flowChartDecision", box_of());
        shape.md = "승인?".into();
        shape.shape = Some(ShapeSpec {
            preset: "flowChartDecision".into(),
            fill: Some(Fill {
                color: "#dbeafe".into(),
                opacity: 100.0,
            }),
            line: Some(Line {
                color: "#2a78d6".into(),
                width: 2.0,
                dash: Dash::Dash,
            }),
            rotation: 15.0,
            flip_h: true,
            flip_v: false,
            adjust: [("adj".to_string(), 20000.0)].into_iter().collect(),
        });
        slide.blocks.push(shape);

        let files = write_slide(&slide);
        // The text is in the markdown, the geometry in the JSON — as everywhere.
        assert!(files.md.contains("승인?"), "{}", files.md);
        let json = serde_json::to_string(&files.layout).unwrap();
        assert!(
            !json.contains("승인"),
            "text leaked into the layout: {json}"
        );

        let back = read_slide(&files.md, Some(&files.layout));
        let spec = back.blocks[0].shape.as_ref().expect("the shape survived");
        assert_eq!(spec.preset, "flowChartDecision");
        assert_eq!(spec.label(), "판단");
        assert_eq!(spec.rotation, 15.0);
        assert!(spec.flip_h && !spec.flip_v);
        assert_eq!(spec.adjust["adj"], 20000.0);
        assert_eq!(spec.line.as_ref().unwrap().dash, Dash::Dash);
        assert_eq!(back.blocks[0].md, "승인?");
    }

    #[test]
    fn a_shape_with_no_fill_stays_unfilled() {
        let mut slide = make_slide("blank", None, 1);
        let mut shape = make_shape("line", box_of());
        shape.shape = Some(ShapeSpec {
            preset: "line".into(),
            fill: None,
            line: Some(Line {
                color: "#000000".into(),
                width: 1.5,
                dash: Dash::Solid,
            }),
            ..ShapeSpec::default()
        });
        slide.blocks.push(shape);

        let files = write_slide(&slide);
        let back = read_slide(&files.md, Some(&files.layout));
        assert!(back.blocks[0].shape.as_ref().unwrap().fill.is_none());
    }

    #[test]
    fn a_table_keeps_its_text_in_markdown_and_layout_in_json() {
        let mut slide = make_slide("blank", Some("표"), 1);
        let mut table = make_table(3, 3, box_of());
        table.md = "| 항목 | 1분기 | 2분기 |\n|---|---:|---:|\n| 제품 A | 1,200 | 1,350 |\n| 제품 B | 980 | 1,120 |".into();
        table.table = Some(TableSpec {
            cols: vec![180.0, 120.0, 120.0],
            rows: vec![32.0, 28.0, 28.0],
            merges: vec!["B1:C1".into()],
            header_row: true,
            banded_rows: true,
            first_col: true,
            style: TableStyle::Plain,
            cells: Default::default(),
        });
        slide.blocks.push(table);

        let files = write_slide(&slide);
        assert!(
            files.md.contains("| 제품 A | 1,200 | 1,350 |"),
            "{}",
            files.md
        );
        let json = serde_json::to_string(&files.layout).unwrap();
        assert!(
            !json.contains("제품 A"),
            "cell text leaked into the layout: {json}"
        );
        assert!(json.contains("\"cols\":[180"), "{json}");

        let back = read_slide(&files.md, Some(&files.layout));
        let spec = back.blocks[0].table.as_ref().expect("the table survived");
        assert_eq!(spec.cols, vec![180.0, 120.0, 120.0]);
        assert_eq!(spec.merges, vec!["B1:C1"]);
        assert_eq!(spec.style, TableStyle::Plain);
        assert!(spec.first_col);
        // The markdown's alignment row is the authority for alignment.
        assert_eq!(spec.cells["B1"].align.as_deref(), Some("right"));
        assert_eq!(spec.cells["C1"].align.as_deref(), Some("right"));
    }

    #[test]
    fn a_hand_edited_alignment_row_wins_over_stale_json() {
        let md = "<!-- block:b_t -->\n| a | b |\n|---|:---:|\n| 1 | 2 |\n";
        let layout = json!({
            "blocks": {
                "b_t": {
                    "x": 0, "y": 0, "w": 400, "h": 120, "z": 1, "kind": "table",
                    "table": { "cells": { "B1": { "align": "left" } } }
                }
            }
        });
        let slide = read_slide(md, Some(&layout));
        let spec = slide.blocks[0].table.as_ref().unwrap();
        assert_eq!(spec.cells["B1"].align.as_deref(), Some("center"));
    }

    #[test]
    fn a_merge_that_no_longer_fits_is_dropped() {
        // The user deleted a column by editing the markdown.
        let md = "<!-- block:b_t -->\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let layout = json!({
            "blocks": {
                "b_t": {
                    "x": 0, "y": 0, "w": 400, "h": 120, "z": 1, "kind": "table",
                    "table": { "merges": ["A1:B1", "C1:D1"] }
                }
            }
        });
        let slide = read_slide(md, Some(&layout));
        assert_eq!(
            slide.blocks[0].table.as_ref().unwrap().merges,
            vec!["A1:B1"]
        );
    }

    #[test]
    fn changing_a_blocks_kind_drops_the_stale_spec() {
        let mut slide = make_slide("blank", None, 1);
        slide.blocks.push(make_shape("ellipse", box_of()));
        // The user turned it into a text box.
        slide.blocks[0].kind = Kind::Text;
        let normalized = normalize_slide(&slide);
        assert!(normalized.blocks[0].shape.is_none());
        assert!(normalized.blocks[0].table.is_none());
    }

    #[test]
    fn a_hand_written_shape_block_still_draws() {
        // Only `kind` given; the reader fills in a usable default.
        let md = "<!-- block:b_s -->\n손으로 쓴 도형\n";
        let layout = json!({
            "blocks": { "b_s": { "x": 10, "y": 10, "w": 200, "h": 100, "z": 1, "kind": "shape" } }
        });
        let slide = read_slide(md, Some(&layout));
        let spec = slide.blocks[0].shape.as_ref().unwrap();
        assert_eq!(spec.preset, "rect");
        assert!(spec.fill.is_some());
    }
}
