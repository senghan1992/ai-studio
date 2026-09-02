//! `.pptx` — block geometry maps straight across, and charts go across as real
//! chart parts PowerPoint can edit.

use std::path::Path;

use ai_format::blocks::Kind;
use ai_format::chart::{parse_chart_block, ChartSpec, ChartType, PALETTE_LIGHT};
use ai_format::model::{Project, Slide, SlideBlock};
use ai_format::shape::ShapeSpec;
use ai_format::table::{parse_markdown_table, TableSpec, TableStyle};
use serde_json::Value as Json;

use crate::mdruns::{image_alt, image_source, parse_markdown, runs_to_text, Block, Run};
use crate::ooxml::{
    emu, esc, font_size_100, hex, image_content_type, image_size, relationships,
    relationships_with_modes, Package, Result,
};

const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// One slide's parts: its XML plus the relationships it needs.
struct SlidePart {
    xml: String,
    /// `(rel id, relationship type suffix, target)`
    rels: Vec<(String, String, String)>,
    notes: Option<String>,
}

#[derive(Default)]
struct Assets {
    /// `(part path, bytes, content type)`
    media: Vec<(String, Vec<u8>, &'static str)>,
    /// Chart part XML, in order.
    charts: Vec<String>,
}

/* -------------------------------------------------------------------- text */

fn style_num(style: &indexmap::IndexMap<String, Json>, key: &str) -> Option<f64> {
    match style.get(key) {
        Some(Json::Number(n)) => n.as_f64(),
        Some(Json::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

fn style_str<'a>(style: &'a indexmap::IndexMap<String, Json>, key: &str) -> Option<&'a str> {
    style.get(key).and_then(|v| v.as_str())
}

/// Run properties shared by every text run in a block.
///
/// A hyperlink needs a slide relationship, so the URLs are collected as they are
/// met and the caller turns them into `_rels` entries.
fn run_props(
    run: &Run,
    size_px: f64,
    color: &str,
    bold_default: bool,
    links: &mut Vec<String>,
) -> String {
    let mut out = format!(
        "<a:rPr lang=\"ko-KR\" altLang=\"en-US\" sz=\"{}\"",
        font_size_100(size_px)
    );
    if run.bold || bold_default {
        out.push_str(" b=\"1\"");
    }
    if run.italic {
        out.push_str(" i=\"1\"");
    }
    if run.strike {
        out.push_str(" strike=\"sngStrike\"");
    }
    out.push('>');
    out.push_str(&format!(
        "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
        hex(color)
    ));
    if run.code {
        out.push_str("<a:latin typeface=\"Consolas\"/>");
    }
    if let Some(url) = &run.link {
        links.push(url.clone());
        out.push_str(&format!(
            "<a:hlinkClick xmlns:r=\"{NS_R}\" r:id=\"rIdLink{}\"/>",
            links.len()
        ));
    }
    out.push_str("</a:rPr>");
    out
}

struct Paragraph {
    /// Bullet level, or `None` for a plain paragraph.
    bullet: Option<(usize, bool)>,
    align: &'static str,
    size_px: f64,
    bold: bool,
    runs: Vec<Run>,
}

fn paragraph_xml(p: &Paragraph, color: &str, line_height: f64, links: &mut Vec<String>) -> String {
    let mut props = format!("<a:pPr algn=\"{}\" lnSpcReduction=\"0\"", p.align);
    if let Some((level, _)) = p.bullet {
        if level > 0 {
            props.push_str(&format!(" lvl=\"{}\"", level.min(8)));
        }
        props.push_str(&format!(
            " indent=\"{}\" marL=\"{}\"",
            emu(-14.0),
            emu(20.0 * (level as f64 + 1.0))
        ));
    }
    props.push('>');
    props.push_str(&format!(
        "<a:lnSpc><a:spcPct val=\"{}\"/></a:lnSpc>",
        (line_height * 100_000.0).round() as i64
    ));
    match p.bullet {
        None => props.push_str("<a:buNone/>"),
        Some((_, true)) => {
            props.push_str("<a:buFont typeface=\"+mj-lt\"/><a:buAutoNum type=\"arabicPeriod\"/>")
        }
        Some((_, false)) => {
            props.push_str("<a:buFont typeface=\"Arial\"/><a:buChar char=\"\u{2022}\"/>")
        }
    }
    props.push_str("</a:pPr>");

    let mut runs = String::new();
    for run in p.runs.iter().filter(|r| !r.text.is_empty()) {
        runs.push_str(&format!(
            "<a:r>{}<a:t>{}</a:t></a:r>",
            run_props(run, p.size_px, color, p.bold, links),
            esc(&run.text)
        ));
    }

    format!("<a:p>{props}{runs}</a:p>")
}

/// Flatten a block's markdown into slide paragraphs.
fn block_paragraphs(md: &str, style: &indexmap::IndexMap<String, Json>) -> Vec<Paragraph> {
    let base_size = style_num(style, "fontSize").unwrap_or(20.0);
    let align = match style_str(style, "align") {
        Some("center") => "ctr",
        Some("right") => "r",
        Some("justify") => "just",
        _ => "l",
    };
    let base_bold = style_num(style, "weight")
        .map(|w| w >= 600.0)
        .unwrap_or(false);

    let mut out = Vec::new();
    for block in parse_markdown(md) {
        match block {
            Block::Heading { level, runs } => out.push(Paragraph {
                bullet: None,
                align,
                // The editor draws a heading as a multiple of the block's own
                // size, so the export has to as well or every title shrinks.
                size_px: base_size * ai_format::mdblocks::heading_em(level),
                bold: true,
                runs,
            }),
            Block::Paragraph { runs } | Block::Quote { runs } => out.push(Paragraph {
                bullet: None,
                align,
                size_px: base_size,
                bold: base_bold,
                runs,
            }),
            Block::List { ordered, items } => {
                for item in items {
                    out.push(Paragraph {
                        bullet: Some((item.level, ordered)),
                        align,
                        size_px: base_size
                            * ai_format::mdblocks::NESTED_LIST_EM.powi(item.level as i32),
                        bold: base_bold,
                        runs: item.runs,
                    });
                }
            }
            Block::Code { text, .. } => {
                for line in text.split('\n') {
                    out.push(Paragraph {
                        bullet: None,
                        align: "l",
                        size_px: base_size * 0.85,
                        bold: false,
                        runs: vec![Run {
                            text: line.to_string(),
                            code: true,
                            ..Run::default()
                        }],
                    });
                }
            }
            Block::Table { rows } => {
                for row in rows {
                    let joined = row
                        .iter()
                        .map(|cell| runs_to_text(cell))
                        .collect::<Vec<_>>()
                        .join("  |  ");
                    out.push(Paragraph {
                        bullet: None,
                        align,
                        size_px: base_size * 0.9,
                        bold: base_bold,
                        runs: vec![Run::plain(joined)],
                    });
                }
            }
            Block::Image { alt, .. } => {
                if !alt.is_empty() {
                    out.push(Paragraph {
                        bullet: None,
                        align,
                        size_px: base_size,
                        bold: false,
                        runs: vec![Run {
                            text: alt,
                            italic: true,
                            ..Run::default()
                        }],
                    });
                }
            }
            Block::Hr => out.push(Paragraph {
                bullet: None,
                align,
                size_px: base_size,
                bold: false,
                runs: vec![Run::plain("\u{2500}".repeat(24))],
            }),
        }
    }
    out
}

/* ------------------------------------------------------------------ shapes */

fn xfrm(block: &SlideBlock) -> String {
    format!(
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(block.x),
        emu(block.y),
        emu(block.w.max(1.0)),
        emu(block.h.max(1.0)),
    )
}

/// The same, plus a shape's rotation and flips.
///
/// `rot` is in 60,000ths of a degree, and negative values are legal — Office
/// normalises them, so pass the angle through rather than clamping it.
fn xfrm_with(block: &SlideBlock, spec: &ShapeSpec) -> String {
    let mut attrs = String::new();
    if spec.rotation != 0.0 {
        attrs.push_str(&format!(
            " rot=\"{}\"",
            (spec.rotation * 60_000.0).round() as i64
        ));
    }
    if spec.flip_h {
        attrs.push_str(" flipH=\"1\"");
    }
    if spec.flip_v {
        attrs.push_str(" flipV=\"1\"");
    }
    format!(
        "<a:xfrm{attrs}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(block.x),
        emu(block.y),
        emu(block.w.max(1.0)),
        emu(block.h.max(1.0)),
    )
}

fn text_shape(id: usize, block: &SlideBlock, links: &mut Vec<String>) -> String {
    let paragraphs = block_paragraphs(&block.md, &block.style);
    if paragraphs.is_empty() {
        return String::new();
    }
    let color = style_str(&block.style, "color").unwrap_or("#1f2937");
    let line_height = style_num(&block.style, "lineHeight").unwrap_or(1.3);
    let anchor = match style_str(&block.style, "valign") {
        Some("middle") => "ctr",
        Some("bottom") => "b",
        _ => "t",
    };

    let body: String = paragraphs
        .iter()
        .map(|p| paragraph_xml(p, color, line_height, links))
        .collect();
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Text {id}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
<p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>\
<p:txBody><a:bodyPr wrap=\"square\" anchor=\"{anchor}\" lIns=\"18000\" tIns=\"18000\" rIns=\"18000\" bIns=\"18000\"><a:normAutofit/></a:bodyPr><a:lstStyle/>{body}</p:txBody></p:sp>",
        xfrm(block)
    )
}

/// A shape, with the preset geometry it was drawn or imported as.
///
/// The `prstGeom` name goes across verbatim, including one this renderer cannot
/// draw — PowerPoint knows them all, so an unfamiliar preset survives the trip
/// out even when the editor showed it as a rectangle.
fn shape_element(id: usize, block: &SlideBlock, links: &mut Vec<String>) -> String {
    let spec = block.shape.clone().unwrap_or_default();

    let mut geom = format!("<a:prstGeom prst=\"{}\">", esc(&spec.preset));
    if spec.adjust.is_empty() {
        geom.push_str("<a:avLst/>");
    } else {
        geom.push_str("<a:avLst>");
        for (name, value) in &spec.adjust {
            geom.push_str(&format!(
                "<a:gd name=\"{}\" fmla=\"val {}\"/>",
                esc(name),
                value.round() as i64
            ));
        }
        geom.push_str("</a:avLst>");
    }
    geom.push_str("</a:prstGeom>");

    let fill = match &spec.fill {
        None => "<a:noFill/>".to_string(),
        Some(fill) if fill.opacity >= 100.0 => {
            format!(
                "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
                hex(&fill.color)
            )
        }
        Some(fill) => format!(
            "<a:solidFill><a:srgbClr val=\"{}\"><a:alpha val=\"{}\"/></a:srgbClr></a:solidFill>",
            hex(&fill.color),
            (fill.opacity.clamp(0.0, 100.0) * 1000.0).round() as i64
        ),
    };

    let line = match &spec.line {
        None => "<a:ln><a:noFill/></a:ln>".to_string(),
        Some(line) => {
            let dash = if line.dash == ai_format::shape::Dash::Solid {
                String::new()
            } else {
                format!("<a:prstDash val=\"{}\"/>", line.dash.as_ooxml())
            };
            format!(
                "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>{dash}</a:ln>",
                emu(line.width.max(0.25)),
                hex(&line.color)
            )
        }
    };

    // A shape can hold text, and in this format that text is the block's markdown.
    let body = shape_text_body(block, links);

    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{} {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
<p:spPr>{}{geom}{fill}{line}</p:spPr>{body}</p:sp>",
        esc(spec.label()),
        xfrm_with(block, &spec),
    )
}

/// The text inside a shape, or an empty body when it has none.
fn shape_text_body(block: &SlideBlock, links: &mut Vec<String>) -> String {
    let paragraphs = block_paragraphs(&block.md, &block.style);
    if paragraphs.is_empty() {
        return "<p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody>".to_string();
    }
    let color = style_str(&block.style, "color").unwrap_or("#1f2937");
    let line_height = style_num(&block.style, "lineHeight").unwrap_or(1.3);
    let anchor = match style_str(&block.style, "valign") {
        Some("middle") => "ctr",
        Some("bottom") => "b",
        _ => "t",
    };
    let body: String = paragraphs
        .iter()
        .map(|p| paragraph_xml(p, color, line_height, links))
        .collect();
    format!(
        "<p:txBody><a:bodyPr wrap=\"square\" anchor=\"{anchor}\" lIns=\"45720\" tIns=\"45720\" rIns=\"45720\" bIns=\"45720\"><a:normAutofit/></a:bodyPr><a:lstStyle/>{body}</p:txBody>"
    )
}

fn picture_shape(id: usize, block: &SlideBlock, rel_id: &str, natural: (f64, f64)) -> String {
    // Contain: the image keeps its aspect ratio inside the block's box.
    let (nw, nh) = natural;
    let scale = (block.w / nw.max(1.0)).min(block.h / nh.max(1.0));
    let (w, h) = (nw * scale, nh * scale);
    let x = block.x + (block.w - w) / 2.0;
    let y = block.y + (block.h - h) / 2.0;

    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Picture {id}\" descr=\"{}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"{rel_id}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
        esc(&image_alt(&block.md)),
        emu(x),
        emu(y),
        emu(w),
        emu(h),
    )
}

