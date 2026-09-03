//! Reading a `.pptx` into a deck project.
//!
//! A slide is a pixel canvas in both formats, so geometry crosses over exactly:
//! EMU divided by 914400, times 96. That is why an imported deck can genuinely
//! look like itself rather than approximately like itself.
//!
//! Text is imported as **plain text carrying its real font size**, not as
//! markdown headings. A `#` renders at 1.9em of the block's size here, so
//! turning a 40px title into `# 제목` would draw it at 76px — the structure
//! would be right and the appearance wrong. The slide's title placeholder still
//! becomes the slide title in the frontmatter, which is where `AI.md` needs it.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::{json, Value as Json};

use ai_format::blocks::Kind;
use ai_format::chart::{ChartSpec, Series};
use ai_format::geometry::{Canvas, DEFAULT_CANVAS};
use ai_format::ids::{new_block_id, new_slide_id};
use ai_format::model::{Slide, SlideBlock};
use ai_format::shape::{Dash, Fill, Line, ShapeSpec};
use ai_format::table::{apply_markdown_authority, to_markdown_table, CellFormat, TableSpec};

use crate::ooxml::{px, solid_color, Node, Package, Relationship, Result};
use crate::{Asset, Warnings};

/// What a placeholder inherits when the slide itself does not say.
#[derive(Clone, Debug, Default)]
struct Inherited {
    geometry: Option<Geometry>,
    /// Bullet per level, from the placeholder's own `lstStyle`. `None` at a level
    /// means the placeholder said nothing and the master decides.
    bullets: Vec<Option<Option<BulletStyle>>>,
    /// Size, weight, colour and alignment per level, from the same `lstStyle`.
    text: Vec<TextDefaults>,
}

/// The text formatting a level declares, wherever it was declared.
///
/// A title in a real deck carries no `sz` on its runs — 44pt lives in the
/// master's `titleStyle`, and the layout may override it to 32pt for this one
/// layout. Reading only the run leaves every imported title at the app's own
/// body size, which is the difference between a deck that looks like itself and
/// one that looks like a text dump.
#[derive(Clone, Debug, Default)]
struct TextDefaults {
    /// px, already converted from hundredths of a point.
    size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    color: Option<String>,
    align: Option<String>,
    /// Line spacing as a multiplier, from `lnSpc/spcPct`.
    line: Option<f64>,
}

impl TextDefaults {
    /// Fill in from a weaker source: `self` is the nearer declaration and wins
    /// field by field, which is how OOXML inheritance actually works. A layout
    /// that states only the size still takes the master's colour.
    fn under(mut self, weaker: &TextDefaults) -> TextDefaults {
        self.size = self.size.or(weaker.size);
        self.bold = self.bold.or(weaker.bold);
        self.italic = self.italic.or(weaker.italic);
        self.color = self.color.clone().or_else(|| weaker.color.clone());
        self.align = self.align.clone().or_else(|| weaker.align.clone());
        self.line = self.line.or(weaker.line);
        self
    }

    fn is_empty(&self) -> bool {
        self.size.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.color.is_none()
            && self.align.is_none()
            && self.line.is_none()
    }
}

/// Whether a text body holds text somebody typed, as opposed to only fields.
///
/// The distinction decides whether a template's footer placeholder is content or
/// furniture: `<a:r><a:t>대외비</a:t></a:r>` is content, and a lone
/// `<a:fld type="slidenum">` is not.
fn authored(body: Option<&Node>) -> bool {
    let Some(body) = body else { return false };
    body.descendants("p").iter().any(|paragraph| {
        paragraph
            .children_named("r")
            .any(|run| !run.all_text().trim().is_empty())
    })
}

/// A boolean attribute that distinguishes "off" from "not stated".
///
/// A title placeholder writes `b="0"` to undo the master's bold. Reading that as
/// "says nothing" lets the master win and the title comes in bold anyway.
fn flag(node: Option<&Node>, name: &str) -> Option<bool> {
    node.and_then(|n| n.attr(name))
        .map(|v| matches!(v, "1" | "true" | "on"))
}

/// The alignment name for an OOXML `algn` value.
fn align_name(algn: &str) -> Option<&'static str> {
    match algn {
        "ctr" => Some("center"),
        "r" => Some("right"),
        "just" => Some("justify"),
        "l" => Some("left"),
        _ => None,
    }
}

/// Line spacing as a multiplier: `spcPct val="150000"` is 1.5.
///
/// `spcPts` is an absolute height in points, which cannot become a multiplier
/// without knowing the font size, so it is left to the renderer's default.
fn line_spacing(properties: &Node) -> Option<f64> {
    let pct = properties
        .path(&["lnSpc", "spcPct"])
        .and_then(|n| n.attr_i64("val"))?;
    Some(((pct as f64 / 100_000.0) * 100.0).round() / 100.0)
}

/// The formatting one `lvlNpPr`-shaped node declares.
fn defaults_from_props(properties: &Node, theme: &Theme) -> TextDefaults {
    let run = properties.child("defRPr");
    TextDefaults {
        size: run
            .and_then(|r| r.attr_i64("sz"))
            .map(|sz| ai_format::font::px_for_pt(sz as f64 / 100.0)),
        bold: flag(run, "b"),
        italic: flag(run, "i"),
        color: run
            .and_then(|r| r.child("solidFill"))
            .and_then(|f| theme.color_of(f)),
        align: properties
            .attr("algn")
            .and_then(align_name)
            .map(str::to_string),
        line: line_spacing(properties),
    }
}

/// Nine levels of text formatting out of an `a:lstStyle` or `p:*Style`.
fn text_levels_of(style: &Node, theme: &Theme) -> Vec<TextDefaults> {
    (1..=9)
        .map(|level| {
            style
                .child(&format!("lvl{level}pPr"))
                .map(|node| defaults_from_props(node, theme))
                .unwrap_or_default()
        })
        .collect()
}

type Placeholders = HashMap<(String, String), Inherited>;

/// The slide master's colour scheme and list styles.
///
/// Most shapes in a real deck do not state their own colours: they reference the
/// theme (`accent1` shaded 50%) through `p:style`. Reading only explicit fills
/// imports those shapes as invisible, which is the single most visible way an
/// imported deck fails to look like itself.
#[derive(Default)]
struct Theme {
    /// Scheme name (`accent1`, `dk1`, …) to `#rrggbb`.
    colors: HashMap<String, String>,
    /// `tx1` -> `dk1` and friends, from the master's `clrMap`.
    map: HashMap<String, String>,
    /// The master's `bodyStyle`, level by level.
    body_bullets: Vec<Option<Option<BulletStyle>>>,
    /// The master's `titleStyle`. Titles are not bulleted in any real template,
    /// but reading it rather than assuming keeps an unusual one working.
    title_bullets: Vec<Option<Option<BulletStyle>>>,
    /// The master's text styles, level by level: where a title's 44pt lives.
    title_text: Vec<TextDefaults>,
    body_text: Vec<TextDefaults>,
    /// `otherStyle`, which a text box and an autoshape follow. PowerPoint's own
    /// default is 18pt, and it is stated here rather than assumed.
    other_text: Vec<TextDefaults>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BulletStyle {
    Char,
    AutoNum,
}

/// The bullet a `lvlNpPr` declares: `Some` for a bullet, `None` for `buNone`.
///
/// The distinction between "says no bullet" and "says nothing" is the whole
/// point — a subtitle's `buNone` has to beat the master's `buChar`.
fn bullet_from_props(node: &Node) -> Option<Option<BulletStyle>> {
    if node.child("buNone").is_some() {
        return Some(None);
    }
    if node.child("buAutoNum").is_some() {
        return Some(Some(BulletStyle::AutoNum));
    }
    if node.child("buChar").is_some() {
        return Some(Some(BulletStyle::Char));
    }
    None
}

/// Nine levels of bullet declarations out of an `a:lstStyle` or `p:*Style`.
fn levels_of(style: &Node) -> Vec<Option<Option<BulletStyle>>> {
    (1..=9)
        .map(|level| {
            style
                .child(&format!("lvl{level}pPr"))
                .and_then(bullet_from_props)
        })
        .collect()
}

impl Theme {
    /// Resolve a `schemeClr` value to a concrete colour.
    fn scheme(&self, name: &str) -> Option<String> {
        let mapped = self.map.get(name).map(String::as_str).unwrap_or(name);
        self.colors.get(mapped).cloned()
    }

    /// A colour element, following a scheme reference and its modifiers.
    fn color_of(&self, node: &Node) -> Option<String> {
        if let Some(found) = node.descendants("srgbClr").first() {
            let base = found.attr("val").and_then(crate::ooxml::color)?;
            return Some(apply_modifiers(&base, found));
        }
        if let Some(found) = node.descendants("schemeClr").first() {
            let base = self.scheme(found.attr("val")?)?;
            return Some(apply_modifiers(&base, found));
        }
        if let Some(found) = node.descendants("sysClr").first() {
            let base = found
                .attr("lastClr")
                .and_then(crate::ooxml::color)
                .or_else(|| found.attr("val").and_then(crate::ooxml::color))?;
            return Some(apply_modifiers(&base, found));
        }
        None
    }

    /// The master's bullet for a level of the given placeholder family.
    fn bullet_for(&self, family: Family, level: usize) -> Option<Option<BulletStyle>> {
        let levels = match family {
            Family::Title => &self.title_bullets,
            Family::Body => &self.body_bullets,
            // A text box or autoshape follows `otherStyle`, which no template
            // bullets; treat it as an explicit "no bullet".
            Family::Other => return Some(None),
        };
        levels.get(level).copied().flatten()
    }

    /// A `p:bg`'s colour, whether it states one or references the theme.
    ///
    /// `bgRef idx="1001"` names a fill style in the theme as well as a colour;
    /// only the colour is read, because a gradient or texture behind the slide
    /// is not something this format can hold.
    fn background_of(&self, bg: &Node) -> Option<String> {
        if let Some(fill) = bg.path(&["bgPr", "solidFill"]) {
            return self.color_of(fill).or_else(|| solid_color(fill));
        }
        if let Some(reference) = bg.child("bgRef") {
            return self.color_of(reference);
        }
        // A `bgPr` with a gradient or a picture: take its first colour rather
        // than leaving the slide white, which is further from the original.
        bg.child("bgPr").and_then(|p| self.color_of(p))
    }

