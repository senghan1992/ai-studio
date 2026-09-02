//! Evaluate formulas against a small sheet, for checking behaviour by hand.
//!
//!   cargo run -p ai-formula --example try -- '=PMT(0.05/12,360,300000000)'

use indexmap::IndexMap;

fn main() {
    let mut cells: IndexMap<String, ai_formula::evaluate::Cell> = IndexMap::new();
    for (reference, value) in [
        ("A1", serde_json::json!(10)),
        ("A2", serde_json::json!(20)),
        ("A3", serde_json::json!(30)),
        ("A4", serde_json::json!(40)),
        ("B1", serde_json::json!("서울")),
        ("B2", serde_json::json!("부산")),
        ("B3", serde_json::json!("서울")),
        ("B4", serde_json::json!("대구")),
    ] {
        cells.insert(
            reference.to_string(),
            ai_formula::evaluate::Cell {
                v: value,
                ..Default::default()
            },
        );
    }
    let names = ai_formula::evaluate::Names::new();
    for formula in std::env::args().skip(1) {
        let value = ai_formula::evaluate::evaluate_in(&formula, &cells, &names);
        println!("{formula:52} → {}", ai_formula::values::to_text(&value));
    }
}
