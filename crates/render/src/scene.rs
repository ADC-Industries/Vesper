//! Slides → GPU draws.
//!
//! Each element is drawn in its own **frame**: element-local pixels at output resolution
//! (canvas px × screen scale), x right / y down, mapped to the target by an affine transform
//! (screen scale, position, rotation about the element centre, flips). Unrotated elements get a
//! pixel-snapped frame, so glyph and mask textures are sampled exactly at texel centres and
//! stay crisp.
//!
//! Draw order inside an element: fill (colour / gradient / image), stroke, text (line masks and
//! highlights, text shadow, outline, glyph fill). Elements with a drop shadow (or a cut-out
//! text fill) are first rendered into their own texture, whose alpha is blurred for the shadow.

use crate::decode::ImageDecoder;
use crate::geom::{Affine, RectF, place_media, premul_linear, swaps_axes, uv_matrix};
use crate::gpu::{Draw, FLAG_CLIP_PAINT, Gpu, Pass, Pipe, Tex, Uniforms, channel, cov, paint};
use crate::media::{Fetch, Lookup, MediaCache, MediaResolver};
use crate::raster::{GlyphRasterizer, rasterize_path, shape_path};
use crate::report::ReportState;
use crate::text::TextEngine;
use model::{
    Color, EffectKind, Element, ElementKind, FeatherStyle, Fill, FillKind, Gradient, GradientKind,
    MediaDisplay, Shape, Slide, StrokeStyle, TextBox, VisibilityCondition,
};

/// Shared state for building one batch of passes.
pub(crate) struct Scene<'a> {
    pub gpu: &'a Gpu,
    pub text: &'a mut TextEngine,
    pub glyphs: &'a mut GlyphRasterizer,
    pub media: &'a mut MediaCache,
    pub decoder: &'a ImageDecoder,
    /// What to do with images that are not decoded yet.
    pub fetch: Fetch,
    /// Box images are decoded for (the target size, px).
    pub image_box: (u32, u32),
    /// Images left out because they are still decoding.
    pub pending: usize,
    pub report: &'a mut ReportState,
    /// Passes that must run before the pass the draws go into (uploads, blurs, element
    /// textures), in dependency order.
    pub pre: Vec<Pass>,
    /// Transient textures kept alive until submission.
    pub keep: Vec<Tex>,
}

/// An element's local frame.
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// Frame px → target px.
    xf: Affine,
    /// The element bounds in frame px.
    rect: RectF,
    /// Canvas px → frame px.
    scale: f32,
}

const BLUR_MIN_SIGMA: f32 = 0.3;

fn sigma_of(radius_px: f32) -> f32 {
    // ProPresenter / Cocoa blur radius ≈ 2 σ of a gaussian.
    radius_px * 0.5
}

fn taps(sigma: f32) -> u32 {
    (sigma * 3.0).ceil().clamp(1.0, 250.0) as u32
}

impl<'a> Scene<'a> {
    /// Draws the slide (scaled from `canvas` to fit `target`, letterboxed) into `draws`.
    pub fn slide(
        &mut self,
        slide: &Slide,
        canvas: (f32, f32),
        target: (u32, u32),
        resolver: Option<&dyn MediaResolver>,
        draws: &mut Vec<Draw>,
    ) {
        let (tw, th) = (target.0 as f32, target.1 as f32);
        let (cw, ch) = (canvas.0.max(1.0), canvas.1.max(1.0));
        let s = (tw / cw).min(th / ch);
        let ox = (tw - cw * s) * 0.5;
        let oy = (th - ch * s) * 0.5;
        let canvas_xf = Affine::translate(ox, oy).then_after(&Affine::scale(s, s));
        if let Some(bg) = slide.background {
            let r = RectF::new(ox, oy, cw * s, ch * s);
            draws.push(solid_rect(&Affine::IDENTITY, r, premul_linear(bg, 1.0)));
        }
        for el in &slide.elements {
            if el.hidden || !visible(slide, el) {
                continue;
            }
            if el.bounds.width <= 0.0 && el.bounds.height <= 0.0 {
                continue;
            }
            self.element(el, &canvas_xf, s, (tw, th), resolver, draws);
        }
    }

    fn frame(&self, el: &Element, canvas_xf: &Affine, s: f32) -> Frame {
        let b = &el.bounds;
        let (w, h) = (b.width as f32 * s, b.height as f32 * s);
        let (ox, oy) = canvas_xf.apply(b.x as f32, b.y as f32);
        let rot = el.rotation as f32 % 360.0;
        let flip = el.flip.horizontal || el.flip.vertical;
        if rot == 0.0 && !flip {
            let (ix, iy) = (ox.floor(), oy.floor());
            return Frame {
                xf: Affine::translate(ix, iy),
                rect: RectF::new(ox - ix, oy - iy, w, h),
                scale: s,
            };
        }
        let (cx, cy) = (w * 0.5, h * 0.5);
        let local = Affine::translate(cx, cy)
            .then_after(&Affine::rotate(rot))
            .then_after(&Affine::scale(
                if el.flip.horizontal { -1.0 } else { 1.0 },
                if el.flip.vertical { -1.0 } else { 1.0 },
            ))
            .then_after(&Affine::translate(-cx, -cy));
        Frame {
            xf: Affine::translate(ox, oy).then_after(&local),
            rect: RectF::new(0.0, 0.0, w, h),
            scale: s,
        }
    }

