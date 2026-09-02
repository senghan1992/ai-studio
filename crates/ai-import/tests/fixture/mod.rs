//! Hand-built OOXML packages, for the details a round trip cannot reach.
//!
//! Each integration test compiles its own copy of this module and uses part of
//! it, so anything the *other* test needs looks unused here.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Workspace(PathBuf);

impl Workspace {
    pub fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ai-studio-import-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Workspace(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn geometry() -> ai_format::geometry::Box {
    ai_format::geometry::Box {
        x: 96.0,
        y: 200.0,
        w: 600.0,
        h: 320.0,
        z: 9.0,
    }
}

const NS: &str = concat!(
    " xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"",
    " xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"",
    " xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\""
);

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The default Office colour scheme, so a themed shape resolves to the colours
/// a reader would actually see.
const THEME: &str = r#"<?xml version="1.0"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
      <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
      <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
  </a:themeElements>
</a:theme>"#;

/// A minimal presentation, with the master and layout a real file has.
#[derive(Default)]
pub struct Builder {
    slides: Vec<String>,
    layout_shapes: String,
    master_styles: Option<String>,
    layout_bg: String,
    slide_attrs: String,
    /// Extra relationships on every slide, and the parts they point at — for the
    /// things that live outside the slide: diagrams, embedded objects, media.
    slide_rels: String,
    parts: Vec<(String, String)>,
    master_shapes: String,
    layout_hf: String,
    /// `p:sldSz` in EMU. Defaults to 16:9.
    size: Option<(i64, i64)>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    /// Shapes to place on the slide layout, where placeholders are inherited from.
    pub fn layout(mut self, shapes: &str) -> Builder {
        self.layout_shapes = shapes.to_string();
        self
    }

    pub fn slide(mut self, shapes: &str) -> Builder {
        self.slides.push(shapes.to_string());
        self
    }

    /// The layout's `p:hf`, which says whether the footer, date and slide-number
    /// fields are shown.
    pub fn layout_hf(mut self, hf: &str) -> Builder {
        self.layout_hf = hf.to_string();
        self
    }

    /// The presentation's slide size in EMU — 4:3 is 9144000 x 6858000.
    pub fn size(mut self, cx: i64, cy: i64) -> Builder {
        self.size = Some((cx, cy));
        self
    }

    /// Shapes on the slide master, where a template's own furniture lives.
    pub fn master_shapes(mut self, shapes: &str) -> Builder {
        self.master_shapes = shapes.to_string();
        self
    }

    /// Relationships to add to each slide's own `.rels`.
    pub fn slide_rels(mut self, rels: &str) -> Builder {
        self.slide_rels = rels.to_string();
        self
    }

    /// A part to drop into the package as-is.
    pub fn part(mut self, path: &str, body: &str) -> Builder {
        self.parts.push((path.to_string(), body.to_string()));
        self
    }

    /// The layout's `p:bg`, where a themed deck states its background.
    pub fn layout_bg(mut self, bg: &str) -> Builder {
        self.layout_bg = bg.to_string();
        self
    }

    /// Attributes on `p:sld` itself, such as `showMasterSp="0"`.
    pub fn slide_attrs(mut self, attrs: &str) -> Builder {
        self.slide_attrs = attrs.to_string();
        self
    }

    /// The master's `p:txStyles`, which is where a template's real font sizes
    /// live. The default has bullets but no sizes, as the earlier tests assume.
    pub fn master_styles(mut self, styles: &str) -> Builder {
        self.master_styles = Some(styles.to_string());
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut parts: Vec<(String, String)> = Vec::new();

        let mut ids = String::new();
        let mut presentation_rels = format!(
            "<Relationship Id=\"rIdMaster\" Type=\"{REL}/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>"
        );
        for (i, shapes) in self.slides.iter().enumerate() {
            let n = i + 1;
            ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{n}\"/>", 255 + n));
            presentation_rels.push_str(&format!(
                "<Relationship Id=\"rId{n}\" Type=\"{REL}/slide\" Target=\"slides/slide{n}.xml\"/>"
            ));
            parts.push((
                format!("ppt/slides/slide{n}.xml"),
                format!(
                    "<?xml version=\"1.0\"?><p:sld{NS}{}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sld>",
                    self.slide_attrs, shapes
                ),
            ));
            parts.push((
                format!("ppt/slides/_rels/slide{n}.xml.rels"),
                rels(&format!(
                    "<Relationship Id=\"rIdLayout\" Type=\"{REL}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>{}",
                    self.slide_rels
                )),
            ));
        }

        parts.push((
            "ppt/presentation.xml".into(),
            format!(
                "<?xml version=\"1.0\"?><p:presentation{NS}>\
                 <p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst>\
                 <p:sldIdLst>{ids}</p:sldIdLst>\
                 <p:sldSz cx=\"{}\" cy=\"{}\"/></p:presentation>",
                self.size.unwrap_or((12192000, 6858000)).0,
                self.size.unwrap_or((12192000, 6858000)).1
            ),
        ));
        parts.push((
            "ppt/_rels/presentation.xml.rels".into(),
            rels(&presentation_rels),
        ));

        parts.push((
            "ppt/slideLayouts/slideLayout1.xml".into(),
            format!(
                "<?xml version=\"1.0\"?><p:sldLayout{NS}><p:cSld>{}<p:spTree>{}</p:spTree></p:cSld>{}</p:sldLayout>",
                self.layout_bg, self.layout_shapes, self.layout_hf
            ),
        ));
        parts.push((
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
            rels(&format!(
                "<Relationship Id=\"rIdMaster\" Type=\"{REL}/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/>"
            )),
        ));

        // The master's body style is where Office's familiar bullets live.
        parts.push((
            "ppt/slideMasters/slideMaster1.xml".into(),
            format!(
                "<?xml version=\"1.0\"?><p:sldMaster{NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld>\
                 <p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
                 {}</p:sldMaster>",
                self.master_shapes,
                self.master_styles.as_deref().unwrap_or(
                    "<p:txStyles>\
                       <p:titleStyle><a:lvl1pPr><a:buNone/></a:lvl1pPr></p:titleStyle>\
                       <p:bodyStyle>\
                         <a:lvl1pPr><a:buFont typeface=\"Arial\"/><a:buChar char=\"\u{2022}\"/></a:lvl1pPr>\
                         <a:lvl2pPr><a:buFont typeface=\"Arial\"/><a:buChar char=\"\u{2013}\"/></a:lvl2pPr>\
                       </p:bodyStyle>\
                       <p:otherStyle><a:lvl1pPr/></p:otherStyle>\
                     </p:txStyles>"
                )
            ),
        ));
        parts.push((
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
            rels(&format!(
                "<Relationship Id=\"rIdLayout\" Type=\"{REL}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
                 <Relationship Id=\"rIdTheme\" Type=\"{REL}/theme\" Target=\"../theme/theme1.xml\"/>"
            )),
        ));
        parts.push(("ppt/theme/theme1.xml".into(), THEME.into()));
        parts.push((
            "_rels/.rels".into(),
            rels(&format!(
                "<Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"ppt/presentation.xml\"/>"
            )),
        ));

        parts.extend(self.parts.clone());
        zip_of(&parts)
    }
}

fn rels(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{body}</Relationships>"
    )
}

pub fn zip_of(parts: &[(String, String)]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for (path, body) in parts {
            zip.start_file(path, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buffer.into_inner()
}