    /// The master's text formatting for a level of the given family.
    fn text_for(&self, family: Family, level: usize) -> TextDefaults {
        let levels = match family {
            Family::Title => &self.title_text,
            Family::Body => &self.body_text,
            Family::Other => &self.other_text,
        };
        levels.get(level).cloned().unwrap_or_default()
    }
}

/// Which of the master's text styles a shape follows.
#[derive(Clone, Copy, PartialEq)]
enum Family {
    Title,
    Body,
    Other,
}

impl Family {
    /// A shape with no `p:ph` is a text box or an autoshape; one with a `ph` and
    /// no type defaults to body, which is how Office writes a content
    /// placeholder.
    fn of(placeholder: Option<&Node>) -> Family {
        match placeholder {
            None => Family::Other,
            Some(ph) => match ph.attr("type").unwrap_or("body") {
                "title" | "ctrTitle" => Family::Title,
                "dt" | "ftr" | "sldNum" => Family::Other,
                _ => Family::Body,
            },
        }
    }
}

/// Apply `lumMod`/`lumOff`/`shade`/`tint` to a base colour.
///
/// Office's default shape outline is `accent1` at 50% shade; without this every
/// outline would come in at the accent's full strength, which reads as a
/// different design.
///
/// The arithmetic is in sRGB rather than linear light, which is a shade or two
/// off what Office computes. Matching exactly would mean a full colour-space
/// conversion for a difference no one can see next to the original.
fn apply_modifiers(base: &str, node: &Node) -> String {
    let (mut r, mut g, mut b) = match rgb(base) {
        Some(rgb) => rgb,
        None => return base.to_string(),
    };
    let factor = |name: &str| {
        node.child(name)
            .and_then(|n| n.attr_i64("val"))
            .map(|v| v as f64 / 100_000.0)
    };

    if let Some(shade) = factor("shade") {
        r *= shade;
        g *= shade;
        b *= shade;
    }
    if let Some(tint) = factor("tint") {
        r = r + (255.0 - r) * (1.0 - tint);
        g = g + (255.0 - g) * (1.0 - tint);
        b = b + (255.0 - b) * (1.0 - tint);
    }
    if let Some(lum) = factor("lumMod") {
        r *= lum;
        g *= lum;
        b *= lum;
    }
    if let Some(off) = factor("lumOff") {
        r += 255.0 * off;
        g += 255.0 * off;
        b += 255.0 * off;
    }
    format!(
        "#{:02x}{:02x}{:02x}",
        r.clamp(0.0, 255.0).round() as u8,
        g.clamp(0.0, 255.0).round() as u8,
        b.clamp(0.0, 255.0).round() as u8
    )
}

/// The average of a list of colours, for the fills this format draws as one.
fn average_color(colors: &[String]) -> Option<String> {
    let parsed: Vec<(f64, f64, f64)> = colors.iter().filter_map(|c| rgb(c)).collect();
    if parsed.is_empty() {
        return None;
    }
    let n = parsed.len() as f64;
    let (r, g, b) = parsed.iter().fold((0.0, 0.0, 0.0), |acc, c| {
        (acc.0 + c.0, acc.1 + c.1, acc.2 + c.2)
    });
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        (r / n).round() as u8,
        (g / n).round() as u8,
        (b / n).round() as u8
    ))
}

fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let v = hex.trim_start_matches('#');
    if v.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&v[0..2], 16).ok()? as f64,
        u8::from_str_radix(&v[2..4], 16).ok()? as f64,
        u8::from_str_radix(&v[4..6], 16).ok()? as f64,
    ))
}

/// Read the colour scheme, colour map and body list style behind a slide.
fn read_theme(package: &Package, slide_rels: &HashMap<String, Relationship>) -> Theme {
    let mut theme = Theme::default();

    let Some(layout) = slide_rels.values().find(|r| r.kind == "slideLayout") else {
        return theme;
    };
    let layout_rels = package.rels_for(&layout.target);
    let Some(master_rel) = layout_rels.values().find(|r| r.kind == "slideMaster") else {
        return theme;
    };
    let Some(master_doc) = package.xml_opt(&master_rel.target) else {
        return theme;
    };
    let master = match master_doc.child("sldMaster") {
        Some(master) => master,
        None => return theme,
    };

    if let Some(map) = master.child("clrMap") {
        for (key, value) in &map.attrs {
            theme.map.insert(key.clone(), value.clone());
        }
    }

    // The colour scheme is read before the text styles, which reference it:
    // `bodyStyle` states its colour as `tx1`, and resolving that needs the map
    // and the scheme already in hand.
    let master_rels = package.rels_for(&master_rel.target);
    if let Some(theme_rel) = master_rels.values().find(|r| r.kind == "theme") {
        if let Some(document) = package.xml_opt(&theme_rel.target) {
            if let Some(scheme) = document.path(&["theme", "themeElements", "clrScheme"]) {
                for entry in &scheme.children {
                    if let Some(color) = crate::ooxml::solid_color(entry) {
                        theme.colors.insert(entry.name.clone(), color);
                    }
                }
            }
        }
    }

    if let Some(body) = master.path(&["txStyles", "bodyStyle"]) {
        theme.body_bullets = levels_of(body);
        theme.body_text = text_levels_of(body, &theme);
    }
    if let Some(title) = master.path(&["txStyles", "titleStyle"]) {
        theme.title_bullets = levels_of(title);
        theme.title_text = text_levels_of(title, &theme);
    }
    if let Some(other) = master.path(&["txStyles", "otherStyle"]) {
        theme.other_text = text_levels_of(other, &theme);
    }

    // A master with no `otherStyle` still draws a text box at PowerPoint's own
    // default of 18pt, so the app has to as well or every caption comes in
    // smaller than it was.
    if theme.other_text.iter().all(TextDefaults::is_empty) {
        theme.other_text = (0..9)
            .map(|_| TextDefaults {
                size: Some(ai_format::font::px_for_pt(18.0)),
                ..TextDefaults::default()
            })
            .collect();
    }
    theme
}

/// The layout and master behind one slide: their shape trees, their
/// relationships and their background.
///
/// A deck built from any Office theme keeps its banner, rule and logo here, not
/// on the slide. Reading the slide alone imports a themed deck as plain white
/// boxes on white, which is the difference an author notices before any other.
struct Design {
    layout: Option<(Node, HashMap<String, Relationship>)>,
    master: Option<(Node, HashMap<String, Relationship>)>,
}

impl Design {
    fn behind(package: &Package, slide_rels: &HashMap<String, Relationship>) -> Design {
        let mut design = Design {
            layout: None,
            master: None,
        };
        let Some(rel) = slide_rels.values().find(|r| r.kind == "slideLayout") else {
            return design;
        };
        let layout_rels = package.rels_for(&rel.target);
        if let Some(node) = package
            .xml_opt(&rel.target)
            .and_then(|doc| doc.child("sldLayout").cloned())
        {
            design.layout = Some((node, layout_rels.clone()));
        }
        if let Some(master_rel) = layout_rels.values().find(|r| r.kind == "slideMaster") {
            let master_rels = package.rels_for(&master_rel.target);
            if let Some(node) = package
                .xml_opt(&master_rel.target)
                .and_then(|doc| doc.child("sldMaster").cloned())
            {
                design.master = Some((node, master_rels));
            }
        }
        design
    }

    fn layout_tree(&self) -> Option<&Node> {
        self.layout.as_ref()?.0.path(&["cSld", "spTree"])
    }

    fn master_tree(&self) -> Option<&Node> {
        self.master.as_ref()?.0.path(&["cSld", "spTree"])
    }

    fn layout_rels(&self) -> Option<&HashMap<String, Relationship>> {
        self.layout.as_ref().map(|(_, rels)| rels)
    }

    fn master_rels(&self) -> Option<&HashMap<String, Relationship>> {
        self.master.as_ref().map(|(_, rels)| rels)
    }

    /// `p:hf`, from the layout if it has one, else the master's.
    fn header_footer(&self) -> Option<Node> {
        [&self.layout, &self.master]
            .into_iter()
            .flatten()
            .find_map(|(node, _)| node.child("hf").cloned())
    }

