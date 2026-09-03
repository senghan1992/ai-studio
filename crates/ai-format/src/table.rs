//! Tables.
//!
//! The split is the format's own: **the cells' text is the markdown table**, and
//! everything about how it looks — column widths, merges, banding, per-cell
//! alignment — is in the JSON beside it. So a table stays greppable and an agent
//! can add a row by typing a row, while Office's formatting still survives a
//! round trip.
//!
//! Two consequences worth stating plainly:
//!
//! * A cell's text is one markdown line. A hard line break inside a cell is
//!   stored as `<br>`, which is what GitHub-flavoured markdown does too.
//! * A merged cell's text lives in its top-left cell; the covered cells are
//!   empty in the markdown. `merges` says which they are.

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use ai_formula::refs::{parse_range, range_to_string, to_ref, CellRange};

/// Office's built-in table style families, reduced to what changes the drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TableStyle {
    /// Header band plus banded body rows — Office's default for a new table.
    #[default]
    Banded,
    /// Grid lines only.
    Plain,
    /// No lines; spacing carries the structure.
    Borderless,
}

impl TableStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            TableStyle::Banded => "banded",
            TableStyle::Plain => "plain",
            TableStyle::Borderless => "borderless",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TableStyle::Banded => "줄무늬",
            TableStyle::Plain => "격자",
            TableStyle::Borderless => "테두리 없음",
        }
    }
}

/// A cell's place in a merged region.
///
/// The two `continues_*` flags matter because OOXML wants `hMerge` only on a
/// cell continuing a horizontal span and `vMerge` only on a vertical one.
/// Setting both on a single-row merge is not what a one-row merge means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub rows: usize,
    pub cols: usize,
    pub continues_row: bool,
    pub continues_col: bool,
}

impl Span {
    /// True for the top-left cell, the one that holds the text.
    pub fn is_anchor(&self) -> bool {
        !self.continues_row && !self.continues_col
    }
}

/// Per-cell formatting. Absent fields inherit from the table style.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valign: Option<String>,
    /// Background colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl CellFormat {
    pub fn is_empty(&self) -> bool {
        self.align.is_none() && self.valign.is_none() && self.fill.is_none() && self.color.is_none()
    }
}

/// The markdown's alignment row applied over a spec.
///
/// Called from both the load path and the editor path: markdown is the authority
/// for anything markdown can express, so a hand-typed `|---:|` must win wherever
/// the table came from.
pub fn apply_markdown_authority(spec: &mut TableSpec, md: &str) {
    let cells = parse_markdown_table(md);
    let rows = cells.len();

    // `|---:|` aligns the whole column, as it does in GFM and on paper: the
    // numbers under a right-aligned heading are what the alignment is for.
    // Recording only the header cell left every data row flush left in the
    // exported .pptx and .docx.
    let has_rule = md.lines().map(str::trim).any(|l| TABLE_RULE.is_match(l));
    if has_rule {
        // The rule row is the whole authority: first clear, then set, so a
        // column returned to `|---|` loses its stale alignment too.
        for format in spec.cells.values_mut() {
            format.align = None;
        }
        for (reference, format) in alignment_from_markdown(md) {
            let Some(column) = parse_first_ref(&reference) else {
                continue;
            };
            for row in 0..rows.max(1) {
                spec.cells.entry(to_ref(column, row)).or_default().align = format.align.clone();
            }
        }
    }
    spec.cells.retain(|_, format| !format.is_empty());

    let columns = cells.first().map(|r| r.len()).unwrap_or(0);
    spec.clamp_merges(columns, rows);
}

/// The zero-based column index of a cell reference like `C1`.
fn parse_first_ref(reference: &str) -> Option<usize> {
    ai_formula::refs::parse_ref(reference).map(|r| r.col)
}

/// A table's layout, stored beside its markdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableSpec {
    /// Column widths in px. Shorter than the markdown's column count means the
    /// rest share the remaining width.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cols: Vec<f64>,
    /// Row heights in px, header row first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<f64>,
    /// Merged regions as `A1:B2`, addressed like a spreadsheet with row 1 being
    /// the header row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<String>,
    #[serde(rename = "headerRow", default = "yes")]
    pub header_row: bool,
    #[serde(rename = "bandedRows", default = "yes")]
    pub banded_rows: bool,
    #[serde(rename = "firstCol", default, skip_serializing_if = "is_false")]
    pub first_col: bool,
    #[serde(default)]
    pub style: TableStyle,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub cells: IndexMap<String, CellFormat>,
}

fn yes() -> bool {
    true
}
fn is_false(v: &bool) -> bool {
    !*v
}

impl Default for TableSpec {
    fn default() -> Self {
        TableSpec {
            cols: Vec::new(),
            rows: Vec::new(),
            merges: Vec::new(),
            header_row: true,
            banded_rows: true,
            first_col: false,
            style: TableStyle::Banded,
            cells: IndexMap::new(),
        }
    }
}

