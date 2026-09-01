//! Structural validation of the produced Office files, in the spirit of
//! `scripts/export-check.mjs`: unzip the result and inspect the XML, rather
//! than trusting that the writer ran without erroring.

use std::io::Read;
use std::path::{Path, PathBuf};

use ai_export::{export, Format};
use ai_format::model::{Items, ProjectType};
use ai_format::project::{create_project, load_project, save_project};

struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ai-studio-export-{tag}-{}-{}",
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

/// Every part of a package, as text.
struct Parts(Vec<(String, String)>);

impl Parts {
    fn of(bytes: &[u8]) -> Parts {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec()))
            .expect("the export is a valid zip");
        let mut out = Vec::new();
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).unwrap();
            let name = file.name().to_string();
            let mut text = String::new();
            // Media parts are not UTF-8; record them by name only.
            if file.read_to_string(&mut text).is_err() {
                text = String::new();
            }
            out.push((name, text));
        }
        Parts(out)
    }

    fn names(&self) -> Vec<&str> {
        self.0.iter().map(|(n, _)| n.as_str()).collect()
    }

    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n == name)
    }

    fn get(&self, name: &str) -> &str {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.as_str())
            .unwrap_or_else(|| panic!("missing part {name}; have {:?}", self.names()))
    }

    /// Every part references a target that exists, and every declared override
    /// names a real part — the two things that make Office declare a file corrupt.
    fn assert_internally_consistent(&self, root: &str) {
        let content_types = self.get("[Content_Types].xml");
        for (name, _) in &self.0 {
            if name.ends_with(".xml") && !name.contains("_rels") && name != "[Content_Types].xml" {
                let declared = content_types.contains(&format!("PartName=\"/{name}\""))
                    || content_types.contains("Extension=\"xml\"");
                assert!(declared, "{name} is not declared in [Content_Types].xml");
            }
        }

        for (name, text) in &self.0 {
            if !name.ends_with(".rels") {
                continue;
            }
            let base = Path::new(name)
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            for target in targets(text) {
                if target.starts_with("http") {
                    continue;
                }
                let resolved = normalize(&base.join(&target));
                let full = if resolved.starts_with(root) || root.is_empty() {
                    resolved.clone()
                } else {
                    resolved
                };
                assert!(
                    self.has(&full),
                    "{name} points at {target} -> {full}, which is not in the package.\nHave: {:?}",
                    self.names()
                );
            }
        }
    }
}

fn targets(rels_xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = rels_xml;
    while let Some(at) = rest.find("Target=\"") {
        rest = &rest[at + 8..];
        if let Some(end) = rest.find('"') {
            out.push(rest[..end].to_string());
            rest = &rest[end..];
        }
    }
    out
}

/// Collapse `a/../b` the way a package consumer does.
fn normalize(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop();
            }
            _ => {}
        }
    }
    parts.join("/")
}

/* -------------------------------------------------------------------- pptx */

#[test]
fn a_deck_exports_to_a_well_formed_pptx() {
    let ws = Workspace::new("pptx");
    let project = create_project(ws.path(), ProjectType::Deck, "내보내기 검증", true).unwrap();
    let bytes = export(&project, Format::Pptx).unwrap();
    let parts = Parts::of(&bytes);

    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "ppt/presentation.xml",
        "ppt/_rels/presentation.xml.rels",
        "ppt/slideMasters/slideMaster1.xml",
        "ppt/slideLayouts/slideLayout1.xml",
        "ppt/theme/theme1.xml",
        "ppt/slides/slide1.xml",
        "ppt/slides/slide2.xml",
        "docProps/core.xml",
    ] {
        assert!(
            parts.has(required),
            "missing {required}; have {:?}",
            parts.names()
        );
    }
    parts.assert_internally_consistent("ppt");

    // The slide size is the canvas, in EMU.
    let presentation = parts.get("ppt/presentation.xml");
    assert!(
        presentation.contains("<p:sldSz cx=\"12192000\" cy=\"6858000\"/>"),
        "{presentation}"
    );

    // Text and geometry both crossed over.
    let slide = parts.get("ppt/slides/slide1.xml");
    assert!(slide.contains("내보내기 검증"), "{slide}");
    assert!(
        slide.contains("<a:off x=\"914400\" y="),
        "96px is 914400 EMU: {slide}"
    );
}

