//! The rest of the function library.
//!
//! [`crate::functions`] holds the core an editor needs to feel alive — SUM, IF,
//! VLOOKUP and their neighbours. This module holds what a *workbook someone else
//! wrote* turns out to contain: the statistics, the loan maths, the date
//! arithmetic and the newer lookups. They are split by family rather than piled
//! into one file, because "which of these does Excel round differently" is a
//! question you ask a family at a time.
//!
//! Every function here is reached through one dispatch, so adding a family is
//! adding a module and a line.

pub mod date;
pub mod finance;
pub mod info;
pub mod lookup;
pub mod math;
pub mod stat;
pub mod text;

use crate::values::{err, Value, VALUE_ERR};

/// Functions the evaluator answers straight from the syntax tree, because they
/// take a *reference* rather than a value — see `evaluate::reference_call`.
///
/// They are listed here so the formula bar offers them, and answered here only
/// when the evaluator could not: `ROW(1)` names no cell.
pub const REFERENCE_NAMES: &[&str] = &["COLUMN", "INDIRECT", "OFFSET", "ROW"];

/// Names this module answers to, in no particular order — [`crate::functions`]
/// sorts the merged list.
pub fn names() -> Vec<&'static str> {
    let mut out = Vec::new();
    out.extend_from_slice(math::NAMES);
    out.extend_from_slice(text::NAMES);
    out.extend_from_slice(date::NAMES);
    out.extend_from_slice(stat::NAMES);
    out.extend_from_slice(finance::NAMES);
    out.extend_from_slice(lookup::NAMES);
    out.extend_from_slice(info::NAMES);
    out.extend_from_slice(REFERENCE_NAMES);
    out
}

/// `None` when no family here knows the name.
pub fn call(name: &str, args: &[Value]) -> Option<Value> {
    if REFERENCE_NAMES.contains(&name) {
        return Some(err(VALUE_ERR));
    }
    math::call(name, args)
        .or_else(|| text::call(name, args))
        .or_else(|| date::call(name, args))
        .or_else(|| stat::call(name, args))
        .or_else(|| finance::call(name, args))
        .or_else(|| lookup::call(name, args))
        .or_else(|| info::call(name, args))
}
