//! Reading `.xlsx`.
//!
//! The strongest check available without a copy of Excel: export a workbook with
//! this project's own writer, read it back with the importer, and require the
//! two models to agree. Anything the writer emits and the reader drops shows up
//! immediately, in both directions.

use std::path::{Path, PathBuf};

use ai_export::{export, Format};
use ai_format::model::{Items, ProjectType, Sheet};
use ai_format::project::{create_project, save_project};
use ai_formula::evaluate::display_value;
use ai_import::ooxml::Package;

struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ai-studio-import-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Workspace(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Export a grid project and read it straight back.
fn round_trip(project: &ai_format::model::Project) -> Vec<Sheet> {
    let bytes = export(project, Format::Xlsx).unwrap();
    let package = Package::open(&bytes).unwrap();
    ai_import::xlsx::read(&package).unwrap()
}

#[test]
fn a_workbook_survives_a_full_round_trip() {
    let ws = Workspace::new("xlsxround");
    let project = create_project(ws.path(), ProjectType::Grid, "예산", true).unwrap();
    let Items::Sheets(original) = &project.items else {
        panic!()
    };

    let sheets = round_trip(&project);
    assert_eq!(sheets.len(), original.len());

    let before = ai_format::grid::recalculated(&original[0]);
    let after = &sheets[0];
    assert_eq!(after.name, before.name);

    // Every cell that had something now has the same thing.
    for (reference, cell) in &before.cells {
        let read = after
            .cells
            .get(reference)
            .unwrap_or_else(|| panic!("{reference} was dropped"));
        assert_eq!(read.f, cell.f, "{reference} formula");
        assert_eq!(read.fmt, cell.fmt, "{reference} number format");
        assert_eq!(
            display_value(read),
            display_value(cell),
            "{reference} displays differently"
        );
    }

    assert_eq!(after.frozen, before.frozen, "frozen panes");
}

#[test]
fn formulas_arrive_as_formulas_not_values() {
    let ws = Workspace::new("xlsxformula");
    let project = create_project(ws.path(), ProjectType::Grid, "수식", true).unwrap();
    let sheets = round_trip(&project);

    assert_eq!(sheets[0].cells["D2"].f.as_deref(), Some("=B2+C2"));
    assert_eq!(sheets[0].cells["B5"].f.as_deref(), Some("=SUM(B2:B4)"));
    // And the sheet still computes: recalculating from the formulas alone gives
    // the same numbers.
    let recalculated = ai_format::grid::recalculated(&sheets[0]);
    assert_eq!(display_value(&recalculated.cells["B5"]), "2,820");
    assert_eq!(display_value(&recalculated.cells["D5"]), "6,180");
}

#[test]
fn styles_and_formats_come_back() {
    let ws = Workspace::new("xlsxstyle");
    let project = create_project(ws.path(), ProjectType::Grid, "서식", true).unwrap();
    let sheets = round_trip(&project);

    let header = &sheets[0].cells["A1"];
    let style = header
        .extra
        .get("style")
        .expect("the header keeps its styling");
    assert_eq!(style.get("bold").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        style.get("bg").and_then(|v| v.as_str()),
        Some("#f1f5f9"),
        "fill colour"
    );
    assert_eq!(sheets[0].cells["B2"].fmt.as_deref(), Some("#,##0"));
}

#[test]
fn merges_column_widths_and_named_ranges_come_back() {
    let ws = Workspace::new("xlsxlayout");
    let mut project = create_project(ws.path(), ProjectType::Grid, "배치", true).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    sheets[0].merges = vec![serde_json::json!("A1:B1")];
    sheets[0].col_widths.insert("A".into(), 210);
    sheets[0].row_heights.insert("1".into(), 40);
    sheets[0]
        .names
        .insert("매출데이터".into(), serde_json::json!("A1:D5"));
    let project = save_project(&project).unwrap();

    let read = round_trip(&project);
    assert_eq!(read[0].merges, vec![serde_json::json!("A1:B1")]);
    // 210px -> 30 characters -> 210px.
    assert_eq!(read[0].col_widths.get("A"), Some(&210));
    assert_eq!(read[0].row_heights.get("1"), Some(&40));
    assert_eq!(
        read[0].names.get("매출데이터").and_then(|v| v.as_str()),
        Some("A1:D5")
    );
}

#[test]
fn an_error_cell_stays_an_error() {
    let ws = Workspace::new("xlsxerror");
    let mut project = create_project(ws.path(), ProjectType::Grid, "오류", false).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    sheets[0].cells.insert(
        "A1".into(),
        ai_formula::evaluate::Cell {
            f: Some("=1/0".into()),
            ..Default::default()
        },
    );
    let project = save_project(&project).unwrap();

    let read = round_trip(&project);
    assert_eq!(read[0].cells["A1"].t.as_deref(), Some("e"));
    assert_eq!(display_value(&read[0].cells["A1"]), "#DIV/0!");
}

#[test]
fn a_date_cell_is_recognised_by_its_format() {
    let ws = Workspace::new("xlsxdate");
    let mut project = create_project(ws.path(), ProjectType::Grid, "날짜", false).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    sheets[0].cells.insert(
        "A1".into(),
        ai_formula::evaluate::Cell {
            v: serde_json::json!(46266),
            t: Some("d".into()),
            fmt: Some("yyyy-mm-dd".into()),
            ..Default::default()
        },
    );
    let project = save_project(&project).unwrap();

    let read = round_trip(&project);
    let cell = &read[0].cells["A1"];
    assert_eq!(
        cell.t.as_deref(),
        Some("d"),
        "the date format made it a date"
    );
    assert_eq!(display_value(cell), "2026-09-01");
}

#[test]
fn several_sheets_keep_their_order_and_names() {
    let ws = Workspace::new("xlsxsheets");
    let mut project = create_project(ws.path(), ProjectType::Grid, "여러 시트", false).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    sheets[0].name = "첫째".into();
    sheets.push(ai_format::grid::make_sheet("둘째", true));
    sheets.push(ai_format::grid::make_sheet("셋째", false));
    let project = save_project(&project).unwrap();

    let read = round_trip(&project);
    let names: Vec<&str> = read.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["첫째", "둘째", "셋째"]);
    assert!(!read[1].cells.is_empty(), "the middle sheet kept its data");
}

