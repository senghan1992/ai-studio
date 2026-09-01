//! The document model shared by the editors, the API and the exporters.
//!
//! Field names are the ones the JSON API already used, so the React editors
//! need no translation layer.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use ai_formula::evaluate::{Cell, Names};

use crate::blocks::Kind;
use crate::chart::{CellSource, ChartSpec};
use crate::geometry::{Box, Canvas};
use crate::mdblocks::BlockType;

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
            font: "Inter".into(),
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
    #[serde(default)]
    pub style: IndexMap<String, Json>,
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
    /// Relative path of the markdown file this came from, when loaded from disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub size: String,
    #[serde(default)]
    pub margin: Margin,
    #[serde(default = "one")]
    pub columns: u32,
}

fn one() -> u32 {
    1
}

impl Default for Page {
    fn default() -> Self {
        Self {
            size: "A4".into(),
            margin: Margin::default(),
            columns: 1,
        }
    }
}

impl Page {
    /// Page pixel dimensions at the editor's 96dpi.
    pub fn dimensions(&self) -> (f64, f64) {
        match self.size.as_str() {
            "Letter" => (816.0, 1056.0),
            "A5" => (559.0, 794.0),
            _ => (794.0, 1123.0),
        }
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
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
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
