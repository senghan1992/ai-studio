//! Reading a `.docx` into a document project.
//!
//! A Word document is a flow of paragraphs, and so is a Doc section, so the
//! structure maps directly. What does not map is pixel-exact typography: this
//! format renders markdown, so a paragraph's *meaning* — heading level, list,
//! quote, table, alignment, indent — comes across, and the rest is approximated.
//! That is a deliberate trade: the point of opening the file here is to get a
//! document an agent can read, not a pixel copy of Word's rendering.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::json;

use ai_format::ids::{new_block_id, new_section_id};
use ai_format::mdblocks::BlockType;
use ai_format::model::{DocBlock, Margin, Override, Page, Running, Section, Spacing};
use ai_format::table::{apply_markdown_authority, to_markdown_table, CellFormat, TableSpec};

use crate::ooxml::{px_from_half_point, px_from_twip, Node, Package, Relationship, Result};
use crate::{Asset, Warnings};

pub struct Document {
    pub sections: Vec<Section>,
    pub assets: Vec<Asset>,
}

pub fn read(package: &Package, warnings: &mut Warnings) -> Result<Document> {
    let document = package.xml("word/document.xml")?;
    let rels = package.rels_for("word/document.xml");
    let styles = read_styles(package);
    let numbering = read_numbering(package);

    let Some(body) = document.path(&["document", "body"]) else {
        return Ok(Document {
            sections: Vec::new(),
            assets: Vec::new(),
        });
    };

    let running = RunningParts {
        package,
        rels: &rels,
    };
    if package.has("word/comments.xml") {
        warnings.note("검토 주석(메모)은 넘어오지 않습니다");
    }
    let notes = Notes::read(package, body);
    let mut ctx = DocCtx {
        package,
        rels: &rels,
        styles: &styles,
        numbering: &numbering,
        notes: &notes,
        pending_notes: std::cell::RefCell::new(Vec::new()),
        inline: std::cell::RefCell::new(InlineBase::default()),
        warnings,
        assets: Vec::new(),
    };

    // Word splits sections with a `sectPr` on the last paragraph of each; the
    // body's own trailing `sectPr` closes the final one.
    let mut sections: Vec<Section> = Vec::new();
    let mut blocks: Vec<DocBlock> = Vec::new();

    for child in &body.children {
        match child.name.as_str() {
            "p" => {
                let breaks = child.path(&["pPr", "sectPr"]).cloned();
                if let Some(block) = ctx.read_paragraph(child) {
                    push_block(&mut blocks, block);
                }
                if let Some(properties) = breaks {
                    blocks.extend(note_blocks(&notes, &ctx.pending_notes.take()));
                    sections.push(finish_section(
                        &mut blocks,
                        &properties,
                        sections.len(),
                        &running,
                    ));
                }
            }
            "tbl" => {
                if let Some(block) = ctx.read_table(child) {
                    blocks.push(block);
                }
            }
            "sectPr" => {
                blocks.extend(note_blocks(&notes, &ctx.pending_notes.take()));
                sections.push(finish_section(&mut blocks, child, sections.len(), &running));
            }
            _ => {}
        }
    }
    if !blocks.is_empty() || sections.is_empty() {
        blocks.extend(note_blocks(&notes, &ctx.pending_notes.take()));
        let empty = Node::default();
        sections.push(finish_section(
            &mut blocks,
            &empty,
            sections.len(),
            &running,
        ));
    }

    // Word's model: a section that declares no header or footer of its own
    // continues the previous section's. A policy document defines the page
    // number once, up front — without this, every section after the first
    // lost it on the way in.
    for i in 1..sections.len() {
        if sections[i].page.header.is_none() {
            sections[i].page.header = sections[i - 1].page.header.clone();
        }
        if sections[i].page.footer.is_none() {
            sections[i].page.footer = sections[i - 1].page.footer.clone();
        }
    }

    Ok(Document {
        sections,
        assets: ctx.assets,
    })
}

/// Append a block, merging a list item into the run it continues.
///
/// Word has one paragraph per bullet; this format has one block per list. Kept
/// separate they would be joined by a blank line on save, which markdown renders
/// as a loose list — visibly different spacing from the same list typed here.
fn push_block(blocks: &mut Vec<DocBlock>, block: DocBlock) {
    if block.block_type == BlockType::List {
        if let Some(previous) = blocks.last_mut() {
            let same_run = previous.block_type == BlockType::List
                && previous.format_override == block.format_override
                && ordered_marker(&previous.md) == ordered_marker(&block.md);
            if same_run {
                previous.md.push('\n');
                previous.md.push_str(&block.md);
                return;
            }
        }
    }
    blocks.push(block);
}

/// True for a numbered list, so bullets and numbers do not merge into one block.
fn ordered_marker(md: &str) -> bool {
    md.trim_start().starts_with(|c: char| c.is_ascii_digit())
}

/// The blocks a section's footnotes become: a rule, then one paragraph each.
///
/// Word's own layout is a rule at the foot of the page with the notes under it,
/// so this is the same arrangement without the page.
fn note_blocks(notes: &Notes, ids: &[String]) -> Vec<DocBlock> {
    let lines: Vec<String> = ids.iter().filter_map(|id| notes.line(id)).collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = vec![DocBlock {
        id: new_block_id(),
        md: "---".into(),
        block_type: BlockType::Hr,
        format_override: None,
        table: None,
    }];
    for line in lines {
        out.push(DocBlock {
            id: new_block_id(),
            md: line,
            block_type: BlockType::Paragraph,
            // Smaller than the body, as a footnote is in Word.
            format_override: Some(Override {
                style: [("fontSize".to_string(), json!(12))].into_iter().collect(),
                ..Override::default()
            }),
            table: None,
        });
    }
    out
}

