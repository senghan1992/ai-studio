//! Warnings for a file, which is what the import dialog shows.

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("readable file");
        let imported = ai_import::read(&bytes, &path).expect("importable");
        println!("── {path}");
        for warning in &imported.warnings {
            println!("   · {warning}");
        }
    }
}
