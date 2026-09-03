//! Reading an OOXML package: the zip, the relationship graph, and a tree of
//! elements simple enough to walk without a schema.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Office 파일을 열 수 없습니다: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("XML을 해석할 수 없습니다: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("{0} 파트가 없습니다")]
    MissingPart(String),
    #[error("지원하지 않는 파일 형식입니다: {0}")]
    UnsupportedFormat(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One XML element, with its attributes and children.
///
/// Deliberately untyped. An OOXML part is deep and mostly irrelevant to us, and
/// a schema-shaped reader would have to model every element it walks past just
/// to reach the four it needs.
#[derive(Debug, Clone, Default)]
pub struct Node {
    /// Local name, namespace prefix stripped: `sp`, `xfrm`, `prstGeom`.
    pub name: String,
    pub attrs: HashMap<String, String>,
    pub children: Vec<Node>,
    /// Concatenated text directly inside this element.
    pub text: String,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    pub fn attr_f64(&self, name: &str) -> Option<f64> {
        self.attr(name)?.trim().parse().ok()
    }

    pub fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name)?.trim().parse().ok()
    }

    /// An OOXML boolean: absent means false, `"1"`/`"true"`/`"on"` mean true.
    pub fn attr_bool(&self, name: &str) -> bool {
        matches!(self.attr(name), Some("1" | "true" | "on"))
    }

    /// The first direct child with this name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Direct children with this name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Follow a chain of child names: `node.path(&["spPr", "xfrm", "off"])`.
    pub fn path(&self, names: &[&str]) -> Option<&Node> {
        let mut current = self;
        for name in names {
            current = current.child(name)?;
        }
        Some(current)
    }

    /// Every descendant with this name, in document order.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        if self.name == name {
            out.push(self);
        }
        for child in &self.children {
            child.find_all(name, out);
        }
    }

    pub fn descendants(&self, name: &str) -> Vec<&Node> {
        let mut out = Vec::new();
        self.find_all(name, &mut out);
        out
    }

    /// All text anywhere inside, concatenated.
    pub fn all_text(&self) -> String {
        let mut out = self.text.clone();
        for child in &self.children {
            out.push_str(&child.all_text());
        }
        out
    }
}

/// The deepest element nesting we will build a tree for. Real Office parts nest
/// well under twenty levels; anything past this is a hostile file crafted to
/// overflow the stack when the tree is walked or dropped. Rejecting keeps the
/// recursion in `find_all`/`all_text`/`Drop` bounded.
const MAX_XML_DEPTH: usize = 256;

/// Parse an XML part into a tree.
pub fn parse_xml(bytes: &[u8]) -> Result<Node> {
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::from_str(&text);
    let config = reader.config_mut();
    config.trim_text(false);
    config.expand_empty_elements = false;

    let mut root = Node {
        name: "#document".into(),
        ..Node::default()
    };
    let mut stack: Vec<Node> = Vec::new();

    loop {
        match reader.read_event() {
            Err(e) => return Err(Error::Xml(e)),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(Error::UnsupportedFormat(
                        "XML 구조가 너무 깊습니다 — 손상됐거나 안전하지 않은 파일일 수 있습니다"
                            .into(),
                    ));
                }
                stack.push(node_from(&e)?)
            }
            Ok(Event::Empty(e)) => {
                let node = node_from(&e)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root.children.push(node),
                }
            }
            Ok(Event::End(_)) => {
                if let Some(node) = stack.pop() {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => root.children.push(node),
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&e.unescape().unwrap_or_default());
                }
            }
            _ => {}
        }
    }
    Ok(root)
}

fn node_from(e: &quick_xml::events::BytesStart) -> Result<Node> {
    let name = local_name(e.name().as_ref());
    let mut attrs = HashMap::new();
    for attr in e.attributes().with_checks(false) {
        let Ok(attr) = attr else { continue };
        let key = local_name(attr.key.as_ref());
        let value = attr.unescape_value().unwrap_or_default().into_owned();
        attrs.insert(key, value);
    }
    Ok(Node {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
    })
}

