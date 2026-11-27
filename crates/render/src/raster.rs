//! CPU coverage rasterization: glyphs (fill and vector stroke) and shape paths.
//!
//! Glyph outlines are rasterized with swash (analytic area coverage, no hinting, 4 horizontal
//! subpixel positions). The **text outline is a true vector stroke** of the glyph outlines
//! (round joins, width centred on the glyph edge), not a dilation, so it looks like the
//! outline Cocoa draws in ProPresenter. Results are cached per glyph / size / subpixel bin /
//! stroke width.
//!
//! Masks are uploaded to the GPU, where shadows (gaussian blur), colouring and compositing
//! happen.

use crate::geom::RectF;
use crate::text::TextLayout;
use cosmic_text::FontSystem;
use model::{Color, Shape};
use std::collections::HashMap;
use std::sync::Arc;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Angle, Format, Join, Transform, Vector};
use zeno::{Command, Mask, Point as ZPoint, Stroke as ZStroke, Style};

/// Synthetic italic slant.
const FAUX_ITALIC_DEG: f32 = 12.0;
/// Synthetic bold strength, fraction of the font size.
const FAUX_BOLD: f32 = 0.025;
/// Horizontal subpixel positions per pixel.
const SUBPIXEL_BINS: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: fontdb::ID,
    glyph: u16,
    size: u32,
    bin: u8,
    flags: u8,
    stroke: u32,
}

pub(crate) struct GlyphBitmap {
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    data: Vec<u8>,
}

pub(crate) struct GlyphRasterizer {
    ctx: ScaleContext,
    cache: HashMap<GlyphKey, Option<Arc<GlyphBitmap>>>,
    bytes: usize,
}

/// A rasterized text block, in frame px, ready for upload.
pub(crate) struct TextRaster {
    /// Integer top-left of the images in the frame.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Glyph fill colour: RGBA8, straight alpha, sRGB-encoded (alpha = coverage × colour alpha).
    pub fill: Vec<u8>,
    /// Union of the stroked outline and the fill coverage (R8), when an outline is drawn.
    pub outline: Option<Vec<u8>>,
}

impl TextRaster {
    pub fn rect(&self) -> RectF {
        RectF::new(self.x as f32, self.y as f32, self.w as f32, self.h as f32)
    }
}

impl GlyphRasterizer {
    pub fn new() -> Self {
        GlyphRasterizer {
            ctx: ScaleContext::new(),
            cache: HashMap::new(),
            bytes: 0,
        }
    }

