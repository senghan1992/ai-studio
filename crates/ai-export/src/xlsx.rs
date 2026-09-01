//! `.xlsx` — formulas go across as formulas, so the file stays live in Excel.

use indexmap::IndexMap;
use serde_json::Value as Json;

use ai_format::chart::{describe_chart, resolve_spec, CellSource};
use ai_format::grid::{recalculated, used_range};
use ai_format::model::{Project, Sheet};
use ai_formula::evaluate::Cell;
use ai_formula::refs::{index_to_col, parse_ref, to_ref};

use crate::ooxml::{esc, hex, pt, relationships, Package, Result};

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// Excel column width is measured in characters, roughly px / 7.
const PX_PER_CHAR: f64 = 7.0;
const DEFAULT_COL_CHARS: i64 = 13;

/* --------------------------------------------------------------- registries */

/// Deduplicating table of style components, mapped to the `cellXfs` indices
/// Excel wants. Index 0 of every table is the built-in default.
#[derive(Default)]
struct Styles {
    num_fmts: Vec<String>,
    fonts: Vec<String>,
    fills: Vec<String>,
    borders: Vec<String>,
    xfs: Vec<Xf>,
}

#[derive(Default, PartialEq, Clone, Copy)]
struct Xf {
    num_fmt: usize,
    font: usize,
    fill: usize,
    border: usize,
    align: Option<Alignment>,
}

#[derive(PartialEq, Clone, Copy)]
enum Alignment {
    Left,
    Center,
    Right,
}

impl Alignment {
    fn as_str(self) -> &'static str {
        match self {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
        }
    }

    fn parse(s: &str) -> Option<Alignment> {
        Some(match s {
            "left" => Alignment::Left,
            "center" => Alignment::Center,
            "right" => Alignment::Right,
            _ => return None,
        })
    }
}

impl Styles {
    fn new() -> Self {
        Styles {
            num_fmts: Vec::new(),
            // The default font and the two fills Excel requires in slots 0 and 1.
            fonts: vec!["<font><sz val=\"11\"/><name val=\"Calibri\"/></font>".to_string()],
            fills: vec![
                "<fill><patternFill patternType=\"none\"/></fill>".to_string(),
                "<fill><patternFill patternType=\"gray125\"/></fill>".to_string(),
            ],
            borders: vec!["<border><left/><right/><top/><bottom/><diagonal/></border>".to_string()],
            xfs: vec![Xf::default()],
        }
    }

    /// Custom number formats start at 164; the lower ids are Excel's built-ins.
    fn num_fmt(&mut self, code: &str) -> usize {
        if code.is_empty() {
            return 0;
        }
        if let Some(at) = self.num_fmts.iter().position(|c| c == code) {
            return 164 + at;
        }
        self.num_fmts.push(code.to_string());
        164 + self.num_fmts.len() - 1
    }

    fn intern(list: &mut Vec<String>, xml: String) -> usize {
        match list.iter().position(|x| *x == xml) {
            Some(at) => at,
            None => {
                list.push(xml);
                list.len() - 1
            }
        }
    }

    fn font(&mut self, bold: bool, italic: bool, underline: bool, color: Option<&str>) -> usize {
        if !bold && !italic && !underline && color.is_none() {
            return 0;
        }
        let mut xml = String::from("<font>");
        if bold {
            xml.push_str("<b/>");
        }
        if italic {
            xml.push_str("<i/>");
        }
        if underline {
            xml.push_str("<u/>");
        }
        if let Some(color) = color {
            xml.push_str(&format!("<color rgb=\"FF{}\"/>", hex(color)));
        }
        xml.push_str("<sz val=\"11\"/><name val=\"Calibri\"/></font>");
        Self::intern(&mut self.fonts, xml)
    }

    fn fill(&mut self, bg: Option<&str>) -> usize {
        let Some(bg) = bg else { return 0 };
        let xml = format!(
            "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{}\"/><bgColor indexed=\"64\"/></patternFill></fill>",
            hex(bg)
        );
        Self::intern(&mut self.fills, xml)
    }

