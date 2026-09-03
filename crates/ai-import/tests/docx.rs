//! Reading `.docx`.
//!
//! A Word document is a flow of paragraphs and so is a Doc section, so what is
//! checked here is that each paragraph's *meaning* survives: heading level,
//! list kind and depth, quote, table structure, page setup, alignment.

use ai_export::{export, Format};
use ai_format::mdblocks::BlockType;
use ai_format::model::{Items, ProjectType, Section};
use ai_format::project::{create_project, save_project};
use ai_import::ooxml::Package;
use ai_import::Warnings;

mod fixture;
use fixture::Workspace;

fn read_doc(bytes: &[u8]) -> Vec<Section> {
    let package = Package::open(bytes).expect("a docx package");
    let mut warnings = Warnings::default();
    ai_import::docx::read(&package, &mut warnings)
        .expect("readable document")
        .sections
}

/// A `word/document.xml` body with the styles and numbering a real file has.
fn doc_of(body: &str) -> Vec<u8> {
    const NS: &str = concat!(
        " xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"",
        " xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"",
        " xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"",
        " xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\""
    );
    let document =
        format!("<?xml version=\"1.0\"?><w:document{NS}><w:body>{body}</w:body></w:document>");
    let styles = r#"<?xml version="1.0"?>
      <w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/>
          <w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/>
          <w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/></w:style>
        <w:style w:type="paragraph" w:styleId="ListBullet"><w:name w:val="List Bullet"/>
          <w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="ListNumber"><w:name w:val="List Number"/>
          <w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr></w:style>
      </w:styles>"#;
    let numbering = r#"<?xml version="1.0"?>
      <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
        <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>
        <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
        <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
      </w:numbering>"#;
    let rels = r#"<?xml version="1.0"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;

    fixture::zip_of(&[
        ("word/document.xml".into(), document),
        ("word/_rels/document.xml.rels".into(), rels.into()),
        ("word/styles.xml".into(), styles.into()),
        ("word/numbering.xml".into(), numbering.into()),
    ])
}

/* ------------------------------------------------------------- round trips */

#[test]
fn a_document_this_project_wrote_reads_back_the_same() {
    let ws = Workspace::new("docxround");
    let project = create_project(ws.path(), ProjectType::Doc, "왕복", true).unwrap();
    let Items::Sections(original) = &project.items else {
        panic!()
    };

    let sections = read_doc(&export(&project, Format::Docx).unwrap());
    assert_eq!(sections.len(), original.len());

    let before: Vec<&str> = original[0].blocks.iter().map(|b| b.md.as_str()).collect();
    let after: Vec<&str> = sections[0].blocks.iter().map(|b| b.md.as_str()).collect();
    assert_eq!(after, before);
    assert_eq!(sections[0].page.size, original[0].page.size);
    assert_eq!(sections[0].page.margin, original[0].page.margin);
}

#[test]
fn a_table_survives_the_round_trip() {
    let ws = Workspace::new("docxtableround");
    let mut project = create_project(ws.path(), ProjectType::Doc, "표", false).unwrap();
    let Items::Sections(sections) = &mut project.items else {
        panic!()
    };
    sections[0].blocks.push(ai_format::model::DocBlock {
        id: "b_tbl".into(),
        md: "| 구분 | OOXML | AI Studio |\n|---|---|---|\n| 컨테이너 | zip | 폴더 |".into(),
        block_type: BlockType::Table,
        format_override: None,
        table: Some(ai_format::table::TableSpec {
            cols: vec![200.0, 200.0, 200.0],
            header_row: true,
            ..Default::default()
        }),
    });
    let project = save_project(&project).unwrap();

    let sections = read_doc(&export(&project, Format::Docx).unwrap());
    let table = sections[0]
        .blocks
        .iter()
        .find(|b| b.block_type == BlockType::Table)
        .expect("the table came back");
    let cells = ai_format::table::parse_markdown_table(&table.md);
    assert_eq!(cells[0], vec!["구분", "OOXML", "AI Studio"]);
    assert_eq!(cells[1], vec!["컨테이너", "zip", "폴더"]);
    assert!(table.table.as_ref().unwrap().header_row);
}

/* --------------------------------------------------- structure and meaning */