    /// The layout's background, or the master's.
    fn background(&self, theme: &Theme) -> Option<String> {
        for source in [&self.layout, &self.master] {
            if let Some(bg) = source.as_ref()?.0.path(&["cSld", "bg"]) {
                if let Some(color) = theme.background_of(bg) {
                    return Some(color);
                }
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rotation: f64,
    flip_h: bool,
    flip_v: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Geometry {
            x: 0.0,
            y: 0.0,
            w: 240.0,
            h: 120.0,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
        }
    }
}

pub struct Deck {
    pub slides: Vec<Slide>,
    pub assets: Vec<Asset>,
}

pub fn read(package: &Package, warnings: &mut Warnings) -> Result<Deck> {
    let presentation = package.xml("ppt/presentation.xml")?;
    let rels = package.rels_for("ppt/presentation.xml");
    let Some(root) = presentation.child("presentation") else {
        return Ok(Deck {
            slides: Vec::new(),
            assets: Vec::new(),
        });
    };

    let canvas = read_canvas(root);

    // The slide id list is the running order; the relationship gives the part.
    let mut parts: Vec<String> = Vec::new();
    if let Some(list) = root.child("sldIdLst") {
        for entry in list.children_named("sldId") {
            if let Some(rel) = entry.attr("id").and_then(|id| rels.get(id)) {
                parts.push(rel.target.clone());
            }
        }
    }
    if parts.is_empty() {
        // A file with no id list still has slides; fall back to a sorted scan.
        let mut found: Vec<String> = package
            .names()
            .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
            .map(str::to_string)
            .collect();
        found.sort_by_key(|n| slide_number(n));
        parts = found;
    }

    let mut assets: Vec<Asset> = Vec::new();
    let mut slides = Vec::new();

    for part in &parts {
        let Some(document) = package.xml_opt(part) else {
            continue;
        };
        let Some(sld) = document.child("sld") else {
            continue;
        };
        let slide_rels = package.rels_for(part);
        let theme = read_theme(package, &slide_rels);
        let placeholders = inherited_placeholders(package, &slide_rels, &theme);
        let design = Design::behind(package, &slide_rels);

        let mut ctx = SlideCtx {
            package,
            rels: &slide_rels,
            placeholders,
            theme,
            canvas,
            assets: &mut assets,
            warnings,
            blocks: Vec::new(),
            z: 1.0,
            title: None,
            decoration: false,
            number: slides.len() + 1,
            running: HashMap::new(),
            header_footer: design.header_footer(),
        };

        // The design comes first, so it sits behind the slide's own content —
        // which is where the banner, the rule and the logo were.
        //
        // `showMasterSp="0"` is how a slide says it does not want the master's
        // shapes, and a deck's one full-bleed image slide usually does say it.
        ctx.decoration = true;
        let master_shapes = sld.attr("showMasterSp").is_none_or(|v| v != "0");
        if master_shapes {
            if let (Some(tree), Some(rels)) = (design.master_tree(), design.master_rels()) {
                ctx.rels = rels;
                ctx.walk_tree(tree, Transform::identity());
            }
            if let (Some(tree), Some(rels)) = (design.layout_tree(), design.layout_rels()) {
                ctx.rels = rels;
                ctx.walk_tree(tree, Transform::identity());
            }
        }
        ctx.decoration = false;
        // Everything so far is the design's furniture — the banner, the rule,
        // the footer text. It must never name the slide.
        let decoration_blocks = ctx.blocks.len();
        ctx.rels = &slide_rels;

        if let Some(tree) = sld.path(&["cSld", "spTree"]) {
            ctx.walk_tree(tree, Transform::identity());
        }

        // Movement has no equivalent in a document that is not a slideshow
        // engine; saying so is better than a silent difference.
        if sld.child("transition").is_some() {
            ctx.warnings.note("화면 전환 효과는 넘어오지 않습니다");
        }
        if sld.child("timing").is_some_and(|t| !t.children.is_empty()) {
            ctx.warnings.note("애니메이션은 넘어오지 않습니다");
        }

        let notes = read_notes(package, &slide_rels);
        // A slide states its background only when it differs from the design's.
        // Reading the slide alone imports every themed deck on white.
        let background = sld
            .path(&["cSld", "bg"])
            .and_then(|bg| ctx.theme.background_of(bg))
            .or_else(|| design.background(&ctx.theme))
            .unwrap_or_else(|| canvas.bg.as_str().to_string());

        let mut blocks = std::mem::take(&mut ctx.blocks);
        // The design's furniture is marked, so an export that puts the slide
        // back on its original layout can leave those blocks to the master
        // rather than drawing the logo twice.
        for block in blocks.iter_mut().take(decoration_blocks) {
            block.style.insert("design".to_string(), json!(true));
        }
        let layout_part = slide_rels
            .values()
            .find(|r| r.kind == "slideLayout")
            .and_then(|r| r.target.strip_prefix("ppt/"))
            .map(str::to_string);
        // A slide with no title placeholder still needs a name, and the first
        // readable line *of its own content* is what a person would call it —
        // a themed layout's footer ("AI Studio · 대외비") named every slide
        // in the deck after itself when the whole list was scanned.
        let own = blocks
            .get(decoration_blocks.min(blocks.len())..)
            .unwrap_or(&[]);
        let title = ctx
            .title
            .clone()
            .unwrap_or_else(|| derive_title(own, slides.len() + 1));

        slides.push(Slide {
            id: new_slide_id(),
            title,
            layout_name: "blank".to_string(),
            notes,
            canvas: Canvas {
                w: canvas.w,
                h: canvas.h,
                bg: background.into(),
            },
            blocks,
            layout_part,
            master_shapes,
            file: None,
        });
    }

    if let Some(bundle) = design_bundle(package) {
        assets.push(Asset {
            name: TEMPLATE_ASSET.to_string(),
            bytes: bundle,
            source: "ppt/".to_string(),
        });
    }

    Ok(Deck { slides, assets })
}

/// The asset an imported deck keeps its Office design in — masters, layouts,
/// theme, notes master and the media they draw — so an export can hand the
/// slides back on the template they came from. A plain zip of the original
/// parts, not a `.pptx`: it has no slides and PowerPoint has no reason to open it.
pub const TEMPLATE_ASSET: &str = "office-template.zip";

/// Zip the design parts of a presentation, or `None` when it has no master.
fn design_bundle(package: &Package) -> Option<Vec<u8>> {
    let mut keep: Vec<String> = package
        .names()
        .filter(|n| {
            n.starts_with("ppt/slideMasters/")
                || n.starts_with("ppt/slideLayouts/")
                || n.starts_with("ppt/theme/")
                || n.starts_with("ppt/notesMasters/")
                || matches!(
                    *n,
                    "ppt/presProps.xml" | "ppt/viewProps.xml" | "ppt/tableStyles.xml"
                )
        })
        .map(str::to_string)
        .collect();
    if !keep
        .iter()
        .any(|n| n.starts_with("ppt/slideMasters/") && n.ends_with(".xml"))
    {
        return None;
    }
    // The pictures a master or layout draws (a logo, a background) live in
    // `ppt/media` beside the slides' own pictures; only the design's are kept.
    let mut media: Vec<String> = Vec::new();
    for part in keep.iter().filter(|n| !n.contains("/_rels/")) {
        for rel in package.rels_for(part).values() {
            if !rel.external && rel.target.starts_with("ppt/media/") && package.has(&rel.target) {
                media.push(rel.target.clone());
            }
        }
    }
    keep.extend(media);
    for meta in [
        "ppt/presentation.xml",
        "ppt/_rels/presentation.xml.rels",
        "[Content_Types].xml",
    ] {
        if package.has(meta) {
            keep.push(meta.to_string());
        }
    }
    keep.sort();
    keep.dedup();

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for name in &keep {
            let Some(bytes) = package.bytes(name) else {
                continue;
            };
            zip.start_file(name.clone(), options).ok()?;
            std::io::Write::write_all(&mut zip, bytes).ok()?;
        }
        zip.finish().ok()?;
    }
    Some(buffer.into_inner())
}

/// A name for a slide with no title placeholder.
///
/// The first readable line of prose, which is what a person would call it. A
/// table or a chart is skipped: naming a slide `| 항목 | 상반기 |` would put a
/// markdown row in the folder listing and in `AI.md`'s table of contents.
fn derive_title(blocks: &[SlideBlock], number: usize) -> String {
    for block in blocks {
        if matches!(block.kind, Kind::Table | Kind::Chart) {
            continue;
        }
        let label = ai_format::blocks::block_label(&block.md, 80);
        if !label.is_empty() {
            return label;
        }
    }
    // Nothing readable: name it by position, as PowerPoint's outline pane does.
    match blocks.iter().find(|b| b.kind == Kind::Table) {
        Some(_) => format!("슬라이드 {number} (표)"),
        None => match blocks.iter().find(|b| b.kind == Kind::Chart) {
            Some(_) => format!("슬라이드 {number} (차트)"),
            None => format!("슬라이드 {number}"),
        },
    }
}

/// A stroke width in px, to two decimals.
///
/// A 2pt outline is 2.6666666666666665px, and writing that into the JSON makes
/// the file noisy for a difference no screen can show.
fn round_px(emu: i64) -> f64 {
    (px(emu) * 100.0).round() / 100.0
}

fn slide_number(part: &str) -> usize {
    part.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(usize::MAX)
}

fn read_canvas(root: &Node) -> Canvas {
    let size = root.child("sldSz");
    let w = size
        .and_then(|s| s.attr_i64("cx"))
        .map(px)
        .unwrap_or(DEFAULT_CANVAS.w);
    let h = size
        .and_then(|s| s.attr_i64("cy"))
        .map(px)
        .unwrap_or(DEFAULT_CANVAS.h);
    Canvas {
        w: w.round(),
        h: h.round(),
        bg: DEFAULT_CANVAS.bg,
    }
}

fn read_notes(package: &Package, rels: &HashMap<String, Relationship>) -> String {
    let Some(rel) = rels.values().find(|r| r.kind == "notesSlide") else {
        return String::new();
    };
    let Some(document) = package.xml_opt(&rel.target) else {
        return String::new();
    };
    let Some(tree) = document.path(&["notes", "cSld", "spTree"]) else {
        return String::new();
    };

    // Only the body placeholder is the speaker's note; the slide-image and
    // slide-number placeholders are furniture.
    let mut out: Vec<String> = Vec::new();
    for sp in tree.descendants("sp") {
        let is_body = sp
            .path(&["nvSpPr", "nvPr", "ph"])
            .map(|ph| ph.attr("type") == Some("body"))
            .unwrap_or(false);
        if !is_body {
            continue;
        }
        if let Some(body) = sp.child("txBody") {
            let text = paragraphs_to_text(body);
            if !text.trim().is_empty() {
                out.push(text);
            }
        }
    }
    out.join("\n").trim().to_string()
}

/// The geometry of each placeholder on the slide's layout, then its master.
///
/// Real decks lean on this constantly: a title shape usually has no `xfrm` of
/// its own. Without the inheritance every such shape would land at 0,0 with a
/// default size, which is the difference between "looks like itself" and "looks
/// like a pile".
fn inherited_placeholders(
    package: &Package,
    slide_rels: &HashMap<String, Relationship>,
    theme: &Theme,
) -> Placeholders {
    let mut out = Placeholders::new();

    let Some(layout) = slide_rels.values().find(|r| r.kind == "slideLayout") else {
        return out;
    };

    // The master first, so the layout's own values win where both have one.
    let layout_rels = package.rels_for(&layout.target);
    if let Some(master) = layout_rels.values().find(|r| r.kind == "slideMaster") {
        if let Some(document) = package.xml_opt(&master.target) {
            if let Some(tree) = document.path(&["sldMaster", "cSld", "spTree"]) {
                collect_placeholders(tree, theme, &mut out);
            }
        }
    }
    if let Some(document) = package.xml_opt(&layout.target) {
        if let Some(tree) = document.path(&["sldLayout", "cSld", "spTree"]) {
            collect_placeholders(tree, theme, &mut out);
        }
    }
    out
}

fn collect_placeholders(tree: &Node, theme: &Theme, out: &mut Placeholders) {
    for sp in tree.descendants("sp") {
        let Some(ph) = sp.path(&["nvSpPr", "nvPr", "ph"]) else {
            continue;
        };
        let key = placeholder_key(ph);
        let entry = out.entry(key).or_default();
        if let Some(geometry) = read_geometry(sp) {
            entry.geometry = Some(geometry);
        }
        // The layout states `buNone` for a subtitle here; without reading it the
        // master's bullet would win and every subtitle would gain a bullet.
        if let Some(style) = sp.path(&["txBody", "lstStyle"]) {
            let levels = levels_of(style);
            if levels.iter().any(Option::is_some) {
                entry.bullets = levels;
            }
            // The layout is where a template says "this deck's titles are
            // 32pt", overriding the master for that one layout.
            let text = text_levels_of(style, theme);
            if text.iter().any(|d| !d.is_empty()) {
                entry.text = merge_levels(&text, &entry.text);
            }
        }
        // A layout placeholder can also state the size on its body properties
        // rather than per level; `normAutofit` there is read with the shape.
    }
}

/// Nearer levels over further ones, field by field.
fn merge_levels(nearer: &[TextDefaults], further: &[TextDefaults]) -> Vec<TextDefaults> {
    (0..9)
        .map(|level| {
            nearer
                .get(level)
                .cloned()
                .unwrap_or_default()
                .under(further.get(level).unwrap_or(&TextDefaults::default()))
        })
        .collect()
}

/// A placeholder is addressed by type and index, and both default.
fn placeholder_key(ph: &Node) -> (String, String) {
    (
        ph.attr("type").unwrap_or("body").to_string(),
        ph.attr("idx").unwrap_or("0").to_string(),
    )
}

/// What a placeholder inherits, by index and then by type alone.
///
/// A layout's footer is `idx="11"` while the master's is `idx="3"`: the index is
/// only meaningful among the body placeholders, and matching on it alone leaves
/// the footer, the date and the slide number with no geometry — which puts them
/// in the middle of the slide instead of along the bottom.
fn inherited_for<'a>(placeholders: &'a Placeholders, ph: &Node) -> Option<&'a Inherited> {
    let key = placeholder_key(ph);
    placeholders.get(&key).or_else(|| {
        placeholders
            .iter()
            .find(|((kind, _), _)| *kind == key.0)
            .map(|(_, entry)| entry)
    })
}