#[test]
fn built_in_number_formats_are_recognised_without_a_format_code() {
    // Excel leaves ids under 164 implicit, so a file from Excel has no
    // `formatCode` for them. Hand-build the minimal package to prove the reader
    // does not need one.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData>
            <row r="1"><c r="A1" s="1"><v>1234.5</v></c><c r="B1" s="2"><v>0.42</v></c>
                       <c r="C1" s="3"><v>46266</v></c></row>
          </sheetData>
        </worksheet>"#;
    let styles = r#"<?xml version="1.0"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <cellXfs count="4">
            <xf numFmtId="0"/><xf numFmtId="4"/><xf numFmtId="9"/><xf numFmtId="14"/>
          </cellXfs>
        </styleSheet>"#;
    let bytes = tiny_workbook(sheet, styles);
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();

    assert_eq!(sheets[0].cells["A1"].fmt.as_deref(), Some("#,##0.00"));
    assert_eq!(display_value(&sheets[0].cells["A1"]), "1,234.50");
    assert_eq!(display_value(&sheets[0].cells["B1"]), "42%");
    assert_eq!(sheets[0].cells["C1"].t.as_deref(), Some("d"));
    assert_eq!(display_value(&sheets[0].cells["C1"]), "2026-09-01");
}

#[test]
fn a_shared_string_split_across_runs_is_joined() {
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>
        </worksheet>"#;
    let shared = r#"<?xml version="1.0"?>
        <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">
          <si><r><t>매출 </t></r><r><rPr><b/></rPr><t>142억</t></r></si>
        </sst>"#;
    let mut bytes = tiny_workbook(sheet, "<styleSheet/>");
    bytes = with_part(&bytes, "xl/sharedStrings.xml", shared);
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    assert_eq!(display_value(&sheets[0].cells["A1"]), "매출 142억");
}

/* ------------------------------------------------------------------ fixtures */