#[test]
fn heading_levels_come_from_the_style_not_the_text() {
    let sections = read_doc(&doc_of(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>제목</w:t></w:r></w:p>
           <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>소제목</w:t></w:r></w:p>
           <w:p><w:r><w:t>본문</w:t></w:r></w:p>"#,
    ));
    let blocks = &sections[0].blocks;
    assert_eq!(blocks[0].md, "# 제목");
    assert_eq!(blocks[0].block_type, BlockType::Heading);
    assert_eq!(blocks[1].md, "## 소제목");
    assert_eq!(blocks[2].block_type, BlockType::Paragraph);
    // The section takes its name from the first heading.
    assert_eq!(sections[0].name, "제목");
}

#[test]
fn lists_are_recognised_even_when_the_numbering_is_in_the_style() {
    // `List Bullet` and `List Number` put `numPr` in the style definition, which
    // is why a reader that only checks the paragraph sees plain paragraphs.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:pPr><w:pStyle w:val="ListBullet"/></w:pPr><w:r><w:t>하나</w:t></w:r></w:p>
           <w:p><w:pPr><w:pStyle w:val="ListBullet"/></w:pPr><w:r><w:t>둘</w:t></w:r></w:p>
           <w:p><w:pPr><w:pStyle w:val="ListNumber"/></w:pPr><w:r><w:t>첫째</w:t></w:r></w:p>
           <w:p><w:pPr><w:pStyle w:val="ListNumber"/></w:pPr><w:r><w:t>둘째</w:t></w:r></w:p>"#,
    ));
    let blocks = &sections[0].blocks;
    // Consecutive items of the same kind become one block, so markdown renders
    // them as a tight list rather than one paragraph per bullet.
    assert_eq!(blocks.len(), 2, "{blocks:?}");
    assert_eq!(blocks[0].md, "- 하나\n- 둘");
    assert_eq!(blocks[1].md, "1. 첫째\n1. 둘째");
    assert!(blocks.iter().all(|b| b.block_type == BlockType::List));
}

#[test]
fn a_nested_list_keeps_its_depth() {
    let sections = read_doc(&doc_of(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>
             <w:r><w:t>상위</w:t></w:r></w:p>
           <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr>
             <w:r><w:t>하위</w:t></w:r></w:p>"#,
    ));
    assert_eq!(sections[0].blocks[0].md, "- 상위\n  - 하위");
}

#[test]
fn a_paragraph_can_opt_out_of_its_list_style() {
    // `numId="0"` is how Word un-lists one paragraph inside a list style.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:pPr><w:pStyle w:val="ListBullet"/>
             <w:numPr><w:numId w:val="0"/></w:numPr></w:pPr>
             <w:r><w:t>목록이 아님</w:t></w:r></w:p>"#,
    ));
    assert_eq!(sections[0].blocks[0].block_type, BlockType::Paragraph);
    assert_eq!(sections[0].blocks[0].md, "목록이 아님");
}

#[test]
fn emphasis_becomes_markdown_and_paragraph_wide_formatting_becomes_an_override() {
    let sections = read_doc(&doc_of(
        r#"<w:p>
             <w:r><w:t xml:space="preserve">일반 </w:t></w:r>
             <w:r><w:rPr><w:b/></w:rPr><w:t>굵게</w:t></w:r>
             <w:r><w:t xml:space="preserve"> 그리고 </w:t></w:r>
             <w:r><w:rPr><w:i/></w:rPr><w:t>기울임</w:t></w:r>
             <w:r><w:rPr><w:strike/></w:rPr><w:t>취소</w:t></w:r>
           </w:p>
           <w:p><w:pPr><w:jc w:val="center"/><w:ind w:left="360"/>
                <w:spacing w:before="240" w:after="120"/></w:pPr>
             <w:r><w:rPr><w:sz w:val="28"/><w:color w:val="6B7280"/></w:rPr><w:t>서식 있는 문단</w:t></w:r>
           </w:p>"#,
    ));
    let blocks = &sections[0].blocks;
    assert_eq!(blocks[0].md, "일반 **굵게** 그리고 *기울임*~~취소~~");
    assert!(
        blocks[0].format_override.is_none(),
        "no run-wide formatting here"
    );

    let over = blocks[1].format_override.as_ref().expect("an override");
    assert_eq!(over.align.as_deref(), Some("center"));
    assert_eq!(over.indent, Some(24.0), "360 twips is 24px");
    assert_eq!(over.spacing.unwrap().before, Some(16.0));
    // A colour and size on every run of the paragraph is formatting, not emphasis.
    assert_eq!(over.style["color"], serde_json::json!("#6b7280"));
    assert_eq!(
        over.style["fontSize"],
        serde_json::json!(19.0),
        "14pt is 18.67px"
    );
}

