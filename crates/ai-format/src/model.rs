//! The document model shared by the editors, the API and the exporters.
//!
//! Field names are the ones the JSON API already used, so the React editors
//! need no translation layer.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use ai_formula::evaluate::{Cell, Names};

use crate::blocks::Kind;

/// Deserialize a field, treating an explicit `null` the same as a missing key.
///
/// `#[serde(default)]` only fires when the key is absent; a hand-edited or
/// third-party file that writes `"style": null` would otherwise fail the whole
/// load. Office files in the wild do this, so we tolerate it.
fn null_to_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
use crate::chart::{CellSource, ChartSpec};
use crate::geometry::{Box, Canvas};
use crate::mdblocks::BlockType;
use crate::shape::ShapeSpec;
use crate::table::TableSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Deck,
    Doc,
    Grid,
}

pub const PROJECT_TYPES: [ProjectType; 3] =
    [ProjectType::Deck, ProjectType::Doc, ProjectType::Grid];

impl ProjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectType::Deck => "deck",
            ProjectType::Doc => "doc",
            ProjectType::Grid => "grid",
        }
    }

    pub fn from_name(s: &str) -> Option<ProjectType> {
        Some(match s {
            "deck" => ProjectType::Deck,
            "doc" => ProjectType::Doc,
            "grid" => ProjectType::Grid,
            _ => return None,
        })
    }

    /// The folder extension, e.g. `.aideck`.
    pub fn ext(self) -> &'static str {
        match self {
            ProjectType::Deck => ".aideck",
            ProjectType::Doc => ".aidoc",
            ProjectType::Grid => ".aigrid",
        }
    }

    /// The subdirectory items live in.
    pub fn dir(self) -> &'static str {
        match self {
            ProjectType::Deck => "slides",
            ProjectType::Doc => "content",
            ProjectType::Grid => "sheets",
        }
    }

    /// The manifest key listing items.
    pub fn key(self) -> &'static str {
        match self {
            ProjectType::Deck => "slides",
            ProjectType::Doc => "sections",
            ProjectType::Grid => "sheets",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProjectType::Deck => "프레젠테이션",
            ProjectType::Doc => "문서",
            ProjectType::Grid => "스프레드시트",
        }
    }

    pub fn format_id(self) -> &'static str {
        match self {
            ProjectType::Deck => "ai-studio/deck",
            ProjectType::Doc => "ai-studio/doc",
            ProjectType::Grid => "ai-studio/grid",
        }
    }

    /// The sidecar JSON suffix that pairs with a `.md`.
    pub fn json_suffix(self) -> &'static str {
        match self {
            ProjectType::Deck => ".layout.json",
            ProjectType::Doc => ".meta.json",
            ProjectType::Grid => ".cells.json",
        }
    }

    pub fn from_folder(name: &str) -> Option<ProjectType> {
        PROJECT_TYPES.into_iter().find(|t| name.ends_with(t.ext()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub accent: String,
    pub font: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "aurora".into(),
            accent: "#4f46e5".into(),
            font: crate::font::FAMILY.into(),
        }
    }
}

/// One item's file pair, as recorded in the manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub id: String,
    pub name: String,
    pub md: String,
    pub json: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub id: String,
    pub title: String,
    pub created: String,
    pub modified: String,
    #[serde(default)]
    pub theme: Theme,
    /// The item list, written under `slides`/`sections`/`sheets`.
    #[serde(skip)]
    pub entries: Vec<ManifestEntry>,
}

/* -------------------------------------------------------------------- deck */