    fn element(
        &mut self,
        el: &Element,
        canvas_xf: &Affine,
        s: f32,
        target: (f32, f32),
        resolver: Option<&dyn MediaResolver>,
        draws: &mut Vec<Draw>,
    ) {
        let (shape, text) = match &el.kind {
            ElementKind::Shape { shape } => (shape, None),
            ElementKind::Text { shape, text } => (shape, Some(text)),
            ElementKind::Unknown => {
                self.report.unsupported("element kind (unknown)");
                return;
            }
        };
        let mut frame = self.frame(el, canvas_xf, s);
        let opacity = el.opacity.clamp(0.0, 1.0) as f32;
        if opacity <= 0.0 {
            return;
        }
        let cutout = text.is_some_and(|t| t.fill.enabled && t.fill.kind == FillKind::CutOut);
        let shadow = el.shadow.enabled && el.shadow.opacity > 0.0;
        // Text first: its layout can resize the box (resizeBox) and its extent sizes the layer.
        let prepared = text.and_then(|t| self.prepare_text(t, &mut frame));

        // Element opacity fades the element as one group: parts that overlap (a fill or
        // stroke under the text; a text outline, shadow or highlight under the glyphs) are
        // drawn opaque into a layer first, or they would show through each other.
        let parts_with_text =
            text.is_some_and(|t| !t.is_blank()) && (el.fill.enabled || el.stroke.enabled);
        let overlapping_text = prepared.as_ref().is_some_and(|p| {
            p.fill.is_some() && (p.outline.is_some() || p.shadow.is_some() || !p.boxes.is_empty())
        });
        let isolate = shadow || cutout || (opacity < 1.0 && (parts_with_text || overlapping_text));

        if !isolate {
            let mut parts = Vec::new();
            self.parts(
                el,
                shape,
                &frame,
                &frame.xf,
                prepared.as_ref(),
                resolver,
                opacity,
                &mut parts,
            );
            draws.extend(parts);
            return;
        }

        // Isolated: render the element into its own texture (frame px), then composite.
        let mut bounds = frame.rect.outset(el.stroke.width as f32 * s * 0.5 + 2.0);
        if let Some(p) = &prepared {
            bounds = bounds.union(&p.extent);
        }
        let sig = sigma_of(el.shadow.radius as f32 * s);
        let pad = if shadow { taps(sig) as f32 + 2.0 } else { 0.0 };
        let mut bounds = bounds.outset(pad);
        if el.rotation as f32 % 360.0 == 0.0 && !el.flip.horizontal && !el.flip.vertical {
            // An upright element's layer only needs the part that can reach the target (with
            // what its shadow's offset and blur pull in): a huge box (a scrolling strip's
            // layout box) would otherwise be cut at the largest texture from its far start.
            let (fx, fy) = frame.xf.apply(0.0, 0.0);
            let off = if shadow {
                let o = el.shadow.offset_vector();
                (o.x.abs().max(o.y.abs()) as f32) * s
            } else {
                0.0
            };
            let reach = RectF::new(-fx, -fy, target.0, target.1).outset(pad + off + 2.0);
            let x0 = bounds.x.max(reach.x);
            let y0 = bounds.y.max(reach.y);
            let x1 = bounds.right().min(reach.right());
            let y1 = bounds.bottom().min(reach.bottom());
            if x1 <= x0 || y1 <= y0 {
                return;
            }
            bounds = RectF::new(x0, y0, x1 - x0, y1 - y0);
        }
        let ex = bounds.x.floor();
        let ey = bounds.y.floor();
        let ew = ((bounds.right().ceil() - ex) as u32).clamp(1, self.gpu.max_texture);
        let eh = ((bounds.bottom().ceil() - ey) as u32).clamp(1, self.gpu.max_texture);
        let region = RectF::new(ex, ey, ew as f32, eh as f32);
        let layer = self.gpu.linear_target("element", ew, eh);
        let to_layer = Affine::translate(-ex, -ey);
        let mut parts = Vec::new();
        self.parts(
            el,
            shape,
            &frame,
            &to_layer,
            prepared.as_ref(),
            resolver,
            1.0,
            &mut parts,
        );
        self.pre.push(Pass {
            label: "element layer",
            target: layer.view.clone(),
            size: (ew, eh),
            clear: Some(wgpu::Color::TRANSPARENT),
            draws: parts,
        });
        if shadow {
            let off = el.shadow.offset_vector();
            let (dx, dy) = (off.x as f32 * s, off.y as f32 * s);
            let color = premul_linear(el.shadow.color, el.shadow.opacity as f32 * opacity);
            let src = layer.clone();
            self.shadow_draws(
                &src,
                channel::A,
                region,
                sig,
                dx,
                dy,
                color,
                &frame.xf,
                draws,
            );
        }
        let u = Uniforms::new(&frame.xf, region.corners())
            .paint(paint::TEXTURE_PREMUL)
            .paint_rect(region.xywh())
            .opacity(opacity);
        draws.push(Draw::new(Pipe::Over, u).with_paint(&layer.view));
        self.keep.push(layer);
    }

