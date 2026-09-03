//! Cell addresses, ranges, and rewriting references when the grid shifts.

use once_cell::sync::Lazy;
use regex::Regex;

const A: u32 = b'A' as u32;

/// `'A' -> 0`, `'Z' -> 25`, `'AA' -> 26`. `None` if not a column name.
pub fn col_to_index(col: &str) -> Option<usize> {
    if col.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for c in col.chars() {
        let up = c.to_ascii_uppercase() as u32;
        if !(A..=A + 25).contains(&up) {
            return None;
        }
        n = n * 26 + (up - A) as usize + 1;
    }
    Some(n - 1)
}

/// `0 -> 'A'`, `26 -> 'AA'`
pub fn index_to_col(index: usize) -> String {
    let mut n = index + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push((A + rem as u32) as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRef {
    pub col: usize,
    pub row: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

impl CellRange {
    pub fn rows(&self) -> usize {
        self.end.row - self.start.row + 1
    }
    pub fn cols(&self) -> usize {
        self.end.col - self.start.col + 1
    }
    pub fn contains(&self, cell: CellRef) -> bool {
        cell.col >= self.start.col
            && cell.col <= self.end.col
            && cell.row >= self.start.row
            && cell.row <= self.end.row
    }
}

static REF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\$?([A-Za-z]{1,3})\$?([1-9]\d{0,6})$").unwrap());

/// Excel's grid bounds: columns A..=XFD, rows 1..=1048576. The `{1,3}`-letter /
/// 7-digit regex above admits addresses far past these (`ZZZ9999999`), and a
/// range built from such a phantom corner would ask `expand_range` to allocate
/// ~1.8e11 cells — a multi-terabyte allocation that aborts the process. A
/// reference outside the real grid is not a valid cell, so we reject it here.
pub const MAX_COLS: usize = 16_384;
pub const MAX_ROWS: usize = 1_048_576;

/// `'B12' -> { col: 1, row: 11 }` (both zero-based). `None` if not a ref, or if
/// it names a cell outside Excel's grid.
pub fn parse_ref(reference: &str) -> Option<CellRef> {
    let caps = REF_RE.captures(reference.trim())?;
    let col = col_to_index(caps.get(1)?.as_str())?;
    let row: usize = caps.get(2)?.as_str().parse().ok()?;
    if col >= MAX_COLS || row > MAX_ROWS {
        return None;
    }
    Some(CellRef { col, row: row - 1 })
}

/// `{ col, row } -> 'B12'`
pub fn to_ref(col: usize, row: usize) -> String {
    format!("{}{}", index_to_col(col), row + 1)
}

/// `'A1:B3'` normalized so `start` is top-left. `None` if invalid.
pub fn parse_range(range: &str) -> Option<CellRange> {
    let (left, right) = range.split_once(':')?;
    if right.contains(':') {
        return None;
    }
    let a = parse_ref(left)?;
    let b = parse_ref(right)?;
    Some(CellRange {
        start: CellRef {
            col: a.col.min(b.col),
            row: a.row.min(b.row),
        },
        end: CellRef {
            col: a.col.max(b.col),
            row: a.row.max(b.row),
        },
    })
}

pub fn range_to_string(r: &CellRange) -> String {
    format!(
        "{}:{}",
        to_ref(r.start.col, r.start.row),
        to_ref(r.end.col, r.end.row)
    )
}

/// The most cells we will pre-size the output vector for. `parse_ref` keeps
/// every corner inside the real grid, but a whole-sheet range (A1:XFD1048576) is
/// still ~1.7e10 cells — pre-allocating that many `String`s aborts the process.
/// We size to the real count only up to this cap and let the vector grow past it
/// on the rare genuine large range, so the hint can never itself be the crash.
const MAX_EXPAND_HINT: usize = 1 << 20;

/// Every cell address in a range, row-major.
pub fn expand_range(r: &CellRange) -> Vec<String> {
    let mut out = Vec::with_capacity(r.rows().saturating_mul(r.cols()).min(MAX_EXPAND_HINT));
    for row in r.start.row..=r.end.row {
        for col in r.start.col..=r.end.col {
            out.push(to_ref(col, row));
        }
    }
    out
}

pub fn expand_range_str(range: &str) -> Vec<String> {
    parse_range(range)
        .map(|r| expand_range(&r))
        .unwrap_or_default()
}

/// Split `Sheet2!A1` or `'2분기 실적'!A1:B4` into the sheet name and the rest.
///
/// A workbook's formulas reach across sheets constantly — a summary sheet is
/// nothing but cross-sheet references — so the sheet name travels with the
/// reference rather than being stripped somewhere and lost.
pub fn split_sheet(reference: &str) -> (Option<&str>, &str) {
    match reference.rsplit_once('!') {
        Some((sheet, rest)) => {
            let name = sheet.trim().trim_matches('\'');
            if name.is_empty() {
                (None, rest)
            } else {
                (Some(name), rest)
            }
        }
        None => (None, reference),
    }
}

/// Attach a sheet name to a local reference, if there is one.
pub fn with_sheet(sheet: Option<&str>, reference: &str) -> String {
    match sheet {
        Some(name) => format!("{name}!{reference}"),
        None => reference.to_string(),
    }
}

/// `$E$7 -> E7`. The stored key form for a cell.
///
/// A sheet name keeps its own case — sheet names are compared case-insensitively
/// but shown as written, and upper-casing 'Q3 실적' would be a different label.
pub fn bare_ref(reference: &str) -> String {
    let (sheet, local) = split_sheet(reference);
    let bare = local
        .chars()
        .filter(|c| *c != '$')
        .collect::<String>()
        .to_uppercase();
    with_sheet(sheet, &bare)
}

static ANCHORED_REF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\$?)([A-Za-z]{1,3})(\$?)([1-9]\d{0,6})$").unwrap());

/// Shift a reference by `(dc, dr)`, respecting `$` anchors.
/// Returns `#REF!` if the result falls off the sheet.
pub fn shift_ref(reference: &str, dc: i64, dr: i64) -> String {
    let Some(caps) = ANCHORED_REF_RE.captures(reference) else {
        return reference.to_string();
    };
    let col_abs = caps.get(1).unwrap().as_str();
    let col_str = caps.get(2).unwrap().as_str();
    let row_abs = caps.get(3).unwrap().as_str();
    let row_str = caps.get(4).unwrap().as_str();

    let Some(base_col) = col_to_index(col_str) else {
        return reference.to_string();
    };
    let base_row: i64 = row_str.parse::<i64>().unwrap() - 1;

    let col = if col_abs.is_empty() {
        base_col as i64 + dc
    } else {
        base_col as i64
    };
    let row = if row_abs.is_empty() {
        base_row + dr
    } else {
        base_row
    };
    if col < 0 || row < 0 {
        return crate::values::REF_ERR.to_string();
    }
    format!(
        "{col_abs}{}{row_abs}{}",
        index_to_col(col as usize),
        row + 1
    )
}

/// A reference-shaped run of characters inside a formula.
///
/// The pattern is deliberately unanchored — `SUM(B2:B4)` has to yield `B2` and
/// `B4` — so the caller checks the surrounding characters. Without that check
/// `=LOG10(A1)` parses `LOG10` as a reference and `shift_formula` corrupts the
/// function name; the JavaScript original had exactly that bug.
static REF_TOKEN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\$?)([A-Za-z]{1,3})(\$?)([1-9]\d{0,6})").unwrap());

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$' || !b.is_ascii()
}

