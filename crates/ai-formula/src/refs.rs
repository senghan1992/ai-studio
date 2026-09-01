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

/// `'B12' -> { col: 1, row: 11 }` (both zero-based). `None` if not a ref.
pub fn parse_ref(reference: &str) -> Option<CellRef> {
    let caps = REF_RE.captures(reference.trim())?;
    let col = col_to_index(caps.get(1)?.as_str())?;
    let row: usize = caps.get(2)?.as_str().parse().ok()?;
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

/// Every cell address in a range, row-major.
pub fn expand_range(r: &CellRange) -> Vec<String> {
    let mut out = Vec::with_capacity(r.rows() * r.cols());
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

/// `$E$7 -> E7`. The stored key form for a cell.
pub fn bare_ref(reference: &str) -> String {
    reference
        .chars()
        .filter(|c| *c != '$')
        .collect::<String>()
        .to_uppercase()
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
fn map_outside_strings<F>(src: &str, mut f: F) -> String
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
