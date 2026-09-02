//! Project folders on disk: create, load, save, list, rename, delete.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

use crate::deck::{make_slide, normalize_slide, read_slide, write_slide};
use crate::digest::build_digest;
use crate::doc::{make_section, normalize_section, read_section, write_section};
use crate::grid::{make_sheet, normalize_sheet, read_sheet, write_sheet_in};
use crate::ids::{new_project_id, pad, slugify};
use crate::model::{Items, Manifest, ManifestEntry, Project, ProjectSummary, ProjectType, Theme};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("경로가 작업 폴더를 벗어납니다: {0}")]
    Escape(String),
    #[error("AI Studio 프로젝트가 아닙니다: {0}")]
    NotAProject(String),
    #[error("알 수 없는 문서 종류: {0}")]
    UnknownType(String),
    #[error("문서를 찾을 수 없습니다: {0}")]
    NotFound(String),
    #[error("사용 가능한 폴더 이름을 찾지 못했습니다")]
    NoFreeName,
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Reject paths that would escape the workspace root.
///
/// Resolution is lexical on purpose: the target usually does not exist yet
/// (creating a project), so `canonicalize` is not available, and a symlink
/// inside the workspace is the user's own doing.
pub fn resolve_inside(root: &Path, target: &str) -> Result<PathBuf> {
    let candidate = Path::new(target);
    if candidate.is_absolute() {
        return Err(Error::Escape(target.to_string()));
    }
    let mut out = root.to_path_buf();
    for part in candidate.components() {
        use std::path::Component;
        match part {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() || !out.starts_with(root) {
                    return Err(Error::Escape(target.to_string()));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::Escape(target.to_string()))
            }
        }
    }
    if !out.starts_with(root) {
        return Err(Error::Escape(target.to_string()));
    }
    Ok(out)
}

pub fn type_from_path(dir: &Path) -> Option<ProjectType> {
    ProjectType::from_folder(&dir.file_name()?.to_string_lossy())
}

/* --------------------------------------------------------------- utilities */

fn read_json(file: &Path) -> Option<Json> {
    serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
}

fn read_text(file: &Path) -> String {
    fs::read_to_string(file).unwrap_or_default()
}

fn write_json(file: &Path, data: &Json) -> Result<()> {
    write_text(file, &format!("{}\n", crate::json::to_pretty(data)))
}

fn write_text(file: &Path, text: &str) -> Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, text)?;
    Ok(())
}

fn now_iso() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn json_path_for(project_type: ProjectType, md_path: &str) -> String {
    match md_path.strip_suffix(".md") {
        Some(stem) => format!("{stem}{}", project_type.json_suffix()),
        None => md_path.to_string(),
    }
}

