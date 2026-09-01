//! `.docx` — page setup and paragraph overrides expressed in Word's vocabulary.

use std::path::Path;

use ai_format::chart::{chart_to_markdown_table, describe_chart, parse_chart_block};
use ai_format::model::{Override, Project, Section};
use serde_json::Value as Json;

use crate::mdruns::{parse_markdown, Block, Run};
use crate::ooxml::{
    emu, esc, hex, image_content_type, image_size, pt, relationships, twip, Package, Result,
};

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

/// Heading font sizes in px, matching the editor's own scale.
const HEADING_SIZE: [f64; 6] = [40.0, 32.0, 26.0, 22.0, 19.0, 17.0];

/// Images embedded so far, as `(part path, content type, rel id)`.
struct Media {
    parts: Vec<(String, Vec<u8>, &'static str)>,
}

impl Media {
    fn new() -> Self {
        Media { parts: Vec::new() }
    }

    /// Embed an image and return its relationship id and natural size.
    fn add(&mut self, data: Vec<u8>, ext: &str) -> Option<(String, (f64, f64))> {
        let content_type = image_content_type(ext)?;
        let size = image_size(&data).unwrap_or((480.0, 300.0));
        let index = self.parts.len() + 1;
        let path = format!("media/image{index}.{}", ext.to_ascii_lowercase());
        self.parts.push((path, data, content_type));
        // Media relationships start after the two fixed ones (styles, numbering).
        Some((format!("rId{}", index + 2), size))
    }
}

/* -------------------------------------------------------------------- runs */

fn run_xml(run: &Run, style: Option<&Override>) -> String {
    let s = style.map(|o| &o.style);
    let flag = |key: &str| {
        s.and_then(|m| m.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let string_of = |key: &str| s.and_then(|m| m.get(key)).and_then(|v| v.as_str());
    let number_of = |key: &str| {
        s.and_then(|m| m.get(key)).and_then(|v| match v {
            Json::Number(n) => n.as_f64(),
            Json::String(t) => t.trim().parse().ok(),
            _ => None,
        })
    };

    let mut props = String::new();
    if run.code {
        props.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/>");
    }
    if run.bold || flag("bold") {
        props.push_str("<w:b/>");
    }
    if run.italic || flag("italic") {
        props.push_str("<w:i/>");
    }
    if flag("underline") {
        props.push_str("<w:u w:val=\"single\"/>");
    }
    if run.strike {
        props.push_str("<w:strike/>");
    }
    if let Some(color) = string_of("color") {
        props.push_str(&format!("<w:color w:val=\"{}\"/>", hex(color)));
    }
    if let Some(size) = number_of("fontSize") {
        // w:sz is in half-points.
        props.push_str(&format!(
            "<w:sz w:val=\"{}\"/>",
            (pt(size) * 2.0).round() as i64
        ));
    }
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{props}</w:rPr>")
    };

    format!(
        "<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        esc(&run.text)
    )
}

fn runs_xml(runs: &[Run], style: Option<&Override>, hyperlinks: &mut Vec<String>) -> String {
    runs.iter()
        .map(|run| match &run.link {
            None => run_xml(run, style),
            Some(url) => {
                // External hyperlinks need their own relationship.
                hyperlinks.push(url.clone());
                let id = format!("rIdLink{}", hyperlinks.len());
                format!(
                    "<w:hyperlink r:id=\"{id}\"><w:r><w:rPr><w:rStyle w:val=\"Hyperlink\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:hyperlink>",
                    esc(&run.text)
                )
            }
        })
        .collect()
}

fn alignment(align: &str) -> &'static str {
    match align {
        "center" => "center",
        "right" => "right",
        "justify" => "both",
        _ => "left",
    }
}

/// The `<w:pPr>` body for a paragraph's override, minus the tags a caller adds.
fn paragraph_props(style: Option<&Override>, extra: &str) -> String {
    let mut props = String::from(extra);
    if let Some(o) = style {
        if let Some(align) = &o.align {
            props.push_str(&format!("<w:jc w:val=\"{}\"/>", alignment(align)));
        }
        if let Some(indent) = o.indent {
            props.push_str(&format!("<w:ind w:left=\"{}\"/>", twip(indent)));
        }
        if let Some(spacing) = &o.spacing {
            let before = spacing.before.map(|v| format!(" w:before=\"{}\"", twip(v)));
            let after = spacing.after.map(|v| format!(" w:after=\"{}\"", twip(v)));
            props.push_str(&format!(
                "<w:spacing{}{}/>",
                before.unwrap_or_default(),
                after.unwrap_or_default()
            ));
        }
    }
    if props.is_empty() {
        String::new()
    } else {
        format!("<w:pPr>{props}</w:pPr>")
    }
}

/* ------------------------------------------------------------------ blocks */

struct Ctx<'a> {
    dir: &'a Path,
    media: Media,
    hyperlinks: Vec<String>,
}

