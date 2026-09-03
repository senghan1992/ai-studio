//! Reading `.pptx`.
//!
//! Two kinds of check: a round trip through this project's own exporter, and
//! hand-built packages for the details that only a real PowerPoint file has —
//! placeholder inheritance, theme colours, group transforms.

use ai_export::{export, Format};
use ai_format::blocks::Kind;
use ai_format::model::{Items, ProjectType, Slide};
use ai_format::project::{create_project, save_project};
use ai_import::ooxml::Package;
use ai_import::Warnings;

mod fixture;
use fixture::{Builder, Workspace};

fn read_deck(bytes: &[u8]) -> Vec<Slide> {
    let package = Package::open(bytes).expect("a pptx package");
    let mut warnings = Warnings::default();
    ai_import::pptx::read(&package, &mut warnings)
        .expect("readable deck")
        .slides
}

/// The pictures an import stored, not counting the deck's preserved design.
fn pictures(assets: &[ai_import::Asset]) -> usize {
    assets
        .iter()
        .filter(|a| a.name != ai_import::pptx::TEMPLATE_ASSET)
        .count()
}

/* ------------------------------------------------------------- round trips */

#[test]
fn a_deck_this_project_wrote_reads_back_the_same() {
    let ws = Workspace::new("pptxround");
    let project = create_project(ws.path(), ProjectType::Deck, "왕복", true).unwrap();
    let Items::Slides(original) = &project.items else {
        panic!()
    };

    let slides = read_deck(&export(&project, Format::Pptx).unwrap());
    assert_eq!(slides.len(), original.len());

    for (before, after) in original.iter().zip(&slides) {
        assert_eq!(after.canvas.w, before.canvas.w);
        assert_eq!(after.canvas.h, before.canvas.h);
        assert_eq!(
            after.blocks.len(),
            before.blocks.len(),
            "slide “{}” lost or gained a block",
            before.title
        );
        for (b, a) in before.blocks.iter().zip(&after.blocks) {
            // Geometry is exact: EMU is a lossless carrier for these pixels.
            assert_eq!((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h), "geometry");
            assert_eq!(
                ai_format::mdblocks::plain_text(&a.md),
                ai_format::mdblocks::plain_text(&b.md),
                "text"
            );
        }
    }
}