    /// Blurred (or hard) shadow of `src` (covering `region` in the frame), offset by (dx, dy).
    #[allow(clippy::too_many_arguments)]
    fn shadow_draws(
        &mut self,
        src: &Tex,
        ch: u32,
        region: RectF,
        sigma: f32,
        dx: f32,
        dy: f32,
        color: [f32; 4],
        xf: &Affine,
        draws: &mut Vec<Draw>,
    ) {
        let (w, h) = (src.width, src.height);
        let (mask_view, mask_ch) = if sigma >= BLUR_MIN_SIGMA {
            let t1 = self.gpu.linear_target("blur h", w, h);
            let t2 = self.gpu.linear_target("blur v", w, h);
            let local = RectF::new(0.0, 0.0, w as f32, h as f32);
            let mut blur = |from: &wgpu::TextureView, from_ch: u32, to: &Tex, dir: (f32, f32)| {
                let mut u = Uniforms::new(&Affine::IDENTITY, local.corners())
                    .mask(local.xywh(), from_ch)
                    .coverage(cov::BLUR);
                u.blur = [
                    dir.0 / w as f32,
                    dir.1 / h as f32,
                    sigma,
                    taps(sigma) as f32,
                ];
                self.pre.push(Pass {
                    label: "blur",
                    target: to.view.clone(),
                    size: (w, h),
                    clear: None,
                    draws: vec![Draw::new(Pipe::Replace, u).with_mask(from)],
                });
            };
            blur(&src.view, ch, &t1, (1.0, 0.0));
            blur(&t1.view, channel::A, &t2, (0.0, 1.0));
            let v = t2.view.clone();
            self.keep.push(t1);
            self.keep.push(t2);
            (v, channel::A)
        } else {
            (src.view.clone(), ch)
        };
        let r = region.translate(dx, dy);
        let u = Uniforms::new(xf, r.corners())
            .mask(r.xywh(), mask_ch)
            .color(color);
        draws.push(Draw::new(Pipe::Over, u).with_mask(&mask_view));
    }

