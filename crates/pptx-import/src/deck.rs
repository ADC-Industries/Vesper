//! The package → a presentation: slides, their masters and layouts (placeholders, list styles,
//! backgrounds, decorations), shapes, pictures, groups and notes.

use crate::opc::{Rels, attr, attr_bool, attr_i64, child, children, is, path};
use crate::text::{self, BodyPr, Levels, RPr, Scale};
use crate::theme::Palette;
use crate::zip::Archive;
use crate::{MediaFile, Unmapped};
use anyhow::{Context, Result};
use model::{
    Color, Element, ElementKind, Fill, FillKind, Flip, Gradient, GradientKind, GradientStop,
    MediaAssetId, MediaDisplay, Rect, ScaleMode, Shape, Stroke, StrokeStyle,
};
use roxmltree::Node;
use std::collections::HashMap;

/// Endings of pictures Vesper shows (what the Codex takes as pictures).
pub const PICTURE_ENDINGS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "bmp", "tif", "tiff", "webp",
];

/// A placeholder of a master or layout.
#[derive(Clone, Debug, Default)]
pub struct Ph {
    pub kind: String,
    pub idx: Option<u32>,
    pub xfrm: Option<Xfrm>,
    pub body: BodyPr,
    pub levels: Levels,
}

/// A position in EMU (`a:xfrm`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Xfrm {
    pub x: f64,
    pub y: f64,
    pub cx: f64,
    pub cy: f64,
    pub rot: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

fn read_xfrm(n: Node) -> Option<Xfrm> {
    let off = child(n, "off")?;
    let ext = child(n, "ext")?;
    Some(Xfrm {
        x: attr_i64(off, "x")? as f64,
        y: attr_i64(off, "y")? as f64,
        cx: attr_i64(ext, "cx")? as f64,
        cy: attr_i64(ext, "cy")? as f64,
        rot: attr_i64(n, "rot").unwrap_or(0) as f64 / 60_000.0,
        flip_h: attr_bool(n, "flipH").unwrap_or(false),
        flip_v: attr_bool(n, "flipV").unwrap_or(false),
    })
}

/// A group's child coordinate mapping (EMU → EMU).
#[derive(Clone, Copy, Debug)]
struct Tf {
    ox: f64,
    oy: f64,
    sx: f64,
    sy: f64,
}

impl Tf {
    const ID: Tf = Tf {
        ox: 0.0,
        oy: 0.0,
        sx: 1.0,
        sy: 1.0,
    };
    fn apply(&self, x: Xfrm) -> Xfrm {
        Xfrm {
            x: self.ox + x.x * self.sx,
            y: self.oy + x.y * self.sy,
            cx: x.cx * self.sx,
            cy: x.cy * self.sy,
            ..x
        }
    }
    /// Then `inner` (a group's own mapping inside this one). Scales stay within
    /// [`Tf::MIN_SCALE`, `Tf::MAX_SCALE`], so nested groups (at most [`crate::opc::MAX_DEPTH`])
    /// can't scale a position to infinity.
    fn then(&self, inner: Tf) -> Tf {
        let scale = |v: f64| {
            if v.is_finite() {
                v.clamp(Tf::MIN_SCALE, Tf::MAX_SCALE)
            } else {
                1.0
            }
        };
        Tf {
            ox: self.ox + inner.ox * self.sx,
            oy: self.oy + inner.oy * self.sy,
            sx: scale(self.sx * inner.sx),
            sy: scale(self.sy * inner.sy),
        }
    }
    const MIN_SCALE: f64 = 1e-9;
    const MAX_SCALE: f64 = 1e9;
}

/// A slide background.
#[derive(Clone, Debug)]
pub enum Background {
    Color(Color),
    Fill(Box<Fill>),
    Unmapped(String),
}

/// What a shape became.
#[derive(Clone, Debug)]
pub enum Item {
    /// An editable element.
    El(Element),
    /// Something the model can't hold: a picture of it when there is one, else `approx`.
    Region {
        bounds: Rect,
        rotation: f64,
        name: String,
        kind: String,
        approx: Option<Element>,
    },
}

/// A master's or layout's styles and decorations.
#[derive(Clone, Debug, Default)]
pub struct Part {
    pub phs: Vec<Ph>,
    pub background: Option<Background>,
    pub items: Vec<Item>,
    pub show_master_shapes: bool,
    /// Masters only: `p:titleStyle`, `p:bodyStyle`, `p:otherStyle` (over the presentation's
    /// default text style).
    pub title: Levels,
    pub body: Levels,
    pub other: Levels,
    pub palette: Palette,
}

