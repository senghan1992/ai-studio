//! Write every export of every project in a workspace, for external validation.
//!
//!     cargo run -p ai-export --example dump -- workspace /tmp/out

use ai_export::{export, suggested_filename, Format};
use ai_format::model::ProjectType;

fn main() {
    let mut args = std::env::args().skip(1);
    let src = std::path::PathBuf::from(args.next().expect("usage: dump <workspace> <out>"));
    let out = std::path::PathBuf::from(args.next().expect("usage: dump <workspace> <out>"));
    std::fs::create_dir_all(&out).unwrap();

    for summary in ai_format::project::list_projects(&src).unwrap() {
        let project = ai_format::project::load_project(&src.join(&summary.folder)).unwrap();
        let formats: &[Format] = match project.project_type {
            ProjectType::Deck => &[Format::Pptx],
            ProjectType::Doc => &[Format::Docx],
            ProjectType::Grid => &[Format::Xlsx, Format::Csv],
        };
        for format in formats {
            let bytes = export(&project, *format).unwrap();
            let name = suggested_filename(&project, *format);
            std::fs::write(out.join(&name), &bytes).unwrap();
            println!("{name}  {} bytes", bytes.len());
        }
    }
}