    /// Element parts (fill, stroke, text) through `xf` (frame px → destination px).
    #[allow(clippy::too_many_arguments)]
    fn parts(
        &mut self,
        el: &Element,
        shape: &Shape,
        frame: &Frame,
        xf: &Affine,
        text: Option<&PreparedText>,
        resolver: Option<&dyn MediaResolver>,
        opacity: f32,
        out: &mut Vec<Draw>,
    ) {
        let r = frame.rect;
        let s = frame.scale;
        let stroke_w = if el.stroke.enabled {
            (el.stroke.width as f32 * s).max(0.0)
        } else {
            0.0
        };
        let feather = if el.feather.enabled {
            el.feather.radius.clamp(0.0, 1.0) as f32 * r.w.min(r.h) * 0.5
        } else {
            0.0
        };
        let feather_style = match el.feather.style {
            FeatherStyle::Center => 1.0,
            FeatherStyle::Outside => 2.0,
            _ => 0.0,
        };
        let geo = ShapeGeo::new(shape, r);

        // fill
        if el.fill.enabled {
            let fill_cov = self.fill_coverage(&geo, feather, feather_style);
            if let Some(mut u) = self.fill_paint(&el.fill, r, s, resolver, xf, &fill_cov, opacity) {
                fill_cov.apply(&mut u);
                let mut d = Draw::new(Pipe::Over, u.0);
                if let Some(m) = &fill_cov.mask {
                    d = d.with_mask(m);
                }
                if let Some(p) = &u.1 {
                    d = d.with_paint(p);
                }
                out.push(d);
            }
        }
        // stroke
        if stroke_w > 0.0 && el.stroke.color.a > 0.0 {
            let color = premul_linear(el.stroke.color, opacity);
            match &geo {
                ShapeGeo::RRect { rect, radius } if !geo_dashed(&el.stroke.style) => {
                    for (off, lw) in stroke_lines(&el.stroke.style, stroke_w) {
                        let line = rect.outset(off);
                        let q = line.outset(lw * 0.5 + 1.0);
                        let mut u = Uniforms::new(xf, q.corners())
                            .coverage(cov::RRECT_STROKE)
                            .color(color);
                        u.shape = line.corners();
                        let rad = if *radius > 0.0 {
                            (*radius + off).max(0.0)
                        } else {
                            0.0
                        };
                        u.shape_p = [rad, lw, 0.0, 0.0];
                        out.push(Draw::new(Pipe::Over, u));
                    }
                }
                ShapeGeo::Ellipse { rect } if !geo_dashed(&el.stroke.style) => {
                    for (off, lw) in stroke_lines(&el.stroke.style, stroke_w) {
                        let line = rect.outset(off);
                        let q = line.outset(lw * 0.5 + 1.0);
                        let mut u = Uniforms::new(xf, q.corners())
                            .coverage(cov::ELLIPSE_STROKE)
                            .color(color);
                        u.shape = line.corners();
                        u.shape_p = [0.0, lw, 0.0, 0.0];
                        out.push(Draw::new(Pipe::Over, u));
                    }
                }
                _ => {
                    let pattern = dash_pattern(&el.stroke.style, &el.stroke.pattern, stroke_w, s);
                    let cmds = geo.path();
                    if let Some(m) = rasterize_path(
                        &cmds,
                        Some((stroke_w, &pattern)),
                        r.outset(stroke_w + 2.0),
                        self.gpu.max_texture,
                    ) {
                        let t = self.gpu.upload(
                            "stroke mask",
                            m.w,
                            m.h,
                            wgpu::TextureFormat::R8Unorm,
                            &m.data,
                        );
                        let u = Uniforms::new(xf, m.region.corners())
                            .mask(m.region.xywh(), channel::R)
                            .color(color);
                        out.push(Draw::new(Pipe::Over, u).with_mask(&t.view));
                        self.keep.push(t);
                    }
                }
            }
        }
        // text
        if let Some(t) = text {
            self.text_draws(t, xf, opacity, out);
        }
    }

    fn fill_coverage(&mut self, geo: &ShapeGeo, feather: f32, feather_style: f32) -> Coverage {
        match geo {
            ShapeGeo::RRect { rect, radius } => Coverage {
                mode: cov::RRECT_FILL,
                channel: channel::R,
                quad: rect.outset(1.0 + feather),
                shape: rect.corners(),
                shape_p: [*radius, 0.0, feather, feather_style],
                mask: None,
                mask_rect: RectF::default(),
            },
            ShapeGeo::Ellipse { rect } => Coverage {
                mode: cov::ELLIPSE_FILL,
                channel: channel::R,
                quad: rect.outset(1.0 + feather),
                shape: rect.corners(),
                shape_p: [0.0, 0.0, feather, feather_style],
                mask: None,
                mask_rect: RectF::default(),
            },
            ShapeGeo::Path { cmds, rect } => {
                let region = rect.outset(2.0 + feather);
                match rasterize_path(cmds, None, region, self.gpu.max_texture) {
                    Some(m) => {
                        let t = self.gpu.upload(
                            "fill mask",
                            m.w,
                            m.h,
                            wgpu::TextureFormat::R8Unorm,
                            &m.data,
                        );
                        let (view, ch) = if feather > 0.5 {
                            // approximate feather: blur the coverage
                            let sig = feather * 0.5;
                            let blurred = self.blur_mask(&t, channel::R, sig);
                            (blurred, channel::A)
                        } else {
                            (t.view.clone(), channel::R)
                        };
                        self.keep.push(t);
                        Coverage {
                            mode: cov::MASK,
                            channel: ch,
                            quad: m.region,
                            shape: [0.0; 4],
                            shape_p: [0.0; 4],
                            mask: Some(view),
                            mask_rect: m.region,
                        }
                    }
                    None => Coverage::none(),
                }
            }
        }
    }

    fn blur_mask(&mut self, src: &Tex, ch: u32, sigma: f32) -> wgpu::TextureView {
        let (w, h) = (src.width, src.height);
        let t1 = self.gpu.linear_target("blur h", w, h);
        let t2 = self.gpu.linear_target("blur v", w, h);
        let local = RectF::new(0.0, 0.0, w as f32, h as f32);
        for (from, from_ch, to, dir) in [
            (&src.view, ch, &t1, (1.0f32, 0.0f32)),
            (&t1.view, channel::A, &t2, (0.0, 1.0)),
        ] {
            let mut u = Uniforms::new(&Affine::IDENTITY, local.corners())
                .mask(local.xywh(), from_ch)
                .coverage(cov::BLUR);
            u.blur = [
                dir.0 / w as f32,
                dir.1 / h as f32,
                sigma,
                taps(sigma) as f32,
            ];
            self.pre.push(Pass {
                label: "blur",
                target: to.view.clone(),
                size: (w, h),
                clear: None,
                draws: vec![Draw::new(Pipe::Replace, u).with_mask(from)],
            });
        }
        let v = t2.view.clone();
        self.keep.push(t1);
        self.keep.push(t2);
        v
    }

