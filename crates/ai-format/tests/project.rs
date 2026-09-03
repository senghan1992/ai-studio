//! Disk-level assertions, in the spirit of `scripts/e2e.mjs`: create real
//! projects, edit them, save, and then read back what actually landed on disk.

use std::path::{Path, PathBuf};

use ai_format::model::{Items, ProjectType};
use ai_format::project::{
    create_project, delete_project, list_projects, load_project, project_files, read_project_file,
    rename_project, resolve_inside, save_project, Error,
};

struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ai-studio-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Workspace(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn a_new_deck_lands_on_disk_as_md_json_pairs() {
    let ws = Workspace::new("deck");
    let project =
        create_project(ws.path(), ProjectType::Deck, "2026 3분기 사업 리뷰", true).unwrap();

    let folder = project
        .dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(folder, "2026-3분기-사업-리뷰.aideck");

    let files: Vec<String> = project_files(&project.dir)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert!(files.contains(&"AI.md".to_string()));
    assert!(files.contains(&"manifest.json".to_string()));
    assert!(files
        .iter()
        .any(|f| f.starts_with("slides/01-") && f.ends_with(".md")));
    assert!(files.iter().any(|f| f.ends_with(".layout.json")));

    // Ordinal prefixes so `ls` reflects deck order.
    let slide_mds: Vec<&String> = files
        .iter()
        .filter(|f| f.starts_with("slides/") && f.ends_with(".md"))
        .collect();
    assert_eq!(slide_mds.len(), 2);
    assert!(slide_mds[1].starts_with("slides/02-"));
}

#[test]
fn coordinates_stay_out_of_markdown_and_text_stays_out_of_json() {
    let ws = Workspace::new("split");
    let project = create_project(ws.path(), ProjectType::Deck, "분리 검증", true).unwrap();

    for entry in &project.manifest.entries {
        let md = read(&project.dir, &entry.md);
        let json = read(&project.dir, &entry.json);

        for needle in ["\"x\":", "\"w\":", "fontSize"] {
            assert!(
                !md.contains(needle),
                "geometry leaked into {}: {md}",
                entry.md
            );
        }
        // Every text run in the markdown body must be absent from the JSON.
        for line in md
            .lines()
            .filter(|l| l.starts_with('#') || l.starts_with("- "))
        {
            let text = line.trim_start_matches(['#', '-', ' ']);
            if text.len() > 3 {
                assert!(
                    !json.contains(text),
                    "text leaked into {}: {text}",
                    entry.json
                );
            }
        }
    }
}

#[test]
fn a_grid_writes_formulas_to_json_and_values_to_markdown() {
    let ws = Workspace::new("grid");
    let project = create_project(ws.path(), ProjectType::Grid, "예산", true).unwrap();
    let entry = &project.manifest.entries[0];

    let json = read(&project.dir, &entry.json);
    assert!(json.contains("\"f\": \"=SUM(B2:B4)\""), "{json}");
    assert!(
        json.contains("\"v\": 2820"),
        "the cached value is written too: {json}"
    );

    let md = read(&project.dir, &entry.md);
    assert!(md.contains("### 수식"));
    assert!(md.contains("`B5` = `=SUM(B2:B4)` → 2,820"), "{md}");
    // Column letters and row numbers, so a model can answer by address.
    assert!(md.contains("| A | B | C | D |"), "{md}");
}

#[test]
fn ai_md_translates_pixels_into_words() {
    let ws = Workspace::new("digest");
    let project = create_project(ws.path(), ProjectType::Deck, "다이제스트", true).unwrap();
    let digest = read(&project.dir, "AI.md");

    assert!(digest.starts_with("# 다이제스트\n"));
    assert!(digest.contains("## 목차"));
    assert!(digest.contains("자동 생성됩니다"));
    assert!(digest.contains("## 슬라이드 1 —"));
    assert!(digest.contains("### 1) 텍스트 —"));
    assert!(digest.contains("## 전체 통계"));
    // No raw coordinates anywhere in the digest.
    assert!(!digest.contains("x="), "{digest}");
    // A position phrase instead.
    assert!(
        ["상단", "정중앙", "하단"]
            .iter()
            .any(|w| digest.contains(w)),
        "{digest}"
    );
}

#[test]
fn hand_edited_markdown_reopens_with_the_new_block() {
    let ws = Workspace::new("handedit");
    let project = create_project(ws.path(), ProjectType::Deck, "손편집", false).unwrap();
    let entry = project.manifest.entries[0].clone();

    let md = read(&project.dir, &entry.md);
    let edited = format!("{md}\n<!-- block:b_byhand -->\n# 손으로 추가한 블록\n");
    std::fs::write(project.dir.join(&entry.md), edited).unwrap();

    let reopened = load_project(&project.dir).unwrap();
    let Items::Slides(slides) = &reopened.items else {
        panic!()
    };
    let added = slides[0].blocks.iter().find(|b| b.id == "b_byhand");
    let added = added.expect("the hand-added block appears");
    assert_eq!(added.md, "# 손으로 추가한 블록");
    // It got a layout rather than piling up at the origin.
    assert!(added.w >= 40.0 && added.h >= 28.0);
}

#[test]
fn a_markdown_only_sheet_imports_its_table() {
    let ws = Workspace::new("mdonly");
    let project = create_project(ws.path(), ProjectType::Grid, "표만", false).unwrap();
    let entry = project.manifest.entries[0].clone();

    std::fs::remove_file(project.dir.join(&entry.json)).unwrap();
    std::fs::write(
        project.dir.join(&entry.md),
        "---\nid: sh_1\nname: 표만\n---\n\n| 이름 | 값 |\n|---|---|\n| 서울 | 100 |\n| 부산 | 200 |\n",
    )
    .unwrap();

    let reopened = load_project(&project.dir).unwrap();
    let Items::Sheets(sheets) = &reopened.items else {
        panic!()
    };
    assert_eq!(sheets[0].cells["A2"].v, serde_json::json!("서울"));
    assert_eq!(sheets[0].cells["B3"].v, serde_json::json!(200.0));
}

#[test]
fn renaming_moves_the_folder_and_keeps_the_content() {
    let ws = Workspace::new("rename");
    let project = create_project(ws.path(), ProjectType::Doc, "초안", true).unwrap();
    let folder = project
        .dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let next = rename_project(ws.path(), &folder, "최종 보고서").unwrap();
    assert_eq!(next, "최종-보고서.aidoc");
    assert!(!ws.path().join(&folder).exists());

    let moved = load_project(&ws.path().join(&next)).unwrap();
    assert_eq!(moved.manifest.title, "최종 보고서");
    assert!(read(&moved.dir, "AI.md").starts_with("# 최종 보고서"));
}

#[test]
fn saving_prunes_files_for_removed_items() {
    let ws = Workspace::new("prune");
    let mut project = create_project(ws.path(), ProjectType::Deck, "정리", true).unwrap();
    let removed = project.manifest.entries[1].clone();
    assert!(project.dir.join(&removed.md).exists());

    let Items::Slides(slides) = &mut project.items else {
        panic!()
    };
    slides.truncate(1);
    save_project(&project).unwrap();

    assert!(!project.dir.join(&removed.md).exists());
    assert!(!project.dir.join(&removed.json).exists());
    assert_eq!(load_project(&project.dir).unwrap().slides().len(), 1);
}

#[test]
fn a_second_project_with_the_same_title_gets_its_own_folder() {
    let ws = Workspace::new("dup");
    let a = create_project(ws.path(), ProjectType::Doc, "메모", false).unwrap();
    let b = create_project(ws.path(), ProjectType::Doc, "메모", false).unwrap();
    assert_ne!(a.dir, b.dir);
    assert!(b.dir.to_string_lossy().ends_with("메모-2.aidoc"));
}

#[test]
fn the_launcher_listing_is_newest_first() {
    let ws = Workspace::new("list");
    create_project(ws.path(), ProjectType::Deck, "가", false).unwrap();
    create_project(ws.path(), ProjectType::Grid, "나", false).unwrap();
    std::fs::create_dir_all(ws.path().join("not-a-project")).unwrap();

    let listed = list_projects(ws.path()).unwrap();
    assert_eq!(listed.len(), 2, "non-project folders are skipped");
    assert!(listed[0].modified >= listed[1].modified);
    assert!(listed.iter().all(|p| p.count > 0));
    assert!(listed.iter().any(|p| p.label == "스프레드시트"));
}

#[test]
fn deleting_removes_the_whole_folder() {
    let ws = Workspace::new("delete");
    let project = create_project(ws.path(), ProjectType::Doc, "삭제 대상", false).unwrap();
    let folder = project
        .dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    delete_project(ws.path(), &folder).unwrap();
    assert!(!project.dir.exists());
    assert!(list_projects(ws.path()).unwrap().is_empty());
}

/* ------------------------------------------------------- path containment */

#[test]
fn paths_cannot_escape_the_workspace() {
    let root = Path::new("/tmp/ws");
    assert!(resolve_inside(root, "a.aideck").is_ok());
    assert!(resolve_inside(root, "sub/a.aideck").is_ok());
    // `a/../b` stays inside and is fine.
    assert_eq!(resolve_inside(root, "a/../b").unwrap(), root.join("b"));

    for bad in ["../etc/passwd", "..", "a/../../etc", "/etc/passwd"] {
        assert!(
            matches!(resolve_inside(root, bad), Err(Error::Escape(_))),
            "{bad} should be refused"
        );
    }
}

#[test]
fn reading_a_project_file_is_confined_to_the_project() {
    let ws = Workspace::new("confine");
    let project = create_project(ws.path(), ProjectType::Doc, "격리", false).unwrap();
    std::fs::write(ws.path().join("secret.txt"), "비밀").unwrap();

    assert!(read_project_file(&project.dir, "AI.md")
        .unwrap()
        .contains("# 격리"));
    assert!(matches!(
        read_project_file(&project.dir, "../secret.txt"),
        Err(Error::Escape(_))
    ));
}

#[test]
fn a_folder_that_is_not_a_project_is_refused() {
    let ws = Workspace::new("notproject");
    std::fs::create_dir_all(ws.path().join("plain")).unwrap();
    assert!(matches!(
        load_project(&ws.path().join("plain")),
        Err(Error::NotAProject(_))
    ));
    assert!(matches!(
        delete_project(ws.path(), "plain"),
        Err(Error::NotAProject(_))
    ));
}

#[test]
fn a_missing_manifest_still_opens_the_project() {
    let ws = Workspace::new("nomanifest");
    let project = create_project(ws.path(), ProjectType::Grid, "매니페스트 없음", true).unwrap();
    std::fs::remove_file(project.dir.join("manifest.json")).unwrap();

    // The directory scan finds the sheets anyway.
    let reopened = load_project(&project.dir).unwrap();
    assert_eq!(reopened.manifest.title, "매니페스트-없음");
    assert_eq!(reopened.sheets().len(), 1);
    assert!(!reopened.sheets()[0].cells.is_empty());
}

#[test]
fn every_project_type_survives_a_save_load_save_cycle() {
    let ws = Workspace::new("cycle");
    for project_type in [ProjectType::Deck, ProjectType::Doc, ProjectType::Grid] {
        let project = create_project(
            ws.path(),
            project_type,
            &format!("{} 순환", project_type.as_str()),
            true,
        )
        .unwrap();
        let first: Vec<(String, String)> = project_files(&project.dir)
            .unwrap()
            .into_iter()
            .filter(|f| f.path.ends_with(".md"))
            .map(|f| (f.path.clone(), read(&project.dir, &f.path)))
            .collect();

        let reloaded = load_project(&project.dir).unwrap();
        save_project(&reloaded).unwrap();

        for (path, before) in first {
            assert_eq!(
                before,
                read(&project.dir, &path),
                "{path} changed on re-save"
            );
        }
    }
}

#[test]
fn ai_md_keeps_a_multi_line_cell_inside_its_row_bullet() {
    // A description column with line breaks, and a header that begins with `*`,
    // both straight out of a company workbook. Neither may break the markdown:
    // the row must stay one bullet and the header must not fuse with `**`.
    let ws = Workspace::new("digestcells");
    let mut project = create_project(ws.path(), ProjectType::Grid, "사전", false).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    let text = |s: &str| ai_formula::evaluate::Cell {
        v: serde_json::json!(s),
        t: Some("s".into()),
        ..Default::default()
    };
    sheets[0].cells.insert("A1".into(), text("*데이터셋 ID"));
    sheets[0].cells.insert("B1".into(), text("설명"));
    sheets[0].cells.insert("A2".into(), text("DS-001"));
    sheets[0].cells.insert(
        "B2".into(),
        text("1.설명: 고객 마스터\n2.범위: 국내\n3.주기: 일"),
    );
    let project = save_project(&project).unwrap();

    let digest = read(&project.dir, "AI.md");
    let row = digest
        .lines()
        .find(|l| l.contains("DS-001"))
        .expect("the row bullet");
    assert!(
        row.contains("1.설명: 고객 마스터 2.범위: 국내 3.주기: 일"),
        "line breaks collapse into the bullet: {row}"
    );
    assert!(digest.contains("\\*데이터셋 ID"), "{digest}");
    assert!(!digest.contains("***데이터셋"), "{digest}");
}

#[test]
fn shared_cell_styles_are_written_once_and_read_back_inline() {
    // Thousands of body cells in an imported workbook carry one identical
    // style; writing it on every cell made cells.json fifty times the xlsx.
    let ws = Workspace::new("styletable");
    let mut project = create_project(ws.path(), ProjectType::Grid, "스타일표", false).unwrap();
    let Items::Sheets(sheets) = &mut project.items else {
        panic!()
    };
    let body = serde_json::json!({ "color": "#757575", "wrap": true });
    let styled = |v: &str, style: serde_json::Value| ai_formula::evaluate::Cell {
        v: serde_json::json!(v),
        t: Some("s".into()),
        extra: [("style".to_string(), style)].into_iter().collect(),
        ..Default::default()
    };
    sheets[0].cells.insert(
        "A1".into(),
        styled("머리글", serde_json::json!({ "bold": true })),
    );
    sheets[0]
        .cells
        .insert("A2".into(), styled("동부", body.clone()));
    sheets[0]
        .cells
        .insert("A3".into(), styled("서부", body.clone()));
    let project = save_project(&project).unwrap();

    let Items::Sheets(saved) = &project.items else {
        panic!()
    };
    let json: serde_json::Value = serde_json::from_str(&read(
        &project.dir,
        saved[0]
            .file
            .as_deref()
            .unwrap()
            .replace(".md", ".cells.json")
            .as_str(),
    ))
    .unwrap();
    let table = json["styles"].as_object().expect("a style table");
    assert_eq!(
        table.len(),
        1,
        "only the shared style is interned: {table:?}"
    );
    assert_eq!(json["cells"]["A2"]["s"], json["cells"]["A3"]["s"]);
    assert!(json["cells"]["A2"].get("style").is_none());
    assert_eq!(
        json["cells"]["A1"]["style"],
        serde_json::json!({ "bold": true }),
        "a style used once stays inline"
    );

    // Loading expands the table: the model never sees `s`.
    let loaded = load_project(&project.dir).unwrap();
    let Items::Sheets(loaded) = &loaded.items else {
        panic!()
    };
    assert_eq!(loaded[0].cells["A2"].extra["style"], body);
    assert_eq!(loaded[0].cells["A3"].extra["style"], body);
    assert!(loaded[0].cells["A2"].extra.get("s").is_none());
}
