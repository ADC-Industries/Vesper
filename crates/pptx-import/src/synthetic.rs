//! Synthetic `.pptx` files for tests: the XML is written here, the content made up (no real
//! decks). A master with a theme (fonts, colours), title / body / other text styles, two
//! placeholders and a decoration; one layout; slides from raw shape XML.

use crate::zip::Writer;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A slide of a synthetic deck.
#[derive(Clone, Debug, Default)]
pub struct SlideSpec {
    /// Shapes (`p:sp`, `p:pic`, `p:graphicFrame` … XML), back to front.
    pub shapes: String,
    /// Media this slide's relationships name: (`rId`, file name in `ppt/media/`, bytes).
    pub media: Vec<(String, String, Vec<u8>)>,
    /// Notes text (one paragraph per line).
    pub notes: Option<String>,
    /// A raw `p:bg` element.
    pub background: Option<String>,
    pub hidden: bool,
}

/// A synthetic deck.
#[derive(Clone, Debug)]
pub struct DeckSpec {
    /// Slide size, EMU.
    pub size: (i64, i64),
    pub title: Option<String>,
    /// Theme fonts: heading, body.
    pub fonts: (String, String),
    /// Theme `accent1` (`RRGGBB`).
    pub accent1: String,
    pub slides: Vec<SlideSpec>,
    /// Sections: name and slide indices.
    pub sections: Vec<(String, Vec<usize>)>,
    /// `docProps/thumbnail.jpeg` (any picture bytes).
    pub thumbnail: Option<Vec<u8>>,
    /// The master's decoration (a bar along the bottom in `accent1`).
    pub master_bar: bool,
    /// `docProps/core.xml`'s `dc:language`.
    pub language: Option<String>,
}

impl Default for DeckSpec {
    fn default() -> Self {
        DeckSpec {
            size: (12_192_000, 6_858_000),
            title: Some("Synthetic deck".into()),
            fonts: ("Georgia".into(), "Verdana".into()),
            accent1: "2E75B6".into(),
            slides: Vec::new(),
            sections: Vec::new(),
            thumbnail: None,
            master_bar: true,
            language: None,
        }
    }
}

fn xfrm(x: i64, y: i64, cx: i64, cy: i64) -> String {
    format!(r#"<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#)
}

/// A title placeholder holding `text` (its place and style come from the layout and master).
pub fn title(id: u32, text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Title {id}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>"#,
        esc(text)
    )
}

/// The body placeholder (`idx` 1) with one paragraph per line, at `lvl` 0.
pub fn body(id: u32, lines: &[&str]) -> String {
    let ps: String = lines
        .iter()
        .map(|l| {
            format!(
                r#"<a:p><a:r><a:rPr lang="en-US"/><a:t>{}</a:t></a:r></a:p>"#,
                esc(l)
            )
        })
        .collect();
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Content {id}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{ps}</p:txBody></p:sp>"#
    )
}

/// A text box at a place (EMU) with raw paragraphs (`a:p` XML) and body properties.
pub fn text_box(
    id: u32,
    name: &str,
    at: (i64, i64, i64, i64),
    body_pr: &str,
    paras: &str,
) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody>{body_pr}<a:lstStyle/>{paras}</p:txBody></p:sp>"#,
        esc(name),
        xfrm(at.0, at.1, at.2, at.3)
    )
}

/// A shape (`prst` geometry) filled with `fill` (raw XML such as `<a:solidFill>…`).
pub fn shape(id: u32, name: &str, prst: &str, at: (i64, i64, i64, i64), fill: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>{fill}<a:ln w="25400"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></p:spPr></p:sp>"#,
        esc(name),
        xfrm(at.0, at.1, at.2, at.3)
    )
}

/// A picture of relationship `rid` at a place, with an optional `a:srcRect` (raw).
pub fn picture(id: u32, name: &str, rid: &str, at: (i64, i64, i64, i64), src_rect: &str) -> String {
    format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="{}"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{rid}"/>{src_rect}<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        esc(name),
        xfrm(at.0, at.1, at.2, at.3)
    )
}