/// A table PowerPoint can edit: a real `a:tbl`, with the spans and banding the
/// layout JSON recorded.
///
/// The previous exporter flattened a table into a grid of plain strings, which
/// lost merges and any cell formatting. Because a table's text is markdown here,
/// each cell's emphasis also has to become runs rather than literal asterisks.
fn table_shape(id: usize, block: &SlideBlock, links: &mut Vec<String>) -> String {
    let cells = parse_markdown_table(&block.md);
    let spec = block.table.clone().unwrap_or_default();
    let columns = cells.iter().map(|r| r.len()).max().unwrap_or(1);
    let row_count = cells.len().max(1);

    let base_size = style_num(&block.style, "fontSize").unwrap_or(16.0);

    let mut grid = String::from("<a:tblGrid>");
    for c in 0..columns {
        grid.push_str(&format!(
            "<a:gridCol w=\"{}\"/>",
            emu(spec.col_width(c, columns, block.w))
        ));
    }
    grid.push_str("</a:tblGrid>");

    let mut body = String::new();
    for (r, row) in cells.iter().enumerate() {
        let height = spec
            .rows
            .get(r)
            .copied()
            .filter(|h| *h > 0.0)
            .unwrap_or(block.h / row_count as f64);
        body.push_str(&format!("<a:tr h=\"{}\">", emu(height)));

        for c in 0..columns {
            let span = spec.span_at(c, r);
            // A covered cell is emitted as a merge continuation, never as content.
            let attrs = match &span {
                Some(span) if span.is_anchor() => {
                    let mut a = String::new();
                    if span.cols > 1 {
                        a.push_str(&format!(" gridSpan=\"{}\"", span.cols));
                    }
                    if span.rows > 1 {
                        a.push_str(&format!(" rowSpan=\"{}\"", span.rows));
                    }
                    a
                }
                Some(span) => {
                    let mut a = String::new();
                    if span.continues_row {
                        a.push_str(" hMerge=\"1\"");
                    }
                    if span.continues_col {
                        a.push_str(" vMerge=\"1\"");
                    }
                    a
                }
                None => String::new(),
            };
            let covered = span.as_ref().is_some_and(|s| !s.is_anchor());

            let text = row.get(c).map(String::as_str).unwrap_or("");
            let format = spec.cells.get(&ai_formula::refs::to_ref(c, r));
            let is_header = spec.header_row && r == 0;
            let is_first_col = spec.first_col && c == 0;

            let content = if covered || text.is_empty() {
                "<a:p/>".to_string()
            } else {
                let align = match format.and_then(|f| f.align.as_deref()) {
                    Some("center") => "ctr",
                    Some("right") => "r",
                    _ => "l",
                };
                let color = format
                    .and_then(|f| f.color.as_deref())
                    .unwrap_or(if is_header { "#ffffff" } else { "#1f2937" });
                let mut runs = String::new();
                for run in crate::mdruns::inline_runs(text) {
                    if run.text.is_empty() {
                        continue;
                    }
                    runs.push_str(&format!(
                        "<a:r>{}<a:t>{}</a:t></a:r>",
                        run_props(&run, base_size, color, is_header || is_first_col, links),
                        esc(&run.text)
                    ));
                }
                format!("<a:p><a:pPr algn=\"{align}\"/>{runs}</a:p>")
            };

            let valign = match format.and_then(|f| f.valign.as_deref()) {
                Some("top") => "t",
                Some("bottom") => "b",
                _ => "ctr",
            };
            let fill = cell_fill(&spec, format, r, c);

            body.push_str(&format!(
                "<a:tc{attrs}><a:txBody><a:bodyPr/><a:lstStyle/>{content}</a:txBody>\
<a:tcPr marL=\"45720\" marR=\"45720\" marT=\"27432\" marB=\"27432\" anchor=\"{valign}\">{}{fill}</a:tcPr></a:tc>",
                cell_borders(&spec)
            ));
        }
        body.push_str("</a:tr>");
    }

    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table {id}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>\
<p:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\">\
<a:tbl><a:tblPr firstRow=\"{}\" firstCol=\"{}\" bandRow=\"{}\"/>{grid}{body}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
        emu(block.x),
        emu(block.y),
        emu(block.w),
        emu(block.h),
        i32::from(spec.header_row),
        i32::from(spec.first_col),
        i32::from(spec.banded_rows),
    )
}