#[test]
fn a_bold_off_marker_does_not_make_text_bold() {
    // `<w:b w:val="0"/>` turns bold off; treating its presence as "on" would
    // bold every run in a document whose style sets bold and unsets it locally.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:rPr><w:b w:val="0"/></w:rPr><w:t>보통</w:t></w:r></w:p>"#,
    ));
    assert_eq!(sections[0].blocks[0].md, "보통");
}

#[test]
fn a_quote_style_becomes_a_markdown_quote() {
    let sections = read_doc(&doc_of(
        r#"<w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr><w:r><w:t>인용문</w:t></w:r></w:p>"#,
    ));
    assert_eq!(sections[0].blocks[0].md, "> 인용문");
    assert_eq!(sections[0].blocks[0].block_type, BlockType::Quote);
}

#[test]
fn a_hyperlink_becomes_a_markdown_link() {
    let document = r#"<?xml version="1.0"?>
      <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
        <w:body><w:p>
          <w:hyperlink r:id="rId9"><w:r><w:t>사양 문서</w:t></w:r></w:hyperlink>
        </w:p></w:body></w:document>"#;
    let rels = r#"<?xml version="1.0"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId9"
          Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
          Target="https://example.test/spec" TargetMode="External"/>
      </Relationships>"#;
    let bytes = fixture::zip_of(&[
        ("word/document.xml".into(), document.into()),
        ("word/_rels/document.xml.rels".into(), rels.into()),
    ]);
    let sections = read_doc(&bytes);
    assert_eq!(
        sections[0].blocks[0].md,
        "[사양 문서](https://example.test/spec)"
    );
}

#[test]
fn page_setup_keeps_the_paper_it_names() {
    // 8.27in wide is A4; 8.5in is Letter.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="11906" w:h="16838"/>
             <w:pgMar w:top="1440" w:right="1080" w:bottom="1440" w:left="1080"/>
           </w:sectPr>"#,
    ));
    assert_eq!(sections[0].page.size, "A4");
    assert_eq!(sections[0].page.margin.top, 96.0, "1440 twips is one inch");
    assert_eq!(sections[0].page.margin.left, 72.0);

    let letter = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#,
    ));
    assert_eq!(letter[0].page.size, "Letter");
}

#[test]
fn a_section_break_starts_a_new_section() {
    let sections = read_doc(&doc_of(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>1부</w:t></w:r></w:p>
           <w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr>
             <w:r><w:t>첫 섹션 끝</w:t></w:r></w:p>
           <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>2부</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#,
    ));
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].name, "1부");
    assert_eq!(sections[1].name, "2부");
    assert_eq!(sections[0].page.size, "A4");
    assert_eq!(sections[1].page.size, "Letter");
}

#[test]
fn a_table_keeps_its_grid_merges_and_cell_shading() {
    let sections = read_doc(&doc_of(
        r#"<w:tbl>
             <w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="1500"/><w:gridCol w:w="1500"/></w:tblGrid>
             <w:tr><w:trPr><w:tblHeader/></w:trPr>
               <w:tc><w:tcPr><w:shd w:fill="F1F5F9"/></w:tcPr><w:p><w:r><w:t>항목</w:t></w:r></w:p></w:tc>
               <w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>상반기</w:t></w:r></w:p></w:tc>
             </w:tr>
             <w:tr>
               <w:tc><w:p><w:r><w:t>제품 A</w:t></w:r></w:p></w:tc>
               <w:tc><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>1,200</w:t></w:r></w:p></w:tc>
               <w:tc><w:p><w:r><w:t>1,350</w:t></w:r></w:p></w:tc>
             </w:tr>
           </w:tbl>"#,
    ));
    let block = &sections[0].blocks[0];
    assert_eq!(block.block_type, BlockType::Table);
    let spec = block.table.as_ref().unwrap();
    assert_eq!(spec.cols, vec![200.0, 100.0, 100.0], "3000 twips is 200px");
    assert_eq!(spec.merges, vec!["B1:C1"]);
    assert!(spec.header_row);
    assert_eq!(spec.cells["A1"].fill.as_deref(), Some("#f1f5f9"));

    let cells = ai_format::table::parse_markdown_table(&block.md);
    assert_eq!(cells[0], vec!["항목", "상반기", ""]);
    assert_eq!(cells[1], vec!["제품 A", "1,200", "1,350"]);
}