    /// Paint uniforms for a fill (`None` = draws nothing).
    #[allow(clippy::too_many_arguments)]
    fn fill_paint(
        &mut self,
        fill: &Fill,
        r: RectF,
        s: f32,
        resolver: Option<&dyn MediaResolver>,
        xf: &Affine,
        c: &Coverage,
        opacity: f32,
    ) -> Option<(Uniforms, Option<wgpu::TextureView>)> {
        let base = Uniforms::new(xf, c.quad.corners());
        match &fill.kind {
            FillKind::Color { color } => {
                if color.a <= 0.0 {
                    return None;
                }
                Some((base.color(premul_linear(*color, opacity)), None))
            }
            FillKind::Gradient { gradient } => {
                Some((gradient_uniforms(base, gradient, r, opacity), None))
            }
            FillKind::Media { asset, display, .. } => {
                let Some(res) = resolver else {
                    self.report
                        .media_issue(&format!("media asset {}", asset.0), "no media resolver");
                    return None;
                };
                let Some(src) = res.resolve(*asset) else {
                    self.report
                        .media_issue(&format!("media asset {}", asset.0), "missing");
                    return None;
                };
                let img = match self.media.get(
                    self.gpu,
                    self.decoder,
                    &src,
                    display.alpha,
                    self.image_box,
                    self.fetch,
                    &mut self.pre,
                ) {
                    Lookup::Ready(i) => i,
                    Lookup::Pending => {
                        self.pending += 1;
                        return None;
                    }
                    Lookup::Failed(e) => {
                        self.report.media_issue(&src.describe(), &e);
                        return None;
                    }
                };
                let u = media_uniforms(
                    base,
                    display,
                    r,
                    img.width,
                    img.height,
                    s,
                    opacity,
                    self.report,
                );
                Some((u, Some(img.tex.view.clone())))
            }
            FillKind::BackgroundEffect { .. } => {
                self.report
                    .unsupported("background effect fill (blur / invert behind)");
                None
            }
            // Drawn over the slide from its frame in `LayerSource::Slide.videos` (nothing
            // until the source sends pictures).
            FillKind::CutOut | FillKind::LiveSource { .. } => None,
            FillKind::Unsupported { source_type, .. } => {
                self.report.unsupported(&format!("fill {source_type}"));
                None
            }
            FillKind::Unknown => None,
        }
    }

    fn prepare_text(&mut self, tb: &TextBox, frame: &mut Frame) -> Option<PreparedText> {
        let layout = self.text.layout(tb, frame.rect, frame.scale);
        if let Some(h) = layout.resized_height {
            frame.rect.h = h;
        }
        let has_glyphs = !layout.glyphs.is_empty();
        let mask = tb.line_mask.enabled && tb.line_mask.fill.a > 0.0;
        if !has_glyphs && !mask {
            return None;
        }
        let s = frame.scale;
        let outline = (tb.outline.enabled && tb.outline.width > 0.0 && tb.outline.color.a > 0.0)
            .then_some(tb.outline.width as f32 * s);
        let shadow =
            tb.text_shadow.enabled && tb.text_shadow.opacity > 0.0 && tb.text_shadow.color.a > 0.0;
        let sigma = sigma_of(tb.text_shadow.radius as f32 * s);
        let pad = if shadow && sigma >= BLUR_MIN_SIGMA {
            taps(sigma) + 2
        } else {
            1
        };
        let raster = if has_glyphs {
            self.glyphs.rasterize(
                &mut self.text.fs,
                &layout,
                outline,
                pad,
                self.gpu.max_texture,
            )
        } else {
            None
        };
        let mut extent = RectF::default();
        let (fill_tex, outline_tex) = match &raster {
            Some(rs) => {
                extent = rs.rect();
                let f = self.gpu.upload(
                    "text fill",
                    rs.w,
                    rs.h,
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    &rs.fill,
                );
                let o = rs.outline.as_ref().map(|o| {
                    self.gpu
                        .upload("text outline", rs.w, rs.h, wgpu::TextureFormat::R8Unorm, o)
                });
                (Some(f), o)
            }
            None => (None, None),
        };
        let mut boxes = Vec::new();
        if mask {
            let lm = &tb.line_mask;
            let max_w = layout.lines.iter().map(|l| l.w).fold(0.0, f32::max);
            for l in &layout.lines {
                let (x, w) = match lm.style {
                    model::LineMaskStyle::LineWidth => (l.x, l.w),
                    model::LineMaskStyle::MaxLineWidth => (l.x + (l.w - max_w) * 0.5, max_w),
                    _ => (layout.content.x, layout.content.w),
                };
                let r = RectF::new(
                    x - lm.width_offset as f32 * s * 0.5 + lm.horizontal_offset as f32 * s,
                    l.y - lm.height_offset as f32 * s * 0.5 + lm.vertical_offset as f32 * s,
                    w + lm.width_offset as f32 * s,
                    l.h + lm.height_offset as f32 * s,
                );
                extent = extent.union(&r);
                boxes.push((r, lm.fill));
            }
        }
        for h in &layout.highlights {
            boxes.push((h.rect, h.color));
        }
        if shadow && let Some(rs) = &raster {
            let off = tb.text_shadow.offset_vector();
            extent = extent.union(&rs.rect().translate(off.x as f32 * s, off.y as f32 * s));
        }
        if tb.fill.enabled
            && !matches!(
                tb.fill.kind,
                FillKind::Color { .. } | FillKind::CutOut | FillKind::Gradient { .. }
            )
        {
            self.report
                .unsupported("text fill other than colour / gradient / cut-out");
        }
        Some(PreparedText {
            raster_rect: raster.as_ref().map(|r| r.rect()).unwrap_or_default(),
            fill: fill_tex,
            outline: outline_tex,
            outline_color: tb.outline.color,
            shadow: shadow.then_some(TextShadow {
                sigma,
                dx: tb.text_shadow.offset_vector().x as f32 * s,
                dy: tb.text_shadow.offset_vector().y as f32 * s,
                color: tb.text_shadow.color,
                opacity: tb.text_shadow.opacity as f32,
            }),
            boxes,
            solid: match (&tb.fill.enabled, &tb.fill.kind) {
                (true, FillKind::Color { color }) => Some(*color),
                _ => None,
            },
            gradient: match (&tb.fill.enabled, &tb.fill.kind) {
                (true, FillKind::Gradient { gradient }) => Some(gradient.clone()),
                _ => None,
            },
            cutout: tb.fill.enabled && tb.fill.kind == FillKind::CutOut,
            content: layout.content,
            extent,
        })
    }