#[test]
fn a_shape_survives_the_round_trip_with_its_preset() {
    let ws = Workspace::new("pptxshaperound");
    let mut project = create_project(ws.path(), ProjectType::Deck, "도형", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    let mut block = ai_format::deck::make_shape("flowChartDecision", fixture::geometry());
    block.md = "승인?".into();
    block.shape = Some(ai_format::shape::ShapeSpec {
        preset: "flowChartDecision".into(),
        fill: Some(ai_format::shape::Fill {
            color: "#dbeafe".into(),
            opacity: 100.0,
        }),
        line: Some(ai_format::shape::Line {
            color: "#2a78d6".into(),
            width: 2.0,
            dash: ai_format::shape::Dash::Dash,
        }),
        rotation: 15.0,
        flip_h: true,
        flip_v: false,
        adjust: [("adj".to_string(), 20000.0)].into_iter().collect(),
    });
    slides[0].blocks.push(block);
    let project = save_project(&project).unwrap();

    let slides = read_deck(&export(&project, Format::Pptx).unwrap());
    let shape = slides[0]
        .blocks
        .iter()
        .find(|b| b.kind == Kind::Shape)
        .expect("the shape came back");
    let spec = shape.shape.as_ref().unwrap();
    assert_eq!(spec.preset, "flowChartDecision");
    assert_eq!(spec.fill.as_ref().unwrap().color, "#dbeafe");
    assert_eq!(
        spec.line.as_ref().unwrap().dash,
        ai_format::shape::Dash::Dash
    );
    assert_eq!(spec.rotation, 15.0);
    assert!(spec.flip_h && !spec.flip_v);
    assert_eq!(spec.adjust.get("adj"), Some(&20000.0));
    assert_eq!(shape.md, "승인?");
}

#[test]
fn a_table_survives_the_round_trip_with_its_merges() {
    let ws = Workspace::new("pptxtableround");
    let mut project = create_project(ws.path(), ProjectType::Deck, "표", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    let mut block = ai_format::deck::make_table(3, 3, fixture::geometry());
    block.md = "| 항목 | 상반기 | |\n|---|---:|---:|\n| 제품 A | 1,200 | 1,350 |".into();
    block.table = Some(ai_format::table::TableSpec {
        cols: vec![240.0, 180.0, 180.0],
        merges: vec!["B1:C1".into()],
        header_row: true,
        first_col: true,
        ..Default::default()
    });
    slides[0].blocks.push(block);
    let project = save_project(&project).unwrap();

    let slides = read_deck(&export(&project, Format::Pptx).unwrap());
    let table = slides[0]
        .blocks
        .iter()
        .find(|b| b.kind == Kind::Table)
        .expect("the table came back");
    let spec = table.table.as_ref().unwrap();
    assert_eq!(spec.merges, vec!["B1:C1"]);
    assert_eq!(spec.cols, vec![240.0, 180.0, 180.0]);
    assert!(spec.header_row && spec.first_col);

    let cells = ai_format::table::parse_markdown_table(&table.md);
    assert_eq!(cells[0][0], "항목");
    assert_eq!(cells[1], vec!["제품 A", "1,200", "1,350"]);
    // The alignment row came back from the cell alignment.
    assert_eq!(spec.cells["B1"].align.as_deref(), Some("right"));
}

#[test]
fn a_chart_comes_back_with_its_numbers() {
    let ws = Workspace::new("pptxchartround");
    let mut project = create_project(ws.path(), ProjectType::Deck, "차트", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides[0].blocks.push(ai_format::model::SlideBlock {
        id: "b_chart".into(),
        kind: Kind::Chart,
        md: "```chart\n{\"type\":\"column\",\"title\":\"분기별 매출\",\"labels\":[\"1분기\",\"2분기\"],\"series\":[{\"name\":\"2025\",\"values\":[98,101]},{\"name\":\"2026\",\"values\":[120,133]}]}\n```".into(),
        x: 96.0, y: 200.0, w: 600.0, h: 360.0, z: 9.0,
        style: Default::default(), shape: None, table: None, locked: false,
    });
    let project = save_project(&project).unwrap();

    let slides = read_deck(&export(&project, Format::Pptx).unwrap());
    let chart = slides[0]
        .blocks
        .iter()
        .find(|b| b.kind == Kind::Chart)
        .expect("the chart came back");
    let spec = ai_format::chart::parse_chart_block(&chart.md).expect("a chart spec");
    assert_eq!(spec.chart_type, ai_format::chart::ChartType::Column);
    assert_eq!(spec.title, "분기별 매출");
    assert_eq!(spec.labels, ["1분기", "2분기"]);
    assert_eq!(spec.series.len(), 2);
    assert_eq!(spec.series[1].name, "2026");
    assert_eq!(spec.series[1].values, [Some(120.0), Some(133.0)]);
}

#[test]
fn speaker_notes_come_back() {
    let ws = Workspace::new("pptxnotesround");
    let mut project = create_project(ws.path(), ProjectType::Deck, "노트", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides[0].notes = "전년 동기 대비라는 점을 반드시 짚는다.".into();
    let project = save_project(&project).unwrap();

    let slides = read_deck(&export(&project, Format::Pptx).unwrap());
    assert_eq!(slides[0].notes, "전년 동기 대비라는 점을 반드시 짚는다.");
}

/* ----------------------------------------------- what only a real file has */

#[test]
fn a_placeholder_with_no_geometry_inherits_it_from_the_layout() {
    // The single most common shape in a real deck: a title with no `xfrm`.
    let deck = Builder::new()
        .layout(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/>
                 <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="838200" y="365125"/>
                 <a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr>
               </p:sp>"#,
        )
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>
                 <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
                 <p:spPr/>
                 <p:txBody><a:bodyPr/><a:p><a:r><a:t>상속된 제목</a:t></a:r></a:p></p:txBody>
               </p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    assert_eq!(slides[0].blocks.len(), 1);
    let block = &slides[0].blocks[0];
    assert_eq!(block.md, "상속된 제목");
    // 838200 EMU is 88px; 365125 rounds to 38.
    assert_eq!((block.x, block.y), (88.0, 38.0));
    assert_eq!(block.w, 1104.0);
    assert_eq!(slides[0].title, "상속된 제목");
}

#[test]
fn a_themed_shape_is_not_imported_invisible() {
    // Most PowerPoint shapes state no colours at all; they point at the theme.
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Rectangle 1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
                   <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom></p:spPr>
                 <p:style>
                   <a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef>
                   <a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef>
                 </p:style>
               </p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let spec = slides[0].blocks[0].shape.as_ref().expect("a shape");
    assert_eq!(spec.preset, "roundRect");
    assert_eq!(
        spec.fill.as_ref().map(|f| f.color.as_str()),
        Some("#4472c4"),
        "accent1 from the theme"
    );
    // The outline is accent1 shaded to half, as Office draws it.
    assert_eq!(
        spec.line.as_ref().map(|l| l.color.as_str()),
        Some("#223962")
    );
}

#[test]
fn an_explicitly_transparent_shape_stays_transparent() {
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Arrow"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm>
                   <a:prstGeom prst="rightArrow"><a:avLst/></a:prstGeom>
                   <a:noFill/></p:spPr>
                 <p:style><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef></p:style>
               </p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let spec = slides[0].blocks[0].shape.as_ref().unwrap();
    assert!(
        spec.fill.is_none(),
        "the theme must not repaint a noFill shape"
    );
}

#[test]
fn body_placeholders_get_the_masters_bullets_and_nothing_else_does() {
    let deck = Builder::new()
        .layout(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="3" name="Subtitle"/><p:cNvSpPr/>
                 <p:nvPr><p:ph type="subTitle" idx="1"/></p:nvPr></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/><a:lstStyle>
                   <a:lvl1pPr><a:buNone/></a:lvl1pPr>
                 </a:lstStyle><a:p/></p:txBody>
               </p:sp>"#,
        )
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Content"/><p:cNvSpPr/>
                 <p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/>
                   <a:p><a:r><a:t>본문 항목</a:t></a:r></a:p>
                   <a:p><a:pPr lvl="1"/><a:r><a:t>하위 항목</a:t></a:r></a:p>
                 </p:txBody></p:sp>
               <p:sp><p:nvSpPr><p:cNvPr id="3" name="Subtitle 1"/><p:cNvSpPr/>
                 <p:nvPr><p:ph type="subTitle" idx="1"/></p:nvPr></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="914400"/><a:ext cx="914400" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/><a:p><a:r><a:t>부제목</a:t></a:r></a:p></p:txBody>
               </p:sp>
               <p:sp><p:nvSpPr><p:cNvPr id="4" name="TextBox 1"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="1828800"/><a:ext cx="914400" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/><a:p><a:r><a:t>캡션</a:t></a:r></a:p></p:txBody>
               </p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let by_text = |needle: &str| {
        slides[0]
            .blocks
            .iter()
            .find(|b| b.md.contains(needle))
            .unwrap_or_else(|| panic!("no block containing {needle}"))
    };
    assert_eq!(by_text("본문 항목").md, "- 본문 항목\n  - 하위 항목");
    assert_eq!(by_text("부제목").md, "부제목", "the layout said buNone");
    assert_eq!(by_text("캡션").md, "캡션", "a text box follows otherStyle");
}

#[test]
fn grouped_shapes_land_where_they_are_drawn() {
    // A group maps a child coordinate space onto its own box. Ignoring that puts
    // every grouped shape in the wrong place, usually off-slide.
    let deck = Builder::new()
        .slide(
            r#"<p:grpSp>
                 <p:nvGrpSpPr><p:cNvPr id="2" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
                 <p:grpSpPr><a:xfrm>
                   <a:off x="914400" y="914400"/><a:ext cx="1828800" cy="914400"/>
                   <a:chOff x="0" y="0"/><a:chExt cx="3657600" cy="1828800"/>
                 </a:xfrm></p:grpSpPr>
                 <p:sp><p:nvSpPr><p:cNvPr id="3" name="Inner"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                   <p:spPr><a:xfrm><a:off x="1828800" y="914400"/>
                     <a:ext cx="914400" cy="457200"/></a:xfrm>
                     <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>
                     <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></p:spPr>
                 </p:sp>
               </p:grpSp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let block = &slides[0].blocks[0];
    // The child space is twice the group box, so the shape halves in size and
    // sits at the group's centre: 96 + 192*0.5 = 192, 96 + 96*0.5 = 144.
    assert_eq!((block.x, block.y), (192.0, 144.0));
    assert_eq!((block.w, block.h), (48.0, 24.0));
    assert_eq!(block.shape.as_ref().unwrap().preset, "ellipse");
}

#[test]
fn a_text_box_with_no_fill_is_text_not_a_bordered_shape() {
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox 1"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm>
                   <a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>
                 <p:txBody><a:bodyPr/><a:p><a:r><a:t>설명</a:t></a:r></a:p></p:txBody>
               </p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    assert_eq!(
        slides[0].blocks[0].kind,
        Kind::Text,
        "not a shape with a border"
    );
}

#[test]
fn emphasis_and_font_size_both_survive() {
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr anchor="ctr"/>
                   <a:p><a:pPr algn="ctr"/>
                     <a:r><a:rPr sz="2400" b="1"><a:solidFill><a:srgbClr val="1F2937"/></a:solidFill></a:rPr><a:t>굵게</a:t></a:r>
                     <a:r><a:rPr sz="2400"><a:solidFill><a:srgbClr val="1F2937"/></a:solidFill></a:rPr><a:t> 그리고 </a:t></a:r>
                     <a:r><a:rPr sz="2400" i="1"/><a:t>기울임</a:t></a:r>
                   </a:p>
                 </p:txBody></p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let block = &slides[0].blocks[0];
    assert_eq!(block.md, "**굵게** 그리고 *기울임*");
    // 2400 hundredths of a point is 24pt, which is 32px.
    assert_eq!(block.style["fontSize"], serde_json::json!(32.0));
    // Only the first run is bold; that lives in the `**` marks. Promoting it
    // to the block made the plain half of the sentence bold too.
    assert!(
        block.style.get("weight").is_none(),
        "mixed-run boldness must stay in the markdown: {:?}",
        block.style
    );
    assert_eq!(block.style["align"], serde_json::json!("center"));
    assert_eq!(block.style["valign"], serde_json::json!("middle"));
    assert_eq!(block.style["color"], serde_json::json!("#1f2937"));
}

#[test]
fn slides_keep_the_order_the_id_list_gives() {
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="a"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                  <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
                  <p:txBody><a:bodyPr/><a:p><a:r><a:t>첫째</a:t></a:r></a:p></p:txBody></p:sp>"#,
        )
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="b"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                  <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
                  <p:txBody><a:bodyPr/><a:p><a:r><a:t>둘째</a:t></a:r></a:p></p:txBody></p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    assert_eq!(slides.len(), 2);
    assert_eq!(slides[0].blocks[0].md, "첫째");
    assert_eq!(slides[1].blocks[0].md, "둘째");
}

#[test]
fn a_gradient_fill_becomes_the_colour_it_reads_as() {
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="G"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
                   <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                   <a:gradFill><a:gsLst>
                     <a:gs pos="0"><a:srgbClr val="DBEAFE"/></a:gs>
                     <a:gs pos="100000"><a:srgbClr val="1E40AF"/></a:gs>
                   </a:gsLst></a:gradFill></p:spPr></p:sp>"#,
        )
        .build();

    let package = Package::open(&deck).unwrap();
    let mut warnings = Warnings::default();
    let deck = ai_import::pptx::read(&package, &mut warnings).unwrap();
    let spec = deck.slides[0].blocks[0].shape.as_ref().unwrap();
    // The average of the stops, not the first one: a gradient from #dbeafe to
    // #1e40af reads as the blue in the middle, and taking the first stop would
    // draw a shape far lighter than the slide showed.
    assert_eq!(
        spec.fill.as_ref().map(|f| f.color.as_str()),
        Some("#7d95d7")
    );
    let warnings = warnings.into_vec();
    assert!(
        warnings.iter().any(|w| w.contains("그라데이션")),
        "the user should be told: {warnings:?}"
    );
}

#[test]
fn an_object_with_nothing_to_convert_is_reported() {
    // A diagram frame with no model and no drawing: there is nothing to turn
    // into shapes or words, and silence would leave the reader hunting.
    let deck = Builder::new()
        .slide(
            r#"<p:graphicFrame>
                 <p:nvGraphicFramePr><p:cNvPr id="2" name="Diagram"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
                 <p:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></p:xfrm>
                 <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic>
               </p:graphicFrame>"#,
        )
        .build();

    let package = Package::open(&deck).unwrap();
    let mut warnings = Warnings::default();
    ai_import::pptx::read(&package, &mut warnings).unwrap();
    let warnings = warnings.into_vec();
    // Named rather than lumped together: the reader is told which thing on their
    // slide is missing.
    assert!(
        warnings.iter().any(|w| w == "SmartArt는 넘어오지 않습니다"),
        "{warnings:?}"
    );
}

/* ----------------------------------------------------- inherited text size */

/// A template's real font sizes: 44pt titles, 32pt body, 18pt everything else.
const SIZED_MASTER: &str = "<p:txStyles>\
   <p:titleStyle><a:lvl1pPr><a:buNone/><a:defRPr sz=\"4400\" b=\"1\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill></a:defRPr></a:lvl1pPr></p:titleStyle>\
   <p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"3200\"/></a:lvl1pPr><a:lvl2pPr><a:defRPr sz=\"2800\"/></a:lvl2pPr></p:bodyStyle>\
   <p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1800\"/></a:lvl1pPr></p:otherStyle>\
 </p:txStyles>";

fn placeholder(kind: &str, body: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"ph\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"{kind}\"/></p:nvPr></p:nvSpPr>\
         <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"3048000\" cy=\"1143000\"/></a:xfrm></p:spPr>\
         <p:txBody>{body}</p:txBody></p:sp>"
    )
}

#[test]
fn a_title_takes_its_size_from_the_master() {
    // No `sz` anywhere on the run — 44pt lives in the master's `titleStyle`, and
    // a deck that states its sizes on runs does not exist.
    let bytes = Builder::new()
        .master_styles(SIZED_MASTER)
        .slide(&placeholder(
            "title",
            "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>",
        ))
        .build();
    let slides = read_deck(&bytes);
    let block = &slides[0].blocks[0];
    assert_eq!(
        block.style["fontSize"],
        serde_json::json!(59.0),
        "44pt is 58.67px"
    );
    assert_eq!(block.style["weight"], serde_json::json!(700));
    assert_eq!(
        block.style["color"],
        serde_json::json!("#000000"),
        "`tx1` maps to `dk1`, which the theme states as black"
    );
}

#[test]
fn a_layout_overrides_the_masters_size_for_one_placeholder() {
    let layout = "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"ph\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\
        <p:spPr/><p:txBody><a:lstStyle><a:lvl1pPr><a:defRPr sz=\"3200\"/></a:lvl1pPr></a:lstStyle></p:txBody></p:sp>";
    let bytes = Builder::new()
        .master_styles(SIZED_MASTER)
        .layout(layout)
        .slide(&placeholder(
            "title",
            "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>",
        ))
        .build();
    let block = &read_deck(&bytes)[0].blocks[0];
    assert_eq!(block.style["fontSize"], serde_json::json!(43.0), "32pt");
    assert_eq!(
        block.style["weight"],
        serde_json::json!(700),
        "the layout changed only the size, so the master's bold still applies"
    );
}

#[test]
fn a_second_level_bullet_takes_the_second_levels_size() {
    let bytes = Builder::new()
        .master_styles(SIZED_MASTER)
        .slide(&placeholder(
            "body",
            "<a:bodyPr/><a:p><a:pPr lvl=\"1\"/><a:r><a:t>하위 항목</a:t></a:r></a:p>",
        ))
        .build();
    let block = &read_deck(&bytes)[0].blocks[0];
    assert_eq!(block.style["fontSize"], serde_json::json!(37.0), "28pt");
}

#[test]
fn a_bold_off_run_beats_the_masters_bold() {
    let bytes = Builder::new()
        .master_styles(SIZED_MASTER)
        .slide(&placeholder(
            "title",
            "<a:bodyPr/><a:p><a:r><a:rPr b=\"0\"/><a:t>제목</a:t></a:r></a:p>",
        ))
        .build();
    let block = &read_deck(&bytes)[0].blocks[0];
    assert!(
        !block.style.contains_key("weight"),
        "`b=\"0\"` states not-bold, which is not the same as stating nothing"
    );
}

#[test]
fn autofit_shrinks_the_size_the_way_powerpoint_did() {
    // PowerPoint fitted this text at 62.5% and recorded it. Ignoring the scale
    // imports the shape overflowing, in the one case where the author had
    // already run out of room.
    let bytes = Builder::new()
        .master_styles(SIZED_MASTER)
        .slide(&placeholder(
            "body",
            "<a:bodyPr><a:normAutofit fontScale=\"62500\" lnSpcReduction=\"20000\"/></a:bodyPr>\
             <a:p><a:pPr><a:lnSpc><a:spcPct val=\"150000\"/></a:lnSpc></a:pPr><a:r><a:t>긴 글</a:t></a:r></a:p>",
        ))
        .build();
    let block = &read_deck(&bytes)[0].blocks[0];
    assert_eq!(
        block.style["fontSize"],
        serde_json::json!(27.0),
        "32pt at 62.5% is 26.67px"
    );
    assert_eq!(
        block.style["lineHeight"],
        serde_json::json!(1.44),
        "1.5 lines is 1.8 in CSS terms, reduced by 20%"
    );
}

#[test]
fn a_text_box_takes_the_masters_other_style() {
    // A shape with no `p:ph` follows `otherStyle`. PowerPoint's own default there
    // is 18pt; without reading it every imported caption comes in at the app's
    // own body size.
    let sp = "<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"tb\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
        <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"1828800\" cy=\"457200\"/></a:xfrm></p:spPr>\
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>설명</a:t></a:r></a:p></p:txBody></p:sp>";
    let bytes = Builder::new().master_styles(SIZED_MASTER).slide(sp).build();
    let block = &read_deck(&bytes)[0].blocks[0];
    assert_eq!(block.style["fontSize"], serde_json::json!(24.0), "18pt");
}

#[test]
fn a_master_with_no_other_style_still_draws_a_text_box_at_powerpoints_default() {
    let sp = "<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"tb\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
        <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"1828800\" cy=\"457200\"/></a:xfrm></p:spPr>\
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>설명</a:t></a:r></a:p></p:txBody></p:sp>";
    // The default fixture master has an empty `otherStyle`.
    let block = &read_deck(&Builder::new().slide(sp).build())[0].blocks[0];
    assert_eq!(block.style["fontSize"], serde_json::json!(24.0), "18pt");
}

#[test]
fn the_fonts_a_file_asks_for_are_reported_once() {
    let sp = "<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"tb\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
        <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"1828800\" cy=\"457200\"/></a:xfrm></p:spPr>\
        <p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:latin typeface=\"맑은 고딕\"/></a:rPr><a:t>설명</a:t></a:r></a:p></p:txBody></p:sp>";
    let bytes = Builder::new().slide(sp).build();
    let imported = ai_import::read(&bytes, "deck.pptx").expect("importable");
    let font_notes: Vec<&String> = imported
        .warnings
        .iter()
        .filter(|w| w.contains("글꼴"))
        .collect();
    assert_eq!(font_notes.len(), 1, "one line, however many families");
    assert!(font_notes[0].contains("맑은 고딕"), "{}", font_notes[0]);
    assert!(font_notes[0].contains(ai_format::font::FAMILY));
}

/* ------------------------------------------------------------ design layer */

/// A banner on the layout and a dark background on the master — where every
/// Office theme keeps them.
const BANNER: &str = "<p:sp><p:nvSpPr><p:cNvPr id=\"9\" name=\"띠\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
   <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"12192000\" cy=\"457200\"/></a:xfrm>\
     <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\
     <a:solidFill><a:srgbClr val=\"1F3864\"/></a:solidFill></p:spPr><p:txBody><a:bodyPr/><a:p/></p:txBody></p:sp>";

/// A layout placeholder with its prompt text, which must not become content.
const PROMPT: &str = "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"ph\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\
   <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"3048000\" cy=\"1143000\"/></a:xfrm></p:spPr>\
   <p:txBody><a:bodyPr/><a:p><a:r><a:t>제목을 입력하십시오</a:t></a:r></a:p></p:txBody></p:sp>";

#[test]
fn the_layouts_own_shapes_come_across_behind_the_slide() {
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    let bytes = Builder::new()
        .layout(&format!("{BANNER}{PROMPT}"))
        .slide(&slide)
        .build();
    let blocks = &read_deck(&bytes)[0].blocks;

    assert_eq!(
        blocks.len(),
        2,
        "the banner and the title, not the layout's prompt: {:?}",
        blocks.iter().map(|b| b.md.clone()).collect::<Vec<_>>()
    );
    assert_eq!(blocks[0].kind, Kind::Shape, "the banner");
    assert_eq!(
        blocks[0]
            .shape
            .as_ref()
            .unwrap()
            .fill
            .as_ref()
            .unwrap()
            .color,
        "#1f3864"
    );
    assert!(
        blocks[0].z < blocks[1].z,
        "the design sits behind the content"
    );
    assert_eq!(ai_format::mdblocks::plain_text(&blocks[1].md), "제목");
    assert!(
        !blocks.iter().any(|b| b.md.contains("입력하십시오")),
        "a layout placeholder's prompt is not content"
    );
}

#[test]
fn a_slide_can_turn_the_masters_shapes_off() {
    // `showMasterSp="0"` is what PowerPoint writes for a full-bleed slide, and
    // drawing the banner over it anyway is worse than not drawing it at all.
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    let bytes = Builder::new()
        .layout(BANNER)
        .slide_attrs(" showMasterSp=\"0\"")
        .slide(&slide)
        .build();
    let blocks = &read_deck(&bytes)[0].blocks;
    assert_eq!(blocks.len(), 1, "only the slide's own content");
}

#[test]
fn a_background_stated_on_the_layout_reaches_the_slide() {
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    let bytes = Builder::new()
        .layout_bg(
            "<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"0F172A\"/></a:solidFill></p:bgPr></p:bg>",
        )
        .slide(&slide)
        .build();
    assert_eq!(read_deck(&bytes)[0].canvas.bg.as_str(), "#0f172a");
}

#[test]
fn a_themed_background_reference_resolves_to_a_colour() {
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    let bytes = Builder::new()
        .layout_bg("<p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"accent1\"/></p:bgRef></p:bg>")
        .slide(&slide)
        .build();
    assert_eq!(
        read_deck(&bytes)[0].canvas.bg.as_str(),
        "#4472c4",
        "accent1 in the fixture's theme"
    );
}

/* ------------------------------------------- objects with no exact equivalent */

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The frame a SmartArt diagram sits in.
const DIAGRAM_FRAME: &str = "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"5\" name=\"도해\"/></p:nvGraphicFramePr>\
   <p:xfrm><a:off x=\"914400\" y=\"914400\"/><a:ext cx=\"5486400\" cy=\"3200400\"/></p:xfrm>\
   <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/diagram\">\
     <dgm:relIds xmlns:dgm=\"http://schemas.openxmlformats.org/drawingml/2006/diagram\" \
       xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:dm=\"rIdDm\"/>\
   </a:graphicData></a:graphic></p:graphicFrame>";

#[test]
fn smartart_becomes_the_shapes_powerpoint_drew_for_it() {
    // Office stores the laid-out shapes beside the diagram model, for readers
    // that cannot lay a diagram out. They are the same boxes, colours and words,
    // so they are what an imported slide should show.
    let drawing = "<?xml version=\"1.0\"?><dsp:drawing xmlns:dsp=\"http://schemas.microsoft.com/office/drawing/2008/diagram\" \
        xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><dsp:spTree>\
        <dsp:sp><dsp:nvSpPr><dsp:cNvPr id=\"1\" name=\"단계 1\"/><dsp:cNvSpPr/></dsp:nvSpPr>\
          <dsp:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"1828800\" cy=\"914400\"/></a:xfrm>\
            <a:prstGeom prst=\"roundRect\"><a:avLst/></a:prstGeom>\
            <a:solidFill><a:srgbClr val=\"4472C4\"/></a:solidFill></dsp:spPr>\
          <dsp:txBody><a:bodyPr/><a:p><a:r><a:t>기획</a:t></a:r></a:p></dsp:txBody></dsp:sp>\
        <dsp:sp><dsp:nvSpPr><dsp:cNvPr id=\"2\" name=\"단계 2\"/><dsp:cNvSpPr/></dsp:nvSpPr>\
          <dsp:spPr><a:xfrm><a:off x=\"2286000\" y=\"0\"/><a:ext cx=\"1828800\" cy=\"914400\"/></a:xfrm>\
            <a:prstGeom prst=\"roundRect\"><a:avLst/></a:prstGeom>\
            <a:solidFill><a:srgbClr val=\"ED7D31\"/></a:solidFill></dsp:spPr>\
          <dsp:txBody><a:bodyPr/><a:p><a:r><a:t>실행</a:t></a:r></a:p></dsp:txBody></dsp:sp>\
        </dsp:spTree></dsp:drawing>";
    let data = "<?xml version=\"1.0\"?><dgm:dataModel xmlns:dgm=\"http://schemas.openxmlformats.org/drawingml/2006/diagram\" \
        xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><dgm:ptLst>\
        <dgm:pt modelId=\"1\"><dgm:t><a:p><a:r><a:t>기획</a:t></a:r></a:p></dgm:t></dgm:pt>\
        <dgm:pt modelId=\"2\"><dgm:t><a:p><a:r><a:t>실행</a:t></a:r></a:p></dgm:t></dgm:pt>\
        </dgm:ptLst></dgm:dataModel>";

    let bytes = Builder::new()
        .slide(DIAGRAM_FRAME)
        .slide_rels(&format!(
            "<Relationship Id=\"rIdDm\" Type=\"{REL}/diagramData\" Target=\"../diagrams/data1.xml\"/>"
        ))
        .part("ppt/diagrams/data1.xml", data)
        .part(
            "ppt/diagrams/_rels/data1.xml.rels",
            &format!(
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                 <Relationship Id=\"rIdDrawing\" Type=\"{REL}/diagramDrawing\" Target=\"../drawings/drawing1.xml\"/></Relationships>"
            ),
        )
        .part("ppt/drawings/drawing1.xml", drawing)
        .build();

    let slides = read_deck(&bytes);
    let blocks = &slides[0].blocks;
    assert_eq!(blocks.len(), 2, "one block per diagram shape");
    assert_eq!(blocks[0].kind, Kind::Shape);
    assert_eq!(blocks[0].shape.as_ref().unwrap().preset, "roundRect");
    assert_eq!(
        blocks[0]
            .shape
            .as_ref()
            .unwrap()
            .fill
            .as_ref()
            .unwrap()
            .color,
        "#4472c4"
    );
    assert_eq!(ai_format::mdblocks::plain_text(&blocks[0].md), "기획");
    // Placed inside the frame: the frame starts at 96px, and the first shape at
    // the frame's own origin.
    assert_eq!((blocks[0].x, blocks[0].y), (96.0, 96.0));
    assert_eq!(blocks[1].x, 96.0 + 240.0, "the second shape beside it");

    let imported = ai_import::read(&bytes, "deck.pptx").expect("importable");
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("SmartArt는 같은 모양의 도형들로")),
        "{:?}",
        imported.warnings
    );
}

