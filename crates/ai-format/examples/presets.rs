//! Which gallery presets the web renderer draws, and which fall back to a box.
fn main() {
    for (group, presets) in ai_format::shape::gallery() {
        let names: Vec<&str> = presets.iter().map(|p| p.name).collect();
        println!("{} ({}): {}", group.label(), names.len(), names.join(" "));
    }
}
