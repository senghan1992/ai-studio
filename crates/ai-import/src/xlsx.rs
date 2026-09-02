//! Reading a `.xlsx` into a grid project.
//!
//! This is the highest-fidelity of the three importers, because almost
//! everything in a worksheet already has a home in this format: a formula is a
//! formula, a number format is a number format. What arrives is what Excel had.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::json;

use ai_format::grid::normalize_sheet;
use ai_format::model::{Dims, Frozen, Sheet, SheetChart};
use ai_formula::evaluate::Cell;
use ai_formula::refs::{index_to_col, parse_range, parse_ref, shift_formula, CellRange};

use crate::ooxml::{color, px, px_from_half_point, Node, Package, Result};
use crate::Warnings;

/// Excel column width is in characters; the editor stores px.
const PX_PER_CHAR: f64 = 7.0;

/// Read a workbook, discarding what could not be carried across.
///
/// `read_with` is the one the app calls; this exists for tests and probes that
/// have nothing to show a warning in.
pub fn read(package: &Package) -> Result<Vec<Sheet>> {
    read_with(package, &mut Warnings::default())
}

pub fn read_with(package: &Package, warnings: &mut Warnings) -> Result<Vec<Sheet>> {
    let workbook = package.xml("xl/workbook.xml")?;
    let rels = package.rels_for("xl/workbook.xml");
    let strings = shared_strings(package);
    let styles = Styles::read(package);
    let names = defined_names(&workbook);

    let Some(book) = workbook.child("workbook") else {
        return Ok(Vec::new());
    };

    let mut sheets = Vec::new();
    for (index, entry) in book
        .child("sheets")
        .map(|s| s.children_named("sheet").collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let name = entry
            .attr("name")
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("시트{}", index + 1));

        let Some(target) = entry
            .attr("id")
            .and_then(|id| rels.get(id))
            .map(|r| r.target.clone())
        else {
            continue;
        };
        let Some(part) = package.xml_opt(&target) else {
            continue;
        };
        let Some(worksheet) = part.child("worksheet") else {
            continue;
        };

        sheets.push(read_sheet(
            &name, worksheet, &strings, &styles, &names, package, &target, warnings,
        ));
    }

    if sheets.is_empty() {
        sheets.push(ai_format::grid::make_sheet("시트1", false));
    }
    Ok(sheets)
}

