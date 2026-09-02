//! Reading existing Office files into AI Studio projects.
//!
//! The conversion is one-way on purpose. A `.pptx` opened here becomes a
//! `.aideck` folder: markdown for the content, JSON for the geometry, and an
//! `AI.md` beside them. Editing it in place and saving back as `.pptx` would
//! give up everything the format exists for — so the original file is left
//! untouched and the export path is how you get one back.

pub mod docx;
pub mod ooxml;
pub mod pptx;
pub mod xlsx;

use ai_format::model::{Items, ProjectType};

pub use ooxml::{Error, Package, Result};

/// A media file lifted out of the source package.
#[derive(Debug)]
pub struct Asset {
    /// The name to write into the project's `assets/` folder.
    pub name: String,
    pub bytes: Vec<u8>,
    /// The part it came from, so the same image is stored once.
    pub source: String,
}

/// What could not be carried across.
///
/// Reported rather than swallowed: a user who sees "그라데이션 채우기는 첫
/// 색으로 단순화했습니다" knows what to look at, where silence would leave them
/// hunting for the difference.
#[derive(Debug, Default)]
pub struct Warnings {
    notes: Vec<String>,
    /// Families the source asked for, gathered rather than reported one by one:
    /// a real deck names four or five, and five separate warnings would read as
    /// five separate problems instead of one deliberate rule.
    fonts: std::collections::BTreeSet<String>,
}

impl Warnings {
    pub fn note(&mut self, message: &str) {
        if !self.notes.iter().any(|m| m == message) {
            self.notes.push(message.to_string());
        }
    }

    /// A font the source file asked for.
    pub fn saw_font(&mut self, name: &str) {
        if ai_format::font::is_substitution(name) {
            self.fonts.insert(name.trim().to_string());
        }
    }

    pub fn into_vec(mut self) -> Vec<String> {
        if !self.fonts.is_empty() {
            let shown: Vec<&str> = self.fonts.iter().take(5).map(String::as_str).collect();
            let rest = self.fonts.len().saturating_sub(shown.len());
            let list = match rest {
                0 => shown.join(", "),
                n => format!("{} 외 {n}개", shown.join(", ")),
            };
            self.notes.push(format!(
                "글꼴은 모두 {}로 바꿔 열었습니다 ({list})",
                ai_format::font::FAMILY
            ));
        }
        self.notes
    }
}

/// A file read into a project, before it is written to disk.
#[derive(Debug)]
pub struct Imported {
    pub project_type: ProjectType,
    pub title: String,
    pub items: Items,
    pub assets: Vec<Asset>,
    pub warnings: Vec<String>,
}

