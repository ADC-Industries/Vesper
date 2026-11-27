//! The slides a signage channel draws: the main zone's item (slide layer) and the zones and
//! the override (messages layer), built for the channel's screen size.

use super::feeds::{self, Row};
use crate::data::ShowData;
use model::{
    CharStyle, Color, Element, ElementId, FillKind, FitMode, MediaAssetId, MediaDisplay, Paragraph,
    ParagraphStyle, Playback, PresentationId, Rect, Repeat, ScaleMode, Size, Slide, SlideId,
    TextAlign, TextBox, TextScroll, VerticalAlign,
};
use protocol::{
    SignageContent, SignageFeed, SignageLayout, SignageOverride, SignageOverrideStyle,
    SignageWeather,
};

/// The font every built-in signage text uses.
const FONT: &str = "DejaVu Sans";
/// The sidebar's and ticker's panel colour.
const PANEL: Color = Color::rgb(0.07, 0.08, 0.11);
/// The accent (sidebar headings, the ticker's edge).
const ACCENT: Color = Color::rgb(0.95, 0.72, 0.30);
/// An announcement's background.
const CARD: Color = Color::rgb(0.10, 0.16, 0.28);

/// Where a layout's zones are on a screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zones {
    /// The playlist's zone.
    pub main: Rect,
    /// The sidebar (clock, countdown, weather).
    pub sidebar: Option<Rect>,
    /// The ticker band.
    pub ticker: Option<Rect>,
}

/// The zones of `layout` on a screen of `size`.
pub fn zones(layout: SignageLayout, size: Size) -> Zones {
    let (w, h) = (size.width, size.height);
    let sw = if layout.sidebar() {
        (w * 0.26).round()
    } else {
        0.0
    };
    let th = if layout.ticker() {
        (h * 0.09).round()
    } else {
        0.0
    };
    Zones {
        main: Rect::new(0.0, 0.0, w - sw, h - th),
        sidebar: layout.sidebar().then(|| Rect::new(w - sw, 0.0, sw, h - th)),
        ticker: layout.ticker().then(|| Rect::new(0.0, h - th, w, th)),
    }
}

fn text(r: Rect, s: &str, size: f64, color: Color, weight: u16, align: TextAlign) -> Element {
    Element::text(
        r,
        TextBox {
            paragraphs: Paragraph::from_lines(s),
            default_style: CharStyle {
                font_family: Some(FONT.into()),
                font_weight: Some(weight),
                size: Some(size),
                color: Some(color),
                ..CharStyle::default()
            },
            default_paragraph: ParagraphStyle {
                alignment: Some(align),
                ..ParagraphStyle::default()
            },
            vertical_align: VerticalAlign::Middle,
            fit: FitMode::ShrinkToFit,
            ..TextBox::default()
        },
    )
}

fn panel(r: Rect, c: Color) -> Element {
    let mut e = Element::new(r);
    e.fill = model::Fill::color(c);
    e
}

fn media(r: Rect, asset: MediaAssetId, video: bool) -> Element {
    let mut e = Element::new(r);
    e.fill = model::Fill::enabled(FillKind::Media {
        asset,
        display: MediaDisplay {
            scale: ScaleMode::Fit,
            ..MediaDisplay::default()
        },
        playback: if video {
            Playback {
                repeat: Repeat::Loop,
                ..Playback::default()
            }
        } else {
            Playback::default()
        },
        audio: Default::default(),
    });
    e
}

/// Gives a built slide and its elements IDs that follow from `key` (the channel and the item,
/// or the channel's zones) and the element's place, never random ones (H02-1): the renderer
/// knows a slide by its ID (the same slide with new content keeps its clock, so the ticker
/// scrolls on and a video element keeps its player) and redraws only when its content changed.
pub fn stamp(slide: &mut Slide, key: &str) {
    let base = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("vesper-signage:{key}").as_bytes(),
    );
    slide.id = SlideId(base);
    for (i, e) in slide.elements.iter_mut().enumerate() {
        e.id = ElementId(uuid::Uuid::new_v5(&base, &(i as u64).to_le_bytes()));
    }
}

