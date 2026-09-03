//! Version history: a bounded trail of snapshots under `.history/`.
//!
//! Every save first copies the state it is about to overwrite into
//! `.history/<저장시각>/` — the same md/json pairs the format is made of, so a
//! snapshot is itself readable with `cat` and diffable with `diff`. Restoring
//! snapshots the current state first, which makes the restore itself undoable.
//! Assets are not copied: a save never rewrites them, so the images an old
//! snapshot refers to are still in `assets/`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::ProjectType;
use crate::project::{Error, Result};

pub const HISTORY_DIR: &str = ".history";
/// Snapshots kept per project. At the trail's cap the oldest falls off; the
/// number is generous because snapshots are small text.
pub const HISTORY_KEEP: usize = 30;
/// A save arriving within this many seconds of the newest snapshot does not
/// add another one. The editor autosaves every couple of seconds and an agent
/// can save faster still; without a floor, thirty snapshots cover ninety
/// seconds of typing and the version a person made yesterday falls off the
/// end. Two minutes keeps the trail a history rather than a keystroke log.
pub const HISTORY_MIN_INTERVAL_SECS: i64 = 120;

/// One kept version, newest first in a listing.
#[derive(Debug, serde::Serialize)]
pub struct SnapshotEntry {
    /// The folder name under `.history/` — pass this back to restore.
    pub name: String,
    /// When that state was saved (`manifest.modified` at the time).
    #[serde(rename = "savedAt")]
    pub saved_at: String,
    /// The document title at the time.
    pub title: String,
    /// Item files in the snapshot (md/json pairs, plus the manifest).
    pub files: usize,
    pub bytes: u64,
}

/// The project files a snapshot preserves: the manifest and every md/json under
/// the item directory. `AI.md` is regenerated on save and assets never change
/// on save, so neither is copied.
fn tracked_files(dir: &Path, project_type: ProjectType) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if let Ok(bytes) = fs::read(dir.join("manifest.json")) {
        out.push(("manifest.json".to_string(), bytes));
    }
    let sub = dir.join(project_type.dir());
    let mut names: Vec<String> = match fs::read_dir(&sub) {
        Err(_) => Vec::new(),
        Ok(read) => read
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".md") || n.ends_with(".json"))
            .collect(),
    };
    names.sort();
    for name in names {
        if let Ok(bytes) = fs::read(sub.join(&name)) {
            out.push((format!("{}/{name}", project_type.dir()), bytes));
        }
    }
    out
}

/// The content that decides whether a new snapshot is worth keeping: the item
/// files without the manifest, whose `modified` timestamp changes on every save
/// and would defeat the comparison.
fn content_of(files: &[(String, Vec<u8>)]) -> Vec<(&str, &[u8])> {
    files
        .iter()
        .filter(|(rel, _)| rel != "manifest.json")
        .map(|(rel, bytes)| (rel.as_str(), bytes.as_slice()))
        .collect()
}

/// Snapshot the current on-disk state, returning the snapshot name.
///
/// Returns `Ok(None)` when there is nothing to keep: the project has no files
/// yet (first save), or the content is identical to the latest snapshot (a
/// save that changed nothing but the timestamp).
pub fn snapshot(dir: &Path, project_type: ProjectType) -> Result<Option<String>> {
    snapshot_with(dir, project_type, false)
}

/// The same, unconditionally — a restore is about to replace the current
/// state, and that state must be kept whatever the clock says.
pub fn snapshot_forced(dir: &Path, project_type: ProjectType) -> Result<Option<String>> {
    snapshot_with(dir, project_type, true)
}

fn snapshot_with(dir: &Path, project_type: ProjectType, force: bool) -> Result<Option<String>> {
    let files = tracked_files(dir, project_type);
    if files.iter().all(|(rel, _)| rel == "manifest.json") {
        return Ok(None);
    }

    if let Some(latest) = list(dir)?.first() {
        let previous = tracked_files(&dir.join(HISTORY_DIR).join(&latest.name), project_type);
        if content_of(&files) == content_of(&previous) {
            return Ok(None);
        }
        if !force && within_min_interval(&latest.saved_at) {
            return Ok(None);
        }
    }

    let name = snapshot_name(dir);
    let target = dir.join(HISTORY_DIR).join(&name);
    if target.exists() {
        // The same saved state was already kept.
        return Ok(None);
    }
    for (rel, bytes) in &files {
        let file = target.join(rel);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, bytes)?;
    }

    prune(dir)?;
    Ok(Some(name))
}