    fn text_draws(&mut self, t: &PreparedText, xf: &Affine, opacity: f32, out: &mut Vec<Draw>) {
        for (r, c) in &t.boxes {
            out.push(solid_rect(xf, *r, premul_linear(*c, opacity)));
        }
        let Some(fill) = &t.fill else {
            return;
        };
        let rr = t.raster_rect;
        if let Some(sh) = &t.shadow {
            let (src, ch) = match &t.outline {
                Some(o) => (o.clone(), channel::R),
                None => (fill.clone(), channel::A),
            };
            let color = premul_linear(sh.color, sh.opacity * opacity);
            self.shadow_draws(&src, ch, rr, sh.sigma, sh.dx, sh.dy, color, xf, out);
        }
        if let Some(o) = &t.outline {
            let u = Uniforms::new(xf, rr.corners())
                .mask(rr.xywh(), channel::R)
                .color(premul_linear(t.outline_color, opacity));
            out.push(Draw::new(Pipe::Over, u).with_mask(&o.view));
        }
        if t.cutout {
            let u = Uniforms::new(xf, rr.corners()).mask(rr.xywh(), channel::A);
            out.push(Draw::new(Pipe::DestOut, u).with_mask(&fill.view));
        } else if let Some(c) = t.solid {
            let u = Uniforms::new(xf, rr.corners())
                .mask(rr.xywh(), channel::A)
                .color(premul_linear(c, opacity));
            out.push(Draw::new(Pipe::Over, u).with_mask(&fill.view));
        } else if let Some(g) = &t.gradient {
            let u = gradient_uniforms(Uniforms::new(xf, rr.corners()), g, t.content, opacity)
                .mask(rr.xywh(), channel::A);
            out.push(Draw::new(Pipe::Over, u).with_mask(&fill.view));
        } else {
            let mut u = Uniforms::new(xf, rr.corners())
                .paint(paint::TEXTURE_STRAIGHT)
                .paint_rect(rr.xywh())
                .opacity(opacity);
            u.modes[3] = FLAG_CLIP_PAINT;
            out.push(Draw::new(Pipe::Over, u).with_paint(&fill.view));
        }
        self.keep.push(fill.clone());
        if let Some(o) = &t.outline {
            self.keep.push(o.clone());
        }
    }
}

struct TextShadow {
    sigma: f32,
    dx: f32,
    dy: f32,
    color: Color,
    opacity: f32,
}

struct PreparedText {
    raster_rect: RectF,
    fill: Option<Tex>,
    outline: Option<Tex>,
    outline_color: Color,
    shadow: Option<TextShadow>,
    boxes: Vec<(RectF, Color)>,
    /// Text-box colour fill replacing the run colours.
    solid: Option<Color>,
    gradient: Option<Gradient>,
    cutout: bool,
    content: RectF,
    /// Everything the text draws, frame px.
    extent: RectF,
}

