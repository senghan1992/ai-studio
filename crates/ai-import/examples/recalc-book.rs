//! Recalculate an imported workbook the way saving does, and show what happened.

fn main() {
    let path = std::env::args().nth(1).expect("usage: recalc-book <xlsx>");
    let bytes = std::fs::read(&path).expect("readable file");
    let package = ai_import::ooxml::Package::open(&bytes).expect("a package");
    let sheets = ai_import::xlsx::read(&package).expect("readable workbook");

    for sheet in ai_format::grid::recalculated_all(&sheets) {
        println!("── {}", sheet.name);
        for (reference, cell) in &sheet.cells {
            println!(
                "   {reference:4} {:22} {}",
                ai_formula::evaluate::display_value(cell),
                cell.f.as_deref().unwrap_or("")
            );
        }
    }
}