/// The header band, the row banding, and any explicit per-cell colour.
fn cell_fill(
    spec: &TableSpec,
    format: Option<&ai_format::table::CellFormat>,
    row: usize,
    _col: usize,
) -> String {
    if let Some(color) = format.and_then(|f| f.fill.as_deref()) {
        return format!(
            "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
            hex(color)
        );
    }
    if spec.header_row && row == 0 {
        return format!(
            "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
            hex(PALETTE_LIGHT[0])
        );
    }
    if spec.banded_rows && spec.style != TableStyle::Borderless {
        let body_row = if spec.header_row {
            row.saturating_sub(1)
        } else {
            row
        };
        if body_row % 2 == 1 {
            return "<a:solidFill><a:srgbClr val=\"F1F5F9\"/></a:solidFill>".to_string();
        }
    }
    "<a:noFill/>".to_string()
}

fn cell_borders(spec: &TableSpec) -> String {
    if spec.style == TableStyle::Borderless {
        return String::new();
    }
    ["lnL", "lnR", "lnT", "lnB"]
        .iter()
        .map(|edge| {
            format!(
                "<a:{edge} w=\"9525\"><a:solidFill><a:srgbClr val=\"D1D5DB\"/></a:solidFill></a:{edge}>"
            )
        })
        .collect()
}

