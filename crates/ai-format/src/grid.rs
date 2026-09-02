//! Grid: the cells JSON is the source, the markdown is an AI-readable projection.

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value as Json};

use ai_formula::evaluate::{display_value, parse_cell_input, recalc_sheet, Book, Cell, Names};
use ai_formula::jsnum;
use ai_formula::refs::{index_to_col, parse_ref, to_ref};

use crate::chart::{
    chart_to_markdown_table, describe_chart, normalize_spec, resolve_spec, CellSource,
};
use crate::frontmatter::{meta_str, parse_frontmatter, serialize_frontmatter, Meta};
use crate::ids::{new_block_id, new_sheet_id};
use crate::model::{Dims, Frozen, Sheet, SheetChart};

/// The size a cell's text is drawn at when it says nothing, in px.
///
/// Excel's own default is 11pt, which is 14.67px at 96dpi. Import compares
/// against this, and the stylesheet draws it, so a sheet that says 11pt carries
/// no per-cell size at all.
pub const CELL_PX: f64 = 15.0;

pub const DEFAULT_COL_WIDTH: i64 = 104;
pub const DEFAULT_ROW_HEIGHT: i64 = 28;

/// How many rows the markdown projection will render before truncating.
const MD_ROW_LIMIT: usize = 500;

/// Merge a sheet's cells JSON with its markdown projection.
///
/// The JSON is authoritative — it holds formulas, formats and styles. The
/// markdown is a generated projection. The one exception: when cells JSON is
/// absent (a hand-written markdown table dropped into the project), we import
/// the table so the sheet still opens with data.
pub fn read_sheet(md: &str, cells_json: Option<&Json>) -> Sheet {
    let split = parse_frontmatter(md);
    let meta_id = meta_str(&split.meta, "id");
    let meta_name = meta_str(&split.meta, "name");

    let has_cells = cells_json
        .and_then(|c| c.get("cells"))
        .and_then(|c| c.as_object())
        .is_some_and(|o| !o.is_empty());

    if has_cells {
        let mut parsed: Sheet =
            serde_json::from_value(cells_json.unwrap().clone()).unwrap_or_else(|_| empty_sheet());
        if parsed.id.is_empty() {
            parsed.id = meta_id.unwrap_or_else(new_sheet_id);
        }
        if parsed.name.is_empty() {
            parsed.name = meta_name.unwrap_or_else(|| "시트1".to_string());
        }
        return normalize_sheet(&parsed);
    }

    let imported = import_markdown_table(md);
    let base = cells_json;
    let mut sheet = Sheet {
        id: base
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(meta_id)
            .unwrap_or_else(new_sheet_id),
        name: base
            .and_then(|c| c.get("name"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(meta_name)
            .unwrap_or_else(|| "시트1".to_string()),
        dims: read_or_default(base.and_then(|c| c.get("dims"))),
        frozen: read_or_default(base.and_then(|c| c.get("frozen"))),
        col_widths: read_or_default(base.and_then(|c| c.get("colWidths"))),
        row_heights: read_or_default(base.and_then(|c| c.get("rowHeights"))),
        merges: read_or_default(base.and_then(|c| c.get("merges"))),
        names: read_or_default(base.and_then(|c| c.get("names"))),
        charts: Vec::new(),
        cells: imported,
        file: None,
    };
    if let Some(charts) = base.and_then(|c| c.get("charts")) {
        sheet.charts = read_charts(charts);
    }
    normalize_sheet(&sheet)
}

fn read_or_default<T: Default + serde::de::DeserializeOwned>(v: Option<&Json>) -> T {
    v.and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn empty_sheet() -> Sheet {
    Sheet {
        id: new_sheet_id(),
        name: "시트1".into(),
        dims: Dims::default(),
        frozen: Frozen::default(),
        col_widths: IndexMap::new(),
        row_heights: IndexMap::new(),
        merges: Vec::new(),
        names: Names::new(),
        charts: Vec::new(),
        cells: IndexMap::new(),
        file: None,
    }
}

fn read_charts(value: &Json) -> Vec<SheetChart> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .take(24)
        .enumerate()
        .map(|(i, chart)| SheetChart {
            id: chart
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(new_block_id),
            x: clamp_int(chart.get("x"), 80.0 + i as f64 * 24.0, 0.0, 20000.0),
            y: clamp_int(chart.get("y"), 80.0 + i as f64 * 24.0, 0.0, 40000.0),
            w: clamp_int(chart.get("w"), 460.0, 160.0, 2400.0),
            h: clamp_int(chart.get("h"), 300.0, 120.0, 1600.0),
            spec: normalize_spec(chart.get("spec").unwrap_or(&Json::Null)),
        })
        .collect()
}

pub fn normalize_sheet(sheet: &Sheet) -> Sheet {
    let mut cells: IndexMap<String, Cell> = IndexMap::new();
    for (reference, cell) in &sheet.cells {
        let Some(parsed) = parse_ref(reference) else {
            continue;
        };
        let key = to_ref(parsed.col, parsed.row);

        let mut next = Cell::default();
        if let Some(f) = cell.formula() {
            next.f = Some(f.to_string());
        }
        next.v = cell.v.clone();
        if let Some(t) = &cell.t {
            if !t.is_empty() {
                next.t = Some(t.clone());
            }
        }
        if let Some(fmt) = &cell.fmt {
            if !fmt.is_empty() {
                next.fmt = Some(fmt.clone());
            }
        }
        for key in ["style", "note"] {
            match cell.extra.get(key) {
                Some(Json::Object(o)) if o.is_empty() => {}
                Some(Json::Null) | None => {}
                Some(v) => {
                    next.extra.insert(key.to_string(), v.clone());
                }
            }
        }

        let carries_something =
            next.f.is_some() || !next.v.is_null() || next.fmt.is_some() || !next.extra.is_empty();
        if carries_something {
            cells.insert(key, next);
        }
    }

    Sheet {
        id: if sheet.id.is_empty() {
            new_sheet_id()
        } else {
            sheet.id.clone()
        },
        name: if sheet.name.is_empty() {
            "시트1".into()
        } else {
            sheet.name.clone()
        },
        dims: Dims {
            rows: clamp_int(Some(&json!(sheet.dims.rows)), 200.0, 1.0, 100000.0) as u32,
            cols: clamp_int(Some(&json!(sheet.dims.cols)), 26.0, 1.0, 702.0) as u32,
        },
        frozen: Frozen {
            rows: clamp_int(Some(&json!(sheet.frozen.rows)), 1.0, 0.0, 20.0) as u32,
            cols: clamp_int(Some(&json!(sheet.frozen.cols)), 0.0, 0.0, 20.0) as u32,
        },
        col_widths: positive_ints(&sheet.col_widths),
        row_heights: positive_ints(&sheet.row_heights),
        merges: sheet.merges.clone(),
        names: sheet.names.clone(),
        charts: sheet.charts.clone(),
        cells,
        file: sheet.file.clone(),
    }
}

fn clamp_int(v: Option<&Json>, fallback: f64, min: f64, max: f64) -> f64 {
    let n = match v {
        Some(Json::Number(n)) => n.as_f64(),
        Some(Json::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    match n {
        Some(n) if n.is_finite() => crate::geometry::js_round(n).clamp(min, max),
        _ => fallback,
    }
}

fn positive_ints(map: &IndexMap<String, i64>) -> IndexMap<String, i64> {
    map.iter()
        .filter(|(_, v)| **v > 0)
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

/// The smallest rectangle containing every non-empty cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsedRange {
    pub max_col: usize,
    pub max_row: usize,
    pub first_col: usize,
    pub first_row: usize,
}

pub fn used_range(cells: &IndexMap<String, Cell>) -> Option<UsedRange> {
    let mut min_col = usize::MAX;
    let mut min_row = usize::MAX;
    let mut max_col: i64 = -1;
    let mut max_row: i64 = -1;

    for (reference, cell) in cells {
        let Some(p) = parse_ref(reference) else {
            continue;
        };
        let empty = cell.v.is_null() && cell.f.is_none() && !cell.extra.contains_key("style");
        if empty {
            continue;
        }
        min_col = min_col.min(p.col);
        min_row = min_row.min(p.row);
        max_col = max_col.max(p.col as i64);
        max_row = max_row.max(p.row as i64);
    }
    if max_col < 0 {
        return None;
    }
    Some(UsedRange {
        max_col: max_col as usize,
        max_row: max_row as usize,
        first_col: min_col,
        first_row: min_row,
    })
}

/// Recalculate a sheet's formulas and return the sheet with values filled in.
pub fn recalculated(sheet: &Sheet) -> Sheet {
    let out = recalc_sheet(&sheet.cells, &sheet.names);
    Sheet {
        cells: out.cells,
        ..sheet.clone()
    }
}

/// The same, with the workbook's other sheets available to cross-sheet formulas.
///
/// `=요약!B4` is most of what a summary sheet contains, and a reader that
/// recalculates one sheet in isolation cannot resolve it.
pub fn recalculated_in(sheet: &Sheet, book: &Book<'_>) -> Sheet {
    let out = ai_formula::evaluate::recalc_sheet_in(&sheet.cells, &sheet.names, &sheet.name, book);
    Sheet {
        cells: out.cells,
        ..sheet.clone()
    }
}

/// Every sheet in a workbook, addressable by name.
pub fn book(sheets: &[Sheet]) -> Book<'_> {
    ai_formula::evaluate::book_of(sheets.iter().map(|s| (s.name.as_str(), &s.cells)))
}

/// Recalculate a whole workbook, each sheet seeing the others.
pub fn recalculated_all(sheets: &[Sheet]) -> Vec<Sheet> {
    let book = book(sheets);
    sheets.iter().map(|s| recalculated_in(s, &book)).collect()
}

pub struct SheetFiles {
    pub md: String,
    pub cells: Json,
}

/// Split a sheet into the cells JSON (source) + markdown projection pair.
pub fn write_sheet(sheet: &Sheet) -> SheetFiles {
    static EMPTY: std::sync::LazyLock<Book<'static>> = std::sync::LazyLock::new(Book::new);
    write_sheet_in(sheet, &EMPTY)
}

/// The same, with the workbook's other sheets available, so the markdown
/// projection of a cross-sheet formula carries its real value.
pub fn write_sheet_in(sheet: &Sheet, book: &Book<'_>) -> SheetFiles {
    let s = normalize_sheet(sheet);
    let with_values = recalculated_in(&s, book);

    let mut cells = serde_json::Map::new();
    cells.insert("id".into(), json!(s.id));
    cells.insert("name".into(), json!(s.name));
    cells.insert("dims".into(), json!(s.dims));
    cells.insert("frozen".into(), json!(s.frozen));
    cells.insert("colWidths".into(), json!(s.col_widths));
    cells.insert("rowHeights".into(), json!(s.row_heights));
    cells.insert("merges".into(), json!(s.merges));
    cells.insert("names".into(), json!(s.names));
    if !s.charts.is_empty() {
        cells.insert("charts".into(), json!(s.charts));
    }
    cells.insert("cells".into(), json!(strip_empty(&with_values.cells)));

    SheetFiles {
        md: render_sheet_markdown(&with_values),
        cells: Json::Object(cells),
    }
}

fn strip_empty(cells: &IndexMap<String, Cell>) -> IndexMap<String, Cell> {
    cells
        .iter()
        .filter(|(_, c)| {
            let has_style = matches!(c.extra.get("style"), Some(Json::Object(o)) if !o.is_empty());
            c.f.is_some()
                || !c.v.is_null()
                || c.fmt.is_some()
                || has_style
                || c.extra.contains_key("note")
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Render the AI-readable markdown projection of a sheet.
///
/// Column letters and row numbers are included deliberately: they let a model
/// reason in spreadsheet terms ("the total in D4") instead of guessing from a
/// headerless table. Formulas get their own section with both the expression and
/// the computed value, so a RAG chunk carries the intent *and* the answer.
pub fn render_sheet_markdown(sheet: &Sheet) -> String {
    let range = used_range(&sheet.cells);
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("## {}", sheet.name));
    lines.push(String::new());

    match range {
        None => lines.push("_빈 시트_".to_string()),
        Some(r) => {
            let rows = (r.max_row + 1).min(MD_ROW_LIMIT);
            let cols = r.max_col + 1;

            let mut header = vec!["   ".to_string()];
            header.extend((0..cols).map(index_to_col));
            lines.push(format!("| {} |", header.join(" | ")));
            lines.push(format!("|{}|", vec!["---"; header.len()].join("|")));

            for row in 0..rows {
                let in_row: Vec<String> = (0..cols)
                    .map(|col| md_cell(sheet.cells.get(&to_ref(col, row))))
                    .collect();
                lines.push(format!("| **{}** | {} |", row + 1, in_row.join(" | ")));
            }

            if r.max_row + 1 > MD_ROW_LIMIT {
                lines.push(String::new());
                lines.push(format!(
                    "_… {}개 행 생략 (전체 데이터는 `{}.cells.json` 참조)_",
                    r.max_row + 1 - MD_ROW_LIMIT,
                    sheet.name
                ));
            }
        }
    }

    let mut formulas: Vec<(&String, &Cell)> =
        sheet.cells.iter().filter(|(_, c)| c.f.is_some()).collect();
    formulas.sort_by_key(|(reference, _)| ref_order(reference));

    if !formulas.is_empty() {
        lines.push(String::new());
        lines.push("### 수식".to_string());
        lines.push(String::new());
        for (reference, cell) in &formulas {
            let shown = display_value(cell);
            let shown = if shown.is_empty() {
                "(빈 값)".to_string()
            } else {
                shown
            };
            lines.push(format!(
                "- `{reference}` = `{}` → {shown}",
                cell.f.as_deref().unwrap_or("")
            ));
        }
    }

    if !sheet.names.is_empty() {
        lines.push(String::new());
        lines.push("### 이름 있는 범위".to_string());
        lines.push(String::new());
        for (name, target) in &sheet.names {
            lines.push(format!("- `{name}` → `{}`", name_target(target)));
        }
    }

    if !sheet.charts.is_empty() {
        lines.push(String::new());
        lines.push("### 차트".to_string());
        lines.push(String::new());
        for chart in &sheet.charts {
            let resolved = resolve_spec(&chart.spec, Some(sheet as &dyn CellSource));
            lines.push(format!("- {}", describe_chart(&resolved)));
            lines.push(String::new());
            // The chart's numbers, not a picture of them: this is what a model reads.
            for row in chart_to_markdown_table(&resolved).lines() {
                lines.push(format!("  {row}"));
            }
            lines.push(String::new());
        }
    }

    let summary = summarize_columns(sheet);
    if !summary.is_empty() {
        lines.push(String::new());
        lines.push("### 열 요약".to_string());
        lines.push(String::new());
        if let Some(scope) = summary_scope(sheet) {
            let excluded = if scope.excluded.is_empty() {
                String::new()
            } else {
                let rows: Vec<String> = scope.excluded.iter().map(|r| format!("{r}행")).collect();
                format!(" (집계 행 {} 제외)", rows.join(", "))
            };
            lines.push(format!(
                "집계 대상: {}–{}행{excluded}",
                scope.first_row, scope.last_row
            ));
            lines.push(String::new());
        }
        lines.push("| 열 | 머리글 | 유형 | 값 개수 | 합계 | 평균 | 최소 | 최대 |".to_string());
        lines.push("|---|---|---|---|---|---|---|---|".to_string());
        for col in &summary {
            lines.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |",
                col.col,
                escape_pipes(&col.header),
                col.col_type,
                col.count,
                col.sum.as_deref().unwrap_or("—"),
                col.avg.as_deref().unwrap_or("—"),
                col.min.as_deref().unwrap_or("—"),
                col.max.as_deref().unwrap_or("—"),
            ));
        }
    }

    let mut meta = Meta::new();
    meta.insert("id".into(), json!(sheet.id));
    meta.insert("name".into(), json!(sheet.name));
    meta.insert(
        "rows".into(),
        json!(range.map(|r| r.max_row + 1).unwrap_or(0)),
    );
    meta.insert(
        "cols".into(),
        json!(range.map(|r| r.max_col + 1).unwrap_or(0)),
    );
    meta.insert("formulas".into(), json!(formulas.len()));
    serialize_frontmatter(&meta, &lines.join("\n"))
}

fn name_target(target: &Json) -> String {
    match target {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn md_cell(cell: Option<&Cell>) -> String {
    let Some(cell) = cell else {
        return String::new();
    };
    let text = escape_pipes(&display_value(cell));
    if text.is_empty() {
        return String::new();
    }
    let bold = cell
        .extra
        .get("style")
        .and_then(|s| s.get("bold"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if bold {
        format!("**{text}**")
    } else {
        text
    }
}

fn escape_pipes(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn ref_order(reference: &str) -> usize {
    parse_ref(reference)
        .map(|p| p.row * 1000 + p.col)
        .unwrap_or(0)
}

/// Where the leading table ends: the row before the first completely blank row.
///
/// Sheets routinely put unrelated blocks below a blank row (an analysis section,
/// a second table). Summing straight down a column across that boundary produces
/// numbers that match nothing in the sheet, which is worse than no summary at all
/// for a model reading the digest.
pub fn table_extent(cells: &IndexMap<String, Cell>, max_row: usize, max_col: usize) -> usize {
    for row in 1..=max_row {
        let blank = (0..=max_col).all(|col| {
            cells
                .get(&to_ref(col, row))
                .map(|c| c.v.is_null())
                .unwrap_or(true)
        });
        if blank {
            return row - 1;
        }
    }
    max_row
}

static TOTAL_LABEL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*(합계|총계|소계|누계|계|total|subtotal|sum)\s*$").unwrap());

/// A row that aggregates the rows above it, which must be excluded from column
/// sums or every total gets counted twice.
///
/// Detected two ways: by its label, and structurally by a cell whose formula
/// aggregates a range inside its own column — the latter catches unlabelled
/// total rows in any language.
fn is_aggregate_row(cells: &IndexMap<String, Cell>, row: usize, max_col: usize) -> bool {
    let label = cells
        .get(&to_ref(0, row))
        .map(display_value)
        .unwrap_or_default();
    if TOTAL_LABEL.is_match(&label) {
        return true;
    }

    for col in 0..=max_col {
        let Some(cell) = cells.get(&to_ref(col, row)) else {
            continue;
        };
        let Some(formula) = cell.f.as_deref() else {
            continue;
        };
        if aggregates_own_column(formula, &index_to_col(col)) {
            return true;
        }
    }
    false
}

static AGG_CALL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(SUM|AVERAGE|COUNT|MAX|MIN)\(\$?([A-Za-z]{1,3})\$?\d+:\$?([A-Za-z]{1,3})\$?\d+\)",
    )
    .unwrap()
});

fn aggregates_own_column(formula: &str, col: &str) -> bool {
    AGG_CALL.captures_iter(formula).any(|c| {
        c.get(2).unwrap().as_str().eq_ignore_ascii_case(col)
            && c.get(3).unwrap().as_str().eq_ignore_ascii_case(col)
    })
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ColumnSummary {
    pub col: String,
    pub header: String,
    #[serde(rename = "type")]
    pub col_type: String,
    pub count: usize,
    pub sum: Option<String>,
    pub avg: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
}

/// Per-column statistics appended to the markdown.
///
/// A model asking "what is in column C?" gets an answer without scanning rows —
/// and the aggregates are restricted to the leading table's data rows so they
/// reconcile with the sheet's own total row instead of contradicting it.
pub fn summarize_columns(sheet: &Sheet) -> Vec<ColumnSummary> {
    let Some(range) = used_range(&sheet.cells) else {
        return Vec::new();
    };
    let last_row = table_extent(&sheet.cells, range.max_row, range.max_col);
    let data_rows: Vec<usize> = (1..=last_row)
        .filter(|r| !is_aggregate_row(&sheet.cells, *r, range.max_col))
        .collect();

    let mut out = Vec::new();
    for col in 0..=range.max_col {
        let header = sheet
            .cells
            .get(&to_ref(col, 0))
            .map(display_value)
            .unwrap_or_default();
        let mut count = 0usize;
        let mut numeric = 0usize;
        let mut sum = 0.0f64;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;

        for row in &data_rows {
            let Some(cell) = sheet.cells.get(&to_ref(col, *row)) else {
                continue;
            };
            if cell.v.is_null() {
                continue;
            }
            count += 1;
            let is_plain_number = cell.v.is_number()
                && cell.t.as_deref() != Some("d")
                && cell.t.as_deref() != Some("e");
            if is_plain_number {
                let v = cell.v.as_f64().unwrap_or(0.0);
                numeric += 1;
                sum += v;
                min = min.min(v);
                max = max.max(v);
            }
        }
        if count == 0 && header.is_empty() {
            continue;
        }

        let is_numeric = count > 0 && numeric as f64 / count as f64 >= 0.6;
        let fmt = data_rows
            .first()
            .copied()
            .or(Some(1))
            .and_then(|r| sheet.cells.get(&to_ref(col, r)))
            .and_then(|c| c.fmt.clone());
        let stat = |v: f64| format_stat(v, fmt.as_deref());

        out.push(ColumnSummary {
            col: index_to_col(col),
            header: if header.is_empty() {
                "(머리글 없음)".to_string()
            } else {
                header
            },
            col_type: if is_numeric {
                "숫자".to_string()
            } else if count > 0 {
                "텍스트".to_string()
            } else {
                "빈 열".to_string()
            },
            count,
            sum: (is_numeric).then(|| stat(sum)),
            avg: (is_numeric && numeric > 0).then(|| stat(sum / numeric as f64)),
            min: (is_numeric && numeric > 0).then(|| stat(min)),
            max: (is_numeric && numeric > 0).then(|| stat(max)),
        });
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct SummaryScope {
    pub first_row: usize,
    pub last_row: usize,
    /// One-based row numbers excluded as aggregates.
    pub excluded: Vec<usize>,
    pub truncated: bool,
}

/// Rows the summary is based on, so the markdown can say so explicitly.
pub fn summary_scope(sheet: &Sheet) -> Option<SummaryScope> {
    let range = used_range(&sheet.cells)?;
    let last_row = table_extent(&sheet.cells, range.max_row, range.max_col);
    let excluded = (1..=last_row)
        .filter(|r| is_aggregate_row(&sheet.cells, *r, range.max_col))
        .map(|r| r + 1)
        .collect();
    Some(SummaryScope {
        first_row: 2,
        last_row: last_row + 1,
        excluded,
        truncated: last_row < range.max_row,
    })
}

fn format_stat(n: f64, fmt: Option<&str>) -> String {
    if fmt.is_some_and(|f| f.contains('%')) {
        let pct = crate::geometry::js_round(n * 1000.0) / 10.0;
        return format!(
            "{}%",
            jsnum::to_locale_string(pct.abs(), decimals_of(pct), true).with_sign(pct)
        );
    }
    let rounded = crate::geometry::js_round(n * 100.0) / 100.0;
    jsnum::to_locale_string(rounded.abs(), decimals_of(rounded), true).with_sign(rounded)
}

/// `toLocaleString` with no options keeps up to 3 fraction digits; the values
/// here are already rounded to 2 (or 1 for percentages), so print what is there.
fn decimals_of(n: f64) -> usize {
    let s = jsnum::number_to_string(n.abs());
    match s.split_once('.') {
        Some((_, frac)) => frac.len().min(3),
        None => 0,
    }
}

trait WithSign {
    fn with_sign(self, n: f64) -> String;
}

impl WithSign for String {
    fn with_sign(self, n: f64) -> String {
        if n < 0.0 {
            format!("-{self}")
        } else {
            self
        }
    }
}

/// Split a table row on its cell separators.
///
/// `\|` is an escaped pipe *inside* a cell — the projection writes it whenever a
/// value contains one. Splitting naively on every `|`, as the JavaScript did,
/// tore `a \| b` into two cells and lost the tail on re-import.
fn split_unescaped_pipes(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cell.push('\\');
                if let Some(next) = chars.next() {
                    cell.push(next);
                }
            }
            '|' => {
                out.push(cell.trim().to_string());
                cell = String::new();
            }
            c => cell.push(c),
        }
    }
    out.push(cell.trim().to_string());
    out
}

static TABLE_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[ \t]*\|").unwrap());
/// The alignment row of a markdown table. It must contain a dash: `|   |   |`
/// is a row of empty cells, and accepting it here deleted a blank sheet row.
static TABLE_RULE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\|[\s:|-]*-[\s:|-]*\|$").unwrap());
static COL_LETTER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Z]{1,3}$").unwrap());

/// Import a markdown table into cells.
///
/// Recognises the projection format this module emits (leading blank corner +
/// column letters, bolded row numbers) and also plain markdown tables, which
/// land at A1 with the header row intact.
pub fn import_markdown_table(md: &str) -> IndexMap<String, Cell> {
    let split = parse_frontmatter(md);
    let mut table_lines: Vec<&str> = Vec::new();
    let mut in_table = false;
    for line in split.body.lines() {
        if TABLE_LINE.is_match(line) {
            in_table = true;
            table_lines.push(line.trim());
            continue;
        }
        if in_table {
            break;
        }
    }
    if table_lines.len() < 2 {
        return IndexMap::new();
    }

    let rows: Vec<Vec<String>> = table_lines
        .iter()
        .filter(|l| !TABLE_RULE.is_match(l))
        .map(|l| {
            let trimmed = l.strip_prefix('|').unwrap_or(l);
            let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
            split_unescaped_pipes(trimmed)
        })
        .collect();
    if rows.is_empty() {
        return IndexMap::new();
    }

    let header = &rows[0];
    let looks_projected = header.len() > 1
        && header[0].is_empty()
        && header[1..].iter().all(|h| COL_LETTER.is_match(h));

    let mut cells = IndexMap::new();
    let data_rows = if looks_projected {
        &rows[1..]
    } else {
        &rows[..]
    };

    for (r_idx, row) in data_rows.iter().enumerate() {
        let cols = if looks_projected { &row[1..] } else { &row[..] };
        let mut row_index = r_idx;
        if looks_projected {
            let label = row[0].replace('*', "");
            if let Ok(parsed) = label.trim().parse::<usize>() {
                if parsed > 0 {
                    row_index = parsed - 1;
                }
            }
        }
        for (c_idx, raw) in cols.iter().enumerate() {
            let bold = raw.len() > 4 && raw.starts_with("**") && raw.ends_with("**");
            let text = raw
                .trim_start_matches("**")
                .trim_end_matches("**")
                .replace("\\|", "|")
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            let Some(mut cell) = parse_cell_input(&text) else {
                continue;
            };
            if bold {
                cell.extra.insert("style".into(), json!({ "bold": true }));
            }
            cells.insert(to_ref(c_idx, row_index), cell);
        }
    }
    cells
}

pub fn make_sheet(name: &str, with_sample: bool) -> Sheet {
    let mut cells: IndexMap<String, Cell> = IndexMap::new();
    if with_sample {
        for (i, header) in ["항목", "1분기", "2분기", "합계"].iter().enumerate() {
            cells.insert(
                to_ref(i, 0),
                Cell {
                    v: json!(header),
                    t: Some("s".into()),
                    extra: [(
                        "style".to_string(),
                        json!({ "bold": true, "bg": "#f1f5f9" }),
                    )]
                    .into_iter()
                    .collect(),
                    ..Cell::default()
                },
            );
        }
        let data: [(&str, i64, i64); 3] = [
            ("제품 A", 1200, 1350),
            ("제품 B", 980, 1120),
            ("제품 C", 640, 890),
        ];
        for (i, (label, q1, q2)) in data.iter().enumerate() {
            let r = i + 1;
            cells.insert(
                to_ref(0, r),
                Cell {
                    v: json!(label),
                    t: Some("s".into()),
                    ..Cell::default()
                },
            );
            for (col, value) in [(1usize, q1), (2usize, q2)] {
                cells.insert(
                    to_ref(col, r),
                    Cell {
                        v: json!(value),
                        t: Some("n".into()),
                        fmt: Some("#,##0".into()),
                        ..Cell::default()
                    },
                );
            }
            cells.insert(
                to_ref(3, r),
                Cell {
                    f: Some(format!("=B{}+C{}", r + 1, r + 1)),
                    t: Some("n".into()),
                    fmt: Some("#,##0".into()),
                    ..Cell::default()
                },
            );
        }
        let total_row = data.len() + 1;
        cells.insert(
            to_ref(0, total_row),
            Cell {
                v: json!("총계"),
                t: Some("s".into()),
                extra: [("style".to_string(), json!({ "bold": true }))]
                    .into_iter()
                    .collect(),
                ..Cell::default()
            },
        );
        for col in 1..=3usize {
            let letter = index_to_col(col);
            cells.insert(
                to_ref(col, total_row),
                Cell {
                    f: Some(format!("=SUM({letter}2:{letter}{total_row})")),
                    t: Some("n".into()),
                    fmt: Some("#,##0".into()),
                    extra: [(
                        "style".to_string(),
                        json!({ "bold": true, "bg": "#f8fafc" }),
                    )]
                    .into_iter()
                    .collect(),
                    ..Cell::default()
                },
            );
        }
    }

    normalize_sheet(&Sheet {
        name: name.to_string(),
        cells,
        frozen: Frozen { rows: 1, cols: 1 },
        ..empty_sheet()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet_of(pairs: &[(&str, Cell)]) -> Sheet {
        normalize_sheet(&Sheet {
            cells: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            ..empty_sheet()
        })
    }

    fn text(v: &str) -> Cell {
        Cell {
            v: json!(v),
            t: Some("s".into()),
            ..Cell::default()
        }
    }
    fn number(v: f64) -> Cell {
        Cell {
            v: json!(v),
            t: Some("n".into()),
            ..Cell::default()
        }
    }
    fn formula(f: &str) -> Cell {
        Cell {
            f: Some(f.into()),
            t: Some("n".into()),
            ..Cell::default()
        }
    }

    #[test]
    fn the_used_range_ignores_truly_empty_cells() {
        let s = sheet_of(&[("A1", text("a")), ("C3", number(1.0))]);
        let r = used_range(&s.cells).unwrap();
        assert_eq!((r.max_col, r.max_row), (2, 2));
        assert!(used_range(&IndexMap::new()).is_none());
    }

    #[test]
    fn the_markdown_projection_carries_addresses() {
        let s = make_sheet("예산", true);
        let files = write_sheet(&s);
        // Column letters and bolded row numbers.
        assert!(files.md.contains("| A | B | C | D |"), "{}", files.md);
        assert!(files.md.contains("| **1** |"), "{}", files.md);
        // Formulas appear with both expression and result.
        assert!(files.md.contains("### 수식"));
        assert!(files.md.contains("`D2` = `=B2+C2` → 2,550"), "{}", files.md);
        assert!(
            files.md.contains("`B5` = `=SUM(B2:B4)` → 2,820"),
            "{}",
            files.md
        );
    }

    #[test]
    fn the_column_summary_excludes_the_total_row() {
        // Without the exclusion, B's sum would double-count the total.
        let s = make_sheet("예산", true);
        let with_values = recalculated(&s);
        let summary = summarize_columns(&with_values);
        let b = summary.iter().find(|c| c.col == "B").unwrap();
        assert_eq!(b.col_type, "숫자");
        assert_eq!(b.count, 3);
        assert_eq!(b.sum.as_deref(), Some("2,820"));
        assert_eq!(b.avg.as_deref(), Some("940"));

        let scope = summary_scope(&with_values).unwrap();
        assert_eq!(scope.excluded, vec![5], "the 총계 row is row 5");
    }

    #[test]
    fn an_unlabelled_total_row_is_still_detected() {
        let s = sheet_of(&[
            ("A1", text("항목")),
            ("B1", text("값")),
            ("A2", text("x")),
            ("B2", number(10.0)),
            ("A3", text("y")),
            ("B3", number(20.0)),
            ("A4", text("")),
            ("B4", formula("=SUM(B2:B3)")),
        ]);
        let with_values = recalculated(&s);
        let b = summarize_columns(&with_values)
            .into_iter()
            .find(|c| c.col == "B")
            .unwrap();
        assert_eq!(
            b.sum.as_deref(),
            Some("30"),
            "the aggregate row must be excluded"
        );
    }

    #[test]
    fn the_summary_stops_at_a_blank_row() {
        let s = sheet_of(&[
            ("A1", text("항목")),
            ("B1", text("값")),
            ("A2", text("x")),
            ("B2", number(10.0)),
            // blank row 3
            ("A4", text("무관한 블록")),
            ("B4", number(999.0)),
        ]);
        let b = summarize_columns(&s)
            .into_iter()
            .find(|c| c.col == "B")
            .unwrap();
        assert_eq!(b.sum.as_deref(), Some("10"), "999 sits past the blank row");
        assert!(summary_scope(&s).unwrap().truncated);
    }

    #[test]
    fn a_projection_round_trips_when_the_json_is_missing() {
        let original = make_sheet("예산", true);
        let files = write_sheet(&original);
        // Only the markdown survives — the hand-edited case from the README.
        let back = read_sheet(&files.md, None);
        assert_eq!(back.name, "예산");
        assert_eq!(display_value(back.cells.get("A1").unwrap()), "항목");
        // Values come back as values; the formula text is not in the projection.
        assert_eq!(display_value(back.cells.get("D2").unwrap()), "2,550");
        assert_eq!(
            back.cells.get("A1").unwrap().extra.get("style"),
            Some(&json!({ "bold": true })),
            "bold markers in the table survive"
        );
    }

    #[test]
    fn a_blank_row_in_a_projection_is_not_swallowed() {
        // `|   |   |` used to match the alignment-rule pattern, so re-importing a
        // sheet with an empty row shifted every row below it up by one.
        let cells = import_markdown_table(
            "|     | A | B |\n|---|---|---|\n| **1** | 머리글 | 값 |\n| **2** |   |   |\n| **3** | 자료 | 7 |\n",
        );
        assert_eq!(display_value(cells.get("A3").unwrap()), "자료");
        assert_eq!(display_value(cells.get("B3").unwrap()), "7");
    }

    #[test]
    fn a_plain_markdown_table_lands_at_a1() {
        let cells = import_markdown_table("| 이름 | 값 |\n|---|---|\n| a | 1 |\n");
        assert_eq!(display_value(cells.get("A1").unwrap()), "이름");
        assert_eq!(display_value(cells.get("B2").unwrap()), "1");
    }

    #[test]
    fn escaped_pipes_survive_the_projection() {
        let s = sheet_of(&[("A1", text("a | b"))]);
        let files = write_sheet(&s);
        assert!(files.md.contains("a \\| b"), "{}", files.md);
        let back = read_sheet(&files.md, None);
        assert_eq!(display_value(back.cells.get("A1").unwrap()), "a | b");
    }

    #[test]
    fn the_cells_json_is_the_authority_when_both_exist() {
        let original = make_sheet("예산", true);
        let files = write_sheet(&original);
        let back = read_sheet(&files.md, Some(&files.cells));
        // The formula is restored, not the projected value.
        assert_eq!(back.cells.get("D2").unwrap().f.as_deref(), Some("=B2+C2"));
        assert_eq!(back.frozen, Frozen { rows: 1, cols: 1 });
    }

    #[test]
    fn a_recalculated_sheet_reports_the_documented_totals() {
        let s = recalculated(&make_sheet("예산", true));
        assert_eq!(display_value(s.cells.get("D5").unwrap()), "6,180");
    }

    #[test]
    fn empty_cells_are_stripped_from_the_json() {
        let s = sheet_of(&[("A1", text("a"))]);
        let mut with_ghost = s.clone();
        with_ghost.cells.insert("Z9".into(), Cell::default());
        let files = write_sheet(&with_ghost);
        assert!(files.cells["cells"].get("Z9").is_none());
        assert!(files.cells["cells"].get("A1").is_some());
    }

    #[test]
    fn charts_project_their_numbers_into_the_markdown() {
        let mut s = sheet_of(&[
            ("A1", text("구분")),
            ("B1", text("2026")),
            ("A2", text("1분기")),
            ("B2", number(120.0)),
        ]);
        s.charts.push(SheetChart {
            id: "c1".into(),
            x: 80.0,
            y: 80.0,
            w: 460.0,
            h: 300.0,
            spec: normalize_spec(&json!({ "type": "column", "title": "매출", "range": "A1:B2" })),
        });
        let files = write_sheet(&s);
        assert!(files.md.contains("### 차트"));
        assert!(files.md.contains("세로 막대 차트 \"매출\""), "{}", files.md);
        assert!(files.md.contains("| 1분기 | 120 |"), "{}", files.md);
    }

    #[test]
    fn out_of_range_dimensions_are_clamped() {
        let s = normalize_sheet(&Sheet {
            dims: Dims {
                rows: 999_999,
                cols: 9999,
            },
            frozen: Frozen { rows: 99, cols: 99 },
            ..empty_sheet()
        });
        assert_eq!(
            s.dims,
            Dims {
                rows: 100_000,
                cols: 702
            }
        );
        assert_eq!(s.frozen, Frozen { rows: 20, cols: 20 });
    }

    #[test]
    fn a_percentage_column_formats_its_stats_as_percentages() {
        let pct = |v: f64| Cell {
            v: json!(v),
            t: Some("n".into()),
            fmt: Some("0.0%".into()),
            ..Cell::default()
        };
        let s = sheet_of(&[("A1", text("비중")), ("A2", pct(0.48)), ("A3", pct(0.52))]);
        let a = summarize_columns(&s).into_iter().next().unwrap();
        assert_eq!(a.sum.as_deref(), Some("100%"));
        assert_eq!(a.avg.as_deref(), Some("50%"));
    }
}