    fn border(&mut self, edges: Option<&Json>) -> usize {
        let Some(edges) = edges else { return 0 };
        let on = |key: &str| edges.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
        if !on("t") && !on("b") && !on("l") && !on("r") {
            return 0;
        }
        const THIN: &str = "<color rgb=\"FF9CA3AF\"/>";
        let edge = |name: &str, present: bool| {
            if present {
                format!("<{name} style=\"thin\">{THIN}</{name}>")
            } else {
                format!("<{name}/>")
            }
        };
        let xml = format!(
            "<border>{}{}{}{}<diagonal/></border>",
            edge("left", on("l")),
            edge("right", on("r")),
            edge("top", on("t")),
            edge("bottom", on("b")),
        );
        Self::intern(&mut self.borders, xml)
    }

    fn xf(&mut self, xf: Xf) -> usize {
        match self.xfs.iter().position(|x| *x == xf) {
            Some(at) => at,
            None => {
                self.xfs.push(xf);
                self.xfs.len() - 1
            }
        }
    }

    /// The style index for one cell, or 0 for a cell with no styling at all.
    fn for_cell(&mut self, cell: &Cell) -> usize {
        let style = cell.extra.get("style");
        let get_bool = |key: &str| {
            style
                .and_then(|s| s.get(key))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };
        let get_str = |key: &str| style.and_then(|s| s.get(key)).and_then(|v| v.as_str());

        let xf = Xf {
            num_fmt: self.num_fmt(&excel_num_fmt(cell.fmt.as_deref().unwrap_or(""))),
            font: self.font(
                get_bool("bold"),
                get_bool("italic"),
                get_bool("underline"),
                get_str("color"),
            ),
            fill: self.fill(get_str("bg")),
            border: self.border(style.and_then(|s| s.get("border"))),
            align: get_str("align").and_then(Alignment::parse),
        };
        if xf == Xf::default() {
            return 0;
        }
        self.xf(xf)
    }

    fn to_xml(&self) -> String {
        let mut out = format!("<styleSheet xmlns=\"{NS_MAIN}\">");

        out.push_str(&format!("<numFmts count=\"{}\">", self.num_fmts.len()));
        for (i, code) in self.num_fmts.iter().enumerate() {
            out.push_str(&format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                164 + i,
                esc(code)
            ));
        }
        out.push_str("</numFmts>");

        out.push_str(&format!("<fonts count=\"{}\">", self.fonts.len()));
        out.push_str(&self.fonts.concat());
        out.push_str("</fonts>");

        out.push_str(&format!("<fills count=\"{}\">", self.fills.len()));
        out.push_str(&self.fills.concat());
        out.push_str("</fills>");

        out.push_str(&format!("<borders count=\"{}\">", self.borders.len()));
        out.push_str(&self.borders.concat());
        out.push_str("</borders>");

        // Excel requires a cellStyleXfs entry before cellXfs can reference styles.
        out.push_str(
            "<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>",
        );

        out.push_str(&format!("<cellXfs count=\"{}\">", self.xfs.len()));
        for xf in &self.xfs {
            out.push_str(&format!(
                "<xf numFmtId=\"{}\" fontId=\"{}\" fillId=\"{}\" borderId=\"{}\" xfId=\"0\"{}{}{}{}",
                xf.num_fmt,
                xf.font,
                xf.fill,
                xf.border,
                if xf.num_fmt != 0 { " applyNumberFormat=\"1\"" } else { "" },
                if xf.font != 0 { " applyFont=\"1\"" } else { "" },
                if xf.fill != 0 { " applyFill=\"1\"" } else { "" },
                if xf.border != 0 { " applyBorder=\"1\"" } else { "" },
            ));
            match xf.align {
                Some(align) => out.push_str(&format!(
                    " applyAlignment=\"1\"><alignment horizontal=\"{}\"/></xf>",
                    align.as_str()
                )),
                None => out.push_str("/>"),
            }
        }
        out.push_str("</cellXfs>");

        out.push_str(
            "<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>",
        );
        out.push_str("</styleSheet>");
        out
    }
}

/// Deduplicating shared-string table.
#[derive(Default)]
struct SharedStrings {
    index: IndexMap<String, usize>,
    total: usize,
}

impl SharedStrings {
    fn intern(&mut self, text: &str) -> usize {
        self.total += 1;
        let next = self.index.len();
        *self.index.entry(text.to_string()).or_insert(next)
    }

    fn to_xml(&self) -> String {
        let mut out = format!(
            "<sst xmlns=\"{NS_MAIN}\" count=\"{}\" uniqueCount=\"{}\">",
            self.total,
            self.index.len()
        );
        for text in self.index.keys() {
            // `xml:space` matters: Excel trims leading and trailing spaces otherwise.
            out.push_str(&format!(
                "<si><t xml:space=\"preserve\">{}</t></si>",
                esc(text)
            ));
        }
        out.push_str("</sst>");
        out
    }
}

