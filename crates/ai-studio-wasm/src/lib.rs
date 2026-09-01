//! The AI Studio core, compiled for the browser.
//!
//! The editors need the formula engine and the serializers synchronously — a
//! keystroke in a cell recalculates its dependents before the next paint, and
//! the `{ } 저장 포맷` panel re-renders as you type. Routing that through
//! asynchronous IPC would mean rewriting the editors around promises, so the
//! same Rust crates are compiled to WebAssembly instead.
//!
//! There is exactly one implementation of the format and the formula engine, and
//! this is it. Nothing here touches the filesystem; that stays on the native
//! side, behind `ai-core`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use wasm_bindgen::prelude::*;

use ai_format::model::{Section, Sheet, Slide};
use ai_formula::evaluate::{Cell, Names};

/// Turn a Rust panic into a readable console message instead of `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Serialize for the editors.
///
/// `serde_wasm_bindgen`'s default turns every map into a JS `Map`, but the whole
/// document model is addressed as plain objects — `sheet.cells.A1`, `block.style`
/// — and a `Map` would silently read as empty everywhere.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
    value
        .serialize(&serializer)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

fn from_js<T: for<'de> Deserialize<'de>>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|e| JsValue::from_str(&e.to_string()))
}

/* ------------------------------------------------------------------- refs */

#[wasm_bindgen(js_name = indexToCol)]
pub fn index_to_col(index: usize) -> String {
    ai_formula::refs::index_to_col(index)
}

#[wasm_bindgen(js_name = colToIndex)]
pub fn col_to_index(col: &str) -> i32 {
    ai_formula::refs::col_to_index(col)
        .map(|c| c as i32)
        .unwrap_or(-1)
}

#[wasm_bindgen(js_name = toRef)]
pub fn to_ref(col: usize, row: usize) -> String {
    ai_formula::refs::to_ref(col, row)
}

/// `{ col, row }`, or `null` if the text is not a reference.
#[wasm_bindgen(js_name = parseRef)]
pub fn parse_ref(reference: &str) -> Result<JsValue, JsValue> {
    match ai_formula::refs::parse_ref(reference) {
        None => Ok(JsValue::NULL),
        Some(r) => to_js(&serde_json::json!({ "col": r.col, "row": r.row })),
    }
}

/// `{ start: {col,row}, end: {col,row} }`, normalized, or `null`.
#[wasm_bindgen(js_name = parseRange)]
pub fn parse_range(range: &str) -> Result<JsValue, JsValue> {
    match ai_formula::refs::parse_range(range) {
        None => Ok(JsValue::NULL),
        Some(r) => to_js(&serde_json::json!({
            "start": { "col": r.start.col, "row": r.start.row },
            "end": { "col": r.end.col, "row": r.end.row },
        })),
    }
}

#[wasm_bindgen(js_name = expandRange)]
pub fn expand_range(range: &str) -> Vec<String> {
    ai_formula::refs::expand_range_str(range)
}

// Offsets cross the boundary as `i32`: `i64` would arrive as a `BigInt` and the
// editors pass ordinary numbers. A sheet is at most 100,000 rows, so the narrower
// type loses nothing.

#[wasm_bindgen(js_name = shiftRef)]
pub fn shift_ref(reference: &str, dc: i32, dr: i32) -> String {
    ai_formula::refs::shift_ref(reference, dc as i64, dr as i64)
}

#[wasm_bindgen(js_name = shiftFormula)]
pub fn shift_formula(formula: &str, dc: i32, dr: i32) -> String {
    ai_formula::refs::shift_formula(formula, dc as i64, dr as i64)
}

/// `axis` is `"row"` or `"col"`.
#[wasm_bindgen(js_name = adjustRefs)]
pub fn adjust_refs(formula: &str, axis: &str, at: i32, delta: i32) -> String {
    let axis = if axis == "col" {
        ai_formula::refs::Axis::Col
    } else {
        ai_formula::refs::Axis::Row
    };
    ai_formula::refs::adjust_refs(formula, axis, at as i64, delta as i64)
}

/* ---------------------------------------------------------------- formulas */

#[wasm_bindgen(js_name = recalcSheet)]
pub fn recalc_sheet(sheet: JsValue) -> Result<JsValue, JsValue> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Input {
        cells: IndexMap<String, Cell>,
        names: Names,
    }
    #[derive(Serialize)]
    struct Output {
        cells: IndexMap<String, Cell>,
        changed: Vec<String>,
        errors: IndexMap<String, String>,
    }

    let input: Input = from_js(sheet)?;
    let out = ai_formula::evaluate::recalc_sheet(&input.cells, &input.names);
    to_js(&Output {
        cells: out.cells,
        changed: out.changed,
        errors: out.errors,
    })
}