impl TableSpec {
    /// Column width, falling back to an even share of `total`.
    pub fn col_width(&self, index: usize, columns: usize, total: f64) -> f64 {
        if let Some(w) = self.cols.get(index) {
            if *w > 0.0 {
                return *w;
            }
        }
        let assigned: f64 = self.cols.iter().take(columns).filter(|w| **w > 0.0).sum();
        let unassigned = columns - self.cols.iter().take(columns).filter(|w| **w > 0.0).count();
        if unassigned == 0 {
            return total / columns.max(1) as f64;
        }
        ((total - assigned) / unassigned as f64).max(24.0)
    }

    /// The merge covering a cell, if any.
    pub fn span_at(&self, col: usize, row: usize) -> Option<Span> {
        for spec in &self.merges {
            let Some(range) = parse_range(spec) else {
                continue;
            };
            if !range.contains(ai_formula::refs::CellRef { col, row }) {
                continue;
            }
            return Some(Span {
                rows: range.rows(),
                cols: range.cols(),
                continues_row: col > range.start.col,
                continues_col: row > range.start.row,
            });
        }
        None
    }

    /// Drop merges that no longer fit, after rows or columns were removed.
    pub fn clamp_merges(&mut self, columns: usize, rows: usize) {
        self.merges.retain(|spec| {
            parse_range(spec)
                .map(|r| r.end.col < columns && r.end.row < rows)
                .unwrap_or(false)
        });
    }
}

/* ------------------------------------------------------------------ markdown */

/// The alignment row under a table's header.
///
/// It must contain a dash. `|   |   |` is a row of empty cells, and a pattern
/// that accepts it deletes real data — an empty first body row would vanish.
static TABLE_RULE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\|[\s:|-]*-[\s:|-]*\|?$").unwrap());

/// A table's cells, parsed out of its markdown.
///
/// Row 0 is the header row when the table has one. Ragged rows are padded, so a
/// hand-edited table with a missing trailing `|` still opens as a rectangle.
pub fn parse_markdown_table(md: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = md
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('|') && !TABLE_RULE.is_match(l))
        .map(|l| {
            let body = l.strip_prefix('|').unwrap_or(l);
            let body = body.strip_suffix('|').unwrap_or(body);
            split_cells(body)
        })
        .collect();

    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    rows
}

/// Split a row on its separators and unescape each cell.
///
/// The cell's logical text holds a bare `|`; only the markdown representation
/// escapes it. Keeping the backslash here would make every save escape it again.
fn split_cells(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                // Only these two are escapes the table layer introduced.
                Some(next @ ('|' | '\\')) => cell.push(next),
                Some(next) => {
                    cell.push('\\');
                    cell.push(next);
                }
                None => cell.push('\\'),
            },
            '|' => out.push(std::mem::take(&mut cell).trim().to_string()),
            c => cell.push(c),
        }
    }
    out.push(cell.trim().to_string());
    out
}

