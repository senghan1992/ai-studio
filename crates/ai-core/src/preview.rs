//! The exact md/json a save would write, without writing it.
//!
//! This backs the `{ } 저장 포맷` panel. It shares the serializers with the real
//! save path, so what the panel shows and what lands on disk cannot disagree.

use serde::Serialize;

use ai_format::deck::write_slide;
use ai_format::digest::build_digest;
use ai_format::doc::write_section;
use ai_format::grid::write_sheet;
use ai_format::ids::{pad, slugify};
use ai_format::model::{Items, Project};

#[derive(Debug, Serialize)]
pub struct PreviewFile {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub files: Vec<PreviewFile>,
    /// `AI.md`, kept separate because the panel shows it in its own tab.
    pub digest: String,
}

pub fn of(project: &Project) -> Preview {
    let project_type = project.project_type;
    let dir = project_type.dir();
    let mut files = Vec::new();

    let names: Vec<String> = match &project.items {
        Items::Slides(v) => v.iter().map(|s| s.title.clone()).collect(),
        Items::Sections(v) => v.iter().map(|s| s.name.clone()).collect(),
        Items::Sheets(v) => v.iter().map(|s| s.name.clone()).collect(),
    };

    for (i, name) in names.iter().enumerate() {
        let stem = format!(
            "{}-{}",
            pad(i + 1, 2),
            slugify(name, &format!("item-{}", i + 1))
        );
        let (md, json) = match &project.items {
            Items::Slides(v) => {
                let out = write_slide(&v[i]);
                (out.md, out.layout)
            }
            Items::Sections(v) => {
                let out = write_section(&v[i]);
                (out.md, out.meta)
            }
            Items::Sheets(v) => {
                let out = write_sheet(&v[i]);
                (out.md, out.cells)
            }
        };
        files.push(PreviewFile {
            path: format!("{dir}/{stem}.md"),
            text: md,
        });
        files.push(PreviewFile {
            path: format!("{dir}/{stem}{}", project_type.json_suffix()),
            text: format!("{}\n", ai_format::json::to_pretty(&json)),
        });
    }

    Preview {
        files,
        digest: build_digest(project),
    }
}