fn chart_frame(id: usize, block: &SlideBlock, rel_id: &str) -> String {
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Chart {id}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>\
<p:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm>\
<a:graphic><a:graphicData uri=\"{NS_C}\"><c:chart xmlns:c=\"{NS_C}\" xmlns:r=\"{NS_R}\" r:id=\"{rel_id}\"/></a:graphicData></a:graphic></p:graphicFrame>",
        emu(block.x),
        emu(block.y),
        emu(block.w),
        emu(block.h),
    )
}

fn placeholder_text(id: usize, block: &SlideBlock, text: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Placeholder {id}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
<p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>\
<p:txBody><a:bodyPr anchor=\"ctr\"/><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr lang=\"ko-KR\" sz=\"1200\" i=\"1\"><a:solidFill><a:srgbClr val=\"9CA3AF\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xfrm(block),
        esc(text)
    )
}

/* ------------------------------------------------------------------ charts */

/// Write a chart part PowerPoint can edit, rather than a picture of one.
///
/// The recipient can retype a number and the chart updates — which is the whole
/// reason to export to `.pptx` instead of a PNG.
fn chart_xml(spec: &ChartSpec) -> String {
    let labels: Vec<String> = if spec.labels.is_empty() {
        (1..=ai_format::chart::point_count(spec))
            .map(|i| i.to_string())
            .collect()
    } else {
        spec.labels.clone()
    };

    let mut series_xml = String::new();
    for (i, series) in spec.series.iter().enumerate() {
        let name = if series.name.is_empty() {
            format!("계열 {}", i + 1)
        } else {
            series.name.clone()
        };
        let color = PALETTE_LIGHT[i % PALETTE_LIGHT.len()];

        let cat: String = labels
            .iter()
            .enumerate()
            .map(|(j, label)| format!("<c:pt idx=\"{j}\"><c:v>{}</c:v></c:pt>", esc(label)))
            .collect();
        // PowerPoint has no notion of a gap, so a missing point becomes zero.
        let val: String = (0..labels.len())
            .map(|j| {
                let v = series.values.get(j).copied().flatten().unwrap_or(0.0);
                format!(
                    "<c:pt idx=\"{j}\"><c:v>{}</c:v></c:pt>",
                    ai_formula::values::format_plain_number(v)
                )
            })
            .collect();

        let marks = if spec.chart_type.is_radial() {
            // A radial chart colours each point, not each series.
            labels
                .iter()
                .enumerate()
                .map(|(j, _)| {
                    format!(
                        "<c:dPt><c:idx val=\"{j}\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></c:spPr></c:dPt>",
                        hex(PALETTE_LIGHT[j % PALETTE_LIGHT.len()])
                    )
                })
                .collect::<String>()
        } else {
            String::new()
        };

        let shape = match spec.chart_type {
            ChartType::Line => format!(
                "<c:spPr><a:ln w=\"25400\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/></c:marker>",
                hex(color)
            ),
            _ => format!(
                "<c:spPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></c:spPr>",
                hex(color)
            ),
        };

        series_xml.push_str(&format!(
            "<c:ser><c:idx val=\"{i}\"/><c:order val=\"{i}\"/>\
<c:tx><c:strRef><c:f>Sheet1!$A${}</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx>\
{shape}{marks}\
<c:cat><c:strRef><c:f>Sheet1!$A$2:$A${}</c:f><c:strCache><c:ptCount val=\"{}\"/>{cat}</c:strCache></c:strRef></c:cat>\
<c:val><c:numRef><c:f>Sheet1!${}$2:${}${}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{val}</c:numCache></c:numRef></c:val>\
</c:ser>",
            i + 2,
            esc(&name),
            labels.len() + 1,
            labels.len(),
            ai_formula::refs::index_to_col(i + 1),
            ai_formula::refs::index_to_col(i + 1),
            labels.len() + 1,
            labels.len(),
        ));
    }

    let show_value = if spec.options.value_labels == ai_format::chart::ValueLabels::All {
        "1"
    } else {
        "0"
    };
    let data_labels = format!(
        "<c:dLbls><c:showLegendKey val=\"0\"/><c:showVal val=\"{show_value}\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/></c:dLbls>"
    );

    let grouping = if spec.options.stacked {
        "stacked"
    } else {
        "clustered"
    };
    let plot = match spec.chart_type {
        ChartType::Column | ChartType::Bar => format!(
            "<c:barChart><c:barDir val=\"{}\"/><c:grouping val=\"{grouping}\"/><c:varyColors val=\"0\"/>{series_xml}{data_labels}<c:gapWidth val=\"60\"/>{}<c:axId val=\"111\"/><c:axId val=\"222\"/></c:barChart>",
            if spec.chart_type == ChartType::Bar { "bar" } else { "col" },
            if spec.options.stacked { "<c:overlap val=\"100\"/>" } else { "<c:overlap val=\"-10\"/>" },
        ),
        ChartType::Line => format!(
            "<c:lineChart><c:grouping val=\"{}\"/><c:varyColors val=\"0\"/>{series_xml}{data_labels}<c:marker val=\"1\"/><c:axId val=\"111\"/><c:axId val=\"222\"/></c:lineChart>",
            if spec.options.stacked { "stacked" } else { "standard" }
        ),
        ChartType::Area => format!(
            "<c:areaChart><c:grouping val=\"{}\"/><c:varyColors val=\"0\"/>{series_xml}{data_labels}<c:axId val=\"111\"/><c:axId val=\"222\"/></c:areaChart>",
            if spec.options.stacked { "stacked" } else { "standard" }
        ),
        ChartType::Pie => format!(
            "<c:pieChart><c:varyColors val=\"1\"/>{series_xml}{data_labels}<c:firstSliceAng val=\"0\"/></c:pieChart>"
        ),
        ChartType::Donut => format!(
            "<c:doughnutChart><c:varyColors val=\"1\"/>{series_xml}{data_labels}<c:firstSliceAng val=\"0\"/><c:holeSize val=\"58\"/></c:doughnutChart>"
        ),
    };

    // Radial charts have no axes; cartesian ones need exactly the pair the plot
    // references, or PowerPoint reports the file as corrupt.
    let axes = if spec.chart_type.is_radial() {
        String::new()
    } else {
        let val_title = if spec.options.y_title.is_empty() {
            String::new()
        } else {
            format!(
                "<c:title><c:tx><c:rich><a:bodyPr rot=\"-5400000\" vert=\"horz\"/><a:lstStyle/><a:p><a:r><a:rPr lang=\"ko-KR\" sz=\"900\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title>",
                esc(&spec.options.y_title)
            )
        };
        let grid = if spec.options.grid {
            "<c:majorGridlines><c:spPr><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"E6E5E2\"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>"
        } else {
            ""
        };
        format!(
            "<c:catAx><c:axId val=\"111\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"900\"/></a:pPr><a:endParaRPr lang=\"ko-KR\"/></a:p></c:txPr><c:crossAx val=\"222\"/></c:catAx>\
<c:valAx><c:axId val=\"222\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/>{grid}{val_title}<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"900\"/></a:pPr><a:endParaRPr lang=\"ko-KR\"/></a:p></c:txPr><c:crossAx val=\"111\"/></c:valAx>"
        )
    };

    let title = if spec.title.is_empty() {
        "<c:autoTitleDeleted val=\"1\"/>".to_string()
    } else {
        format!(
            "<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"ko-KR\" sz=\"1300\" b=\"1\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>",
            esc(&spec.title)
        )
    };

    // A legend is always present for two or more series — identity is never
    // colour alone.
    let legend = if spec.options.legend && (spec.series.len() > 1 || spec.chart_type.is_radial()) {
        "<c:legend><c:legendPos val=\"b\"/><c:overlay val=\"0\"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"900\"/></a:pPr><a:endParaRPr lang=\"ko-KR\"/></a:p></c:txPr></c:legend>"
    } else {
        ""
    };

    format!(
        "<c:chartSpace xmlns:c=\"{NS_C}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\">\
<c:chart>{title}<c:plotArea><c:layout/>{plot}{axes}</c:plotArea>{legend}<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart>\
</c:chartSpace>"
    )
}