/// Render cells back to a markdown table, with the alignment row `TableSpec`
/// implies so the markdown reads the same way in any viewer.
pub fn to_markdown_table(cells: &[Vec<String>], spec: &TableSpec) -> String {
    let columns = cells.iter().map(|r| r.len()).max().unwrap_or(0);
    if columns == 0 {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let row_text = |row: &Vec<String>| {
        let mut fields: Vec<String> = row.iter().map(|c| escape_cell(c)).collect();
        fields.resize(columns, String::new());
        format!("| {} |", fields.join(" | "))
    };

    let mut iter = cells.iter();
    match iter.next() {
        None => return String::new(),
        Some(first) => lines.push(row_text(first)),
    }

    // The rule row carries the per-column alignment.
    let rule: Vec<&str> = (0..columns)
        .map(|c| {
            match spec
                .cells
                .get(&to_ref(c, 0))
                .and_then(|f| f.align.as_deref())
            {
                Some("center") => ":---:",
                Some("right") => "---:",
                _ => "---",
            }
        })
        .collect();
    lines.push(format!("|{}|", rule.join("|")));

    for row in iter {
        lines.push(row_text(row));
    }
    lines.join("\n")
}

/// A cell's text, safe to sit between two `|`.
///
/// The backslash goes first, or escaping the pipe would corrupt a literal one.
fn escape_cell(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br>")
}

/// An empty table of the given size, the way Office's grid picker makes one.
///
/// Every cell is empty, including the header row — Office's inserted table has
/// no placeholder text, and seeding `**A**` would leave the user deleting it
/// before typing. The header row *looks* like one because `headerRow` says so,
/// not because its text is bold.
pub fn blank_table(columns: usize, rows: usize) -> (String, TableSpec) {
    let columns = columns.max(1);
    let rows = rows.max(1);
    let cells: Vec<Vec<String>> = vec![vec![String::new(); columns]; rows];
    let spec = TableSpec::default();
    (to_markdown_table(&cells, &spec), spec)
}

/// Read the alignment row of a markdown table into per-cell formats.
///
/// A hand-written `|---:|` should show as right-aligned rather than being lost,
/// so the markdown stays the authority for what it can express.
pub fn alignment_from_markdown(md: &str) -> IndexMap<String, CellFormat> {
    let mut out = IndexMap::new();
    let Some(rule) = md.lines().map(str::trim).find(|l| TABLE_RULE.is_match(l)) else {
        return out;
    };
    let body = rule.strip_prefix('|').unwrap_or(rule);
    let body = body.strip_suffix('|').unwrap_or(body);
    for (i, field) in body.split('|').enumerate() {
        let field = field.trim();
        let align = match (field.starts_with(':'), field.ends_with(':')) {
            (true, true) => "center",
            (false, true) => "right",
            _ => continue,
        };
        out.insert(
            to_ref(i, 0),
            CellFormat {
                align: Some(align.to_string()),
                ..CellFormat::default()
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_alignment_reaches_every_row() {
        let md = "| 지역 | 목표 |\n|---|---:|\n| 서울 | 12.0 |\n| 부산 | 8.5 |";
        let mut spec = TableSpec::default();
        apply_markdown_authority(&mut spec, md);
        for row in ["B1", "B2", "B3"] {
            assert_eq!(spec.cells[row].align.as_deref(), Some("right"), "{row}");
        }
        assert!(!spec.cells.contains_key("A1"), "left columns stay default");

        // Removing the alignment from the rule clears the stale cells.
        let mut spec = spec;
        apply_markdown_authority(&mut spec, "| 지역 | 목표 |\n|---|---|\n| 서울 | 12.0 |");
        assert!(spec
            .cells
            .get("B2")
            .and_then(|c| c.align.as_deref())
            .is_none());
    }

    #[test]
    fn markdown_round_trips() {
        let md = "| 항목 | 1분기 |\n|---|---|\n| 제품 A | 1,200 |";
        let cells = parse_markdown_table(md);
        assert_eq!(cells, vec![vec!["항목", "1분기"], vec!["제품 A", "1,200"]]);
        assert_eq!(to_markdown_table(&cells, &TableSpec::default()), md);
    }

    #[test]
    fn a_ragged_table_is_padded_into_a_rectangle() {
        let cells = parse_markdown_table("| a | b | c |\n|---|---|---|\n| 1 |");
        assert_eq!(cells[1], vec!["1", "", ""]);
    }

    #[test]
    fn a_pipe_inside_a_cell_survives_any_number_of_saves() {
        let cells = parse_markdown_table("| a \\| b | c |\n|---|---|");
        assert_eq!(cells[0], vec!["a | b", "c"], "the cell holds a bare pipe");

        // Escaping must be idempotent: the old code escaped the backslash again
        // on every save, so `a | b` drifted to `a \\| b`, `a \\\\| b`, ...
        let mut md = to_markdown_table(&cells, &TableSpec::default());
        for _ in 0..3 {
            let round = parse_markdown_table(&md);
            assert_eq!(round[0], cells[0]);
            md = to_markdown_table(&round, &TableSpec::default());
        }
        assert!(md.starts_with("| a \\| b | c |"), "{md}");
    }

    #[test]
    fn an_empty_body_row_is_not_mistaken_for_the_alignment_rule() {
        let cells = parse_markdown_table("| a | b |\n|---|---|\n|   |   |\n| 1 | 2 |");
        assert_eq!(cells.len(), 3, "the blank row is data: {cells:?}");
        assert_eq!(cells[1], vec!["", ""]);
    }

    #[test]
    fn a_line_break_inside_a_cell_becomes_br() {
        let cells = vec![vec!["첫 줄\n둘째 줄".to_string()]];
        let md = to_markdown_table(&cells, &TableSpec::default());
        assert!(md.contains("첫 줄<br>둘째 줄"), "{md}");
        // And the table is still one row per line.
        assert_eq!(md.lines().count(), 2);
    }

    #[test]
    fn alignment_travels_in_the_rule_row() {
        let mut spec = TableSpec::default();
        spec.cells.insert(
            "B1".into(),
            CellFormat {
                align: Some("right".into()),
                ..CellFormat::default()
            },
        );
        spec.cells.insert(
            "C1".into(),
            CellFormat {
                align: Some("center".into()),
                ..CellFormat::default()
            },
        );
        let md = to_markdown_table(&[vec!["a".into(), "b".into(), "c".into()]], &spec);
        assert!(md.contains("|---|---:|:---:|"), "{md}");

        // And reading it back recovers the same alignment.
        let back = alignment_from_markdown(&md);
        assert_eq!(back["B1"].align.as_deref(), Some("right"));
        assert_eq!(back["C1"].align.as_deref(), Some("center"));
        assert!(!back.contains_key("A1"), "a default column carries nothing");
    }

    #[test]
    fn merges_report_their_span_and_which_axis_continues() {
        let spec = TableSpec {
            merges: vec!["B1:C2".into()],
            ..TableSpec::default()
        };
        let anchor = spec.span_at(1, 0).unwrap();
        assert!(anchor.is_anchor());
        assert_eq!((anchor.rows, anchor.cols), (2, 2));

        // C1 continues the row; B2 continues the column.
        let right = spec.span_at(2, 0).unwrap();
        assert!(right.continues_row && !right.continues_col);
        let below = spec.span_at(1, 1).unwrap();
        assert!(!below.continues_row && below.continues_col);

        assert_eq!(spec.span_at(0, 0), None);
    }

    #[test]
    fn a_single_row_merge_continues_only_horizontally() {
        let spec = TableSpec {
            merges: vec!["A1:B1".into()],
            ..TableSpec::default()
        };
        let covered = spec.span_at(1, 0).unwrap();
        assert!(covered.continues_row);
        assert!(!covered.continues_col, "there is no row to continue");
    }

    #[test]
    fn merges_outside_the_table_are_dropped() {
        let mut spec = TableSpec {
            merges: vec!["A1:B1".into(), "D1:E1".into()],
            ..Default::default()
        };
        spec.clamp_merges(3, 2);
        assert_eq!(spec.merges, vec!["A1:B1"]);
    }

    #[test]
    fn column_widths_share_what_is_left() {
        let spec = TableSpec {
            cols: vec![200.0],
            ..TableSpec::default()
        };
        assert_eq!(spec.col_width(0, 3, 600.0), 200.0);
        assert_eq!(spec.col_width(1, 3, 600.0), 200.0, "400 split two ways");
        let even = TableSpec::default();
        assert_eq!(even.col_width(0, 4, 600.0), 150.0);
    }

    #[test]
    fn a_blank_table_looks_like_office_grid_picker() {
        let (md, spec) = blank_table(3, 2);
        assert_eq!(md.lines().count(), 3, "header + rule + one body row: {md}");
        assert!(spec.header_row, "the header row is a flag, not bold text");

        // Empty everywhere, and the empty rows survive being read back — the
        // alignment-rule pattern must not mistake `|   |   |` for the rule.
        let cells = parse_markdown_table(&md);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0], vec!["", "", ""]);
        assert_eq!(cells[1], vec!["", "", ""]);

        // And it survives a full write/read cycle at every size.
        for (columns, rows) in [(1, 1), (2, 3), (10, 8)] {
            let (md, spec) = blank_table(columns, rows);
            let table = Table::read(&md, &spec);
            assert_eq!(
                (table.columns(), table.rows()),
                (columns, rows),
                "{columns}x{rows}"
            );
        }
    }
}

/* ------------------------------------------------------------------- edits */

/// Which way a structural edit runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Row,
    Col,
}

/// A table as the editor holds it: the cells' text plus its layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub cells: Vec<Vec<String>>,
    pub spec: TableSpec,
}

