//! Read a `.docx` and print the document it became.
//!
//!     cargo run -p ai-import --example probe-doc -- some.docx

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe-doc <file>");
    let bytes = std::fs::read(&path).expect("readable file");
    let name = path.rsplit('/').next().unwrap_or(&path);
    let imported = ai_import::read(&bytes, name).expect("readable package");

    println!("제목: {}", imported.title);
    if !imported.warnings.is_empty() {
        println!("경고: {:?}", imported.warnings);
    }
    if !imported.assets.is_empty() {
        let names: Vec<&str> = imported.assets.iter().map(|a| a.name.as_str()).collect();
        println!("자산: {names:?}");
    }

    let ai_format::model::Items::Sections(sections) = &imported.items else {
        println!("(문서가 아닙니다)");
        return;
    };

    for (i, section) in sections.iter().enumerate() {
        println!(
            "\n── 섹션 {} “{}”  {} {:?} 여백 {:?} 단 {}",
            i + 1,
            section.name,
            section.page.size,
            section.page.dimensions(),
            section.page.margin,
            section.page.columns
        );
        for (label, running) in [
            ("머리글", &section.page.header),
            ("바닥글", &section.page.footer),
        ] {
            if let Some(r) = running {
                println!("   {label}: [{}] [{}] [{}]", r.left, r.center, r.right);
            }
        }
        for b in &section.blocks {
            let over = b
                .format_override
                .as_ref()
                .map(|o| format!("  over={}", serde_json::to_string(o).unwrap()))
                .unwrap_or_default();
            let table = b
                .table
                .as_ref()
                .map(|t| {
                    format!(
                        "  table={{cols:{:?}, merges:{:?}, header:{}}}",
                        t.cols, t.merges, t.header_row
                    )
                })
                .unwrap_or_default();
            println!("   [{}]{over}{table}", b.block_type.as_str());
            for line in b.md.lines() {
                println!("      | {line}");
            }
        }
    }
}