/// Close a section: take the blocks gathered so far and read its page setup.
fn finish_section(
    blocks: &mut Vec<DocBlock>,
    properties: &Node,
    index: usize,
    running: &RunningParts,
) -> Section {
    let mut taken = std::mem::take(blocks);
    if taken.is_empty() {
        taken.push(DocBlock {
            id: new_block_id(),
            md: String::new(),
            block_type: BlockType::Paragraph,
            format_override: None,
            table: None,
        });
    }

    // A section is named after its first heading, which is what a reader would
    // call it and what the file on disk will be named.
    let name = taken
        .iter()
        .find(|b| b.block_type == BlockType::Heading)
        .map(|b| ai_format::mdblocks::plain_text(&b.md))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("섹션 {}", index + 1));

    Section {
        id: new_section_id(),
        name,
        page: read_page(properties, running),
        blocks: taken,
        file: None,
    }
}

/// Page size and margins, exactly as the section states them.
///
/// The size is not snapped to A4: an author who set the page to landscape, or
/// typed 180×250mm for a booklet, laid the whole document out against that
/// width. Reflowing it onto A4 on open is the most visible way an imported
/// document stops looking like itself, and the one an author notices first.
fn read_page(properties: &Node, running: &RunningParts) -> Page {
    let size = properties.child("pgSz");
    let width = size.and_then(|s| s.attr_i64("w")).map(px_from_twip);
    let height = size.and_then(|s| s.attr_i64("h")).map(px_from_twip);

    let margin = properties.child("pgMar");
    let edge = |name: &str, fallback: f64| {
        margin
            .and_then(|m| m.attr_i64(name))
            .map(|v| px_from_twip(v).round().clamp(0.0, 400.0))
            .unwrap_or(fallback)
    };
    let margins = Margin {
        top: edge("top", 72.0),
        right: edge("right", 72.0),
        bottom: edge("bottom", 72.0),
        left: edge("left", 72.0),
    };
    let columns = properties
        .child("cols")
        .and_then(|c| c.attr_i64("num"))
        .unwrap_or(1)
        .clamp(1, 4) as u32;

    let header = running.read(properties, "headerReference");
    let footer = running.read(properties, "footerReference");

    let (Some(w), Some(h)) = (width, height) else {
        return Page {
            margin: margins,
            columns,
            header,
            footer,
            ..Page::default()
        };
    };

    // `w:orient` is advisory — Word swaps `w` and `h` itself when the author
    // turns the page — but a file written by another tool may state the
    // orientation and leave the size portrait.
    let landscape = size
        .and_then(|s| s.attr("orient"))
        .is_some_and(|o| o == "landscape");
    let (w, h) = if landscape && h > w { (h, w) } else { (w, h) };

    Page {
        header,
        footer,
        ..Page::sized(w.round(), h.round(), margins, columns)
    }
}

/// The header and footer parts of a document, and how a section reaches them.
///
/// Word keeps them in their own parts and points at them from the section, so a
/// reader that only walks the body never sees the page numbers at all.
struct RunningParts<'a> {
    package: &'a Package,
    rels: &'a HashMap<String, Relationship>,
}

impl RunningParts<'_> {
    fn read(&self, properties: &Node, kind: &str) -> Option<Running> {
        // `default` is the one that applies to every page; `first` and `even`
        // are variants this format has no room for.
        let reference = properties
            .children_named(kind)
            .find(|r| r.attr("type").unwrap_or("default") == "default")
            .or_else(|| properties.children_named(kind).next())?;
        let target = reference.attr("id").and_then(|id| self.rels.get(id))?;
        let document = self.package.xml_opt(&target.target)?;
        let root = document.child("hdr").or_else(|| document.child("ftr"))?;

        // Word aligns the three slots with tab stops, which is also how Excel
        // models a header, so tabs are what splits them.
        let mut slots = [String::new(), String::new(), String::new()];
        for paragraph in root.descendants("p") {
            // With no tabs, the paragraph's alignment says which slot the text
            // belongs in — a centred header is centre text, not left text.
            let mut at = match paragraph
                .path(&["pPr", "jc"])
                .and_then(|j| j.attr("val"))
                .unwrap_or("left")
            {
                "center" => 1usize,
                "right" | "end" => 2,
                _ => 0,
            };
            // In document order, so `PAGE / NUMPAGES` does not come out as
            // `/ PAGE NUMPAGES`.
            collect_running(paragraph, &mut slots, &mut at);
        }
        let running = Running {
            left: slots[0].trim().to_string(),
            center: slots[1].trim().to_string(),
            right: slots[2].trim().to_string(),
        };
        (!running.is_empty()).then_some(running)
    }
}

/// Walk a header paragraph in document order, filling the three slots.
///
/// Word writes a page number either as one `fldSimple` or as a run carrying
/// `instrText` between `fldChar` marks, and the surrounding text is in ordinary
/// runs — so the order only comes out right if all three are read in one pass.
fn collect_running(node: &Node, slots: &mut [String; 3], at: &mut usize) {
    for child in &node.children {
        match child.name.as_str() {
            // Paragraph properties hold the *tab stops*, which are elements
            // named `tab` as well. Walking into them advances the slot before
            // any text arrives, and everything lands on the right.
            "pPr" | "rPr" => {}
            "t" => slots[*at].push_str(&child.all_text()),
            "tab" => *at = (*at + 1).min(2),
            "br" => slots[*at].push(' '),
            "instrText" => {
                if let Some(token) = field_token(&child.all_text()) {
                    slots[*at].push_str(token);
                }
            }
            "fldSimple" => match field_token(child.attr("instr").unwrap_or("")) {
                // The field's own cached result sits inside it; the token
                // replaces it rather than joining it.
                Some(token) => slots[*at].push_str(token),
                None => collect_running(child, slots, at),
            },
            // A run, a hyperlink, a smart-tag wrapper: keep descending.
            _ => collect_running(child, slots, at),
        }
    }
}