    /// Cached glyphs and their bitmap bytes.
    pub fn size(&self) -> (u64, u64) {
        (self.cache.len() as u64, self.bytes as u64)
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph(
        &mut self,
        fs: &mut FontSystem,
        font: fontdb::ID,
        weight: fontdb::Weight,
        glyph: u16,
        size: f32,
        bin: u8,
        faux_bold: bool,
        faux_italic: bool,
        stroke: f32,
    ) -> Option<Arc<GlyphBitmap>> {
        let key = GlyphKey {
            font,
            glyph,
            size: (size * 16.0).round() as u32,
            bin,
            flags: faux_bold as u8 | ((faux_italic as u8) << 1),
            stroke: (stroke * 16.0).round() as u32,
        };
        if let Some(g) = self.cache.get(&key) {
            return g.clone();
        }
        if self.bytes > 96 << 20 {
            self.cache.clear();
            self.bytes = 0;
        }
        let f = fs.get_font(font, weight)?;
        let mut scaler = self
            .ctx
            .builder(f.as_swash())
            .size(size)
            .hint(false)
            .build();
        let sources = [Source::Outline, Source::Bitmap(StrikeWith::BestFit)];
        let mut render = Render::new(&sources);
        render
            .format(Format::Alpha)
            .offset(Vector::new(bin as f32 / SUBPIXEL_BINS, 0.0));
        if faux_bold {
            render.embolden(size * FAUX_BOLD);
        }
        if faux_italic {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(FAUX_ITALIC_DEG),
                Angle::ZERO,
            )));
        }
        let stroke_style;
        if stroke > 0.0 {
            stroke_style = {
                let mut s = ZStroke::new(stroke);
                s.join(Join::Round);
                s
            };
            render.style(stroke_style);
        }
        let img = render.render(&mut scaler, glyph);
        let bmp = img.and_then(|img| {
            if img.placement.width == 0 || img.placement.height == 0 {
                return None;
            }
            let data = match img.content {
                swash::scale::image::Content::Mask => img.data,
                // colour glyphs: keep the alpha channel as coverage
                _ => img.data.as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
            };
            Some(Arc::new(GlyphBitmap {
                left: img.placement.left,
                top: img.placement.top,
                w: img.placement.width,
                h: img.placement.height,
                data,
            }))
        });
        self.bytes += bmp.as_ref().map(|b| b.data.len() + 64).unwrap_or(64);
        self.cache.insert(key, bmp.clone());
        bmp
    }

    /// Rasterizes a laid-out text block. `origin` is the fractional frame offset added to every
    /// glyph (so pixel-aligned frames get crisp glyphs). `outline` is the stroke width in px.
    pub fn rasterize(
        &mut self,
        fs: &mut FontSystem,
        layout: &TextLayout,
        outline: Option<f32>,
        pad: u32,
        max_side: u32,
    ) -> Option<TextRaster> {
        struct Item {
            x: i32,
            y: i32,
            fill: Option<Arc<GlyphBitmap>>,
            stroke: Option<Arc<GlyphBitmap>>,
            span: usize,
        }
        let mut items = Vec::with_capacity(layout.glyphs.len());
        let mut x0 = i32::MAX;
        let mut y0 = i32::MAX;
        let mut x1 = i32::MIN;
        let mut y1 = i32::MIN;
        let mut grow = |l: i32, t: i32, r: i32, b: i32| {
            x0 = x0.min(l);
            y0 = y0.min(t);
            x1 = x1.max(r);
            y1 = y1.max(b);
        };
        for g in &layout.glyphs {
            let span = &layout.spans[g.span];
            // A glyph bigger than the largest texture cannot be drawn, and its mask (an em
            // squared and more) could need gigabytes or overflow the rasterizer's sizes.
            let extent = g.size + 2.0 * outline.unwrap_or(0.0).max(0.0);
            if extent.is_nan() || extent > max_side as f32 {
                continue;
            }
            let px = g.x.floor();
            let mut bin = ((g.x - px) * SUBPIXEL_BINS).round() as i32;
            let mut ix = px as i32;
            if bin >= SUBPIXEL_BINS as i32 {
                bin = 0;
                ix += 1;
            }
            let iy = g.y.round() as i32;
            let fill = self.glyph(
                fs,
                g.font,
                g.weight,
                g.glyph,
                g.size,
                bin as u8,
                span.faux_bold,
                span.faux_italic,
                0.0,
            );
            let stroke = match outline {
                Some(w) if w > 0.0 => self.glyph(
                    fs,
                    g.font,
                    g.weight,
                    g.glyph,
                    g.size,
                    bin as u8,
                    span.faux_bold,
                    span.faux_italic,
                    w,
                ),
                _ => None,
            };
            for b in [&fill, &stroke].into_iter().flatten() {
                grow(
                    ix + b.left,
                    iy - b.top,
                    ix + b.left + b.w as i32,
                    iy - b.top + b.h as i32,
                );
            }
            items.push(Item {
                x: ix,
                y: iy,
                fill,
                stroke,
                span: g.span,
            });
        }
        let half = outline.unwrap_or(0.0) * 0.5;
        for d in &layout.decorations {
            let r = d.rect.outset(half + 1.0);
            grow(
                r.x.floor() as i32,
                r.y.floor() as i32,
                r.right().ceil() as i32,
                r.bottom().ceil() as i32,
            );
        }
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let pad = pad as i32;
        let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
        let w = ((x1 - x0) as u32).min(max_side);
        let h = ((y1 - y0) as u32).min(max_side);
        let mut fill = vec![0u8; (w * h * 4) as usize];
        let mut out = outline.map(|_| vec![0u8; (w * h) as usize]);
        for it in &items {
            let span = &layout.spans[it.span];
            let col = color8(span.color);
            if let Some(b) = &it.fill {
                let ox = it.x + b.left - x0;
                let oy = it.y - b.top - y0;
                blit(b, ox, oy, w, h, |idx, c| {
                    over_straight(&mut fill[idx * 4..idx * 4 + 4], col, c);
                    if let Some(o) = out.as_mut() {
                        o[idx] = o[idx].max(c);
                    }
                });
            }
            if let (Some(b), Some(o)) = (&it.stroke, out.as_mut()) {
                let ox = it.x + b.left - x0;
                let oy = it.y - b.top - y0;
                blit(b, ox, oy, w, h, |idx, c| o[idx] = o[idx].max(c));
            }
        }
        for d in &layout.decorations {
            let col = color8(d.color);
            let r = d.rect.translate(-x0 as f32, -y0 as f32);
            fill_rect(r, w, h, |idx, c| {
                over_straight(&mut fill[idx * 4..idx * 4 + 4], col, c)
            });
            if let Some(o) = out.as_mut() {
                fill_rect(r.outset(half), w, h, |idx, c| o[idx] = o[idx].max(c));
            }
        }
        Some(TextRaster {
            x: x0,
            y: y0,
            w,
            h,
            fill,
            outline: out,
        })
    }
}