fn read_geometry(sp: &Node) -> Option<Geometry> {
    let xfrm = sp.path(&["spPr", "xfrm"]).or_else(|| sp.path(&["xfrm"]))?;
    let off = xfrm.child("off")?;
    let ext = xfrm.child("ext")?;
    Some(Geometry {
        // Whole pixels: the canvas is a pixel grid and the editor nudges by 8px,
        // so carrying 38.33 through would only produce fractional drift.
        x: px(off.attr_i64("x")?).round(),
        y: px(off.attr_i64("y")?).round(),
        w: px(ext.attr_i64("cx")?).round(),
        h: px(ext.attr_i64("cy")?).round(),
        rotation: xfrm
            .attr_i64("rot")
            .map(|r| r as f64 / 60_000.0)
            .unwrap_or(0.0),
        flip_h: xfrm.attr_bool("flipH"),
        flip_v: xfrm.attr_bool("flipV"),
    })
}

/// Names the image format when a browser cannot draw it, else `None`. PNG, JPG,
/// GIF, WebP, BMP and SVG all render; Windows metafiles and TIFF do not.
fn unrenderable_image_format(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "emf" | "emz" => Some("EMF"),
        "wmf" | "wmz" => Some("WMF"),
        "tif" | "tiff" => Some("TIFF"),
        _ => None,
    }
}

/// A `<a:srcRect>` crop as whole-percent edges, or `None` when nothing is
/// cropped. OOXML stores each edge as 1/1000 of a percent (`100000` = 100%) —
/// the fraction trimmed off that side of the source before it fills the box.
/// Negative values (an outset) are clamped to zero: this format has no way to
/// pad an image, and drawing it un-padded is closer than drawing it wrong.
fn read_crop(blip_fill: &Node) -> Option<Json> {
    let sr = blip_fill.child("srcRect")?;
    let pct = |name: &str| (sr.attr_i64(name).unwrap_or(0) as f64 / 1000.0).max(0.0);
    let (l, t, r, b) = (pct("l"), pct("t"), pct("r"), pct("b"));
    if l == 0.0 && t == 0.0 && r == 0.0 && b == 0.0 {
        return None;
    }
    Some(json!({ "l": l, "t": t, "r": r, "b": b }))
}

/// Rotation, flip and crop for an image block, written into the open-ended
/// `style` map — the same job `ShapeSpec` does for shapes, but images already
/// ride in `style` (`fit`, `radius`). Only non-default values are written, so
/// an ordinary upright, uncropped picture keeps an empty style and its saved
/// JSON stays clean.
fn image_style(geometry: &Geometry, blip_fill: Option<&Node>) -> IndexMap<String, Json> {
    let mut style = IndexMap::new();
    if geometry.rotation != 0.0 {
        style.insert("rotation".to_string(), json!(geometry.rotation));
    }
    if geometry.flip_h {
        style.insert("flipH".to_string(), json!(true));
    }
    if geometry.flip_v {
        style.insert("flipV".to_string(), json!(true));
    }
    if let Some(crop) = blip_fill.and_then(read_crop) {
        style.insert("crop".to_string(), crop);
    }
    style
}

/// A group's coordinate mapping, so nested shapes land where they are drawn.
///
/// PowerPoint groups declare a child coordinate space (`chOff`/`chExt`) that is
/// scaled onto the group's own box; ignoring it puts every grouped shape in the
/// wrong place and often off-slide.
#[derive(Clone, Copy)]
struct Transform {
    offset_x: f64,
    offset_y: f64,
    scale_x: f64,
    scale_y: f64,
}

impl Transform {
    fn identity() -> Transform {
        Transform {
            offset_x: 0.0,
            offset_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }

    fn apply(&self, g: Geometry) -> Geometry {
        Geometry {
            x: (self.offset_x + g.x * self.scale_x).round(),
            y: (self.offset_y + g.y * self.scale_y).round(),
            w: (g.w * self.scale_x).max(1.0).round(),
            h: (g.h * self.scale_y).max(1.0).round(),
            ..g
        }
    }

    /// A transform that just moves things, for a part whose shapes are laid out
    /// from its own origin — a SmartArt drawing, which knows nothing about where
    /// on the slide its frame sits.
    fn translated(x: f64, y: f64) -> Transform {
        Transform {
            offset_x: x,
            offset_y: y,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }

    /// Compose with a group's own transform.
    fn nest(&self, group: &Node) -> Transform {
        let Some(xfrm) = group.path(&["grpSpPr", "xfrm"]) else {
            return *self;
        };
        let (Some(off), Some(ext)) = (xfrm.child("off"), xfrm.child("ext")) else {
            return *self;
        };
        let gx = off.attr_i64("x").map(px).unwrap_or(0.0);
        let gy = off.attr_i64("y").map(px).unwrap_or(0.0);
        let gw = ext.attr_i64("cx").map(px).unwrap_or(0.0);
        let gh = ext.attr_i64("cy").map(px).unwrap_or(0.0);

        let ch_off = xfrm.child("chOff");
        let ch_ext = xfrm.child("chExt");
        let cx = ch_off.and_then(|o| o.attr_i64("x")).map(px).unwrap_or(0.0);
        let cy = ch_off.and_then(|o| o.attr_i64("y")).map(px).unwrap_or(0.0);
        let cw = ch_ext.and_then(|e| e.attr_i64("cx")).map(px).unwrap_or(gw);
        let ch = ch_ext.and_then(|e| e.attr_i64("cy")).map(px).unwrap_or(gh);

        let sx = if cw > 0.0 { gw / cw } else { 1.0 };
        let sy = if ch > 0.0 { gh / ch } else { 1.0 };

        // Map the child space onto the group box, then through our own transform.
        let outer = self.apply(Geometry {
            x: gx,
            y: gy,
            w: gw,
            h: gh,
            ..Geometry::default()
        });
        Transform {
            offset_x: outer.x - cx * sx * self.scale_x,
            offset_y: outer.y - cy * sy * self.scale_y,
            scale_x: sx * self.scale_x,
            scale_y: sy * self.scale_y,
        }
    }
}

struct SlideCtx<'a> {
    package: &'a Package,
    rels: &'a HashMap<String, Relationship>,
    placeholders: Placeholders,
    theme: Theme,
    canvas: Canvas,
    assets: &'a mut Vec<Asset>,
    warnings: &'a mut Warnings,
    blocks: Vec<SlideBlock>,
    z: f64,
    title: Option<String>,
    /// Set while walking a master or layout, where the placeholders are empty
    /// prompts ("제목을 입력하십시오") and only the design belongs to the slide.
    decoration: bool,
    /// This slide's number, for the slide-number placeholder.
    number: usize,
    /// The layout's or master's `p:hf`, which says whether the footer, date and
    /// slide-number fields are shown.
    header_footer: Option<Node>,
    /// Blocks already made from a footer, date or slide-number placeholder, by
    /// placeholder type. The layout's version replaces the master's, the way
    /// PowerPoint resolves them.
    running: HashMap<String, usize>,
}

impl SlideCtx<'_> {
    fn walk_tree(&mut self, tree: &Node, transform: Transform) {
        for child in &tree.children {
            match child.name.as_str() {
                "sp" => self.read_sp(child, transform),
                "cxnSp" => self.read_sp(child, transform),
                "pic" => self.read_pic(child, transform),
                "graphicFrame" => self.read_graphic_frame(child, transform),
                "grpSp" => {
                    let nested = transform.nest(child);
                    self.walk_tree(child, nested);
                }
                _ => {}
            }
        }
    }

    fn next_z(&mut self) -> f64 {
        let z = self.z;
        self.z += 1.0;
        z
    }

    /// Geometry from the shape, or inherited from its placeholder.
    ///
    /// When neither the slide, its layout nor the master says where a shape goes,
    /// a generous box inset from the canvas is a far better guess than a stub at
    /// the origin — the content is at least readable and movable.
    fn geometry_of(&self, sp: &Node, transform: Transform) -> Geometry {
        let own = read_geometry(sp);
        let inherited = sp
            .path(&["nvSpPr", "nvPr", "ph"])
            .and_then(|ph| inherited_for(&self.placeholders, ph))
            .and_then(|entry| entry.geometry);
        let fallback = || Geometry {
            x: (self.canvas.w * 0.075).round(),
            y: (self.canvas.h * 0.14).round(),
            w: (self.canvas.w * 0.85).round(),
            h: (self.canvas.h * 0.2).round(),
            ..Geometry::default()
        };
        // Inherited geometry is only trustworthy while the design and the slide
        // agree on the slide's size. A tool that changed `sldSz` without touching
        // the layout leaves placeholders laid out for the old size, and a title
        // 864px wide on a 745px slide hangs off the edge. The shape's own
        // geometry is never touched: an author who put something half off the
        // slide meant to.
        let inherited = inherited.map(|g| self.fitted(g));
        transform.apply(own.or(inherited).unwrap_or_else(fallback))
    }

    /// A box brought back onto the canvas, keeping its position where it can.
    fn fitted(&self, g: Geometry) -> Geometry {
        if g.x >= 0.0 && g.y >= 0.0 && g.x + g.w <= self.canvas.w && g.y + g.h <= self.canvas.h {
            return g;
        }
        let box_ = ai_format::geometry::clamp_box(
            ai_format::geometry::Box {
                x: g.x,
                y: g.y,
                w: g.w,
                h: g.h,
                z: 0.0,
            },
            &self.canvas,
        );
        Geometry {
            x: box_.x,
            y: box_.y,
            w: box_.w,
            h: box_.h,
            ..g
        }
    }

    /// Bullet per level for one shape, resolved the way Office resolves it: the
    /// shape's own `lstStyle`, then its placeholder's on the layout, then the
    /// master's style for that family.
    fn list_style(&self, sp: &Node, placeholder: Option<&Node>) -> ListStyle {
        let family = Family::of(placeholder);
        let own = sp
            .path(&["txBody", "lstStyle"])
            .map(levels_of)
            .unwrap_or_default();
        let inherited = placeholder
            .and_then(|ph| inherited_for(&self.placeholders, ph))
            .map(|entry| entry.bullets.clone())
            .unwrap_or_default();

        let levels = (0..9)
            .map(|level| {
                own.get(level)
                    .copied()
                    .flatten()
                    .or_else(|| inherited.get(level).copied().flatten())
                    .or_else(|| self.theme.bullet_for(family, level))
                    .flatten()
            })
            .collect();
        ListStyle { levels }
    }

    /// Whether the design says to show a footer, date or slide-number field.
    ///
    /// `p:hf` is how a template states it. Its absence is treated as "not
    /// stated" rather than "yes", because every default template omits it while
    /// carrying the placeholders empty.
    fn shows(&self, kind: &str) -> bool {
        let attribute = match kind {
            "dt" => "dt",
            "ftr" => "ftr",
            "sldNum" => "sldNum",
            _ => return false,
        };
        self.header_footer
            .as_ref()
            .and_then(|hf| hf.attr(attribute))
            .is_some_and(|value| matches!(value, "1" | "true"))
    }