/// Our format strings are already Excel-compatible; date codes need lowering.
fn excel_num_fmt(fmt: &str) -> String {
    let has_date_letter = fmt
        .chars()
        .any(|c| matches!(c, 'y' | 'Y' | 'm' | 'M' | 'd' | 'D'));
    let has_digit_slot = fmt.contains('#') || fmt.contains('0');
    if has_date_letter && !has_digit_slot {
        fmt.to_lowercase()
    } else {
        fmt.to_string()
    }
}

/* -------------------------------------------------------------------- sheet */

/// One cell as it will be written, keyed by (row, col) so rows can be grouped.
struct Written {
    reference: String,
    xml: String,
}

fn cell_xml(reference: &str, cell: &Cell, style: usize, strings: &mut SharedStrings) -> String {
    let s_attr = if style == 0 {
        String::new()
    } else {
        format!(" s=\"{style}\"")
    };

    if cell.t.as_deref() == Some("e") {
        let code = cell.v.as_str().unwrap_or("#VALUE!");
        return match &cell.f {
            Some(f) => format!(
                "<c r=\"{reference}\"{s_attr} t=\"e\"><f>{}</f><v>{}</v></c>",
                esc(f.trim_start_matches('=')),
                esc(code)
            ),
            None => format!(
                "<c r=\"{reference}\"{s_attr} t=\"e\"><v>{}</v></c>",
                esc(code)
            ),
        };
    }

    // A formula cell carries its cached value so the sheet reads correctly
    // before Excel recalculates. Text results go inline as `t="str"`, because
    // the shared-string table is for literals only.
    let (type_attr, value_xml) = match (&cell.v, cell.f.is_some()) {
        (Json::Null, _) => ("", String::new()),
        (Json::Bool(b), _) => (" t=\"b\"", format!("<v>{}</v>", i32::from(*b))),
        (Json::Number(n), _) => (
            "",
            format!(
                "<v>{}</v>",
                ai_formula::values::format_plain_number(n.as_f64().unwrap_or(0.0))
            ),
        ),
        (Json::String(s), true) => (" t=\"str\"", format!("<v>{}</v>", esc(s))),
        (Json::String(s), false) => (" t=\"s\"", format!("<v>{}</v>", strings.intern(s))),
        (other, _) => (
            " t=\"s\"",
            format!("<v>{}</v>", strings.intern(&other.to_string())),
        ),
    };

    match &cell.f {
        Some(f) => format!(
            "<c r=\"{reference}\"{s_attr}{type_attr}><f>{}</f>{value_xml}</c>",
            esc(f.trim_start_matches('='))
        ),
        None if value_xml.is_empty() && s_attr.is_empty() => String::new(),
        None => format!("<c r=\"{reference}\"{s_attr}{type_attr}>{value_xml}</c>"),
    }
}

