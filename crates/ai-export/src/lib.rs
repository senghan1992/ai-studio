//! Office exports.

pub mod csv;
pub mod docx;
pub mod mdruns;
pub mod ooxml;
pub mod pptx;
pub mod xlsx;

use ai_format::model::{Manifest, Project, ProjectType};

use crate::ooxml::esc;

/// The formats a project can be exported to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pptx,
    Docx,
    Xlsx,
    Csv,
}

impl Format {
    pub fn from_ext(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "pptx" => Format::Pptx,
            "docx" => Format::Docx,
            "xlsx" => Format::Xlsx,
            "csv" => Format::Csv,
            _ => return None,
        })
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Pptx => "pptx",
            Format::Docx => "docx",
            Format::Xlsx => "xlsx",
            Format::Csv => "csv",
        }
    }

    /// The project type this format applies to.
    pub fn project_type(self) -> ProjectType {
        match self {
            Format::Pptx => ProjectType::Deck,
            Format::Docx => ProjectType::Doc,
            Format::Xlsx | Format::Csv => ProjectType::Grid,
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Format::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Format::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            Format::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Format::Csv => "text/csv; charset=utf-8",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}은(는) .{1} 형식으로 내보낼 수 없습니다")]
    WrongType(&'static str, &'static str),
    #[error(transparent)]
    Package(#[from] crate::ooxml::Error),
}

/// Render a project in the given format.
pub fn export(project: &Project, format: Format) -> std::result::Result<Vec<u8>, ExportError> {
    if project.project_type != format.project_type() {
        return Err(ExportError::WrongType(
            project.project_type.label(),
            format.ext(),
        ));
    }
    Ok(match format {
        Format::Pptx => pptx::export(project)?,
        Format::Docx => docx::export(project)?,
        Format::Xlsx => xlsx::export(project)?,
        Format::Csv => csv::export(project, 0).into_bytes(),
    })
}

/// A filename for a download, e.g. `2026 예산.xlsx`.
pub fn suggested_filename(project: &Project, format: Format) -> String {
    let title = if project.manifest.title.is_empty() {
        "ai-studio"
    } else {
        &project.manifest.title
    };
    format!("{title}.{}", format.ext())
}

/// `docProps/core.xml`, shared by every format.
pub fn core_properties(manifest: &Manifest) -> String {
    format!(
        "<cp:coreProperties \
xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
xmlns:dcterms=\"http://purl.org/dc/terms/\" \
xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
<dc:title>{}</dc:title>\
<dc:creator>AI Studio</dc:creator>\
<cp:lastModifiedBy>AI Studio</cp:lastModifiedBy>\
<dcterms:created xsi:type=\"dcterms:W3CDTF\">{}</dcterms:created>\
<dcterms:modified xsi:type=\"dcterms:W3CDTF\">{}</dcterms:modified>\
</cp:coreProperties>",
        esc(&manifest.title),
        esc(&w3cdtf(&manifest.created)),
        esc(&w3cdtf(&manifest.modified)),
    )
}

/// Trim a stored ISO timestamp to the second, which is all W3CDTF allows here.
fn w3cdtf(iso: &str) -> String {
    match iso.find('.') {
        Some(at) if iso.len() > at => format!("{}Z", &iso[..at]),
        _ => iso.to_string(),
    }
}