    /// The text formatting behind a shape, level by level.
    ///
    /// Nearest first: the shape's own `lstStyle`, then the layout and master
    /// placeholder it fills, then the master's style for its family. Each level
    /// is merged field by field, so a layout that changes only the colour keeps
    /// the master's size.
    fn text_defaults(&self, sp: &Node, placeholder: Option<&Node>) -> Vec<TextDefaults> {
        let family = Family::of(placeholder);
        let own = sp
            .path(&["txBody", "lstStyle"])
            .map(|style| text_levels_of(style, &self.theme))
            .unwrap_or_default();
        let inherited = placeholder
            .and_then(|ph| inherited_for(&self.placeholders, ph))
            .map(|entry| entry.text.clone())
            .unwrap_or_default();
        let from_master: Vec<TextDefaults> = (0..9)
            .map(|level| self.theme.text_for(family, level))
            .collect();

        merge_levels(&merge_levels(&own, &inherited), &from_master)
    }

    fn read_sp(&mut self, sp: &Node, transform: Transform) {
        let geometry = self.geometry_of(sp, transform);
        let placeholder = sp.path(&["nvSpPr", "nvPr", "ph"]);
        let placeholder_type = placeholder
            .and_then(|ph| ph.attr("type"))
            .unwrap_or("")
            .to_string();
        // The footer, the date and the slide number are the one kind of
        // placeholder whose *content* lives on the layout rather than the slide.
        // A template puts the company name and the page number there, and they
        // appear on every slide — so they come across as text, not as prompts.
        let running = matches!(placeholder_type.as_str(), "dt" | "ftr" | "sldNum");
        if self.decoration && placeholder.is_some() && !running {
            return;
        }
        // A running placeholder on the layout is only drawn when the template
        // put something in it: text the author typed, or an explicit `p:hf`
        // saying to show the field. Every Office template ships these three
        // placeholders holding nothing but a field, and importing those would
        // stamp a stale date and a page number onto a deck that never showed
        // either. When the author does turn them on, PowerPoint copies the
        // placeholder onto each slide — and a slide's own shapes are never
        // skipped here.
        if self.decoration
            && running
            && !(authored(sp.child("txBody")) || self.shows(&placeholder_type))
        {
            return;
        }
        let list_style = self.list_style(sp, placeholder);
        let defaults = self.text_defaults(sp, placeholder);

        let body = sp.child("txBody");
        let markdown = body
            .map(|b| {
                let inline = Inline {
                    theme: Some(&self.theme),
                    color: dominant_run_color(b, &self.theme),
                    font: dominant_run_font(b),
                };
                paragraphs_to_markdown_in(b, self.rels, &list_style, &inline)
            })
            .unwrap_or_default();
        let text_style = body
            .map(|b| text_style(b, &defaults, &self.theme))
            .unwrap_or_default();
        let is_title = placeholder
            .and_then(|ph| ph.attr("type"))
            .is_some_and(|t| matches!(t, "title" | "ctrTitle"));
        if is_title && self.title.is_none() {
            let plain = ai_format::mdblocks::plain_text(&markdown);
            if !plain.is_empty() {
                self.title = Some(plain);
            }
        }

        let preset = sp
            .path(&["spPr", "prstGeom"])
            .and_then(|g| g.attr("prst"))
            .unwrap_or(if sp.name == "cxnSp" { "line" } else { "rect" })
            .to_string();
        let spec = self.shape_spec(sp, &preset, geometry);

        // A shape filled with a picture: the picture is what the slide showed, and
        // this format draws images. The shape's own outline is lost, which is far
        // less of the original than the photograph is.
        if let Some(blip_fill) = sp.path(&["spPr", "blipFill"]) {
            if markdown.trim().is_empty() && self.stash_blip(blip_fill, geometry).is_some() {
                return;
            }
        }

        // A plain rectangle with no fill and no outline is a text box, and
        // importing it as a shape would put a border round every caption.
        let decorative = spec.fill.is_some() || spec.line.is_some() || preset != "rect";
        let kind = if decorative { Kind::Shape } else { Kind::Text };
        if kind == Kind::Text && markdown.trim().is_empty() {
            return;
        }

        // The layout's cached text for a slide number is `‹#›`; the slide's own
        // number is what PowerPoint draws there.
        let markdown = if placeholder_type == "sldNum" {
            self.number.to_string()
        } else {
            markdown
        };

        let z = self.next_z();
        let block = SlideBlock {
            id: new_block_id(),
            kind,
            md: markdown,
            x: geometry.x,
            y: geometry.y,
            w: geometry.w,
            h: geometry.h,
            z,
            style: text_style,
            shape: (kind == Kind::Shape).then_some(spec),
            table: None,
            locked: false,
        };
        // One footer per slide: the layout's replaces the master's rather than
        // being drawn on top of it.
        if running {
            if let Some(at) = self.running.get(&placeholder_type).copied() {
                let z = self.blocks[at].z;
                self.blocks[at] = SlideBlock { z, ..block };
                return;
            }
            self.running.insert(placeholder_type, self.blocks.len());
        }
        self.blocks.push(block);
    }

    fn shape_spec(&mut self, sp: &Node, preset: &str, geometry: Geometry) -> ShapeSpec {
        let props = sp.child("spPr");

        // "Stated no fill" and "stated nothing" are different: only the second
        // falls through to the theme. Collapsing them repainted every
        // deliberately transparent shape with the accent colour.
        let states_no_fill = props.is_some_and(|p| p.child("noFill").is_some());
        let fill =
            props.and_then(|p| {
                if p.child("noFill").is_some() {
                    return None;
                }
                if let Some(solid) = p.child("solidFill") {
                    let opacity = solid
                        .descendants("alpha")
                        .first()
                        .and_then(|a| a.attr_i64("val"))
                        .map(|v| v as f64 / 1000.0)
                        .unwrap_or(100.0);
                    // Through the theme first: a `schemeClr tx1` with lumMod/
                    // lumOff is how PowerPoint writes most grey and tinted
                    // fills, and `solid_color` alone sees no colour in it. A
                    // label chip lost its dark fill that way and its white text
                    // vanished into the white slide.
                    return self
                        .theme
                        .color_of(solid)
                        .or_else(|| solid_color(solid))
                        .map(|color| Fill { color, opacity });
                }
                if let Some(node) = p.child("gradFill") {
                    // The average of the stops rather than the first one: a
                    // gradient reads as its middle, and taking the first stop
                    // makes every gradient shape too light or too dark.
                    self.warnings
                        .note("그라데이션 채우기는 중간 색의 단색으로 바꿨습니다");
                    let stops: Vec<String> = node
                        .descendants("gs")
                        .iter()
                        .filter_map(|stop| self.theme.color_of(stop))
                        .collect();
                    return average_color(&stops)
                        .or_else(|| self.theme.color_of(node))
                        .map(|color| Fill {
                            color,
                            opacity: 100.0,
                        });
                }
                for pattern in ["pattFill", "blipFill"] {
                    if let Some(node) = p.child(pattern) {
                        self.warnings.note(match pattern {
                            "pattFill" => "패턴 채우기는 단색으로 바꿨습니다",
                            // A picture-filled shape comes across as the picture
                            // itself; see `read_sp`.
                            _ => "그림으로 채운 도형은 그림으로 바꿨습니다",
                        });
                        return self.theme.color_of(node).or_else(|| solid_color(node)).map(
                            |color| Fill {
                                color,
                                opacity: 100.0,
                            },
                        );
                    }
                }
                None
            });

        // Nothing explicit: the shape is themed, which is how Office draws most
        // of them. `fillRef`/`lnRef` point into the theme's colour scheme.
        let styled = sp.child("style");
        let fill = fill.or_else(|| {
            if states_no_fill {
                return None;
            }
            let reference = styled?.child("fillRef")?;
            // `idx="0"` means no fill at all.
            if reference.attr("idx") == Some("0") {
                return None;
            }
            self.theme.color_of(reference).map(|color| Fill {
                color,
                opacity: 100.0,
            })
        });

        let explicit_line = props.and_then(|p| p.child("ln"));
        let line = explicit_line.and_then(|ln| {
            if ln.child("noFill").is_some() {
                return None;
            }
            let color = self.theme.color_of(ln).or_else(|| solid_color(ln))?;
            let width = ln.attr_i64("w").map(round_px).unwrap_or(1.0).max(0.5);
            let dash = ln
                .child("prstDash")
                .and_then(|d| d.attr("val"))
                .map(Dash::from_ooxml)
                .unwrap_or(Dash::Solid);
            Some(Line { color, width, dash })
        });
        // A themed outline, unless the shape stated one and said "none".
        let stated_none = explicit_line.is_some_and(|ln| ln.child("noFill").is_some());
        let line = line.or_else(|| {
            if stated_none {
                return None;
            }
            let reference = styled?.child("lnRef")?;
            if reference.attr("idx") == Some("0") {
                return None;
            }
            let width = explicit_line
                .and_then(|ln| ln.attr_i64("w"))
                .map(round_px)
                .unwrap_or(1.0)
                .max(0.5);
            let dash = explicit_line
                .and_then(|ln| ln.child("prstDash"))
                .and_then(|d| d.attr("val"))
                .map(Dash::from_ooxml)
                .unwrap_or(Dash::Solid);
            self.theme
                .color_of(reference)
                .map(|color| Line { color, width, dash })
        });

        let adjust: IndexMap<String, f64> = props
            .and_then(|p| p.path(&["prstGeom", "avLst"]))
            .map(|av| {
                av.children_named("gd")
                    .filter_map(|gd| {
                        let name = gd.attr("name")?.to_string();
                        // `fmla` is `val 16667`; only literal values apply here.
                        let value = gd.attr("fmla")?.strip_prefix("val ")?.trim().parse().ok()?;
                        Some((name, value))
                    })
                    .collect()
            })
            .unwrap_or_default();

        ShapeSpec {
            preset: preset.to_string(),
            fill,
            line,
            rotation: geometry.rotation,
            flip_h: geometry.flip_h,
            flip_v: geometry.flip_v,
            adjust,
        }
    }

    /// Store the picture a shape is filled with and place it as a block.
    ///
    /// Used for a shape whose fill is a picture: the picture is the block, and it
    /// carries the shape's rotation, flip and crop just like a `p:pic` would.
    fn stash_blip(&mut self, blip_fill: &Node, geometry: Geometry) -> Option<()> {
        let target = blip_fill
            .child("blip")
            .and_then(|b| b.attr("embed"))
            .and_then(|id| self.rels.get(id))
            .map(|rel| rel.target.clone())?;
        let name = self.stash_asset(&target)?;
        let z = self.next_z();
        self.blocks.push(SlideBlock {
            id: new_block_id(),
            kind: Kind::Image,
            md: format!("![이미지](../assets/{name})"),
            x: geometry.x,
            y: geometry.y,
            w: geometry.w,
            h: geometry.h,
            z,
            style: image_style(&geometry, Some(blip_fill)),
            shape: None,
            table: None,
            locked: false,
        });
        Some(())
    }