fn sheet_xml(
    sheet: &Sheet,
    extra_cells: &[(usize, usize, ExtraCell)],
    styles: &mut Styles,
    strings: &mut SharedStrings,
) -> String {
    let range = used_range(&sheet.cells);

    let mut rows: IndexMap<usize, Vec<Written>> = IndexMap::new();
    for (reference, cell) in &sheet.cells {
        let Some(p) = parse_ref(reference) else {
            continue;
        };
        let style = styles.for_cell(cell);
        let xml = cell_xml(reference, cell, style, strings);
        if xml.is_empty() {
            continue;
        }
        rows.entry(p.row).or_default().push(Written {
            reference: reference.clone(),
            xml,
        });
    }
    for (row, col, extra) in extra_cells {
        let reference = to_ref(*col, *row);
        let mut cell = Cell::default();
        match &extra.value {
            ExtraValue::Text(t) => {
                cell.v = Json::String(t.clone());
                cell.t = Some("s".into());
            }
            ExtraValue::Number(n) => {
                cell.v = serde_json::json!(n);
                cell.t = Some("n".into());
            }
        }
        if let Some(style) = &extra.style {
            cell.extra.insert("style".into(), style.clone());
        }
        let style = styles.for_cell(&cell);
        let xml = cell_xml(&reference, &cell, style, strings);
        rows.entry(*row)
            .or_default()
            .push(Written { reference, xml });
    }

    let mut out = format!("<worksheet xmlns=\"{NS_MAIN}\" xmlns:r=\"{REL}\">");

    // Freeze panes.
    let (fx, fy) = (sheet.frozen.cols, sheet.frozen.rows);
    out.push_str("<sheetViews><sheetView workbookViewId=\"0\"");
    if fx > 0 || fy > 0 {
        let top_left = to_ref(fx as usize, fy as usize);
        let pane_attr = match (fx > 0, fy > 0) {
            (true, true) => "bottomRight",
            (true, false) => "topRight",
            _ => "bottomLeft",
        };
        out.push_str(&format!(
            "><pane xSplit=\"{fx}\" ySplit=\"{fy}\" topLeftCell=\"{top_left}\" activePane=\"{pane_attr}\" state=\"frozen\"/><selection pane=\"{pane_attr}\" activeCell=\"{top_left}\" sqref=\"{top_left}\"/></sheetView>"
        ));
    } else {
        out.push_str("/>");
    }
    out.push_str("</sheetViews>");

    // Column widths.
    let cols = range.map(|r| r.max_col + 1).unwrap_or(0);
    if cols > 0 {
        out.push_str("<cols>");
        for c in 0..cols {
            let chars = match sheet.col_widths.get(&index_to_col(c)) {
                Some(px) => ((*px as f64 / PX_PER_CHAR).round() as i64).max(1),
                None => DEFAULT_COL_CHARS,
            };
            out.push_str(&format!(
                "<col min=\"{}\" max=\"{}\" width=\"{chars}\" customWidth=\"1\"/>",
                c + 1,
                c + 1
            ));
        }
        out.push_str("</cols>");
    }

    out.push_str("<sheetData>");
    let mut ordered: Vec<usize> = rows.keys().copied().collect();
    ordered.sort_unstable();
    for row in ordered {
        let mut cells = rows.swap_remove(&row).unwrap_or_default();
        cells.sort_by_key(|c| parse_ref(&c.reference).map(|p| p.col).unwrap_or(0));
        let height = sheet
            .row_heights
            .get(&(row + 1).to_string())
            .map(|px| format!(" ht=\"{}\" customHeight=\"1\"", pt(*px as f64).round()))
            .unwrap_or_default();
        out.push_str(&format!("<row r=\"{}\"{height}>", row + 1));
        for cell in cells {
            out.push_str(&cell.xml);
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData>");

    // Merges. An unparseable or single-cell spec is skipped rather than failing
    // the export — the same tolerance the rest of the format has.
    let merges: Vec<String> = sheet
        .merges
        .iter()
        .filter_map(|m| m.as_str())
        .filter(|spec| ai_formula::refs::parse_range(spec).is_some())
        .map(str::to_string)
        .collect();
    if !merges.is_empty() {
        out.push_str(&format!("<mergeCells count=\"{}\">", merges.len()));
        for spec in merges {
            out.push_str(&format!("<mergeCell ref=\"{}\"/>", esc(&spec)));
        }
        out.push_str("</mergeCells>");
    }

    out.push_str("</worksheet>");
    out
}

enum ExtraValue {
    Text(String),
    Number(f64),
}

struct ExtraCell {
    value: ExtraValue,
    style: Option<Json>,
}

/// Chart data blocks appended below the sheet.
///
/// A `.xlsx` written by hand could carry a real chart part, but a chart in this
/// format is a view of a *range*, and its range lives in the same sheet — so the
/// most useful thing to hand Excel is the numbers, labelled, with the chart's
/// description. The recipient inserts a native chart from them in two clicks.
fn chart_blocks(sheet: &Sheet) -> Vec<(usize, usize, ExtraCell)> {
    if sheet.charts.is_empty() {
        return Vec::new();
    }
    let bold = serde_json::json!({ "bold": true });
    let header_fill = serde_json::json!({ "bold": true, "bg": "#f1f5f9" });
    let caption = serde_json::json!({ "italic": true, "color": "#6b7280" });

    let mut out = Vec::new();
    let mut row = used_range(&sheet.cells).map(|r| r.max_row + 1).unwrap_or(0) + 3;

    for chart in &sheet.charts {
        let resolved = resolve_spec(&chart.spec, Some(sheet as &dyn CellSource));
        let title = if resolved.title.is_empty() {
            resolved.chart_type.label().to_string()
        } else {
            resolved.title.clone()
        };
        out.push((
            row,
            0,
            ExtraCell {
                value: ExtraValue::Text(format!("차트: {title}")),
                style: Some(bold.clone()),
            },
        ));
        row += 1;
        out.push((
            row,
            0,
            ExtraCell {
                value: ExtraValue::Text(describe_chart(&resolved)),
                style: Some(caption.clone()),
            },
        ));
        row += 1;

        out.push((
            row,
            0,
            ExtraCell {
                value: ExtraValue::Text("구분".to_string()),
                style: Some(header_fill.clone()),
            },
        ));
        for (i, series) in resolved.series.iter().enumerate() {
            let name = if series.name.is_empty() {
                format!("계열 {}", i + 1)
            } else {
                series.name.clone()
            };
            out.push((
                row,
                i + 1,
                ExtraCell {
                    value: ExtraValue::Text(name),
                    style: Some(header_fill.clone()),
                },
            ));
        }
        row += 1;

        let points = ai_format::chart::point_count(&resolved);
        for i in 0..points {
            let label = resolved
                .labels
                .get(i)
                .filter(|l| !l.is_empty())
                .cloned()
                .unwrap_or_else(|| (i + 1).to_string());
            out.push((
                row,
                0,
                ExtraCell {
                    value: ExtraValue::Text(label),
                    style: None,
                },
            ));
            for (c, series) in resolved.series.iter().enumerate() {
                if let Some(v) = series.values.get(i).copied().flatten() {
                    out.push((
                        row,
                        c + 1,
                        ExtraCell {
                            value: ExtraValue::Number(v),
                            style: None,
                        },
                    ));
                }
            }
            row += 1;
        }
        row += 2;
    }
    out
}

/// A sheet name Excel accepts: 31 characters, none of `[]:*?/\`.
fn safe_sheet_name(name: &str, index: usize) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !"[]:*?/\\".contains(*c))
        .take(31)
        .collect();
    let cleaned = cleaned.trim().trim_matches('\'').to_string();
    if cleaned.is_empty() {
        format!("시트{}", index + 1)
    } else {
        cleaned
    }
}