/// Read an Office file. `filename` decides which reader runs.
pub fn read(bytes: &[u8], filename: &str) -> Result<Imported> {
    let extension = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    // The format check comes before opening the package: a legacy `.ppt` is not
    // a zip at all, and "invalid Zip archive" tells the user nothing they can
    // act on where "Office에서 .pptx로 저장하세요" tells them exactly what to do.
    if matches!(extension.as_str(), "ppt" | "doc" | "xls") {
        return Err(Error::UnsupportedFormat(format!(
            ".{extension}는 2007년 이전 형식입니다. Office에서 .{extension}x로 저장한 뒤 열어 주세요"
        )));
    }
    if !matches!(
        extension.as_str(),
        "pptx" | "pptm" | "potx" | "xlsx" | "xlsm" | "xltx" | "docx" | "docm" | "dotx"
    ) {
        return Err(Error::UnsupportedFormat(format!(
            ".{extension} — pptx · docx · xlsx만 열 수 있습니다"
        )));
    }

    let title = title_from(filename);
    let package = Package::open(bytes)?;
    let mut warnings = Warnings::default();
    // Every reader replaces the file's fonts with the one this format draws, so
    // the substitution is reported once, here, rather than three times over.
    for name in ooxml::font_names(&package) {
        warnings.saw_font(&name);
    }

    match extension.as_str() {
        "pptx" | "pptm" | "potx" => {
            let deck = pptx::read(&package, &mut warnings)?;
            Ok(Imported {
                project_type: ProjectType::Deck,
                title,
                items: Items::Slides(deck.slides),
                assets: deck.assets,
                warnings: warnings.into_vec(),
            })
        }
        "docx" | "docm" | "dotx" => {
            let document = docx::read(&package, &mut warnings)?;
            Ok(Imported {
                project_type: ProjectType::Doc,
                title,
                items: Items::Sections(document.sections),
                assets: document.assets,
                warnings: warnings.into_vec(),
            })
        }
        "xlsx" | "xlsm" | "xltx" => {
            let sheets = xlsx::read_with(&package, &mut warnings)?;
            // A formula this build cannot evaluate keeps both its text and the
            // value the file cached, so nothing is lost — but the reader should
            // know which cells will not update when they edit the sheet.
            let mut unsupported: Vec<String> = Vec::new();
            for sheet in &sheets {
                for cell in sheet.cells.values() {
                    for name in cell
                        .f
                        .as_deref()
                        .map(ai_formula::functions::unsupported_functions)
                        .unwrap_or_default()
                    {
                        if !unsupported.contains(&name) {
                            unsupported.push(name);
                        }
                    }
                }
            }
            if !unsupported.is_empty() {
                unsupported.sort();
                warnings.note(&format!(
                    "함수 {}는 계산하지 않습니다 (수식과 저장된 값은 그대로 둡니다)",
                    unsupported.join(", ")
                ));
            }
            Ok(Imported {
                project_type: ProjectType::Grid,
                title,
                items: Items::Sheets(sheets),
                assets: Vec::new(),
                warnings: warnings.into_vec(),
            })
        }
        other => Err(Error::UnsupportedFormat(format!(".{other}"))),
    }
}

/// The document title, from the filename with its extension removed.
fn title_from(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    let trimmed = stem.trim();
    if trimmed.is_empty() {
        "가져온 문서".to_string()
    } else {
        trimmed.to_string()
    }
}

/// An asset name not already taken, keeping the original where possible.
pub(crate) fn unique_asset_name(base: &str, taken: &[Asset]) -> String {
    let exists = |name: &str| taken.iter().any(|a| a.name == name);
    if !exists(base) {
        return base.to_string();
    }
    let (stem, extension) = match base.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (base, String::new()),
    };
    for i in 2..10_000 {
        let candidate = format!("{stem}-{i}{extension}");
        if !exists(&candidate) {
            return candidate;
        }
    }
    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_comes_from_the_filename() {
        assert_eq!(title_from("2026 사업 리뷰.pptx"), "2026 사업 리뷰");
        assert_eq!(title_from("/a/b/예산.xlsx"), "예산");
        assert_eq!(title_from("C:\\Users\\me\\보고서.docx"), "보고서");
        assert_eq!(title_from(".pptx"), "가져온 문서");
    }

    #[test]
    fn the_old_binary_formats_say_what_to_do() {
        let error = read(b"not a zip", "old.ppt").unwrap_err();
        assert!(error.to_string().contains(".pptx"), "{error}");
    }

    #[test]
    fn asset_names_do_not_collide() {
        let taken = vec![Asset {
            name: "image1.png".into(),
            bytes: Vec::new(),
            source: "a".into(),
        }];
        assert_eq!(unique_asset_name("image2.png", &taken), "image2.png");
        assert_eq!(unique_asset_name("image1.png", &taken), "image1-2.png");
    }

    #[test]
    fn warnings_are_deduplicated() {
        let mut warnings = Warnings::default();
        warnings.note("같은 경고");
        warnings.note("같은 경고");
        warnings.note("다른 경고");
        assert_eq!(warnings.into_vec().len(), 2);
    }
}