/// `null` clears the cell.
#[wasm_bindgen(js_name = parseCellInput)]
pub fn parse_cell_input(raw: &str) -> Result<JsValue, JsValue> {
    match ai_formula::evaluate::parse_cell_input(raw) {
        None => Ok(JsValue::NULL),
        Some(cell) => to_js(&cell),
    }
}

#[wasm_bindgen(js_name = displayValue)]
pub fn display_value(cell: JsValue) -> Result<String, JsValue> {
    if cell.is_null() || cell.is_undefined() {
        return Ok(String::new());
    }
    let cell: Cell = from_js(cell)?;
    Ok(ai_formula::evaluate::display_value(&cell))
}

#[wasm_bindgen(js_name = editValue)]
pub fn edit_value(cell: JsValue) -> Result<String, JsValue> {
    if cell.is_null() || cell.is_undefined() {
        return Ok(String::new());
    }
    let cell: Cell = from_js(cell)?;
    Ok(ai_formula::evaluate::edit_value(&cell))
}

#[wasm_bindgen(js_name = dependencies)]
pub fn dependencies(formula: &str) -> Vec<String> {
    ai_formula::evaluate::dependencies(formula)
}

#[wasm_bindgen(js_name = applyNumFmt)]
pub fn apply_num_fmt(value: JsValue, fmt: &str) -> Result<String, JsValue> {
    let json: Json = from_js(value)?;
    Ok(ai_formula::numfmt::apply_num_fmt(
        &ai_formula::evaluate::json_to_value(&json),
        fmt,
    ))
}

/// Function names for the formula bar's autocomplete.
#[wasm_bindgen(js_name = functionNames)]
pub fn function_names() -> Vec<String> {
    ai_formula::functions::FUNCTION_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/* ---------------------------------------------------------------- geometry */

#[wasm_bindgen(js_name = positionPhrase)]
pub fn position_phrase(box_: JsValue, canvas: JsValue) -> Result<String, JsValue> {
    #[derive(Deserialize)]
    struct B {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default)]
        w: f64,
        #[serde(default)]
        h: f64,
    }
    let b: B = from_js(box_)?;
    let canvas: ai_format::geometry::Canvas = if canvas.is_null() || canvas.is_undefined() {
        ai_format::geometry::DEFAULT_CANVAS
    } else {
        from_js(canvas)?
    };
    Ok(ai_format::geometry::position_phrase(
        &ai_format::geometry::Box {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
            z: 0.0,
        },
        &canvas,
    ))
}

#[wasm_bindgen(js_name = autoLayout)]
pub fn auto_layout(index: usize, total: usize, canvas: JsValue) -> Result<JsValue, JsValue> {
    let canvas: ai_format::geometry::Canvas = if canvas.is_null() || canvas.is_undefined() {
        ai_format::geometry::DEFAULT_CANVAS
    } else {
        from_js(canvas)?
    };
    let b = ai_format::geometry::auto_layout(index, total, &canvas);
    to_js(&serde_json::json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h, "z": b.z }))
}

#[wasm_bindgen(js_name = clampBox)]
pub fn clamp_box(box_: JsValue, canvas: JsValue) -> Result<JsValue, JsValue> {
    #[derive(Deserialize)]
    struct B {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default)]
        w: f64,
        #[serde(default)]
        h: f64,
        #[serde(default)]
        z: f64,
    }
    let b: B = from_js(box_)?;
    let canvas: ai_format::geometry::Canvas = if canvas.is_null() || canvas.is_undefined() {
        ai_format::geometry::DEFAULT_CANVAS
    } else {
        from_js(canvas)?
    };
    let out = ai_format::geometry::clamp_box(
        ai_format::geometry::Box {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
            z: b.z,
        },
        &canvas,
    );
    to_js(&serde_json::json!({ "x": out.x, "y": out.y, "w": out.w, "h": out.h, "z": out.z }))
}

/* --------------------------------------------------------------- markdown */

#[wasm_bindgen(js_name = headingLevel)]
pub fn heading_level(md: &str) -> usize {
    ai_format::mdblocks::heading_level(md)
}

#[wasm_bindgen(js_name = plainText)]
pub fn plain_text(md: &str) -> String {
    ai_format::mdblocks::plain_text(md)
}

#[wasm_bindgen(js_name = countWords)]
pub fn count_words(text: &str) -> usize {
    ai_format::mdblocks::count_words(text)
}

#[wasm_bindgen(js_name = inferKind)]
pub fn infer_kind(md: &str) -> String {
    ai_format::blocks::infer_kind(md).as_str().to_string()
}

#[wasm_bindgen(js_name = blockLabel)]
pub fn block_label(md: &str, max: Option<usize>) -> String {
    ai_format::blocks::block_label(md, max.unwrap_or(80))
}