/// A chart frame (its chart part isn't there: the import doesn't read it).
pub fn chart(id: u32, name: &str, at: (i64, i64, i64, i64)) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart"/></a:graphicData></a:graphic></p:graphicFrame>"#,
        esc(name),
        at.0,
        at.1,
        at.2,
        at.3
    )
}

/// A `w`×`h` PNG of one colour.
pub fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb(rgb));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("PNG encodes");
    out.into_inner()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The `.pptx` bytes of `d`.
pub fn build(d: &DeckSpec) -> Vec<u8> {
    let mut w = Writer::new();
    let mut add = |name: &str, s: &str| w.add(name, s.as_bytes()).expect("zip");
    let n = d.slides.len();
    let mut ct = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
    );
    for i in 1..=n {
        ct.push_str(&format!(r#"<Override PartName="/ppt/slides/slide{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#));
    }
    ct.push_str("</Types>");
    add("[Content_Types].xml", &ct);
    let thumb = if d.thumbnail.is_some() {
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail" Target="docProps/thumbnail.jpeg"/>"#.to_string()
    } else {
        String::new()
    };
    add(
        "_rels/.rels",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="ppt/presentation.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>{thumb}</Relationships>"#
        ),
    );
    add(
        "docProps/core.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{}</dc:title>{}</cp:coreProperties>"#,
            esc(d.title.as_deref().unwrap_or("")),
            d.language
                .as_deref()
                .map(|l| format!("<dc:language>{}</dc:language>", esc(l)))
                .unwrap_or_default()
        ),
    );
    add(
        "docProps/app.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Synthetic Slides</Application><AppVersion>1.0</AppVersion></Properties>"#,
    );
    // Presentation.
    let ids: String = (1..=n)
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 255 + i, 10 + i))
        .collect();
    let sections = if d.sections.is_empty() {
        String::new()
    } else {
        let s: String = d
            .sections
            .iter()
            .map(|(name, slides)| {
                let ids: String = slides
                    .iter()
                    .map(|i| format!(r#"<p14:sldId id="{}"/>"#, 256 + i))
                    .collect();
                format!(r#"<p14:section name="{}" id="{{00000000-0000-0000-0000-000000000000}}"><p14:sldIdLst>{ids}</p14:sldIdLst></p14:section>"#, esc(name))
            })
            .collect();
        format!(
            r#"<p:extLst><p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}"><p14:sectionLst xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main">{s}</p14:sectionLst></p:ext></p:extLst>"#
        )
    };
    add(
        "ppt/presentation.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="{}" cy="{}"/><p:notesSz cx="6858000" cy="9144000"/><p:defaultTextStyle><a:lvl1pPr marL="0" algn="l"><a:defRPr sz="1800"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:defaultTextStyle>{sections}</p:presentation>"#,
            d.size.0, d.size.1
        ),
    );
    let slide_rels: String = (1..=n)
        .map(|i| {
            format!(
                r#"<Relationship Id="rId{}" Type="{REL}/slide" Target="slides/slide{i}.xml"/>"#,
                10 + i
            )
        })
        .collect();
    add(
        "ppt/_rels/presentation.xml.rels",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="{REL}/theme" Target="theme/theme1.xml"/>{slide_rels}</Relationships>"#
        ),
    );
    // Theme.
    add(
        "ppt/theme/theme1.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme {NS} name="Synthetic"><a:themeElements><a:clrScheme name="S"><a:dk1><a:srgbClr val="1A1A1A"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="203040"/></a:dk2><a:lt2><a:srgbClr val="EEEEEE"/></a:lt2><a:accent1><a:srgbClr val="{}"/></a:accent1><a:accent2><a:srgbClr val="C55A11"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="S"><a:majorFont><a:latin typeface="{}"/></a:majorFont><a:minorFont><a:latin typeface="{}"/></a:minorFont></a:fontScheme><a:fmtScheme name="S"/></a:themeElements></a:theme>"#,
            d.accent1,
            esc(&d.fonts.0),
            esc(&d.fonts.1)
        ),
    );
    // Master: placeholders, text styles, a decoration.
    let bar = if d.master_bar {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="9" name="Footer bar"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>"#,
            xfrm(0, d.size.1 - 342_900, d.size.0, 342_900)
        )
    } else {
        String::new()
    };
    add(
        "ppt/slideMasters/slideMaster1.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldMaster {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:schemeClr val="bg1"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr>{}</p:spPr><p:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:r><a:t>Title</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Text Placeholder 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr>{}</p:spPr><p:txBody><a:bodyPr><a:normAutofit/></a:bodyPr><a:lstStyle/><a:p><a:r><a:t>Text</a:t></a:r></a:p></p:txBody></p:sp>{bar}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr algn="l"><a:defRPr sz="4400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr marL="228600" indent="-228600" algn="l"><a:spcBef><a:spcPts val="1000"/></a:spcBef><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/><a:defRPr sz="2800"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr><a:lvl2pPr marL="685800" indent="-228600" algn="l"><a:buChar char="&#8211;"/><a:defRPr sz="2400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl2pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#,
            xfrm(838_200, 365_125, d.size.0 - 1_676_400, 1_325_563),
            xfrm(838_200, 1_825_625, d.size.0 - 1_676_400, 4_351_338)
        ),
    );
    add(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="{REL}/theme" Target="../theme/theme1.xml"/></Relationships>"#
        ),
    );
    // Layout: centres the title.
    add(
        "ppt/slideLayouts/slideLayout1.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldLayout {NS} type="obj" preserve="1"><p:cSld name="Title and Content"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr algn="ctr"><a:defRPr b="1"/></a:lvl1pPr></a:lstStyle><a:p><a:r><a:t>Title</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Text</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#
        ),
    );
    add(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#
        ),
    );
    // Slides.
    for (i, s) in d.slides.iter().enumerate() {
        let k = i + 1;
        let bg = s.background.clone().unwrap_or_default();
        let show = if s.hidden { r#" show="0""# } else { "" };
        add(
            &format!("ppt/slides/slide{k}.xml"),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld {NS}{show}><p:cSld>{bg}<p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
                s.shapes
            ),
        );
        let mut rels = format!(
            r#"<Relationship Id="rId1" Type="{REL}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>"#
        );
        for (rid, file, _) in &s.media {
            rels.push_str(&format!(
                r#"<Relationship Id="{rid}" Type="{REL}/image" Target="../media/{file}"/>"#
            ));
        }
        if let Some(notes) = &s.notes {
            rels.push_str(&format!(
                r#"<Relationship Id="rIdNotes" Type="{REL}/notesSlide" Target="../notesSlides/notesSlide{k}.xml"/>"#
            ));
            let ps: String = notes
                .lines()
                .map(|l| format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", esc(l)))
                .collect();
            add(
                &format!("ppt/notesSlides/notesSlide{k}.xml"),
                &format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:notes {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image"/><p:cNvSpPr/><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{ps}</p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
                ),
            );
            add(
                &format!("ppt/notesSlides/_rels/notesSlide{k}.xml.rels"),
                &format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/slide" Target="../slides/slide{k}.xml"/></Relationships>"#
                ),
            );
        }
        add(
            &format!("ppt/slides/_rels/slide{k}.xml.rels"),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
            ),
        );
    }
    drop(add);
    let mut seen = std::collections::HashSet::new();
    for s in &d.slides {
        for (_, file, bytes) in &s.media {
            // The same file may be named by two slides.
            if seen.insert(file.clone()) {
                w.add(&format!("ppt/media/{file}"), bytes).expect("zip");
            }
        }
    }
    if let Some(t) = &d.thumbnail {
        w.add("docProps/thumbnail.jpeg", t).expect("zip");
    }
    w.finish().expect("zip")
}