fn block_xml(block: &Block, style: Option<&Override>, ctx: &mut Ctx) -> String {
    match block {
        Block::Heading { level, runs } => {
            let props = paragraph_props(
                style,
                &format!("<w:pStyle w:val=\"Heading{}\"/>", level.clamp(&1, &6)),
            );
            // `HeadingN` already carries the weight and size; only a user
            // override adds anything on top of it.
            format!("<w:p>{props}{}</w:p>", runs_xml(runs, style, &mut ctx.hyperlinks))
        }
        Block::Paragraph { runs } => format!(
            "<w:p>{}{}</w:p>",
            paragraph_props(style, ""),
            runs_xml(runs, style, &mut ctx.hyperlinks)
        ),
        Block::Quote { runs } => {
            let indent = twip(36.0 + style.and_then(|o| o.indent).unwrap_or(0.0));
            let mut extra = format!("<w:ind w:left=\"{indent}\"/><w:pBdr><w:left w:val=\"single\" w:sz=\"12\" w:space=\"4\" w:color=\"D1D5DB\"/></w:pBdr>");
            if let Some(align) = style.and_then(|o| o.align.as_deref()) {
                extra.push_str(&format!("<w:jc w:val=\"{}\"/>", alignment(align)));
            }
            let mut quote_style = style.cloned().unwrap_or_default();
            quote_style.style.insert("italic".into(), Json::Bool(true));
            format!(
                "<w:p><w:pPr>{extra}</w:pPr>{}</w:p>",
                runs_xml(runs, Some(&quote_style), &mut ctx.hyperlinks)
            )
        }
        Block::List { ordered, items } => items
            .iter()
            .map(|item| {
                let numbering = format!(
                    "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>",
                    item.level.min(2),
                    if *ordered { 2 } else { 1 }
                );
                let props = paragraph_props(
                    style,
                    &format!("<w:pStyle w:val=\"ListParagraph\"/>{numbering}"),
                );
                format!(
                    "<w:p>{props}{}</w:p>",
                    runs_xml(&item.runs, style, &mut ctx.hyperlinks)
                )
            })
            .collect(),
        Block::Code { text, .. } => text
            .split('\n')
            .map(|line| {
                let body = if line.is_empty() { " " } else { line };
                format!(
                    "<w:p><w:pPr><w:shd w:val=\"clear\" w:fill=\"F3F4F6\"/><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/><w:sz w:val=\"20\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                    esc(body)
                )
            })
            .collect(),
        Block::Table { rows } => {
            let grid_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
            let mut out = String::from(
                "<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblBorders>",
            );
            for edge in ["top", "left", "bottom", "right", "insideH", "insideV"] {
                out.push_str(&format!(
                    "<w:{edge} w:val=\"single\" w:sz=\"4\" w:color=\"D1D5DB\"/>"
                ));
            }
            out.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
            for _ in 0..grid_cols {
                out.push_str(&format!("<w:gridCol w:w=\"{}\"/>", 9360 / grid_cols.max(1)));
            }
            out.push_str("</w:tblGrid>");

            for (r, row) in rows.iter().enumerate() {
                let is_header = r == 0;
                out.push_str("<w:tr>");
                if is_header {
                    out.push_str("<w:trPr><w:tblHeader/></w:trPr>");
                }
                for cell in row {
                    let shading = if is_header {
                        "<w:shd w:val=\"clear\" w:fill=\"F1F5F9\"/>"
                    } else {
                        ""
                    };
                    let mut cell_style = Override::default();
                    if is_header {
                        cell_style.style.insert("bold".into(), Json::Bool(true));
                    }
                    out.push_str(&format!(
                        "<w:tc><w:tcPr>{shading}</w:tcPr><w:p>{}</w:p></w:tc>",
                        runs_xml(cell, Some(&cell_style), &mut ctx.hyperlinks)
                    ));
                }
                out.push_str("</w:tr>");
            }
            out.push_str("</w:tbl><w:p/>");
            out
        }
        Block::Image { alt, src } => {
            if let Some((data, ext)) = read_asset(ctx.dir, src) {
                if let Some((rel_id, (nw, nh))) = ctx.media.add(data, &ext) {
                    // Fit the image to the printable width, keeping its aspect.
                    let width = 480.0f64.min(nw.max(1.0));
                    let height = (width * nh / nw.max(1.0)).max(1.0);
                    let picture = drawing_xml(&rel_id, alt, width, height, ctx.media.parts.len());
                    let caption = if alt.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:rPr><w:i/><w:sz w:val=\"18\"/><w:color w:val=\"6B7280\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                            esc(alt)
                        )
                    };
                    return format!(
                        "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr>{picture}</w:p>{caption}"
                    );
                }
            }
            // Unresolvable image: keep the alt text so the page is not silently empty.
            let text = if alt.is_empty() { "(이미지)" } else { alt };
            format!(
                "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:rPr><w:i/><w:color w:val=\"9CA3AF\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                esc(text)
            )
        }
        Block::Hr => "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:color=\"D1D5DB\"/></w:pBdr></w:pPr></w:p>".to_string(),
    }
}