/// A defined name Excel accepts: letters, digits, underscore and period only,
/// and never something that looks like a cell reference.
fn safe_defined_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().next()?.is_ascii_digit() {
        return None;
    }
    if ai_formula::refs::parse_ref(&cleaned).is_some() {
        return None;
    }
    Some(cleaned)
}

/// Export a workbook to `.xlsx`.
pub fn export(project: &Project) -> Result<Vec<u8>> {
    let sheets: Vec<Sheet> = project.sheets().iter().map(recalculated).collect();
    let names: Vec<String> = sheets
        .iter()
        .enumerate()
        .map(|(i, s)| safe_sheet_name(&s.name, i))
        .collect();

    let mut styles = Styles::new();
    let mut strings = SharedStrings::default();
    let mut sheet_parts: Vec<String> = Vec::new();

    for sheet in &sheets {
        let extras = chart_blocks(sheet);
        sheet_parts.push(sheet_xml(sheet, &extras, &mut styles, &mut strings));
    }

    let mut pkg = Package::new();

    /* ----------------------------------------------------------- workbook */
    let mut workbook = format!("<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{REL}\"><sheets>");
    for (i, name) in names.iter().enumerate() {
        workbook.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
            esc(name),
            i + 1,
            i + 1
        ));
    }
    workbook.push_str("</sheets>");

    let mut defined: Vec<String> = Vec::new();
    for (i, sheet) in sheets.iter().enumerate() {
        for (name, target) in &sheet.names {
            let Some(safe) = safe_defined_name(name) else {
                continue;
            };
            let Json::String(target) = target else {
                continue;
            };
            if ai_formula::refs::parse_range(target).is_none()
                && ai_formula::refs::parse_ref(target).is_none()
            {
                continue;
            }
            // Absolute and sheet-qualified: a relative defined name means
            // something different to Excel.
            defined.push(format!(
                "<definedName name=\"{}\">'{}'!{}</definedName>",
                esc(&safe),
                esc(&names[i]),
                esc(&absolutize(target))
            ));
        }
    }
    if !defined.is_empty() {
        workbook.push_str(&format!(
            "<definedNames>{}</definedNames>",
            defined.concat()
        ));
    }
    workbook.push_str("</workbook>");

    /* -------------------------------------------------------------- parts */
    let mut rels: Vec<(String, &str, String)> = (0..sheets.len())
        .map(|i| {
            (
                format!("rId{}", i + 1),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet",
                format!("worksheets/sheet{}.xml", i + 1),
            )
        })
        .collect();
    rels.push((
        format!("rId{}", sheets.len() + 1),
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
        "styles.xml".to_string(),
    ));
    rels.push((
        format!("rId{}", sheets.len() + 2),
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings",
        "sharedStrings.xml".to_string(),
    ));

    let mut content_types = String::from(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
    );
    for i in 0..sheets.len() {
        content_types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
            i + 1
        ));
    }
    content_types.push_str("</Types>");

    pkg.add_xml("[Content_Types].xml", &content_types);
    pkg.add_xml(
        "_rels/.rels",
        &relationships(&[
            (
                "rId1".to_string(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
                "xl/workbook.xml".to_string(),
            ),
            (
                "rId2".to_string(),
                "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
                "docProps/core.xml".to_string(),
            ),
        ]),
    );
    pkg.add_xml(
        "docProps/core.xml",
        &crate::core_properties(&project.manifest),
    );
    pkg.add_xml("xl/workbook.xml", &workbook);
    pkg.add_xml("xl/_rels/workbook.xml.rels", &relationships(&rels));
    pkg.add_xml("xl/styles.xml", &styles.to_xml());
    pkg.add_xml("xl/sharedStrings.xml", &strings.to_xml());
    for (i, xml) in sheet_parts.iter().enumerate() {
        pkg.add_xml(&format!("xl/worksheets/sheet{}.xml", i + 1), xml);
    }

    pkg.finish()
}