impl Table {
    /// Read a table out of its markdown and layout.
    pub fn read(md: &str, spec: &TableSpec) -> Table {
        let mut cells = parse_markdown_table(md);
        let mut spec = spec.clone();
        // A table with no rows at all is not a table; give it one empty row so
        // every edit below has something to work on.
        if cells.is_empty() {
            cells.push(vec![String::new()]);
        }
        let width = cells.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
        for row in &mut cells {
            row.resize(width, String::new());
        }
        spec.clamp_merges(width, cells.len());
        Table { cells, spec }
    }

    pub fn columns(&self) -> usize {
        self.cells.first().map(|r| r.len()).unwrap_or(0)
    }

    pub fn rows(&self) -> usize {
        self.cells.len()
    }

    /// Back to the markdown + layout pair that gets stored.
    pub fn write(&self) -> (String, TableSpec) {
        let mut spec = self.spec.clone();
        spec.clamp_merges(self.columns(), self.rows());
        let md = to_markdown_table(&self.cells, &spec);
        (md, spec)
    }

    pub fn cell(&self, col: usize, row: usize) -> &str {
        self.cells
            .get(row)
            .and_then(|r| r.get(col))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Set one cell's text.
    pub fn set_cell(&mut self, col: usize, row: usize, text: &str) {
        if let Some(slot) = self.cells.get_mut(row).and_then(|r| r.get_mut(col)) {
            *slot = text.to_string();
        }
    }

    /// Insert a row or column at `at`, shifting everything after it.
    pub fn insert(&mut self, axis: Axis, at: usize) {
        match axis {
            Axis::Row => {
                let at = at.min(self.rows());
                let width = self.columns().max(1);
                self.cells.insert(at, vec![String::new(); width]);
                if at < self.spec.rows.len() {
                    // The new row inherits the height of the one it displaced,
                    // which is what Office does.
                    let height = self.spec.rows.get(at).copied().unwrap_or(0.0);
                    self.spec.rows.insert(at, height);
                }
            }
            Axis::Col => {
                let at = at.min(self.columns());
                for row in &mut self.cells {
                    row.insert(at.min(row.len()), String::new());
                }
                if at < self.spec.cols.len() {
                    let width = self.spec.cols.get(at).copied().unwrap_or(0.0);
                    self.spec.cols.insert(at, width);
                }
            }
        }
        shift_merges(&mut self.spec.merges, axis, at as i64, 1);
        shift_cell_formats(&mut self.spec.cells, axis, at as i64, 1);
        self.spec.clamp_merges(self.columns(), self.rows());
    }

    /// Delete a row or column. The last one is never removed — a table with no
    /// rows would have nothing to click on to get one back.
    pub fn delete(&mut self, axis: Axis, at: usize) -> bool {
        match axis {
            Axis::Row => {
                if self.rows() <= 1 || at >= self.rows() {
                    return false;
                }
                self.cells.remove(at);
                if at < self.spec.rows.len() {
                    self.spec.rows.remove(at);
                }
            }
            Axis::Col => {
                if self.columns() <= 1 || at >= self.columns() {
                    return false;
                }
                for row in &mut self.cells {
                    if at < row.len() {
                        row.remove(at);
                    }
                }
                if at < self.spec.cols.len() {
                    self.spec.cols.remove(at);
                }
            }
        }
        shift_merges(&mut self.spec.merges, axis, at as i64, -1);
        shift_cell_formats(&mut self.spec.cells, axis, at as i64, -1);
        self.spec.clamp_merges(self.columns(), self.rows());
        true
    }

    /// Merge a rectangle of cells.
    ///
    /// The text of every cell in it moves into the top-left one, joined by a
    /// space — Office concatenates rather than discarding, and losing a cell's
    /// content to a merge is not recoverable by undo alone once saved.
    pub fn merge(&mut self, c0: usize, r0: usize, c1: usize, r1: usize) -> bool {
        let (c0, c1) = (c0.min(c1), c0.max(c1));
        let (r0, r1) = (r0.min(r1), r0.max(r1));
        if c1 >= self.columns() || r1 >= self.rows() || (c0 == c1 && r0 == r1) {
            return false;
        }

        let mut gathered: Vec<String> = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                let text = self.cell(c, r).trim().to_string();
                if !text.is_empty() {
                    gathered.push(text);
                }
                if !(c == c0 && r == r0) {
                    self.set_cell(c, r, "");
                }
            }
        }
        self.set_cell(c0, r0, &gathered.join(" "));