#[test]
fn a_cells_vertical_alignment_survives_the_round_trip() {
    // A middle-aligned cell read from Word must be written back with `w:vAlign`,
    // not silently flattened to the default top alignment on export.
    let sections = read_doc(&doc_of(
        r#"<w:tbl>
             <w:tblGrid><w:gridCol w:w="1500"/><w:gridCol w:w="1500"/></w:tblGrid>
             <w:tr>
               <w:tc><w:tcPr><w:vAlign w:val="center"/></w:tcPr><w:p><w:r><w:t>가운데</w:t></w:r></w:p></w:tc>
               <w:tc><w:tcPr><w:vAlign w:val="bottom"/></w:tcPr><w:p><w:r><w:t>아래</w:t></w:r></w:p></w:tc>
             </w:tr>
           </w:tbl>"#,
    ));
    assert_eq!(
        sections[0].blocks[0].table.as_ref().unwrap().cells["A1"]
            .valign
            .as_deref(),
        Some("middle"),
        "read as middle"
    );

    // Round-trip it through export and back.
    let ws = Workspace::new("docxvalign");
    let mut project = create_project(ws.path(), ProjectType::Doc, "정렬", false).unwrap();
    let Items::Sections(project_sections) = &mut project.items else {
        panic!()
    };
    project_sections[0].blocks = sections[0].blocks.clone();
    let project = save_project(&project).unwrap();

    let after = read_doc(&export(&project, Format::Docx).unwrap());
    let spec = after[0]
        .blocks
        .iter()
        .find_map(|b| b.table.as_ref())
        .expect("the table survived");
    assert_eq!(spec.cells["A1"].valign.as_deref(), Some("middle"));
    assert_eq!(spec.cells["B1"].valign.as_deref(), Some("bottom"));
}

#[test]
fn a_vertical_merge_spans_the_rows_it_covers() {
    let sections = read_doc(&doc_of(
        r#"<w:tbl>
             <w:tblGrid><w:gridCol w:w="1500"/><w:gridCol w:w="1500"/></w:tblGrid>
             <w:tr>
               <w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>합쳐짐</w:t></w:r></w:p></w:tc>
               <w:tc><w:p><w:r><w:t>가</w:t></w:r></w:p></w:tc>
             </w:tr>
             <w:tr>
               <w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc>
               <w:tc><w:p><w:r><w:t>나</w:t></w:r></w:p></w:tc>
             </w:tr>
           </w:tbl>"#,
    ));
    let spec = sections[0].blocks[0].table.as_ref().unwrap();
    assert_eq!(spec.merges, vec!["A1:A2"]);
    let cells = ai_format::table::parse_markdown_table(&sections[0].blocks[0].md);
    assert_eq!(cells[0], vec!["합쳐짐", "가"]);
    assert_eq!(cells[1], vec!["", "나"], "the covered cell is empty");
}

#[test]
fn an_empty_paragraph_is_spacing_not_content() {
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:p/>
           <w:p><w:r><w:t>다음</w:t></w:r></w:p>"#,
    ));
    assert_eq!(sections[0].blocks.len(), 2);
}

#[test]
fn an_image_becomes_an_asset_and_a_markdown_reference() {
    let document = r#"<?xml version="1.0"?>
      <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                  xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">
        <w:body><w:p><w:r><w:drawing><wp:inline>
          <wp:docPr id="1" name="Picture 1" descr="분기별 매출"/>
          <a:graphic><a:graphicData><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
            <pic:blipFill><a:blip r:embed="rId5"/></pic:blipFill>
          </pic:pic></a:graphicData></a:graphic>
        </wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#;
    let rels = r#"<?xml version="1.0"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId5"
          Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
          Target="media/image1.png"/>
      </Relationships>"#;
    let mut parts = vec![
        ("word/document.xml".to_string(), document.to_string()),
        ("word/_rels/document.xml.rels".to_string(), rels.to_string()),
    ];
    parts.push(("word/media/image1.png".to_string(), "PNGBYTES".to_string()));
    let bytes = fixture::zip_of(&parts);

    let package = Package::open(&bytes).unwrap();
    let mut warnings = Warnings::default();
    let document = ai_import::docx::read(&package, &mut warnings).unwrap();
    assert_eq!(document.assets.len(), 1);
    assert_eq!(document.assets[0].name, "image1.png");
    assert_eq!(
        document.sections[0].blocks[0].md,
        "![분기별 매출](../assets/image1.png)"
    );
    assert_eq!(document.sections[0].blocks[0].block_type, BlockType::Image);
}