struct Coverage {
    mode: u32,
    /// Mask channel (mask modes).
    channel: u32,
    quad: RectF,
    shape: [f32; 4],
    shape_p: [f32; 4],
    mask: Option<wgpu::TextureView>,
    mask_rect: RectF,
}

impl Coverage {
    fn none() -> Self {
        Coverage {
            mode: cov::FULL,
            channel: channel::R,
            quad: RectF::default(),
            shape: [0.0; 4],
            shape_p: [0.0; 4],
            mask: None,
            mask_rect: RectF::default(),
        }
    }

    fn apply(&self, u: &mut (Uniforms, Option<wgpu::TextureView>)) {
        let x = &mut u.0;
        x.quad = self.quad.corners();
        x.modes[0] = self.mode;
        if self.mode == cov::MASK {
            x.mask_xf = self.mask_rect.xywh();
            x.modes[2] = self.channel;
        } else {
            x.shape = self.shape;
            x.shape_p = self.shape_p;
        }
    }
}

enum ShapeGeo {
    RRect {
        rect: RectF,
        radius: f32,
    },
    Ellipse {
        rect: RectF,
    },
    Path {
        cmds: Vec<zeno::Command>,
        rect: RectF,
    },
}

fn geo_dashed(style: &StrokeStyle) -> bool {
    !matches!(
        style,
        StrokeStyle::Solid | StrokeStyle::Unknown | StrokeStyle::Double
    )
}

/// The lines (centre-line outset, width) of a stroke `w` wide: one, or two for a double.
fn stroke_lines(style: &StrokeStyle, w: f32) -> Vec<(f32, f32)> {
    match style {
        StrokeStyle::Double => vec![(w / 3.0, w / 3.0), (-w / 3.0, w / 3.0)],
        _ => vec![(0.0, w)],
    }
}

impl ShapeGeo {
    fn new(shape: &Shape, r: RectF) -> Self {
        match shape {
            Shape::Rectangle | Shape::Unknown => ShapeGeo::RRect {
                rect: r,
                radius: 0.0,
            },
            Shape::RoundedRectangle { roundness } => ShapeGeo::RRect {
                rect: r,
                radius: (*roundness as f32).clamp(0.0, 0.5) * r.w.min(r.h),
            },
            Shape::Ellipse => ShapeGeo::Ellipse { rect: r },
            other => ShapeGeo::Path {
                cmds: shape_path(other, r),
                rect: r,
            },
        }
    }

    /// Outline path (for dashed strokes of analytic shapes too).
    fn path(&self) -> Vec<zeno::Command> {
        use zeno::{Command, Point};
        match self {
            ShapeGeo::Path { cmds, .. } => cmds.clone(),
            ShapeGeo::Ellipse { rect } => {
                // 4 cubic arcs
                let k = 0.552_284_8;
                let (cx, cy) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
                let (rx, ry) = (rect.w * 0.5, rect.h * 0.5);
                let p = Point::new;
                vec![
                    Command::MoveTo(p(cx + rx, cy)),
                    Command::CurveTo(
                        p(cx + rx, cy + ry * k),
                        p(cx + rx * k, cy + ry),
                        p(cx, cy + ry),
                    ),
                    Command::CurveTo(
                        p(cx - rx * k, cy + ry),
                        p(cx - rx, cy + ry * k),
                        p(cx - rx, cy),
                    ),
                    Command::CurveTo(
                        p(cx - rx, cy - ry * k),
                        p(cx - rx * k, cy - ry),
                        p(cx, cy - ry),
                    ),
                    Command::CurveTo(
                        p(cx + rx * k, cy - ry),
                        p(cx + rx, cy - ry * k),
                        p(cx + rx, cy),
                    ),
                    Command::Close,
                ]
            }
            ShapeGeo::RRect { rect, radius } => {
                if *radius <= 0.0 {
                    return shape_path(&Shape::Rectangle, *rect);
                }
                let k = 0.552_284_8 * radius;
                let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
                let rd = *radius;
                let p = Point::new;
                vec![
                    Command::MoveTo(p(x0 + rd, y0)),
                    Command::LineTo(p(x1 - rd, y0)),
                    Command::CurveTo(p(x1 - rd + k, y0), p(x1, y0 + rd - k), p(x1, y0 + rd)),
                    Command::LineTo(p(x1, y1 - rd)),
                    Command::CurveTo(p(x1, y1 - rd + k), p(x1 - rd + k, y1), p(x1 - rd, y1)),
                    Command::LineTo(p(x0 + rd, y1)),
                    Command::CurveTo(p(x0 + rd - k, y1), p(x0, y1 - rd + k), p(x0, y1 - rd)),
                    Command::LineTo(p(x0, y0 + rd)),
                    Command::CurveTo(p(x0, y0 + rd - k), p(x0 + rd - k, y0), p(x0 + rd, y0)),
                    Command::Close,
                ]
            }
        }
    }
}