/// True when a snapshot with this save time is younger than the interval floor.
fn within_min_interval(saved_at: &str) -> bool {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(saved_at) else {
        return false;
    };
    let age = chrono::Utc::now().signed_duration_since(then);
    age.num_seconds() < HISTORY_MIN_INTERVAL_SECS
}

/// The snapshot's folder name: the state's own save time, made filename-safe
/// (`:` is not allowed on Windows).
fn snapshot_name(dir: &Path) -> String {
    let modified = fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|m| m.get("modified").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_else(|| {
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string()
        });
    modified.replace(':', "-")
}

/// Every kept snapshot, newest first.
pub fn list(dir: &Path) -> Result<Vec<SnapshotEntry>> {
    let root = dir.join(HISTORY_DIR);
    let Ok(read) = fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in read.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let manifest = fs::read_to_string(entry.path().join("manifest.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let field = |key: &str| {
            manifest
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(String::from)
        };

        let (files, bytes) = measure(&entry.path());
        out.push(SnapshotEntry {
            saved_at: field("modified").unwrap_or_else(|| name.clone()),
            title: field("title").unwrap_or_default(),
            name,
            files,
            bytes,
        });
    }
    // The names are timestamps, so lexical order is chronological.
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}

fn measure(dir: &Path) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(read) = fs::read_dir(&current) else {
            continue;
        };
        for entry in read.filter_map(|e| e.ok()) {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (files, bytes)
}

/// The snapshot directory a client named, refusing anything that is not a
/// direct child of `.history/`.
fn snapshot_dir(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(Error::Escape(name.to_string()));
    }
    let target = dir.join(HISTORY_DIR).join(name);
    if !target.is_dir() {
        return Err(Error::NotFound(format!("버전 {name}")));
    }
    Ok(target)
}