/// Replace every genuine cell reference in `segment`, leaving function names,
/// named ranges and numbers alone.
fn map_refs<F>(segment: &str, mut f: F) -> String
where
    F: FnMut(&regex::Captures) -> String,
{
    let bytes = segment.as_bytes();
    let mut out = String::with_capacity(segment.len());
    let mut last = 0usize;

    for caps in REF_TOKEN.captures_iter(segment) {
        let m = caps.get(0).unwrap();
        let (start, end) = (m.start(), m.end());

        // Preceded by an identifier character: part of a longer name.
        let joined_left = start > 0 && is_ident_byte(bytes[start - 1]);
        // Followed by '(' it is a call; by an identifier character it is a name.
        let joined_right = end < bytes.len() && (bytes[end] == b'(' || is_ident_byte(bytes[end]));

        if joined_left || joined_right {
            continue;
        }
        out.push_str(&segment[last..start]);
        out.push_str(&f(&caps));
        last = end;
    }
    out.push_str(&segment[last..]);
    out
}

/// Apply `f` to the parts of a formula that are not inside a string literal, so
/// `="A1"` is never rewritten as a reference.
pub fn map_outside_strings<F>(src: &str, mut f: F) -> String
where
    F: FnMut(&str) -> String,
{
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match src[i..].find('"') {
            None => {
                out.push_str(&f(&src[i..]));
                break;
            }
            Some(offset) => {
                let quote = i + offset;
                out.push_str(&f(&src[i..quote]));
                let close = find_closing_quote(src, quote);
                out.push_str(&src[quote..=close]);
                i = close + 1;
            }
        }
    }
    out
}