        // Any merge overlapping the new one is replaced by it.
        let range = CellRange {
            start: ai_formula::refs::CellRef { col: c0, row: r0 },
            end: ai_formula::refs::CellRef { col: c1, row: r1 },
        };
        self.spec
            .merges
            .retain(|spec| parse_range(spec).is_none_or(|existing| !overlaps(&existing, &range)));
        self.spec.merges.push(range_to_string(&range));
        self.spec.merges.sort();
        true
    }

    /// Split the merge covering a cell. `false` when it is not merged.
    pub fn split(&mut self, col: usize, row: usize) -> bool {
        let before = self.spec.merges.len();
        self.spec.merges.retain(|spec| {
            parse_range(spec)
                .is_none_or(|range| !range.contains(ai_formula::refs::CellRef { col, row }))
        });
        self.spec.merges.len() != before
    }

    /// The merge covering a cell, as `(c0, r0, c1, r1)`.
    pub fn merge_at(&self, col: usize, row: usize) -> Option<(usize, usize, usize, usize)> {
        for spec in &self.spec.merges {
            let range = parse_range(spec)?;
            if range.contains(ai_formula::refs::CellRef { col, row }) {
                return Some((
                    range.start.col,
                    range.start.row,
                    range.end.col,
                    range.end.row,
                ));
            }
        }
        None
    }

    /// The cell a `Tab` from here lands on, wrapping to the next row.
    ///
    /// `None` at the very last cell, which is where Office adds a row.
    pub fn next_cell(&self, col: usize, row: usize) -> Option<(usize, usize)> {
        let mut c = col + 1;
        let mut r = row;
        if c >= self.columns() {
            c = 0;
            r += 1;
        }
        if r >= self.rows() {
            return None;
        }
        // Land on the anchor of a merge, never on a cell it covers.
        Some(self.anchor_of(c, r))
    }

    pub fn previous_cell(&self, col: usize, row: usize) -> Option<(usize, usize)> {
        let (c, r) = if col == 0 {
            if row == 0 {
                return None;
            }
            (self.columns().saturating_sub(1), row - 1)
        } else {
            (col - 1, row)
        };
        Some(self.anchor_of(c, r))
    }

    /// Move by one cell in a direction, staying inside the table.
    pub fn step(&self, col: usize, row: usize, dc: i64, dr: i64) -> (usize, usize) {
        let c = (col as i64 + dc).clamp(0, self.columns().saturating_sub(1) as i64) as usize;
        let r = (row as i64 + dr).clamp(0, self.rows().saturating_sub(1) as i64) as usize;
        self.anchor_of(c, r)
    }

    /// The top-left cell of whatever merge covers this one.
    pub fn anchor_of(&self, col: usize, row: usize) -> (usize, usize) {
        match self.merge_at(col, row) {
            Some((c0, r0, _, _)) => (c0, r0),
            None => (col, row),
        }
    }

    /// Set a per-cell format field, dropping the entry when nothing is left.
    pub fn format_cell(&mut self, col: usize, row: usize, apply: impl FnOnce(&mut CellFormat)) {
        let reference = to_ref(col, row);
        let mut format = self.spec.cells.get(&reference).cloned().unwrap_or_default();
        apply(&mut format);
        if format.is_empty() {
            self.spec.cells.shift_remove(&reference);
        } else {
            self.spec.cells.insert(reference, format);
        }
    }
}