#[test]
fn a_diagram_with_no_drawing_part_becomes_its_words() {
    // A file written by something other than Office may have the model without
    // the rendered shapes. The labels are still the content, and a bulleted list
    // of them is the closest this format has.
    let data = "<?xml version=\"1.0\"?><dgm:dataModel xmlns:dgm=\"http://schemas.openxmlformats.org/drawingml/2006/diagram\" \
        xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><dgm:ptLst>\
        <dgm:pt modelId=\"1\"><dgm:t><a:p><a:r><a:t>기획</a:t></a:r></a:p></dgm:t></dgm:pt>\
        <dgm:pt modelId=\"2\"><dgm:t><a:p><a:r><a:t>실행</a:t></a:r></a:p></dgm:t></dgm:pt>\
        <dgm:pt modelId=\"3\"><dgm:t><a:p><a:r><a:t>검토</a:t></a:r></a:p></dgm:t></dgm:pt>\
        </dgm:ptLst></dgm:dataModel>";
    let bytes = Builder::new()
        .slide(DIAGRAM_FRAME)
        .slide_rels(&format!(
            "<Relationship Id=\"rIdDm\" Type=\"{REL}/diagramData\" Target=\"../diagrams/data1.xml\"/>"
        ))
        .part("ppt/diagrams/data1.xml", data)
        .build();

    let blocks = &read_deck(&bytes)[0].blocks;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].md, "- 기획\n- 실행\n- 검토");
}