    fn read_pic(&mut self, pic: &Node, transform: Transform) {
        let geometry = transform.apply(read_geometry(pic).unwrap_or_default());
        let alt = pic
            .path(&["nvPicPr", "cNvPr"])
            .and_then(|c| c.attr("descr").or_else(|| c.attr("name")))
            .unwrap_or("이미지")
            .to_string();

        let blip_fill = pic.child("blipFill");
        let embed = blip_fill
            .and_then(|b| b.child("blip"))
            .and_then(|b| b.attr("embed"))
            .and_then(|id| self.rels.get(id));

        let markdown = match embed.and_then(|rel| self.stash_asset(&rel.target)) {
            Some(name) => format!("![{}](../assets/{name})", alt.replace(['[', ']'], "")),
            None => {
                self.warnings
                    .note("이미지를 찾을 수 없어 대체 텍스트로 대신했습니다");
                alt.clone()
            }
        };
        let kind = if markdown.starts_with("![") {
            Kind::Image
        } else {
            Kind::Text
        };

        let z = self.next_z();
        self.blocks.push(SlideBlock {
            id: new_block_id(),
            kind,
            md: markdown,
            x: geometry.x,
            y: geometry.y,
            w: geometry.w,
            h: geometry.h,
            z,
            style: if kind == Kind::Image {
                image_style(&geometry, blip_fill)
            } else {
                IndexMap::new()
            },
            shape: None,
            table: None,
            locked: false,
        });
    }

    /// Copy a media part out of the package, returning the name to reference.
    fn stash_asset(&mut self, part: &str) -> Option<String> {
        let bytes = self.package.bytes(part)?.to_vec();
        let base = part.rsplit('/').next().unwrap_or("image.png").to_string();
        if let Some(existing) = self.assets.iter().find(|a| a.source == part) {
            return Some(existing.name.clone());
        }
        // EMF/WMF (Windows metafiles, often a chart or clip-art pasted from
        // Office) and TIFF are stored verbatim but no browser draws them, so the
        // block would show a broken image. Copy the bytes anyway — an export can
        // hand them back untouched — but name the format so nobody hunts a blank.
        if let Some(fmt) = unrenderable_image_format(&base) {
            self.warnings.note(&format!(
                "일부 이미지가 {fmt} 형식이라 화면에 보이지 않을 수 있습니다 (PowerPoint에서 그림으로 붙여넣기 해 두면 보입니다)"
            ));
        }
        let name = crate::unique_asset_name(&base, self.assets);
        self.assets.push(Asset {
            name: name.clone(),
            bytes,
            source: part.to_string(),
        });
        Some(name)
    }

    fn read_graphic_frame(&mut self, frame: &Node, transform: Transform) {
        let geometry = transform.apply(
            frame
                .child("xfrm")
                .and_then(|_| read_geometry(frame))
                .unwrap_or_default(),
        );
        let Some(data) = frame.path(&["graphic", "graphicData"]) else {
            return;
        };

        if let Some(tbl) = data.child("tbl") {
            self.read_table(tbl, geometry);
            return;
        }
        if let Some(chart) = data.child("chart") {
            if let Some(imported) = chart
                .attr("id")
                .and_then(|id| self.rels.get(id))
                .and_then(|rel| read_chart(self.package, &rel.target))
            {
                if let Some(element) = &imported.substituted {
                    self.warnings.note(&format!(
                        "{} 차트는 꺾은선으로 바꿨습니다",
                        chart_type_label(element)
                    ));
                }
                let spec = imported.spec;
                let z = self.next_z();
                self.blocks.push(SlideBlock {
                    id: new_block_id(),
                    kind: Kind::Chart,
                    md: ai_format::chart::serialize_chart_block(&spec),
                    x: geometry.x,
                    y: geometry.y,
                    w: geometry.w,
                    h: geometry.h,
                    z,
                    style: IndexMap::new(),
                    shape: None,
                    table: None,
                    locked: false,
                });
                return;
            }
            self.warnings.note("차트를 읽을 수 없어 건너뛰었습니다");
            return;
        }
        // A diagram (SmartArt). PowerPoint stores the rendered shapes beside the
        // model, for readers that cannot lay a diagram out — which is exactly
        // this one. Those shapes are the closest thing to the original: the same
        // boxes, arrows, colours and words, as ordinary shapes.
        if let Some(ids) = data.child("relIds") {
            if self.read_diagram(ids, geometry) {
                self.warnings
                    .note("SmartArt는 같은 모양의 도형들로 바꿨습니다");
                return;
            }
        }
        // An embedded object — a spreadsheet, an equation, a PDF — is stored with
        // the picture Office draws in its place. That picture is what the slide
        // looked like, so it is what comes across.
        if let Some(picture) = data
            .child("oleObj")
            .or_else(|| data.child("AlternateContent"))
            .and_then(|node| node.descendants("pic").first().copied())
        {
            self.read_pic(picture, Transform::translated(geometry.x, geometry.y));
            self.warnings
                .note("OLE 개체는 Office가 그려 둔 그림으로 바꿨습니다");
            return;
        }
        // Name what it was: "가져오지 못한 개체" leaves the reader guessing which
        // of the things on their slide is missing.
        let uri = data.attr("uri").unwrap_or("");
        self.warnings.note(&format!(
            "{}는 넘어오지 않습니다",
            match uri.rsplit('/').next().unwrap_or("") {
                "diagram" => "SmartArt",
                "ink" => "잉크(펜) 필기",
                "slicer" | "slicers" => "슬라이서",
                "table" => "표",
                "chart" => "차트",
                "ole" | "oleObject" => "OLE 개체",
                "webextension" | "webextensions" => "웹 추가 기능",
                _ => "이 개체",
            }
        ));
    }

    /// A SmartArt diagram, as the shapes PowerPoint drew for it.
    ///
    /// Returns false when the file has no drawing part, which happens with files
    /// written by tools other than Office; the caller then falls back to the
    /// diagram's text.
    fn read_diagram(&mut self, ids: &Node, geometry: Geometry) -> bool {
        // The drawing is reached through the data part's own relationships.
        let Some(data_part) = ids.attr("dm").and_then(|id| self.rels.get(id)) else {
            return self.read_diagram_text(ids, geometry);
        };
        let data_rels = self.package.rels_for(&data_part.target);
        let drawing = data_rels
            .values()
            .find(|r| r.kind == "diagramDrawing")
            .and_then(|rel| self.package.xml_opt(&rel.target));

        // `dsp:sp` and `p:sp` differ only in namespace, and the parser has
        // already dropped those — so the ordinary shape reader applies.
        if let Some(tree) = drawing
            .as_ref()
            .and_then(|doc| doc.path(&["drawing", "spTree"]))
        {
            let before = self.blocks.len();
            self.walk_tree(tree, Transform::translated(geometry.x, geometry.y));
            if self.blocks.len() > before {
                return true;
            }
        }
        self.read_diagram_text(ids, geometry)
    }

    /// The words in a diagram, as one bulleted text block.
    ///
    /// A list of the diagram's labels is not the picture, but it is the content —
    /// and a reader who sees the words can tell what the diagram said.
    fn read_diagram_text(&mut self, ids: &Node, geometry: Geometry) -> bool {
        let Some(part) = ids.attr("dm").and_then(|id| self.rels.get(id)) else {
            return false;
        };
        let Some(document) = self.package.xml_opt(&part.target) else {
            return false;
        };
        let mut lines: Vec<String> = Vec::new();
        for point in document.descendants("pt") {
            // Only the nodes that hold text; the layout and style points do not.
            let Some(body) = point.child("t") else {
                continue;
            };
            let text = ai_format::mdblocks::plain_text(&paragraphs_to_markdown(
                body,
                self.rels,
                &ListStyle { levels: Vec::new() },
            ));
            if !text.trim().is_empty() && !lines.contains(&text) {
                lines.push(text);
            }
        }
        if lines.is_empty() {
            return false;
        }
        let md = lines
            .iter()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let z = self.next_z();
        self.blocks.push(SlideBlock {
            id: new_block_id(),
            kind: Kind::Text,
            md,
            x: geometry.x,
            y: geometry.y,
            w: geometry.w,
            h: geometry.h,
            z,
            style: IndexMap::new(),
            shape: None,
            table: None,
            locked: false,
        });
        true
    }

    fn read_table(&mut self, tbl: &Node, geometry: Geometry) {
        let rows: Vec<&Node> = tbl.children_named("tr").collect();
        if rows.is_empty() {
            return;
        }

        let mut cells: Vec<Vec<String>> = Vec::with_capacity(rows.len());
        let mut spec = TableSpec {
            cols: tbl
                .child("tblGrid")
                .map(|g| {
                    g.children_named("gridCol")
                        .map(|c| c.attr_i64("w").map(px).unwrap_or(0.0))
                        .collect()
                })
                .unwrap_or_default(),
            rows: rows
                .iter()
                .map(|r| r.attr_i64("h").map(px).unwrap_or(0.0))
                .collect(),
            merges: Vec::new(),
            header_row: tbl
                .child("tblPr")
                .map(|p| p.attr_bool("firstRow"))
                .unwrap_or(false),
            banded_rows: tbl
                .child("tblPr")
                .map(|p| p.attr_bool("bandRow"))
                .unwrap_or(false),
            first_col: tbl
                .child("tblPr")
                .map(|p| p.attr_bool("firstCol"))
                .unwrap_or(false),
            style: ai_format::table::TableStyle::Plain,
            cells: IndexMap::new(),
        };

        for (r, row) in rows.iter().enumerate() {
            let mut line: Vec<String> = Vec::new();
            for (c, tc) in row.children_named("tc").enumerate() {
                // A merge anchor declares its span; the cells it swallowed carry
                // the merge flags and no text.
                let col_span = tc.attr_i64("gridSpan").unwrap_or(1).max(1) as usize;
                let row_span = tc.attr_i64("rowSpan").unwrap_or(1).max(1) as usize;
                if col_span > 1 || row_span > 1 {
                    spec.merges.push(format!(
                        "{}:{}",
                        ai_formula::refs::to_ref(c, r),
                        ai_formula::refs::to_ref(c + col_span - 1, r + row_span - 1)
                    ));
                }

                let covered = tc.attr_bool("hMerge") || tc.attr_bool("vMerge");
                // A header row and a first column are bold by definition here,
                // so their `b="1"` is not emphasis to carry into the markdown —
                // doing so would leave `**항목**` in every header cell.
                let implied_bold = (spec.header_row && r == 0) || (spec.first_col && c == 0);
                let text = if covered {
                    String::new()
                } else {
                    tc.child("txBody")
                        .map(|b| {
                            paragraphs_to_markdown_with(
                                b,
                                self.rels,
                                &ListStyle::default(),
                                implied_bold,
                            )
                        })
                        .unwrap_or_default()
                };
                line.push(text.replace('\n', " ").trim().to_string());

                let reference = ai_formula::refs::to_ref(c, r);
                let format = cell_format(tc);
                if !format.is_empty() {
                    spec.cells.insert(reference, format);
                }
            }
            cells.push(line);
        }

        let md = to_markdown_table(&cells, &spec);
        apply_markdown_authority(&mut spec, &md);

        let z = self.next_z();
        self.blocks.push(SlideBlock {
            id: new_block_id(),
            kind: Kind::Table,
            md,
            x: geometry.x,
            y: geometry.y,
            w: geometry.w.max(120.0),
            h: geometry.h.max(60.0),
            z,
            style: IndexMap::new(),
            shape: None,
            table: Some(spec),
            locked: false,
        });
    }
}