#[test]
fn a_caption_typed_beside_an_image_is_not_dropped() {
    // A picture with text in the same paragraph — a figure and its caption —
    // must keep both. The image path used to win and the words were lost.
    let document = r#"<?xml version="1.0"?>
      <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                  xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">
        <w:body><w:p>
          <w:r><w:drawing><wp:inline>
            <wp:docPr id="1" name="Picture 1" descr="도표"/>
            <a:graphic><a:graphicData><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
              <pic:blipFill><a:blip r:embed="rId5"/></pic:blipFill>
            </pic:pic></a:graphicData></a:graphic>
          </wp:inline></w:drawing></w:r>
          <w:r><w:t>그림 1. 분기별 매출</w:t></w:r>
        </w:p></w:body></w:document>"#;
    let rels = r#"<?xml version="1.0"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId5"
          Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
          Target="media/image1.png"/>
      </Relationships>"#;
    let parts = vec![
        ("word/document.xml".to_string(), document.to_string()),
        ("word/_rels/document.xml.rels".to_string(), rels.to_string()),
        ("word/media/image1.png".to_string(), "PNGBYTES".to_string()),
    ];
    let bytes = fixture::zip_of(&parts);

    let package = Package::open(&bytes).unwrap();
    let mut warnings = Warnings::default();
    let document = ai_import::docx::read(&package, &mut warnings).unwrap();
    assert_eq!(document.assets.len(), 1, "the image is still kept");
    let md = &document.sections[0].blocks[0].md;
    assert!(
        md.contains("![도표](../assets/image1.png)"),
        "image kept: {md}"
    );
    assert!(md.contains("그림 1. 분기별 매출"), "caption kept: {md}");
}

/* ------------------------------------------------------------- page setup */

#[test]
fn a_landscape_page_stays_landscape() {
    // A4 turned sideways: the size is still A4's, and the author laid the whole
    // document out against 1123px of width.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/></w:sectPr>"#,
    ));
    let page = &sections[0].page;
    assert_eq!(page.size, "A4");
    assert_eq!(page.dimensions(), (1123.0, 794.0));
    assert!(page.landscape());
}

#[test]
fn an_orientation_stated_without_a_swapped_size_is_honoured() {
    // Some writers state landscape and leave the size portrait. Word draws that
    // sideways, so reading the size alone would rotate the document back.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="11906" w:h="16838" w:orient="landscape"/></w:sectPr>"#,
    ));
    assert!(sections[0].page.landscape());
    assert_eq!(sections[0].page.dimensions(), (1123.0, 794.0));
}

#[test]
fn a_page_size_no_paper_matches_is_kept_as_typed() {
    // 180 x 250mm, a booklet. Snapping it to A4 reflows every line of the
    // document, which is the most visible way an import stops looking like the
    // file it came from.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="10206" w:h="14174"/></w:sectPr>"#,
    ));
    let page = &sections[0].page;
    assert_eq!(page.size, ai_format::model::CUSTOM_PAPER);
    assert_eq!(page.dimensions(), (680.0, 945.0));
    assert!(!page.landscape());
}

#[test]
fn a_portrait_paper_writes_no_explicit_size() {
    // The name already says it, and writing the pixels too would change every
    // existing project's JSON for nothing.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>본문</w:t></w:r></w:p>
           <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>"#,
    ));
    assert_eq!(sections[0].page.width, None);
    assert_eq!(sections[0].page.height, None);
}

/* ------------------------------------------------------------- text sizes */

/// A `word/styles.xml` with a document default and sized headings, as Word
/// writes them: the size of a paragraph is almost never on its runs.
fn sized_doc(body: &str) -> Vec<u8> {
    let styles = r#"<?xml version="1.0"?>
      <w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/>
          <w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr>
          <w:rPr><w:sz w:val="32"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/>
          <w:basedOn w:val="Heading1"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Small"><w:name w:val="Small"/>
          <w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="18"/></w:rPr></w:style>
      </w:styles>"#;
    let document = format!(
        "<?xml version=\"1.0\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
    );
    fixture::zip_of(&[
        ("word/document.xml".into(), document),
        (
            "word/_rels/document.xml.rels".into(),
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into(),
        ),
        ("word/styles.xml".into(), styles.into()),
        (
            "_rels/.rels".into(),
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".into(),
        ),
    ])
}