fn overlaps(a: &CellRange, b: &CellRange) -> bool {
    a.start.col <= b.end.col
        && b.start.col <= a.end.col
        && a.start.row <= b.end.row
        && b.start.row <= a.end.row
}

/// Move merge ranges when a row or column is inserted or removed.
///
/// A merge that spanned the deleted line shrinks; one that was entirely inside
/// it disappears. This is the same problem formula references have, and getting
/// it wrong leaves merges pointing at cells that moved — which draws a table
/// with holes in it.
fn shift_merges(merges: &mut Vec<String>, axis: Axis, at: i64, delta: i64) {
    let mut out: Vec<String> = Vec::with_capacity(merges.len());
    for spec in merges.iter() {
        let Some(range) = parse_range(spec) else {
            continue;
        };
        let (mut start, mut end) = (range.start, range.end);

        let (s, e) = match axis {
            Axis::Row => (start.row as i64, end.row as i64),
            Axis::Col => (start.col as i64, end.col as i64),
        };
        let shift = |value: i64| -> Option<i64> {
            if delta > 0 {
                Some(if value >= at { value + delta } else { value })
            } else if value >= at && value < at - delta {
                None // inside the deleted span
            } else if value >= at - delta {
                Some(value + delta)
            } else {
                Some(value)
            }
        };

        let new_s = shift(s);
        let new_e = shift(e);
        let (new_s, new_e) = match (new_s, new_e) {
            // Both ends gone: the whole merge was deleted.
            (None, None) => continue,
            // One end gone: the merge shrinks to what is left.
            (None, Some(e)) => (at.min(e), e),
            (Some(s), None) => (s, (at - 1).max(s)),
            (Some(s), Some(e)) => (s, e),
        };
        if new_e <= new_s && matches!(axis, Axis::Row) && range.rows() > 1 && range.cols() == 1 {
            // A vertical-only merge reduced to one row is no longer a merge.
            continue;
        }
        if new_e <= new_s && matches!(axis, Axis::Col) && range.cols() > 1 && range.rows() == 1 {
            continue;
        }

        match axis {
            Axis::Row => {
                start.row = new_s.max(0) as usize;
                end.row = new_e.max(0) as usize;
            }
            Axis::Col => {
                start.col = new_s.max(0) as usize;
                end.col = new_e.max(0) as usize;
            }
        }
        if start.col == end.col && start.row == end.row {
            continue;
        }
        out.push(range_to_string(&CellRange { start, end }));
    }
    out.sort();
    out.dedup();
    *merges = out;
}

