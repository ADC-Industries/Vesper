//! Reads PowerPoint `.pptx` files into Vesper presentations (Keynote and Google Slides come in
//! as `.pptx` exported from them).
//!
//! **Editable where possible, a picture of the slide as fallback** (owner, 2026-11-25). Each
//! slide becomes one Vesper slide: text boxes become text elements (position, size, font,
//! size, colour, bold / italic / underline, alignment, bullets, insets, vertical anchor,
//! shrink-to-fit), pictures become picture elements (their files go into the Codex like
//! other media), rectangles / rounded rectangles / ellipses / simple triangles and diamonds
//! become shapes with their solid or gradient fill, outline and shadow; the slide's
//! background (colour, gradient, picture) and notes come along, and the slide size sets the
//! presentation's canvas (1920 px wide). Theme fonts and colours are resolved through the
//! slide's layout and master (placeholders and list styles: slide → layout → master →
//! presentation default), and the layout's and master's own decorations are drawn under the
//! slide's shapes as PowerPoint does.
//!
//! What the model can't hold (charts, tables, SmartArt, freeforms, other shapes, lines,
//! pattern fills, pictures in formats Vesper doesn't show) is drawn from a **picture of the
//! slide** cut to that object's place, at its place in the stack: the rest of the slide
//! stays editable. Pictures come from LibreOffice when the server has it ([`render`]), else,
//! for the first slide only, from the thumbnail the file carries; without either the object
//! is approximated (a shape drawn as a rectangle with its text) or left out, and the report
//! says so. Every such case is in the [`Report`].

mod deck;
mod opc;
pub mod render;
pub mod synthetic;
mod text;
mod theme;
pub mod zip;

use anyhow::{Context, Result, bail};
use deck::{Background, Item, Media, Part};
use model::{
    Color, EdgeInsets, Element, ElementKind, Fill, FillKind, ImportInfo, ImportSource,
    MediaAssetId, MediaDisplay, Presentation, Rect, ScaleMode, Section, Size, Slide, SlideLabel,
};
use opc::{Rels, attr, attr_bool, attr_i64, child, children, path};
use std::collections::HashMap;
use std::path::PathBuf;
use text::{Levels, Scale};
use theme::Palette;

/// Width of the canvas every import gets, px (the height follows the slide size).
pub const CANVAS_WIDTH: f64 = 1920.0;
/// Most a slide's height may be its width, or its width its height.
const MAX_ASPECT: f64 = 64.0;

/// How to import.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The presentation's name (default: the deck's title, else "Imported slides").
    pub name: Option<String>,
    /// How slide pictures are made for what isn't editable.
    pub renderer: render::Renderer,
    /// A folder for the picture fallback's files (a temporary folder when `None`).
    pub work_dir: Option<PathBuf>,
}

/// A media file of the import: the presentation refers to `asset`, which the caller replaces
/// with the Codex asset the file became.
#[derive(Clone, Debug)]
pub struct MediaFile {
    /// The ID the presentation uses for it.
    pub asset: MediaAssetId,
    /// A file name for it.
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Something that wasn't imported as editable content.
#[derive(Clone, Debug, PartialEq)]
pub struct Unmapped {
    /// What it is (`chart`, `table`, `smartArt`, `line`, `a “cloud” shape`, `video` …).
    pub kind: String,
    /// The object's name in the deck, if any.
    pub name: String,
    /// What became of it.
    pub detail: String,
}

/// Where a slide's picture came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureSource {
    /// Drawn by LibreOffice.
    Renderer,
    /// The deck's own thumbnail (first slide only, small).
    Thumbnail,
}

/// One slide's outcome.
#[derive(Clone, Debug, Default)]
pub struct SlideReport {
    /// 1-based.
    pub number: u32,
    /// The slide's title text, if it has a title.
    pub title: String,
    /// Editable elements on it.
    pub editable: u32,
    /// Objects drawn from the slide's picture.
    pub pictured: Vec<Unmapped>,
    /// Objects approximated or left out (no picture).
    pub approximated: Vec<Unmapped>,
    /// Other notes (video not imported, vertical text …).
    pub notes: Vec<Unmapped>,
    /// Where its picture came from, when it used one.
    pub picture: Option<PictureSource>,
    /// The slide is hidden in the deck (imported disabled).
    pub hidden: bool,
}