#[test]
fn a_chart_block_becomes_an_editable_chart_part() {
    let ws = Workspace::new("pptxchart");
    let mut project = create_project(ws.path(), ProjectType::Deck, "차트", false).unwrap();

    let chart_md = "```chart\n{\"type\":\"column\",\"title\":\"분기별 매출\",\"labels\":[\"1분기\",\"2분기\"],\"series\":[{\"name\":\"2026\",\"values\":[120,133]}]}\n```";
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides[0].blocks.push(ai_format::model::SlideBlock {
        id: "b_chart".into(),
        kind: ai_format::blocks::Kind::Chart,
        md: chart_md.into(),
        x: 96.0,
        y: 200.0,
        w: 600.0,
        h: 360.0,
        z: 9.0,
        style: Default::default(),
        locked: false,
    });
    let project = save_project(&project).unwrap();

    let parts = Parts::of(&export(&project, Format::Pptx).unwrap());
    assert!(
        parts.has("ppt/charts/chart1.xml"),
        "have {:?}",
        parts.names()
    );
    parts.assert_internally_consistent("ppt");

    let chart = parts.get("ppt/charts/chart1.xml");
    assert!(chart.contains("<c:barChart>"));
    assert!(
        chart.contains("<c:v>133</c:v>"),
        "the numbers cross over: {chart}"
    );
    assert!(chart.contains("분기별 매출"));

    // The slide references the chart through a graphicFrame, not an image.
    let slide = parts.get("ppt/slides/slide1.xml");
    assert!(slide.contains("<p:graphicFrame>"), "{slide}");
    assert!(slide.contains("drawingml/2006/chart"), "{slide}");

    // And the chart part is declared.
    assert!(parts
        .get("[Content_Types].xml")
        .contains("/ppt/charts/chart1.xml"));
}

#[test]
fn speaker_notes_become_a_notes_slide() {
    let ws = Workspace::new("pptxnotes");
    let mut project = create_project(ws.path(), ProjectType::Deck, "노트", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides[0].notes = "전년 동기 대비라는 점을 반드시 짚는다.".into();
    let project = save_project(&project).unwrap();

    let parts = Parts::of(&export(&project, Format::Pptx).unwrap());
    assert!(parts.has("ppt/notesSlides/notesSlide1.xml"));
    assert!(
        parts.has("ppt/notesMasters/notesMaster1.xml"),
        "a notes slide needs its master"
    );
    parts.assert_internally_consistent("ppt");
    assert!(parts
        .get("ppt/notesSlides/notesSlide1.xml")
        .contains("전년 동기 대비라는 점을 반드시 짚는다."));
    assert!(parts
        .get("ppt/presentation.xml")
        .contains("<p:notesMasterIdLst>"));
}

#[test]
fn a_deck_without_notes_writes_no_notes_parts() {
    let ws = Workspace::new("pptxnonotes");
    let project = create_project(ws.path(), ProjectType::Deck, "노트 없음", true).unwrap();
    let parts = Parts::of(&export(&project, Format::Pptx).unwrap());
    assert!(parts.names().iter().all(|n| !n.contains("notesSlide")));
    assert!(!parts
        .get("ppt/presentation.xml")
        .contains("notesMasterIdLst"));
}

/* -------------------------------------------------------------------- docx */

#[test]
fn a_doc_exports_to_a_well_formed_docx() {
    let ws = Workspace::new("docx");
    let project = create_project(ws.path(), ProjectType::Doc, "문서 내보내기", true).unwrap();
    let parts = Parts::of(&export(&project, Format::Docx).unwrap());

    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "word/document.xml",
        "word/_rels/document.xml.rels",
        "word/styles.xml",
        "word/numbering.xml",
    ] {
        assert!(
            parts.has(required),
            "missing {required}; have {:?}",
            parts.names()
        );
    }
    parts.assert_internally_consistent("word");

    let document = parts.get("word/document.xml");
    assert!(document.contains("문서 내보내기"), "{document}");
    assert!(
        document.contains("<w:pStyle w:val=\"Heading1\"/>"),
        "{document}"
    );
    // Page setup came from the section's own page block: A4 at 96dpi is
    // 794x1123 px, which is 11910x16845 twips — 8.27 x 11.70 inches.
    assert!(
        document.contains("<w:pgSz w:w=\"11910\" w:h=\"16845\"/>"),
        "{document}"
    );
    assert!(
        document.contains("<w:pgMar w:top=\"1080\""),
        "72px is 0.75in is 1080 twips: {document}"
    );
}