fn cell_format(tc: &Node) -> CellFormat {
    let props = tc.child("tcPr");
    let valign = props.and_then(|p| p.attr("anchor")).and_then(|a| {
        Some(match a {
            "t" => "top",
            "ctr" => "middle",
            "b" => "bottom",
            _ => return None,
        })
    });
    let align = tc
        .child("txBody")
        .and_then(|b| b.children_named("p").next())
        .and_then(|p| p.child("pPr"))
        .and_then(|pr| pr.attr("algn"))
        .and_then(|a| {
            Some(match a {
                "l" => "left",
                "ctr" => "center",
                "r" => "right",
                _ => return None,
            })
        });
    let fill = props
        .filter(|p| p.child("noFill").is_none())
        .and_then(|p| p.child("solidFill"))
        .and_then(solid_color);

    CellFormat {
        align: align.map(str::to_string),
        valign: valign.map(str::to_string),
        fill,
        color: None,
    }
}

/* ---------------------------------------------------------------------- text */

/// A text body as markdown: one line per paragraph, bullets as list items.
/// The bullet in effect at each outline level, already resolved.
#[derive(Default)]
struct ListStyle {
    levels: Vec<Option<BulletStyle>>,
}

impl ListStyle {
    fn at(&self, level: usize) -> Option<BulletStyle> {
        self.levels.get(level).copied().flatten()
    }
}

fn paragraphs_to_markdown(
    body: &Node,
    rels: &HashMap<String, Relationship>,
    list_style: &ListStyle,
) -> String {
    paragraphs_to_markdown_with(body, rels, list_style, false)
}

/// What a run's own colour and family are measured against: the block's. Only
/// a run that differs from the rest of its block gets an inline `<span style>`,
/// so a sub-heading in burgundy over grey body text keeps its colour while a
/// single-colour box carries no markup at all.
struct Inline<'a> {
    theme: Option<&'a Theme>,
    color: Option<String>,
    font: Option<String>,
}

impl Inline<'_> {
    fn none() -> Inline<'static> {
        Inline {
            theme: None,
            color: None,
            font: None,
        }
    }
}

/// A text body's markdown with runs that differ from the block marked inline.
fn paragraphs_to_markdown_in(
    body: &Node,
    rels: &HashMap<String, Relationship>,
    list_style: &ListStyle,
    inline: &Inline<'_>,
) -> String {
    paragraphs_to_markdown_inline(body, rels, list_style, false, inline)
}

/// The same, with `ignore_bold` for text whose weight is already implied by its
/// place — a table's header row, or its first column.
fn paragraphs_to_markdown_with(
    body: &Node,
    rels: &HashMap<String, Relationship>,
    list_style: &ListStyle,
    ignore_bold: bool,
) -> String {
    paragraphs_to_markdown_inline(body, rels, list_style, ignore_bold, &Inline::none())
}