/// The smallest package the reader will accept, for tests about one detail.
fn tiny_workbook(sheet: &str, styles: &str) -> Vec<u8> {
    let workbook = r#"<?xml version="1.0"?>
        <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
          <sheets><sheet name="시트1" sheetId="1" r:id="rId1"/></sheets>
        </workbook>"#;
    let rels = r#"<?xml version="1.0"?>
        <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1"
            Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
            Target="worksheets/sheet1.xml"/>
        </Relationships>"#;

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for (path, body) in [
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/styles.xml", styles),
            ("xl/worksheets/sheet1.xml", sheet),
        ] {
            zip.start_file(path, options).unwrap();
            std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

/// Add one more part to an existing package.
/// A package from an explicit list of parts, for the files that need more than
/// one worksheet or a drawing.
fn fixture_zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for (path, body) in parts {
            zip.start_file(*path, options).unwrap();
            std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

fn with_part(bytes: &[u8], path: &str, body: &str) -> Vec<u8> {
    let mut source = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for i in 0..source.len() {
            let mut file = source.by_index(i).unwrap();
            let name = file.name().to_string();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut data).unwrap();
            zip.start_file(name, options).unwrap();
            std::io::Write::write_all(&mut zip, &data).unwrap();
        }
        zip.start_file(path, options).unwrap();
        std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

#[test]
fn a_cell_font_size_survives_the_round_trip() {
    // A workbook whose author made the title row large. Drawing it at the
    // sheet's own size is the difference a reader sees first.
    let ws = Workspace::new("xlsxsize");
    let mut project = create_project(ws.path(), ProjectType::Grid, "글꼴 크기", true).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    let cell = sheets[0].cells.get_mut("A1").expect("a header cell");
    cell.extra.insert(
        "style".into(),
        serde_json::json!({ "bold": true, "fontSize": 21 }),
    );
    let project = save_project(&project).unwrap();

    let read = round_trip(&project);
    let style = read[0].cells["A1"].extra.get("style").expect("styling");
    assert_eq!(
        style.get("fontSize").and_then(|v| v.as_f64()),
        Some(21.0),
        "16pt is 21.33px: {style}"
    );
}

#[test]
fn vertical_alignment_and_wrapping_survive_the_round_trip() {
    // A cell centred vertically with wrapped text — the reader used to keep only
    // the horizontal alignment, so re-opening the sheet flattened it to the
    // default bottom-aligned, single-line cell.
    let ws = Workspace::new("xlsxwrap");
    let mut project = create_project(ws.path(), ProjectType::Grid, "정렬", true).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    let cell = sheets[0].cells.get_mut("A1").expect("a header cell");
    cell.extra.insert(
        "style".into(),
        serde_json::json!({ "valign": "middle", "wrap": true }),
    );
    let project = save_project(&project).unwrap();

    let read = round_trip(&project);
    let style = read[0].cells["A1"].extra.get("style").expect("styling");
    assert_eq!(
        style.get("valign").and_then(|v| v.as_str()),
        Some("middle"),
        "vertical centring kept: {style}"
    );
    assert_eq!(
        style.get("wrap").and_then(|v| v.as_bool()),
        Some(true),
        "text wrapping kept: {style}"
    );
}

#[test]
fn a_cell_at_the_sheets_own_size_carries_no_size() {
    // 11pt is what the grid already draws, so every cell in a plain workbook
    // would otherwise arrive carrying a style that changes nothing.
    let ws = Workspace::new("xlsxnosize");
    let project = create_project(ws.path(), ProjectType::Grid, "기본 크기", true).unwrap();
    let read = round_trip(&project);
    for (reference, cell) in &read[0].cells {
        let size = cell
            .extra
            .get("style")
            .and_then(|s| s.get("fontSize"))
            .and_then(|v| v.as_f64());
        assert_eq!(size, None, "{reference} gained a font size");
    }
}

#[test]
fn a_shared_formula_is_resolved_for_every_cell_it_covers() {
    // Excel writes a dragged formula once, on the top-left cell of the group,
    // and leaves the rest with an empty `<f t="shared" si="0"/>`. Reading only
    // the text loses the formula from every cell the author filled — which, in a
    // real workbook, is most of them.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData>
            <row r="1"><c r="A1"><v>2</v></c><c r="B1"><v>1000</v></c>
              <c r="C1"><f t="shared" ref="C1:C3" si="0">A1*B1</f><v>2000</v></c></row>
            <row r="2"><c r="A2"><v>3</v></c><c r="B2"><v>1500</v></c>
              <c r="C2"><f t="shared" si="0"/><v>4500</v></c></row>
            <row r="3"><c r="A3"><v>4</v></c><c r="B3"><v>2000</v></c>
              <c r="C3"><f t="shared" si="0"/><v>8000</v></c></row>
          </sheetData>
        </worksheet>"#;
    let bytes = tiny_workbook(sheet, "");
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    assert_eq!(sheets[0].cells["C1"].f.as_deref(), Some("=A1*B1"));
    assert_eq!(
        sheets[0].cells["C2"].f.as_deref(),
        Some("=A2*B2"),
        "shifted off the master, the way filling down shifts it"
    );
    assert_eq!(sheets[0].cells["C3"].f.as_deref(), Some("=A3*B3"));

    // And the sheet still computes to the same numbers.
    let again = ai_format::grid::recalculated(&sheets[0]);
    assert_eq!(display_value(&again.cells["C3"]), "8000");
}

#[test]
fn a_formula_reaching_another_sheet_is_recalculated_from_it() {
    let mut project_sheets = Vec::new();
    let data = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1"><c r="A1"><v>100</v></c><c r="B1"><v>200</v></c></row></sheetData>
        </worksheet>"#;
    let summary = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1">
            <c r="A1"><f>실적!A1+실적!B1</f><v>0</v></c>
            <c r="A2"><f>SUM('실적'!A1:B1)</f><v>0</v></c>
          </row></sheetData>
        </worksheet>"#;
    let bytes = fixture_zip(&[
        ("xl/worksheets/sheet1.xml", data),
        ("xl/worksheets/sheet2.xml", summary),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="실적" sheetId="1" r:id="rId1"/><sheet name="요약" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
    ]);
    let package = Package::open(&bytes).unwrap();
    project_sheets.extend(ai_import::xlsx::read(&package).unwrap());

    let recalculated = ai_format::grid::recalculated_all(&project_sheets);
    let summary = recalculated.iter().find(|s| s.name == "요약").unwrap();
    assert_eq!(display_value(&summary.cells["A1"]), "300");
    assert_eq!(display_value(&summary.cells["A2"]), "300");
}

#[test]
fn a_chart_on_a_sheet_comes_across_pointing_at_its_range() {
    // The chart part, its drawing and the anchor that places it — the three
    // parts a workbook's chart is spread over.
    let chart = r#"<?xml version="1.0"?>
        <c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>분기별 매출</a:t></a:r></a:p></c:rich></c:tx></c:title>
            <c:plotArea><c:barChart><c:barDir val="col"/>
              <c:ser>
                <c:tx><c:strRef><c:f>'실적'!$B$1</c:f></c:strRef></c:tx>
                <c:cat><c:strRef><c:f>'실적'!$A$2:$A$3</c:f></c:strRef></c:cat>
                <c:val><c:numRef><c:f>'실적'!$B$2:$B$3</c:f>
                  <c:numCache><c:pt idx="0"><c:v>1200</c:v></c:pt><c:pt idx="1"><c:v>1500</c:v></c:pt></c:numCache>
                </c:numRef></c:val>
              </c:ser>
            </c:barChart></c:plotArea>
          </c:chart>
        </c:chartSpace>"#;
    let drawing = r#"<?xml version="1.0"?>
        <xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
                  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
          <xdr:twoCellAnchor>
            <xdr:from><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
            <xdr:to><xdr:col>10</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>11</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
            <xdr:graphicFrame><a:graphic><a:graphicData><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart"/></a:graphicData></a:graphic></xdr:graphicFrame>
          </xdr:twoCellAnchor>
        </xdr:wsDr>"#;
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                   xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
          <sheetData>
            <row r="1"><c r="A1" t="inlineStr"><is><t>분기</t></is></c><c r="B1" t="inlineStr"><is><t>매출</t></is></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>Q1</t></is></c><c r="B2"><v>1200</v></c></row>
            <row r="3"><c r="A3" t="inlineStr"><is><t>Q2</t></is></c><c r="B3"><v>1500</v></c></row>
          </sheetData>
          <drawing r:id="rIdDraw"/>
        </worksheet>"#;

    let bytes = fixture_zip(&[
        ("xl/worksheets/sheet1.xml", sheet),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdDraw" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#,
        ),
        ("xl/drawings/drawing1.xml", drawing),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#,
        ),
        ("xl/charts/chart1.xml", chart),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="실적" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
    ]);

    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    let charts = &sheets[0].charts;
    assert_eq!(charts.len(), 1, "the chart came across");
    assert_eq!(charts[0].spec.title, "분기별 매출");
    assert_eq!(
        charts[0].spec.range.as_deref(),
        Some("A1:B3"),
        "a chart in a workbook stays a view of its cells"
    );
    assert!(charts[0].x > 0.0 && charts[0].w > 100.0, "placed and sized");
    // The cached numbers are kept as well, so a range that stops resolving still
    // draws something.
    assert_eq!(charts[0].spec.series.len(), 1);
}

#[test]
fn conditional_formatting_becomes_the_formatting_it_was_showing() {
    // A rule cannot stay a rule in this format, so it is applied against the
    // values the file was saved with. The red negatives are red, the heat map is
    // a heat map, and the sheet looks like the one the author saw.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData>
            <row r="1"><c r="A1" t="inlineStr"><is><t>항목</t></is></c><c r="B1"><v>120</v></c><c r="C1"><v>40</v></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>원가</t></is></c><c r="B2"><v>-45</v></c><c r="C2"><v>90</v></c></row>
          </sheetData>
          <conditionalFormatting sqref="B1:B2">
            <cfRule type="cellIs" dxfId="0" priority="1" operator="lessThan"><formula>0</formula></cfRule>
          </conditionalFormatting>
          <conditionalFormatting sqref="C1:C2">
            <cfRule type="colorScale" priority="2">
              <colorScale>
                <cfvo type="min"/><cfvo type="max"/>
                <color rgb="FFF8696B"/><color rgb="FF63BE7B"/>
              </colorScale>
            </cfRule>
          </conditionalFormatting>
          <conditionalFormatting sqref="A1:A2">
            <cfRule type="expression" dxfId="1" priority="3"><formula>$B1&gt;100</formula></cfRule>
          </conditionalFormatting>
        </worksheet>"#;
    let styles = r#"<?xml version="1.0"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <dxfs count="2">
            <dxf><font><color rgb="FF9C0006"/></font>
              <fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf>
            <dxf><font><b/></font></dxf>
          </dxfs>
        </styleSheet>"#;

    let bytes = tiny_workbook(sheet, styles);
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    let style = |reference: &str| {
        sheets[0].cells[reference]
            .extra
            .get("style")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };

    // The negative is red on pink; the positive beside it is untouched.
    assert_eq!(style("B2")["color"], serde_json::json!("#9c0006"));
    assert_eq!(style("B2")["bg"], serde_json::json!("#ffc7ce"));
    assert_eq!(style("B1"), serde_json::Value::Null);

    // The two ends of the colour scale keep their own colours.
    assert_eq!(
        style("C1")["bg"],
        serde_json::json!("#f8696b"),
        "the low end"
    );
    assert_eq!(
        style("C2")["bg"],
        serde_json::json!("#63be7b"),
        "the high end"
    );

    // A formula rule is moved onto each cell the way filling it down moves it,
    // so only the row whose B is over 100 is bold.
    assert_eq!(style("A1")["bold"], serde_json::json!(true));
    assert_eq!(style("A2"), serde_json::Value::Null);
}

#[test]
fn a_colour_scale_interpolates_between_its_stops() {
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1">
            <c r="A1"><v>0</v></c><c r="B1"><v>50</v></c><c r="C1"><v>100</v></c>
          </row></sheetData>
          <conditionalFormatting sqref="A1:C1">
            <cfRule type="colorScale" priority="1"><colorScale>
              <cfvo type="min"/><cfvo type="max"/>
              <color rgb="FF000000"/><color rgb="FFFFFFFF"/>
            </colorScale></cfRule>
          </conditionalFormatting>
        </worksheet>"#;
    let bytes = tiny_workbook(sheet, "");
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    let bg = |reference: &str| {
        sheets[0].cells[reference].extra["style"]["bg"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(bg("A1"), "#000000");
    assert_eq!(bg("B1"), "#808080", "halfway between the stops");
    assert_eq!(bg("C1"), "#ffffff");
}

#[test]
fn data_bars_are_reported_rather_than_painted() {
    // A bar drawn inside a cell has no equivalent, and filling the cell instead
    // would say something the file does not.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1"><c r="A1"><v>10</v></c></row></sheetData>
          <conditionalFormatting sqref="A1:A1">
            <cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar></cfRule>
          </conditionalFormatting>
        </worksheet>"#;
    let bytes = tiny_workbook(sheet, "");
    let package = Package::open(&bytes).unwrap();
    let mut warnings = ai_import::Warnings::default();
    let sheets = ai_import::xlsx::read_with(&package, &mut warnings).unwrap();
    assert!(sheets[0].cells["A1"].extra.get("style").is_none());
    let warnings = warnings.into_vec();
    assert!(
        warnings.iter().any(|w| w.contains("데이터 막대")),
        "{warnings:?}"
    );
}

#[test]
fn a_spill_survives_the_round_trip() {
    use ai_formula::evaluate::Cell;

    let ws = Workspace::new("spill");
    let created = create_project(ws.path(), ProjectType::Grid, "스필", false).unwrap();
    let mut project = ai_format::project::load_project(&created.dir).unwrap();

    if let Items::Sheets(sheets) = &mut project.items {
        let mut cells: indexmap::IndexMap<String, Cell> = indexmap::IndexMap::new();
        for (reference, text) in [("A1", "서울"), ("A2", "부산"), ("A3", "서울")] {
            cells.insert(
                reference.to_string(),
                Cell {
                    v: serde_json::json!(text),
                    t: Some("s".into()),
                    ..Cell::default()
                },
            );
        }
        cells.insert(
            "C1".into(),
            Cell {
                f: Some("=UNIQUE(A1:A3)".into()),
                ..Cell::default()
            },
        );
        sheets[0].cells = cells;
    }
    // Saving recalculates, which spills the array before it is exported.
    let project = ai_format::project::save_project(&project).unwrap();

    let sheets = round_trip(&project);
    let cells = &sheets[0].cells;
    assert_eq!(cells["C1"].spill.as_deref(), Some("C1:C2"), "{cells:?}");
    assert_eq!(cells["C1"].f.as_deref(), Some("=UNIQUE(A1:A3)"));
    assert_eq!(cells["C2"].spill_from.as_deref(), Some("C1"));
    assert_eq!(cells["C2"].v, serde_json::json!("부산"));
    assert!(cells["C2"].f.is_none(), "a spilled cell has no formula");
}

#[test]
fn a_spill_reference_survives_the_round_trip_as_anchorarray() {
    use ai_formula::evaluate::Cell;

    let ws = Workspace::new("spillref");
    let created = create_project(ws.path(), ProjectType::Grid, "스필 참조", false).unwrap();
    let mut project = ai_format::project::load_project(&created.dir).unwrap();

    if let Items::Sheets(sheets) = &mut project.items {
        let mut cells: indexmap::IndexMap<String, Cell> = indexmap::IndexMap::new();
        for (reference, n) in [("A1", 3.0), ("A2", 1.0), ("A3", 3.0)] {
            cells.insert(
                reference.to_string(),
                Cell {
                    v: serde_json::json!(n),
                    t: Some("n".into()),
                    ..Cell::default()
                },
            );
        }
        cells.insert(
            "C1".into(),
            Cell {
                f: Some("=UNIQUE(A1:A3)".into()),
                ..Cell::default()
            },
        );
        cells.insert(
            "E1".into(),
            Cell {
                f: Some("=SUM(C1#)".into()),
                ..Cell::default()
            },
        );
        sheets[0].cells = cells;
    }
    let project = ai_format::project::save_project(&project).unwrap();

    // On the way out, `C1#` is stored the way Excel stores it…
    let bytes = ai_export::export(&project, ai_export::Format::Xlsx).unwrap();
    let text = {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut s = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
    assert!(text.contains("SUM(_xlfn.ANCHORARRAY(C1))"), "{text}");
    assert!(
        !text.contains("C1#"),
        "the raw # form never reaches the file"
    );

    // …and on the way back it is `C1#` again, with the value intact.
    let sheets = round_trip(&project);
    let cells = &sheets[0].cells;
    assert_eq!(cells["E1"].f.as_deref(), Some("=SUM(C1#)"));
    assert_eq!(cells["E1"].v, serde_json::json!(4.0));
}

#[test]
fn a_cross_sheet_named_range_survives_the_round_trip() {
    use ai_formula::evaluate::Cell;
    use serde_json::json;

    let ws = Workspace::new("xname");
    let created = create_project(ws.path(), ProjectType::Grid, "이름범위", false).unwrap();
    let mut project = ai_format::project::load_project(&created.dir).unwrap();

    if let Items::Sheets(sheets) = &mut project.items {
        sheets[0].name = "실적".into();
        for (reference, n) in [("B2", 100.0), ("B3", 250.0)] {
            sheets[0].cells.insert(
                reference.to_string(),
                Cell {
                    v: json!(n),
                    t: Some("n".into()),
                    ..Cell::default()
                },
            );
        }
        let mut summary = ai_format::grid::make_sheet("요약", false);
        summary.cells.insert(
            "B1".into(),
            Cell {
                f: Some("=SUM(실적범위)".into()),
                ..Cell::default()
            },
        );
        // The name points into the other sheet — the shape FORMAT.md documents.
        summary.names.insert("실적범위".into(), json!("실적!B2:B3"));
        // …and, as an import would have it, the first sheet carries it too.
        sheets[0]
            .names
            .insert("실적범위".into(), json!("실적!B2:B3"));
        sheets.push(summary);
    }
    let project = ai_format::project::save_project(&project).unwrap();

    // The name reaches the workbook exactly once, owned by the right sheet.
    let bytes = ai_export::export(&project, ai_export::Format::Xlsx).unwrap();
    let workbook = {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut s = String::new();
        zip.by_name("xl/workbook.xml")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
    assert_eq!(
        workbook.matches("definedName name=\"실적범위\"").count(),
        1,
        "{workbook}"
    );
    assert!(workbook.contains("'실적'!$B$2:$B$3"), "{workbook}");

    // And coming back, the summary still computes rather than #NAME?.
    let sheets = round_trip(&project);
    let summary = sheets.iter().find(|s| s.name == "요약").expect("요약 시트");
    assert_eq!(
        summary.cells["B1"].v,
        json!(350.0),
        "{:?}",
        summary.cells["B1"]
    );
}

#[test]
fn a_font_that_states_no_underline_is_not_underlined() {
    // Apache POI writes `<u val="none"/>` on every font it emits, and Excel
    // writes `<b val="0"/>` to switch bold off. Reading presence as "on" put a
    // line under every cell of a real company workbook — and exported it.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData>
            <row r="1"><c r="A1" s="1" t="inlineStr"><is><t>없음</t></is></c>
                       <c r="B1" s="2" t="inlineStr"><is><t>있음</t></is></c></row>
          </sheetData>
        </worksheet>"#;
    let styles = r#"<?xml version="1.0"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <fonts count="3">
            <font><sz val="11"/></font>
            <font><b val="0"/><i val="false"/><u val="none"/><sz val="10"/></font>
            <font><b/><u/><sz val="10"/></font>
          </fonts>
          <cellXfs count="3">
            <xf numFmtId="0" fontId="0"/><xf numFmtId="0" fontId="1"/><xf numFmtId="0" fontId="2"/>
          </cellXfs>
        </styleSheet>"#;
    let bytes = tiny_workbook(sheet, styles);
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();

    let a1 = sheets[0].cells["A1"].extra.get("style");
    let flag = |style: Option<&serde_json::Value>, key: &str| {
        style.and_then(|s| s.get(key)).and_then(|v| v.as_bool())
    };
    assert_eq!(
        flag(a1, "underline"),
        None,
        "val=none is no underline: {a1:?}"
    );
    assert_eq!(flag(a1, "bold"), None, "val=0 is not bold: {a1:?}");
    assert_eq!(flag(a1, "italic"), None, "val=false is not italic: {a1:?}");
    let b1 = sheets[0].cells["B1"].extra.get("style");
    assert_eq!(flag(b1, "underline"), Some(true), "a bare <u/> still is");
    assert_eq!(flag(b1, "bold"), Some(true));
}

#[test]
fn a_dotted_light_border_keeps_its_line_and_colour_through_the_round_trip() {
    // A company data template draws its grid with a light dotted right edge
    // and a thin bottom edge. Both used to come out as the same dark solid
    // line; the sheet's whole look changed on export.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>항목</t></is></c></row></sheetData>
        </worksheet>"#;
    let styles = r#"<?xml version="1.0"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <borders count="2">
            <border><left/><right/><top/><bottom/></border>
            <border><left/><right style="dotted"><color rgb="FFE5E5E5"/></right><top/>
                    <bottom style="thin"/></border>
          </borders>
          <cellXfs count="2"><xf numFmtId="0" borderId="0"/><xf numFmtId="0" borderId="1"/></cellXfs>
        </styleSheet>"#;
    let bytes = tiny_workbook(sheet, styles);
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    let border = sheets[0].cells["A1"].extra["style"]["border"].clone();
    assert_eq!(
        border,
        serde_json::json!({ "b": true, "r": { "style": "dotted", "color": "#e5e5e5" } }),
        "a plain thin edge stays `true`; a styled one carries style and colour"
    );

    // And back out through the exporter unchanged.
    let ws = Workspace::new("xlsxborder");
    let mut project = create_project(ws.path(), ProjectType::Grid, "테두리", true).unwrap();
    let Items::Sheets(project_sheets) = &mut project.items else {
        panic!()
    };
    let cell = project_sheets[0]
        .cells
        .get_mut("A1")
        .expect("a header cell");
    cell.extra
        .insert("style".into(), serde_json::json!({ "border": border }));
    let project = save_project(&project).unwrap();
    let read = round_trip(&project);
    assert_eq!(
        read[0].cells["A1"].extra["style"]["border"],
        serde_json::json!({ "b": true, "r": { "style": "dotted", "color": "#e5e5e5" } })
    );
}

#[test]
fn the_authors_font_name_rides_along_to_the_export() {
    // The screen draws Pretendard, but a workbook written in 맑은 고딕 must ask
    // Excel for 맑은 고딕 again on export — Pretendard is not on a company PC.
    let sheet = r#"<?xml version="1.0"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>항목</t></is></c>
                                <c r="B1" s="2" t="inlineStr"><is><t>기본</t></is></c></row></sheetData>
        </worksheet>"#;
    let styles = r#"<?xml version="1.0"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <fonts count="3">
            <font><sz val="11"/><name val="Calibri"/></font>
            <font><sz val="10"/><name val="맑은 고딕"/></font>
            <font><sz val="10"/><name val="Pretendard"/></font>
          </fonts>
          <cellXfs count="3">
            <xf numFmtId="0" fontId="0"/><xf numFmtId="0" fontId="1"/><xf numFmtId="0" fontId="2"/>
          </cellXfs>
        </styleSheet>"#;
    let bytes = tiny_workbook(sheet, styles);
    let package = Package::open(&bytes).unwrap();
    let sheets = ai_import::xlsx::read(&package).unwrap();
    assert_eq!(
        sheets[0].cells["A1"].extra["style"]["font"],
        serde_json::json!("맑은 고딕")
    );
    assert!(
        sheets[0].cells["B1"].extra["style"].get("font").is_none(),
        "the format's own font is the default and is not recorded"
    );

    let ws = Workspace::new("xlsxfont");
    let mut project = create_project(ws.path(), ProjectType::Grid, "글꼴", true).unwrap();
    let Items::Sheets(project_sheets) = &mut project.items else {
        panic!()
    };
    let cell = project_sheets[0]
        .cells
        .get_mut("A1")
        .expect("a header cell");
    cell.extra
        .insert("style".into(), serde_json::json!({ "font": "맑은 고딕" }));
    let project = save_project(&project).unwrap();
    let read = round_trip(&project);
    assert_eq!(
        read[0].cells["A1"].extra["style"]["font"],
        serde_json::json!("맑은 고딕")
    );
}