#[test]
fn paragraph_overrides_map_onto_word() {
    let ws = Workspace::new("docxfmt");
    let mut project = create_project(ws.path(), ProjectType::Doc, "서식", false).unwrap();
    let Items::Sections(sections) = &mut project.items else {
        panic!()
    };
    let mut o = ai_format::model::Override {
        align: Some("center".into()),
        indent: Some(24.0),
        ..Default::default()
    };
    o.style.insert("bold".into(), serde_json::json!(true));
    o.style.insert("color".into(), serde_json::json!("#ff0000"));
    sections[0].blocks[1].format_override = Some(o);
    let project = save_project(&project).unwrap();

    let document = Parts::of(&export(&project, Format::Docx).unwrap())
        .get("word/document.xml")
        .to_string();
    assert!(document.contains("<w:jc w:val=\"center\"/>"), "{document}");
    assert!(document.contains("<w:ind w:left=\"360\"/>"), "{document}");
    assert!(
        document.contains("<w:color w:val=\"FF0000\"/>"),
        "{document}"
    );
}

#[test]
fn a_chart_in_a_doc_becomes_its_data_table() {
    let ws = Workspace::new("docxchart");
    let mut project = create_project(ws.path(), ProjectType::Doc, "차트 문서", false).unwrap();
    let Items::Sections(sections) = &mut project.items else {
        panic!()
    };
    sections[0].blocks.push(ai_format::model::DocBlock {
        id: "b_chart".into(),
        md: "```chart\n{\"type\":\"line\",\"title\":\"추이\",\"labels\":[\"1월\"],\"series\":[{\"name\":\"매출\",\"values\":[10]}]}\n```".into(),
        block_type: ai_format::mdblocks::BlockType::Code,
        format_override: None,
    });
    let project = save_project(&project).unwrap();

    let document = Parts::of(&export(&project, Format::Docx).unwrap())
        .get("word/document.xml")
        .to_string();
    assert!(document.contains("추이"), "{document}");
    assert!(
        document.contains("<w:tbl>"),
        "the numbers arrive as a table: {document}"
    );
    assert!(
        document.contains("표 데이터 ·"),
        "with a caption naming the shape"
    );
    assert!(document.contains("꺾은선"), "{document}");
}

/* -------------------------------------------------------------------- xlsx */

#[test]
fn a_grid_exports_to_a_well_formed_xlsx_with_live_formulas() {
    let ws = Workspace::new("xlsx");
    let project = create_project(ws.path(), ProjectType::Grid, "예산 내보내기", true).unwrap();
    let parts = Parts::of(&export(&project, Format::Xlsx).unwrap());

    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "xl/sharedStrings.xml",
        "xl/worksheets/sheet1.xml",
    ] {
        assert!(
            parts.has(required),
            "missing {required}; have {:?}",
            parts.names()
        );
    }
    parts.assert_internally_consistent("xl");

    let sheet = parts.get("xl/worksheets/sheet1.xml");
    // Formulas go across as formulas, not their cached values.
    assert!(sheet.contains("<f>B2+C2</f>"), "{sheet}");
    assert!(sheet.contains("<f>SUM(B2:B4)</f>"), "{sheet}");
    // With the cached value alongside, so the sheet reads before recalculation.
    assert!(sheet.contains("<f>SUM(B2:B4)</f><v>2820</v>"), "{sheet}");
    // Frozen panes from the sheet's own setting.
    assert!(sheet.contains("state=\"frozen\""), "{sheet}");
    assert!(sheet.contains("xSplit=\"1\" ySplit=\"1\""), "{sheet}");

    // Number formats became custom numFmts.
    let styles = parts.get("xl/styles.xml");
    assert!(styles.contains("formatCode=\"#,##0\""), "{styles}");
    assert!(
        styles.contains("<b/>"),
        "bold cells produced a font: {styles}"
    );

    // Text went to the shared string table.
    assert!(parts.get("xl/sharedStrings.xml").contains("항목"));
    assert!(
        parts
            .get("xl/workbook.xml")
            .contains("name=\"예산 내보내기\"")
            || parts.get("xl/workbook.xml").contains("<sheet name=")
    );
}

