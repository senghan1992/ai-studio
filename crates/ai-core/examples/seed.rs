//! Copy the sample projects in `templates/` into a workspace.
//!
//! The point is to have something worth reading in `AI.md` — a deck with speaker
//! notes, a doc with real structure, a sheet with a formula chain — so the
//! storage format can be judged on real content rather than placeholder text.
//!
//!     cargo run -p ai-core --example seed -- workspace
//!
//! Each folder is copied and then re-saved, which regenerates `manifest.json`
//! and `AI.md` from the markdown. That doubles as a check: if a template's
//! markdown drifts from what the parsers accept, seeding shows it immediately.

use std::path::{Path, PathBuf};

fn main() {
    let workspace = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("AI_STUDIO_WORKSPACE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("workspace"));

    let templates = repo_root().join("templates");
    if !templates.is_dir() {
        eprintln!("템플릿 폴더가 없습니다: {}", templates.display());
        std::process::exit(1);
    }
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        eprintln!("작업 폴더를 만들 수 없습니다: {e}");
        std::process::exit(1);
    }

    let mut entries: Vec<_> = std::fs::read_dir(&templates)
        .expect("templates is a directory")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut seeded = 0;
    for entry in entries {
        let source = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if ai_format::model::ProjectType::from_folder(&name).is_none() {
            continue;
        }

        let target = unique_target(&workspace, &name);
        if let Err(e) = copy_tree(&source, &target) {
            eprintln!("  {name}: 복사 실패 — {e}");
            continue;
        }

        match ai_format::project::load_project(&target)
            .and_then(|p| ai_format::project::save_project(&p))
        {
            Ok(saved) => {
                let count = match &saved.items {
                    ai_format::model::Items::Slides(v) => v.len(),
                    ai_format::model::Items::Sections(v) => v.len(),
                    ai_format::model::Items::Sheets(v) => v.len(),
                };
                println!(
                    "  {} — {} {}개",
                    target.file_name().unwrap_or_default().to_string_lossy(),
                    saved.project_type.label(),
                    count
                );
                seeded += 1;
            }
            Err(e) => eprintln!("  {name}: 저장 실패 — {e}"),
        }
    }

    println!("\n{seeded}개 문서를 만들었습니다: {}", workspace.display());
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Never overwrite a document the user already has.
fn unique_target(workspace: &Path, name: &str) -> PathBuf {
    let candidate = workspace.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(at) => (&name[..at], &name[at..]),
        None => (name, ""),
    };
    for i in 2..1000 {
        let candidate = workspace.join(format!("{stem}-{i}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    candidate
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.filter_map(|e| e.ok()) {
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
