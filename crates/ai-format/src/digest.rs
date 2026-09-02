//! `AI.md` — a single markdown file that flattens a whole project for an LLM.
//!
//! The design bet: a model reasons far better about "상단 중앙, 전체 폭" than
//! about `x=96 y=220 w=1088`, and better about a sheet's column summary than
//! about 500 rows of numbers. So the digest is not a dump of the source files;
//! it is a translation of them into prose a model already understands, with
//! enough structure (slide numbers, cell addresses, heading levels) to stay
//! verifiable against the source.

use ai_formula::evaluate::display_value;
use ai_formula::refs::{index_to_col, to_ref};

use crate::blocks::Kind;
use crate::chart::{
    chart_to_markdown_table, describe_chart, parse_chart_block, resolve_spec, CellSource,
};
use crate::doc::describe_override;
use crate::geometry::{position_phrase, reading_order};
use crate::grid::{summarize_columns, summary_scope, used_range};
use crate::mdblocks::{count_words, heading_level, plain_text};
use crate::model::{Items, Project, Section, Sheet, Slide};

/// Build `AI.md` for a project.
pub fn build_digest(project: &Project) -> String {
    match &project.items {
        Items::Slides(slides) => deck_digest(project, slides),
        Items::Sections(sections) => doc_digest(project, sections),
        Items::Sheets(sheets) => grid_digest(project, sheets),
    }
}

fn header(project: &Project, subtitle: &str) -> Vec<String> {
    let m = &project.manifest;
    let modified = if m.modified.len() >= 10 {
        &m.modified[..10]
    } else {
        "—"
    };
    vec![
        format!("# {}", m.title),
        String::new(),
        format!("> {subtitle} · 최종 수정 {modified}"),
        String::new(),
        "<!-- 이 파일은 저장할 때마다 자동 생성됩니다. 직접 편집하면 다음 저장 시 덮어써집니다. -->"
            .to_string(),
        String::new(),
    ]
}