/// The token a Word field instruction becomes.
fn field_token(instruction: &str) -> Option<&'static str> {
    let upper = instruction.trim().to_uppercase();
    let name = upper.split_whitespace().next().unwrap_or("");
    match name {
        "PAGE" => Some("{PAGE}"),
        "NUMPAGES" => Some("{PAGES}"),
        "DATE" | "PRINTDATE" | "CREATEDATE" | "SAVEDATE" => Some("{DATE}"),
        _ => None,
    }
}

/// A document's footnotes and endnotes.
///
/// Word draws these at the foot of the page with a rule above them. This format
/// has no page-level anything, so the closest arrangement is what a markdown
/// document does: a superscript marker where the reference was, a horizontal
/// rule at the end of the section, and the notes under it. The words survive,
/// the marker still points at them, and nothing looks out of place.
#[derive(Default)]
struct Notes {
    /// Note id to its text.
    text: HashMap<String, String>,
    /// Note id to the number it is shown as — Word's ids are arbitrary, and the
    /// number a reader sees is the order of appearance.
    number: HashMap<String, usize>,
}

impl Notes {
    fn read(package: &Package, body: &Node) -> Notes {
        let mut notes = Notes::default();
        for (part, root) in [
            ("word/footnotes.xml", "footnotes"),
            ("word/endnotes.xml", "endnotes"),
        ] {
            let Some(document) = package.xml_opt(part) else {
                continue;
            };
            let Some(list) = document.child(root) else {
                continue;
            };
            for note in &list.children {
                let Some(id) = note.attr("id") else {
                    continue;
                };
                // Word's own separator notes carry ids 0 and -1 and no content.
                let text: String = note
                    .descendants("t")
                    .iter()
                    .map(|t| t.all_text())
                    .collect::<String>()
                    .trim()
                    .to_string();
                if !text.is_empty() {
                    notes.text.insert(id.to_string(), text);
                }
            }
        }
        // Numbered in the order the body refers to them.
        let mut next = 1usize;
        for reference in body
            .descendants("footnoteReference")
            .into_iter()
            .chain(body.descendants("endnoteReference"))
        {
            let Some(id) = reference.attr("id") else {
                continue;
            };
            if notes.text.contains_key(id) && !notes.number.contains_key(id) {
                notes.number.insert(id.to_string(), next);
                next += 1;
            }
        }
        notes
    }

    /// The marker shown where the reference was: `1` becomes `¹`.
    fn marker(&self, id: &str) -> Option<String> {
        const SUPERSCRIPT: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
        let number = self.number.get(id)?;
        Some(
            number
                .to_string()
                .chars()
                .filter_map(|c| c.to_digit(10))
                .map(|d| SUPERSCRIPT[d as usize])
                .collect(),
        )
    }

    fn line(&self, id: &str) -> Option<String> {
        Some(format!("{} {}", self.marker(id)?, self.text.get(id)?))
    }
}

/// A stand-in for absent element properties, so a reader can ask for children
/// without every call site branching on "the element was not there".
static EMPTY_NODE: std::sync::LazyLock<Node> = std::sync::LazyLock::new(Node::default);

struct DocCtx<'a> {
    package: &'a Package,
    rels: &'a HashMap<String, Relationship>,
    styles: &'a Styles,
    numbering: &'a Numbering,
    notes: &'a Notes,
    /// Notes referred to since the last section break, in order, waiting to be
    /// written under the section that referred to them.
    pending_notes: std::cell::RefCell<Vec<String>>,
    /// The paragraph being read: what its runs' colour and family are measured
    /// against, so only a run unlike the rest of its paragraph is marked inline.
    inline: std::cell::RefCell<InlineBase>,
    warnings: &'a mut Warnings,
    assets: Vec<Asset>,
}

