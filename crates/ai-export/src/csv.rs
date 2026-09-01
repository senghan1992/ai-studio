//! `.csv` — one sheet's used range, values as displayed.

use ai_format::grid::{recalculated, used_range};
use ai_format::model::Project;
use ai_formula::evaluate::display_value;
use ai_formula::refs::to_ref;

/// CSV of a single sheet's used range.
pub fn export(project: &Project, sheet_index: usize) -> String {
    let Some(sheet) = project.sheets().get(sheet_index) else {
        return String::new();
    };
    let sheet = recalculated(sheet);
    let Some(range) = used_range(&sheet.cells) else {
        return String::new();
    };

    let mut lines: Vec<String> = Vec::new();
    for r in 0..=range.max_row {
        let row: Vec<String> = (0..=range.max_col)
            .map(|c| {
                cell(
                    &sheet
                        .cells
                        .get(&to_ref(c, r))
                        .map(display_value)
                        .unwrap_or_default(),
                )
            })
            .collect();
        lines.push(row.join(","));
    }
    // Excel needs a BOM to read UTF-8 CSV correctly on Windows.
    format!("\u{FEFF}{}\r\n", lines.join("\r\n"))
}

fn cell(text: &str) -> String {
    if text.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_follows_rfc_4180() {
        assert_eq!(cell("plain"), "plain");
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("line\nbreak"), "\"line\nbreak\"");
    }
}