#[test]
fn an_embedded_object_becomes_the_picture_office_drew_for_it() {
    // An embedded workbook or equation is stored with the image Office shows in
    // its place — which is what the slide looked like.
    let frame = "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"6\" name=\"개체\"/></p:nvGraphicFramePr>\
       <p:xfrm><a:off x=\"457200\" y=\"457200\"/><a:ext cx=\"3657600\" cy=\"2743200\"/></p:xfrm>\
       <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/presentationml/2006/ole\">\
         <p:oleObj spid=\"_x0000_s1026\" name=\"Worksheet\" r:id=\"rIdOle\" imgW=\"3657600\" imgH=\"2743200\">\
           <p:pic><p:nvPicPr><p:cNvPr id=\"0\" name=\"미리보기\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>\
             <p:blipFill><a:blip r:embed=\"rIdImage\"/></p:blipFill>\
             <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"3657600\" cy=\"2743200\"/></a:xfrm></p:spPr>\
           </p:pic></p:oleObj>\
       </a:graphicData></a:graphic></p:graphicFrame>";
    let bytes = Builder::new()
        .slide(frame)
        .slide_rels(&format!(
            "<Relationship Id=\"rIdImage\" Type=\"{REL}/image\" Target=\"../media/image1.png\"/>"
        ))
        .part("ppt/media/image1.png", "PNG-BYTES")
        .build();

    let imported = ai_import::read(&bytes, "deck.pptx").expect("importable");
    let ai_format::model::Items::Slides(slides) = &imported.items else {
        panic!()
    };
    let blocks = &slides[0].blocks;
    assert_eq!(blocks.len(), 1, "the preview picture came across");
    assert_eq!(blocks[0].kind, Kind::Image);
    assert_eq!(blocks[0].x, 48.0, "placed where the object was");
    assert_eq!(pictures(&imported.assets), 1, "and the image was stored");
    assert!(
        imported.warnings.iter().any(|w| w.contains("OLE 개체는")),
        "{:?}",
        imported.warnings
    );
}

