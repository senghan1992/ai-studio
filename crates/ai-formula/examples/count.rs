//! How many functions the engine has, and which.
fn main() {
    let names = &*ai_formula::functions::FUNCTION_NAMES;
    println!("{} 함수", names.len());
    for chunk in names.chunks(12) {
        println!("  {}", chunk.join(" "));
    }
}