/// Why an item can't be shown now, or its slide and canvas for a screen of `size`.
pub fn item(
    content: &SignageContent,
    data: &ShowData,
    feed: Option<&(SignageFeed, Vec<Row>)>,
    zone: Rect,
    size: Size,
) -> Result<(Slide, Size), String> {
    let mut s = Slide::new();
    s.background = Some(Color::BLACK);
    match content {
        SignageContent::Slide {
            presentation,
            slide,
        } => {
            let id = presentation
                .parse::<uuid::Uuid>()
                .map(PresentationId)
                .map_err(|_| format!("presentation {presentation} is not an ID"))?;
            let p = data
                .content
                .presentation(id)
                .ok_or_else(|| format!("presentation {presentation} is not in the library"))?;
            let sl = p
                .slides()
                .nth(*slide as usize)
                .ok_or_else(|| format!("{} has no slide {}", p.name, slide + 1))?;
            Ok(fit_slide(sl, p.canvas_size, zone, size))
        }
        SignageContent::Image { asset } | SignageContent::Video { asset } => {
            let id = asset
                .parse::<uuid::Uuid>()
                .map(MediaAssetId)
                .map_err(|_| format!("media {asset} is not an ID"))?;
            let path = data.content.asset_path(id)?;
            if !path.is_file() {
                return Err(format!(
                    "{} is missing ({})",
                    data.content.asset_name(id),
                    path.display()
                ));
            }
            let video = matches!(content, SignageContent::Video { .. });
            s.elements.push(media(zone, id, video));
            Ok((s, size))
        }
        SignageContent::Text { title, body } => {
            s.elements.push(panel(zone, CARD));
            let pad = zone.height * 0.08;
            let inner = Rect::new(
                zone.x + pad,
                zone.y + pad,
                zone.width - 2.0 * pad,
                zone.height - 2.0 * pad,
            );
            let k = zone.height / 1080.0;
            if body.trim().is_empty() {
                s.elements.push(text(
                    inner,
                    title,
                    120.0 * k,
                    Color::WHITE,
                    700,
                    TextAlign::Center,
                ));
            } else {
                let th = inner.height * 0.35;
                s.elements.push(text(
                    Rect::new(inner.x, inner.y, inner.width, th),
                    title,
                    110.0 * k,
                    Color::WHITE,
                    700,
                    TextAlign::Center,
                ));
                s.elements.push(text(
                    Rect::new(inner.x, inner.y + th, inner.width, inner.height - th),
                    body,
                    64.0 * k,
                    Color::rgb(0.86, 0.89, 0.95),
                    400,
                    TextAlign::Center,
                ));
            }
            Ok((s, size))
        }
        SignageContent::Feed { rows: n, .. } => {
            let (feed, rows) = feed.ok_or("the feed is gone")?;
            if rows.is_empty() {
                return Err(format!(
                    "the feed {} has no rows{}",
                    feed.name,
                    feed.problem
                        .as_deref()
                        .map(|p| format!(" ({p})"))
                        .unwrap_or_default()
                ));
            }
            s.elements.push(panel(zone, CARD));
            let k = zone.height / 1080.0;
            let pad = 60.0 * k;
            let head = 150.0 * k;
            s.elements.push(text(
                Rect::new(
                    zone.x + pad,
                    zone.y + pad * 0.5,
                    zone.width - 2.0 * pad,
                    head,
                ),
                &feed.name,
                80.0 * k,
                ACCENT,
                700,
                TextAlign::Left,
            ));
            let shown: Vec<&Row> = rows.iter().take((*n).clamp(1, 8) as usize).collect();
            let top = zone.y + pad * 0.5 + head;
            let each = (zone.height - (top - zone.y) - pad) / shown.len() as f64;
            for (i, r) in shown.iter().enumerate() {
                let (when, title, body) = feeds::mapped(r, &feed.mapping, &feed.columns);
                let line = match (when.is_empty(), body.is_empty()) {
                    (true, true) => title,
                    (false, true) => format!("{when} · {title}"),
                    (true, false) => format!("{title}\n{body}"),
                    (false, false) => format!("{when} · {title}\n{body}"),
                };
                s.elements.push(text(
                    Rect::new(
                        zone.x + pad,
                        top + each * i as f64,
                        zone.width - 2.0 * pad,
                        each * 0.92,
                    ),
                    &line,
                    54.0 * k,
                    Color::WHITE,
                    500,
                    TextAlign::Left,
                ));
            }
            Ok((s, size))
        }
    }
}

