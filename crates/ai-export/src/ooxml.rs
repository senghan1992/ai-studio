//! The small amount of OOXML plumbing every Office format needs.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("내보내기 패키지를 만들 수 없습니다: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An OOXML package under construction: a set of named parts, zipped at the end.
#[derive(Default)]
pub struct Package {
    parts: Vec<(String, Vec<u8>)>,
}

impl Package {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: &str, data: impl Into<Vec<u8>>) {
        self.parts.push((path.to_string(), data.into()));
    }

    /// Add an XML part, prefixing the declaration every Office part carries.
    pub fn add_xml(&mut self, path: &str, body: &str) {
        let mut out = String::with_capacity(body.len() + 64);
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n");
        out.push_str(body);
        self.add(path, out.into_bytes());
    }

    pub fn finish(self) -> Result<Vec<u8>> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            // Deflate on everything: media is usually already compressed, but the
            // XML parts dominate the size and compress by roughly 10x.
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, data) in self.parts {
                zip.start_file(path, options)?;
                zip.write_all(&data)?;
            }
            zip.finish()?;
        }
        Ok(buffer.into_inner())
    }
}

/// Escape text for an XML text node or attribute value.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most control characters outright; dropping them
            // beats writing a file Office refuses to open.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// A colour as OOXML wants it: six upper-case hex digits, no `#`.
pub fn hex(color: &str) -> String {
    let value = color.trim().trim_start_matches('#');
    let is_hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());
    match value.len() {
        6 if is_hex(value) => value.to_uppercase(),
        3 if is_hex(value) => value
            .chars()
            .flat_map(|c| [c, c])
            .collect::<String>()
            .to_uppercase(),
        _ => "000000".to_string(),
    }
}

/// px -> EMU (English Metric Units), at the 96dpi the canvas assumes.
/// 914400 EMU per inch.
pub fn emu(px: f64) -> i64 {
    (px / 96.0 * 914_400.0).round() as i64
}

/// px -> points.
pub fn pt(px: f64) -> f64 {
    px * 0.75
}

/// px -> twips (1/20 pt), the unit `.docx` uses for indents and margins.
pub fn twip(px: f64) -> i64 {
    (px * 15.0).round() as i64
}

/// px -> points for a font size, snapped to what Office actually offers.
///
/// The editor works in whole px, so an imported 10pt (13.33px) is stored as
/// 13px and would come back as 9.75pt — every cell of a re-saved workbook read
/// a quarter-point smaller. A whole-px rounding is off by at most 0.375pt, so
/// snapping to the nearest whole point restores every integer size exactly;
/// anything further away is a genuine half-point size and is kept to 0.5.
pub fn font_pt(px: f64) -> f64 {
    ai_format::font::pt_for_px(px)
}

/// px -> hundredths of a point, the unit DrawingML uses for font sizes.
pub fn font_size_100(px: f64) -> i64 {
    (font_pt(px) * 100.0).round().max(100.0) as i64
}

/// A `_rels` part for a list of `(id, type, target)` relationships.
pub fn relationships(rels: &[(String, &str, String)]) -> String {
    let mut out = String::from(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in rels {
        out.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"/>",
            esc(id),
            kind,
            esc(target)
        ));
    }
    out.push_str("</Relationships>");
    out
}

/// The same, marking the listed ids as external targets — what a hyperlink
/// relationship requires.
pub fn relationships_with_modes(
    rels: &[(String, &str, String)],
    external: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::from(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in rels {
        let mode = if external.contains(id.as_str()) {
            " TargetMode=\"External\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{mode}/>",
            esc(id),
            kind,
            esc(target)
        ));
    }
    out.push_str("</Relationships>");
    out
}