/// The same for per-cell formats, so a colour stays on the cell it was set on.
fn shift_cell_formats(cells: &mut IndexMap<String, CellFormat>, axis: Axis, at: i64, delta: i64) {
    let mut out: IndexMap<String, CellFormat> = IndexMap::new();
    for (reference, format) in cells.iter() {
        let Some(cell) = ai_formula::refs::parse_ref(reference) else {
            continue;
        };
        let value = match axis {
            Axis::Row => cell.row as i64,
            Axis::Col => cell.col as i64,
        };
        let next = if delta > 0 {
            if value >= at {
                value + delta
            } else {
                value
            }
        } else if value >= at && value < at - delta {
            continue; // the cell itself was deleted
        } else if value >= at - delta {
            value + delta
        } else {
            value
        };
        if next < 0 {
            continue;
        }
        let (col, row) = match axis {
            Axis::Row => (cell.col, next as usize),
            Axis::Col => (next as usize, cell.row),
        };
        out.insert(to_ref(col, row), format.clone());
    }
    *cells = out;
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn table_of(rows: &[&[&str]]) -> Table {
        Table {
            cells: rows
                .iter()
                .map(|r| r.iter().map(|c| c.to_string()).collect())
                .collect(),
            spec: TableSpec::default(),
        }
    }

    fn text(table: &Table) -> Vec<Vec<String>> {
        table.cells.clone()
    }

    #[test]
    fn reading_and_writing_round_trips() {
        let md = "| 항목 | 값 |\n|---|---:|\n| 가 | 1 |";
        let mut spec = TableSpec {
            cols: vec![200.0, 100.0],
            ..TableSpec::default()
        };
        apply_markdown_authority(&mut spec, md);

        let table = Table::read(md, &spec);
        assert_eq!(table.columns(), 2);
        assert_eq!(table.rows(), 2);
        let (back, out) = table.write();
        assert_eq!(back, md);
        assert_eq!(out.cols, vec![200.0, 100.0]);
    }

    #[test]
    fn a_table_with_no_rows_still_opens() {
        let table = Table::read("", &TableSpec::default());
        assert_eq!((table.columns(), table.rows()), (1, 1));
    }

    #[test]
    fn inserting_a_row_shifts_the_rows_below() {
        let mut table = table_of(&[&["a", "b"], &["1", "2"], &["3", "4"]]);
        table.spec.rows = vec![40.0, 30.0, 30.0];
        table.insert(Axis::Row, 1);

        assert_eq!(table.rows(), 4);
        assert_eq!(text(&table)[1], vec!["", ""]);
        assert_eq!(text(&table)[2], vec!["1", "2"]);
        // The new row takes the height of the one it pushed down.
        assert_eq!(table.spec.rows, vec![40.0, 30.0, 30.0, 30.0]);
    }

    #[test]
    fn inserting_a_column_shifts_the_widths_too() {
        let mut table = table_of(&[&["a", "b"], &["1", "2"]]);
        table.spec.cols = vec![200.0, 100.0];
        table.insert(Axis::Col, 1);

        assert_eq!(table.columns(), 3);
        assert_eq!(text(&table)[0], vec!["a", "", "b"]);
        assert_eq!(table.spec.cols, vec![200.0, 100.0, 100.0]);
    }

    #[test]
    fn the_last_row_and_column_cannot_be_deleted() {
        let mut table = table_of(&[&["only"]]);
        assert!(!table.delete(Axis::Row, 0));
        assert!(!table.delete(Axis::Col, 0));
        assert_eq!((table.columns(), table.rows()), (1, 1));
    }

    #[test]
    fn deleting_a_row_moves_the_merges_below_it() {
        let mut table = table_of(&[&["a", "b"], &["1", "2"], &["3", "4"]]);
        table.spec.merges = vec!["A3:B3".into()];
        table.delete(Axis::Row, 0);
        assert_eq!(table.spec.merges, vec!["A2:B2"]);
        assert_eq!(text(&table)[0], vec!["1", "2"]);
    }

    #[test]
    fn a_merge_inside_a_deleted_row_disappears() {
        let mut table = table_of(&[&["a", "b"], &["1", "2"]]);
        table.spec.merges = vec!["A2:B2".into()];
        table.delete(Axis::Row, 1);
        assert!(table.spec.merges.is_empty());
    }

    #[test]
    fn a_merge_spanning_a_deleted_row_shrinks() {
        let mut table = table_of(&[&["a"], &["1"], &["2"], &["3"]]);
        // A1:A3 covers three rows; deleting the middle one leaves two.
        table.spec.merges = vec!["A1:A3".into()];
        table.delete(Axis::Row, 1);
        assert_eq!(table.spec.merges, vec!["A1:A2"]);
    }

    #[test]
    fn a_vertical_merge_reduced_to_one_row_stops_being_a_merge() {
        let mut table = table_of(&[&["a"], &["1"], &["2"]]);
        table.spec.merges = vec!["A1:A2".into()];
        table.delete(Axis::Row, 1);
        assert!(table.spec.merges.is_empty(), "{:?}", table.spec.merges);
    }

    #[test]
    fn deleting_a_column_moves_the_cell_formats_with_it() {
        let mut table = table_of(&[&["a", "b", "c"]]);
        table.spec.cells.insert(
            "C1".into(),
            CellFormat {
                align: Some("right".into()),
                ..CellFormat::default()
            },
        );
        table.spec.cells.insert(
            "A1".into(),
            CellFormat {
                fill: Some("#eeeeee".into()),
                ..CellFormat::default()
            },
        );
        table.delete(Axis::Col, 1);

        assert_eq!(table.spec.cells["B1"].align.as_deref(), Some("right"));
        assert_eq!(table.spec.cells["A1"].fill.as_deref(), Some("#eeeeee"));
        assert!(!table.spec.cells.contains_key("C1"));
    }

    #[test]
    fn a_format_on_a_deleted_column_goes_away() {
        let mut table = table_of(&[&["a", "b"]]);
        table.spec.cells.insert(
            "B1".into(),
            CellFormat {
                align: Some("right".into()),
                ..CellFormat::default()
            },
        );
        table.delete(Axis::Col, 1);
        assert!(table.spec.cells.is_empty());
    }

    #[test]
    fn merging_gathers_the_text_rather_than_losing_it() {
        let mut table = table_of(&[&["가", "나"], &["1", "2"]]);
        assert!(table.merge(0, 0, 1, 1));

        assert_eq!(table.spec.merges, vec!["A1:B2"]);
        // Every cell's text ends up in the anchor; nothing is silently dropped.
        assert_eq!(table.cell(0, 0), "가 나 1 2");
        assert_eq!(table.cell(1, 0), "");
        assert_eq!(table.cell(0, 1), "");
    }

    #[test]
    fn merging_a_single_cell_is_not_a_merge() {
        let mut table = table_of(&[&["a", "b"]]);
        assert!(!table.merge(0, 0, 0, 0));
        assert!(table.spec.merges.is_empty());
    }

    #[test]
    fn a_new_merge_replaces_the_ones_it_overlaps() {
        let mut table = table_of(&[&["a", "b", "c"], &["1", "2", "3"]]);
        table.spec.merges = vec!["A1:B1".into()];
        table.merge(1, 0, 2, 1);
        assert_eq!(table.spec.merges, vec!["B1:C2"], "the old overlap is gone");
    }

    #[test]
    fn splitting_removes_the_merge_covering_a_cell() {
        let mut table = table_of(&[&["a", "b"], &["1", "2"]]);
        table.spec.merges = vec!["A1:B1".into()];
        assert!(table.split(1, 0), "any covered cell splits it");
        assert!(table.spec.merges.is_empty());
        assert!(!table.split(0, 1), "an unmerged cell has nothing to split");
    }

    #[test]
    fn tab_walks_the_cells_and_stops_at_the_end() {
        let table = table_of(&[&["a", "b"], &["1", "2"]]);
        assert_eq!(table.next_cell(0, 0), Some((1, 0)));
        assert_eq!(table.next_cell(1, 0), Some((0, 1)), "wraps to the next row");
        assert_eq!(
            table.next_cell(1, 1),
            None,
            "the last cell is where a row is added"
        );
        assert_eq!(table.previous_cell(0, 1), Some((1, 0)));
        assert_eq!(table.previous_cell(0, 0), None);
    }

    #[test]
    fn navigation_lands_on_a_merge_anchor_not_a_covered_cell() {
        let mut table = table_of(&[&["a", "b", "c"], &["1", "2", "3"]]);
        table.spec.merges = vec!["B1:C1".into()];
        // Tabbing from A1 reaches B1, the anchor. Tabbing again skips C1's text
        // slot and lands on the next row, because C1 has no editable content.
        assert_eq!(table.next_cell(0, 0), Some((1, 0)));
        assert_eq!(
            table.next_cell(1, 0),
            Some((1, 0)),
            "C1 resolves to its anchor"
        );
        assert_eq!(table.step(0, 0, 2, 0), (1, 0));
    }

    #[test]
    fn stepping_stays_inside_the_table() {
        let table = table_of(&[&["a", "b"], &["1", "2"]]);
        assert_eq!(table.step(0, 0, -1, -1), (0, 0));
        assert_eq!(table.step(1, 1, 5, 5), (1, 1));
        assert_eq!(table.step(0, 0, 0, 1), (0, 1));
    }

    #[test]
    fn formatting_a_cell_and_clearing_it_again_leaves_no_entry() {
        let mut table = table_of(&[&["a"]]);
        table.format_cell(0, 0, |f| f.align = Some("center".into()));
        assert_eq!(table.spec.cells["A1"].align.as_deref(), Some("center"));
        table.format_cell(0, 0, |f| f.align = None);
        assert!(table.spec.cells.is_empty(), "an empty format is not stored");
    }

    #[test]
    fn a_structural_edit_survives_a_write_and_read() {
        let mut table = Table::read("| a | b |\n|---|---:|\n| 1 | 2 |", &TableSpec::default());
        table.insert(Axis::Col, 1);
        table.set_cell(1, 0, "새 열");
        table.merge(0, 0, 1, 0);

        let (md, spec) = table.write();
        let back = Table::read(&md, &spec);
        assert_eq!(back.columns(), 3);
        assert_eq!(back.cell(0, 0), "a 새 열");
        assert_eq!(back.spec.merges, vec!["A1:B1"]);
    }
}