impl DocCtx<'_> {
    fn read_paragraph(&mut self, p: &Node) -> Option<DocBlock> {
        let properties = p.child("pPr");
        let style_id = properties
            .and_then(|pr| pr.child("pStyle"))
            .and_then(|s| s.attr("val"))
            .unwrap_or("");

        // Runs that differ from the paragraph in colour or family are marked
        // inline; the paragraph-wide colour and family go into the override.
        let runs: Vec<&Node> = p.children_named("r").collect();
        self.inline.replace(InlineBase {
            active: true,
            color: uniform_colour(&runs),
            font: uniform_font(&runs),
        });
        let text = self.runs_to_markdown(p);
        self.inline.replace(InlineBase::default());

        // A paragraph holding only a picture is an image block. But a picture
        // often sits beside text — an inline icon, or a figure with its caption
        // typed on the same line. Taking the image path unconditionally dropped
        // that text, so we only do it when the paragraph has none of its own;
        // otherwise the image markdown is folded into the text below.
        let image_block = self.image_block(p);
        if let Some(block) = &image_block {
            if text.trim().is_empty() {
                return Some(block.clone());
            }
        }
        let image_md = image_block.map(|b| b.md);
        let text = match image_md {
            Some(img) => format!("{img} {text}"),
            None => text,
        };
        let level = self.styles.heading_level(style_id);
        let list = self.list_marker(properties, style_id);

        let (md, block_type) = if level > 0 {
            (
                format!("{} {text}", "#".repeat(level.min(6))),
                BlockType::Heading,
            )
        } else if let Some((marker, indent)) = list {
            (
                format!("{}{marker} {text}", "  ".repeat(indent)),
                BlockType::List,
            )
        } else if self.styles.is_quote(style_id) || properties.is_some_and(has_left_border) {
            (format!("> {text}"), BlockType::Quote)
        } else if text.trim().is_empty() {
            // An empty paragraph is Word's spacing, not content.
            return None;
        } else {
            (
                ai_format::mdblocks::escape_literal_marker(&text),
                BlockType::Paragraph,
            )
        };

        Some(DocBlock {
            id: new_block_id(),
            md,
            block_type,
            format_override: self.paragraph_override(p, style_id, level),
            table: None,
        })
    }

    /// The list marker and depth for a paragraph, if it is a list item.
    ///
    /// The paragraph's own `numPr` wins; failing that its style's does. Either
    /// can supply the level, and the numbering definition says ordered or not.
    fn list_marker(
        &self,
        properties: Option<&Node>,
        style_id: &str,
    ) -> Option<(&'static str, usize)> {
        let own = properties.and_then(|pr| pr.child("numPr"));
        let styled = self.styles.list_of(style_id);
        if own.is_none() && styled.is_none() {
            return None;
        }

        // `numId="0"` explicitly removes numbering from a paragraph, which is how
        // Word un-lists one item inside a list style.
        let own_num = own
            .and_then(|n| n.child("numId"))
            .and_then(|n| n.attr_i64("val"));
        if own_num == Some(0) {
            return None;
        }

        let num_id = own_num.or_else(|| styled.and_then(|s| s.num_id));
        let level = own
            .and_then(|n| n.child("ilvl"))
            .and_then(|l| l.attr_i64("val"))
            .map(|v| v.clamp(0, 8) as usize)
            .or_else(|| styled.map(|s| s.level))
            .unwrap_or(0);

        let ordered = num_id
            .and_then(|id| self.numbering.kind_of(id))
            .or_else(|| styled.and_then(|s| s.ordered))
            .unwrap_or(false);

        Some((if ordered { "1." } else { "-" }, level))
    }

    /// Alignment, indent, spacing and size that differ from the document default.
    fn paragraph_override(&self, p: &Node, style_id: &str, level: usize) -> Option<Override> {
        let mut out = Override::default();
        // A paragraph with no `pPr` can still have run-wide formatting, so the
        // properties are optional rather than a precondition.
        let properties = p.child("pPr").unwrap_or(&EMPTY_NODE);

        if let Some(align) = properties.child("jc").and_then(|j| j.attr("val")) {
            let mapped = match align {
                "center" => Some("center"),
                "right" | "end" => Some("right"),
                "both" | "distribute" => Some("justify"),
                _ => None,
            };
            out.align = mapped.map(str::to_string);
        }
        if let Some(indent) = properties.child("ind").and_then(|i| i.attr_i64("left")) {
            let px = px_from_twip(indent).round();
            if px > 0.0 {
                out.indent = Some(px);
            }
        }
        // Paragraph shading — a highlighted note or a callout box in Word.
        if let Some(fill) = properties
            .child("shd")
            .and_then(|s| s.attr("fill"))
            .filter(|f| !f.eq_ignore_ascii_case("auto"))
            .and_then(crate::ooxml::color)
            .filter(|c| c != "#ffffff")
        {
            out.style.insert("bg".into(), json!(fill));
        }
        if let Some(spacing) = properties.child("spacing") {
            let before = spacing.attr_i64("before").map(|v| px_from_twip(v).round());
            let after = spacing.attr_i64("after").map(|v| px_from_twip(v).round());
            let before = before.filter(|v| *v > 0.0);
            let after = after.filter(|v| *v > 0.0);
            if before.is_some() || after.is_some() {
                out.spacing = Some(Spacing { before, after });
            }
        }

        // Run formatting that applies to the whole paragraph is style, not
        // emphasis: a paragraph in 18px grey is formatting, and marking each run
        // with `**` instead would be wrong.
        let runs: Vec<&Node> = p.children_named("r").collect();
        if let Some(colour) = uniform_colour(&runs) {
            out.style.insert("color".into(), json!(colour));
        }
        if let Some(font) = uniform_font(&runs) {
            out.style.insert("font".into(), json!(font));
        }
        let sizes: Vec<f64> = runs
            .iter()
            .filter_map(|r| r.path(&["rPr", "sz"]))
            .filter_map(|s| s.attr_i64("val"))
            .map(px_from_half_point)
            .collect();
        let uniform_size = (!sizes.is_empty()
            && sizes.len() == runs.len()
            && sizes.iter().all(|s| *s == sizes[0]))
        .then(|| sizes[0]);

        // Run, then the paragraph's own mark, then the style, then the document
        // default — the order Word resolves it in. Only a size the editor would
        // not have drawn anyway is worth writing: an 11pt body paragraph is
        // already 15px here, and an override on it would add an anchor to the
        // markdown for no visible difference.
        let size = uniform_size
            .or_else(|| {
                properties
                    .path(&["rPr", "sz"])
                    .and_then(|s| s.attr_i64("val"))
                    .map(px_from_half_point)
            })
            .or_else(|| self.styles.size_of(style_id));
        let expected = if level > 0 {
            ai_format::doc::heading_px(level)
        } else {
            ai_format::doc::BODY_PX
        };
        if let Some(size) = size
            .map(|s| ai_format::font::px_for_pt(ai_format::font::pt_for_px(s)))
            .filter(|s| *s != expected.round())
        {
            out.style.insert("fontSize".into(), json!(size));
        }

        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn image_block(&mut self, p: &Node) -> Option<DocBlock> {
        let blip = p.descendants("blip").first().copied()?;
        let embed = blip.attr("embed")?;
        let rel = self.rels.get(embed)?;
        let bytes = self.package.bytes(&rel.target)?.to_vec();

        let name = match self.assets.iter().find(|a| a.source == rel.target) {
            Some(existing) => existing.name.clone(),
            None => {
                let base = rel
                    .target
                    .rsplit('/')
                    .next()
                    .unwrap_or("image.png")
                    .to_string();
                let name = crate::unique_asset_name(&base, &self.assets);
                self.assets.push(Asset {
                    name: name.clone(),
                    bytes,
                    source: rel.target.clone(),
                });
                name
            }
        };

        let alt = p
            .descendants("docPr")
            .first()
            .and_then(|d| d.attr("descr").or_else(|| d.attr("name")))
            .unwrap_or("이미지")
            .replace(['[', ']'], "");

        Some(DocBlock {
            id: new_block_id(),
            md: format!("![{alt}](../assets/{name})"),
            block_type: BlockType::Image,
            format_override: None,
            table: None,
        })
    }

    fn runs_to_markdown(&self, p: &Node) -> String {
        self.runs_to_markdown_with(p, false)
    }

    /// The same, with `ignore_bold` where the weight is already implied.
    ///
    /// Runs are not always direct children of the paragraph. A tracked insertion
    /// wraps them in `w:ins`, a content control in `w:sdt`, a spell-check hint in
    /// `w:smartTag` — all common in real documents, and all of them invisible to
    /// a reader that looks only one level down. The text inside a deletion is
    /// skipped, because that is text the author removed.
    fn runs_to_markdown_with(&self, p: &Node, ignore_bold: bool) -> String {
        let mut out = String::new();
        self.collect_runs(p, ignore_bold, &mut out);
        out.trim().to_string()
    }

    fn collect_runs(&self, node: &Node, ignore_bold: bool, out: &mut String) {
        for child in &node.children {
            match child.name.as_str() {
                "r" => out.push_str(&self.run_to_markdown(child, ignore_bold)),
                "hyperlink" => {
                    let mut text = String::new();
                    self.collect_runs(child, ignore_bold, &mut text);
                    let url = child
                        .attr("id")
                        .and_then(|id| self.rels.get(id))
                        .filter(|rel| rel.external)
                        .map(|rel| rel.target.clone());
                    match url {
                        Some(url) => out.push_str(&format!("[{}]({})", text.trim(), url)),
                        None => out.push_str(&text),
                    }
                }
                // Deleted text, and the properties that are not content.
                "del" | "pPr" | "rPr" | "sectPr" => {}
                // `w:ins`, `w:sdt`, `w:sdtContent`, `w:smartTag`, `mc:Fallback`…
                _ => self.collect_runs(child, ignore_bold, out),
            }
        }
    }

    fn run_to_markdown(&self, r: &Node, ignore_bold: bool) -> String {
        // `w:br` is a line break; `w:tab` a tab. Both belong in the text.
        let mut text = String::new();
        for child in &r.children {
            match child.name.as_str() {
                "t" => text.push_str(&child.all_text()),
                "br" => text.push('\n'),
                "tab" => text.push(' '),
                // A footnote or endnote: the marker stays in the sentence, and
                // the note itself is written under the section.
                "footnoteReference" | "endnoteReference" => {
                    if let Some(id) = child.attr("id") {
                        if let Some(marker) = self.notes.marker(id) {
                            text.push_str(&marker);
                            let mut pending = self.pending_notes.borrow_mut();
                            if !pending.iter().any(|p| p == id) {
                                pending.push(id.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // A text box's words are inside the drawing rather than the run, and a
        // pull quote in a text box is often the most prominent line on the page.
        // Only the first copy is taken: Word writes the same content twice, once
        // for modern readers and once as a `v:textbox` fallback.
        if text.trim().is_empty() {
            if let Some(box_content) = r.descendants("txbxContent").first() {
                let mut inner = String::new();
                for paragraph in box_content.children_named("p") {
                    let line = self.runs_to_markdown_with(paragraph, ignore_bold);
                    if !line.is_empty() {
                        if !inner.is_empty() {
                            inner.push('\n');
                        }
                        inner.push_str(&line);
                    }
                }
                return inner;
            }
        }
        if text.trim().is_empty() {
            return text;
        }

        let properties = r.child("rPr");
        let flag = |name: &str| {
            properties.and_then(|p| p.child(name)).is_some_and(|n| {
                // `<w:b/>` means on; `<w:b w:val="0"/>` and `<w:u w:val="none"/>` off.
                !matches!(n.attr("val"), Some("0" | "false" | "none"))
            })
        };
        let underline = flag("u");
        let bold = !ignore_bold && flag("b");
        let italic = flag("i");
        let strike = flag("strike");
        let code = properties
            .and_then(|p| p.child("rFonts"))
            .and_then(|f| f.attr("ascii"))
            .is_some_and(|f| f.contains("Consol") || f.contains("Courier") || f.contains("Mono"));

        let leading: String = text.chars().take_while(|c| c.is_whitespace()).collect();
        let trailing: String = text
            .chars()
            .rev()
            .take_while(|c| c.is_whitespace())
            .collect();
        let mut core = text.trim().to_string();

        if code {
            core = format!("`{core}`");
        }
        if strike {
            core = format!("~~{core}~~");
        }
        match (bold, italic) {
            (true, true) => core = format!("***{core}***"),
            (true, false) => core = format!("**{core}**"),
            (false, true) => core = format!("*{core}*"),
            (false, false) => {}
        }
        if underline {
            core = format!("<u>{core}</u>");
        }

        // A colour or family this run has and its paragraph does not — the one
        // burgundy lead-in of a grey paragraph. The paragraph-wide case is the
        // override's; a black run among default (black) runs is not a colour.
        let base = self.inline.borrow();
        if base.active {
            let own_colour = run_colour(r).filter(|c| {
                base.color.as_deref() != Some(c.as_str())
                    && (base.color.is_some() || c != "#000000")
            });
            let own_font = run_font(r)
                .filter(|f| !f.contains(['"', '<', '>', ';']))
                .filter(|f| base.font.as_deref() != Some(f.as_str()));
            let mut css: Vec<String> = Vec::new();
            if let Some(c) = own_colour {
                css.push(format!("color:{c}"));
            }
            if let Some(f) = own_font {
                css.push(format!("font-family:{f}"));
            }
            if !css.is_empty() {
                core = format!("<span style=\"{}\">{core}</span>", css.join(";"));
            }
        }
        format!("{leading}{core}{trailing}")
    }

    fn read_table(&mut self, tbl: &Node) -> Option<DocBlock> {
        let rows: Vec<&Node> = tbl.children_named("tr").collect();
        if rows.is_empty() {
            return None;
        }

        let grid: Vec<f64> = tbl
            .child("tblGrid")
            .map(|g| {
                g.children_named("gridCol")
                    .map(|c| c.attr_i64("w").map(px_from_twip).unwrap_or(0.0).round())
                    .collect()
            })
            .unwrap_or_default();

        let header_row = rows
            .first()
            .map(|r| r.path(&["trPr", "tblHeader"]).is_some())
            .unwrap_or(false);

        let mut spec = TableSpec {
            cols: grid,
            rows: rows
                .iter()
                .map(|r| {
                    r.path(&["trPr", "trHeight"])
                        .and_then(|h| h.attr_i64("val"))
                        .map(|v| px_from_twip(v).round())
                        .unwrap_or(0.0)
                })
                .collect(),
            merges: Vec::new(),
            header_row,
            banded_rows: false,
            first_col: false,
            style: ai_format::table::TableStyle::Plain,
            cells: IndexMap::new(),
        };

        // Word covers a horizontal merge with the anchor's `gridSpan` and writes
        // no cell for the columns it swallowed; a vertical merge, by contrast,
        // has a real `tc` in every row carrying `<w:vMerge/>`. So the column a
        // cell occupies is the running sum of the spans before it — never its
        // position among its siblings.
        let mut cells: Vec<Vec<String>> = Vec::new();
        let mut open: HashMap<usize, usize> = HashMap::new();
        let width = spec.cols.len().max(
            rows.iter()
                .map(|r| {
                    r.children_named("tc")
                        .map(|tc| {
                            tc.path(&["tcPr", "gridSpan"])
                                .and_then(|g| g.attr_i64("val"))
                                .unwrap_or(1)
                                .max(1) as usize
                        })
                        .sum::<usize>()
                })
                .max()
                .unwrap_or(0),
        );

        for (r, row) in rows.iter().enumerate() {
            let mut line: Vec<String> = vec![String::new(); width];
            let mut column = 0usize;

            for tc in row.children_named("tc") {
                if column >= width {
                    break;
                }
                let properties = tc.child("tcPr");
                let span = properties
                    .and_then(|pr| pr.child("gridSpan"))
                    .and_then(|g| g.attr_i64("val"))
                    .unwrap_or(1)
                    .max(1) as usize;
                let vmerge = properties.and_then(|pr| pr.child("vMerge"));
                let continues = vmerge.is_some_and(|v| !matches!(v.attr("val"), Some("restart")));

                if continues {
                    // Extend the merge that started above, if there is one.
                    if let Some(start) = open.get(&column).copied() {
                        record_merge(&mut spec, column, start, r, span);
                    }
                    column += span;
                    continue;
                }
                if vmerge.is_some() {
                    open.insert(column, r);
                } else {
                    open.remove(&column);
                }
                if span > 1 {
                    record_merge(&mut spec, column, r, r, span);
                }

                // A header row and a first column are bold by definition, so
                // their `<w:b/>` is not emphasis to carry into the markdown.
                let implied_bold = header_row && r == 0;
                // A cell's own paragraphs, and the paragraphs of any table nested
                // inside it. A markdown table cannot hold a table, so the nested
                // one's cells become lines in this one — which keeps the words and
                // their order, where skipping them loses the cell entirely.
                let text = tc
                    .children
                    .iter()
                    .filter(|child| matches!(child.name.as_str(), "p" | "tbl"))
                    .flat_map(|child| match child.name.as_str() {
                        "p" => vec![self.runs_to_markdown_with(child, implied_bold)],
                        _ => child
                            .descendants("p")
                            .iter()
                            .map(|p| self.runs_to_markdown_with(p, implied_bold))
                            .collect(),
                    })
                    .filter(|t| !t.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("<br>");
                line[column] = text;

                if let Some(format) = cell_format(tc) {
                    spec.cells
                        .insert(ai_formula::refs::to_ref(column, r), format);
                }
                column += span;
            }
            cells.push(line);
        }
        // Merges that never continued past their first row are not merges.
        spec.merges.retain(|m| {
            ai_formula::refs::parse_range(m)
                .map(|r| r.rows() > 1 || r.cols() > 1)
                .unwrap_or(false)
        });
        spec.merges.sort();
        spec.merges.dedup();

        if cells.iter().flatten().all(|c| c.trim().is_empty()) {
            self.warnings.note("빈 표는 건너뛰었습니다");
            return None;
        }

        let md = to_markdown_table(&cells, &spec);
        apply_markdown_authority(&mut spec, &md);

        Some(DocBlock {
            id: new_block_id(),
            md,
            block_type: BlockType::Table,
            format_override: None,
            table: Some(spec),
        })
    }
}

fn record_merge(spec: &mut TableSpec, column: usize, from_row: usize, to_row: usize, span: usize) {
    let anchor = ai_formula::refs::to_ref(column, from_row);
    let end = ai_formula::refs::to_ref(column + span - 1, to_row);
    let range = format!("{anchor}:{end}");
    // Replace a shorter merge starting at the same anchor.
    spec.merges
        .retain(|m| !m.starts_with(&format!("{anchor}:")));
    spec.merges.push(range);
}

fn cell_format(tc: &Node) -> Option<CellFormat> {
    let properties = tc.child("tcPr");
    let fill = properties
        .and_then(|pr| pr.child("shd"))
        .and_then(|s| s.attr("fill"))
        .filter(|f| !f.eq_ignore_ascii_case("auto"))
        .and_then(crate::ooxml::color);
    let valign = properties
        .and_then(|pr| pr.child("vAlign"))
        .and_then(|v| v.attr("val"))
        .and_then(|v| {
            Some(match v {
                "top" => "top",
                "center" => "middle",
                "bottom" => "bottom",
                _ => return None,
            })
        });
    let align = tc
        .children_named("p")
        .next()
        .and_then(|p| p.path(&["pPr", "jc"]))
        .and_then(|j| j.attr("val"))
        .and_then(|v| {
            Some(match v {
                "center" => "center",
                "right" | "end" => "right",
                "left" | "start" => "left",
                _ => return None,
            })
        });

    let format = CellFormat {
        align: align.map(str::to_string),
        valign: valign.map(str::to_string),
        fill,
        color: None,
    };
    (!format.is_empty()).then_some(format)
}

/// What a paragraph's runs are measured against when marking one inline.
#[derive(Default)]
struct InlineBase {
    active: bool,
    color: Option<String>,
    font: Option<String>,
}

/// A run's stated colour, as `#rrggbb`. `auto` and theme colours are not stated.
fn run_colour(r: &Node) -> Option<String> {
    r.path(&["rPr", "color"])
        .and_then(|c| c.attr("val"))
        .and_then(crate::ooxml::color)
}

/// A run's family, when it names a real one. Korean documents put it in
/// `eastAsia`; `ascii` is the Latin fallback beside it.
fn run_font(r: &Node) -> Option<String> {
    r.path(&["rPr", "rFonts"])
        .and_then(|f| f.attr("eastAsia").or_else(|| f.attr("ascii")))
        .map(str::trim)
        .filter(|f| ai_format::font::is_substitution(f))
        .map(str::to_string)
}

/// Non-blank characters in a run, so a colour's weight is the text it covers.
fn run_weight(r: &Node) -> usize {
    r.descendants("t")
        .iter()
        .map(|t| t.all_text().chars().filter(|c| !c.is_whitespace()).count())
        .sum::<usize>()
        .max(1)
}

/// The value most of a paragraph's text carries, when every run states one.
///
/// A paragraph's colour is formatting only if it applies to the whole
/// paragraph, so a run left at the default must veto it — otherwise that run
/// would take the paragraph's colour on screen. When every run does state a
/// colour, the one covering the most characters is the paragraph's and the
/// minority runs are marked inline.
fn dominant_stated<F>(runs: &[&Node], stated: F) -> Option<String>
where
    F: Fn(&Node) -> Option<String>,
{
    if runs.is_empty() {
        return None;
    }
    let mut weights: Vec<(String, usize)> = Vec::new();
    for run in runs {
        let value = stated(run)?;
        match weights.iter_mut().find(|(v, _)| *v == value) {
            Some((_, w)) => *w += run_weight(run),
            None => weights.push((value, run_weight(run))),
        }
    }
    let mut best: Option<(String, usize)> = None;
    for (value, weight) in weights {
        if best.as_ref().is_none_or(|(_, w)| weight > *w) {
            best = Some((value, weight));
        }
    }
    best.map(|(value, _)| value)
}

/// The paragraph's colour — see `dominant_stated`. Plain black is the default
/// and not worth an override.
fn uniform_colour(runs: &[&Node]) -> Option<String> {
    dominant_stated(runs, run_colour).filter(|c| c != "#000000")
}

/// The paragraph's family — see `dominant_stated`.
fn uniform_font(runs: &[&Node]) -> Option<String> {
    dominant_stated(runs, run_font)
}

fn has_left_border(properties: &Node) -> bool {
    properties
        .path(&["pBdr", "left"])
        .is_some_and(|b| b.attr("val").is_some_and(|v| v != "none" && v != "nil"))
}

/* -------------------------------------------------------------------- styles */

/// The style definitions a paragraph can point at.
///
/// Heading level comes from the style, not the text: Word has no `#`, so
/// `<w:pStyle w:val="Heading2"/>` is the only thing that says "this is an H2".
/// A localised template names its styles differently, so `outlineLvl` is read
/// too — it is the language-independent signal.
#[derive(Default)]
struct Styles {
    heading: HashMap<String, usize>,
    quote: Vec<String>,
    /// A style that makes its paragraphs a list, and the numbering it uses.
    ///
    /// `List Bullet` and `List Number` put their `numPr` in the *style*, not on
    /// the paragraph, so a reader that only looks at the paragraph turns every
    /// Word list into a run of plain paragraphs.
    list: HashMap<String, StyleList>,
    /// The text size a style sets, in px, with `basedOn` already followed.
    ///
    /// Word puts the body size in `docDefaults` and the heading sizes in the
    /// styles; a run states a size only where the author overrode one. Reading
    /// runs alone draws a 16pt heading at the app's own 25px and a 10pt report
    /// at 15px.
    sizes: HashMap<String, f64>,
    /// `docDefaults`, which is the size of everything that says nothing.
    default_size: Option<f64>,
}

#[derive(Clone, Copy)]
struct StyleList {
    num_id: Option<i64>,
    /// Set when the style's name says which kind it is.
    ordered: Option<bool>,
    level: usize,
}

impl Styles {
    fn heading_level(&self, style_id: &str) -> usize {
        self.heading.get(style_id).copied().unwrap_or(0)
    }

    fn is_quote(&self, style_id: &str) -> bool {
        self.quote.iter().any(|q| q == style_id)
    }

    fn list_of(&self, style_id: &str) -> Option<StyleList> {
        self.list.get(style_id).copied()
    }

    /// The size a style implies, or the document default.
    fn size_of(&self, style_id: &str) -> Option<f64> {
        self.sizes.get(style_id).copied().or(self.default_size)
    }
}

fn read_styles(package: &Package) -> Styles {
    let mut out = Styles::default();
    let Some(document) = package.xml_opt("word/styles.xml") else {
        return out;
    };
    let Some(root) = document.child("styles") else {
        return out;
    };

    out.default_size = root
        .path(&["docDefaults", "rPrDefault", "rPr", "sz"])
        .and_then(|s| s.attr_i64("val"))
        .map(px_from_half_point);

    // `basedOn` is followed after the whole file is read, because a style may be
    // based on one defined further down.
    let mut own_size: HashMap<String, f64> = HashMap::new();
    let mut based_on: HashMap<String, String> = HashMap::new();

    for style in root.children_named("style") {
        let Some(id) = style.attr("styleId") else {
            continue;
        };
        if let Some(size) = style
            .path(&["rPr", "sz"])
            .and_then(|s| s.attr_i64("val"))
            .map(px_from_half_point)
        {
            own_size.insert(id.to_string(), size);
        }
        if let Some(parent) = style.child("basedOn").and_then(|b| b.attr("val")) {
            based_on.insert(id.to_string(), parent.to_string());
        }
        let name = style
            .child("name")
            .and_then(|n| n.attr("val"))
            .unwrap_or_default()
            .to_ascii_lowercase();

        // `outlineLvl` is zero-based; `heading 3` in the name is one-based.
        let level = style
            .path(&["pPr", "outlineLvl"])
            .and_then(|o| o.attr_i64("val"))
            .map(|v| (v + 1).clamp(1, 6) as usize)
            .or_else(|| {
                name.strip_prefix("heading ")
                    .and_then(|n| n.trim().parse::<usize>().ok())
                    .map(|n| n.clamp(1, 6))
            })
            .or_else(|| {
                id.strip_prefix("Heading")
                    .and_then(|n| n.parse::<usize>().ok())
                    .map(|n| n.clamp(1, 6))
            });
        if let Some(level) = level {
            out.heading.insert(id.to_string(), level);
        }
        if name.contains("quote") || name.contains("인용") {
            out.quote.push(id.to_string());
        }

        // A list style carries its numbering in its own `pPr`.
        let numbering = style.path(&["pPr", "numPr"]);
        let named_list = name.starts_with("list ")
            || name.starts_with("목록")
            || id.starts_with("List")
            || id.starts_with("ListParagraph");
        if numbering.is_some() || named_list {
            let ordered = if name.contains("number") || name.contains("번호") {
                Some(true)
            } else if name.contains("bullet") || name.contains("글머리") {
                Some(false)
            } else {
                None
            };
            // `List Paragraph` on its own is not a list: Word applies it to any
            // indented paragraph. Only take it when it has numbering.
            let is_plain_list_paragraph = id == "ListParagraph" && numbering.is_none();
            if !is_plain_list_paragraph {
                out.list.insert(
                    id.to_string(),
                    StyleList {
                        num_id: numbering
                            .and_then(|n| n.child("numId"))
                            .and_then(|n| n.attr_i64("val")),
                        ordered,
                        level: numbering
                            .and_then(|n| n.child("ilvl"))
                            .and_then(|l| l.attr_i64("val"))
                            .map(|v| v.clamp(0, 8) as usize)
                            .unwrap_or(0),
                    },
                );
            }
        }
    }

    // A style with no size of its own takes its parent's, up the chain. Ten steps
    // is more than any real file and stops a cycle from hanging the import.
    for id in own_size.keys().chain(based_on.keys()) {
        let mut current = id.clone();
        let mut size = None;
        for _ in 0..10 {
            if let Some(found) = own_size.get(&current) {
                size = Some(*found);
                break;
            }
            match based_on.get(&current) {
                Some(parent) => current = parent.clone(),
                None => break,
            }
        }
        if let Some(size) = size {
            out.sizes.insert(id.clone(), size);
        }
    }

    // Word's built-in ids, in case the file omits their definitions.
    for level in 1..=6 {
        out.heading
            .entry(format!("Heading{level}"))
            .or_insert(level);
        out.heading
            .entry(format!("heading {level}"))
            .or_insert(level);
    }
    out.quote.push("Quote".to_string());
    out.quote.push("IntenseQuote".to_string());

    // Word's built-in list styles, in case the file omits their definitions.
    for (id, ordered) in [("ListBullet", false), ("ListNumber", true)] {
        out.list.entry(id.to_string()).or_insert(StyleList {
            num_id: None,
            ordered: Some(ordered),
            level: 0,
        });
        // Levels 2..5 have their own ids.
        for level in 2..=5 {
            out.list.entry(format!("{id}{level}")).or_insert(StyleList {
                num_id: None,
                ordered: Some(ordered),
                level: level - 1,
            });
        }
    }
    out
}

/// Which numbering definitions are ordered, so a list gets the right marker.
#[derive(Default)]
struct Numbering {
    ordered: HashMap<i64, bool>,
}

impl Numbering {
    /// `Some(true)` for an ordered list, `Some(false)` for bullets, `None` when
    /// the file has no definition for this id.
    fn kind_of(&self, num_id: i64) -> Option<bool> {
        self.ordered.get(&num_id).copied()
    }
}

fn read_numbering(package: &Package) -> Numbering {
    let mut out = Numbering::default();
    let Some(document) = package.xml_opt("word/numbering.xml") else {
        return out;
    };
    let Some(root) = document.child("numbering") else {
        return out;
    };

    // `num` points at an `abstractNum`, which is where the format lives.
    let mut abstracts: HashMap<i64, bool> = HashMap::new();
    for definition in root.children_named("abstractNum") {
        let Some(id) = definition.attr_i64("abstractNumId") else {
            continue;
        };
        let ordered = definition
            .children_named("lvl")
            .next()
            .and_then(|lvl| lvl.child("numFmt"))
            .and_then(|f| f.attr("val"))
            .is_some_and(|f| f != "bullet" && f != "none");
        abstracts.insert(id, ordered);
    }
    for num in root.children_named("num") {
        let Some(id) = num.attr_i64("numId") else {
            continue;
        };
        let ordered = num
            .child("abstractNumId")
            .and_then(|a| a.attr_i64("val"))
            .and_then(|a| abstracts.get(&a).copied())
            .unwrap_or(false);
        out.ordered.insert(id, ordered);
    }
    out
}