/// A presentation's slide scaled into `zone` (centred, its aspect kept) on a screen of `size`:
/// the slide keeps its elements and fonts, drawn on a canvas so much larger than the screen
/// that it lands in the zone (slides are scaled uniformly from their canvas to the screen).
pub fn fit_slide(slide: &Slide, canvas: Size, zone: Rect, size: Size) -> (Slide, Size) {
    let (cw, ch) = (canvas.width.max(1.0), canvas.height.max(1.0));
    let k = (zone.width / cw).min(zone.height / ch).max(1e-3);
    let big = Size::new(size.width / k, size.height / k);
    let dx = zone.x / k + (zone.width / k - cw) / 2.0;
    let dy = zone.y / k + (zone.height / k - ch) / 2.0;
    let mut s = slide.clone();
    for e in &mut s.elements {
        e.bounds.x += dx;
        e.bounds.y += dy;
    }
    (s, big)
}

/// What the zones show.
pub struct Chrome<'a> {
    /// The channel's layout.
    pub layout: SignageLayout,
    /// The clock ("10:42") and the date ("Wednesday 25 November").
    pub clock: (String, String),
    /// The next service: its name and the countdown.
    pub countdown: Option<(String, String)>,
    /// The weather (not stale), with the unit's sign.
    pub weather: Option<(&'a SignageWeather, &'static str)>,
    /// The ticker's text.
    pub ticker: String,
    /// The override, when it shows on this channel.
    pub over: Option<&'a SignageOverride>,
}