fn drawing_xml(rel_id: &str, alt: &str, width: f64, height: f64, index: usize) -> String {
    format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
<wp:docPr id=\"{index}\" name=\"Picture {index}\" descr=\"{alt}\"/>\
<a:graphic xmlns:a=\"{NS_A}\"><a:graphicData uri=\"{NS_PIC}\">\
<pic:pic xmlns:pic=\"{NS_PIC}\">\
<pic:nvPicPr><pic:cNvPr id=\"{index}\" name=\"Picture {index}\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"{rel_id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
        cx = emu(width),
        cy = emu(height),
        alt = esc(alt),
    )
}

/// A chart becomes the table of its own numbers with a caption naming the shape.
///
/// A `.docx` has no chart primitive we can write without also writing an
/// embedded workbook part, and nothing a reader needs is lost — Word turns the
/// table into a chart in two clicks.
fn chart_block_xml(spec: &ai_format::chart::ChartSpec, ctx: &mut Ctx) -> String {
    let title = if spec.title.is_empty() {
        "차트"
    } else {
        &spec.title
    };
    let mut out = format!(
        "<w:p><w:pPr><w:spacing w:before=\"200\" w:after=\"80\"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val=\"26\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        esc(title)
    );

    let table = chart_to_markdown_table(spec);
    for block in parse_markdown(&table) {
        if matches!(block, Block::Table { .. }) {
            out.push_str(&block_xml(&block, None, ctx));
        }
    }

    out.push_str(&format!(
        "<w:p><w:pPr><w:spacing w:after=\"200\"/></w:pPr><w:r><w:rPr><w:i/><w:sz w:val=\"18\"/><w:color w:val=\"6B7280\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        esc(&format!("표 데이터 · {}", describe_chart(spec)))
    ));
    out
}

