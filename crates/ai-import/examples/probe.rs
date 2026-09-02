//! Read an Office file and print what the importer made of it.
//!
//!     cargo run -p ai-import --example probe -- some.xlsx

use ai_formula::evaluate::display_value;

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe <file>");
    let bytes = std::fs::read(&path).expect("readable file");
    let package = ai_import::ooxml::Package::open(&bytes).expect("an Office package");

    let sheets = ai_import::xlsx::read(&package).expect("readable workbook");
    for sheet in &sheets {
        println!(
            "\n── 시트 {}  ({} 셀, 틀고정 {:?}, 병합 {:?})",
            sheet.name,
            sheet.cells.len(),
            sheet.frozen,
            sheet.merges
        );
        if !sheet.names.is_empty() {
            println!("   이름 범위: {:?}", sheet.names);
        }
        if !sheet.col_widths.is_empty() {
            println!("   열 너비: {:?}", sheet.col_widths);
        }
        if !sheet.row_heights.is_empty() {
            println!("   행 높이: {:?}", sheet.row_heights);
        }
        let computed = ai_format::grid::recalculated(sheet);
        for (reference, cell) in computed.cells.iter().take(40) {
            let style = cell
                .extra
                .get("style")
                .map(|s| format!("  style={s}"))
                .unwrap_or_default();
            let formula = cell
                .f
                .as_deref()
                .map(|f| format!("  f={f}"))
                .unwrap_or_default();
            let fmt = cell
                .fmt
                .as_deref()
                .map(|f| format!("  fmt={f}"))
                .unwrap_or_default();
            println!(
                "   {reference:<4} {:<22} t={}{formula}{fmt}{style}",
                display_value(cell),
                cell.t.as_deref().unwrap_or("-")
            );
        }
    }
}