/// The zones and the override as one slide on a screen of `size`.
pub fn chrome(c: &Chrome<'_>, size: Size) -> Slide {
    let z = zones(c.layout, size);
    let mut s = Slide::new();
    let k = size.height / 1080.0;
    if let Some(r) = z.sidebar {
        s.elements.push(panel(r, PANEL));
        let pad = 36.0 * k;
        let w = r.width - 2.0 * pad;
        let x = r.x + pad;
        let mut y = r.y + pad;
        s.elements.push(text(
            Rect::new(x, y, w, 150.0 * k),
            &c.clock.0,
            130.0 * k,
            Color::WHITE,
            700,
            TextAlign::Center,
        ));
        y += 150.0 * k;
        s.elements.push(text(
            Rect::new(x, y, w, 50.0 * k),
            &c.clock.1,
            34.0 * k,
            Color::rgb(0.75, 0.78, 0.85),
            400,
            TextAlign::Center,
        ));
        y += 90.0 * k;
        if let Some((name, when)) = &c.countdown {
            s.elements.push(text(
                Rect::new(x, y, w, 44.0 * k),
                name,
                32.0 * k,
                ACCENT,
                700,
                TextAlign::Center,
            ));
            y += 44.0 * k;
            s.elements.push(text(
                Rect::new(x, y, w, 80.0 * k),
                when,
                62.0 * k,
                Color::WHITE,
                700,
                TextAlign::Center,
            ));
            y += 120.0 * k;
        }
        if let Some((wx, unit)) = c.weather {
            s.elements.push(text(
                Rect::new(x, y, w, 120.0 * k),
                &format!("{:.0}{unit}", wx.temperature),
                100.0 * k,
                Color::WHITE,
                700,
                TextAlign::Center,
            ));
            y += 120.0 * k;
            s.elements.push(text(
                Rect::new(x, y, w, 90.0 * k),
                &format!(
                    "{}\nH {:.0}{unit}  L {:.0}{unit}",
                    wx.summary, wx.high, wx.low
                ),
                34.0 * k,
                Color::rgb(0.80, 0.83, 0.90),
                400,
                TextAlign::Center,
            ));
            s.elements.push(text(
                Rect::new(x, r.y + r.height - pad - 30.0 * k, w, 30.0 * k),
                &wx.credit,
                20.0 * k,
                Color::rgb(0.60, 0.63, 0.70),
                400,
                TextAlign::Center,
            ));
        }
    }
    if let Some(r) = z.ticker {
        s.elements.push(panel(r, PANEL));
        s.elements
            .push(panel(Rect::new(r.x, r.y, r.width, 4.0 * k), ACCENT));
        if !c.ticker.trim().is_empty() {
            let mut t = text(
                Rect::new(r.x, r.y + 4.0 * k, r.width, r.height - 4.0 * k),
                c.ticker.trim(),
                r.height * 0.5,
                Color::WHITE,
                600,
                TextAlign::Left,
            );
            if let model::ElementKind::Text { text, .. } = &mut t.kind {
                text.fit = FitMode::None;
            }
            t.scroll = TextScroll {
                enabled: true,
                speed: 140.0 * k,
                ..TextScroll::default()
            };
            s.elements.push(t);
        }
    }
    if let Some(o) = c.over {
        let bg = match o.style {
            SignageOverrideStyle::Urgent => Color::rgb(0.70, 0.07, 0.07),
            SignageOverrideStyle::Info => Color::rgb(0.07, 0.25, 0.60),
        };
        let all = Rect::new(0.0, 0.0, size.width, size.height);
        s.elements.push(panel(all, bg));
        let pad = 100.0 * k;
        s.elements.push(text(
            Rect::new(pad, pad, size.width - 2.0 * pad, size.height - 2.0 * pad),
            &o.text,
            140.0 * k,
            Color::WHITE,
            700,
            TextAlign::Center,
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_split_the_screen() {
        let hd = Size::HD;
        let z = zones(SignageLayout::Full, hd);
        assert_eq!(z.main, Rect::new(0.0, 0.0, 1920.0, 1080.0));
        assert!(z.sidebar.is_none() && z.ticker.is_none());
        let z = zones(SignageLayout::MainSidebarTicker, hd);
        assert_eq!(z.main, Rect::new(0.0, 0.0, 1421.0, 983.0));
        assert_eq!(z.sidebar, Some(Rect::new(1421.0, 0.0, 499.0, 983.0)));
        assert_eq!(z.ticker, Some(Rect::new(0.0, 983.0, 1920.0, 97.0)));
    }

    #[test]
    fn a_slide_lands_in_its_zone() {
        let mut sl = Slide::new();
        sl.elements
            .push(Element::new(Rect::new(0.0, 0.0, 1920.0, 1080.0)));
        let zone = Rect::new(0.0, 0.0, 960.0, 540.0);
        let (s, canvas) = fit_slide(&sl, Size::HD, zone, Size::HD);
        // Half size: the canvas is twice the screen and the slide sits in its top-left
        // quarter, which is the zone once scaled.
        assert_eq!(canvas, Size::new(3840.0, 2160.0));
        assert_eq!(s.elements[0].bounds, Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let zone = Rect::new(0.0, 0.0, 1421.0, 983.0);
        let (s, canvas) = fit_slide(&sl, Size::HD, zone, Size::HD);
        let k = 1080.0 / canvas.height;
        let b = s.elements[0].bounds;
        assert!((b.x * k).abs() < 1.0, "left edge {}", b.x * k);
        assert!(((b.x + b.width) * k - 1421.0).abs() < 1.0);
        assert!(
            ((b.y + b.height / 2.0) * k - 983.0 / 2.0).abs() < 1.0,
            "centred"
        );
    }
}
