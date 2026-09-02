//! Round-trip a document with a header and footer through `.docx`.

fn main() {
    let dir = std::env::temp_dir().join(format!("ai-studio-running-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut project = ai_format::project::create_project(
        &dir,
        ai_format::model::ProjectType::Doc,
        "머리글",
        true,
    )
    .unwrap();
    let ai_format::model::Items::Sections(sections) = &mut project.items else {
        panic!()
    };
    sections[0].page.header = Some(ai_format::model::Running {
        center: "AI Studio 제안서".into(),
        ..Default::default()
    });
    sections[0].page.footer = Some(ai_format::model::Running {
        left: "{DATE}".into(),
        center: "{PAGE} / {PAGES}".into(),
        right: "기밀".into(),
    });
    let project = ai_format::project::save_project(&project).unwrap();
    let bytes = ai_export::export(&project, ai_export::Format::Docx).unwrap();
    std::fs::write(dir.join("out.docx"), &bytes).unwrap();

    let package = ai_import::ooxml::Package::open(&bytes).unwrap();
    let mut warnings = ai_import::Warnings::default();
    let back = ai_import::docx::read(&package, &mut warnings).unwrap();
    for section in &back.sections {
        println!("머리글: {:?}", section.page.header);
        println!("바닥글: {:?}", section.page.footer);
    }
    println!("파일: {}", dir.join("out.docx").display());
}