/// A picture part's asset and pixel size, or why it can't be used.
type Picture = Result<(MediaAssetId, (u32, u32)), String>;

/// The media of the deck, one file per part.
#[derive(Default)]
pub struct Media {
    pub files: Vec<MediaFile>,
    by_part: HashMap<String, Picture>,
    pub prefix: String,
}

impl Media {
    /// No media yet; file names start with `prefix`.
    pub fn new(prefix: String) -> Media {
        Media {
            prefix,
            ..Media::default()
        }
    }

    /// The asset of picture part `part` and its pixel size, or why it can't be used.
    pub fn picture(
        &mut self,
        zip: &Archive,
        part: &str,
    ) -> Result<(MediaAssetId, (u32, u32)), String> {
        if let Some(r) = self.by_part.get(part) {
            return r.clone();
        }
        let r = (|| {
            let file = part.rsplit('/').next().unwrap_or(part);
            let ext = file
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            if !PICTURE_ENDINGS.contains(&ext.as_str()) {
                return Err(format!(
                    "a picture in a format Vesper doesn't show (.{ext})"
                ));
            }
            let bytes = zip
                .read(part)
                .map_err(|e| format!("a picture that can't be read ({e})"))?;
            let dims = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .ok_or_else(|| format!("a picture that can't be decoded (.{ext})"))?;
            let id = MediaAssetId::new();
            self.files.push(MediaFile {
                asset: id,
                name: format!("{}{file}", self.prefix),
                bytes,
            });
            Ok((id, dims))
        })();
        self.by_part.insert(part.to_string(), r.clone());
        r
    }
}

/// Everything a shape conversion needs.
pub struct Ctx<'z, 'm> {
    pub zip: &'z Archive<'z>,
    pub palette: &'m Palette,
    pub scale: Scale,
    pub rels: &'m Rels,
    pub media: &'m mut Media,
    /// Placeholders a slide's (or layout's) placeholders inherit from: layout, then master.
    pub chain: Vec<&'m Part>,
    /// The master (text styles).
    pub master: Option<&'m Part>,
    /// The presentation's default text style.
    pub default_text: &'m Levels,
    /// Placeholders are drawn (slides) or only styles (layouts, masters).
    pub draw_placeholders: bool,
    pub notes: Vec<Unmapped>,
}

fn norm_kind(k: &str) -> &str {
    match k {
        "ctrTitle" => "title",
        "subTitle" | "obj" | "" => "body",
        k => k,
    }
}