fn paragraphs_to_markdown_inline(
    body: &Node,
    rels: &HashMap<String, Relationship>,
    list_style: &ListStyle,
    ignore_bold: bool,
    inline: &Inline<'_>,
) -> String {
    let lines: Vec<String> = body
        .children_named("p")
        .map(|p| {
            let text = runs_to_markdown_inline(p, rels, ignore_bold, inline);
            match bullet_of(p, list_style) {
                None => text,
                Some(Bullet::Unordered(level)) => {
                    format!("{}- {text}", "  ".repeat(level))
                }
                Some(Bullet::Ordered(level)) => {
                    format!("{}1. {text}", "  ".repeat(level))
                }
            }
        })
        .collect();
    // Trailing empty paragraphs are PowerPoint's placeholder padding.
    let mut lines = lines;
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// The same, without list markers — for speaker notes.
fn paragraphs_to_text(body: &Node) -> String {
    body.children_named("p")
        .map(|p| {
            p.descendants("t")
                .iter()
                .map(|t| t.all_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

enum Bullet {
    Unordered(usize),
    Ordered(usize),
}

/// Whether a paragraph is a list item, and at what depth.
///
/// The paragraph's own `buChar`/`buNone` wins. Failing that a body placeholder
/// inherits the master's list style, which is where Office's familiar bullets
/// come from — the paragraphs themselves usually say nothing at all.
fn bullet_of(p: &Node, list_style: &ListStyle) -> Option<Bullet> {
    let props = p.child("pPr");
    let level = props
        .and_then(|pr| pr.attr_i64("lvl"))
        .unwrap_or(0)
        .clamp(0, 8) as usize;

    if let Some(props) = props {
        if props.child("buNone").is_some() {
            return None;
        }
        if props.child("buAutoNum").is_some() {
            return Some(Bullet::Ordered(level));
        }
        if props.child("buChar").is_some() {
            return Some(Bullet::Unordered(level));
        }
    }

    match list_style.at(level) {
        Some(BulletStyle::AutoNum) => Some(Bullet::Ordered(level)),
        Some(BulletStyle::Char) => Some(Bullet::Unordered(level)),
        None => None,
    }
}

fn runs_to_markdown_inline(
    p: &Node,
    rels: &HashMap<String, Relationship>,
    ignore_bold: bool,
    inline: &Inline<'_>,
) -> String {
    let mut out = String::new();
    for child in &p.children {
        match child.name.as_str() {
            "r" => out.push_str(&run_to_markdown(child, rels, ignore_bold, inline)),
            // A line break inside a paragraph (Shift+Enter). It is a new line
            // on the slide, so it is a new line here; reading it as a space
            // ran a text box's lines together on every re-open.
            "br" => out.push('\n'),
            // A field is a computed value; its cached text is what was shown.
            "fld" => out.push_str(
                &child
                    .descendants("t")
                    .iter()
                    .map(|t| t.all_text())
                    .collect::<String>(),
            ),
            _ => {}
        }
    }
    out.trim_end().to_string()
}

fn run_to_markdown(
    r: &Node,
    rels: &HashMap<String, Relationship>,
    ignore_bold: bool,
    inline: &Inline<'_>,
) -> String {
    let text = r
        .descendants("t")
        .iter()
        .map(|t| t.all_text())
        .collect::<String>();
    if text.is_empty() {
        return text;
    }
    let props = r.child("rPr");
    let bold = !ignore_bold && props.map(|p| p.attr_bool("b")).unwrap_or(false);
    let italic = props.map(|p| p.attr_bool("i")).unwrap_or(false);
    let strike = props
        .and_then(|p| p.attr("strike"))
        .is_some_and(|s| s != "noStrike");

    // Emphasis wraps the trimmed text; the surrounding spaces stay outside, or
    // markdown will not recognise the marker.
    let leading: String = text.chars().take_while(|c| c.is_whitespace()).collect();
    let trailing: String = text
        .chars()
        .rev()
        .take_while(|c| c.is_whitespace())
        .collect();
    let core = text.trim();
    if core.is_empty() {
        return text;
    }

    let mut wrapped = core.to_string();
    if strike {
        wrapped = format!("~~{wrapped}~~");
    }
    match (bold, italic) {
        (true, true) => wrapped = format!("***{wrapped}***"),
        (true, false) => wrapped = format!("**{wrapped}**"),
        (false, true) => wrapped = format!("*{wrapped}*"),
        (false, false) => {}
    }

    // A colour or family this run has and its block does not. Colour is
    // block-wide in this format, so the odd run out is marked inline — the
    // renderer draws the span and both exporters read it back into the run.
    if let Some(theme) = inline.theme {
        let own_color = props
            .and_then(|p| p.child("solidFill"))
            .and_then(|f| theme.color_of(f).or_else(|| solid_color(f)))
            .filter(|c| inline.color.as_deref() != Some(c.as_str()));
        let own_font = props
            .and_then(|p| {
                ["ea", "latin"]
                    .iter()
                    .filter_map(|tag| p.child(tag))
                    .filter_map(|n| n.attr("typeface"))
                    .map(str::trim)
                    .find(|f| ai_format::font::is_substitution(f))
            })
            .filter(|f| !f.contains(['"', '<', '>', ';']))
            .map(str::to_string)
            .filter(|f| inline.font.as_deref() != Some(f.as_str()));
        let mut css: Vec<String> = Vec::new();
        if let Some(c) = own_color {
            css.push(format!("color:{c}"));
        }
        if let Some(f) = own_font {
            css.push(format!("font-family:{f}"));
        }
        if !css.is_empty() {
            wrapped = format!("<span style=\"{}\">{wrapped}</span>", css.join(";"));
        }
    }

    // A hyperlink's target is in the slide's relationships, not in the run.
    if let Some(url) = r
        .path(&["rPr", "hlinkClick"])
        .and_then(|h| h.attr("id"))
        .and_then(|id| rels.get(id))
        .filter(|rel| rel.external && !rel.target.is_empty())
        .map(|rel| rel.target.clone())
    {
        let safe = url.replace([' ', ')'], "");
        wrapped = format!("[{wrapped}]({safe})");
    }

    format!("{leading}{wrapped}{trailing}")
}

/// The block style a text body implies: size, weight, colour, alignment.
///
/// Resolved the way PowerPoint resolves it — run, then paragraph, then the
/// inherited defaults from the layout and master — because a real deck states
/// almost none of this on the run itself.
fn text_style(
    body: &Node,
    defaults: &[TextDefaults],
    theme: &Theme,
) -> IndexMap<String, serde_json::Value> {
    let mut style = IndexMap::new();

    let first_paragraph = body.children_named("p").next();
    let paragraph_props = first_paragraph.and_then(|p| p.child("pPr"));
    let level = paragraph_props
        .and_then(|pr| pr.attr_i64("lvl"))
        .unwrap_or(0)
        .clamp(0, 8) as usize;
    let inherited = defaults.get(level).cloned().unwrap_or_default();

    // A paragraph's own `pPr` outranks anything inherited, and the first run
    // outranks the paragraph.
    let from_paragraph = paragraph_props
        .map(|pr| defaults_from_props(pr, theme))
        .unwrap_or_default();
    let first_run = first_paragraph.and_then(|p| p.children_named("r").next());
    let props = first_run.and_then(|r| r.child("rPr"));
    let run_bold = flag(props, "b");
    let from_run = TextDefaults {
        size: props
            .and_then(|p| p.attr_i64("sz"))
            .map(|sz| ai_format::font::px_for_pt(sz as f64 / 100.0)),
        bold: run_bold,
        italic: flag(props, "i"),
        // Colour is block-wide in this format, so take the colour most of the
        // text is drawn in rather than the first run's. A text box whose first
        // line is a burgundy sub-heading and the rest body grey arrived all
        // burgundy the other way.
        color: dominant_run_color(body, theme),
        align: None,
        line: None,
    };
    let resolved = from_run.under(&from_paragraph).under(&inherited);

    // PowerPoint shrinks text that does not fit and records what it did. Without
    // the scale a shape whose text was fitted at 62% arrives overflowing its
    // box, which is exactly the case where the author was already out of room.
    let autofit = body.path(&["bodyPr", "normAutofit"]);
    let scale = autofit
        .and_then(|n| n.attr_i64("fontScale"))
        .map(|v| v as f64 / 100_000.0)
        .unwrap_or(1.0);
    let reduction = autofit
        .and_then(|n| n.attr_i64("lnSpcReduction"))
        .map(|v| v as f64 / 100_000.0)
        .unwrap_or(0.0);

    if let Some(size) = resolved.size {
        style.insert("fontSize".to_string(), json!((size * scale).round()));
    }
    // Weight is block-wide in this format, so it may only be promoted when
    // the boldness really is block-wide: inherited from the layout/master, or
    // explicit on every run. Promoting the *first* run's `b="1"` made "**매출
    // 142억** — 전년 대비 +24%" arrive with the plain half bold too.
    let block_bold = match run_bold {
        Some(true) => {
            let paragraph_bold = from_paragraph.bold.or(inherited.bold);
            let mut any_text = false;
            let mut all_bold = true;
            for paragraph in body.children_named("p") {
                let p_bold = paragraph
                    .child("pPr")
                    .map(|pr| defaults_from_props(pr, theme).bold)
                    .unwrap_or(None)
                    .or(paragraph_bold);
                for run in paragraph.children_named("r") {
                    any_text = true;
                    if !flag(run.child("rPr"), "b").or(p_bold).unwrap_or(false) {
                        all_bold = false;
                    }
                }
            }
            any_text && all_bold
        }
        Some(false) => false,
        // Nothing on the runs: the paragraph or the master said it, and that
        // is block-wide by nature — the only carrier, since no run wrote `**`.
        None => resolved.bold == Some(true),
    };
    if block_bold {
        style.insert("weight".to_string(), json!(700));
    }
    if resolved.italic == Some(true) {
        style.insert("italic".to_string(), json!(true));
    }
    if let Some(color) = resolved.color {
        style.insert("color".to_string(), json!(color));
    }
    if let Some(font) = dominant_run_font(body) {
        style.insert("font".to_string(), json!(font));
    }
    if let Some(align) = resolved.align {
        style.insert("align".to_string(), json!(align));
    }
    if let Some(line) = resolved.line.map(|l| l * (1.0 - reduction)) {
        style.insert(
            "lineHeight".to_string(),
            json!((line * 100.0).round() / 100.0),
        );
    }
    if let Some(anchor) = body.child("bodyPr").and_then(|b| b.attr("anchor")) {
        let mapped = match anchor {
            "ctr" => Some("middle"),
            "b" => Some("bottom"),
            "t" => Some("top"),
            _ => None,
        };
        if let Some(mapped) = mapped {
            style.insert("valign".to_string(), json!(mapped));
        }
    }
    style
}

/// The explicit run colour covering the most non-blank characters in a text
/// body, resolved through the theme, or `None` when no run states one.
fn dominant_run_color(body: &Node, theme: &Theme) -> Option<String> {
    let mut weights: IndexMap<String, usize> = IndexMap::new();
    for paragraph in body.children_named("p") {
        for run in paragraph.children_named("r") {
            let Some(fill) = run.child("rPr").and_then(|p| p.child("solidFill")) else {
                continue;
            };
            let Some(color) = theme.color_of(fill).or_else(|| solid_color(fill)) else {
                continue;
            };
            let chars = run
                .child("t")
                .map(|t| t.all_text().chars().filter(|c| !c.is_whitespace()).count())
                .unwrap_or(0);
            *weights.entry(color).or_insert(0) += chars.max(1);
        }
    }
    // The first colour wins a tie, so a single-colour box is unaffected.
    let mut best: Option<(String, usize)> = None;
    for (color, weight) in weights {
        if best.as_ref().is_none_or(|(_, w)| weight > *w) {
            best = Some((color, weight));
        }
    }
    best.map(|(color, _)| color)
}

/// The Latin/East-Asian family covering the most characters in a text body,
/// when it is a real family and not the one this format draws. Theme
/// references (`+mn-lt`) are resolved by PowerPoint, so they are skipped.
fn dominant_run_font(body: &Node) -> Option<String> {
    let mut weights: IndexMap<String, usize> = IndexMap::new();
    for paragraph in body.children_named("p") {
        for run in paragraph.children_named("r") {
            let Some(props) = run.child("rPr") else {
                continue;
            };
            let Some(face) = ["ea", "latin"]
                .iter()
                .filter_map(|tag| props.child(tag))
                .filter_map(|n| n.attr("typeface"))
                .map(str::trim)
                .find(|f| ai_format::font::is_substitution(f))
            else {
                continue;
            };
            let chars = run
                .child("t")
                .map(|t| t.all_text().chars().filter(|c| !c.is_whitespace()).count())
                .unwrap_or(0);
            *weights.entry(face.to_string()).or_insert(0) += chars.max(1);
        }
    }
    let mut best: Option<(String, usize)> = None;
    for (face, weight) in weights {
        if best.as_ref().is_none_or(|(_, w)| weight > *w) {
            best = Some((face, weight));
        }
    }
    best.map(|(face, _)| face)
}

/* --------------------------------------------------------------------- chart */

/// A chart part into a chart spec.
///
/// The cached categories and values are what the chart was drawing, so the
/// numbers come across even though the source workbook does not.
/// A chart part read into a spec, with the plot type it had to become when this
/// format cannot draw the original.
pub(crate) struct ImportedChart {
    pub spec: ChartSpec,
    /// The OOXML element name of the plot, when it was not one of the five this
    /// format draws exactly.
    pub substituted: Option<String>,
}

/// What a chart type this format cannot draw was turned into, in words.
///
/// Named rather than silent: "방사형 차트는 꺾은선으로 바꿨습니다" tells a reader
/// what to look at, where a missing chart tells them nothing.
pub(crate) fn chart_type_label(element: &str) -> &'static str {
    match element {
        "scatterChart" => "분산형",
        "bubbleChart" => "거품형",
        "radarChart" => "방사형",
        "stockChart" => "주식형",
        "surfaceChart" | "surface3DChart" => "표면형",
        "bar3DChart" => "3차원 막대",
        "line3DChart" => "3차원 꺾은선",
        "pie3DChart" => "3차원 원형",
        "area3DChart" => "3차원 영역",
        "ofPieChart" => "원형 대 원형",
        _ => "이 차트",
    }
}

pub(crate) fn read_chart(package: &Package, part: &str) -> Option<ImportedChart> {
    let document = package.xml_opt(part)?;
    let space = document.child("chartSpace")?;
    let chart = space.child("chart")?;
    let plot = chart.child("plotArea")?;

    // The plot in document order, so a combination chart comes in as its first
    // plot rather than as whichever type this list happens to mention first.
    //
    // A type this format cannot draw becomes the nearest one it can: a scatter
    // or radar plot is a line, a 3-D column is a column. The alternative is
    // dropping the chart, and a line where a radar was is far closer to the
    // original than an empty rectangle.
    let (chart_type, plot_node) = plot
        .children
        .iter()
        .find(|node| node.name.ends_with("Chart"))
        .map(|node| (node.name.as_str(), node))?;

    let stacked = plot_node
        .child("grouping")
        .and_then(|g| g.attr("val"))
        .is_some_and(|v| v == "stacked" || v == "percentStacked");

    let kind = match chart_type {
        "barChart" | "bar3DChart" => {
            let horizontal = plot_node
                .child("barDir")
                .and_then(|d| d.attr("val"))
                .is_some_and(|v| v == "bar");
            if horizontal {
                "bar"
            } else {
                "column"
            }
        }
        "areaChart" | "area3DChart" => "area",
        "pieChart" | "pie3DChart" | "ofPieChart" => "pie",
        "doughnutChart" => "donut",
        // Everything else — line, scatter, bubble, radar, stock, surface — plots
        // a value per category, which a line chart shows.
        _ => "line",
    };
    let exact = matches!(
        chart_type,
        "barChart" | "lineChart" | "areaChart" | "pieChart" | "doughnutChart"
    );
    let substituted = (!exact).then_some(chart_type);

    let mut labels: Vec<String> = Vec::new();
    let mut series: Vec<Series> = Vec::new();

    for ser in plot_node.children_named("ser") {
        let name = ser
            .child("tx")
            .map(|tx| {
                tx.descendants("v")
                    .iter()
                    .map(|v| v.all_text())
                    .collect::<String>()
            })
            .unwrap_or_default();

        if labels.is_empty() {
            if let Some(cat) = ser.child("cat") {
                labels = cached_points(cat);
            }
        }
        // A scatter or bubble series names its values `yVal`, and a scatter's
        // `xVal` is what a category axis would have shown.
        if labels.is_empty() {
            if let Some(x) = ser.child("xVal") {
                labels = cached_points(x);
            }
        }
        let values: Vec<Option<f64>> = ["val", "yVal", "bubbleSize"]
            .iter()
            .find_map(|key| ser.child(key))
            .map(|val| {
                cached_points(val)
                    .into_iter()
                    .map(|v| v.trim().parse::<f64>().ok().filter(|n| n.is_finite()))
                    .collect()
            })
            .unwrap_or_default();

        series.push(Series {
            name: name.trim().to_string(),
            values,
        });
    }

    if series.is_empty() {
        return None;
    }

    let title = chart
        .child("title")
        .filter(|_| {
            !chart
                .child("autoTitleDeleted")
                .map(|d| d.attr_bool("val"))
                .unwrap_or(false)
        })
        .map(|t| {
            t.descendants("t")
                .iter()
                .map(|n| n.all_text())
                .collect::<String>()
        })
        .unwrap_or_default();

    let legend = chart.child("legend").is_some();

    Some(ImportedChart {
        spec: ai_format::chart::normalize_spec(&json!({
            "type": kind,
            "title": title.trim(),
            "labels": labels,
            "series": series.iter().map(|s| json!({ "name": s.name, "values": s.values })).collect::<Vec<_>>(),
            "options": { "stacked": stacked, "legend": legend },
        })),
        substituted: substituted.map(str::to_string),
    })
}

/// The `pt` values of a cached reference, in index order.
fn cached_points(node: &Node) -> Vec<String> {
    let mut points: Vec<(usize, String)> = Vec::new();
    for cache in ["strCache", "numCache"] {
        for found in node.descendants(cache) {
            for pt in found.children_named("pt") {
                let index = pt.attr_i64("idx").unwrap_or(0).max(0) as usize;
                let value = pt.child("v").map(|v| v.all_text()).unwrap_or_default();
                points.push((index, value));
            }
        }
    }
    if points.is_empty() {
        return Vec::new();
    }
    let length = points.iter().map(|(i, _)| *i).max().unwrap_or(0) + 1;
    let mut out = vec![String::new(); length];
    for (index, value) in points {
        out[index] = value;
    }
    out
}