/* ------------------------------------------------------------------ slides */

fn read_asset(dir: &Path, src: &str) -> Option<(Vec<u8>, String)> {
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("data:") {
        return None;
    }
    let abs = ai_format::project::resolve_inside(dir, src.trim_start_matches("./")).ok()?;
    if !abs.is_file() {
        return None;
    }
    let ext = abs.extension()?.to_string_lossy().into_owned();
    Some((std::fs::read(&abs).ok()?, ext))
}

fn slide_part(slide: &Slide, dir: &Path, assets: &mut Assets) -> SlidePart {
    let mut shapes = String::new();
    let mut rels: Vec<(String, String, String)> = Vec::new();
    let mut links: Vec<String> = Vec::new();

    let mut ordered: Vec<&SlideBlock> = slide.blocks.iter().collect();
    ordered.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal));

    // Shape ids start at 2: the group shape holding them all is 1.
    for (id, block) in (2usize..).zip(ordered) {
        match block.kind {
            Kind::Shape => shapes.push_str(&shape_element(id, block, &mut links)),
            Kind::Image => {
                let embedded = image_source(&block.md)
                    .and_then(|src| read_asset(dir, &src))
                    .and_then(|(data, ext)| {
                        let content_type = image_content_type(&ext)?;
                        let natural = image_size(&data).unwrap_or((block.w, block.h));
                        let index = assets.media.len() + 1;
                        let path = format!("media/image{index}.{}", ext.to_ascii_lowercase());
                        assets.media.push((path.clone(), data, content_type));
                        Some((path, natural))
                    });
                match embedded {
                    Some((path, natural)) => {
                        let rel_id = format!("rId{}", rels.len() + 2);
                        rels.push((rel_id.clone(), "image".to_string(), format!("../{path}")));
                        shapes.push_str(&picture_shape(id, block, &rel_id, natural));
                    }
                    None => {
                        // Unresolvable image: keep the alt text so the slide is
                        // not silently empty.
                        let alt = image_alt(&block.md);
                        let text = if alt.is_empty() {
                            "(이미지를 찾을 수 없음)"
                        } else {
                            &alt
                        };
                        shapes.push_str(&placeholder_text(id, block, text));
                    }
                }
            }
            Kind::Chart => match parse_chart_block(&block.md) {
                Some(spec) if !spec.series.is_empty() => {
                    assets.charts.push(chart_xml(&spec));
                    let rel_id = format!("rId{}", rels.len() + 2);
                    rels.push((
                        rel_id.clone(),
                        "chart".to_string(),
                        format!("../charts/chart{}.xml", assets.charts.len()),
                    ));
                    shapes.push_str(&chart_frame(id, block, &rel_id));
                }
                _ => shapes.push_str(&placeholder_text(id, block, "(차트 정의를 읽을 수 없음)")),
            },
            Kind::Table => {
                if parse_markdown_table(&block.md).is_empty() {
                    shapes.push_str(&text_shape(id, block, &mut links));
                } else {
                    shapes.push_str(&table_shape(id, block, &mut links));
                }
            }
            Kind::Text => shapes.push_str(&text_shape(id, block, &mut links)),
        }
    }

    let background = if slide.canvas.bg.as_str() != "#ffffff" {
        format!(
            "<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>",
            hex(slide.canvas.bg.as_str())
        )
    } else {
        String::new()
    };

    let xml = format!(
        "<p:sld xmlns:p=\"{NS_P}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\"><p:cSld>{background}\
<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    );

    for (i, url) in links.iter().enumerate() {
        rels.push((
            format!("rIdLink{}", i + 1),
            "hyperlink".to_string(),
            url.clone(),
        ));
    }

    let notes = slide.notes.trim();
    SlidePart {
        xml,
        rels,
        notes: (!notes.is_empty()).then(|| notes.to_string()),
    }
}