#[test]
fn the_layouts_footer_and_slide_number_become_text() {
    // A template's footer and page number live on the layout as placeholders, and
    // they are what a reader sees at the bottom of every slide. The slide number
    // is the slide's own, not the layout's `‹#›`.
    let footers = "<p:sp><p:nvSpPr><p:cNvPr id=\"10\" name=\"바닥글\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"ftr\" idx=\"11\"/></p:nvPr></p:nvSpPr>\
       <p:spPr><a:xfrm><a:off x=\"457200\" y=\"6248400\"/><a:ext cx=\"2743200\" cy=\"365125\"/></a:xfrm></p:spPr>\
       <p:txBody><a:bodyPr/><a:p><a:r><a:t>AI Studio · 대외비</a:t></a:r></a:p></p:txBody></p:sp>\
     <p:sp><p:nvSpPr><p:cNvPr id=\"11\" name=\"슬라이드 번호\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"sldNum\" idx=\"12\"/></p:nvPr></p:nvSpPr>\
       <p:spPr><a:xfrm><a:off x=\"11049000\" y=\"6248400\"/><a:ext cx=\"685800\" cy=\"365125\"/></a:xfrm></p:spPr>\
       <p:txBody><a:bodyPr/><a:p><a:fld id=\"{1}\" type=\"slidenum\"><a:t>\u{2039}#\u{203a}</a:t></a:fld></a:p></p:txBody></p:sp>";
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    let bytes = Builder::new()
        .layout(footers)
        // A template that shows the number says so; the placeholder alone is
        // furniture every default template carries.
        .layout_hf("<p:hf sldNum=\"1\" dt=\"0\"/>")
        .slide(&slide)
        .slide(&slide)
        .build();

    let slides = read_deck(&bytes);
    for (index, slide) in slides.iter().enumerate() {
        let text: Vec<String> = slide
            .blocks
            .iter()
            .map(|b| ai_format::mdblocks::plain_text(&b.md))
            .collect();
        assert!(
            text.contains(&"AI Studio · 대외비".to_string()),
            "slide {}: {text:?}",
            index + 1
        );
        assert!(
            text.contains(&(index + 1).to_string()),
            "the slide's own number, not the layout's placeholder: {text:?}"
        );
    }
}