fn or_untitled(title: &str) -> &str {
    if title.is_empty() {
        "(제목 없음)"
    } else {
        title
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/* -------------------------------------------------------------------- deck */

fn deck_digest(project: &Project, slides: &[Slide]) -> String {
    let mut lines = header(
        project,
        &format!("AI Studio 프레젠테이션 · 슬라이드 {}장", slides.len()),
    );

    lines.push("## 목차".into());
    lines.push(String::new());
    for (i, slide) in slides.iter().enumerate() {
        lines.push(format!(
            "{}. {} — `{}`",
            i + 1,
            or_untitled(&slide.title),
            slide.layout_name
        ));
    }
    lines.push(String::new());
    lines.push("---".into());
    lines.push(String::new());

    for (i, slide) in slides.iter().enumerate() {
        lines.push(format!(
            "## 슬라이드 {} — {}",
            i + 1,
            or_untitled(&slide.title)
        ));
        lines.push(String::new());
        lines.push(format!("- 레이아웃: `{}`", slide.layout_name));
        lines.push(format!("- 요소 {}개", slide.blocks.len()));
        if !slide.notes.trim().is_empty() {
            lines.push(format!("- 발표자 노트: {}", collapse(&slide.notes)));
        }
        lines.push(String::new());

        let ordered = reading_order(&slide.blocks, |b| b.geometry(), 40.0);
        if ordered.is_empty() {
            lines.push("_빈 슬라이드_".into());
            lines.push(String::new());
            continue;
        }

        for (bi, block) in ordered.iter().enumerate() {
            let where_ = position_phrase(&block.geometry(), &slide.canvas);
            // A shape says which shape it is: "도형(판단)" carries the meaning of
            // a flowchart node, where a bare "도형" carries none.
            let kind = match (&block.kind, &block.shape) {
                (Kind::Shape, Some(shape)) => format!("{}({})", block.kind.label(), shape.label()),
                _ => block.kind.label().to_string(),
            };
            lines.push(format!("### {}) {kind} — {where_}", bi + 1));
            lines.push(String::new());
            let content = block.md.trim();

            // A chart is unreadable as a fenced JSON blob; give the model the
            // shape and the numbers instead.
            let chart = if block.kind == Kind::Chart {
                parse_chart_block(content)
            } else {
                None
            };
            match chart {
                Some(spec) => {
                    lines.push(describe_chart(&spec));
                    lines.push(String::new());
                    lines.push(chart_to_markdown_table(&spec));
                }
                None => lines.push(if content.is_empty() {
                    "_(빈 요소)_".to_string()
                } else {
                    content.to_string()
                }),
            }
            lines.push(String::new());
        }

        lines.push("---".into());
        lines.push(String::new());
    }

    let all_text = slides
        .iter()
        .flat_map(|s| s.blocks.iter().map(|b| plain_text(&b.md)))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let element_count: usize = slides.iter().map(|s| s.blocks.len()).sum();
    let noted = slides.iter().filter(|s| !s.notes.trim().is_empty()).count();

    lines.push("## 전체 통계".into());
    lines.push(String::new());
    lines.push(format!("- 슬라이드: {}장", slides.len()));
    lines.push(format!("- 요소: {element_count}개"));
    lines.push(format!("- 본문 단어: 약 {}개", count_words(&all_text)));
    lines.push(format!("- 발표자 노트가 있는 슬라이드: {noted}장"));

    format!("{}\n", lines.join("\n"))
}

/* --------------------------------------------------------------------- doc */

fn doc_digest(project: &Project, sections: &[Section]) -> String {
    let mut lines = header(
        project,
        &format!("AI Studio 문서 · 섹션 {}개", sections.len()),
    );

    let outline: Vec<(usize, String)> = sections
        .iter()
        .flat_map(|s| s.blocks.iter())
        .filter_map(|b| {
            let level = heading_level(&b.md);
            (level > 0).then(|| (level, plain_text(&b.md)))
        })
        .collect();

    if !outline.is_empty() {
        lines.push("## 목차".into());
        lines.push(String::new());
        for (level, text) in &outline {
            lines.push(format!("{}- {text}", "  ".repeat(level.saturating_sub(1))));
        }
        lines.push(String::new());
        lines.push("---".into());
        lines.push(String::new());
    }

    for (si, section) in sections.iter().enumerate() {
        lines.push(format!("## 섹션 {} — {}", si + 1, section.name));
        lines.push(String::new());
        for block in &section.blocks {
            let md = block.md.trim();
            if md.is_empty() {
                continue;
            }
            match parse_chart_block(md) {
                Some(spec) => {
                    lines.push(describe_chart(&spec));
                    lines.push(String::new());
                    lines.push(chart_to_markdown_table(&spec));
                }
                None => lines.push(md.to_string()),
            }
            if let Some(o) = &block.format_override {
                lines.push(format!("<!-- 서식: {} -->", describe_override(o)));
            }
            lines.push(String::new());
        }
        lines.push("---".into());
        lines.push(String::new());
    }

    let text = sections
        .iter()
        .flat_map(|s| s.blocks.iter().map(|b| plain_text(&b.md)))
        .collect::<Vec<_>>()
        .join("\n");

    lines.push("## 전체 통계".into());
    lines.push(String::new());
    lines.push(format!("- 섹션: {}개", sections.len()));
    lines.push(format!("- 단어: 약 {}개", count_words(&text)));
    lines.push(format!(
        "- 글자(공백 제외): {}자",
        text.chars().filter(|c| !c.is_whitespace()).count()
    ));
    lines.push(format!("- 제목: {}개", outline.len()));

    format!("{}\n", lines.join("\n"))
}

/* -------------------------------------------------------------------- grid */

const GRID_DIGEST_ROWS: usize = 60;

fn grid_digest(project: &Project, sheets: &[Sheet]) -> String {
    // Recalculate here rather than trusting the caller's cached `v` values: the
    // digest is generated on save, and a caller may hand us cells whose formulas
    // have never been evaluated (an API client, or a freshly parsed file).
    // The whole workbook at once: a summary sheet's `=요약!B4` cannot be
    // resolved by recalculating its sheet alone.
    let sheets: Vec<Sheet> = crate::grid::recalculated_all(sheets);
    let mut lines = header(
        project,
        &format!("AI Studio 스프레드시트 · 시트 {}개", sheets.len()),
    );

    lines.push("## 시트 목록".into());
    lines.push(String::new());
    for (i, sheet) in sheets.iter().enumerate() {
        let size = match used_range(&sheet.cells) {
            Some(r) => format!("{}행 × {}열", r.max_row + 1, r.max_col + 1),
            None => "빈 시트".to_string(),
        };
        let formulas = sheet.cells.values().filter(|c| c.f.is_some()).count();
        lines.push(format!(
            "{}. **{}** — {size}, 수식 {formulas}개",
            i + 1,
            sheet.name
        ));
    }
    lines.push(String::new());
    lines.push("---".into());
    lines.push(String::new());

    for (i, sheet) in sheets.iter().enumerate() {
        lines.push(format!("## 시트 {} — {}", i + 1, sheet.name));
        lines.push(String::new());

        let Some(range) = used_range(&sheet.cells) else {
            lines.push("_빈 시트_".into());
            lines.push(String::new());
            lines.push("---".into());
            lines.push(String::new());
            continue;
        };

        lines.push(format!(
            "데이터 범위: `A1:{}`",
            to_ref(range.max_col, range.max_row)
        ));
        lines.push(String::new());

        let summary = summarize_columns(sheet);
        if !summary.is_empty() {
            lines.push("### 열 구성".into());
            lines.push(String::new());
            if let Some(scope) = summary_scope(sheet) {
                let excluded = if scope.excluded.is_empty() {
                    String::new()
                } else {
                    let rows: Vec<String> =
                        scope.excluded.iter().map(|r| format!("{r}행")).collect();
                    format!(", 집계 행 {}은 제외", rows.join("/"))
                };
                lines.push(format!(
                    "아래 통계는 {}–{}행 데이터 기준입니다{excluded}.",
                    scope.first_row, scope.last_row
                ));
                lines.push(String::new());
            }
            for col in &summary {
                let stats = match (&col.sum, &col.avg, &col.min, &col.max) {
                    (Some(sum), Some(avg), Some(min), Some(max)) => {
                        format!(" (합계 {sum}, 평균 {avg}, 범위 {min}~{max})")
                    }
                    _ => String::new(),
                };
                lines.push(format!(
                    "- `{}` **{}** — {}, 값 {}개{stats}",
                    col.col, col.header, col.col_type, col.count
                ));
            }
            lines.push(String::new());
        }

        // Records read better than a grid when there is a header row.
        let header_row: Vec<String> = (0..=range.max_col)
            .map(|c| {
                let shown = sheet
                    .cells
                    .get(&to_ref(c, 0))
                    .map(display_value)
                    .unwrap_or_default();
                if shown.is_empty() {
                    index_to_col(c)
                } else {
                    shown
                }
            })
            .collect();
        let has_header = header_row
            .iter()
            .enumerate()
            .any(|(idx, h)| *h != index_to_col(idx));

        lines.push("### 데이터".into());
        lines.push(String::new());
        let limit = range.max_row.min(GRID_DIGEST_ROWS);
        if has_header {
            for r in 1..=limit {
                let parts: Vec<String> = (0..=range.max_col)
                    .filter_map(|c| {
                        let shown = sheet
                            .cells
                            .get(&to_ref(c, r))
                            .map(display_value)
                            .unwrap_or_default();
                        (!shown.is_empty()).then(|| format!("{}={shown}", header_row[c]))
                    })
                    .collect();
                if !parts.is_empty() {
                    lines.push(format!("- {}행: {}", r + 1, parts.join(", ")));
                }
            }
        } else {
            for r in 0..=limit {
                let parts: Vec<String> = (0..=range.max_col)
                    .filter_map(|c| {
                        let shown = sheet
                            .cells
                            .get(&to_ref(c, r))
                            .map(display_value)
                            .unwrap_or_default();
                        (!shown.is_empty()).then(|| format!("{}={shown}", to_ref(c, r)))
                    })
                    .collect();
                if !parts.is_empty() {
                    lines.push(format!("- {}", parts.join(", ")));
                }
            }
        }
        if range.max_row > GRID_DIGEST_ROWS {
            lines.push(format!(
                "- _… {}개 행 생략_",
                range.max_row - GRID_DIGEST_ROWS
            ));
        }
        lines.push(String::new());

        let formulas: Vec<_> = sheet.cells.iter().filter(|(_, c)| c.f.is_some()).collect();
        if !formulas.is_empty() {
            lines.push("### 수식과 계산 결과".into());
            lines.push(String::new());
            for (reference, cell) in &formulas {
                let shown = display_value(cell);
                let shown = if shown.is_empty() {
                    "(빈 값)".to_string()
                } else {
                    shown
                };
                lines.push(format!(
                    "- `{reference}`: `{}` → **{shown}**",
                    cell.f.as_deref().unwrap_or("")
                ));
            }
            lines.push(String::new());
        }

        if !sheet.charts.is_empty() {
            lines.push("### 차트".into());
            lines.push(String::new());
            for chart in &sheet.charts {
                let resolved = resolve_spec(&chart.spec, Some(sheet as &dyn CellSource));
                lines.push(format!("- {}", describe_chart(&resolved)));
                lines.push(String::new());
                lines.push(chart_to_markdown_table(&resolved));
                lines.push(String::new());
            }
        }

        if !sheet.names.is_empty() {
            lines.push("### 이름 있는 범위".into());
            lines.push(String::new());
            for (name, target) in &sheet.names {
                let target = match target {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                lines.push(format!("- `{name}` → `{target}`"));
            }
            lines.push(String::new());
        }

        lines.push("---".into());
        lines.push(String::new());
    }

    format!("{}\n", lines.join("\n"))
}