pub const SLIDE_LAYOUTS: [&str; 5] = ["title", "title-content", "two-column", "section", "blank"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlideBlock {
    pub id: String,
    #[serde(with = "kind_serde")]
    pub kind: Kind,
    pub md: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub z: f64,
    /// Text formatting: font size, weight, alignment, colour. Open-ended on
    /// purpose — the editors add keys here as they grow.
    #[serde(default, deserialize_with = "null_to_default")]
    pub style: IndexMap<String, Json>,
    /// Geometry and outline for a `shape` block. Typed rather than folded into
    /// `style` because the importer and the exporter both depend on the exact
    /// field names, and a typo there loses a shape silently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeSpec>,
    /// Layout for a `table` block. Its text is the markdown table in `md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableSpec>,
    #[serde(default)]
    pub locked: bool,
}

impl SlideBlock {
    pub fn geometry(&self) -> Box {
        Box {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            z: self.z,
        }
    }
}

mod kind_serde {
    use super::Kind;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(kind: &Kind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(kind.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Kind, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Kind::from_name(&s).unwrap_or(Kind::Text))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub id: String,
    pub title: String,
    #[serde(rename = "layoutName")]
    pub layout_name: String,
    #[serde(default)]
    pub notes: String,
    pub canvas: Canvas,
    pub blocks: Vec<SlideBlock>,
    /// The layout part this slide sat on in the Office file it came from
    /// (`slideLayouts/slideLayout2.xml`), so an export can put it back on that
    /// layout of the preserved design. `None` for a slide made here.
    #[serde(
        rename = "layoutPart",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_part: Option<String>,
    /// Whether the design's own shapes (logo, footer, rule) draw behind this
    /// slide. Only an imported slide that said `showMasterSp="0"` is `false`.
    #[serde(
        rename = "masterShapes",
        default = "default_true",
        skip_serializing_if = "is_true"
    )]
    pub master_shapes: bool,
    /// Relative path of the markdown file this came from, when loaded from disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/* --------------------------------------------------------------------- doc */

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Margin {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for Margin {
    fn default() -> Self {
        Self {
            top: 72.0,
            right: 72.0,
            bottom: 72.0,
            left: 72.0,
        }
    }
}

/// A header or footer line: three slots, as Word and Excel both model them.
///
/// The tokens `{PAGE}`, `{PAGES}` and `{DATE}` are substituted when the page is
/// drawn, which is what makes a page number a page number rather than a literal
/// "1" repeated on every sheet.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Running {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub left: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub center: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub right: String,
}

impl Running {
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.center.is_empty() && self.right.is_empty()
    }

    /// The three slots with the page tokens filled in.
    pub fn resolved(&self, page: usize, pages: usize, today: &str) -> [String; 3] {
        let fill = |text: &str| {
            text.replace("{PAGE}", &page.to_string())
                .replace("{PAGES}", &pages.to_string())
                .replace("{DATE}", today)
        };
        [fill(&self.left), fill(&self.center), fill(&self.right)]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub size: String,
    /// Explicit page size in px, written only when the paper name does not
    /// already say it — a landscape A4, or a size Word let the author type in.
    ///
    /// A name alone cannot describe "A4 rotated" or "180×250mm", and a document
    /// whose author set the page deliberately is exactly the one that must not
    /// be reflowed onto A4 when it is opened here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(default)]
    pub margin: Margin,
    #[serde(default = "one")]
    pub columns: u32,
    /// Drawn at the top of every page, above the text area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Running>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<Running>,
}

fn one() -> u32 {
    1
}

impl Default for Page {
    fn default() -> Self {
        Self {
            size: "A4".into(),
            width: None,
            height: None,
            margin: Margin::default(),
            columns: 1,
            header: None,
            footer: None,
        }
    }
}

/// The papers this format names, portrait, in px at 96dpi.
///
/// The millimetre sizes rounded the way a 96dpi screen rounds them: A4 is
/// 210mm, which is 793.7px, which is the 794 Word itself lays out.
pub const PAPERS: &[(&str, f64, f64)] = &[
    ("A3", 1123.0, 1587.0),
    ("A4", 794.0, 1123.0),
    ("A5", 559.0, 794.0),
    ("B5", 688.0, 971.0),
    ("Letter", 816.0, 1056.0),
    ("Legal", 816.0, 1344.0),
];

/// The label a page of no named size carries.
pub const CUSTOM_PAPER: &str = "사용자 지정";

impl Page {
    /// Page pixel dimensions at the editor's 96dpi.
    ///
    /// An explicit size wins over the name, which is what makes a landscape or
    /// hand-typed page keep its proportions.
    pub fn dimensions(&self) -> (f64, f64) {
        match (self.width, self.height) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => (w, h),
            _ => Page::paper(&self.size).unwrap_or((794.0, 1123.0)),
        }
    }

    /// Portrait dimensions of a named paper.
    pub fn paper(name: &str) -> Option<(f64, f64)> {
        PAPERS
            .iter()
            .find(|(paper, ..)| *paper == name)
            .map(|(_, w, h)| (*w, *h))
    }

    /// The paper a size is, in either orientation, within a pixel of rounding.
    ///
    /// Word writes A4 as 11906×16838 twips, which is 793.73×1122.53px; a size
    /// has to be recognised through that, not compared exactly.
    pub fn name_for(w: f64, h: f64) -> Option<&'static str> {
        PAPERS
            .iter()
            .find(|(_, pw, ph)| {
                let near = |a: f64, b: f64| (a - b).abs() <= 2.0;
                (near(w, *pw) && near(h, *ph)) || (near(w, *ph) && near(h, *pw))
            })
            .map(|(name, ..)| *name)
    }

    /// The same page at a new size. Everything but the size is carried over.
    pub fn resized(&self, w: f64, h: f64) -> Page {
        Page {
            header: self.header.clone(),
            footer: self.footer.clone(),
            ..Page::sized(w, h, self.margin, self.columns)
        }
    }

    /// A page of the given pixel size, named when it is a known paper and only
    /// carrying explicit dimensions when the name is not enough.
    pub fn sized(w: f64, h: f64, margin: Margin, columns: u32) -> Page {
        let name = Page::name_for(w, h);
        let portrait = name
            .and_then(Page::paper)
            .is_some_and(|(pw, ph)| (pw - w).abs() <= 2.0 && (ph - h).abs() <= 2.0);
        Page {
            size: name.unwrap_or(CUSTOM_PAPER).to_string(),
            width: (!portrait).then_some(w),
            height: (!portrait).then_some(h),
            margin,
            columns,
            header: None,
            footer: None,
        }
    }

    /// True when the page is wider than it is tall, which is what Office calls
    /// landscape orientation.
    pub fn landscape(&self) -> bool {
        let (w, h) = self.dimensions();
        w > h
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spacing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<f64>,
}