/// `a:prstGeom` -> `prstGeom`. Namespaces in OOXML are fixed by the schema, so
/// the prefix carries no information we need and dropping it keeps the reader
/// readable.
fn local_name(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    match text.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => text.into_owned(),
    }
}

/// An opened Office package.
pub struct Package {
    parts: HashMap<String, Vec<u8>>,
}

/// Decompression budget. A zip records the uncompressed size of each part in its
/// own header, but that number is attacker-controlled: a "zip bomb" declares (and
/// deflates to) hundreds of megabytes from a few kilobytes. We never trust it —
/// we allocate at most this per part and read through a hard cap, rejecting a
/// package that blows the per-part or whole-package budget before it can OOM.
const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PARTS: usize = 65_536;

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Package> {
        // The zip crate's own errors are English and speak of central directory
        // records; an office user needs to know what to do, not what a zip is.
        if bytes.is_empty() {
            return Err(Error::UnsupportedFormat(
                "빈 파일입니다 — 파일이 제대로 업로드됐는지 확인해 주세요".into(),
            ));
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).map_err(|_| {
            Error::UnsupportedFormat(
                "Office 파일로 열 수 없습니다 — 파일이 손상됐거나 Office 형식이 아닙니다".into(),
            )
        })?;
        if zip.len() > MAX_PARTS {
            return Err(Error::UnsupportedFormat(
                "파일에 포함된 항목이 너무 많습니다 — 손상됐거나 안전하지 않은 파일일 수 있습니다"
                    .into(),
            ));
        }
        let too_big = || {
            Error::UnsupportedFormat(
                "압축을 풀면 너무 커지는 파일입니다 — 손상됐거나 안전하지 않은 파일일 수 있습니다"
                    .into(),
            )
        };
        let mut parts = HashMap::new();
        let mut total: u64 = 0;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().trim_start_matches('/').to_string();
            // Pre-allocate to the declared size but never past the cap, then read
            // one byte past the cap so an over-budget part is caught, not trusted.
            let hint = file.size().min(MAX_PART_BYTES);
            let mut data = Vec::with_capacity(hint as usize);
            let read = file
                .by_ref()
                .take(MAX_PART_BYTES + 1)
                .read_to_end(&mut data)? as u64;
            if read > MAX_PART_BYTES {
                return Err(too_big());
            }
            total += read;
            if total > MAX_TOTAL_BYTES {
                return Err(too_big());
            }
            parts.insert(name, data);
        }
        Ok(Package { parts })
    }

    pub fn has(&self, path: &str) -> bool {
        self.parts.contains_key(path)
    }

    pub fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.parts.get(path).map(Vec::as_slice)
    }

    pub fn xml(&self, path: &str) -> Result<Node> {
        let bytes = self
            .bytes(path)
            .ok_or_else(|| Error::MissingPart(path.to_string()))?;
        parse_xml(bytes)
    }

    /// The same, but a missing part is not an error — plenty of parts are optional.
    pub fn xml_opt(&self, path: &str) -> Option<Node> {
        self.bytes(path).and_then(|b| parse_xml(b).ok())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }

    /// The relationships declared for a part, as `id -> Relationship`.
    ///
    /// Targets are resolved against the part's own folder, because that is what
    /// a relative target in a `_rels` file means.
    pub fn rels_for(&self, part: &str) -> HashMap<String, Relationship> {
        let (dir, file) = match part.rsplit_once('/') {
            Some((dir, file)) => (dir, file),
            None => ("", part),
        };
        let rels_path = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };

        let mut out = HashMap::new();
        let Some(root) = self.xml_opt(&rels_path) else {
            return out;
        };
        let Some(container) = root.child("Relationships") else {
            return out;
        };

        for rel in container.children_named("Relationship") {
            let Some(id) = rel.attr("Id") else { continue };
            let target = rel.attr("Target").unwrap_or_default();
            let external = rel.attr("TargetMode") == Some("External");
            let resolved = if external {
                target.to_string()
            } else {
                resolve(dir, target)
            };
            out.insert(
                id.to_string(),
                Relationship {
                    kind: rel
                        .attr("Type")
                        .and_then(|t| t.rsplit_once('/').map(|(_, k)| k.to_string()))
                        .unwrap_or_default(),
                    target: resolved,
                    external,
                },
            );
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct Relationship {
    /// The last segment of the relationship type: `slide`, `image`, `chart`.
    pub kind: String,
    /// The part path, resolved relative to the package root.
    pub target: String,
    pub external: bool,
}

/// Resolve a relationship target against the folder of the part declaring it.
fn resolve(dir: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/* -------------------------------------------------------------------- units */

/// EMU -> px at the 96dpi the canvas uses.
pub fn px(emu: i64) -> f64 {
    emu as f64 / 914_400.0 * 96.0
}

/// Hundredths of a point -> px.
pub fn px_from_font_size(hundredths: i64) -> f64 {
    hundredths as f64 / 100.0 / 0.75
}

/// Twentieths of a point -> px.
pub fn px_from_twip(twip: i64) -> f64 {
    twip as f64 / 20.0 / 0.75
}

/// Half-points -> px.
pub fn px_from_half_point(half: i64) -> f64 {
    half as f64 / 2.0 / 0.75
}

/// A `#RRGGBB` colour from an OOXML value, or `None` when it is not a plain one.
pub fn color(value: &str) -> Option<String> {
    let v = value.trim().trim_start_matches('#');
    if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("#{}", v.to_lowercase()));
    }
    if v.len() == 8 && v.chars().all(|c| c.is_ascii_hexdigit()) {
        // Word writes ARGB in some places; the alpha is the leading pair.
        return Some(format!("#{}", v[2..].to_lowercase()));
    }
    None
}