#[test]
fn a_heading_takes_its_size_from_its_style() {
    let sections = read_doc(&sized_doc(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>제목</w:t></w:r></w:p>"#,
    ));
    let over = sections[0].blocks[0]
        .format_override
        .as_ref()
        .expect("the size differs from what the editor draws");
    assert_eq!(
        over.style["fontSize"],
        serde_json::json!(21.0),
        "16pt is 21.33px, where the editor's own h1 is 25px"
    );
}

#[test]
fn a_style_takes_its_size_from_the_one_it_is_based_on() {
    let sections = read_doc(&sized_doc(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>절</w:t></w:r></w:p>"#,
    ));
    let over = sections[0].blocks[0]
        .format_override
        .as_ref()
        .expect("an override");
    assert_eq!(
        over.style["fontSize"],
        serde_json::json!(21.0),
        "Heading2 states no size and is based on Heading1"
    );
}

#[test]
fn a_body_paragraph_at_the_documents_own_size_carries_no_override() {
    // 11pt is 14.67px, which rounds to the 15px the editor already draws. An
    // override there would put an anchor in the markdown and change nothing.
    let sections = read_doc(&sized_doc(r#"<w:p><w:r><w:t>본문입니다</w:t></w:r></w:p>"#));
    assert!(sections[0].blocks[0].format_override.is_none());
}

#[test]
fn a_paragraph_smaller_than_the_default_keeps_its_size() {
    let sections = read_doc(&sized_doc(
        r#"<w:p><w:pPr><w:pStyle w:val="Small"/></w:pPr><w:r><w:t>각주</w:t></w:r></w:p>"#,
    ));
    let over = sections[0].blocks[0]
        .format_override
        .as_ref()
        .expect("an override");
    assert_eq!(over.style["fontSize"], serde_json::json!(12.0), "9pt");
}

#[test]
fn a_page_and_a_text_size_survive_the_round_trip() {
    // The loop a user actually runs: open a Word file, save, export, open the
    // export. A landscape page that comes back portrait, or a 21px heading that
    // comes back at the editor's own 25px, is the difference they would see.
    let ws = Workspace::new("docxcycle");
    let mut project =
        ai_format::project::create_project(ws.path(), ProjectType::Doc, "왕복", false).unwrap();
    let Items::Sections(sections) = &mut project.items else {
        panic!()
    };
    sections[0].page = ai_format::model::Page::sized(1123.0, 794.0, Default::default(), 1);
    let mut over = ai_format::model::Override::default();
    over.style.insert("fontSize".into(), serde_json::json!(21));
    sections[0].blocks = vec![ai_format::model::DocBlock {
        id: ai_format::ids::new_block_id(),
        md: "# 제목".into(),
        block_type: BlockType::Heading,
        format_override: Some(over),
        table: None,
    }];
    let project = save_project(&project).unwrap();

    let again = read_doc(&export(&project, Format::Docx).unwrap());
    assert_eq!(again[0].page.dimensions(), (1123.0, 794.0));
    assert!(again[0].page.landscape());
    let over = again[0].blocks[0]
        .format_override
        .as_ref()
        .expect("the size came back");
    assert_eq!(over.style["fontSize"], serde_json::json!(21.0));
}

/* ---------------------------------------------------- headers and footers */

#[test]
fn a_header_and_a_page_number_field_come_across() {
    // Word keeps these in their own parts and points at them from the section,
    // so a reader that only walks the body never sees the page numbers at all.
    let header = r#"<?xml version="1.0"?>
      <w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>AI Studio 제안서</w:t></w:r></w:p>
      </w:hdr>"#;
    let footer = r#"<?xml version="1.0"?>
      <w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:p>
          <w:pPr><w:tabs><w:tab w:val="center" w:pos="4680"/></w:tabs></w:pPr>
          <w:r><w:t>기밀</w:t></w:r>
          <w:r><w:tab/></w:r>
          <w:r><w:fldChar w:fldCharType="begin"/></w:r>
          <w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>
          <w:r><w:fldChar w:fldCharType="end"/></w:r>
          <w:r><w:t xml:space="preserve"> / </w:t></w:r>
          <w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>1</w:t></w:r></w:fldSimple>
        </w:p>
      </w:ftr>"#;
    let document = r#"<?xml version="1.0"?>
      <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
        <w:body>
          <w:p><w:r><w:t>본문</w:t></w:r></w:p>
          <w:sectPr>
            <w:headerReference w:type="default" r:id="rIdH"/>
            <w:footerReference w:type="default" r:id="rIdF"/>
            <w:pgSz w:w="11906" w:h="16838"/>
          </w:sectPr>
        </w:body>
      </w:document>"#;
    let rels = r#"<?xml version="1.0"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
        <Relationship Id="rIdF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>
      </Relationships>"#;
    let bytes = fixture::zip_of(&[
        ("word/document.xml".into(), document.into()),
        ("word/_rels/document.xml.rels".into(), rels.into()),
        ("word/header1.xml".into(), header.into()),
        ("word/footer1.xml".into(), footer.into()),
        (
            "_rels/.rels".into(),
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".into(),
        ),
    ]);

    let sections = read_doc(&bytes);
    let page = &sections[0].page;
    let header = page.header.as_ref().expect("a header");
    assert_eq!(header.center, "AI Studio 제안서", "centred, not left");
    assert!(header.left.is_empty());

    let footer = page.footer.as_ref().expect("a footer");
    assert_eq!(footer.left, "기밀");
    // Both field spellings, in document order, and the cached "1" inside the
    // field is not text.
    assert_eq!(footer.center, "{PAGE} / {PAGES}");
}

#[test]
fn a_header_and_footer_survive_the_round_trip() {
    let ws = Workspace::new("docxrunning");
    let mut project =
        ai_format::project::create_project(ws.path(), ProjectType::Doc, "머리글", true).unwrap();
    let Items::Sections(sections) = &mut project.items else {
        panic!()
    };
    sections[0].page.header = Some(ai_format::model::Running {
        center: "AI Studio 제안서".into(),
        ..Default::default()
    });
    sections[0].page.footer = Some(ai_format::model::Running {
        left: "{DATE}".into(),
        center: "{PAGE} / {PAGES}".into(),
        right: "기밀".into(),
    });
    let project = save_project(&project).unwrap();

    let again = read_doc(&export(&project, Format::Docx).unwrap());
    let page = &again[0].page;
    assert_eq!(
        page.header.as_ref().map(|r| r.center.as_str()),
        Some("AI Studio 제안서")
    );
    let footer = page.footer.as_ref().expect("the footer came back");
    assert_eq!(
        (
            footer.left.as_str(),
            footer.center.as_str(),
            footer.right.as_str()
        ),
        ("{DATE}", "{PAGE} / {PAGES}", "기밀"),
        "all three slots, in their own positions"
    );
}

#[test]
fn text_inside_tracked_changes_content_controls_and_text_boxes_arrives() {
    // Three shapes that all wrap their runs one level deeper than a plain
    // paragraph. Each of them used to lose the text entirely.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t xml:space="preserve">받은 </w:t></w:r>
             <w:ins w:id="1" w:author="검토자"><w:r><w:t>수정된 </w:t></w:r></w:ins>
             <w:del w:id="2"><w:r><w:delText>지운 </w:delText></w:r></w:del>
             <w:r><w:t>문장</w:t></w:r></w:p>
           <w:p><w:sdt><w:sdtContent><w:r><w:t>콘텐츠 컨트롤 안의 글</w:t></w:r></w:sdtContent></w:sdt></w:p>
           <w:p><w:r><w:drawing><wp:inline><a:graphic><a:graphicData>
             <wps:wsp><wps:txbx><w:txbxContent>
               <w:p><w:r><w:t>텍스트 상자의 인용</w:t></w:r></w:p>
             </w:txbxContent></wps:txbx></wps:wsp>
           </a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
    ));
    let text: Vec<&str> = sections[0].blocks.iter().map(|b| b.md.as_str()).collect();
    assert_eq!(
        text,
        [
            "받은 수정된 문장",
            "콘텐츠 컨트롤 안의 글",
            "텍스트 상자의 인용"
        ],
        "tracked insertions are content, deletions are not"
    );
}