fn find_closing_quote(src: &str, start: usize) -> usize {
    let bytes = src.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                i += 2;
                continue;
            }
            return i;
        }
        i += 1;
    }
    bytes.len() - 1
}

/// Translate every relative reference in a formula by `(dc, dr)`.
///
/// This is what makes copy/paste and the fill handle behave: dragging `=B2+C2`
/// down one row must produce `=B3+C3`, while `$B$2` stays put.
pub fn shift_formula(formula: &str, dc: i64, dr: i64) -> String {
    if !formula.starts_with('=') || (dc == 0 && dr == 0) {
        return formula.to_string();
    }
    map_outside_strings(formula, |segment| {
        map_refs(segment, |caps| {
            shift_ref(caps.get(0).unwrap().as_str(), dc, dr)
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Row,
    Col,
}

/// Rewrite the references in a formula after rows or columns are inserted or
/// removed, so `=SUM(B2:B4)` becomes `=SUM(B3:B5)` when a row is inserted above.
///
/// A reference that pointed into deleted space becomes `#REF!`, matching Excel.
/// String literals are skipped so `="A1"` is left alone.
pub fn adjust_refs(formula: &str, axis: Axis, at: i64, delta: i64) -> String {
    if !formula.starts_with('=') || delta == 0 {
        return formula.to_string();
    }
    map_outside_strings(formula, |segment| {
        map_refs(segment, |caps| {
            let col_abs = caps.get(1).unwrap().as_str();
            let col_str = caps.get(2).unwrap().as_str();
            let row_abs = caps.get(3).unwrap().as_str();
            let row_str = caps.get(4).unwrap().as_str();

            let Some(col) = col_to_index(col_str) else {
                return caps.get(0).unwrap().as_str().to_string();
            };
            let col = col as i64;
            let row = row_str.parse::<i64>().unwrap() - 1;

            let index = if axis == Axis::Row { row } else { col };
            let next = if delta > 0 {
                if index >= at {
                    index + delta
                } else {
                    index
                }
            } else if index >= at && index < at - delta {
                return crate::values::REF_ERR.to_string();
            } else if index >= at - delta {
                index + delta
            } else {
                index
            };

            if next < 0 {
                return crate::values::REF_ERR.to_string();
            }
            let next_col = if axis == Axis::Col { next } else { col };
            let next_row = if axis == Axis::Row { next } else { row };
            format!(
                "{col_abs}{}{row_abs}{}",
                index_to_col(next_col as usize),
                next_row + 1
            )
        })
    })
}

/// A spill reference as a formula writes it: `E2#`, `$E$2#`, `'2분기'!E2#`.
static SPILL_REF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"((?:'[^']+'!|[^\s!'"(),;:+\-*/^&<>=#%{}]+!)?\$?[A-Za-z]{1,3}\$?[1-9]\d{0,6})#"#)
        .unwrap()
});
/// The same reference as Excel stores it in a file.
static ANCHORARRAY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"_xlfn\.ANCHORARRAY\(\s*([^()]+?)\s*\)").unwrap());

/// The implemented functions Excel stores with an `_xlfn.` prefix.
///
/// Excel writes post-2007 functions into `.xlsx` as `_xlfn.XLOOKUP` (and the
/// worksheet-scoped pair as `_xlfn._xlws.SORT`); a bare modern name in a file
/// is treated as an unknown user function and recalculates to `#NAME?`. The
/// list covers what this engine implements — an unimplemented function passes
/// through untouched either way.
const XLFN_FUNCTIONS: &[&str] = &[
    // 2010
    "AGGREGATE",
    "STDEV.S",
    "STDEV.P",
    "VAR.S",
    "VAR.P",
    // 2013
    "DAYS",
    "ISOWEEKNUM",
    "NUMBERVALUE",
    "XOR",
    "IFNA",
    "CEILING.MATH",
    "FLOOR.MATH",
    // 2016
    "TEXTJOIN",
    "CONCAT",
    "IFS",
    "SWITCH",
    "MAXIFS",
    "MINIFS",
    // 2019 / 365
    "XLOOKUP",
    "XMATCH",
    "UNIQUE",
    "SEQUENCE",
    "TEXTBEFORE",
    "TEXTAFTER",
];
/// The two that additionally carry the `_xlws.` worksheet scope.
const XLWS_FUNCTIONS: &[&str] = &["SORT", "FILTER"];