fn notes_slide_xml(notes: &str, slide_index: usize) -> String {
    let paragraphs: String = notes
        .split('\n')
        .map(|line| {
            format!(
                "<a:p><a:r><a:rPr lang=\"ko-KR\" sz=\"1200\"/><a:t>{}</a:t></a:r></a:p>",
                esc(line)
            )
        })
        .collect();
    format!(
        "<p:notes xmlns:p=\"{NS_P}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\"><p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Notes Placeholder {slide_index}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>\
<p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/>\
<p:txBody><a:bodyPr/><a:lstStyle/>{paragraphs}</p:txBody></p:sp>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"
    )
}

/// Export a deck to `.pptx`.
pub fn export(project: &Project) -> Result<Vec<u8>> {
    let slides = project.slides();
    let canvas = slides.first().map(|s| s.canvas).unwrap_or_default();

    let mut assets = Assets::default();
    let parts: Vec<SlidePart> = slides
        .iter()
        .map(|s| slide_part(s, &project.dir, &mut assets))
        .collect();
    let notes_indices: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.notes.as_ref().map(|_| i))
        .collect();

    let mut pkg = Package::new();

    /* ------------------------------------------------------- presentation */
    let mut presentation = format!(
        "<p:presentation xmlns:p=\"{NS_P}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" saveSubsetFonts=\"1\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>"
    );
    for i in 0..slides.len() {
        presentation.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + i,
            i + 2
        ));
    }
    presentation.push_str("</p:sldIdLst>");
    if !notes_indices.is_empty() {
        presentation.push_str(&format!(
            "<p:notesMasterIdLst><p:notesMasterId r:id=\"rId{}\"/></p:notesMasterIdLst>",
            slides.len() + 2
        ));
    }
    presentation.push_str(&format!(
        "<p:sldSz cx=\"{}\" cy=\"{}\"/><p:notesSz cx=\"{}\" cy=\"{}\"/></p:presentation>",
        emu(canvas.w),
        emu(canvas.h),
        emu(canvas.h),
        emu(canvas.w),
    ));

    let mut presentation_rels: Vec<(String, &str, String)> = vec![(
        "rId1".to_string(),
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
        "slideMasters/slideMaster1.xml".to_string(),
    )];
    for i in 0..slides.len() {
        presentation_rels.push((
            format!("rId{}", i + 2),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
            format!("slides/slide{}.xml", i + 1),
        ));
    }
    if !notes_indices.is_empty() {
        presentation_rels.push((
            format!("rId{}", slides.len() + 2),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster",
            "notesMasters/notesMaster1.xml".to_string(),
        ));
    }

    /* ---------------------------------------------------------- structure */
    pkg.add_xml("ppt/presentation.xml", &presentation);
    pkg.add_xml(
        "ppt/_rels/presentation.xml.rels",
        &relationships(&presentation_rels),
    );
    pkg.add_xml("ppt/slideMasters/slideMaster1.xml", &slide_master_xml());
    pkg.add_xml(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        &relationships(&[
            (
                "rId1".to_string(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
                "../slideLayouts/slideLayout1.xml".to_string(),
            ),
            (
                "rId2".to_string(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
                "../theme/theme1.xml".to_string(),
            ),
        ]),
    );
    pkg.add_xml("ppt/slideLayouts/slideLayout1.xml", &slide_layout_xml());
    pkg.add_xml(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        &relationships(&[(
            "rId1".to_string(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
            "../slideMasters/slideMaster1.xml".to_string(),
        )]),
    );
    pkg.add_xml(
        "ppt/theme/theme1.xml",
        &theme_xml(&project.manifest.theme.accent),
    );

    for (i, part) in parts.iter().enumerate() {
        pkg.add_xml(&format!("ppt/slides/slide{}.xml", i + 1), &part.xml);

        let mut rels: Vec<(String, String, String)> = vec![(
            "rId1".to_string(),
            "slideLayout".to_string(),
            "../slideLayouts/slideLayout1.xml".to_string(),
        )];
        rels.extend(part.rels.clone());
        if part.notes.is_some() {
            let notes_number = notes_indices.iter().position(|x| *x == i).unwrap() + 1;
            rels.push((
                format!("rId{}", rels.len() + 1),
                "notesSlide".to_string(),
                format!("../notesSlides/notesSlide{notes_number}.xml"),
            ));
        }
        let typed: Vec<(String, &str, String)> = rels
            .iter()
            .map(|(id, kind, target)| {
                let full: &str = match kind.as_str() {
                    "slideLayout" => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
                    "image" => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
                    "chart" => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
                    "hyperlink" => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                    _ => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide",
                };
                (id.clone(), full, target.clone())
            })
            .collect();
        // A hyperlink is an external target and must say so.
        let external: std::collections::HashSet<&str> = rels
            .iter()
            .filter(|(_, kind, _)| kind == "hyperlink")
            .map(|(id, _, _)| id.as_str())
            .collect();
        pkg.add_xml(
            &format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            &relationships_with_modes(&typed, &external),
        );
    }

    /* -------------------------------------------------------------- notes */
    if !notes_indices.is_empty() {
        pkg.add_xml("ppt/notesMasters/notesMaster1.xml", &notes_master_xml());
        pkg.add_xml(
            "ppt/notesMasters/_rels/notesMaster1.xml.rels",
            &relationships(&[(
                "rId1".to_string(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
                "../theme/theme1.xml".to_string(),
            )]),
        );
        for (n, slide_index) in notes_indices.iter().enumerate() {
            let notes = parts[*slide_index].notes.as_deref().unwrap_or("");
            pkg.add_xml(
                &format!("ppt/notesSlides/notesSlide{}.xml", n + 1),
                &notes_slide_xml(notes, slide_index + 1),
            );
            pkg.add_xml(
                &format!("ppt/notesSlides/_rels/notesSlide{}.xml.rels", n + 1),
                &relationships(&[
                    (
                        "rId1".to_string(),
                        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster",
                        "../notesMasters/notesMaster1.xml".to_string(),
                    ),
                    (
                        "rId2".to_string(),
                        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
                        format!("../slides/slide{}.xml", slide_index + 1),
                    ),
                ]),
            );
        }
    }

    /* ------------------------------------------------------------- charts */
    for (i, xml) in assets.charts.iter().enumerate() {
        pkg.add_xml(&format!("ppt/charts/chart{}.xml", i + 1), xml);
    }
    for (path, data, _) in &assets.media {
        pkg.add(&format!("ppt/{path}"), data.clone());
    }

    /* ------------------------------------------------------ content types */
    let mut content_types = String::from(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
    );
    for i in 0..slides.len() {
        content_types.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>",
            i + 1
        ));
    }
    if !notes_indices.is_empty() {
        content_types.push_str("<Override PartName=\"/ppt/notesMasters/notesMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml\"/>");
        for n in 0..notes_indices.len() {
            content_types.push_str(&format!(
                "<Override PartName=\"/ppt/notesSlides/notesSlide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>",
                n + 1
            ));
        }
    }
    for i in 0..assets.charts.len() {
        content_types.push_str(&format!(
            "<Override PartName=\"/ppt/charts/chart{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/>",
            i + 1
        ));
    }
    for (path, _, content_type) in &assets.media {
        content_types.push_str(&format!(
            "<Override PartName=\"/ppt/{path}\" ContentType=\"{content_type}\"/>"
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
                "ppt/presentation.xml".to_string(),
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

    pkg.finish()
}

/// The minimum master a valid package needs. Slides carry their own geometry, so
/// the master holds no placeholders worth inheriting.
fn slide_master_xml() -> String {
    format!(
        "<p:sldMaster xmlns:p=\"{NS_P}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\"><p:cSld>\
<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld>{}\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>",
        colour_map()
    )
}

fn slide_layout_xml() -> String {
    format!(
        "<p:sldLayout xmlns:p=\"{NS_P}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\">\
<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    )
}

fn notes_master_xml() -> String {
    format!(
        "<p:notesMaster xmlns:p=\"{NS_P}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\"><p:cSld>\
<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Notes Placeholder\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>\
<p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr>\
<p:spPr><a:xfrm><a:off x=\"685800\" y=\"4343400\"/><a:ext cx=\"5486400\" cy=\"4114800\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>\
<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"ko-KR\"/></a:p></p:txBody></p:sp>\
</p:spTree></p:cSld>{}</p:notesMaster>",
        colour_map()
    )
}

fn colour_map() -> &'static str {
    "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>"
}

/// A theme whose accent colours are the validated chart palette, so a chart
/// PowerPoint recolours from the theme still lands on the same hues.
fn theme_xml(accent: &str) -> String {
    let mut scheme = format!(
        "<a:dk1><a:srgbClr val=\"0B0B0B\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1>\
<a:dk2><a:srgbClr val=\"52514E\"/></a:dk2><a:lt2><a:srgbClr val=\"F1F5F9\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"{}\"/></a:accent1>",
        hex(accent)
    );
    for (i, color) in PALETTE_LIGHT.iter().skip(1).take(5).enumerate() {
        scheme.push_str(&format!(
            "<a:accent{}><a:srgbClr val=\"{}\"/></a:accent{}>",
            i + 2,
            hex(color),
            i + 2
        ));
    }
    scheme.push_str(
        "<a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink>",
    );

    format!(
        "<a:theme xmlns:a=\"{NS_A}\" name=\"AI Studio\"><a:themeElements>\
<a:clrScheme name=\"AI Studio\">{scheme}</a:clrScheme>\
<a:fontScheme name=\"AI Studio\">\
<a:majorFont><a:latin typeface=\"Pretendard\"/><a:ea typeface=\"Pretendard\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"Pretendard\"/><a:ea typeface=\"Pretendard\"/><a:cs typeface=\"\"/></a:minorFont>\
</a:fontScheme>\
<a:fmtScheme name=\"AI Studio\">\
<a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
<a:lnStyleLst><a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"25400\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"38100\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst>\
</a:fmtScheme></a:themeElements></a:theme>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_format::chart::normalize_spec;
    use serde_json::json;

    #[test]
    fn a_chart_part_carries_the_numbers_not_a_picture() {
        let spec = normalize_spec(&json!({
            "type": "column", "title": "분기별 매출",
            "labels": ["1분기", "2분기"],
            "series": [{"name": "2026", "values": [120, 133]}],
        }));
        let xml = chart_xml(&spec);
        assert!(xml.contains("<c:barChart>"), "{xml}");
        assert!(xml.contains("<c:v>120</c:v>"), "{xml}");
        assert!(xml.contains("<c:v>1분기</c:v>"), "{xml}");
        assert!(xml.contains("분기별 매출"));
        // A cartesian chart must declare the axes its plot references.
        assert!(
            xml.contains("<c:catAx>") && xml.contains("<c:valAx>"),
            "{xml}"
        );
        assert_eq!(xml.matches("<c:axId val=\"111\"/>").count(), 2);
    }

    #[test]
    fn a_radial_chart_has_no_axes_and_colours_each_point() {
        let spec = normalize_spec(&json!({
            "type": "donut", "labels": ["a", "b"],
            "series": [{"values": [1, 2]}],
        }));
        let xml = chart_xml(&spec);
        assert!(xml.contains("<c:doughnutChart>"), "{xml}");
        assert!(xml.contains("<c:holeSize val=\"58\"/>"));
        assert!(!xml.contains("<c:catAx>"), "a radial chart has no axes");
        assert_eq!(xml.matches("<c:dPt>").count(), 2);
        // A radial chart always gets a legend.
        assert!(xml.contains("<c:legend>"));
    }

    #[test]
    fn a_gap_becomes_zero_because_powerpoint_has_no_gap() {
        let spec = normalize_spec(&json!({
            "labels": ["a", "b"],
            "series": [{"values": [5, null]}],
        }));
        let xml = chart_xml(&spec);
        assert!(xml.contains("<c:pt idx=\"1\"><c:v>0</c:v></c:pt>"), "{xml}");
    }

    #[test]
    fn a_stacked_bar_says_so() {
        let spec = normalize_spec(&json!({
            "type": "bar", "options": { "stacked": true },
            "labels": ["a"], "series": [{"values": [1]}, {"values": [2]}],
        }));
        let xml = chart_xml(&spec);
        assert!(xml.contains("<c:barDir val=\"bar\"/>"), "{xml}");
        assert!(xml.contains("<c:grouping val=\"stacked\"/>"));
        assert!(xml.contains("<c:overlap val=\"100\"/>"));
    }

    #[test]
    fn a_single_series_cartesian_chart_omits_the_legend() {
        let spec = normalize_spec(&json!({ "labels": ["a"], "series": [{"values": [1]}] }));
        assert!(!chart_xml(&spec).contains("<c:legend>"));
    }

    #[test]
    fn geometry_converts_to_emu() {
        let block = SlideBlock {
            id: "b".into(),
            kind: Kind::Text,
            md: "# 제목".into(),
            x: 96.0,
            y: 64.0,
            w: 1088.0,
            h: 70.0,
            z: 1.0,
            style: Default::default(),
            shape: None,
            table: None,
            locked: false,
        };
        let xml = text_shape(2, &block, &mut Vec::new());
        assert!(
            xml.contains(&format!("<a:off x=\"{}\" y=\"{}\"/>", emu(96.0), emu(64.0))),
            "{xml}"
        );
        assert!(xml.contains("제목"));
        // A heading is bold and larger than the base size.
        assert!(xml.contains("b=\"1\""), "{xml}");
    }

    #[test]
    fn a_list_becomes_bulleted_paragraphs() {
        let block = SlideBlock {
            id: "b".into(),
            kind: Kind::Text,
            md: "- 하나\n- 둘".into(),
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
            z: 1.0,
            style: Default::default(),
            shape: None,
            table: None,
            locked: false,
        };
        let xml = text_shape(2, &block, &mut Vec::new());
        assert_eq!(xml.matches("<a:buChar").count(), 2, "{xml}");
        assert!(!xml.contains("<a:buNone/>"));
    }

    #[test]
    fn the_theme_uses_the_chart_palette_for_accents() {
        let xml = theme_xml("#4f46e5");
        assert!(
            xml.contains("<a:accent1><a:srgbClr val=\"4F46E5\"/></a:accent1>"),
            "{xml}"
        );
        assert!(
            xml.contains("<a:accent2><a:srgbClr val=\"EB6834\"/></a:accent2>"),
            "{xml}"
        );
    }
}