/// Paragraph formatting that differs from the document default.
///
/// A block with no override carries no marker in the markdown at all — that is
/// what keeps a Doc's `.md` readable as plain prose.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<Spacing>,
    #[serde(
        default,
        deserialize_with = "null_to_default",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub style: IndexMap<String, Json>,
}

impl Override {
    pub fn is_empty(&self) -> bool {
        self.align.is_none()
            && self.indent.is_none()
            && self.spacing.is_none()
            && self.style.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocBlock {
    pub id: String,
    pub md: String,
    #[serde(rename = "type", with = "block_type_serde")]
    pub block_type: BlockType,
    #[serde(rename = "override", default, skip_serializing_if = "Option::is_none")]
    pub format_override: Option<Override>,
    /// Layout for a table block, alongside the markdown table in `md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableSpec>,
}

mod block_type_serde {
    use super::BlockType;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &BlockType, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(t.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BlockType, D::Error> {
        let s = String::deserialize(d)?;
        Ok(BlockType::from_name(&s).unwrap_or(BlockType::Paragraph))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub page: Page,
    pub blocks: Vec<DocBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/* -------------------------------------------------------------------- grid */

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dims {
    pub rows: u32,
    pub cols: u32,
}

impl Default for Dims {
    fn default() -> Self {
        Self {
            rows: 200,
            cols: 26,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frozen {
    pub rows: u32,
    pub cols: u32,
}

impl Default for Frozen {
    fn default() -> Self {
        Self { rows: 1, cols: 0 }
    }
}

/// A floating chart placed over the grid, the way Excel does it.
///
/// Geometry is in pixels relative to the sheet's top-left so the chart survives
/// column resizing, and the spec keeps a `range` rather than a copy of the
/// numbers — the chart is a view of the cells, not a snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetChart {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub spec: ChartSpec,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub dims: Dims,
    #[serde(default)]
    pub frozen: Frozen,
    #[serde(rename = "colWidths", default)]
    pub col_widths: IndexMap<String, i64>,
    #[serde(rename = "rowHeights", default)]
    pub row_heights: IndexMap<String, i64>,
    #[serde(default)]
    pub merges: Vec<Json>,
    #[serde(default)]
    pub names: Names,
    #[serde(default)]
    pub charts: Vec<SheetChart>,
    #[serde(default)]
    pub cells: IndexMap<String, Cell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl CellSource for Sheet {
    fn cell(&self, reference: &str) -> Option<&Cell> {
        self.cells.get(reference)
    }
}

/* ----------------------------------------------------------------- project */

/// The items of a project. Exactly one variant is populated, matching `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Items {
    Slides(Vec<Slide>),
    Sections(Vec<Section>),
    Sheets(Vec<Sheet>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub project_type: ProjectType,
    /// Absolute path of the project folder.
    pub dir: std::path::PathBuf,
    pub manifest: Manifest,
    pub items: Items,
}

impl Project {
    pub fn slides(&self) -> &[Slide] {
        match &self.items {
            Items::Slides(v) => v,
            _ => &[],
        }
    }
    pub fn sections(&self) -> &[Section] {
        match &self.items {
            Items::Sections(v) => v,
            _ => &[],
        }
    }
    pub fn sheets(&self) -> &[Sheet] {
        match &self.items {
            Items::Sheets(v) => v,
            _ => &[],
        }
    }
}

/// A project's summary row in the launcher.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    #[serde(rename = "type")]
    pub project_type: String,
    pub folder: String,
    pub title: String,
    pub id: Option<String>,
    pub modified: Option<String>,
    pub count: usize,
    pub label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slide_block_tolerates_an_explicit_null_style() {
        let block: SlideBlock = serde_json::from_value(serde_json::json!({
            "id": "b1", "kind": "text", "md": "안녕", "x": 0.0, "y": 0.0,
            "w": 100.0, "h": 50.0, "z": 0.0, "style": null,
        }))
        .expect("null style must load as an empty map, not fail");
        assert!(block.style.is_empty());
    }

    #[test]
    fn a_named_paper_needs_no_explicit_size() {
        let page = Page::sized(794.0, 1123.0, Margin::default(), 1);
        assert_eq!(page.size, "A4");
        assert_eq!((page.width, page.height), (None, None));
        assert_eq!(page.dimensions(), (794.0, 1123.0));
        assert!(!page.landscape());
    }

    #[test]
    fn a_landscape_paper_keeps_its_name_and_states_its_size() {
        // "A4" alone cannot describe a page turned sideways, so the name says
        // which paper and the dimensions say how it is turned.
        let page = Page::sized(1123.0, 794.0, Margin::default(), 1);
        assert_eq!(page.size, "A4");
        assert_eq!(page.dimensions(), (1123.0, 794.0));
        assert!(page.landscape());
    }

    #[test]
    fn a_size_no_paper_matches_is_labelled_and_kept() {
        let page = Page::sized(680.0, 945.0, Margin::default(), 1);
        assert_eq!(page.size, CUSTOM_PAPER);
        assert_eq!(page.dimensions(), (680.0, 945.0));
    }

    #[test]
    fn a_paper_is_recognised_through_words_rounding() {
        // Word writes A4 as 11906 twips, which is 793.73px.
        assert_eq!(Page::name_for(793.0, 1123.0), Some("A4"));
        assert_eq!(Page::name_for(816.0, 1056.0), Some("Letter"));
        assert_eq!(Page::name_for(700.0, 900.0), None);
    }

    #[test]
    fn an_explicit_size_outranks_the_name() {
        // What a file written before explicit sizes existed looks like, and what
        // one written by an editor that resized the page looks like.
        let named: Page = serde_json::from_str(r#"{"size":"A4"}"#).unwrap();
        assert_eq!(named.dimensions(), (794.0, 1123.0));
        let sized: Page =
            serde_json::from_str(r#"{"size":"A4","width":1123,"height":794}"#).unwrap();
        assert_eq!(sized.dimensions(), (1123.0, 794.0));
    }
}