fn unique_dir(root: &Path, base: &str) -> Result<PathBuf> {
    let target = root.join(base);
    if !target.exists() {
        return Ok(target);
    }
    let (stem, ext) = match base.rfind('.') {
        Some(at) => (&base[..at], &base[at..]),
        None => (base, ""),
    };
    for i in 2..1000 {
        let candidate = root.join(format!("{stem}-{i}{ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(Error::NoFreeName)
}

/* ------------------------------------------------------------------ create */

pub fn create_project(
    root: &Path,
    project_type: ProjectType,
    title: &str,
    sample: bool,
) -> Result<Project> {
    let name = {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            format!("제목 없는 {}", project_type.label())
        } else {
            trimmed.to_string()
        }
    };

    let dir = unique_dir(
        root,
        &format!("{}{}", slugify(&name, "untitled"), project_type.ext()),
    )?;
    fs::create_dir_all(dir.join("assets"))?;

    let now = now_iso();
    let items = match project_type {
        ProjectType::Deck => Items::Slides(if sample {
            vec![
                make_slide("title", Some(&name), 1),
                make_slide("title-content", Some("개요"), 2),
            ]
        } else {
            vec![make_slide("title", Some(&name), 1)]
        }),
        ProjectType::Doc => Items::Sections(vec![make_section(&name, true)]),
        ProjectType::Grid => Items::Sheets(vec![make_sheet("시트1", sample)]),
    };

    let project = Project {
        project_type,
        dir: dir.clone(),
        manifest: Manifest {
            format: project_type.format_id().to_string(),
            format_version: 1,
            id: new_project_id(),
            title: name,
            created: now.clone(),
            modified: now,
            theme: Theme::default(),
            entries: Vec::new(),
        },
        items,
    };

    save_project(&project)?;
    load_project(&dir)
}

/* -------------------------------------------------------------------- load */

pub fn load_project(dir: &Path) -> Result<Project> {
    let Some(project_type) = type_from_path(dir) else {
        return Err(Error::NotAProject(
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));
    };

    let manifest_json = read_json(&dir.join("manifest.json"));
    let manifest = read_manifest(manifest_json.as_ref(), dir, project_type);
    let entries = resolve_entries(dir, project_type, manifest_json.as_ref())?;

    let items = match project_type {
        ProjectType::Deck => {
            let mut slides: Vec<_> = entries
                .iter()
                .map(|e| {
                    let md = read_text(&dir.join(&e.0));
                    let layout = read_json(&dir.join(&e.1));
                    let mut slide = read_slide(&md, layout.as_ref());
                    slide.file = Some(e.0.clone());
                    slide
                })
                .collect();
            if slides.is_empty() {
                slides.push(make_slide("title", Some(&manifest.title), 1));
            }
            Items::Slides(slides)
        }
        ProjectType::Doc => {
            let mut sections: Vec<_> = entries
                .iter()
                .map(|e| {
                    let md = read_text(&dir.join(&e.0));
                    let meta = read_json(&dir.join(&e.1));
                    let mut section = read_section(&md, meta.as_ref());
                    section.file = Some(e.0.clone());
                    section
                })
                .collect();
            if sections.is_empty() {
                sections.push(make_section(&manifest.title, true));
            }
            Items::Sections(sections)
        }
        ProjectType::Grid => {
            let mut sheets: Vec<_> = entries
                .iter()
                .map(|e| {
                    let md = read_text(&dir.join(&e.0));
                    let cells = read_json(&dir.join(&e.1));
                    let mut sheet = read_sheet(&md, cells.as_ref());
                    sheet.file = Some(e.0.clone());
                    sheet
                })
                .collect();
            if sheets.is_empty() {
                sheets.push(make_sheet("시트1", false));
            }
            Items::Sheets(sheets)
        }
    };

    Ok(Project {
        project_type,
        dir: dir.to_path_buf(),
        manifest,
        items,
    })
}

fn read_manifest(json: Option<&Json>, dir: &Path, project_type: ProjectType) -> Manifest {
    let folder_title = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .trim_end_matches(project_type.ext())
        .to_string();
    let now = now_iso();
    let get = |key: &str| {
        json.and_then(|m| m.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };

    Manifest {
        format: get("format").unwrap_or_else(|| project_type.format_id().to_string()),
        format_version: json
            .and_then(|m| m.get("formatVersion"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32,
        id: get("id").unwrap_or_else(new_project_id),
        title: get("title").unwrap_or(folder_title),
        created: get("created").unwrap_or_else(|| now.clone()),
        modified: get("modified").unwrap_or(now),
        theme: json
            .and_then(|m| m.get("theme"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        entries: json
            .and_then(|m| m.get(project_type.key()))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
    }
}

/// Work out which md/json pairs belong to the project, as `(md, json)` pairs.
///
/// The manifest is the ordering authority, but a directory scan fills in files
/// added by hand (or by an AI agent writing markdown directly) so they are not
/// silently ignored.
fn resolve_entries(
    dir: &Path,
    project_type: ProjectType,
    manifest: Option<&Json>,
) -> Result<Vec<(String, String)>> {
    let listed = manifest
        .and_then(|m| m.get(project_type.key()))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for item in &listed {
        let Some(md) = item.get("md").and_then(|v| v.as_str()) else {
            continue;
        };
        if !dir.join(md).exists() {
            continue;
        }
        let json = item
            .get("json")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| json_path_for(project_type, md));
        entries.push((md.to_string(), json));
        seen.insert(md.to_string());
    }

    let sub = dir.join(project_type.dir());
    let mut extras: Vec<String> = match fs::read_dir(&sub) {
        Err(_) => Vec::new(),
        Ok(read) => read
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".md"))
            .map(|n| format!("{}/{n}", project_type.dir()))
            .filter(|rel| !seen.contains(rel))
            .collect(),
    };
    extras.sort();

    for rel in extras {
        let json = json_path_for(project_type, &rel);
        entries.push((rel, json));
    }
    Ok(entries)
}

/* -------------------------------------------------------------------- save */

/// Write a project to disk as md/json pairs, then regenerate `AI.md`.
///
/// Files are renumbered to match the current order so `ls` reflects the deck
/// order, and stale files from removed or renamed items are pruned.
pub fn save_project(project: &Project) -> Result<Project> {
    let project_type = project.project_type;
    let dir = &project.dir;

    let items = normalize_items(project);
    let mut written: Vec<ManifestEntry> = Vec::new();
    let mut keep: HashSet<String> = HashSet::new();

    let names = item_names(&items);
    for (i, name) in names.iter().enumerate() {
        let stem = format!(
            "{}-{}",
            pad(i + 1, 2),
            slugify(name, &format!("item-{}", i + 1))
        );
        let md_rel = format!("{}/{stem}.md", project_type.dir());
        let json_rel = json_path_for(project_type, &md_rel);

        let (md, json, id) = serialize_item(&items, i);
        write_text(&dir.join(&md_rel), &md)?;
        write_json(&dir.join(&json_rel), &json)?;

        keep.insert(basename(&md_rel));
        keep.insert(basename(&json_rel));
        written.push(ManifestEntry {
            id,
            name: name.clone(),
            md: md_rel,
            json: json_rel,
        });
    }

    prune_dir(&dir.join(project_type.dir()), &keep)?;

    let manifest = Manifest {
        format: project_type.format_id().to_string(),
        format_version: 1,
        modified: now_iso(),
        entries: written,
        ..project.manifest.clone()
    };
    write_json(
        &dir.join("manifest.json"),
        &manifest_json(&manifest, project_type),
    )?;

    let saved = Project {
        project_type,
        dir: dir.clone(),
        manifest,
        items,
    };
    write_text(&dir.join("AI.md"), &build_digest(&saved))?;
    Ok(saved)
}

fn manifest_json(manifest: &Manifest, project_type: ProjectType) -> Json {
    json!({
        "format": manifest.format,
        "formatVersion": manifest.format_version,
        "id": manifest.id,
        "title": manifest.title,
        "created": manifest.created,
        "modified": manifest.modified,
        "theme": manifest.theme,
        project_type.key(): manifest.entries,
    })
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn normalize_items(project: &Project) -> Items {
    match &project.items {
        Items::Slides(v) => Items::Slides(v.iter().map(normalize_slide).collect()),
        Items::Sections(v) => Items::Sections(v.iter().map(normalize_section).collect()),
        Items::Sheets(v) => Items::Sheets(v.iter().map(normalize_sheet).collect()),
    }
}

fn item_names(items: &Items) -> Vec<String> {
    match items {
        Items::Slides(v) => v
            .iter()
            .map(|s| {
                if s.title.is_empty() {
                    "슬라이드".into()
                } else {
                    s.title.clone()
                }
            })
            .collect(),
        Items::Sections(v) => v
            .iter()
            .map(|s| {
                if s.name.is_empty() {
                    "섹션".into()
                } else {
                    s.name.clone()
                }
            })
            .collect(),
        Items::Sheets(v) => v
            .iter()
            .map(|s| {
                if s.name.is_empty() {
                    "시트".into()
                } else {
                    s.name.clone()
                }
            })
            .collect(),
    }
}

fn serialize_item(items: &Items, i: usize) -> (String, Json, String) {
    match items {
        Items::Slides(v) => {
            let files = write_slide(&v[i]);
            (files.md, files.layout, v[i].id.clone())
        }
        Items::Sections(v) => {
            let files = write_section(&v[i]);
            (files.md, files.meta, v[i].id.clone())
        }
        Items::Sheets(v) => {
            // The sibling sheets come along: a cell holding `=요약!B4` has to
            // project its value, not a `#REF!`.
            let files = write_sheet_in(&v[i], &crate::grid::book(v));
            (files.md, files.cells, v[i].id.clone())
        }
    }
}

fn prune_dir(dir: &Path, keep: &HashSet<String>) -> Result<()> {
    let Ok(read) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_ours = name.ends_with(".md") || name.ends_with(".json");
        if is_ours && !keep.contains(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/* -------------------------------------------------------------------- list */

pub fn list_projects(root: &Path) -> Result<Vec<ProjectSummary>> {
    fs::create_dir_all(root)?;
    let mut out = Vec::new();

    for entry in fs::read_dir(root)?.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().into_owned();
        let Some(project_type) = ProjectType::from_folder(&folder) else {
            continue;
        };
        let dir = root.join(&folder);
        let manifest = read_json(&dir.join("manifest.json"));
        let get = |key: &str| {
            manifest
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };

        let modified = get("modified").or_else(|| {
            fs::metadata(&dir).and_then(|m| m.modified()).ok().map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                    .to_string()
            })
        });

        out.push(ProjectSummary {
            project_type: project_type.as_str().to_string(),
            title: get("title")
                .unwrap_or_else(|| folder.trim_end_matches(project_type.ext()).to_string()),
            folder,
            id: get("id"),
            modified,
            count: manifest
                .as_ref()
                .and_then(|m| m.get(project_type.key()))
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
            label: project_type.label().to_string(),
        });
    }

    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

/// Rename a project, moving its folder to match the new title.
/// Returns the (possibly new) folder name.
pub fn rename_project(root: &Path, folder: &str, title: &str) -> Result<String> {
    let dir = resolve_inside(root, folder)?;
    let Some(project_type) = type_from_path(&dir) else {
        return Err(Error::NotAProject(folder.to_string()));
    };

    let mut manifest = read_json(&dir.join("manifest.json")).unwrap_or_else(|| json!({}));
    manifest["title"] = json!(title);
    manifest["modified"] = json!(now_iso());
    write_json(&dir.join("manifest.json"), &manifest)?;

    let next_dir = unique_dir(
        root,
        &format!("{}{}", slugify(title, "untitled"), project_type.ext()),
    )?;
    if next_dir != dir {
        fs::rename(&dir, &next_dir)?;
        let project = load_project(&next_dir)?;
        save_project(&project)?;
        return Ok(next_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default());
    }
    let project = load_project(&dir)?;
    save_project(&project)?;
    Ok(folder.to_string())
}

pub fn delete_project(root: &Path, folder: &str) -> Result<()> {
    let dir = resolve_inside(root, folder)?;
    if type_from_path(&dir).is_none() {
        return Err(Error::NotAProject(folder.to_string()));
    }
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
}

/// Every file in a project, for the "파일 보기" inspector in the UI.
pub fn project_files(dir: &Path) -> Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    Ok(out)
}

fn walk(current: &Path, rel: &str, out: &mut Vec<FileEntry>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(current)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel_path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk(&entry.path(), &rel_path, out)?;
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(FileEntry {
                path: rel_path,
                size,
            });
        }
    }
    Ok(())
}

pub fn read_project_file(dir: &Path, rel_path: &str) -> Result<String> {
    let abs = resolve_inside(dir, rel_path)?;
    Ok(read_text(&abs))
}

/// A project directory that must already exist, for API handlers.
pub fn existing_project_dir(root: &Path, folder: &str) -> Result<PathBuf> {
    let dir = resolve_inside(root, folder)?;
    if type_from_path(&dir).is_none() {
        return Err(Error::NotAProject(folder.to_string()));
    }
    if !dir.is_dir() {
        return Err(Error::NotFound(folder.to_string()));
    }
    Ok(dir)
}