fn color8(c: Color) -> [f32; 4] {
    [
        c.r.clamp(0.0, 1.0),
        c.g.clamp(0.0, 1.0),
        c.b.clamp(0.0, 1.0),
        c.a.clamp(0.0, 1.0),
    ]
}

/// "Over" of a straight-alpha colour with coverage `c` onto a straight-alpha RGBA8 pixel.
fn over_straight(dst: &mut [u8], col: [f32; 4], c: u8) {
    let sa = col[3] * c as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    for k in 0..3 {
        let d = dst[k] as f32 / 255.0;
        let v = (col[k] * sa + d * da * (1.0 - sa)) / oa;
        dst[k] = (v * 255.0 + 0.5) as u8;
    }
    dst[3] = (oa * 255.0 + 0.5) as u8;
}

fn blit(b: &GlyphBitmap, ox: i32, oy: i32, w: u32, h: u32, mut f: impl FnMut(usize, u8)) {
    for j in 0..b.h as i32 {
        let y = oy + j;
        if y < 0 || y >= h as i32 {
            continue;
        }
        for i in 0..b.w as i32 {
            let x = ox + i;
            if x < 0 || x >= w as i32 {
                continue;
            }
            let c = b.data[(j * b.w as i32 + i) as usize];
            if c > 0 {
                f((y as u32 * w + x as u32) as usize, c);
            }
        }
    }
}

/// Rectangle coverage with anti-aliased fractional edges.
fn fill_rect(r: RectF, w: u32, h: u32, mut f: impl FnMut(usize, u8)) {
    let y_start = r.y.floor().max(0.0) as i32;
    let y_end = (r.bottom().ceil() as i32).min(h as i32);
    let x_start = r.x.floor().max(0.0) as i32;
    let x_end = (r.right().ceil() as i32).min(w as i32);
    for y in y_start..y_end {
        let cy = (r.bottom().min(y as f32 + 1.0) - r.y.max(y as f32)).clamp(0.0, 1.0);
        for x in x_start..x_end {
            let cx = (r.right().min(x as f32 + 1.0) - r.x.max(x as f32)).clamp(0.0, 1.0);
            let c = (cx * cy * 255.0 + 0.5) as u8;
            if c > 0 {
                f((y as u32 * w + x as u32) as usize, c);
            }
        }
    }
}