/// What an import did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub slides: Vec<SlideReport>,
    /// The picture program used, if any.
    pub renderer: Option<String>,
    /// Why pictures couldn't be made, when some were needed.
    pub renderer_problem: Option<String>,
    /// The slide size, EMU.
    pub slide_size: (i64, i64),
}

/// A converted deck.
pub struct Deck {
    pub presentation: Presentation,
    pub media: Vec<MediaFile>,
    pub report: Report,
}

/// Import stages, for progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Reading slides (`done` of `total`).
    Slides,
    /// Drawing slide pictures (`total` slides, `done` when finished).
    Pictures,
}

struct SlideWork {
    items: Vec<Item>,
    background: Option<Background>,
    report: SlideReport,
    notes: model::RichText,
    hidden: bool,
    section: Option<String>,
}

/// Imports `bytes` (a `.pptx`). `progress(stage, done, total)` is called as it goes.
pub fn import(
    bytes: &[u8],
    opts: &Options,
    progress: &mut dyn FnMut(Stage, u32, u32),
) -> Result<Deck> {
    let zip = zip::Archive::open(bytes)?;
    if zip.has("Index/Document.iwa") || zip.has("Index.zip") {
        bail!(
            "this is a Keynote file: in Keynote, choose File ▸ Export To ▸ PowerPoint, then import the .pptx"
        );
    }
    let root_rels = Rels::of(&zip, "");
    let pres_part = root_rels
        .first("officeDocument")
        .unwrap_or("ppt/presentation.xml")
        .to_string();
    let pres_xml = zip
        .text(&pres_part)
        .context("not a PowerPoint presentation (no presentation part)")?;
    let pres_doc = opc::xml(&pres_xml).context("the presentation isn't XML")?;
    let pres = pres_doc.root_element();
    if pres.tag_name().name() != "presentation" {
        bail!("not a PowerPoint presentation");
    }
    let (cx, cy) = child(pres, "sldSz")
        .map(|s| {
            (
                attr_i64(s, "cx").unwrap_or(12_192_000),
                attr_i64(s, "cy").unwrap_or(6_858_000),
            )
        })
        .unwrap_or((12_192_000, 6_858_000));
    // PowerPoint's slides are 1 to 56 inches a side; a crafted size (one EMU high) would give
    // a canvas 0 px high and pictures cut at infinite scale.
    let aspect = cy as f64 / cx as f64;
    if cx <= 0 || cy <= 0 || !(1.0 / MAX_ASPECT..=MAX_ASPECT).contains(&aspect) {
        bail!("the slide size is broken");
    }
    let scale = Scale {
        px: CANVAS_WIDTH / cx as f64,
    };
    let canvas = Size::new(CANVAS_WIDTH, (CANVAS_WIDTH * cy as f64 / cx as f64).round());
    let pres_rels = Rels::of(&zip, &pres_part);
    let base_palette = Palette::default();
    let default_text = child(pres, "defaultTextStyle")
        .map(|d| Levels::read(d, &base_palette))
        .unwrap_or_default();
    // Slides in order, with their numeric IDs (for sections).
    let slide_ids: Vec<(String, String)> = child(pres, "sldIdLst")
        .map(|l| {
            children(l, "sldId")
                .filter_map(|s| {
                    let rid = s
                        .attributes()
                        .find(|a| a.name() == "id" && a.namespace().is_some())?
                        .value();
                    let num = attr(s, "id").unwrap_or_default().to_string();
                    Some((num, pres_rels.target(rid)?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    if slide_ids.is_empty() {
        bail!("the presentation has no slides");
    }
    // Sections (PowerPoint 2010+): slide number ID → section name.
    let mut section_of: HashMap<String, String> = HashMap::new();
    for sec in pres
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "section")
    {
        let name = attr(sec, "name").unwrap_or_default().to_string();
        for s in sec
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "sldId")
        {
            if let Some(id) = attr(s, "id") {
                section_of.insert(id.to_string(), name.clone());
            }
        }
    }

    let doc_name = opts
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| deck_title(&zip))
        .unwrap_or_else(|| "Imported slides".into());
    let mut media = Media::new(format!("{} ", file_safe(&doc_name)));
    let mut deck_notes = Vec::new();

    // Masters, then layouts (each needs its master).
    let mut masters: HashMap<String, Part> = HashMap::new();
    let mut layouts: HashMap<String, (Part, String)> = HashMap::new();
    let total = slide_ids.len() as u32;
    let mut work: Vec<SlideWork> = Vec::new();
    // The languages of the slides' text runs, by characters (the deck's language).
    let mut languages: HashMap<String, usize> = HashMap::new();
    for (i, (num, slide_part)) in slide_ids.iter().enumerate() {
        progress(Stage::Slides, i as u32, total);
        let slide_rels = Rels::of(&zip, slide_part);
        let layout_part = slide_rels.first("slideLayout").map(str::to_string);
        if let Some(lp) = &layout_part
            && !layouts.contains_key(lp)
        {
            let master_part = Rels::of(&zip, lp).first("slideMaster").map(str::to_string);
            if let Some(mp) = &master_part
                && !masters.contains_key(mp)
            {
                let mut palette = Palette::default();
                let mrels = Rels::of(&zip, mp);
                if let Some(t) = mrels.first("theme")
                    && let Ok(tx) = zip.text(t)
                    && let Ok(td) = opc::xml(&tx)
                {
                    palette.read_theme(td.root_element());
                }
                if let Ok(mx) = zip.text(mp)
                    && let Ok(md) = opc::xml(&mx)
                    && let Some(m) = child(md.root_element(), "clrMap")
                {
                    palette.read_map(m);
                }
                let default_text = child(pres, "defaultTextStyle")
                    .map(|d| Levels::read(d, &palette))
                    .unwrap_or_else(|| default_text.clone());
                let m = deck::read_part(
                    &zip,
                    mp,
                    &palette,
                    scale,
                    &mut media,
                    Vec::new(),
                    None,
                    &default_text,
                    &mut deck_notes,
                )
                .with_context(|| format!("reading {mp}"))?;
                masters.insert(mp.clone(), m);
            }
            let mp = master_part.unwrap_or_default();
            let master = masters.get(&mp);
            let palette = master.map(|m| m.palette.clone()).unwrap_or_default();
            let default_text = child(pres, "defaultTextStyle")
                .map(|d| Levels::read(d, &palette))
                .unwrap_or_else(|| default_text.clone());
            let l = deck::read_part(
                &zip,
                lp,
                &palette,
                scale,
                &mut media,
                master.into_iter().collect(),
                master,
                &default_text,
                &mut deck_notes,
            )
            .with_context(|| format!("reading {lp}"))?;
            layouts.insert(lp.clone(), (l, mp));
        }
        let (layout, master) = match layout_part.as_ref().and_then(|lp| layouts.get(lp)) {
            Some((l, mp)) => (Some(l), masters.get(mp)),
            None => (None, None),
        };
        let palette = master.map(|m| m.palette.clone()).unwrap_or_default();
        let default_text = child(pres, "defaultTextStyle")
            .map(|d| Levels::read(d, &palette))
            .unwrap_or_else(|| default_text.clone());
        let xml = zip
            .text(slide_part)
            .with_context(|| format!("slide {} can't be read", i + 1))?;
        let sd = opc::xml(&xml).with_context(|| format!("slide {} isn't XML", i + 1))?;
        let root = sd.root_element();
        let show_master = attr_bool(root, "showMasterSp").unwrap_or(true);
        let hidden = attr(root, "show").is_some_and(|v| v == "0" || v == "false");
        let mut ctx = deck::Ctx {
            zip: &zip,
            palette: &palette,
            scale,
            rels: &slide_rels,
            media: &mut media,
            chain: layout.into_iter().chain(master).collect(),
            master,
            default_text: &default_text,
            draw_placeholders: true,
            notes: Vec::new(),
        };
        let mut items = Vec::new();
        if show_master {
            if let (Some(m), Some(l)) = (master, layout)
                && l.show_master_shapes
            {
                items.extend(m.items.iter().cloned().map(fresh));
            }
            if let Some(l) = layout {
                items.extend(l.items.iter().cloned().map(fresh));
            }
        }
        let tree = path(root, &["cSld", "spTree"]);
        if let Some(t) = tree {
            ctx.top(t, &mut items);
        }
        let background = path(root, &["cSld", "bg"])
            .and_then(|bg| ctx.background(bg))
            .or_else(|| layout.and_then(|l| l.background.clone()))
            .or_else(|| master.and_then(|m| m.background.clone()));
        let slide_notes = std::mem::take(&mut ctx.notes);
        let notes = slide_rels
            .first("notesSlide")
            .and_then(|np| zip.text(np).ok())
            .map(|nx| notes_of(&nx))
            .unwrap_or_default();
        let title = tree.map(title_of).unwrap_or_default();
        if let Some(t) = tree {
            count_languages(t, &mut languages);
        }
        work.push(SlideWork {
            items,
            background,
            report: SlideReport {
                number: i as u32 + 1,
                title,
                notes: slide_notes,
                hidden,
                ..SlideReport::default()
            },
            notes,
            hidden,
            section: section_of.get(num).cloned(),
        });
    }
    progress(Stage::Slides, total, total);

    // Pictures for the slides that need one.
    let needs: Vec<usize> = work
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            w.items.iter().any(|i| matches!(i, Item::Region { .. }))
                || matches!(w.background, Some(Background::Unmapped(_)))
        })
        .map(|(i, _)| i)
        .collect();
    let mut report = Report {
        slide_size: (cx, cy),
        ..Report::default()
    };
    let mut pictures: HashMap<usize, (Vec<u8>, PictureSource)> = HashMap::new();
    if !needs.is_empty() {
        progress(Stage::Pictures, 0, needs.len() as u32);
        match opts.renderer.program() {
            Some(program) => {
                report.renderer = Some(program.display().to_string());
                let tmp;
                let dir = match &opts.work_dir {
                    Some(d) => d.clone(),
                    None => {
                        tmp = TempDir::new()?;
                        tmp.0.clone()
                    }
                };
                let px = (canvas.width as u32, canvas.height as u32);
                let r = render::SlideList::read(&pres_xml, |rid| pres_rels.target(rid).is_some())
                    .and_then(|list| {
                        render::render(&program, &zip, &pres_part, &list, &needs, px, &dir)
                    });
                match r {
                    Ok(m) => {
                        let missing = needs.iter().filter(|i| !m.contains_key(i)).count();
                        if missing > 0 {
                            report.renderer_problem = Some(format!(
                                "LibreOffice drew {} of the {} slides that needed a picture",
                                m.len(),
                                needs.len()
                            ));
                        }
                        pictures.extend(
                            m.into_iter()
                                .map(|(i, b)| (i, (b, PictureSource::Renderer))),
                        );
                    }
                    Err(e) => report.renderer_problem = Some(format!("{e:#}")),
                }
            }
            None if opts.renderer == render::Renderer::Off => {
                report.renderer_problem = Some("slide pictures are turned off".into());
            }
            None => {
                report.renderer_problem = Some(
                    "LibreOffice isn't installed on the server: slide pictures can't be made"
                        .into(),
                );
            }
        }
        if needs.contains(&0)
            && !pictures.contains_key(&0)
            && let Some(t) = thumbnail(&zip)
        {
            pictures.insert(0, (t, PictureSource::Thumbnail));
        }
        progress(Stage::Pictures, needs.len() as u32, needs.len() as u32);
    }

    // Slides.
    let mut presentation = Presentation::new(doc_name.clone());
    presentation.canvas_size = canvas;
    presentation.language = deck_language(&zip).or_else(|| most_used(&languages));
    presentation.import = Some(ImportInfo {
        source: ImportSource::PowerPoint,
        original_name: Some(doc_name.clone()),
        writer_version: app_version(&zip),
        ..ImportInfo::default()
    });
    let mut sections: Vec<Section> = Vec::new();
    for (i, w) in work.into_iter().enumerate() {
        let SlideWork {
            items,
            background,
            report: mut report_slide,
            notes,
            hidden,
            section,
        } = w;
        let picture = pictures.get(&i).map(|(bytes, src)| {
            let dims = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .unwrap_or((canvas.width as u32, canvas.height as u32));
            let asset = MediaAssetId::new();
            media.files.push(MediaFile {
                asset,
                name: format!("{}slide {}.png", media.prefix, i + 1),
                bytes: bytes.clone(),
            });
            (asset, dims, *src)
        });
        let mut slide = Slide::new();
        slide.enabled = !hidden;
        slide.notes = notes;
        slide.import = Some(ImportInfo {
            source: ImportSource::PowerPoint,
            original_name: (!report_slide.title.is_empty()).then(|| report_slide.title.clone()),
            ..ImportInfo::default()
        });
        if !report_slide.title.is_empty() {
            slide.label = Some(SlideLabel {
                text: report_slide.title.chars().take(40).collect(),
                color: None,
            });
        }
        let mut used_picture = false;
        match background {
            Some(Background::Color(c)) => slide.background = Some(c),
            Some(Background::Fill(f)) => {
                slide.background = Some(Color::BLACK);
                let mut e = Element::new(Rect::full(canvas));
                e.name = "Background".into();
                e.fill = *f;
                slide.elements.push(e);
            }
            Some(Background::Unmapped(what)) => {
                let u = Unmapped {
                    kind: "background".into(),
                    name: String::new(),
                    detail: what,
                };
                if let Some((asset, dims, _)) = picture {
                    slide.elements.push(cut(
                        asset,
                        dims,
                        canvas,
                        Rect::full(canvas),
                        0.0,
                        "Background (picture)",
                    ));
                    used_picture = true;
                    report_slide.pictured.push(u);
                } else {
                    slide.background = Some(Color::WHITE);
                    report_slide.approximated.push(Unmapped {
                        detail: format!("{} (drawn white)", u.detail),
                        ..u
                    });
                }
            }
            None => slide.background = Some(Color::WHITE),
        }
        for item in items {
            // A crafted deck's geometry can come out infinite or NaN, which the saved
            // presentation can't hold (JSON has no such numbers): such objects are left out.
            let unreadable = match &item {
                Item::El(e) => !finite_element(e),
                Item::Region {
                    bounds, rotation, ..
                } => !finite_rect(bounds) || !rotation.is_finite(),
            };
            if unreadable {
                let (kind, name) = match &item {
                    Item::El(e) => ("object".to_string(), e.name.clone()),
                    Item::Region { kind, name, .. } => (kind.clone(), name.clone()),
                };
                report_slide.approximated.push(Unmapped {
                    kind,
                    name,
                    detail: "left out (its place or size can't be read)".into(),
                });
                continue;
            }
            match item {
                Item::El(e) => {
                    report_slide.editable += 1;
                    slide.elements.push(e);
                }
                Item::Region {
                    bounds,
                    rotation,
                    name,
                    kind,
                    approx,
                } => {
                    let u = Unmapped {
                        kind,
                        name: name.clone(),
                        detail: String::new(),
                    };
                    if let Some((asset, dims, _)) = picture {
                        let label = if name.is_empty() {
                            "Picture of the slide".to_string()
                        } else {
                            format!("{name} (picture)")
                        };
                        slide
                            .elements
                            .push(cut(asset, dims, canvas, bounds, rotation, &label));
                        used_picture = true;
                        report_slide.pictured.push(Unmapped {
                            detail: "drawn from the slide's picture".into(),
                            ..u
                        });
                    } else if let Some(e) = approx {
                        let what = if matches!(e.kind, ElementKind::Text { .. }) {
                            "imported as an editable rectangle with its text"
                        } else {
                            "imported as a rectangle"
                        };
                        report_slide.editable += 1;
                        slide.elements.push(e);
                        report_slide.approximated.push(Unmapped {
                            detail: what.into(),
                            ..u
                        });
                    } else {
                        report_slide.approximated.push(Unmapped {
                            detail: "left out (no slide picture)".into(),
                            ..u
                        });
                    }
                }
            }
        }
        if used_picture {
            report_slide.picture = picture.map(|(_, _, s)| s);
        }
        let name = section.unwrap_or_default();
        match sections.last_mut() {
            Some(s) if s.name == name => s.slides.push(slide),
            _ => {
                let mut s = Section::new(name);
                s.slides.push(slide);
                sections.push(s);
            }
        }
        report.slides.push(report_slide);
    }
    for s in &mut sections {
        if s.name.is_empty() {
            s.name = "Slides".into();
        }
    }
    presentation.sections = sections;
    if !deck_notes.is_empty()
        && let Some(first) = report.slides.first_mut()
    {
        // Master and layout notes, once (they show on every slide using them).
        let mut seen = Vec::new();
        for n in deck_notes {
            if !seen.contains(&n) {
                seen.push(n);
            }
        }
        first.notes.extend(seen);
    }
    Ok(Deck {
        presentation,
        media: media.files,
        report,
    })
}

/// A master's or layout's item for one slide (fresh element IDs).
fn fresh(i: Item) -> Item {
    match i {
        Item::El(e) => Item::El(e.duplicate()),
        Item::Region {
            bounds,
            rotation,
            name,
            kind,
            approx,
        } => Item::Region {
            bounds,
            rotation,
            name,
            kind,
            approx: approx.map(|e| e.duplicate()),
        },
    }
}

/// An element showing the part of the slide's picture under `bounds` (rotated by `rotation`:
/// the box around it).
fn cut(
    asset: MediaAssetId,
    (pw, ph): (u32, u32),
    canvas: Size,
    bounds: Rect,
    rotation: f64,
    name: &str,
) -> Element {
    let (cx, cy) = (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );
    let r = rotation.to_radians();
    let (w, h) = (
        bounds.width * r.cos().abs() + bounds.height * r.sin().abs(),
        bounds.width * r.sin().abs() + bounds.height * r.cos().abs(),
    );
    let x0 = (cx - w / 2.0).clamp(0.0, canvas.width);
    let y0 = (cy - h / 2.0).clamp(0.0, canvas.height);
    let x1 = (cx + w / 2.0).clamp(0.0, canvas.width);
    let y1 = (cy + h / 2.0).clamp(0.0, canvas.height);
    let (sx, sy) = (pw as f64 / canvas.width, ph as f64 / canvas.height);
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let mut e = Element::new(Rect::new(
        round(x0),
        round(y0),
        round((x1 - x0).max(1.0)),
        round((y1 - y0).max(1.0)),
    ));
    e.name = name.into();
    e.fill = Fill::enabled(FillKind::Media {
        asset,
        display: MediaDisplay {
            scale: ScaleMode::Stretch,
            crop: EdgeInsets {
                left: round(x0 * sx),
                top: round(y0 * sy),
                right: round((canvas.width - x1) * sx),
                bottom: round((canvas.height - y1) * sy),
            },
            ..MediaDisplay::default()
        },
        playback: Default::default(),
        audio: Default::default(),
    });
    e
}

fn finite_rect(r: &Rect) -> bool {
    [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
}

/// True when `e`'s place, size, rotation and picture crop are finite numbers.
fn finite_element(e: &Element) -> bool {
    let crop = match &e.fill.kind {
        FillKind::Media { display, .. } => {
            let c = &display.crop;
            [c.left, c.top, c.right, c.bottom]
                .iter()
                .all(|v| v.is_finite())
        }
        _ => true,
    };
    finite_rect(&e.bounds) && e.rotation.is_finite() && e.opacity.is_finite() && crop
}

/// The text of the slide's title placeholder.
fn title_of(tree: roxmltree::Node) -> String {
    for sp in tree
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "sp")
    {
        let ph = sp
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "ph");
        let is_title = ph
            .and_then(|p| attr(p, "type"))
            .is_some_and(|t| t == "title" || t == "ctrTitle");
        if !is_title {
            continue;
        }
        let t: Vec<String> = sp
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "p")
            .map(|p| {
                p.descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "t")
                    .filter_map(|t| t.text())
                    .collect::<String>()
            })
            .filter(|s| !s.trim().is_empty())
            .collect();
        return t.join(" ").trim().to_string();
    }
    String::new()
}