#[test]
fn a_table_inside_a_table_keeps_its_words() {
    // A markdown table cannot hold a table, so the nested one's cells become
    // lines in the cell that held it — the words and their order survive, where
    // skipping the nested table loses the cell entirely.
    let sections = read_doc(&doc_of(
        r#"<w:tbl>
             <w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/></w:tblGrid>
             <w:tr>
               <w:tc><w:p><w:r><w:t>구분</w:t></w:r></w:p></w:tc>
               <w:tc>
                 <w:p><w:r><w:t>세부</w:t></w:r></w:p>
                 <w:tbl><w:tr>
                   <w:tc><w:p><w:r><w:t>1분기</w:t></w:r></w:p></w:tc>
                   <w:tc><w:p><w:r><w:t>2분기</w:t></w:r></w:p></w:tc>
                 </w:tr></w:tbl>
               </w:tc>
             </w:tr>
           </w:tbl>"#,
    ));
    let table = sections[0]
        .blocks
        .iter()
        .find(|b| b.block_type == BlockType::Table)
        .expect("a table");
    assert!(
        table.md.contains("1분기") && table.md.contains("2분기"),
        "{}",
        table.md
    );
}

#[test]
fn a_paragraphs_font_family_survives_the_round_trip() {
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Malgun Gothic" w:eastAsia="맑은 고딕"/></w:rPr><w:t>본문 한 줄</w:t></w:r></w:p>"#,
    ));
    let over = sections[0].blocks[0]
        .format_override
        .as_ref()
        .expect("the family is an override");
    assert_eq!(over.style["font"], serde_json::json!("맑은 고딕"));

    let ws = Workspace::new("docxfont");
    let mut project = create_project(ws.path(), ProjectType::Doc, "글꼴", false).unwrap();
    let Items::Sections(project_sections) = &mut project.items else {
        panic!()
    };
    project_sections[0].blocks = sections[0].blocks.clone();
    let project = save_project(&project).unwrap();
    let after = read_doc(&export(&project, Format::Docx).unwrap());
    let over = after[0].blocks[0].format_override.as_ref().expect("kept");
    assert_eq!(over.style["font"], serde_json::json!("맑은 고딕"));
}

