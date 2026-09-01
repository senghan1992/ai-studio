//! Load every project in a workspace, re-save it into a copy, and report which
//! bytes changed. Run against the JS-generated `workspace/` to check fidelity:
//!
//!     cargo run -p ai-format --example roundtrip -- workspace /tmp/out

use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let src = PathBuf::from(args.next().expect("usage: roundtrip <workspace> <out>"));
    let out = PathBuf::from(args.next().expect("usage: roundtrip <workspace> <out>"));

    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();

    let mut differences = 0usize;
    let mut compared = 0usize;

    for summary in ai_format::project::list_projects(&src).unwrap() {
        let dir = src.join(&summary.folder);
        let project = ai_format::project::load_project(&dir).unwrap();

        let copy_dir = out.join(&summary.folder);
        copy_tree(&dir, &copy_dir);
        let copy = ai_format::model::Project {
            dir: copy_dir.clone(),
            ..project
        };
        ai_format::project::save_project(&copy).unwrap();

        println!("\n=== {} ({})", summary.folder, summary.label);
        for file in ai_format::project::project_files(&dir).unwrap() {
            if !(file.path.ends_with(".md") || file.path.ends_with(".json")) {
                continue;
            }
            let before = std::fs::read_to_string(dir.join(&file.path)).unwrap_or_default();
            let after = std::fs::read_to_string(copy_dir.join(&file.path)).unwrap_or_default();
            compared += 1;
            if before == after {
                println!("  same     {}", file.path);
            } else {
                differences += 1;
                println!("  DIFFERS  {}", file.path);
                report_diff(&before, &after);
            }
        }
        // Files the Rust save wrote that the JS never had.
        for file in ai_format::project::project_files(&copy_dir).unwrap() {
            if !dir.join(&file.path).exists() {
                differences += 1;
                println!("  EXTRA    {}", file.path);
            }
        }
    }

    println!("\n{compared} files compared, {differences} differ");
}

fn report_diff(before: &str, after: &str) {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let mut shown = 0;
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or("<none>"),
            b.get(i).copied().unwrap_or("<none>"),
        );
        if x != y {
            println!(
                "      line {}:\n        js:   {x}\n        rust: {y}",
                i + 1
            );
            shown += 1;
            if shown >= 6 {
                println!("      …");
                return;
            }
        }
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().filter_map(|e| e.ok()) {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
