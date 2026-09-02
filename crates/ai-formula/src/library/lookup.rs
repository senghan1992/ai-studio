//! Lookups, and the array shaping functions that go with them.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::functions::make_criteria;
use crate::refs::{index_to_col, to_ref};
use crate::values::{
    compare_values, err, first_error, flatten, num_of, to_text, RangeValue, Value, NA_ERR, NUM_ERR,
    REF_ERR, VALUE_ERR,
};

pub const NAMES: &[&str] = &[
    "ADDRESS",
    "CHOOSE",
    "FILTER",
    "HYPERLINK",
    "LOOKUP",
    "SEQUENCE",
    "SORT",
    "SWITCH",
    "TRANSPOSE",
    "UNIQUE",
    "XLOOKUP",
    "XMATCH",
];

/// A range or array as a flat list, which is what every function here wants.
fn list(v: &Value) -> Vec<Value> {
    flatten(std::slice::from_ref(v))
}

/// An array result. Nothing here spills into neighbouring cells — the format has
/// one value per cell — so a multi-value result is usable inside an aggregate and
/// shows its first value on its own.
fn array(values: Vec<Value>) -> Value {
    Value::Array(values)
}

/// Find `needle` in `haystack` the way XLOOKUP/XMATCH do.
fn search(needle: &Value, haystack: &[Value], mode: f64) -> Option<usize> {
    let exact = |v: &Value| compare_values(v, needle) == Ordering::Equal;
    match mode as i64 {
        // Exact, or the next smaller / larger item.
        -1 => {
            let mut best: Option<usize> = None;
            for (i, v) in haystack.iter().enumerate() {
                if exact(v) {
                    return Some(i);
                }
                if compare_values(v, needle) == Ordering::Less
                    && best
                        .map(|b| compare_values(v, &haystack[b]) == Ordering::Greater)
                        .unwrap_or(true)
                {
                    best = Some(i);
                }
            }
            best
        }
        1 => {
            let mut best: Option<usize> = None;
            for (i, v) in haystack.iter().enumerate() {
                if exact(v) {
                    return Some(i);
                }
                if compare_values(v, needle) == Ordering::Greater
                    && best
                        .map(|b| compare_values(v, &haystack[b]) == Ordering::Less)
                        .unwrap_or(true)
                {
                    best = Some(i);
                }
            }
            best
        }
        // 2 is wildcard matching; 0 and anything else is exact.
        2 => {
            let test = make_criteria(needle);
            haystack.iter().position(&*test)
        }
        _ => haystack.iter().position(exact),
    }
}

pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if !NAMES.contains(&name) {
        return None;
    }
    // IFERROR-style functions aside, an error argument is the answer — except in
    // XLOOKUP, whose `if_not_found` exists to replace one.
    if !matches!(name, "XLOOKUP" | "SWITCH") {
        if let Some(e) = first_error(args) {
            return Some(e);
        }
    }
    Some(match name {
        "CHOOSE" => match num_of(args.first().unwrap_or(&Value::Blank)) {
            Err(e) => e,
            Ok(index) => {
                let i = index.trunc();
                if i < 1.0 || i as usize >= args.len() {
                    return Some(err(VALUE_ERR));
                }
                args[i as usize].clone()
            }
        },
        "SWITCH" => {
            let subject = args.first().cloned().unwrap_or(Value::Blank);
            let rest = &args[1.min(args.len())..];
            let mut i = 0;
            while i + 1 < rest.len() {
                if compare_values(&subject, &rest[i]) == Ordering::Equal {
                    return Some(rest[i + 1].clone());
                }
                i += 2;
            }
            // A trailing odd argument is the default.
            match rest.len() % 2 {
                1 => rest[rest.len() - 1].clone(),
                _ => err(NA_ERR),
            }
        }
        "LOOKUP" => {
            // The two-argument vector form searches the first row/column of an
            // array and returns from its last.
            let needle = args.first().cloned().unwrap_or(Value::Blank);
            let haystack = list(args.get(1).unwrap_or(&Value::Blank));
            let results = args.get(2).map(list).unwrap_or_else(|| haystack.clone());
            // LOOKUP assumes sorted data and takes the last value not greater
            // than the needle.
            let mut found = None;
            for (i, v) in haystack.iter().enumerate() {
                if compare_values(v, &needle) != Ordering::Greater {
                    found = Some(i);
                } else {
                    break;
                }
            }
            match found.and_then(|i| results.get(i)) {
                Some(v) => v.clone(),
                None => err(NA_ERR),
            }
        }
        "XLOOKUP" => {
            let needle = args.first().cloned().unwrap_or(Value::Blank);
            let haystack = list(args.get(1).unwrap_or(&Value::Blank));
            let results = list(args.get(2).unwrap_or(&Value::Blank));
            let mode = args.get(4).map(|v| num_of(v).unwrap_or(0.0)).unwrap_or(0.0);
            let reverse = args
                .get(5)
                .map(|v| num_of(v).unwrap_or(1.0) < 0.0)
                .unwrap_or(false);
            let mut order: Vec<usize> = (0..haystack.len()).collect();
            if reverse {
                order.reverse();
            }
            let ordered: Vec<Value> = order.iter().map(|i| haystack[*i].clone()).collect();
            match search(&needle, &ordered, mode).map(|i| order[i]) {
                Some(i) => results.get(i).cloned().unwrap_or(Value::Blank),
                // `if_not_found` is the fourth argument, and its whole purpose is
                // replacing the #N/A that IFERROR used to wrap.
                None => match args.get(3) {
                    Some(Value::Blank) | None => err(NA_ERR),
                    Some(v) => v.clone(),
                },
            }
        }
        "XMATCH" => {
            let needle = args.first().cloned().unwrap_or(Value::Blank);
            let haystack = list(args.get(1).unwrap_or(&Value::Blank));
            let mode = args.get(2).map(|v| num_of(v).unwrap_or(0.0)).unwrap_or(0.0);
            match search(&needle, &haystack, mode) {
                Some(i) => Value::Number(i as f64 + 1.0),
                None => err(NA_ERR),
            }
        }
        "UNIQUE" => {
            let mut out: Vec<Value> = Vec::new();
            for v in list(args.first().unwrap_or(&Value::Blank)) {
                if !out
                    .iter()
                    .any(|seen| compare_values(seen, &v) == Ordering::Equal)
                {
                    out.push(v);
                }
            }
            array(out)
        }
        "SORT" => {
            let mut values = list(args.first().unwrap_or(&Value::Blank));
            let descending = args
                .get(2)
                .map(|v| num_of(v).unwrap_or(1.0) < 0.0)
                .unwrap_or(false);
            values.sort_by(|a, b| {
                let c = compare_values(a, b);
                if descending {
                    c.reverse()
                } else {
                    c
                }
            });
            array(values)
        }
        "FILTER" => {
            let values = list(args.first().unwrap_or(&Value::Blank));
            let mask = list(args.get(1).unwrap_or(&Value::Blank));
            if values.len() != mask.len() {
                return Some(err(VALUE_ERR));
            }
            let kept: Vec<Value> = values
                .into_iter()
                .zip(&mask)
                .filter(|(_, keep)| matches!(crate::values::to_boolean(keep), Value::Bool(true)))
                .map(|(v, _)| v)
                .collect();
            if kept.is_empty() {
                // Excel's own answer when nothing matches and no fallback was
                // given.
                return Some(match args.get(2) {
                    Some(v) if !matches!(v, Value::Blank) => v.clone(),
                    // Excel says #CALC! here; this engine has no such code, and
                    // #N/A is the closest thing it can show.
                    _ => err(NA_ERR),
                });
            }
            array(kept)
        }
        "SEQUENCE" => {
            let rows = match num_of(args.first().unwrap_or(&Value::Number(1.0))) {
                Err(e) => return Some(e),
                Ok(n) => n.trunc().max(0.0),
            };
            let cols = args
                .get(1)
                .map(|v| num_of(v).unwrap_or(1.0).trunc().max(0.0))
                .unwrap_or(1.0);
            let start = args.get(2).map(|v| num_of(v).unwrap_or(1.0)).unwrap_or(1.0);
            let step = args.get(3).map(|v| num_of(v).unwrap_or(1.0)).unwrap_or(1.0);
            let count = rows * cols;
            if count <= 0.0 || count > 100_000.0 {
                return Some(err(NUM_ERR));
            }
            array(
                (0..count as usize)
                    .map(|i| Value::Number(start + step * i as f64))
                    .collect(),
            )
        }
        "TRANSPOSE" => match args.first() {
            Some(Value::Range(r)) => {
                let grid = r.grid();
                let mut cells = Vec::with_capacity(r.cells.len());
                for col in 0..r.cols {
                    for row in 0..r.rows {
                        let v = grid
                            .get(row)
                            .and_then(|line| line.get(col))
                            .cloned()
                            .unwrap_or(Value::Blank);
                        cells.push((to_ref(row, col), v));
                    }
                }
                Value::Range(Rc::new(RangeValue {
                    cells,
                    rows: r.cols,
                    cols: r.rows,
                }))
            }
            Some(other) => other.clone(),
            None => err(VALUE_ERR),
        },
        "ADDRESS" => {
            let (Ok(row), Ok(col)) = (
                num_of(args.first().unwrap_or(&Value::Blank)),
                num_of(args.get(1).unwrap_or(&Value::Blank)),
            ) else {
                return Some(err(VALUE_ERR));
            };
            if row < 1.0 || col < 1.0 {
                return Some(err(VALUE_ERR));
            }
            // `abs_num`: 1 absolute, 2 absolute row, 3 absolute column, 4 relative.
            let kind = args
                .get(2)
                .map(|v| num_of(v).unwrap_or(1.0).trunc())
                .unwrap_or(1.0) as i64;
            let (row_mark, col_mark) = match kind {
                2 => ("$", ""),
                3 => ("", "$"),
                4 => ("", ""),
                _ => ("$", "$"),
            };
            let letters = index_to_col(col.trunc() as usize - 1);
            let local = format!("{col_mark}{letters}{row_mark}{}", row.trunc() as i64);
            match args.get(4).map(to_text).filter(|s| !s.is_empty()) {
                Some(sheet) => Value::Text(format!("{sheet}!{local}")),
                None => Value::Text(local),
            }
        }
        // The label, which is what a spreadsheet shows in the cell.
        "HYPERLINK" => match args.get(1) {
            Some(v) if !matches!(v, Value::Blank) => Value::Text(to_text(v)),
            _ => Value::Text(to_text(args.first().unwrap_or(&Value::Blank))),
        },
        _ => return None,
    })
}

/// `REF!` for the reference functions that live in the evaluator, re-exported so
/// callers do not need the values module.
pub fn ref_error() -> Value {
    err(REF_ERR)
}