#[wasm_bindgen(js_name = splitMarkdownBlocks)]
pub fn split_markdown_blocks(md: &str) -> Result<JsValue, JsValue> {
    let blocks: Vec<Json> = ai_format::mdblocks::split_markdown_blocks(md)
        .into_iter()
        .map(|b| serde_json::json!({ "md": b.md, "type": b.block_type.as_str() }))
        .collect();
    to_js(&blocks)
}

/* ------------------------------------------------------------------ charts */

#[wasm_bindgen(js_name = normalizeChartSpec)]
pub fn normalize_chart_spec(spec: JsValue) -> Result<JsValue, JsValue> {
    let raw: Json = if spec.is_null() || spec.is_undefined() {
        Json::Null
    } else {
        from_js(spec)?
    };
    to_js(&ai_format::chart::normalize_spec(&raw))
}

#[wasm_bindgen(js_name = parseChartBlock)]
pub fn parse_chart_block(md: &str) -> Result<JsValue, JsValue> {
    match ai_format::chart::parse_chart_block(md) {
        None => Ok(JsValue::NULL),
        Some(spec) => to_js(&spec),
    }
}

#[wasm_bindgen(js_name = serializeChartBlock)]
pub fn serialize_chart_block(spec: JsValue) -> Result<String, JsValue> {
    let raw: Json = from_js(spec)?;
    Ok(ai_format::chart::serialize_chart_block(
        &ai_format::chart::normalize_spec(&raw),
    ))
}

/// Resolve a chart's data against a sheet, so a range-backed chart shows the
/// current numbers. `sheet` may be `null` for a chart carrying its own data.
#[wasm_bindgen(js_name = resolveChartSpec)]
pub fn resolve_chart_spec(spec: JsValue, sheet: JsValue) -> Result<JsValue, JsValue> {
    let raw: Json = from_js(spec)?;
    let normalized = ai_format::chart::normalize_spec(&raw);
    if sheet.is_null() || sheet.is_undefined() {
        return to_js(&normalized);
    }
    let sheet: Sheet = from_js(sheet)?;
    to_js(&ai_format::chart::resolve_spec(&normalized, Some(&sheet)))
}

#[wasm_bindgen(js_name = chartToMarkdownTable)]
pub fn chart_to_markdown_table(spec: JsValue) -> Result<String, JsValue> {
    let raw: Json = from_js(spec)?;
    Ok(ai_format::chart::chart_to_markdown_table(
        &ai_format::chart::normalize_spec(&raw),
    ))
}

#[wasm_bindgen(js_name = describeChart)]
pub fn describe_chart(spec: JsValue) -> Result<String, JsValue> {
    let raw: Json = from_js(spec)?;
    Ok(ai_format::chart::describe_chart(
        &ai_format::chart::normalize_spec(&raw),
    ))
}

#[wasm_bindgen(js_name = chartTypes)]
pub fn chart_types() -> Result<JsValue, JsValue> {
    let types: Vec<Json> = ai_format::chart::CHART_TYPES
        .iter()
        .map(|t| {
            let parsed = ai_format::chart::normalize_spec(&serde_json::json!({ "type": t }));
            serde_json::json!({ "type": t, "label": parsed.chart_type.label() })
        })
        .collect();
    to_js(&types)
}

#[wasm_bindgen(js_name = chartPalette)]
pub fn chart_palette() -> Result<JsValue, JsValue> {
    to_js(&serde_json::json!({
        "light": ai_format::chart::PALETTE_LIGHT,
        "dark": ai_format::chart::PALETTE_DARK,
        "maxSeries": ai_format::chart::MAX_SERIES,
    }))
}

/* ------------------------------------------------------------ items & save */

#[wasm_bindgen(js_name = makeSlide)]
pub fn make_slide(
    layout: &str,
    title: Option<String>,
    index: Option<usize>,
) -> Result<JsValue, JsValue> {
    to_js(&ai_format::deck::make_slide(
        layout,
        title.as_deref(),
        index.unwrap_or(1),
    ))
}

#[wasm_bindgen(js_name = makeSection)]
pub fn make_section(name: Option<String>, heading: Option<bool>) -> Result<JsValue, JsValue> {
    to_js(&ai_format::doc::make_section(
        name.as_deref().unwrap_or("새 섹션"),
        heading.unwrap_or(true),
    ))
}

#[wasm_bindgen(js_name = makeSheet)]
pub fn make_sheet(name: Option<String>, with_sample: Option<bool>) -> Result<JsValue, JsValue> {
    to_js(&ai_format::grid::make_sheet(
        name.as_deref().unwrap_or("시트1"),
        with_sample.unwrap_or(false),
    ))
}

