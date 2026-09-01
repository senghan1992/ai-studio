//! The one API surface.
//!
//! Both front ends call these methods: the desktop app through Tauri commands,
//! the headless server through HTTP routes. Neither holds any document logic of
//! its own, so the two can never drift.

use std::path::{Path, PathBuf};

use base64::Engine;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use ai_export::Format;
use ai_format::model::{
    Items, Manifest, Project, ProjectSummary, ProjectType, Section, Sheet, Slide,
};
use ai_format::project as fsproj;
use ai_formula::evaluate::{recalc_sheet, Cell, Names};

pub mod preview;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    BadRequest(String),
    #[error("문서를 찾을 수 없습니다: {0}")]
    NotFound(String),
    #[error(transparent)]
    Project(#[from] fsproj::Error),
    #[error(transparent)]
    Export(#[from] ai_export::ExportError),
    #[error("파일을 읽을 수 없습니다: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// The HTTP status this maps to, so the server needs no error table.
    pub fn status(&self) -> u16 {
        match self {
            Error::BadRequest(_) => 400,
            Error::NotFound(_) => 404,
            Error::Project(fsproj::Error::Escape(_)) => 400,
            Error::Project(fsproj::Error::NotAProject(_)) => 400,
            Error::Project(fsproj::Error::UnknownType(_)) => 400,
            Error::Project(fsproj::Error::NotFound(_)) => 404,
            Error::Export(ai_export::ExportError::WrongType(_, _)) => 400,
            _ => 500,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/* ------------------------------------------------------------------ payloads */

/// A whole project, in the shape the editors already consume.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectPayload {
    #[serde(rename = "type", default)]
    pub project_type: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub manifest: ManifestPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slides: Option<Vec<Slide>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<Vec<Section>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheets: Option<Vec<Sheet>>,
}

/// The manifest as the API exposes it: the typed fields plus the item list under
/// its type-specific key, exactly as `manifest.json` has it.
///
/// Every field is optional on the way in. An agent editing a document over the
/// API should be able to `PUT` just the slides it changed without having to
/// echo back a manifest it never looked at; whatever it omits is taken from the
/// copy already on disk.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ManifestPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(
        default,
        rename = "formatVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub format_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ai_format::model::Theme>,
    #[serde(flatten)]
    pub entries: IndexMap<String, Json>,
}

impl ProjectPayload {
    fn of(project: &Project) -> ProjectPayload {
        let m = &project.manifest;
        let mut entries = IndexMap::new();
        entries.insert(
            project.project_type.key().to_string(),
            serde_json::to_value(&m.entries).unwrap_or(Json::Null),
        );

        let (slides, sections, sheets) = match &project.items {
            Items::Slides(v) => (Some(v.clone()), None, None),
            Items::Sections(v) => (None, Some(v.clone()), None),
            Items::Sheets(v) => (None, None, Some(v.clone())),
        };

        ProjectPayload {
            project_type: project.project_type.as_str().to_string(),
            dir: project.dir.to_string_lossy().into_owned(),
            folder: project
                .dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            manifest: ManifestPayload {
                format: Some(m.format.clone()),
                format_version: Some(m.format_version),
                id: Some(m.id.clone()),
                title: Some(m.title.clone()),
                created: Some(m.created.clone()),
                modified: Some(m.modified.clone()),
                theme: Some(m.theme.clone()),
                entries,
            },
            slides,
            sections,
            sheets,
        }
    }

    /// Rebuild a `Project` from what a client sent.
    ///
    /// `base` is the manifest already on disk: the folder is authoritative for
    /// where the project lives, and for `id` and `created`, which a client has
    /// no business inventing.
    fn into_project(
        self,
        dir: PathBuf,
        project_type: ProjectType,
        base: Manifest,
    ) -> Result<Project> {
        let items = match project_type {
            ProjectType::Deck => Items::Slides(self.slides.unwrap_or_default()),
            ProjectType::Doc => Items::Sections(self.sections.unwrap_or_default()),
            ProjectType::Grid => Items::Sheets(self.sheets.unwrap_or_default()),
        };
        let m = self.manifest;
        Ok(Project {
            project_type,
            dir,
            manifest: Manifest {
                format: project_type.format_id().to_string(),
                format_version: 1,
                id: m.id.filter(|s| !s.is_empty()).unwrap_or(base.id),
                title: m.title.filter(|s| !s.is_empty()).unwrap_or(base.title),
                created: m.created.filter(|s| !s.is_empty()).unwrap_or(base.created),
                modified: m.modified.unwrap_or(base.modified),
                theme: m.theme.unwrap_or(base.theme),
                entries: Vec::new(),
            },
            items,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub ok: bool,
    pub workspace: String,
    pub types: Vec<&'static str>,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ProjectList {
    pub workspace: String,
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    #[serde(rename = "type")]
    pub project_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "yes")]
    pub sample: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct RecalcRequest {
    #[serde(default)]
    pub cells: IndexMap<String, Cell>,
    #[serde(default)]
    pub names: Names,
}

#[derive(Debug, Serialize)]
pub struct RecalcResponse {
    pub cells: IndexMap<String, Cell>,
    pub changed: Vec<String>,
    pub errors: IndexMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct FileList {
    pub files: Vec<fsproj::FileEntry>,
}

#[derive(Debug, Serialize)]
pub struct FileBody {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct UploadAssetRequest {
    pub name: String,
    /// A `data:` URL, as the browser's FileReader produces.
    #[serde(rename = "dataUrl")]
    pub data_url: String,
}

#[derive(Debug, Serialize)]
pub struct AssetList {
    pub assets: Vec<AssetEntry>,
}

#[derive(Debug, Serialize)]
pub struct AssetEntry {
    pub name: String,
    /// The path to put in markdown, relative to the item's folder.
    pub path: String,
    pub size: u64,
}

/// A rendered export, ready to hand to a download.
pub struct ExportBody {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub filename: String,
}

/* -------------------------------------------------------------------- studio */

pub struct Studio {
    workspace: PathBuf,
}

impl Studio {
    /// Open a workspace, creating the folder if it is not there yet.
    pub fn open(workspace: impl Into<PathBuf>) -> Result<Studio> {
        let workspace = workspace.into();
        std::fs::create_dir_all(&workspace)?;
        // Canonicalize so path containment checks compare like with like.
        let workspace = workspace.canonicalize().unwrap_or(workspace);
        Ok(Studio { workspace })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn health(&self) -> Health {
        Health {
            ok: true,
            workspace: self.workspace.to_string_lossy().into_owned(),
            types: ai_format::model::PROJECT_TYPES
                .iter()
                .map(|t| t.as_str())
                .collect(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn list_projects(&self) -> Result<ProjectList> {
        Ok(ProjectList {
            workspace: self.workspace.to_string_lossy().into_owned(),
            projects: fsproj::list_projects(&self.workspace)?,
        })
    }

    pub fn create_project(&self, request: CreateRequest) -> Result<ProjectPayload> {
        let Some(project_type) = ProjectType::from_name(&request.project_type) else {
            return Err(Error::BadRequest(format!(
                "알 수 없는 문서 종류: {}",
                request.project_type
            )));
        };
        let project = fsproj::create_project(
            &self.workspace,
            project_type,
            &request.title,
            request.sample,
        )?;
        Ok(ProjectPayload::of(&project))
    }

    fn dir_of(&self, folder: &str) -> Result<PathBuf> {
        Ok(fsproj::existing_project_dir(&self.workspace, folder)?)
    }

    pub fn get_project(&self, folder: &str) -> Result<ProjectPayload> {
        let dir = self.dir_of(folder)?;
        Ok(ProjectPayload::of(&fsproj::load_project(&dir)?))
    }

    /// Save what a client sent, then hand back what actually landed on disk.
    pub fn save_project(&self, folder: &str, payload: ProjectPayload) -> Result<ProjectPayload> {
        let dir = self.dir_of(folder)?;
        let Some(project_type) = fsproj::type_from_path(&dir) else {
            return Err(Error::BadRequest("AI Studio 프로젝트가 아닙니다".into()));
        };
        let base = fsproj::load_project(&dir)?.manifest;
        let project = payload.into_project(dir, project_type, base)?;
        Ok(ProjectPayload::of(&fsproj::save_project(&project)?))
    }

    /// Rename a project, moving its folder to match.
    ///
    /// Returns the project as it now stands — the caller needs the new folder
    /// name, and usually the regenerated manifest with it.
    pub fn rename_project(&self, folder: &str, title: &str) -> Result<ProjectPayload> {
        self.dir_of(folder)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::BadRequest("제목이 비어 있습니다".into()));
        }
        let next = fsproj::rename_project(&self.workspace, folder, title)?;
        self.get_project(&next)
    }

    pub fn delete_project(&self, folder: &str) -> Result<()> {
        self.dir_of(folder)?;
        Ok(fsproj::delete_project(&self.workspace, folder)?)
    }

    pub fn project_files(&self, folder: &str) -> Result<FileList> {
        let dir = self.dir_of(folder)?;
        Ok(FileList {
            files: fsproj::project_files(&dir)?,
        })
    }

    pub fn read_file(&self, folder: &str, path: &str) -> Result<FileBody> {
        let dir = self.dir_of(folder)?;
        Ok(FileBody {
            path: path.to_string(),
            text: fsproj::read_project_file(&dir, path)?,
        })
    }

    pub fn digest(&self, folder: &str) -> Result<String> {
        let dir = self.dir_of(folder)?;
        Ok(fsproj::read_project_file(&dir, "AI.md")?)
    }

    pub fn export(&self, folder: &str, ext: &str) -> Result<ExportBody> {
        let Some(format) = Format::from_ext(ext) else {
            return Err(Error::BadRequest(format!(
                "지원하지 않는 내보내기 형식: {ext}"
            )));
        };
        let dir = self.dir_of(folder)?;
        let project = fsproj::load_project(&dir)?;
        let bytes = ai_export::export(&project, format)?;
        Ok(ExportBody {
            bytes,
            mime: format.mime(),
            filename: ai_export::suggested_filename(&project, format),
        })
    }

    /* ------------------------------------------------------------- assets */

    pub fn list_assets(&self, folder: &str) -> Result<AssetList> {
        let dir = self.dir_of(folder)?;
        let assets_dir = dir.join("assets");
        let mut out = Vec::new();
        if let Ok(read) = std::fs::read_dir(&assets_dir) {
            let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                out.push(AssetEntry {
                    // Markdown keeps a relative path so the folder stays portable.
                    path: format!("../assets/{name}"),
                    size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                    name,
                });
            }
        }
        Ok(AssetList { assets: out })
    }

    /// Store an uploaded image in the project's `assets/` folder.
    pub fn upload_asset(&self, folder: &str, request: UploadAssetRequest) -> Result<AssetEntry> {
        let dir = self.dir_of(folder)?;
        let (mime, bytes) = decode_data_url(&request.data_url)?;
        if !mime.starts_with("image/") {
            return Err(Error::BadRequest(format!("이미지가 아닙니다: {mime}")));
        }
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(Error::BadRequest("이미지가 너무 큽니다 (최대 16MB)".into()));
        }

        let name = safe_asset_name(&request.name, &mime);
        let assets_dir = dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;
        let target = unique_path(&assets_dir, &name);
        std::fs::write(&target, &bytes)?;

        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name);
        Ok(AssetEntry {
            path: format!("../assets/{name}"),
            size: bytes.len() as u64,
            name,
        })
    }

    /// Read an image out of a project. Only `assets/` is reachable.
    pub fn read_asset(&self, folder: &str, path: &str) -> Result<(Vec<u8>, String)> {
        let dir = self.dir_of(folder)?;
        let relative = path.trim_start_matches("../").trim_start_matches("./");
        if !relative.starts_with("assets/") {
            return Err(Error::BadRequest(
                "assets/ 밖의 파일은 제공하지 않습니다".into(),
            ));
        }
        let abs = fsproj::resolve_inside(&dir, relative)?;
        if !abs.is_file() {
            return Err(Error::NotFound(path.to_string()));
        }
        let ext = abs
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mime = ai_export::ooxml::image_content_type(&ext).unwrap_or("application/octet-stream");
        Ok((std::fs::read(&abs)?, mime.to_string()))
    }

    /* -------------------------------------------------------------- calc */

    /// Recalculate a sheet without touching disk — what the grid editor calls
    /// after an edit that the client cannot resolve locally.
    pub fn recalc(&self, request: RecalcRequest) -> RecalcResponse {
        let out = recalc_sheet(&request.cells, &request.names);
        RecalcResponse {
            cells: out.cells,
            changed: out.changed,
            errors: out.errors,
        }
    }

    /// The md/json a save would write, without writing it — the `{ } 저장 포맷`
    /// panel.
    pub fn preview(&self, folder: &str, payload: ProjectPayload) -> Result<preview::Preview> {
        let dir = self.dir_of(folder)?;
        let Some(project_type) = fsproj::type_from_path(&dir) else {
            return Err(Error::BadRequest("AI Studio 프로젝트가 아닙니다".into()));
        };
        let base = fsproj::load_project(&dir)?.manifest;
        let project = payload.into_project(dir, project_type, base)?;
        Ok(preview::of(&project))
    }
}

/// `data:image/png;base64,iVBOR…` -> `("image/png", bytes)`
fn decode_data_url(url: &str) -> Result<(String, Vec<u8>)> {
    let rest = url
        .strip_prefix("data:")
        .ok_or_else(|| Error::BadRequest("data: URL이 아닙니다".into()))?;
    let (header, payload) = rest
        .split_once(',')
        .ok_or_else(|| Error::BadRequest("잘못된 data: URL".into()))?;
    let mime = header.split(';').next().unwrap_or("").to_string();
    if !header.contains("base64") {
        return Err(Error::BadRequest("base64 data: URL만 받습니다".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|_| Error::BadRequest("base64를 해석할 수 없습니다".into()))?;
    Ok((mime, bytes))
}

/// A filename safe to write: no path separators, and an extension matching the
/// declared type rather than whatever the client claimed.
fn safe_asset_name(name: &str, mime: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem: String = stem
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .take(60)
        .collect();
    let stem = if stem.trim().is_empty() {
        "image".to_string()
    } else {
        stem
    };
    let ext = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        _ => "bin",
    };
    format!("{stem}.{ext}")
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(at) => (&name[..at], &name[at..]),
        None => (name, ""),
    };
    for i in 2..10_000 {
        let candidate = dir.join(format!("{stem}-{i}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_decode() {
        // "hi" in base64.
        let (mime, bytes) = decode_data_url("data:image/png;base64,aGk=").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"hi");
        assert!(decode_data_url("http://x/y.png").is_err());
        assert!(decode_data_url("data:image/png,raw").is_err());
    }

    #[test]
    fn asset_names_are_sanitised_and_get_the_real_extension() {
        assert_eq!(safe_asset_name("chart.png", "image/png"), "chart.png");
        // The extension follows the declared type, not the client's claim.
        assert_eq!(safe_asset_name("evil.php", "image/png"), "evil.png");
        assert_eq!(
            safe_asset_name("../../etc/passwd", "image/jpeg"),
            "passwd.jpg"
        );
        assert_eq!(safe_asset_name("", "image/png"), "image.png");
        assert_eq!(safe_asset_name("표.png", "image/png"), "표.png");
    }

    #[test]
    fn errors_map_to_the_right_status() {
        assert_eq!(Error::BadRequest("x".into()).status(), 400);
        assert_eq!(Error::NotFound("x".into()).status(), 404);
        assert_eq!(
            Error::Project(fsproj::Error::Escape("x".into())).status(),
            400
        );
        assert_eq!(
            Error::Project(fsproj::Error::NotFound("x".into())).status(),
            404
        );
    }
}