/// A notes slide's body text.
fn notes_of(xml: &str) -> model::RichText {
    let Ok(doc) = opc::xml(xml) else {
        return model::RichText::default();
    };
    for sp in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "sp")
    {
        let body = sp
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "ph")
            .is_some_and(|p| attr(p, "type") == Some("body"));
        if body && let Some(tx) = child(sp, "txBody") {
            return text::notes(tx);
        }
    }
    model::RichText::default()
}

/// `docProps/core.xml`'s title.
fn deck_title(zip: &zip::Archive) -> Option<String> {
    let x = zip.text("docProps/core.xml").ok()?;
    let d = opc::xml(&x).ok()?;
    let t = d
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "title")?
        .text()?
        .trim()
        .to_string();
    (!t.is_empty()).then_some(t)
}

/// `docProps/core.xml`'s `dc:language`, when it is a language code.
fn deck_language(zip: &zip::Archive) -> Option<String> {
    let x = zip.text("docProps/core.xml").ok()?;
    let d = opc::xml(&x).ok()?;
    let t = d
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "language")?
        .text()?;
    model::language_code(t)
}

/// Adds the characters of each text run under `tree` to its language (`a:rPr lang`, as
/// PowerPoint marks every run with the language it was typed in). Runs without visible text
/// or a valid code don't count.
fn count_languages(tree: roxmltree::Node, counts: &mut HashMap<String, usize>) {
    for r in tree
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "r")
    {
        let chars = child(r, "t")
            .and_then(|t| t.text())
            .map_or(0, |t| t.chars().filter(|c| !c.is_whitespace()).count());
        if chars == 0 {
            continue;
        }
        if let Some(code) = child(r, "rPr")
            .and_then(|p| attr(p, "lang"))
            .and_then(model::language_code)
        {
            *counts.entry(code).or_default() += chars;
        }
    }
}