#[test]
fn a_footer_on_both_the_master_and_the_layout_is_drawn_once() {
    let footer = |text: &str| {
        format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"10\" name=\"바닥글\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"ftr\" idx=\"11\"/></p:nvPr></p:nvSpPr>\
             <p:spPr><a:xfrm><a:off x=\"457200\" y=\"6248400\"/><a:ext cx=\"2743200\" cy=\"365125\"/></a:xfrm></p:spPr>\
             <p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"
        )
    };
    // The master's text is the default; the layout's overrides it for this
    // layout, which is how PowerPoint resolves the pair.
    let bytes = Builder::new()
        .master_shapes(&footer("마스터 바닥글"))
        .layout(&footer("레이아웃 바닥글"))
        .slide(&placeholder(
            "title",
            "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>",
        ))
        .build();

    let blocks = &read_deck(&bytes)[0].blocks;
    let footers: Vec<&str> = blocks
        .iter()
        .filter(|b| b.md.contains("바닥글"))
        .map(|b| b.md.as_str())
        .collect();
    assert_eq!(footers, ["레이아웃 바닥글"], "one footer, the layout's");
}

#[test]
fn a_shape_filled_with_a_picture_becomes_the_picture() {
    // A photo cropped into a shape is a photo, and a grey box with the photo's
    // average colour is not what the slide showed.
    let sp = "<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"사진\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
       <p:spPr><a:xfrm><a:off x=\"457200\" y=\"457200\"/><a:ext cx=\"2743200\" cy=\"1828800\"/></a:xfrm>\
         <a:prstGeom prst=\"roundRect\"><a:avLst/></a:prstGeom>\
         <a:blipFill><a:blip r:embed=\"rIdImage\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"/></a:blipFill>\
       </p:spPr><p:txBody><a:bodyPr/><a:p/></p:txBody></p:sp>";
    let bytes = Builder::new()
        .slide(sp)
        .slide_rels(&format!(
            "<Relationship Id=\"rIdImage\" Type=\"{REL}/image\" Target=\"../media/image1.png\"/>"
        ))
        .part("ppt/media/image1.png", "PNG-BYTES")
        .build();

    let imported = ai_import::read(&bytes, "deck.pptx").expect("importable");
    let ai_format::model::Items::Slides(slides) = &imported.items else {
        panic!()
    };
    let blocks = &slides[0].blocks;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, Kind::Image);
    assert_eq!((blocks[0].x, blocks[0].w), (48.0, 288.0), "the shape's box");
    assert_eq!(pictures(&imported.assets), 1);
}

#[test]
fn a_picture_keeps_its_crop_rotation_and_flip() {
    // A photo tilted 30° and trimmed to its subject is placed that way on the
    // slide; reading it flat re-frames the picture.
    let pic = "<p:pic><p:nvPicPr><p:cNvPr id=\"4\" name=\"사진\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>\
       <p:blipFill><a:blip r:embed=\"rIdImage\"/><a:srcRect l=\"10000\" t=\"5000\" r=\"0\" b=\"0\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
       <p:spPr><a:xfrm rot=\"1800000\" flipH=\"1\"><a:off x=\"457200\" y=\"457200\"/><a:ext cx=\"2743200\" cy=\"1828800\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>";
    let bytes = Builder::new()
        .slide(pic)
        .slide_rels(&format!(
            "<Relationship Id=\"rIdImage\" Type=\"{REL}/image\" Target=\"../media/image1.png\"/>"
        ))
        .part("ppt/media/image1.png", "PNG-BYTES")
        .build();

    let block = &read_deck(&bytes)[0].blocks[0];
    assert_eq!(block.kind, Kind::Image);
    assert_eq!(block.style["rotation"], serde_json::json!(30.0));
    assert_eq!(block.style["flipH"], serde_json::json!(true));
    assert!(
        !block.style.contains_key("flipV"),
        "no flip that was not set"
    );
    let crop = &block.style["crop"];
    assert_eq!(crop["l"], serde_json::json!(10.0));
    assert_eq!(crop["t"], serde_json::json!(5.0));
}

#[test]
fn an_emf_image_is_reported_as_maybe_invisible() {
    // EMF/WMF vector images cannot be drawn on the web canvas. The bytes are
    // still stored, but the author is told the picture may not show.
    let pic = "<p:pic><p:nvPicPr><p:cNvPr id=\"5\" name=\"도표\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>\
       <p:blipFill><a:blip r:embed=\"rIdImage\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
       <p:spPr><a:xfrm><a:off x=\"457200\" y=\"457200\"/><a:ext cx=\"2743200\" cy=\"1828800\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>";
    let bytes = Builder::new()
        .slide(pic)
        .slide_rels(&format!(
            "<Relationship Id=\"rIdImage\" Type=\"{REL}/image\" Target=\"../media/image1.emf\"/>"
        ))
        .part("ppt/media/image1.emf", "EMF-BYTES")
        .build();

    let imported = ai_import::read(&bytes, "deck.pptx").expect("importable");
    assert_eq!(pictures(&imported.assets), 1, "the bytes are still stored");
    assert!(
        imported.warnings.iter().any(|w| w.contains("EMF")),
        "{:?}",
        imported.warnings
    );
}

/* ------------------------------------------------------- slide sizes */

#[test]
fn a_deck_opens_at_whatever_size_it_was_made_at() {
    // People make decks at every size there is: 4:3 for a projector, A4 for a
    // printed handout, a square for social, and whatever they typed. Each one
    // has to open at its own size — a 4:3 deck shown on a 16:9 canvas puts every
    // shape in the wrong place.
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    for (cx, cy, expected) in [
        // 10 x 7.5in — the standard 4:3 PowerPoint has had since 1987.
        (9_144_000, 6_858_000, (960.0, 720.0)),
        // 13.333 x 7.5in widescreen.
        (12_192_000, 6_858_000, (1280.0, 720.0)),
        // A4 portrait, as a handout template is built.
        (7_099_300, 10_058_400, (745.0, 1056.0)),
        // Something the author typed: 24 x 18cm.
        (8_640_000, 6_480_000, (907.0, 680.0)),
    ] {
        let slides = read_deck(&Builder::new().size(cx, cy).slide(&slide).build());
        assert_eq!(
            (slides[0].canvas.w, slides[0].canvas.h),
            expected,
            "sldSz {cx}x{cy}"
        );
    }
}