impl<'m> Ctx<'_, 'm> {
    fn px(&self, v: f64) -> f64 {
        (v * self.scale.px * 100.0).round() / 100.0
    }

    fn rect(&self, x: Xfrm) -> Rect {
        Rect::new(self.px(x.x), self.px(x.y), self.px(x.cx), self.px(x.cy))
    }

    /// The placeholders `(kind, idx)` inherits from, nearest first.
    fn ph_chain(&self, kind: &str, idx: Option<u32>) -> Vec<&'m Ph> {
        let mut out = Vec::new();
        for (i, &part) in self.chain.iter().enumerate() {
            let last = i + 1 == self.chain.len();
            // A layout matches by index, then type; the master by type only.
            let hit = (!last || self.chain.len() == 1)
                .then(|| idx.and_then(|ix| part.phs.iter().find(|p| p.idx == Some(ix))))
                .flatten()
                .or_else(|| {
                    part.phs
                        .iter()
                        .find(|p| norm_kind(&p.kind) == norm_kind(kind))
                });
            if let Some(p) = hit {
                out.push(p);
            }
        }
        out
    }

    fn style_levels(&self, kind: Option<&str>) -> Levels {
        let Some(m) = self.master else {
            return self.default_text.clone();
        };
        match kind.map(norm_kind) {
            None => self.default_text.clone(),
            Some("title") => m.title.over(self.default_text),
            Some("body") => m.body.over(self.default_text),
            Some(_) => m.other.over(self.default_text),
        }
    }

    fn note(&mut self, kind: &str, name: &str, what: impl Into<String>) {
        self.notes.push(Unmapped {
            kind: kind.into(),
            name: name.into(),
            detail: what.into(),
        });
    }

    /// The shapes of a `p:spTree` (or group) as items, back to front.
    fn tree(&mut self, tree: Node, tf: Tf, out: &mut Vec<Item>) {
        for n in tree.children().filter(|n| n.is_element()) {
            match n.tag_name().name() {
                "sp" => self.shape(n, tf, out),
                "pic" => self.picture(n, tf, out),
                "grpSp" => self.group(n, tf, out),
                "graphicFrame" => self.frame(n, tf, out),
                "cxnSp" => self.connector(n, tf, out),
                "contentPart" => self.note("ink", "", "ink drawing not imported"),
                "AlternateContent" => {
                    // Prefer the fallback (plain DrawingML) over extension content.
                    if let Some(f) = child(n, "Fallback").or_else(|| child(n, "Choice")) {
                        self.tree(f, tf, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn nv<'a, 'i>(n: Node<'a, 'i>) -> (String, bool, Option<Node<'a, 'i>>) {
        let nv = n
            .children()
            .find(|c| c.is_element() && c.tag_name().name().starts_with("nv"));
        let c = nv.and_then(|v| child(v, "cNvPr"));
        let name = c
            .and_then(|c| attr(c, "name"))
            .unwrap_or_default()
            .to_string();
        let hidden = c.and_then(|c| attr_bool(c, "hidden")).unwrap_or(false);
        let ph = nv
            .and_then(|v| child(v, "nvPr"))
            .and_then(|p| child(p, "ph"));
        (name, hidden, ph)
    }

    fn group(&mut self, n: Node, tf: Tf, out: &mut Vec<Item>) {
        let (_, hidden, _) = Self::nv(n);
        if hidden {
            return;
        }
        let x = path(n, &["grpSpPr", "xfrm"]);
        let inner = x.and_then(|x| {
            let off = child(x, "off")?;
            let ext = child(x, "ext")?;
            let choff = child(x, "chOff")?;
            let chext = child(x, "chExt")?;
            let (cx, cy) = (attr_i64(chext, "cx")? as f64, attr_i64(chext, "cy")? as f64);
            let sx = if cx > 0.0 {
                attr_i64(ext, "cx")? as f64 / cx
            } else {
                1.0
            };
            let sy = if cy > 0.0 {
                attr_i64(ext, "cy")? as f64 / cy
            } else {
                1.0
            };
            Some(Tf {
                ox: attr_i64(off, "x")? as f64 - attr_i64(choff, "x")? as f64 * sx,
                oy: attr_i64(off, "y")? as f64 - attr_i64(choff, "y")? as f64 * sy,
                sx,
                sy,
            })
        });
        if x.and_then(|x| attr_i64(x, "rot")).unwrap_or(0) != 0 {
            self.note("group", "", "a rotated group's shapes are placed unrotated");
        }
        self.tree(n, tf.then(inner.unwrap_or(Tf::ID)), out);
    }

    fn frame(&mut self, n: Node, tf: Tf, out: &mut Vec<Item>) {
        let (name, hidden, _) = Self::nv(n);
        if hidden {
            return;
        }
        let uri = path(n, &["graphic", "graphicData"])
            .and_then(|g| attr(g, "uri"))
            .unwrap_or_default();
        let kind = if uri.contains("table") {
            "table"
        } else if uri.contains("chart") {
            "chart"
        } else if uri.contains("diagram") {
            "smartArt"
        } else if uri.contains("ole") {
            "embedded object"
        } else {
            "object"
        };
        let Some(x) = child(n, "xfrm").and_then(read_xfrm) else {
            return;
        };
        let x = tf.apply(x);
        out.push(Item::Region {
            bounds: self.rect(x),
            rotation: x.rot,
            name,
            kind: kind.into(),
            approx: None,
        });
    }

    fn connector(&mut self, n: Node, tf: Tf, out: &mut Vec<Item>) {
        let (name, hidden, _) = Self::nv(n);
        if hidden {
            return;
        }
        let Some(x) = path(n, &["spPr", "xfrm"]).and_then(read_xfrm) else {
            return;
        };
        let x = tf.apply(x);
        // A line's box can be 0 wide or high: give the picture a little room.
        let mut r = self.rect(x);
        let pad = 4.0;
        r = Rect::new(
            r.x - pad,
            r.y - pad,
            r.width + 2.0 * pad,
            r.height + 2.0 * pad,
        );
        out.push(Item::Region {
            bounds: r,
            rotation: x.rot,
            name,
            kind: "line".into(),
            approx: None,
        });
    }

    fn xfrm_of(&self, sp_pr: Option<Node>, phs: &[&Ph]) -> Option<Xfrm> {
        sp_pr
            .and_then(|s| child(s, "xfrm"))
            .and_then(read_xfrm)
            .or_else(|| phs.iter().find_map(|p| p.xfrm))
    }

    /// The fill of `sp_pr`: `None` when it sets none (inherit / style).
    fn fill(&mut self, sp_pr: Node) -> Option<Result<Fill, String>> {
        for f in sp_pr.children().filter(|n| n.is_element()) {
            match f.tag_name().name() {
                "noFill" => return Some(Ok(Fill::default())),
                "solidFill" => {
                    return Some(Ok(self
                        .palette
                        .color_in(f)
                        .map(Fill::color)
                        .unwrap_or_default()));
                }
                "gradFill" => return Some(self.gradient(f).map(Fill::enabled)),
                "blipFill" => return Some(self.blip_fill(f)),
                "pattFill" => return Some(Err("a pattern fill".into())),
                "grpFill" => return Some(Ok(Fill::default())),
                _ => {}
            }
        }
        None
    }

    fn gradient(&self, g: Node) -> Result<FillKind, String> {
        let stops: Vec<GradientStop> = child(g, "gsLst")
            .map(|l| {
                children(l, "gs")
                    .filter_map(|s| {
                        Some(GradientStop {
                            color: self.palette.color_in(s)?,
                            position: attr_i64(s, "pos").unwrap_or(0) as f64 / 100_000.0,
                            midpoint: 0.5,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if stops.len() < 2 {
            return Err("a gradient without stops".into());
        }
        let mut stops = stops;
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        let (kind, angle) = if let Some(lin) = child(g, "lin") {
            (
                GradientKind::Linear,
                attr_i64(lin, "ang").unwrap_or(0) as f64 / 60_000.0,
            )
        } else if child(g, "path").is_some() {
            (GradientKind::Radial, 0.0)
        } else {
            (GradientKind::Linear, 90.0)
        };
        Ok(FillKind::Gradient {
            gradient: Gradient {
                kind,
                angle,
                length: 1.0,
                stops,
            },
        })
    }

    fn blip_fill(&mut self, f: Node) -> Result<Fill, String> {
        let (asset, display) = self.blip(f)?;
        Ok(Fill::enabled(FillKind::Media {
            asset,
            display,
            playback: Default::default(),
            audio: Default::default(),
        }))
    }

    /// A `blipFill`'s picture (stretched, cropped as `a:srcRect` says).
    fn blip(&mut self, f: Node) -> Result<(MediaAssetId, MediaDisplay), String> {
        let blip = child(f, "blip").ok_or("a picture without its image")?;
        if attr(blip, "link").is_some() && attr(blip, "embed").is_none() {
            return Err("a linked (not embedded) picture".into());
        }
        let id = attr(blip, "embed").ok_or("a picture without its image")?;
        let part = self
            .rels
            .target(id)
            .ok_or("a picture whose image is missing")?
            .to_string();
        let (asset, (w, h)) = self.media.picture(self.zip, &part)?;
        let mut display = MediaDisplay {
            scale: ScaleMode::Stretch,
            ..MediaDisplay::default()
        };
        if let Some(src) = child(f, "srcRect") {
            let c = |k: &str, side: u32| {
                let v = attr_i64(src, k).unwrap_or(0).max(0) as f64 / 100_000.0;
                (v * side as f64).round()
            };
            display.crop = model::EdgeInsets {
                left: c("l", w),
                top: c("t", h),
                right: c("r", w),
                bottom: c("b", h),
            };
        }
        if child(f, "tile").is_some() {
            return Err("a tiled picture fill".into());
        }
        Ok((asset, display))
    }

    fn stroke(&self, sp_pr: Option<Node>, style: Option<Node>) -> Stroke {
        let ln = sp_pr.and_then(|s| child(s, "ln"));
        let from_style = || {
            let r = style.and_then(|s| child(s, "lnRef"))?;
            if attr_i64(r, "idx").unwrap_or(0) == 0 {
                return None;
            }
            self.palette.color_in(r)
        };
        let color = match ln {
            Some(l) if child(l, "noFill").is_some() => return Stroke::default(),
            Some(l) => crate::theme::solid_fill(self.palette, l).or_else(from_style),
            None => from_style(),
        };
        let Some(color) = color else {
            return Stroke::default();
        };
        let width = ln
            .and_then(|l| attr_i64(l, "w"))
            .map(|w| w as f64)
            .unwrap_or(9_525.0);
        let dash = ln
            .and_then(|l| child(l, "prstDash"))
            .and_then(|d| attr(d, "val"))
            .unwrap_or("solid");
        Stroke {
            enabled: true,
            width: self.px(width).max(0.5),
            color,
            style: match dash {
                "solid" => StrokeStyle::Solid,
                "dot" | "sysDot" => StrokeStyle::SquareDash,
                "dash" | "sysDash" => StrokeStyle::ShortDash,
                _ => StrokeStyle::LongDash,
            },
            pattern: Vec::new(),
        }
    }

    fn geometry(sp_pr: Option<Node>) -> Result<Shape, String> {
        let Some(sp) = sp_pr else {
            return Ok(Shape::Rectangle);
        };
        if child(sp, "custGeom").is_some() {
            return Err("a freeform shape".into());
        }
        let Some(g) = child(sp, "prstGeom") else {
            return Ok(Shape::Rectangle);
        };
        let adj = |name: &str| {
            child(g, "avLst").and_then(|l| {
                children(l, "gd")
                    .find(|d| attr(*d, "name") == Some(name))
                    .and_then(|d| attr(d, "fmla"))
                    .and_then(|f| f.strip_prefix("val "))
                    .and_then(|v| v.trim().parse::<f64>().ok())
            })
        };
        Ok(match attr(g, "prst").unwrap_or("rect") {
            "rect" => Shape::Rectangle,
            "roundRect" => Shape::RoundedRectangle {
                roundness: (adj("adj").unwrap_or(16_667.0) / 100_000.0).clamp(0.0, 0.5),
            },
            "ellipse" => Shape::Ellipse,
            "triangle" if adj("adj").is_none_or(|a| a == 50_000.0) => Shape::IsoscelesTriangle,
            "rtTriangle" => Shape::RightTriangle,
            "diamond" => Shape::Rhombus,
            other => return Err(format!("a “{other}” shape")),
        })
    }

    fn base(&self, x: Xfrm, name: &str) -> Element {
        let mut e = Element::new(self.rect(x));
        e.name = name.to_string();
        e.rotation = x.rot;
        e.flip = Flip {
            horizontal: x.flip_h,
            vertical: x.flip_v,
        };
        e
    }

    fn shadow(&self, sp_pr: Option<Node>) -> Option<model::Shadow> {
        let s = sp_pr
            .and_then(|s| child(s, "effectLst"))
            .and_then(|e| child(e, "outerShdw"))?;
        text::read_shadow(s, self.palette).map(|s| text::shadow(s, self.scale.px))
    }

    fn shape(&mut self, n: Node, tf: Tf, out: &mut Vec<Item>) {
        let (name, hidden, ph) = Self::nv(n);
        if hidden {
            return;
        }
        let ph_kind = ph.map(|p| attr(p, "type").unwrap_or("body").to_string());
        let ph_idx = ph.and_then(|p| attr_i64(p, "idx")).map(|v| v as u32);
        if ph.is_some() && !self.draw_placeholders {
            return;
        }
        let tx = child(n, "txBody");
        let has_text = tx.is_some_and(|t| {
            children(t, "p").any(|p| {
                children(p, "r").any(|r| {
                    child(r, "t")
                        .and_then(|t| t.text())
                        .is_some_and(|s| !s.is_empty())
                }) || children(p, "fld").next().is_some()
            })
        });
        let sp_pr = child(n, "spPr");
        let style = child(n, "style");
        let phs: Vec<&'m Ph> = match &ph_kind {
            Some(k) => self.ph_chain(k, ph_idx),
            None => Vec::new(),
        };
        // An empty placeholder isn't shown in a slide show.
        if ph.is_some() && !has_text && sp_pr.and_then(|s| self.fill_probe(s)).is_none() {
            return;
        }
        let Some(x) = self.xfrm_of(sp_pr, &phs) else {
            return;
        };
        let x = tf.apply(x);
        let mut el = self.base(x, &name);
        let mut problems = Vec::new();
        let geometry = Self::geometry(sp_pr).unwrap_or_else(|e| {
            problems.push(e);
            Shape::Rectangle
        });
        let fill = match sp_pr.and_then(|s| self.fill(s)) {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                problems.push(e);
                Fill::default()
            }
            None => style
                .and_then(|s| child(s, "fillRef"))
                .filter(|r| attr_i64(*r, "idx").unwrap_or(0) > 0)
                .and_then(|r| self.palette.color_in(r))
                .map(Fill::color)
                .unwrap_or_default(),
        };
        el.fill = fill;
        el.stroke = self.stroke(sp_pr, style);
        if let Some(s) = self.shadow(sp_pr) {
            el.shadow = s;
        }
        if has_text {
            let mut levels = self.style_levels(ph_kind.as_deref());
            let mut body = BodyPr::default();
            for p in phs.iter().rev() {
                levels = p.levels.over(&levels);
                body = p.body.over(&body);
            }
            let base = RPr {
                color: style
                    .and_then(|s| child(s, "fontRef"))
                    .and_then(|r| self.palette.color_in(r)),
                ..RPr::default()
            };
            let c = text::convert(
                tx.expect("has text"),
                &levels,
                &body,
                &base,
                self.palette,
                self.scale,
            );
            for u in c.unmapped {
                self.note("text", &name, format!("{u} shown as plain horizontal text"));
            }
            el.kind = ElementKind::Text {
                shape: geometry,
                text: c.text,
            };
        } else {
            el.kind = ElementKind::Shape { shape: geometry };
        }
        if problems.is_empty() {
            out.push(Item::El(el));
        } else {
            out.push(Item::Region {
                bounds: el.bounds,
                rotation: el.rotation,
                name,
                kind: problems.join(", "),
                approx: Some(el),
            });
        }
    }

    /// True when `sp_pr` sets a visible fill (an empty placeholder with a fill still shows).
    fn fill_probe(&self, sp_pr: Node) -> Option<()> {
        sp_pr
            .children()
            .any(|f| is(&f, "solidFill") || is(&f, "gradFill") || is(&f, "blipFill"))
            .then_some(())
    }

    fn picture(&mut self, n: Node, tf: Tf, out: &mut Vec<Item>) {
        let (name, hidden, ph) = Self::nv(n);
        if hidden {
            return;
        }
        let nvpr = n
            .children()
            .find(|c| c.is_element() && c.tag_name().name().starts_with("nv"))
            .and_then(|v| child(v, "nvPr"));
        let movie = nvpr.is_some_and(|p| {
            child(p, "videoFile").is_some()
                || child(p, "audioFile").is_some()
                || child(p, "quickTimeFile").is_some()
        });
        let phs: Vec<&'m Ph> = match ph {
            Some(p) => {
                let k = attr(p, "type").unwrap_or("pic").to_string();
                let idx = attr_i64(p, "idx").map(|v| v as u32);
                self.ph_chain(&k, idx)
            }
            None => Vec::new(),
        };
        let sp_pr = child(n, "spPr");
        let Some(x) = self.xfrm_of(sp_pr, &phs) else {
            return;
        };
        let x = tf.apply(x);
        if movie {
            self.note(
                "video",
                &name,
                "video or sound not imported (its poster picture is)",
            );
        }
        let mut el = self.base(x, &name);
        let shape = Self::geometry(sp_pr);
        let blip = child(n, "blipFill").ok_or_else(|| "a picture without its image".to_string());
        match blip.and_then(|b| self.blip_fill(b)) {
            Ok(fill) => {
                el.fill = fill;
                el.stroke = self.stroke(sp_pr, child(n, "style"));
                if let Some(s) = self.shadow(sp_pr) {
                    el.shadow = s;
                }
                match shape {
                    Ok(s) => {
                        el.kind = ElementKind::Shape { shape: s };
                        out.push(Item::El(el));
                    }
                    Err(e) => out.push(Item::Region {
                        bounds: el.bounds,
                        rotation: el.rotation,
                        name,
                        kind: format!("a picture cut to {e}"),
                        approx: Some(el),
                    }),
                }
            }
            Err(e) => out.push(Item::Region {
                bounds: el.bounds,
                rotation: el.rotation,
                name,
                kind: e,
                approx: None,
            }),
        }
    }

    /// A background (`p:bg`).
    pub fn background(&mut self, bg: Node) -> Option<Background> {
        if let Some(pr) = child(bg, "bgPr") {
            return match self.fill(pr)? {
                Ok(f) => Some(match (&f.enabled, &f.kind) {
                    (true, FillKind::Color { color }) => Background::Color(*color),
                    (false, _) => Background::Color(Color::WHITE),
                    _ => Background::Fill(Box::new(f)),
                }),
                Err(e) => Some(Background::Unmapped(e)),
            };
        }
        let r = child(bg, "bgRef")?;
        let color = self.palette.color_in(r);
        match (attr_i64(r, "idx").unwrap_or(0), color) {
            (1001, Some(c)) => Some(Background::Color(c)),
            (_, Some(c)) => {
                self.note(
                    "background",
                    "",
                    "a theme background style drawn in its main colour",
                );
                Some(Background::Color(c))
            }
            _ => None,
        }
    }
}

/// Reads a master or layout part.
#[allow(clippy::too_many_arguments)]
pub fn read_part(
    zip: &Archive,
    part_name: &str,
    palette: &Palette,
    scale: Scale,
    media: &mut Media,
    chain: Vec<&Part>,
    master: Option<&Part>,
    default_text: &Levels,
    notes: &mut Vec<Unmapped>,
) -> Result<Part> {
    let xml = zip.text(part_name)?;
    let doc = crate::opc::xml(&xml).with_context(|| format!("{part_name} is not XML"))?;
    let root = doc.root_element();
    let rels = Rels::of(zip, part_name);
    let mut part = Part {
        show_master_shapes: attr_bool(root, "showMasterSp").unwrap_or(true),
        palette: palette.clone(),
        ..Part::default()
    };
    if let Some(ts) = child(root, "txStyles") {
        let lv = |name: &str| {
            child(ts, name)
                .map(|n| Levels::read(n, palette))
                .unwrap_or_default()
        };
        part.title = lv("titleStyle");
        part.body = lv("bodyStyle");
        part.other = lv("otherStyle");
    }
    let tree = path(root, &["cSld", "spTree"]).context("a slide part without shapes")?;
    // Placeholders: positions and styles only.
    collect_placeholders(tree, palette, &mut part.phs);
    let mut ctx = Ctx {
        zip,
        palette,
        scale,
        rels: &rels,
        media,
        chain,
        master,
        default_text,
        draw_placeholders: false,
        notes: Vec::new(),
    };
    if let Some(bg) = path(root, &["cSld", "bg"]) {
        part.background = ctx.background(bg);
    }
    let mut items = Vec::new();
    ctx.tree(tree, Tf::ID, &mut items);
    notes.append(&mut ctx.notes);
    part.items = items;
    Ok(part)
}

fn collect_placeholders(tree: Node, p: &Palette, out: &mut Vec<Ph>) {
    for n in tree.children().filter(|n| n.is_element()) {
        if is(&n, "grpSp") {
            collect_placeholders(n, p, out);
            continue;
        }
        let (_, _, ph) = Ctx::nv(n);
        let Some(ph) = ph else { continue };
        let tx = child(n, "txBody");
        out.push(Ph {
            kind: attr(ph, "type").unwrap_or("body").to_string(),
            idx: attr_i64(ph, "idx").map(|v| v as u32),
            xfrm: path(n, &["spPr", "xfrm"]).and_then(read_xfrm),
            body: tx
                .and_then(|t| child(t, "bodyPr"))
                .map(BodyPr::read)
                .unwrap_or_default(),
            levels: tx
                .and_then(|t| child(t, "lstStyle"))
                .map(|l| Levels::read(l, p))
                .unwrap_or_default(),
        });
    }
}

impl Ctx<'_, '_> {
    /// [`Ctx::tree`] from the top of a slide.
    pub fn top(&mut self, tree: Node, out: &mut Vec<Item>) {
        self.tree(tree, Tf::ID, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_mapping_composes() {
        let outer = Tf {
            ox: 100.0,
            oy: 0.0,
            sx: 2.0,
            sy: 2.0,
        };
        let inner = Tf {
            ox: 10.0,
            oy: 10.0,
            sx: 0.5,
            sy: 0.5,
        };
        let x = Xfrm {
            x: 4.0,
            y: 4.0,
            cx: 8.0,
            cy: 8.0,
            ..Xfrm::default()
        };
        let a = outer.then(inner).apply(x);
        let b = outer.apply(inner.apply(x));
        assert_eq!(a, b);
        assert_eq!(a.x, 100.0 + (10.0 + 2.0) * 2.0);
        assert_eq!(a.cx, 8.0);
    }
}