#[wasm_bindgen(js_name = slideLayouts)]
pub fn slide_layouts() -> Vec<String> {
    ai_format::model::SLIDE_LAYOUTS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[wasm_bindgen(js_name = pageSizes)]
pub fn page_sizes() -> Result<JsValue, JsValue> {
    let sizes: Json = ["A4", "Letter", "A5"]
        .iter()
        .map(|name| {
            let page = ai_format::model::Page {
                size: name.to_string(),
                ..ai_format::model::Page::default()
            };
            let (w, h) = page.dimensions();
            (name.to_string(), serde_json::json!({ "w": w, "h": h }))
        })
        .collect::<serde_json::Map<String, Json>>()
        .into();
    to_js(&sizes)
}

#[wasm_bindgen(js_name = gridDefaults)]
pub fn grid_defaults() -> Result<JsValue, JsValue> {
    to_js(&serde_json::json!({
        "colWidth": ai_format::grid::DEFAULT_COL_WIDTH,
        "rowHeight": ai_format::grid::DEFAULT_ROW_HEIGHT,
        "dims": ai_format::model::Dims::default(),
    }))
}

#[wasm_bindgen(js_name = usedRange)]
pub fn used_range(cells: JsValue) -> Result<JsValue, JsValue> {
    let cells: IndexMap<String, Cell> = from_js(cells)?;
    match ai_format::grid::used_range(&cells) {
        None => Ok(JsValue::NULL),
        Some(r) => to_js(&serde_json::json!({
            "minCol": 0, "minRow": 0,
            "maxCol": r.max_col, "maxRow": r.max_row,
            "firstCol": r.first_col, "firstRow": r.first_row,
        })),
    }
}

#[wasm_bindgen(js_name = newBlockId)]
pub fn new_block_id() -> String {
    ai_format::ids::new_block_id()
}

#[wasm_bindgen(js_name = slugify)]
pub fn slugify(title: &str) -> String {
    ai_format::ids::slugify(title, "untitled")
}

/// The md/json pair a slide would be written as.
#[wasm_bindgen(js_name = writeSlide)]
pub fn write_slide(slide: JsValue) -> Result<JsValue, JsValue> {
    let slide: Slide = from_js(slide)?;
    let out = ai_format::deck::write_slide(&slide);
    to_js(&serde_json::json!({
        "md": out.md,
        "json": format!("{}\n", ai_format::json::to_pretty(&out.layout)),
    }))
}

#[wasm_bindgen(js_name = writeSection)]
pub fn write_section(section: JsValue) -> Result<JsValue, JsValue> {
    let section: Section = from_js(section)?;
    let out = ai_format::doc::write_section(&section);
    to_js(&serde_json::json!({
        "md": out.md,
        "json": format!("{}\n", ai_format::json::to_pretty(&out.meta)),
    }))
}

#[wasm_bindgen(js_name = writeSheet)]
pub fn write_sheet(sheet: JsValue) -> Result<JsValue, JsValue> {
    let sheet: Sheet = from_js(sheet)?;
    let out = ai_format::grid::write_sheet(&sheet);
    to_js(&serde_json::json!({
        "md": out.md,
        "json": format!("{}\n", ai_format::json::to_pretty(&out.cells)),
    }))
}

/// `AI.md` for a project payload, as the save would write it.
#[wasm_bindgen(js_name = buildDigest)]
pub fn build_digest(project: JsValue) -> Result<String, JsValue> {
    let payload: Json = from_js(project)?;
    let project = project_from_json(&payload)?;
    Ok(ai_format::digest::build_digest(&project))
}

fn project_from_json(payload: &Json) -> Result<ai_format::model::Project, JsValue> {
    use ai_format::model::{Items, Manifest, Project, ProjectType, Theme};

    let type_name = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let Some(project_type) = ProjectType::from_name(type_name) else {
        return Err(JsValue::from_str(&format!(
            "알 수 없는 문서 종류: {type_name}"
        )));
    };
    let manifest = payload.get("manifest").cloned().unwrap_or(Json::Null);
    let get = |key: &str| {
        manifest
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };

    let items = match project_type {
        ProjectType::Deck => Items::Slides(read_items(payload, "slides")?),
        ProjectType::Doc => Items::Sections(read_items(payload, "sections")?),
        ProjectType::Grid => Items::Sheets(read_items(payload, "sheets")?),
    };

    Ok(Project {
        project_type,
        dir: std::path::PathBuf::new(),
        manifest: Manifest {
            format: project_type.format_id().to_string(),
            format_version: 1,
            id: get("id"),
            title: get("title"),
            created: get("created"),
            modified: get("modified"),
            theme: manifest
                .get("theme")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_else(Theme::default),
            entries: Vec::new(),
        },
        items,
    })
}

fn read_items<T: for<'de> Deserialize<'de>>(payload: &Json, key: &str) -> Result<Vec<T>, JsValue> {
    match payload.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| JsValue::from_str(&format!("{key}: {e}"))),
    }
}