#[test]
fn named_ranges_become_defined_names() {
    let ws = Workspace::new("xlsxnames");
    let mut project = create_project(ws.path(), ProjectType::Grid, "이름", true).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    sheets[0]
        .names
        .insert("매출데이터".into(), serde_json::json!("A1:D5"));
    sheets[0]
        .names
        .insert("2024".into(), serde_json::json!("A1")); // rejected
    let project = save_project(&project).unwrap();

    let workbook = Parts::of(&export(&project, Format::Xlsx).unwrap())
        .get("xl/workbook.xml")
        .to_string();
    assert!(workbook.contains("<definedNames>"), "{workbook}");
    assert!(workbook.contains("name=\"매출데이터\""), "{workbook}");
    assert!(
        workbook.contains("$A$1:$D$5"),
        "absolute and sheet-qualified: {workbook}"
    );
    assert!(
        !workbook.contains("name=\"2024\""),
        "a digit-leading name is dropped"
    );
}

#[test]
fn merges_and_column_widths_cross_over() {
    let ws = Workspace::new("xlsxmerge");
    let mut project = create_project(ws.path(), ProjectType::Grid, "병합", true).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    sheets[0].merges = vec![serde_json::json!("A1:B1"), serde_json::json!("nonsense")];
    sheets[0].col_widths.insert("A".into(), 210);
    let project = save_project(&project).unwrap();

    let sheet = Parts::of(&export(&project, Format::Xlsx).unwrap())
        .get("xl/worksheets/sheet1.xml")
        .to_string();
    assert!(sheet.contains("<mergeCell ref=\"A1:B1\"/>"), "{sheet}");
    assert!(
        !sheet.contains("nonsense"),
        "an invalid merge is skipped, not fatal"
    );
    assert!(
        sheet.contains("width=\"30\""),
        "210px / 7 = 30 characters: {sheet}"
    );
}

#[test]
fn an_error_cell_exports_as_an_error() {
    let ws = Workspace::new("xlsxerr");
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

    let sheet = Parts::of(&export(&project, Format::Xlsx).unwrap())
        .get("xl/worksheets/sheet1.xml")
        .to_string();
    assert!(sheet.contains("t=\"e\""), "{sheet}");
    assert!(sheet.contains("<v>#DIV/0!</v>"), "{sheet}");
}

/* --------------------------------------------------------------------- csv */

#[test]
fn a_grid_exports_to_csv_with_a_bom() {
    let ws = Workspace::new("csv");
    let project = create_project(ws.path(), ProjectType::Grid, "csv", true).unwrap();
    let bytes = export(&project, Format::Csv).unwrap();
    let text = String::from_utf8(bytes).unwrap();

    assert!(
        text.starts_with('\u{FEFF}'),
        "Excel on Windows needs the BOM"
    );
    assert!(text.contains("\r\n"), "CRLF line endings");
    let body = text.trim_start_matches('\u{FEFF}');
    assert!(body.starts_with("항목,1분기,2분기,합계"), "{body}");
    // Values, as displayed — the formula's result, not the formula.
    assert!(
        body.contains("2,550") || body.contains("\"2,550\""),
        "{body}"
    );
}

/* ------------------------------------------------------------ type guards */

#[test]
fn exporting_a_deck_as_xlsx_is_refused() {
    let ws = Workspace::new("guard");
    let project = create_project(ws.path(), ProjectType::Deck, "타입", false).unwrap();
    assert!(export(&project, Format::Xlsx).is_err());
    assert!(export(&project, Format::Pptx).is_ok());
}

#[test]
fn every_workspace_sample_exports_without_error() {
    // The three seeded documents in the repo's own workspace, if present.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workspace");
    if !root.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()) {
        let dir = entry.path();
        let Some(project_type) = ai_format::project::type_from_path(&dir) else {
            continue;
        };
        let project = load_project(&dir).unwrap();
        let formats: &[Format] = match project_type {
            ProjectType::Deck => &[Format::Pptx],
            ProjectType::Doc => &[Format::Docx],
            ProjectType::Grid => &[Format::Xlsx, Format::Csv],
        };
        for format in formats {
            let bytes = export(&project, *format)
                .unwrap_or_else(|e| panic!("{} as .{}: {e}", dir.display(), format.ext()));
            assert!(
                bytes.len() > 200,
                "{} as .{} is suspiciously small",
                dir.display(),
                format.ext()
            );
            if *format != Format::Csv {
                Parts::of(&bytes).assert_internally_consistent("");
            }
        }
    }
}