#[test]
fn a_four_by_three_deck_keeps_its_size_through_a_round_trip() {
    // Import, save, export, import again: the size has to survive all four, or a
    // deck changes shape every time it passes through.
    let ws = Workspace::new("pptxratio");
    let mut project = create_project(ws.path(), ProjectType::Deck, "표준 비율", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides[0].canvas = ai_format::geometry::Canvas {
        w: 960.0,
        h: 720.0,
        bg: "#ffffff".to_string().into(),
    };
    // A shape at the far right of the 4:3 canvas: on a 16:9 canvas it would be
    // nowhere near the edge, which is how a wrong size shows itself.
    let mut block = ai_format::deck::make_shape(
        "rect",
        ai_format::geometry::Box {
            x: 760.0,
            y: 600.0,
            w: 160.0,
            h: 80.0,
            z: 1.0,
        },
    );
    block.md = "우측 하단".into();
    slides[0].blocks.push(block);
    let project = save_project(&project).unwrap();

    let again = read_deck(&export(&project, Format::Pptx).unwrap());
    assert_eq!((again[0].canvas.w, again[0].canvas.h), (960.0, 720.0));
    let corner = again[0]
        .blocks
        .iter()
        .find(|b| b.md.contains("우측 하단"))
        .expect("the shape came back");
    assert_eq!((corner.x, corner.y), (760.0, 600.0), "still in the corner");
}

#[test]
fn a_portrait_deck_stays_portrait() {
    let ws = Workspace::new("pptxportrait");
    let mut project = create_project(ws.path(), ProjectType::Deck, "세로 덱", false).unwrap();
    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides[0].canvas = ai_format::geometry::Canvas {
        w: 745.0,
        h: 1056.0,
        bg: "#ffffff".to_string().into(),
    };
    let project = save_project(&project).unwrap();

    let bytes = export(&project, Format::Pptx).unwrap();
    let package = Package::open(&bytes).unwrap();
    let presentation = package.xml("ppt/presentation.xml").unwrap();
    let size = presentation
        .path(&["presentation", "sldSz"])
        .expect("a slide size");
    // 745px is 7099300 EMU; the notes page is the other way round, as
    // PowerPoint writes it.
    assert!(
        size.attr_i64("cy").unwrap() > size.attr_i64("cx").unwrap(),
        "taller than it is wide: {:?}",
        size.attrs
    );
    assert_eq!(
        (read_deck(&bytes)[0].canvas.w, read_deck(&bytes)[0].canvas.h),
        (745.0, 1056.0)
    );
}

#[test]
fn an_empty_footer_placeholder_is_not_drawn() {
    // Every Office template carries a date, a footer and a slide-number
    // placeholder holding nothing but a field. Importing those stamps a stale
    // date and a page number onto a deck that never showed either.
    let furniture = "<p:sp><p:nvSpPr><p:cNvPr id=\"10\" name=\"날짜\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"dt\" idx=\"10\"/></p:nvPr></p:nvSpPr>\
       <p:spPr/><p:txBody><a:bodyPr/><a:p><a:fld id=\"{2}\" type=\"datetime1\"><a:t>1/27/13</a:t></a:fld></a:p></p:txBody></p:sp>\
     <p:sp><p:nvSpPr><p:cNvPr id=\"11\" name=\"바닥글\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"ftr\" idx=\"11\"/></p:nvPr></p:nvSpPr>\
       <p:spPr/><p:txBody><a:bodyPr/><a:p/></p:txBody></p:sp>\
     <p:sp><p:nvSpPr><p:cNvPr id=\"12\" name=\"번호\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"sldNum\" idx=\"12\"/></p:nvPr></p:nvSpPr>\
       <p:spPr/><p:txBody><a:bodyPr/><a:p><a:fld id=\"{3}\" type=\"slidenum\"><a:t>\u{2039}#\u{203a}</a:t></a:fld></a:p></p:txBody></p:sp>";
    let slide = placeholder("title", "<a:bodyPr/><a:p><a:r><a:t>제목</a:t></a:r></a:p>");
    let blocks = &read_deck(&Builder::new().layout(furniture).slide(&slide).build())[0].blocks;

    assert_eq!(blocks.len(), 1, "only the slide's own title: {blocks:?}");
    assert_eq!(ai_format::mdblocks::plain_text(&blocks[0].md), "제목");
}

#[test]
fn a_placeholder_laid_out_for_another_slide_size_is_brought_onto_the_canvas() {
    // Change the slide size without touching the layout — which is what every
    // tool but PowerPoint does — and the placeholders are still positioned for
    // the old size. A title 864px wide cannot be shown on a 745px slide.
    let layout = "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"제목\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\
       <p:spPr><a:xfrm><a:off x=\"457200\" y=\"274638\"/><a:ext cx=\"8229600\" cy=\"1143000\"/></a:xfrm></p:spPr>\
       <p:txBody><a:bodyPr/><a:p/></p:txBody></p:sp>";
    // A4 portrait: 745 x 1056px.
    let bytes = Builder::new()
        .size(7_099_300, 10_058_400)
        .layout(layout)
        .slide(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"제목\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\
             <p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>세로 덱</a:t></a:r></a:p></p:txBody></p:sp>",
        )
        .build();

    let slides = read_deck(&bytes);
    let block = &slides[0].blocks[0];
    assert_eq!((slides[0].canvas.w, slides[0].canvas.h), (745.0, 1056.0));
    assert!(
        block.x >= 0.0 && block.x + block.w <= 745.0,
        "on the canvas: x={} w={}",
        block.x,
        block.w
    );
}

#[test]
fn a_scheme_colour_solid_fill_is_resolved_through_the_theme() {
    // `tx1` lightened with lumMod/lumOff is how PowerPoint writes most greys.
    // Seeing no srgbClr in it left the fill empty, and a label chip's white
    // text vanished into the white slide.
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Chip"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm>
                   <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>
                   <a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill>
                 </p:spPr>
                 <p:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1400" b="1"><a:solidFill><a:schemeClr val="bg1"/></a:solidFill></a:rPr><a:t>과제 수행 배경</a:t></a:r></a:p></p:txBody>
               </p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let spec = slides[0].blocks[0].shape.as_ref().expect("a shape");
    assert_eq!(
        spec.fill.as_ref().map(|f| f.color.as_str()),
        Some("#595959"),
        "black at 65% + 35% lift is the dark grey PowerPoint draws"
    );
}

#[test]
fn a_blocks_colour_is_the_one_most_of_its_text_uses() {
    // A short burgundy sub-heading over a grey body must not paint the whole
    // block burgundy just because it came first.
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="1828800"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/>
                   <a:p><a:r><a:rPr sz="1300" b="1"><a:solidFill><a:srgbClr val="A50034"/></a:solidFill></a:rPr><a:t>핵심</a:t></a:r></a:p>
                   <a:p><a:r><a:rPr sz="1000"><a:solidFill><a:srgbClr val="202124"/></a:solidFill></a:rPr><a:t>장시간 쿼리가 반복되어 담당자 대기 시간이 길어진다</a:t></a:r></a:p>
                   <a:p><a:r><a:rPr sz="1000"><a:solidFill><a:srgbClr val="202124"/></a:solidFill></a:rPr><a:t>요청 양식이 팀마다 달라 재작업이 잦다</a:t></a:r></a:p>
                 </p:txBody></p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let block = &slides[0].blocks[0];
    assert_eq!(
        block.style["color"],
        serde_json::json!("#202124"),
        "{:?}",
        block.style
    );
    // Each original paragraph keeps its own line.
    assert_eq!(block.md.lines().count(), 3, "{}", block.md);
    // The odd run out keeps its own colour inline; the grey body has no markup.
    assert_eq!(
        block.md.lines().next().unwrap(),
        "<span style=\"color:#a50034\">**핵심**</span>",
        "{}",
        block.md
    );
    assert!(
        !block.md.lines().nth(1).unwrap().contains("<span"),
        "{}",
        block.md
    );
}

#[test]
fn a_line_break_inside_a_paragraph_is_a_new_line() {
    // `<a:br/>` is Shift+Enter: the next run starts a new line on the slide,
    // and must start a new line in the markdown too, not follow on a space.
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="914400"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/>
                   <a:p><a:r><a:rPr sz="1000"/><a:t>• 첫 줄</a:t></a:r><a:br/><a:r><a:rPr sz="1000"/><a:t>• 둘째 줄</a:t></a:r></a:p>
                 </p:txBody></p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    assert_eq!(slides[0].blocks[0].md, "• 첫 줄\n• 둘째 줄");
}

#[test]
fn a_text_boxs_font_family_is_kept_for_the_export() {
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/>
                   <a:p><a:r><a:rPr sz="1300" b="1"><a:latin typeface="Malgun Gothic"/><a:ea typeface="맑은 고딕"/></a:rPr><a:t>데이터 자판기</a:t></a:r></a:p>
                 </p:txBody></p:sp>"#,
        )
        .build();

    let slides = read_deck(&deck);
    let block = &slides[0].blocks[0];
    assert_eq!(
        block.style["font"],
        serde_json::json!("맑은 고딕"),
        "{:?}",
        block.style
    );

    // Theme references (`+mn-lt`) are PowerPoint's business, not a family.
    let themed = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/>
                   <a:p><a:r><a:rPr sz="1300"><a:latin typeface="+mn-lt"/></a:rPr><a:t>본문</a:t></a:r></a:p>
                 </p:txBody></p:sp>"#,
        )
        .build();
    assert!(read_deck(&themed)[0].blocks[0].style.get("font").is_none());
}