/// Put a kept version back as the project's current files.
///
/// The state being replaced is snapshotted first, so a restore can itself be
/// undone by restoring the snapshot it created. The caller re-saves the project
/// afterwards, which regenerates `AI.md` and the manifest ordering.
pub fn restore(dir: &Path, project_type: ProjectType, name: &str) -> Result<()> {
    let source = snapshot_dir(dir, name)?;
    snapshot_forced(dir, project_type)?;

    // Out with the current item files…
    let sub = dir.join(project_type.dir());
    if let Ok(read) = fs::read_dir(&sub) {
        for entry in read.filter_map(|e| e.ok()) {
            let file = entry.file_name().to_string_lossy().into_owned();
            if file.ends_with(".md") || file.ends_with(".json") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    // …in with the kept ones.
    for (rel, bytes) in tracked_files(&source, project_type) {
        let file = dir.join(&rel);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, bytes)?;
    }
    Ok(())
}

/// Drop the oldest snapshots beyond the cap.
fn prune(dir: &Path) -> Result<()> {
    let snapshots = list(dir)?;
    for stale in snapshots.iter().skip(HISTORY_KEEP) {
        let _ = fs::remove_dir_all(dir.join(HISTORY_DIR).join(&stale.name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProjectType;
    use crate::project::{create_project, load_project, save_project};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ai-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn first_md(dir: &Path) -> PathBuf {
        let content = dir.join("content");
        let mut names: Vec<_> = fs::read_dir(content)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        names.sort();
        names.remove(0)
    }

    /// Backdate the newest snapshot so the next save clears the interval
    /// floor — the tests edit faster than any human saves.
    fn age_latest_snapshot(dir: &Path) {
        let Some(latest) = list(dir).unwrap().into_iter().next() else {
            return;
        };
        let file = dir
            .join(HISTORY_DIR)
            .join(&latest.name)
            .join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        let old = chrono::Utc::now() - chrono::Duration::minutes(10);
        manifest["modified"] = serde_json::json!(old.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
        fs::write(&file, serde_json::to_string(&manifest).unwrap()).unwrap();
    }

    fn edit_body(dir: &Path, text: &str) {
        // The editor's flow: change the in-memory model, then save — so the
        // disk still holds the previous state when the save snapshots it.
        let mut project = load_project(dir).unwrap();
        if let crate::model::Items::Sections(sections) = &mut project.items {
            sections[0].blocks.last_mut().unwrap().md = text.to_string();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        save_project(&project).unwrap();
    }

    #[test]
    fn each_change_keeps_the_state_it_replaced() {
        let root = scratch("trail");
        let project = create_project(&root, ProjectType::Doc, "기록", false).unwrap();
        let dir = project.dir.clone();

        // The creating save had nothing to keep.
        assert!(list(&dir).unwrap().is_empty());

        // Each edit keeps the state it replaced.
        edit_body(&dir, "첫 번째 판");
        let one = list(&dir).unwrap();
        assert_eq!(one.len(), 1, "{one:?}");
        assert_eq!(one[0].title, "기록");

        age_latest_snapshot(&dir);
        edit_body(&dir, "두 번째 판");
        let two = list(&dir).unwrap();
        assert_eq!(two.len(), 2);
        // The newest snapshot holds the first edition the second edit replaced.
        let kept = fs::read_to_string(
            dir.join(HISTORY_DIR)
                .join(&two[0].name)
                .join(first_md(&dir).strip_prefix(&dir).unwrap()),
        )
        .unwrap();
        assert!(kept.contains("첫 번째 판"), "{kept}");

        // Saving the same content twice does not stack duplicates…
        age_latest_snapshot(&dir);
        std::thread::sleep(std::time::Duration::from_millis(5));
        save_project(&load_project(&dir).unwrap()).unwrap();
        let after_noop = list(&dir).unwrap().len();
        age_latest_snapshot(&dir);
        std::thread::sleep(std::time::Duration::from_millis(5));
        save_project(&load_project(&dir).unwrap()).unwrap();
        assert_eq!(list(&dir).unwrap().len(), after_noop);

        // …and a burst of rapid saves folds into the snapshot before it,
        // instead of pushing yesterday's version off the end of the trail.
        for i in 0..5 {
            edit_body(&dir, &format!("연타 {i}"));
        }
        assert_eq!(list(&dir).unwrap().len(), after_noop + 1);

        // Restoring the first edition brings it back, and the restore itself
        // kept the state it replaced.
        let before = list(&dir).unwrap();
        restore(&dir, ProjectType::Doc, &two[0].name).unwrap();
        save_project(&load_project(&dir).unwrap()).unwrap();
        let text = fs::read_to_string(first_md(&dir)).unwrap();
        assert!(text.contains("첫 번째 판"), "{text}");
        assert!(list(&dir).unwrap().len() >= before.len());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_snapshot_name_cannot_escape_the_history_dir() {
        let root = scratch("escape");
        let project = create_project(&root, ProjectType::Doc, "보안", false).unwrap();
        for name in ["../../etc", "..", "a/b", "a\\b", ""] {
            assert!(
                restore(&project.dir, ProjectType::Doc, name).is_err(),
                "{name} should be refused"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_trail_is_bounded() {
        let root = scratch("cap");
        let project = create_project(&root, ProjectType::Doc, "상한", false).unwrap();
        let dir = project.dir.clone();
        for i in 0..(HISTORY_KEEP + 5) {
            let md = first_md(&dir);
            fs::write(&md, format!("# 상한\n\n{i}판\n")).unwrap();
            // Distinct timestamps: the snapshot name is the save time.
            std::thread::sleep(std::time::Duration::from_millis(2));
            save_project(&load_project(&dir).unwrap()).unwrap();
            age_latest_snapshot(&dir);
        }
        assert!(list(&dir).unwrap().len() <= HISTORY_KEEP);
        let _ = fs::remove_dir_all(&root);
    }
}