fn dash_pattern(style: &StrokeStyle, pattern: &[f64], w: f32, s: f32) -> Vec<f32> {
    let w = w.max(1.0);
    match style {
        StrokeStyle::SquareDash => vec![w, w],
        StrokeStyle::ShortDash => vec![w * 3.0, w * 2.0],
        StrokeStyle::LongDash => vec![w * 8.0, w * 3.0],
        StrokeStyle::Custom => {
            let p: Vec<f32> = pattern.iter().map(|v| (*v as f32 * s).max(0.1)).collect();
            if p.len() >= 2 { p } else { Vec::new() }
        }
        _ => Vec::new(),
    }
}

/// An anti-aliased solid rectangle.
pub(crate) fn solid_rect(xf: &Affine, r: RectF, color: [f32; 4]) -> Draw {
    let mut u = Uniforms::new(xf, r.outset(1.0).corners())
        .coverage(cov::RRECT_FILL)
        .color(color);
    u.shape = r.corners();
    Draw::new(Pipe::Over, u)
}

fn gradient_uniforms(base: Uniforms, g: &Gradient, r: RectF, opacity: f32) -> Uniforms {
    let mut u = base.paint(paint::GRADIENT).color([1.0, 1.0, 1.0, opacity]);
    let mut stops: Vec<_> = g.stops.iter().collect();
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    stops.truncate(8);
    let n = stops.len().max(1);
    for (i, st) in stops.iter().enumerate() {
        u.stop_c[i] = [st.color.r, st.color.g, st.color.b, st.color.a];
        u.stop_p[i / 4][i % 4] = st.position as f32;
        u.stop_m[i / 4][i % 4] = st.midpoint as f32;
    }
    if stops.is_empty() {
        u.stop_c[0] = [0.0, 0.0, 0.0, 0.0];
    }
    let kind = match g.kind {
        GradientKind::Radial => 1.0,
        GradientKind::Angle => 2.0,
        _ => 0.0,
    };
    u.grad = [
        kind,
        (g.angle as f32).to_radians(),
        g.length as f32,
        n as f32,
    ];
    u.grad_geo = r.xywh();
    u
}

/// Uniforms placing an image (`img_w × img_h` source px) into `target` per `display`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn media_uniforms(
    base: Uniforms,
    display: &MediaDisplay,
    target: RectF,
    img_w: u32,
    img_h: u32,
    native_scale: f32,
    opacity: f32,
    report: &mut ReportState,
) -> Uniforms {
    let c = &display.crop;
    let cw = (img_w as f32 - (c.left + c.right) as f32).max(1.0);
    let ch = (img_h as f32 - (c.top + c.bottom) as f32).max(1.0);
    let (dw, dh) = if swaps_axes(display.rotation) {
        (ch, cw)
    } else {
        (cw, ch)
    };
    let placed = place_media(target, dw, dh, display.scale, display.anchor, native_scale);
    let mut alpha = opacity;
    for e in display.effects.iter().filter(|e| e.enabled) {
        match &e.kind {
            EffectKind::AlphaAdjust { amount } => alpha *= (*amount as f32).clamp(0.0, 1.0),
            EffectKind::Vendor(v) => report.unsupported(&format!("media effect {}", v.name)),
            other => report.unsupported(&format!(
                "media effect {}",
                match other {
                    EffectKind::ColorAdjust { .. } => "colorAdjust",
                    EffectKind::TwoColor { .. } => "twoColor",
                    EffectKind::Blur { .. } => "blur",
                    _ => "other",
                }
            )),
        }
    }
    let mut u = base
        .paint(paint::TEXTURE_PREMUL)
        .paint_rect(placed.xywh())
        .color([alpha, alpha, alpha, alpha]);
    u.uv_m = uv_matrix(
        display.rotation,
        display.flip.horizontal,
        display.flip.vertical,
    );
    u.uv_crop = [
        c.left as f32 / img_w as f32,
        c.top as f32 / img_h as f32,
        cw / img_w as f32,
        ch / img_h as f32,
    ];
    u.modes[3] |= FLAG_CLIP_PAINT;
    u
}

/// Evaluates the visibility conditions that do not need show state.
pub(crate) fn visible(slide: &Slide, el: &Element) -> bool {
    el.visibility.iter().all(|c| match c {
        VisibilityCondition::ElementHasText { element, has_text } => {
            let other_has = slide
                .element(*element)
                .and_then(Element::text_box)
                .is_some_and(|t| !t.is_blank());
            other_has == *has_text
        }
        // timer / video-input conditions need live show state: visible until the server
        // evaluates them
        _ => true,
    })
}