#[allow(clippy::too_many_arguments)]
fn read_sheet(
    name: &str,
    worksheet: &Node,
    strings: &[String],
    styles: &Styles,
    all_names: &[(String, String, Option<usize>)],
    package: &Package,
    part: &str,
    warnings: &mut Warnings,
) -> Sheet {
    let mut cells: IndexMap<String, Cell> = IndexMap::new();
    // Shared formulas, by their `si` index. Excel writes the text once, on the
    // top-left cell of the range, and leaves every other cell in the range with
    // an empty `<f t="shared" si="0"/>`. A reader that only takes the text loses
    // the formula from every cell the author dragged it into — which, in a real
    // workbook, is most of them.
    let mut shared: HashMap<String, (String, String)> = HashMap::new();

    if let Some(data) = worksheet.child("sheetData") {
        for row in data.children_named("row") {
            for c in row.children_named("c") {
                let Some(reference) = c.attr("r").map(str::to_string) else {
                    continue;
                };
                if parse_ref(&reference).is_none() {
                    continue;
                }
                let reference = reference.to_uppercase();
                if let Some(cell) = read_cell(c, &reference, strings, styles, &mut shared) {
                    cells.insert(reference, cell);
                }
            }
        }
    }

    let merges = worksheet
        .child("mergeCells")
        .map(|m| {
            m.children_named("mergeCell")
                .filter_map(|c| c.attr("ref"))
                .filter(|spec| parse_range(spec).is_some())
                .map(|spec| json!(spec))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let frozen = read_panes(worksheet);
    let col_widths = read_col_widths(worksheet);
    let row_heights = read_row_heights(worksheet);

    // Defined names are a workbook-level thing in Excel and a per-sheet thing
    // here, so every sheet gets every name. One that points at another sheet
    // keeps its sheet prefix, which the formula engine resolves — dropping such
    // names is how `=매출데이터` on a summary sheet becomes `#NAME?`.
    let mut names = IndexMap::new();
    for (label, target, _scope) in all_names {
        let (sheet_part, range) = match target.rsplit_once('!') {
            Some((sheet_part, range)) => (sheet_part.trim_matches('\''), range),
            None => ("", target.as_str()),
        };
        let bare: String = range.chars().filter(|c| *c != '$').collect();
        if parse_range(&bare).is_none() && parse_ref(&bare).is_none() {
            continue;
        }
        let value = if sheet_part.is_empty() || sheet_part == name {
            bare
        } else {
            format!("{sheet_part}!{bare}")
        };
        names.insert(label.clone(), json!(value));
    }

    // Conditional formatting has no equivalent that stays conditional, so it is
    // baked in against the values the file was saved with — see `apply_rules`.
    apply_rules(worksheet, styles, &mut cells, &names, warnings);

    // The parts of a sheet this format has no place for. Each one is a visible
    // difference, so each one is named.
    if worksheet.child("dataValidations").is_some() {
        warnings.note("데이터 유효성 검사(드롭다운)는 넘어오지 않습니다");
    }
    if !worksheet.descendants("autoFilter").is_empty() {
        warnings.note("자동 필터는 넘어오지 않습니다 (값과 서식은 그대로입니다)");
    }
    let hidden = worksheet
        .descendants("row")
        .iter()
        .any(|r| r.attr_bool("hidden"))
        || worksheet
            .descendants("col")
            .iter()
            .any(|c| c.attr_bool("hidden"));
    if hidden {
        warnings.note("숨긴 행·열은 숨김을 표현할 방법이 없어 그대로 보입니다");
    }

    let dims = read_dims(&cells);
    let charts = read_charts(package, part, name, &col_widths, &row_heights, warnings);

    normalize_sheet(&Sheet {
        id: ai_format::ids::new_sheet_id(),
        name: name.to_string(),
        dims,
        frozen,
        col_widths,
        row_heights,
        merges,
        names,
        charts,
        cells,
        file: None,
    })
}

fn read_cell(
    c: &Node,
    reference: &str,
    strings: &[String],
    styles: &Styles,
    shared: &mut HashMap<String, (String, String)>,
) -> Option<Cell> {
    let mut cell = Cell::default();
    let cell_type = c.attr("t").unwrap_or("n");

    if let Some(f) = c.child("f") {
        let text = f.all_text().trim().to_string();
        let index = f.attr("si").map(str::to_string);
        let is_shared = f.attr("t") == Some("shared");

        if !text.is_empty() {
            cell.f = Some(format!("={text}"));
            // The master of a shared group: remember where it was, so the cells
            // that only reference it can be shifted off it.
            if let (true, Some(index)) = (is_shared, index) {
                shared.insert(index, (reference.to_string(), text));
            }
        } else if let Some((master, formula)) = index.and_then(|i| shared.get(&i)) {
            cell.f = Some(shifted_from(master, reference, formula));
        }
    }

    let raw = c.child("v").map(|v| v.all_text()).unwrap_or_default();
    let raw = raw.trim();

    match cell_type {
        "s" => {
            // Shared string: the value is an index into the table.
            let text = raw
                .parse::<usize>()
                .ok()
                .and_then(|i| strings.get(i).cloned())
                .unwrap_or_default();
            if text.is_empty() && cell.f.is_none() {
                return style_only(cell, c, styles);
            }
            cell.v = json!(text);
            cell.t = Some("s".into());
        }
        "inlineStr" => {
            let text = c.child("is").map(|is| is.all_text()).unwrap_or_default();
            if text.is_empty() && cell.f.is_none() {
                return style_only(cell, c, styles);
            }
            cell.v = json!(text);
            cell.t = Some("s".into());
        }
        "str" => {
            cell.v = json!(raw);
            cell.t = Some("s".into());
        }
        "b" => {
            cell.v = json!(raw == "1" || raw.eq_ignore_ascii_case("true"));
            cell.t = Some("b".into());
        }
        "e" => {
            cell.v = json!(raw);
            cell.t = Some("e".into());
        }
        _ => {
            if raw.is_empty() {
                if cell.f.is_none() {
                    return style_only(cell, c, styles);
                }
                cell.t = Some("n".into());
            } else if let Ok(number) = raw.parse::<f64>() {
                cell.v = json!(number);
                cell.t = Some("n".into());
            } else {
                cell.v = json!(raw);
                cell.t = Some("s".into());
            }
        }
    }

    apply_style(&mut cell, c, styles);
    Some(cell)
}

/// A cell with no value but real formatting is still worth keeping: it is how a
/// spreadsheet draws an empty bordered row.
fn style_only(mut cell: Cell, c: &Node, styles: &Styles) -> Option<Cell> {
    apply_style(&mut cell, c, styles);
    if cell.fmt.is_some() || !cell.extra.is_empty() {
        Some(cell)
    } else {
        None
    }
}

fn apply_style(cell: &mut Cell, c: &Node, styles: &Styles) {
    let Some(index) = c.attr("s").and_then(|s| s.parse::<usize>().ok()) else {
        return;
    };
    let Some(xf) = styles.xfs.get(index) else {
        return;
    };

    if let Some(code) = styles.num_fmt(xf.num_fmt) {
        // A date format is what tells us the number is a date.
        let has_date = code
            .chars()
            .any(|ch| matches!(ch, 'y' | 'm' | 'd' | 'h' | 's'));
        let has_digits = code.contains('#') || code.contains('0');
        if has_date && !has_digits && cell.t.as_deref() == Some("n") {
            cell.t = Some("d".into());
        }
        cell.fmt = Some(code);
    }

    let mut style = serde_json::Map::new();
    if let Some(font) = styles.fonts.get(xf.font) {
        if font.bold {
            style.insert("bold".into(), json!(true));
        }
        if font.italic {
            style.insert("italic".into(), json!(true));
        }
        if font.underline {
            style.insert("underline".into(), json!(true));
        }
        if let Some(c) = &font.color {
            style.insert("color".into(), json!(c));
        }
        // Only a size the grid would not have drawn anyway: a workbook states
        // 11pt on every cell, and carrying that would put a style on all of
        // them for no visible difference.
        if let Some(size) = font
            .size_px
            .map(|px| px.round())
            .filter(|px| *px != ai_format::grid::CELL_PX.round())
        {
            style.insert("fontSize".into(), json!(size));
        }
    }
    if let Some(fill) = styles.fills.get(xf.fill).and_then(|f| f.as_ref()) {
        style.insert("bg".into(), json!(fill));
    }
    if let Some(align) = &xf.align {
        style.insert("align".into(), json!(align));
    }
    if let Some(border) = styles.borders.get(xf.border).and_then(|b| b.as_ref()) {
        style.insert("border".into(), json!(border));
    }
    if !style.is_empty() {
        cell.extra
            .insert("style".into(), serde_json::Value::Object(style));
    }
}

/// Turn conditional formatting into ordinary cell styles.
///
/// A rule cannot stay a rule here: this format has no conditional formatting, so
/// keeping it would mean dropping it. Applying it against the values the file was
/// saved with gives a sheet that *looks* like the one the author saw — the red
/// negatives are red, the heat map is a heat map — and stops being conditional,
/// which is what the warning says.
fn apply_rules(
    worksheet: &Node,
    styles: &Styles,
    cells: &mut IndexMap<String, Cell>,
    names: &IndexMap<String, serde_json::Value>,
    warnings: &mut Warnings,
) {
    let blocks: Vec<&Node> = worksheet.children_named("conditionalFormatting").collect();
    if blocks.is_empty() {
        return;
    }
    let snapshot = cells.clone();
    let mut styled = 0usize;
    let mut approximated = false;

    // In priority order, so the rule Excel would have drawn wins.
    let mut rules: Vec<(i64, &Node, Vec<String>)> = Vec::new();
    for block in blocks {
        let refs: Vec<String> = block
            .attr("sqref")
            .unwrap_or("")
            .split_whitespace()
            .flat_map(|part| {
                if part.contains(':') {
                    ai_formula::refs::expand_range_str(part)
                } else {
                    vec![part.to_uppercase()]
                }
            })
            .collect();
        for rule in block.children_named("cfRule") {
            let priority = rule.attr_i64("priority").unwrap_or(i64::MAX);
            rules.push((priority, rule, refs.clone()));
        }
    }
    rules.sort_by_key(|(priority, ..)| *priority);

    let mut decided: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, rule, refs) in rules {
        let kind = rule.attr("type").unwrap_or("");
        let dxf = rule
            .attr_i64("dxfId")
            .and_then(|id| styles.dxfs.get(id as usize))
            .cloned()
            .unwrap_or_default();

        // A colour scale is a background colour by nature, so it converts
        // exactly; a data bar or an icon set is a drawing inside the cell, and
        // painting the whole cell instead would say something the file does not.
        if kind == "colorScale" {
            let values: Vec<f64> = refs
                .iter()
                .filter_map(|r| snapshot.get(r))
                .filter_map(|c| c.v.as_f64())
                .collect();
            let stops: Vec<String> = rule
                .path(&["colorScale"])
                .map(|scale| {
                    scale
                        .children_named("color")
                        .filter_map(|c| c.attr("rgb"))
                        .filter_map(color)
                        .collect()
                })
                .unwrap_or_default();
            if values.len() < 2 || stops.len() < 2 {
                continue;
            }
            let (low, high) = (
                values.iter().cloned().fold(f64::INFINITY, f64::min),
                values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            );
            for reference in &refs {
                let Some(value) = snapshot.get(reference).and_then(|c| c.v.as_f64()) else {
                    continue;
                };
                let t = if high > low {
                    (value - low) / (high - low)
                } else {
                    0.5
                };
                let scale = Dxf {
                    bg: Some(blend(&stops, t)),
                    ..Dxf::default()
                };
                if let Some(cell) = cells.get_mut(reference) {
                    scale.apply(cell);
                    styled += 1;
                }
            }
            continue;
        }
        if matches!(kind, "dataBar" | "iconSet") {
            approximated = true;
            continue;
        }
        if dxf.is_empty() {
            continue;
        }

        let formulas: Vec<String> = rule
            .children_named("formula")
            .map(|f| f.all_text().trim().to_string())
            .collect();
        let text = rule.attr("text").unwrap_or("");
        // `top10` and `aboveAverage` need the whole range before any one cell can
        // be judged.
        let ranked: Vec<f64> = refs
            .iter()
            .filter_map(|r| snapshot.get(r))
            .filter_map(|c| c.v.as_f64())
            .collect();

        for reference in &refs {
            if decided.contains(reference) {
                continue;
            }
            let Some(cell) = snapshot.get(reference) else {
                continue;
            };
            if matches(
                kind,
                rule,
                &formulas,
                text,
                cell,
                reference,
                refs.first().map(String::as_str).unwrap_or(reference),
                &ranked,
                &snapshot,
                names,
            ) {
                if let Some(target) = cells.get_mut(reference) {
                    dxf.apply(target);
                    decided.insert(reference.clone());
                    styled += 1;
                }
            }
        }
    }

    if styled > 0 {
        warnings.note("조건부 서식은 저장된 값 기준의 고정 서식으로 바꿨습니다");
    }
    if approximated {
        warnings.note("데이터 막대·아이콘 집합은 표현할 방법이 없어 넘어갔습니다");
    }
}

/// A colour between the scale's stops.
fn blend(stops: &[String], t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    // With three stops the middle one sits at the halfway point, as Excel's own
    // midpoint percentile does for a default scale.
    let (from, to, local) = if stops.len() >= 3 {
        if t < 0.5 {
            (&stops[0], &stops[1], t * 2.0)
        } else {
            (&stops[1], &stops[2], (t - 0.5) * 2.0)
        }
    } else {
        (&stops[0], &stops[1], t)
    };
    let mix = |a: &str, b: &str| -> String {
        let parse = |hex: &str| {
            let v = hex.trim_start_matches('#');
            (
                u8::from_str_radix(&v[0..2], 16).unwrap_or(0) as f64,
                u8::from_str_radix(&v[2..4], 16).unwrap_or(0) as f64,
                u8::from_str_radix(&v[4..6], 16).unwrap_or(0) as f64,
            )
        };
        let (ar, ag, ab) = parse(a);
        let (br, bg, bb) = parse(b);
        format!(
            "#{:02x}{:02x}{:02x}",
            (ar + (br - ar) * local).round() as u8,
            (ag + (bg - ag) * local).round() as u8,
            (ab + (bb - ab) * local).round() as u8
        )
    };
    mix(from, to)
}

/// Whether one cell satisfies one rule.
#[allow(clippy::too_many_arguments)]
fn matches(
    kind: &str,
    rule: &Node,
    formulas: &[String],
    text: &str,
    cell: &Cell,
    reference: &str,
    // The top-left cell of the range the rule was written for.
    anchor: &str,
    ranked: &[f64],
    sheet: &IndexMap<String, Cell>,
    names: &IndexMap<String, serde_json::Value>,
) -> bool {
    let number = cell.v.as_f64();
    let as_text = match &cell.v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    let threshold = |i: usize| -> Option<f64> {
        formulas
            .get(i)
            .and_then(|f| f.trim().trim_matches('"').parse::<f64>().ok())
    };

    match kind {
        "cellIs" => {
            let Some(value) = number else {
                // A text cell compared as text, which is what Excel does for
                // `equal` on words.
                let target = formulas.first().map(|f| f.trim().trim_matches('"'));
                return matches!(rule.attr("operator"), Some("equal"))
                    && target == Some(&as_text[..]);
            };
            let (a, b) = (threshold(0), threshold(1));
            match rule.attr("operator").unwrap_or("") {
                "lessThan" => a.is_some_and(|t| value < t),
                "lessThanOrEqual" => a.is_some_and(|t| value <= t),
                "greaterThan" => a.is_some_and(|t| value > t),
                "greaterThanOrEqual" => a.is_some_and(|t| value >= t),
                "equal" => a.is_some_and(|t| value == t),
                "notEqual" => a.is_some_and(|t| value != t),
                "between" => {
                    matches!((a, b), (Some(x), Some(y)) if value >= x.min(y) && value <= x.max(y))
                }
                "notBetween" => {
                    matches!((a, b), (Some(x), Some(y)) if value < x.min(y) || value > x.max(y))
                }
                _ => false,
            }
        }
        "containsText" => !text.is_empty() && as_text.contains(text),
        "notContainsText" => !text.is_empty() && !as_text.contains(text),
        "beginsWith" => !text.is_empty() && as_text.starts_with(text),
        "endsWith" => !text.is_empty() && as_text.ends_with(text),
        "containsBlanks" => as_text.trim().is_empty(),
        "notContainsBlanks" => !as_text.trim().is_empty(),
        "duplicateValues" | "uniqueValues" => {
            let same = sheet.iter().filter(|(_, other)| other.v == cell.v).count();
            if kind == "duplicateValues" {
                same > 1
            } else {
                same == 1
            }
        }
        "top10" => {
            let Some(value) = number else { return false };
            let count = rule.attr_i64("rank").unwrap_or(10).max(1) as usize;
            let bottom = rule.attr_bool("bottom");
            let mut sorted = ranked.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let cut = if bottom {
                sorted.get(count.saturating_sub(1)).copied()
            } else {
                sorted.get(sorted.len().saturating_sub(count)).copied()
            };
            match (cut, bottom) {
                (Some(cut), true) => value <= cut,
                (Some(cut), false) => value >= cut,
                _ => false,
            }
        }
        "aboveAverage" => {
            let Some(value) = number else { return false };
            if ranked.is_empty() {
                return false;
            }
            let mean = ranked.iter().sum::<f64>() / ranked.len() as f64;
            if rule.attr("aboveAverage") == Some("0") {
                value < mean
            } else {
                value > mean
            }
        }
        // The rule is a formula written for the range's first cell, so it is
        // moved onto this one exactly the way filling a formula down moves it.
        "expression" => {
            let Some(formula) = formulas.first() else {
                return false;
            };
            let shifted = shift_between(anchor, reference, formula);
            let value = ai_formula::evaluate::evaluate_in(&shifted, sheet, names);
            matches!(
                ai_formula::values::to_boolean(&value),
                ai_formula::values::Value::Bool(true)
            )
        }
        _ => false,
    }
}

/// A formula written for `from` as it applies at `to`.
fn shift_between(from: &str, to: &str, formula: &str) -> String {
    let (Some(a), Some(b)) = (parse_ref(from), parse_ref(to)) else {
        return format!("={formula}");
    };
    shift_formula(
        &format!("={formula}"),
        b.col as i64 - a.col as i64,
        b.row as i64 - a.row as i64,
    )
}

/// The charts anchored on a sheet.
///
/// A workbook's charts are its point, often more than its numbers: dropping them
/// on import loses the part of the file the author spent their time on. The
/// numbers come from the chart's own cached series, the same way a deck's do.
fn read_charts(
    package: &Package,
    worksheet_part: &str,
    sheet_name: &str,
    col_widths: &IndexMap<String, i64>,
    row_heights: &IndexMap<String, i64>,
    warnings: &mut Warnings,
) -> Vec<SheetChart> {
    let sheet_rels = package.rels_for(worksheet_part);
    let Some(drawing) = sheet_rels.values().find(|r| r.kind == "drawing") else {
        return Vec::new();
    };
    let Some(document) = package.xml_opt(&drawing.target) else {
        return Vec::new();
    };
    let drawing_rels = package.rels_for(&drawing.target);

    // Anchors are in (column, row) with an offset in EMU inside the cell, so the
    // pixel position depends on the column widths this sheet actually has.
    let x_of = |col: i64, offset: i64| -> f64 {
        let mut x = 0.0;
        for c in 0..col.max(0) {
            x += *col_widths
                .get(&index_to_col(c as usize))
                .unwrap_or(&ai_format::grid::DEFAULT_COL_WIDTH) as f64;
        }
        x + px(offset)
    };
    let y_of = |row: i64, offset: i64| -> f64 {
        let mut y = 0.0;
        for r in 0..row.max(0) {
            y += *row_heights
                .get(&(r + 1).to_string())
                .unwrap_or(&ai_format::grid::DEFAULT_ROW_HEIGHT) as f64;
        }
        y + px(offset)
    };
    let corner = |anchor: &Node, which: &str| -> Option<(f64, f64)> {
        let node = anchor.child(which)?;
        let col = node.child("col").map(|c| c.all_text()).unwrap_or_default();
        let row = node.child("row").map(|r| r.all_text()).unwrap_or_default();
        let col_off = node
            .child("colOff")
            .map(|c| c.all_text())
            .unwrap_or_default();
        let row_off = node
            .child("rowOff")
            .map(|r| r.all_text())
            .unwrap_or_default();
        Some((
            x_of(
                col.trim().parse().unwrap_or(0),
                col_off.trim().parse().unwrap_or(0),
            ),
            y_of(
                row.trim().parse().unwrap_or(0),
                row_off.trim().parse().unwrap_or(0),
            ),
        ))
    };

    if !document.descendants("pic").is_empty() {
        warnings.note("시트 안의 그림은 넘어오지 않습니다 (차트는 넘어옵니다)");
    }

    let mut out = Vec::new();
    for anchor in document
        .descendants("twoCellAnchor")
        .into_iter()
        .chain(document.descendants("oneCellAnchor"))
    {
        let Some(reference) = anchor
            .descendants("chart")
            .first()
            .and_then(|c| c.attr("id"))
            .and_then(|id| drawing_rels.get(id))
        else {
            continue;
        };
        let Some(imported) = crate::pptx::read_chart(package, &reference.target) else {
            continue;
        };
        if let Some(element) = &imported.substituted {
            warnings.note(&format!(
                "{} 차트는 꺾은선으로 바꿨습니다",
                crate::pptx::chart_type_label(element)
            ));
        }
        let mut spec = imported.spec;
        // A chart in a workbook points at cells in that workbook, so it can stay
        // live rather than keeping a copy of the numbers. The cached values are
        // left in place as the fallback the format already has for a range that
        // resolves to nothing.
        spec.range = chart_range(package, &reference.target, sheet_name);
        let (x, y) = corner(anchor, "from").unwrap_or((80.0, 80.0));
        // `oneCellAnchor` states a size instead of a second corner.
        let (w, h) = match corner(anchor, "to") {
            Some((x2, y2)) => ((x2 - x).abs(), (y2 - y).abs()),
            None => {
                let ext = anchor.child("ext");
                (
                    ext.and_then(|e| e.attr_i64("cx")).map(px).unwrap_or(480.0),
                    ext.and_then(|e| e.attr_i64("cy")).map(px).unwrap_or(288.0),
                )
            }
        };
        out.push(SheetChart {
            id: ai_format::ids::new_block_id(),
            x: x.round(),
            y: y.round(),
            w: w.round().max(120.0),
            h: h.round().max(80.0),
            spec,
        });
    }
    out
}

/// The range on `sheet_name` a chart is a view of, if it is a view of one.
///
/// Conservative on purpose: the range is used only when the chart names its
/// categories and they sit in the left-most column of the block, which is the
/// shape this format's own charts have. Anything else keeps the cached numbers,
/// which are never wrong — only frozen.
fn chart_range(package: &Package, part: &str, sheet_name: &str) -> Option<String> {
    let document = package.xml_opt(part)?;
    let plot = document.path(&["chartSpace", "chart", "plotArea"])?;

    let on_this_sheet = |node: &Node| -> Option<CellRange> {
        let text = node.descendants("f").first()?.all_text();
        let (sheet, local) = ai_formula::refs::split_sheet(&text);
        if !sheet?.eq_ignore_ascii_case(sheet_name) {
            return None;
        }
        let bare: String = local.chars().filter(|c| *c != '$').collect();
        // A series name is a single cell rather than a range, and it is the cell
        // that carries the header row into the block.
        parse_range(&bare).or_else(|| {
            parse_ref(&bare).map(|cell| CellRange {
                start: cell,
                end: cell,
            })
        })
    };

    let mut categories: Option<CellRange> = None;
    let mut values: Vec<CellRange> = Vec::new();
    let mut named = false;
    for series in plot.descendants("ser") {
        if let Some(node) = series.child("cat") {
            if let Some(range) = on_this_sheet(node) {
                categories = Some(range);
            }
        }
        if let Some(node) = series.child("tx") {
            if let Some(range) = on_this_sheet(node) {
                values.push(range);
                named = true;
            }
        }
        if let Some(node) = series.child("val") {
            if let Some(range) = on_this_sheet(node) {
                values.push(range);
            }
        }
    }

    let categories = categories?;
    // Without the series-name cell the block would start at the first data row,
    // and reading it back would take the data as the series names.
    if values.is_empty() || !named {
        return None;
    }
    let mut box_ = categories;
    for range in &values {
        box_.start.col = box_.start.col.min(range.start.col);
        box_.start.row = box_.start.row.min(range.start.row);
        box_.end.col = box_.end.col.max(range.end.col);
        box_.end.row = box_.end.row.max(range.end.row);
    }
    // The categories have to be the first column of the block, or reading the
    // block back would take the wrong column as the labels.
    if categories.start.col != box_.start.col || categories.end.col != box_.start.col {
        return None;
    }
    // And every column in between has to belong to the chart. A radar chart
    // plotting column C against categories in A would otherwise come back as a
    // block that includes B — a series the chart never had.
    for col in box_.start.col..=box_.end.col {
        let claimed = col == categories.start.col
            || values
                .iter()
                .any(|r| r.start.col <= col && col <= r.end.col);
        if !claimed {
            return None;
        }
    }
    Some(ai_formula::refs::range_to_string(&box_))
}

/// A shared formula moved from its master cell to `target`.
///
/// The same relative/absolute rules as filling a formula down a column, which is
/// exactly what the author did to create the group.
fn shifted_from(master: &str, target: &str, formula: &str) -> String {
    let (Some(from), Some(to)) = (parse_ref(master), parse_ref(target)) else {
        return format!("={formula}");
    };
    shift_formula(
        &format!("={formula}"),
        to.col as i64 - from.col as i64,
        to.row as i64 - from.row as i64,
    )
}

fn read_dims(cells: &IndexMap<String, Cell>) -> Dims {
    let mut rows = 0usize;
    let mut cols = 0usize;
    for reference in cells.keys() {
        if let Some(p) = parse_ref(reference) {
            rows = rows.max(p.row + 1);
            cols = cols.max(p.col + 1);
        }
    }
    // Keep the editor's usual breathing room below and to the right of the data.
    Dims {
        rows: (rows + 40).clamp(1, 100_000) as u32,
        cols: (cols + 4).clamp(1, 702) as u32,
    }
}

fn read_panes(worksheet: &Node) -> Frozen {
    let pane = worksheet
        .child("sheetViews")
        .and_then(|v| v.children_named("sheetView").next())
        .and_then(|v| v.child("pane"));
    let Some(pane) = pane else {
        return Frozen { rows: 0, cols: 0 };
    };
    // Only a frozen pane maps onto this format; a split pane is a view state.
    if !matches!(pane.attr("state"), Some("frozen" | "frozenSplit")) {
        return Frozen { rows: 0, cols: 0 };
    }
    Frozen {
        rows: pane.attr_i64("ySplit").unwrap_or(0).clamp(0, 20) as u32,
        cols: pane.attr_i64("xSplit").unwrap_or(0).clamp(0, 20) as u32,
    }
}

fn read_col_widths(worksheet: &Node) -> IndexMap<String, i64> {
    let mut out = IndexMap::new();
    let Some(cols) = worksheet.child("cols") else {
        return out;
    };
    for col in cols.children_named("col") {
        // A `col` covers a range, and Excel writes one entry for a whole run.
        let Some(min) = col.attr_i64("min") else {
            continue;
        };
        let max = col.attr_i64("max").unwrap_or(min).min(min + 701);
        let Some(width) = col.attr_f64("width") else {
            continue;
        };
        if !col.attr_bool("customWidth") && width <= 0.0 {
            continue;
        }
        let px = (width * PX_PER_CHAR).round() as i64;
        if px <= 0 {
            continue;
        }
        for index in min..=max {
            if index < 1 {
                continue;
            }
            out.insert(index_to_col((index - 1) as usize), px);
        }
    }
    out
}

fn read_row_heights(worksheet: &Node) -> IndexMap<String, i64> {
    let mut out = IndexMap::new();
    let Some(data) = worksheet.child("sheetData") else {
        return out;
    };
    for row in data.children_named("row") {
        let Some(index) = row.attr_i64("r") else {
            continue;
        };
        let Some(points) = row.attr_f64("ht") else {
            continue;
        };
        if !row.attr_bool("customHeight") {
            continue;
        }
        let px = (points / 0.75).round() as i64;
        if px > 0 {
            out.insert(index.to_string(), px);
        }
    }
    out
}

fn shared_strings(package: &Package) -> Vec<String> {
    let Some(root) = package.xml_opt("xl/sharedStrings.xml") else {
        return Vec::new();
    };
    let Some(sst) = root.child("sst") else {
        return Vec::new();
    };
    sst.children_named("si")
        .map(|si| {
            // A string with mixed formatting is split across several `r` runs;
            // the text is their concatenation.
            si.descendants("t")
                .iter()
                .map(|t| t.all_text())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

/// `(name, target, sheet scope)` for every defined name in the workbook.
fn defined_names(workbook: &Node) -> Vec<(String, String, Option<usize>)> {
    let Some(book) = workbook.child("workbook") else {
        return Vec::new();
    };
    let Some(container) = book.child("definedNames") else {
        return Vec::new();
    };
    container
        .children_named("definedName")
        .filter_map(|n| {
            let name = n.attr("name")?.to_string();
            // `_xlnm.*` names are Excel's own bookkeeping (print areas, titles).
            if name.starts_with("_xlnm") {
                return None;
            }
            let target = n.all_text().trim().to_string();
            let scope = n.attr("localSheetId").and_then(|s| s.parse::<usize>().ok());
            Some((name, target, scope))
        })
        .collect()
}

/* ------------------------------------------------------------------- styles */

#[derive(Default)]
struct Font {
    bold: bool,
    italic: bool,
    underline: bool,
    color: Option<String>,
    size_px: Option<f64>,
}

struct XfEntry {
    num_fmt: usize,
    font: usize,
    fill: usize,
    border: usize,
    align: Option<String>,
}

/// A conditional format's own formatting: the part of a cell style a rule
/// changes when it matches.
#[derive(Clone, Debug, Default, PartialEq)]
struct Dxf {
    color: Option<String>,
    bg: Option<String>,
    bold: bool,
    italic: bool,
}

impl Dxf {
    fn read(node: &Node) -> Dxf {
        let font = node.child("font");
        Dxf {
            color: font
                .and_then(|f| f.child("color"))
                .and_then(|c| c.attr("rgb"))
                .and_then(color),
            bg: node
                .path(&["fill", "patternFill"])
                .and_then(|p| p.child("bgColor").or_else(|| p.child("fgColor")))
                .and_then(|c| c.attr("rgb"))
                .and_then(color),
            bold: font.is_some_and(|f| f.child("b").is_some()),
            italic: font.is_some_and(|f| f.child("i").is_some()),
        }
    }

    fn is_empty(&self) -> bool {
        self.color.is_none() && self.bg.is_none() && !self.bold && !self.italic
    }

    /// Write this format onto a cell's own style.
    ///
    /// Conditional formatting sits on top of the cell's format in Excel, so it
    /// wins over what is already there.
    fn apply(&self, cell: &mut Cell) {
        let mut style = cell
            .extra
            .get("style")
            .and_then(|s| s.as_object().cloned())
            .unwrap_or_default();
        if let Some(color) = &self.color {
            style.insert("color".into(), json!(color));
        }
        if let Some(bg) = &self.bg {
            style.insert("bg".into(), json!(bg));
        }
        if self.bold {
            style.insert("bold".into(), json!(true));
        }
        if self.italic {
            style.insert("italic".into(), json!(true));
        }
        cell.extra
            .insert("style".into(), serde_json::Value::Object(style));
    }
}

#[derive(Default)]
struct Styles {
    num_fmts: IndexMap<usize, String>,
    fonts: Vec<Font>,
    fills: Vec<Option<String>>,
    /// The partial formats conditional formatting rules point at.
    dxfs: Vec<Dxf>,
    borders: Vec<Option<serde_json::Value>>,
    xfs: Vec<XfEntry>,
}

impl Styles {
    fn read(package: &Package) -> Styles {
        let mut styles = Styles::default();
        let Some(root) = package.xml_opt("xl/styles.xml") else {
            return styles;
        };
        let Some(sheet) = root.child("styleSheet") else {
            return styles;
        };

        if let Some(container) = sheet.child("numFmts") {
            for fmt in container.children_named("numFmt") {
                if let (Some(id), Some(code)) = (
                    fmt.attr("numFmtId").and_then(|s| s.parse().ok()),
                    fmt.attr("formatCode"),
                ) {
                    styles.num_fmts.insert(id, code.to_string());
                }
            }
        }

        if let Some(container) = sheet.child("fonts") {
            for font in container.children_named("font") {
                styles.fonts.push(Font {
                    bold: font.child("b").is_some(),
                    italic: font.child("i").is_some(),
                    underline: font.child("u").is_some(),
                    color: font
                        .child("color")
                        .and_then(|c| c.attr("rgb"))
                        .and_then(color),
                    size_px: font
                        .child("sz")
                        .and_then(|s| s.attr_f64("val"))
                        .map(|pt| px_from_half_point((pt * 2.0).round() as i64)),
                });
            }
        }

        if let Some(container) = sheet.child("dxfs") {
            for dxf in container.children_named("dxf") {
                styles.dxfs.push(Dxf::read(dxf));
            }
        }

        if let Some(container) = sheet.child("fills") {
            for fill in container.children_named("fill") {
                let pattern = fill.child("patternFill");
                let solid = pattern
                    .filter(|p| p.attr("patternType") == Some("solid"))
                    .and_then(|p| p.child("fgColor"))
                    .and_then(|c| c.attr("rgb"))
                    .and_then(color);
                styles.fills.push(solid);
            }
        }

        if let Some(container) = sheet.child("borders") {
            for border in container.children_named("border") {
                let edge = |name: &str| {
                    border
                        .child(name)
                        .is_some_and(|e| e.attr("style").is_some_and(|s| s != "none"))
                };
                let (t, b, l, r) = (edge("top"), edge("bottom"), edge("left"), edge("right"));
                styles.borders.push(if t || b || l || r {
                    Some(json!({ "t": t, "b": b, "l": l, "r": r }))
                } else {
                    None
                });
            }
        }

        if let Some(container) = sheet.child("cellXfs") {
            for xf in container.children_named("xf") {
                let index = |name: &str| {
                    xf.attr(name)
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0)
                };
                styles.xfs.push(XfEntry {
                    num_fmt: index("numFmtId"),
                    font: index("fontId"),
                    fill: index("fillId"),
                    border: index("borderId"),
                    align: xf
                        .child("alignment")
                        .and_then(|a| a.attr("horizontal"))
                        .filter(|h| matches!(*h, "left" | "center" | "right"))
                        .map(str::to_string),
                });
            }
        }
        styles
    }

    /// The format code for a `numFmtId`, custom or built-in.
    ///
    /// Excel leaves the built-ins implicit, so a currency column formatted with
    /// id 44 has no `formatCode` anywhere in the file — the codes have to be
    /// known here or the formatting is lost.
    fn num_fmt(&self, id: usize) -> Option<String> {
        if let Some(code) = self.num_fmts.get(&id) {
            return Some(code.clone());
        }
        Some(
            match id {
                0 => return None,
                1 => "0",
                2 => "0.00",
                3 => "#,##0",
                4 => "#,##0.00",
                9 => "0%",
                10 => "0.00%",
                11 => "0.00E+00",
                14 => "yyyy-mm-dd",
                15 => "d-mmm-yy",
                16 => "d-mmm",
                17 => "mmm-yy",
                18 => "h:mm AM/PM",
                19 => "h:mm:ss AM/PM",
                20 => "h:mm",
                21 => "h:mm:ss",
                22 => "yyyy-mm-dd h:mm",
                37 | 38 => "#,##0",
                39 | 40 => "#,##0.00",
                44 | 42 => "#,##0",
                43 | 41 => "#,##0.00",
                45 => "mm:ss",
                46 => "[h]:mm:ss",
                47 => "mm:ss.0",
                48 => "##0.0E+0",
                49 => "@",
                _ => return None,
            }
            .to_string(),
        )
    }
}