/// The solid colour inside a fill or line element, wherever OOXML put it.
///
/// A gradient is read as its first stop and a theme colour as its `lumMod`-free
/// value: an approximate colour beats a shape that turns invisible.
pub fn solid_color(node: &Node) -> Option<String> {
    for name in ["srgbClr", "sysClr"] {
        if let Some(found) = node.descendants(name).first() {
            // A `sysClr` names a system colour in `val` and carries the value
            // Office last resolved it to in `lastClr`. Every Office theme states
            // its text and background that way, so reading `val` alone loses the
            // black and the white that most of a document is drawn in.
            for value in [found.attr("lastClr"), found.attr("val")]
                .into_iter()
                .flatten()
            {
                if let Some(c) = color(value) {
                    return Some(c);
                }
            }
        }
    }
    None
}

/// Every font family the package asks for, from any part.
///
/// Selected by element rather than by attribute: `a:latin` is a font the file
/// applies, while the `a:font script="Thai"` list beside it in the theme is the
/// fallback table every Office theme carries. Scanning attributes alone reports
/// thirty families for a deck that uses two, which buries the answer.
pub fn font_names(package: &Package) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for name in package.names() {
        if !name.ends_with(".xml") {
            continue;
        }
        let Some(root) = package.xml_opt(name) else {
            continue;
        };
        // `latin`/`ea`/`cs` under a run or a theme's major/minor font; `rFonts`
        // in Word; `name` in a workbook's font table.
        for element in ["latin", "ea", "cs"] {
            for node in root.descendants(element) {
                if let Some(font) = node.attr("typeface") {
                    out.insert(font.to_string());
                }
            }
        }
        for node in root.descendants("rFonts") {
            for key in ["ascii", "hAnsi", "eastAsia", "cs"] {
                if let Some(font) = node.attr(key) {
                    out.insert(font.to_string());
                }
            }
        }
        if name.starts_with("xl/") {
            for node in root.descendants("name") {
                if let Some(font) = node.attr("val") {
                    out.insert(font.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_tree_with_attributes_and_text() {
        let xml = r#"<?xml version="1.0"?>
            <p:sp xmlns:p="x" xmlns:a="y">
              <p:spPr><a:xfrm rot="900000"><a:off x="10" y="20"/></a:xfrm></p:spPr>
              <p:txBody><a:p><a:r><a:t>안녕</a:t></a:r></a:p></p:txBody>
            </p:sp>"#;
        let root = parse_xml(xml.as_bytes()).unwrap();
        let sp = root.child("sp").expect("namespace prefix is stripped");
        let xfrm = sp.path(&["spPr", "xfrm"]).unwrap();
        assert_eq!(xfrm.attr_i64("rot"), Some(900_000));
        assert_eq!(xfrm.child("off").unwrap().attr_f64("x"), Some(10.0));
        assert_eq!(sp.descendants("t").len(), 1);
        assert_eq!(sp.all_text().trim(), "안녕");
    }

    #[test]
    fn deeply_nested_xml_is_rejected_instead_of_overflowing_the_stack() {
        // A hostile part nested past the limit must return an error, never build
        // a tree deep enough to overflow the stack when it is walked or dropped.
        let deep = format!("{}{}", "<a>".repeat(5000), "</a>".repeat(5000));
        let err = parse_xml(deep.as_bytes()).unwrap_err();
        assert!(
            matches!(err, Error::UnsupportedFormat(_)),
            "expected a friendly rejection, got {err:?}"
        );
        // A normally-nested part still parses.
        assert!(parse_xml(b"<a><b><c/></b></a>").is_ok());
    }

    #[test]
    fn empty_elements_become_childless_nodes() {
        let root = parse_xml(br#"<a><b/><c x="1"/></a>"#).unwrap();
        let a = root.child("a").unwrap();
        assert_eq!(a.children.len(), 2);
        assert_eq!(a.child("c").unwrap().attr("x"), Some("1"));
    }

    #[test]
    fn relationship_targets_resolve_against_their_part() {
        assert_eq!(
            resolve("ppt/slides", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(resolve("ppt/slides", "slide2.xml"), "ppt/slides/slide2.xml");
        assert_eq!(resolve("ppt", "/docProps/core.xml"), "docProps/core.xml");
        assert_eq!(resolve("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(resolve("word", "./media/a.png"), "word/media/a.png");
    }

    #[test]
    fn ooxml_booleans_default_to_false() {
        let root = parse_xml(br#"<a p="1" q="true" r="0" s="false"/>"#).unwrap();
        let a = root.child("a").unwrap();
        assert!(a.attr_bool("p") && a.attr_bool("q"));
        assert!(!a.attr_bool("r") && !a.attr_bool("s"));
        assert!(!a.attr_bool("missing"));
    }

    #[test]
    fn units_convert_at_96dpi() {
        assert_eq!(px(914_400), 96.0, "one inch");
        assert_eq!(px(12_192_000), 1280.0, "a 16:9 slide");
        assert_eq!(px_from_font_size(1500), 20.0, "15pt is 20px");
        assert_eq!(px_from_twip(1440), 96.0, "one inch of twips");
        assert_eq!(px_from_half_point(22), 14.666666666666666);
    }

    #[test]
    fn colours_normalise_and_reject_theme_names() {
        assert_eq!(color("4F46E5").as_deref(), Some("#4f46e5"));
        assert_eq!(color("#4F46E5").as_deref(), Some("#4f46e5"));
        assert_eq!(
            color("FF4F46E5").as_deref(),
            Some("#4f46e5"),
            "ARGB drops the alpha"
        );
        assert_eq!(color("accent1"), None);
    }

    #[test]
    fn a_gradient_is_read_as_a_colour_rather_than_nothing() {
        let root = parse_xml(
            br#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="DBEAFE"/></a:gs>
                <a:gs pos="100000"><a:srgbClr val="1E40AF"/></a:gs></a:gsLst></a:gradFill>"#,
        )
        .unwrap();
        let fill = root.child("gradFill").unwrap();
        assert_eq!(solid_color(fill).as_deref(), Some("#dbeafe"));
    }
}