/// The language with the most characters (ties: the first code in alphabetical order, so the
/// result doesn't depend on hashing).
fn most_used(counts: &HashMap<String, usize>) -> Option<String> {
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(code, _)| code.clone())
}

/// `docProps/app.xml`'s application and version (`Microsoft Office PowerPoint 16.0000`).
fn app_version(zip: &zip::Archive) -> Option<String> {
    let x = zip.text("docProps/app.xml").ok()?;
    let d = opc::xml(&x).ok()?;
    let get = |name: &str| {
        d.descendants()
            .find(|n| n.is_element() && n.tag_name().name() == name)
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    match (get("Application"), get("AppVersion")) {
        (Some(a), Some(v)) => Some(format!("{a} {v}")),
        (a, v) => a.or(v),
    }
}

/// The deck's own thumbnail (its first slide), as PNG or JPEG bytes.
fn thumbnail(zip: &zip::Archive) -> Option<Vec<u8>> {
    let target = Rels::of(zip, "")
        .0
        .values()
        .find(|r| r.kind == "thumbnail" && !r.external)
        .map(|r| r.target.clone())
        .unwrap_or_else(|| "docProps/thumbnail.jpeg".into());
    let b = zip.read(&target).ok()?;
    image::ImageReader::new(std::io::Cursor::new(&b))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    Some(b)
}

/// A name usable in a file name.
fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .take(60)
        .collect();
    s.trim().trim_end_matches('.').to_string()
}

/// A temporary folder, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<TempDir> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("vesper-pptx-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p)?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