/// `A1:D4` -> `$A$1:$D$4`, which is what a defined name must hold.
fn absolutize(range: &str) -> String {
    range
        .split(':')
        .map(|part| {
            let bare: String = part.chars().filter(|c| *c != '$').collect();
            match ai_formula::refs::parse_ref(&bare) {
                Some(r) => format!("${}${}", index_to_col(r.col), r.row + 1),
                None => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_lower_only_date_codes() {
        assert_eq!(excel_num_fmt("YYYY-MM-DD"), "yyyy-mm-dd");
        assert_eq!(excel_num_fmt("#,##0"), "#,##0");
        assert_eq!(excel_num_fmt("₩#,##0"), "₩#,##0");
        assert_eq!(excel_num_fmt("0.0%"), "0.0%");
    }

    #[test]
    fn sheet_names_are_sanitised() {
        assert_eq!(safe_sheet_name("예산", 0), "예산");
        assert_eq!(safe_sheet_name("a/b:c*d?e[f]", 0), "abcdef");
        assert_eq!(safe_sheet_name("", 2), "시트3");
        assert_eq!(safe_sheet_name(&"x".repeat(40), 0).len(), 31);
    }

    #[test]
    fn defined_names_are_sanitised_or_dropped() {
        assert_eq!(
            safe_defined_name("매출데이터").as_deref(),
            Some("매출데이터")
        );
        assert_eq!(safe_defined_name("my range!").as_deref(), Some("my_range_"));
        assert_eq!(safe_defined_name("2024"), None, "cannot start with a digit");
        assert_eq!(safe_defined_name("A1"), None, "looks like a cell reference");
        assert_eq!(safe_defined_name(""), None);
    }

    #[test]
    fn ranges_become_absolute() {
        assert_eq!(absolutize("A1:D4"), "$A$1:$D$4");
        assert_eq!(absolutize("$A$1:$D$4"), "$A$1:$D$4");
        assert_eq!(absolutize("B2"), "$B$2");
    }

    #[test]
    fn the_style_table_dedupes() {
        let mut styles = Styles::new();
        let bold = |bg: Option<&str>| Cell {
            extra: [(
                "style".to_string(),
                match bg {
                    Some(bg) => serde_json::json!({ "bold": true, "bg": bg }),
                    None => serde_json::json!({ "bold": true }),
                },
            )]
            .into_iter()
            .collect(),
            ..Cell::default()
        };
        let a = styles.for_cell(&bold(None));
        let b = styles.for_cell(&bold(None));
        let c = styles.for_cell(&bold(Some("#f1f5f9")));
        assert_eq!(a, b, "identical styling reuses one xf");
        assert_ne!(a, c);
        assert_eq!(
            styles.for_cell(&Cell::default()),
            0,
            "an unstyled cell uses xf 0"
        );
        assert!(styles.to_xml().contains("<b/>"));
    }
}