static XLFN_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    let all: Vec<String> = XLFN_FUNCTIONS
        .iter()
        .chain(XLWS_FUNCTIONS)
        .map(|name| regex::escape(name))
        .collect();
    Regex::new(&format!(r"(?i)\b({})\s*\(", all.join("|"))).unwrap()
});
static XLFN_PREFIX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)_xlfn\.(?:_xlws\.)?([A-Za-z_][A-Za-z0-9._]*)").unwrap());

/// Add the storage prefixes on the way into a `.xlsx`. Idempotent: existing
/// prefixes are stripped first, so nothing is ever doubled.
pub fn add_xlfn_prefixes(formula: &str) -> String {
    map_outside_strings(&strip_xlfn_prefixes(formula), |segment| {
        XLFN_CALL_RE
            .replace_all(segment, |caps: &regex::Captures| {
                let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                let upper = name.to_uppercase();
                if XLWS_FUNCTIONS.contains(&upper.as_str()) {
                    format!("_xlfn._xlws.{name}(")
                } else {
                    format!("_xlfn.{name}(")
                }
            })
            .into_owned()
    })
}

/// Remove the storage prefixes on the way out of a `.xlsx`, so `_xlfn.XLOOKUP`
/// becomes the `XLOOKUP` this engine recognises — and an unimplemented
/// `_xlfn.LAMBDA` at least reads as `LAMBDA` in the warning that names it.
pub fn strip_xlfn_prefixes(formula: &str) -> String {
    map_outside_strings(formula, |segment| {
        XLFN_PREFIX_RE
            .replace_all(segment, |caps: &regex::Captures| {
                let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                // `_xlfn.ANCHORARRAY(E2)` is the storage form of `E2#`; its
                // prefix belongs to the spill-reference conversion, not here.
                if name.eq_ignore_ascii_case("ANCHORARRAY") {
                    caps.get(0)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default()
                } else {
                    name.to_string()
                }
            })
            .into_owned()
    })
}

/// `E2#` → `_xlfn.ANCHORARRAY(E2)` — the storage Excel uses for a spill
/// reference inside `.xlsx`, applied outside string literals.
pub fn spill_refs_to_anchorarray(formula: &str) -> String {
    map_outside_strings(formula, |segment| {
        SPILL_REF_RE
            .replace_all(segment, "_xlfn.ANCHORARRAY($1)")
            .into_owned()
    })
}