#[test]
fn an_imported_deck_goes_back_out_on_its_own_design() {
    // The master draws a logo; the slide has a text box. On export the slide
    // must sit on the original layout, the original master part must be in the
    // file byte for byte, and the logo must not be drawn a second time.
    const CT: &str = concat!(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
        "<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>",
        "<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>",
        "<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>",
        "<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>",
        "<Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>",
        "</Types>"
    );
    let logo = r#"<p:sp><p:nvSpPr><p:cNvPr id="7" name="LogoBar"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="12192000" cy="228600"/></a:xfrm>
        <a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="A50034"/></a:solidFill></p:spPr></p:sp>"#;
    let deck = Builder::new()
        .master_shapes(logo)
        .part("[Content_Types].xml", CT)
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1800"/><a:t>본문 한 줄</a:t></a:r></a:p></p:txBody></p:sp>"#,
        )
        .build();
    let original_master = Package::open(&deck)
        .unwrap()
        .bytes("ppt/slideMasters/slideMaster1.xml")
        .unwrap()
        .to_vec();

    let package = Package::open(&deck).unwrap();
    let mut warnings = Warnings::default();
    let imported = ai_import::pptx::read(&package, &mut warnings).unwrap();
    let template = imported
        .assets
        .iter()
        .find(|a| a.name == ai_import::pptx::TEMPLATE_ASSET)
        .expect("the design is kept as an asset");
    let slide = &imported.slides[0];
    assert_eq!(
        slide.layout_part.as_deref(),
        Some("slideLayouts/slideLayout1.xml")
    );
    assert!(slide.master_shapes);
    let designed = slide
        .blocks
        .iter()
        .filter(|b| b.style.get("design") == Some(&serde_json::json!(true)))
        .count();
    assert_eq!(
        designed, 1,
        "the logo bar is marked as the design's: {:?}",
        slide.blocks
    );
    assert_eq!(slide.blocks.len(), 2, "logo + text box");

    // Into a project folder, as the app would put it, and back out.
    let ws = Workspace::new("pptxdesign");
    let mut project = create_project(ws.path(), ProjectType::Deck, "디자인", false).unwrap();
    std::fs::create_dir_all(project.dir.join("assets")).unwrap();
    std::fs::write(
        project.dir.join("assets").join(&template.name),
        &template.bytes,
    )
    .unwrap();
    project.items = Items::Slides(imported.slides.clone());
    let project = save_project(&project).unwrap();
    let exported = export(&project, Format::Pptx).unwrap();
    let out = Package::open(&exported).unwrap();

    assert_eq!(
        out.bytes("ppt/slideMasters/slideMaster1.xml"),
        Some(original_master.as_slice()),
        "the original master, byte for byte"
    );
    assert!(out.has("ppt/theme/theme1.xml") && out.has("ppt/slideLayouts/slideLayout1.xml"));
    let pres = String::from_utf8_lossy(out.bytes("ppt/presentation.xml").unwrap()).into_owned();
    assert!(
        pres.contains("r:id=\"rIdMaster\""),
        "the original master id list: {pres}"
    );
    assert!(pres.contains("<p:sldId id=\"256\""), "{pres}");
    let rels = String::from_utf8_lossy(out.bytes("ppt/slides/_rels/slide1.xml.rels").unwrap())
        .into_owned();
    assert!(rels.contains("../slideLayouts/slideLayout1.xml"), "{rels}");
    let slide_xml =
        String::from_utf8_lossy(out.bytes("ppt/slides/slide1.xml").unwrap()).into_owned();
    assert!(slide_xml.contains("본문 한 줄"));
    assert!(
        !slide_xml.contains("A50034"),
        "the master draws the logo, not the slide: {slide_xml}"
    );
    let types = String::from_utf8_lossy(out.bytes("[Content_Types].xml").unwrap()).into_owned();
    assert!(
        types.contains("/ppt/slideMasters/slideMaster1.xml"),
        "{types}"
    );
    assert!(types.contains("/ppt/slides/slide1.xml"), "{types}");
    assert_eq!(
        types.matches("/ppt/slides/slide1.xml").count(),
        1,
        "declared once: {types}"
    );
    assert!(types.contains("/docProps/core.xml"), "{types}");

    // Reading the export finds the logo once — from the master — plus the text.
    let again = read_deck(&exported);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].blocks.len(), 2, "{:?}", again[0].blocks);
}

#[test]
fn a_numbered_title_typed_as_text_stays_text_through_the_export() {
    // "2. 클라우드 전환" is a section title an author typed, not a list. As
    // markdown it would be an ordered list, and the export renumbered it "1.".
    let deck = Builder::new()
        .slide(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
                 <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="457200"/></a:xfrm></p:spPr>
                 <p:txBody><a:bodyPr/>
                   <a:p><a:r><a:rPr sz="2000" b="1"/><a:t>2. 클라우드 전환</a:t></a:r></a:p>
                   <a:p><a:r><a:rPr sz="2000" b="1"/><a:t>0. 배경</a:t></a:r></a:p>
                 </p:txBody></p:sp>"#,
        )
        .build();
    let slides = read_deck(&deck);
    let block = &slides[0].blocks[0];
    assert_eq!(block.md, "2\\. 클라우드 전환\n0\\. 배경", "{}", block.md);
    // Block-wide bold lives in the style, not as `**` on every run.
    assert_eq!(block.style["weight"], serde_json::json!(700));
    assert_eq!(
        ai_format::mdblocks::plain_text(&block.md),
        "2. 클라우드 전환\n0. 배경"
    );

    let ws = Workspace::new("pptxliteral");
    let mut project = create_project(ws.path(), ProjectType::Deck, "번호", false).unwrap();
    project.items = Items::Slides(slides.clone());
    let project = save_project(&project).unwrap();
    let exported = export(&project, Format::Pptx).unwrap();
    let xml = String::from_utf8_lossy(
        Package::open(&exported)
            .unwrap()
            .bytes("ppt/slides/slide1.xml")
            .unwrap(),
    )
    .into_owned();
    assert!(!xml.contains("buAutoNum"), "not a list: {xml}");
    assert!(xml.contains("2. 클라우드 전환"), "{xml}");
    // And the same markdown comes back: the round trip is a fixed point.
    let again = read_deck(&exported);
    assert_eq!(again[0].blocks[0].md, block.md);
    assert_eq!(
        again[0].blocks[0].style.get("weight"),
        block.style.get("weight")
    );
}