#[test]
fn a_run_coloured_unlike_its_paragraph_is_marked_inline_and_survives_the_round_trip() {
    // A burgundy lead-in on a grey paragraph: the grey is the paragraph's
    // colour (the override), the lead-in keeps its own inline. A black run
    // among default runs is not a colour and gets no markup.
    let sections = read_doc(&doc_of(
        r#"<w:p>
             <w:r><w:rPr><w:b/><w:color w:val="A50034"/></w:rPr><w:t xml:space="preserve">핵심 </w:t></w:r>
             <w:r><w:rPr><w:color w:val="202124"/></w:rPr><w:t xml:space="preserve">본문 첫 부분 </w:t></w:r>
             <w:r><w:rPr><w:color w:val="202124"/></w:rPr><w:t>본문 둘째 부분</w:t></w:r>
           </w:p>
           <w:p>
             <w:r><w:rPr><w:color w:val="000000"/></w:rPr><w:t xml:space="preserve">검정 </w:t></w:r>
             <w:r><w:t>기본</w:t></w:r>
           </w:p>"#,
    ));
    let first = &sections[0].blocks[0];
    assert_eq!(
        first.md, "<span style=\"color:#a50034\">**핵심**</span> 본문 첫 부분 본문 둘째 부분",
        "{}",
        first.md
    );
    assert_eq!(
        first.format_override.as_ref().unwrap().style["color"],
        serde_json::json!("#202124"),
        "the colour most of the text carries is the paragraph's"
    );
    assert_eq!(sections[0].blocks[1].md, "검정 기본");

    let ws = Workspace::new("docxruninline");
    let mut project = create_project(ws.path(), ProjectType::Doc, "런", false).unwrap();
    let Items::Sections(project_sections) = &mut project.items else {
        panic!()
    };
    project_sections[0].blocks = sections[0].blocks.clone();
    let project = save_project(&project).unwrap();
    let again = read_doc(&export(&project, Format::Docx).unwrap());
    assert_eq!(again[0].blocks[0].md, first.md, "idempotent through Word");
}

#[test]
fn a_paragraph_that_looks_like_a_list_marker_stays_a_paragraph() {
    // "1. 배경" typed as plain text, not a numbered list — Word has no numPr
    // on it. Left as-is the export would turn it into a list.
    let sections = read_doc(&doc_of(
        r#"<w:p><w:r><w:t>1. 배경 및 목표</w:t></w:r></w:p>"#,
    ));
    assert_eq!(sections[0].blocks[0].md, "1\\. 배경 및 목표");
    assert_eq!(sections[0].blocks[0].block_type, BlockType::Paragraph);
}