/// The natural pixel size of an image, when we can read it from the header.
///
/// Only the three formats the editor accepts are handled; anything else falls
/// back to the caller's box, which is what the old exporter did unconditionally.
pub fn image_size(data: &[u8]) -> Option<(f64, f64)> {
    // PNG: 8-byte signature, then an IHDR chunk whose payload starts at byte 16.
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
        let w = u32::from_be_bytes(data[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(data[20..24].try_into().ok()?);
        return Some((w as f64, h as f64));
    }
    // GIF: width and height are little-endian at bytes 6..10.
    if (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) && data.len() >= 10 {
        let w = u16::from_le_bytes(data[6..8].try_into().ok()?);
        let h = u16::from_le_bytes(data[8..10].try_into().ok()?);
        return Some((w as f64, h as f64));
    }
    // JPEG: walk the marker chain to the first start-of-frame.
    if data.starts_with(b"\xff\xd8") {
        let mut i = 2usize;
        while i + 3 < data.len() {
            if data[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = data[i + 1];
            // Standalone markers carry no length.
            if (0xD0..=0xD9).contains(&marker) || marker == 0x01 || marker == 0xFF {
                i += 2;
                continue;
            }
            let length = u16::from_be_bytes(data[i + 2..i + 4].try_into().ok()?) as usize;
            let is_sof = (0xC0..=0xCF).contains(&marker)
                && marker != 0xC4
                && marker != 0xC8
                && marker != 0xCC;
            if is_sof {
                if i + 9 > data.len() {
                    return None;
                }
                let h = u16::from_be_bytes(data[i + 5..i + 7].try_into().ok()?);
                let w = u16::from_be_bytes(data[i + 7..i + 9].try_into().ok()?);
                return Some((w as f64, h as f64));
            }
            i += 2 + length;
        }
    }
    None
}

/// The OOXML content-type for an image file extension.
pub fn image_content_type(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        // Vector and legacy raster formats Office embeds constantly — a pasted
        // chart or diagram is EMF/WMF, scans are TIFF/BMP. Dropping them here
        // would silently lose the image when a round-tripped file is re-saved.
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_the_xml_metacharacters() {
        assert_eq!(
            esc("a<b>&\"c\"'d'"),
            "a&lt;b&gt;&amp;&quot;c&quot;&apos;d&apos;"
        );
        assert_eq!(esc("한글 그대로"), "한글 그대로");
        // A stray control character would make the package unopenable.
        assert_eq!(esc("a\u{0}b\u{8}c\td"), "abc\td");
    }

    #[test]
    fn colours_normalise_to_six_hex_digits() {
        assert_eq!(hex("#4f46e5"), "4F46E5");
        assert_eq!(hex("f00"), "FF0000");
        assert_eq!(hex("rgb(1,2,3)"), "000000");
        assert_eq!(hex(""), "000000");
    }

    #[test]
    fn unit_conversions_match_the_96dpi_canvas() {
        assert_eq!(emu(96.0), 914_400, "one inch");
        assert_eq!(emu(1280.0), 12_192_000, "a 16:9 slide is 13.333in wide");
        assert_eq!(pt(96.0), 72.0);
        assert_eq!(twip(72.0), 1080);
        assert_eq!(font_size_100(20.0), 1500, "20px is 15pt");
    }

    #[test]
    fn a_package_zips_its_parts() {
        let mut pkg = Package::new();
        pkg.add_xml("[Content_Types].xml", "<Types/>");
        pkg.add("word/media/a.png", vec![1u8, 2, 3]);
        let bytes = pkg.finish().unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(zip.len(), 2);
        let names: Vec<String> = zip.file_names().map(str::to_string).collect();
        assert!(names.contains(&"[Content_Types].xml".to_string()));
        let mut part = zip.by_name("[Content_Types].xml").unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut part, &mut text).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\""));
        assert!(text.ends_with("<Types/>"));
    }

    #[test]
    fn image_headers_yield_dimensions() {
        // A 2x3 PNG header is enough; the pixel data is irrelevant here.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&2u32.to_be_bytes());
        png.extend_from_slice(&3u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((2.0, 3.0)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&10u16.to_le_bytes());
        gif.extend_from_slice(&20u16.to_le_bytes());
        assert_eq!(image_size(&gif), Some((10.0, 20.0)));

        // JPEG: SOI, then an APP0 segment to skip, then SOF0 with 40x30.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        jpeg.extend_from_slice(&30u16.to_be_bytes());
        jpeg.extend_from_slice(&40u16.to_be_bytes());
        assert_eq!(image_size(&jpeg), Some((40.0, 30.0)));

        assert_eq!(image_size(b"not an image"), None);
    }

    #[test]
    fn font_sizes_snap_back_to_the_points_they_were_imported_from() {
        // The editor stores whole px, so 10pt arrives as 13px; writing 9.75pt
        // back shrank every cell of a re-saved workbook by a quarter point.
        assert_eq!(font_pt(13.0), 10.0, "10pt");
        assert_eq!(font_pt(17.0), 13.0, "13pt");
        assert_eq!(font_pt(19.0), 14.0, "14pt");
        assert_eq!(font_pt(15.0), 11.0, "11pt");
        assert_eq!(font_pt(43.0), 32.0, "32pt");
        // A genuine half-point size is kept as one.
        assert_eq!(font_pt(14.0), 10.5, "10.5pt is exactly 14px");
        assert_eq!(font_size_100(13.0), 1000);
    }

    #[test]
    fn office_vector_and_legacy_image_types_are_kept_not_dropped() {
        // These extensions returning None would drop the image on export, losing
        // a pasted chart or a scanned figure from a round-tripped file.
        assert_eq!(image_content_type("emf"), Some("image/x-emf"));
        assert_eq!(image_content_type("WMF"), Some("image/x-wmf"));
        assert_eq!(image_content_type("tiff"), Some("image/tiff"));
        assert_eq!(image_content_type("tif"), Some("image/tiff"));
        assert_eq!(image_content_type("bmp"), Some("image/bmp"));
        assert_eq!(image_content_type("png"), Some("image/png"));
        assert_eq!(image_content_type("xyz"), None);
    }
}