/// `_xlfn.ANCHORARRAY(E2)` → `E2#`, the readable form this engine evaluates.
pub fn anchorarray_to_spill_refs(formula: &str) -> String {
    map_outside_strings(formula, |segment| {
        ANCHORARRAY_RE.replace_all(segment, "$1#").into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names_round_trip() {
        assert_eq!(col_to_index("A"), Some(0));
        assert_eq!(col_to_index("Z"), Some(25));
        assert_eq!(col_to_index("AA"), Some(26));
        assert_eq!(col_to_index("BZ"), Some(77));
        assert_eq!(col_to_index("1"), None);
        for i in [0usize, 25, 26, 27, 51, 52, 701, 702, 16383] {
            assert_eq!(col_to_index(&index_to_col(i)), Some(i), "index {i}");
        }
    }

    #[test]
    fn parses_refs_and_ranges() {
        assert_eq!(parse_ref("B12"), Some(CellRef { col: 1, row: 11 }));
        assert_eq!(parse_ref("$E$7"), Some(CellRef { col: 4, row: 6 }));
        assert_eq!(parse_ref("B0"), None);
        assert_eq!(parse_ref("hello"), None);
        // Reversed corners normalize.
        let r = parse_range("C3:A1").unwrap();
        assert_eq!(range_to_string(&r), "A1:C3");
        assert_eq!(expand_range_str("A1:B2"), ["A1", "B1", "A2", "B2"]);
        assert_eq!(parse_range("A1"), None);
    }

    #[test]
    fn references_outside_the_real_grid_are_rejected() {
        // The grid's far corner parses; one step past each edge does not. A
        // crafted `A1:ZZZ9999999` range must fail to parse rather than ask
        // `expand_range` for a multi-terabyte allocation that aborts the process.
        assert!(parse_ref("XFD1048576").is_some(), "the last real cell");
        assert_eq!(parse_ref("XFE1"), None, "one column past XFD");
        assert_eq!(parse_ref("A1048577"), None, "one row past the last");
        assert_eq!(parse_ref("ZZZ9999999"), None, "the phantom corner");
        assert!(
            expand_range_str("A1:ZZZ9999999").is_empty(),
            "an out-of-grid range expands to nothing, never a huge allocation"
        );
    }

    #[test]
    fn shifting_respects_anchors() {
        assert_eq!(shift_formula("=B2+C2", 0, 1), "=B3+C3");
        assert_eq!(shift_formula("=$B$2+C2", 0, 1), "=$B$2+C3");
        assert_eq!(shift_formula("=B$2+$C2", 1, 1), "=C$2+$C3");
        assert_eq!(shift_formula("=SUM(B2:B4)", 1, 0), "=SUM(C2:C4)");
        assert_eq!(shift_formula("=A1", 0, -5), "=#REF!");
        // String literals are left alone.
        assert_eq!(shift_formula("=\"A1\"&A1", 0, 1), "=\"A1\"&A2");
    }

    #[test]
    fn a_spill_reference_shifts_with_its_anchor() {
        // Inserting a row moves E2 to E3; the `#` rides along untouched.
        assert_eq!(adjust_refs("=SUM(E2#)", Axis::Row, 0, 1), "=SUM(E3#)");
        assert_eq!(shift_formula("=COUNTA(B1#)*2", 1, 0), "=COUNTA(C1#)*2");
    }

    #[test]
    fn modern_functions_get_and_lose_their_storage_prefix() {
        assert_eq!(
            add_xlfn_prefixes("XLOOKUP(A1,B:B,C:C)+SUM(D1:D9)"),
            "_xlfn.XLOOKUP(A1,B:B,C:C)+SUM(D1:D9)"
        );
        assert_eq!(add_xlfn_prefixes("SORT(A1:A9)"), "_xlfn._xlws.SORT(A1:A9)");
        // Idempotent, and a string literal is never touched.
        assert_eq!(
            add_xlfn_prefixes("_xlfn.XLOOKUP(A1,B:B,C:C)&\"UNIQUE(\""),
            "_xlfn.XLOOKUP(A1,B:B,C:C)&\"UNIQUE(\""
        );
        assert_eq!(
            strip_xlfn_prefixes("_xlfn._xlws.SORT(_xlfn.UNIQUE(A1:A9))"),
            "SORT(UNIQUE(A1:A9))"
        );
        // The spill-reference storage form keeps its prefix for its own pass.
        assert_eq!(
            strip_xlfn_prefixes("SUM(_xlfn.ANCHORARRAY(E2))"),
            "SUM(_xlfn.ANCHORARRAY(E2))"
        );
        // A name that merely contains a modern name is not prefixed.
        assert_eq!(add_xlfn_prefixes("MYCONCAT(A1)"), "MYCONCAT(A1)");
    }

    #[test]
    fn spill_references_convert_to_and_from_excels_storage() {
        assert_eq!(
            spill_refs_to_anchorarray("SUM(E2#)+COUNTA('2분기 실적'!B1#)"),
            "SUM(_xlfn.ANCHORARRAY(E2))+COUNTA(_xlfn.ANCHORARRAY('2분기 실적'!B1))"
        );
        assert_eq!(
            anchorarray_to_spill_refs("SUM(_xlfn.ANCHORARRAY(E2))"),
            "SUM(E2#)"
        );
        // Text literals are never rewritten.
        assert_eq!(
            spill_refs_to_anchorarray("\"품번 E2#\"&E2#"),
            "\"품번 E2#\"&_xlfn.ANCHORARRAY(E2)"
        );
        // A `#` that is part of an error literal is not a spill reference.
        assert_eq!(spill_refs_to_anchorarray("#REF!"), "#REF!");
    }

    #[test]
    fn function_names_are_not_references() {
        // The JS original rewrote LOG10 -> LOG11 here.
        assert_eq!(shift_formula("=LOG10(A1)", 0, 1), "=LOG10(A2)");
        assert_eq!(adjust_refs("=LOG10(A5)", Axis::Row, 0, 1), "=LOG10(A6)");
    }

    #[test]
    fn insert_and_delete_move_references() {
        assert_eq!(adjust_refs("=SUM(B2:B4)", Axis::Row, 1, 1), "=SUM(B3:B5)");
        assert_eq!(adjust_refs("=SUM(B2:B4)", Axis::Row, 9, 1), "=SUM(B2:B4)");
        assert_eq!(adjust_refs("=B5", Axis::Row, 1, -1), "=B4");
        // Deleting the row the reference points at yields #REF!.
        assert_eq!(adjust_refs("=B5", Axis::Row, 4, -1), "=#REF!");
        assert_eq!(adjust_refs("=SUM(B2:D2)", Axis::Col, 0, 1), "=SUM(C2:E2)");
        assert_eq!(adjust_refs("=$E$7", Axis::Row, 0, 1), "=$E$8");
    }
}