fn section_xml(section: &Section, ctx: &mut Ctx) -> (String, String) {
    let mut body = String::new();

    for block in &section.blocks {
        // A chart block is recognised by its fenced spec, wherever it sits.
        if let Some(spec) = parse_chart_block(&block.md) {
            body.push_str(&chart_block_xml(&spec, ctx));
            continue;
        }
        for parsed in parse_markdown(&block.md) {
            body.push_str(&block_xml(&parsed, block.format_override.as_ref(), ctx));
        }
    }
    if body.is_empty() {
        body.push_str("<w:p/>");
    }

    let (w, h) = section.page.dimensions();
    let m = section.page.margin;
    // `twip` already folds in the px -> pt step. The JavaScript exporter applied
    // both conversions, which shipped every page and margin at 75% of its size:
    // A4 arrived in Word as 6.2in wide instead of 8.27in.
    let props = format!(
        "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/>{}</w:sectPr>",
        twip(w),
        twip(h),
        twip(m.top),
        twip(m.right),
        twip(m.bottom),
        twip(m.left),
        if section.page.columns > 1 {
            format!("<w:cols w:num=\"{}\" w:space=\"420\"/>", section.page.columns)
        } else {
            String::new()
        },
    );
    (body, props)
}

/// Read an image referenced from markdown, refusing anything outside the project.
///
/// The export must not read outside the document or fetch over the network.
fn read_asset(dir: &Path, src: &str) -> Option<(Vec<u8>, String)> {
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("data:") {
        return None;
    }
    let relative = src.trim_start_matches("./");
    let abs = ai_format::project::resolve_inside(dir, relative).ok()?;
    if !abs.is_file() {
        return None;
    }
    let ext = abs.extension()?.to_string_lossy().into_owned();
    let data = std::fs::read(&abs).ok()?;
    Some((data, ext))
}

