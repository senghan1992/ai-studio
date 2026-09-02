//! Read a `.pptx` and print the deck it became.
//!
//!     cargo run -p ai-import --example probe-deck -- some.pptx

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe-deck <file>");
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

    let ai_format::model::Items::Slides(slides) = &imported.items else {
        println!("(덱이 아닙니다)");
        return;
    };

    for (i, slide) in slides.iter().enumerate() {
        println!(
            "\n── 슬라이드 {} “{}”  캔버스 {}x{} bg={}",
            i + 1,
            slide.title,
            slide.canvas.w,
            slide.canvas.h,
            slide.canvas.bg.as_str()
        );
        if !slide.notes.is_empty() {
            println!("   노트: {}", slide.notes);
        }
        for b in &slide.blocks {
            let shape = b
                .shape
                .as_ref()
                .map(|s| {
                    format!(
                        "  [{} fill={:?} line={:?} rot={}]",
                        s.preset,
                        s.fill.as_ref().map(|f| f.color.clone()),
                        s.line.as_ref().map(|l| (l.color.clone(), l.dash)),
                        s.rotation
                    )
                })
                .unwrap_or_default();
            let table = b
                .table
                .as_ref()
                .map(|t| format!("  [cols={:?} merges={:?}]", t.cols, t.merges))
                .unwrap_or_default();
            let style = if b.style.is_empty() {
                String::new()
            } else {
                format!("  style={}", serde_json::to_string(&b.style).unwrap())
            };
            println!(
                "   {:<6} {:>5.0},{:>5.0} {:>5.0}x{:>4.0}{shape}{table}{style}",
                b.kind.as_str(),
                b.x,
                b.y,
                b.w,
                b.h
            );
            for line in b.md.lines().take(4) {
                println!("          | {line}");
            }
        }
    }
}