// ---- shapes ----------------------------------------------------------------------------------

/// The most points a star, or sides a polygon, is drawn with: more is indistinguishable
/// from a circle at any screen size, and a document's absurd count (`u32::MAX`) must not
/// allocate gigabytes or stall the render thread.
pub(crate) const MAX_SHAPE_POINTS: u32 = 1000;

/// Path of a parametric shape inside `r` (frame px). `None` for shapes drawn analytically.
pub(crate) fn shape_path(shape: &Shape, r: RectF) -> Vec<Command> {
    let p = |u: f32, v: f32| ZPoint::new(r.x + u * r.w, r.y + v * r.h);
    let poly = |pts: &[(f32, f32)]| {
        let mut c = Vec::with_capacity(pts.len() + 1);
        for (i, &(u, v)) in pts.iter().enumerate() {
            c.push(if i == 0 {
                Command::MoveTo(p(u, v))
            } else {
                Command::LineTo(p(u, v))
            });
        }
        c.push(Command::Close);
        c
    };
    match shape {
        Shape::IsoscelesTriangle => poly(&[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        Shape::RightTriangle => poly(&[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        Shape::Rhombus => poly(&[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
        Shape::Star {
            points,
            inner_radius,
        } => {
            let n = (*points).clamp(3, MAX_SHAPE_POINTS) as usize;
            let ir = (*inner_radius as f32).clamp(0.0, 1.0);
            let mut pts = Vec::with_capacity(n * 2);
            for i in 0..n * 2 {
                let a = std::f32::consts::PI * (i as f32 / n as f32) - std::f32::consts::FRAC_PI_2;
                let rad = if i % 2 == 0 { 0.5 } else { 0.5 * ir };
                pts.push((0.5 + rad * a.cos(), 0.5 + rad * a.sin()));
            }
            poly(&pts)
        }
        Shape::Polygon { sides } => {
            let n = (*sides).clamp(3, MAX_SHAPE_POINTS) as usize;
            let pts: Vec<(f32, f32)> = (0..n)
                .map(|i| {
                    let a =
                        std::f32::consts::TAU * (i as f32 / n as f32) - std::f32::consts::FRAC_PI_2;
                    (0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())
                })
                .collect();
            poly(&pts)
        }
        Shape::RightArrow { corner } => {
            let (cx, cy) = (corner.x as f32, corner.y as f32);
            poly(&[
                (0.0, cy),
                (cx, cy),
                (cx, 0.0),
                (1.0, 0.5),
                (cx, 1.0),
                (cx, 1.0 - cy),
                (0.0, 1.0 - cy),
            ])
        }
        Shape::DoubleArrow { corner } => {
            let (cx, cy) = (corner.x as f32, corner.y as f32);
            poly(&[
                (0.0, 0.5),
                (cx, 0.0),
                (cx, cy),
                (1.0 - cx, cy),
                (1.0 - cx, 0.0),
                (1.0, 0.5),
                (1.0 - cx, 1.0),
                (1.0 - cx, 1.0 - cy),
                (cx, 1.0 - cy),
                (cx, 1.0),
            ])
        }
        Shape::Path { path } => {
            let mut c = Vec::new();
            let pts = &path.points;
            if pts.is_empty() {
                return c;
            }
            let pp = |q: model::Point| p(q.x as f32, q.y as f32);
            c.push(Command::MoveTo(pp(pts[0].point)));
            let n = pts.len();
            let segs = if path.closed { n } else { n - 1 };
            for i in 0..segs {
                let a = &pts[i];
                let b = &pts[(i + 1) % n];
                if a.curved || b.curved {
                    let c1 = if a.curved { a.handle_out } else { a.point };
                    let c2 = if b.curved { b.handle_in } else { b.point };
                    c.push(Command::CurveTo(pp(c1), pp(c2), pp(b.point)));
                } else {
                    c.push(Command::LineTo(pp(b.point)));
                }
            }
            if path.closed {
                c.push(Command::Close);
            }
            c
        }
        // Rectangles, rounded rectangles and ellipses are drawn analytically; unknown shapes
        // fall back to their bounds.
        _ => poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
    }
}

/// A coverage mask of a path over `region` (frame px, integer-aligned).
pub(crate) struct PathMask {
    pub region: RectF,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

/// Rasterizes `cmds` over `region`. A region with a side over `max_side` px (the GPU's
/// largest texture) is rasterized at the scale that fits, still covering all of `region`.
pub(crate) fn rasterize_path(
    cmds: &[Command],
    stroke: Option<(f32, &[f32])>,
    region: RectF,
    max_side: u32,
) -> Option<PathMask> {
    if cmds.is_empty() {
        return None;
    }
    let x0 = region.x.floor();
    let y0 = region.y.floor();
    let fw = (region.right().ceil() - x0).max(1.0);
    let fh = (region.bottom().ceil() - y0).max(1.0);
    let max_side = max_side.max(1);
    // Mask px per region px.
    let k = (max_side as f32 / fw.max(fh)).min(1.0);
    let w = ((fw * k).ceil() as u32).clamp(1, max_side);
    let h = ((fh * k).ceil() as u32).clamp(1, max_side);
    let scaled;
    let cmds = if k < 1.0 {
        let p = |q: ZPoint| ZPoint::new((q.x - x0) * k, (q.y - y0) * k);
        scaled = cmds
            .iter()
            .map(|c| match *c {
                Command::MoveTo(a) => Command::MoveTo(p(a)),
                Command::LineTo(a) => Command::LineTo(p(a)),
                Command::CurveTo(a, b, c) => Command::CurveTo(p(a), p(b), p(c)),
                Command::QuadTo(a, b) => Command::QuadTo(p(a), p(b)),
                Command::Close => Command::Close,
            })
            .collect::<Vec<_>>();
        &scaled[..]
    } else {
        cmds
    };
    let mut mask = Mask::new(cmds);
    mask.size(w, h);
    if k >= 1.0 {
        mask.offset(Vector::new(-x0, -y0));
    }
    let dashes;
    match stroke {
        Some((width, pattern)) => {
            dashes = pattern.iter().map(|d| d * k).collect::<Vec<_>>();
            let mut s = ZStroke::new(width * k);
            s.join(Join::Miter);
            if !dashes.is_empty() {
                s.dash(&dashes, 0.0);
            }
            mask.style(Style::Stroke(s));
        }
        None => {
            mask.style(Style::Fill(zeno::Fill::NonZero));
        }
    }
    let (data, _) = mask.render();
    Some(PathMask {
        region: RectF::new(x0, y0, fw, fh),
        w,
        h,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A02-3: huge point counts are capped, small ones kept.
    #[test]
    fn star_and_polygon_point_counts_are_capped() {
        let r = RectF::new(0.0, 0.0, 100.0, 100.0);
        let lines = |shape: Shape| {
            shape_path(&shape, r)
                .iter()
                .filter(|c| matches!(c, Command::LineTo(_)))
                .count()
        };
        let star = |points| Shape::Star {
            points,
            inner_radius: 0.5,
        };
        // A star has two vertices per point (the first is a move).
        assert_eq!(lines(star(5)), 9);
        assert_eq!(lines(star(u32::MAX)), 2 * MAX_SHAPE_POINTS as usize - 1);
        assert_eq!(lines(Shape::Polygon { sides: 6 }), 5);
        assert_eq!(
            lines(Shape::Polygon { sides: u32::MAX }),
            MAX_SHAPE_POINTS as usize - 1
        );
        assert_eq!(lines(Shape::Polygon { sides: 0 }), 2);
    }
}