/// Export a document to `.docx`.
pub fn export(project: &Project) -> Result<Vec<u8>> {
    let mut ctx = Ctx {
        dir: &project.dir,
        media: Media::new(),
        hyperlinks: Vec::new(),
    };

    let sections = project.sections();
    let mut body = String::new();
    for (i, section) in sections.iter().enumerate() {
        let (content, props) = section_xml(section, &mut ctx);
        body.push_str(&content);
        // Every section but the last carries its properties in a trailing
        // paragraph; the last one carries them on the body itself.
        if i + 1 < sections.len() {
            body.push_str(&format!("<w:p><w:pPr>{props}</w:pPr></w:p>"));
        } else {
            body.push_str(&props);
        }
    }
    if sections.is_empty() {
        body.push_str("<w:p/>");
    }

    let document = format!(
        "<w:document xmlns:w=\"{NS_W}\" xmlns:r=\"{NS_R}\" xmlns:wp=\"{NS_WP}\" xmlns:a=\"{NS_A}\" xmlns:pic=\"{NS_PIC}\"><w:body>{body}</w:body></w:document>"
    );

    let mut pkg = Package::new();

    let styles_rel = format!("{REL}/styles");
    let numbering_rel = format!("{REL}/numbering");
    let image_rel = format!("{REL}/image");
    let hyperlink_rel = format!("{REL}/hyperlink");
    let mut rels: Vec<(String, &str, String)> = Vec::new();
    rels.push((
        "rId1".to_string(),
        styles_rel.as_str(),
        "styles.xml".to_string(),
    ));
    rels.push((
        "rId2".to_string(),
        numbering_rel.as_str(),
        "numbering.xml".to_string(),
    ));
    for (i, (path, _, _)) in ctx.media.parts.iter().enumerate() {
        rels.push((format!("rId{}", i + 3), image_rel.as_str(), path.clone()));
    }
    for (i, url) in ctx.hyperlinks.iter().enumerate() {
        rels.push((
            format!("rIdLink{}", i + 1),
            hyperlink_rel.as_str(),
            url.clone(),
        ));
    }

    let mut content_types = String::from(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
    );
    for (path, _, content_type) in &ctx.media.parts {
        content_types.push_str(&format!(
            "<Override PartName=\"/word/{path}\" ContentType=\"{content_type}\"/>"
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
                "word/document.xml".to_string(),
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
    pkg.add_xml("word/document.xml", &document);
    pkg.add_xml("word/_rels/document.xml.rels", &relationships(&rels));
    pkg.add_xml("word/styles.xml", &styles_xml());
    pkg.add_xml("word/numbering.xml", &numbering_xml());
    for (path, data, _) in ctx.media.parts {
        pkg.add(&format!("word/{path}"), data);
    }

    pkg.finish()
}

fn styles_xml() -> String {
    let mut out = format!("<w:styles xmlns:w=\"{NS_W}\">");
    out.push_str(
        "<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Malgun Gothic\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>",
    );
    out.push_str("<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>");
    for (i, size) in HEADING_SIZE.iter().enumerate() {
        let level = i + 1;
        out.push_str(&format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{level}\"><w:name w:val=\"heading {level}\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepNext/><w:outlineLvl w:val=\"{}\"/><w:spacing w:before=\"240\" w:after=\"120\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"{}\"/></w:rPr></w:style>",
            i,
            (pt(*size) * 2.0).round() as i64
        ));
    }
    out.push_str("<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:contextualSpacing/></w:pPr></w:style>");
    out.push_str("<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>");
    out.push_str("</w:styles>");
    out
}

/// Two numbering definitions: bullets (numId 1) and decimals (numId 2).
fn numbering_xml() -> String {
    let mut out = format!("<w:numbering xmlns:w=\"{NS_W}\">");
    for (abstract_id, ordered) in [(0, false), (1, true)] {
        out.push_str(&format!(
            "<w:abstractNum w:abstractNumId=\"{abstract_id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"
        ));
        for level in 0..3 {
            let indent = twip(36.0 * (level as f64 + 1.0));
            if ordered {
                out.push_str(&format!(
                    "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%{}.\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{indent}\" w:hanging=\"270\"/></w:pPr></w:lvl>",
                    level + 1
                ));
            } else {
                let bullet = ["\u{2022}", "\u{25E6}", "\u{25AA}"][level];
                out.push_str(&format!(
                    "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"{bullet}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{indent}\" w:hanging=\"270\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Symbol\" w:hAnsi=\"Symbol\" w:hint=\"default\"/></w:rPr></w:lvl>"
                ));
            }
        }
        out.push_str("</w:abstractNum>");
    }
    out.push_str("<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>");
    out.push_str("<w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num>");
    out.push_str("</w:numbering>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdruns::inline_runs;

    #[test]
    fn runs_carry_their_marks() {
        let runs = inline_runs("**굵게**");
        let xml = run_xml(&runs[0], None);
        assert!(xml.contains("<w:b/>"), "{xml}");
        assert!(xml.contains("굵게"));
    }

    #[test]
    fn an_override_style_reaches_the_run() {
        let mut o = Override {
            align: Some("center".into()),
            ..Override::default()
        };
        o.style
            .insert("color".into(), Json::String("#ff0000".into()));
        o.style.insert("fontSize".into(), Json::from(24));
        let xml = run_xml(&Run::plain("본문"), Some(&o));
        assert!(xml.contains("<w:color w:val=\"FF0000\"/>"), "{xml}");
        assert!(xml.contains("<w:sz w:val=\"36\"/>"), "24px is 18pt: {xml}");
    }

    #[test]
    fn paragraph_props_map_onto_words_vocabulary() {
        let o = Override {
            align: Some("center".into()),
            indent: Some(24.0),
            spacing: Some(ai_format::model::Spacing {
                before: Some(8.0),
                after: Some(16.0),
            }),
            style: Default::default(),
        };
        let xml = paragraph_props(Some(&o), "");
        assert!(xml.contains("<w:jc w:val=\"center\"/>"), "{xml}");
        assert!(xml.contains("<w:ind w:left=\"360\"/>"), "{xml}");
        assert!(xml.contains("w:before=\"120\""), "8px is 120 twips: {xml}");
        assert_eq!(paragraph_props(None, ""), "");
    }

    #[test]
    fn xml_metacharacters_in_content_are_escaped() {
        let xml = run_xml(&Run::plain("a < b & \"c\""), None);
        assert!(xml.contains("a &lt; b &amp; &quot;c&quot;"), "{xml}");
    }

    #[test]
    fn numbering_defines_both_list_kinds() {
        let xml = numbering_xml();
        assert!(xml.contains("<w:num w:numId=\"1\">"));
        assert!(xml.contains("<w:num w:numId=\"2\">"));
        assert!(xml.contains("w:val=\"decimal\""));
        assert!(xml.contains("w:val=\"bullet\""));
    }
}
