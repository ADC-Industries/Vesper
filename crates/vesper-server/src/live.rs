//! Live data for rendering ([`render::live::DataContext`]) and the slides of the messages layer.
//!
//! - [`data_context`] gathers what data links can show at one instant: timer values (from the
//!   show state's [`crate::timers::Timers`]), the local time, the live and next slide's texts
//!   and notes (the next slide is the one `next` would trigger: playlist context and document
//!   order, as the cursor; arrangements are not followed yet), the playlist names around the
//!   live item, the stage message and its flash phase, media countdowns; the live
//!   presentation's name, notes, copyright line ([`ccli_line`]) and song artwork, the
//!   announcements layer's presentation name; for visibility conditions the messages showing,
//!   the live video input, the layers with content ([`layers_with_content`]) and the media
//!   states.
//! - **The copyright line on its own** ([`ccli_shown`], [`with_ccli`]): with the show's CCLI
//!   display on and the song's `songInfo.display`, the slide layer draws the CCLI template
//!   (`ccli.template`, else [`ccli_builtin`]: small white text, bottom left) over the song's
//!   slides `ccli.showOn` names, counted in the play order; its CCLI text links show
//!   [`ccli_line`].
//! - **Messages** ([`message_text`], [`message_slide`]): a message's text with its tokens
//!   substituted — `${name}` (and `{name}`) for each token: a text token takes the value the
//!   message was shown with, else its default; a timer token the timer's value in the token's
//!   format; a clock token the local time. Unknown placeholders stay as written. The text goes
//!   into the first text slot of the message's template slide (else its first text element
//!   without a data link); the template's other text slots are emptied. Without a template (or
//!   when it is gone) a built-in lower-third banner is drawn: a translucent black bar along the
//!   bottom with centred white text.

use crate::data::ShowData;
use crate::show::{LiveSlide, ShowState};
use crate::timers::Clock;
use model::{
    CharStyle, Color, Element, FitMode, Message, MessageTokenKind, Paragraph, ParagraphStyle,
    PlaylistItem, PlaylistItemKind, PlaylistNodeKind, Rect, Size, Slide, TextAlign, TextBox,
    TokenValue, VerticalAlign,
};
use render::live::{DataContext, PlaylistInfo, SlideInfo, format_clock, format_timer};

/// The section name and slide at `index` (document order) of a live slide.
fn slide_at<'a>(data: &'a ShowData, l: &LiveSlide) -> Option<(&'a str, &'a Slide, usize)> {
    let p = data.content.presentation(l.presentation)?;
    let total = p.slides().count();
    p.sections
        .iter()
        .flat_map(|sec| sec.slides.iter().map(move |s| (sec.name.as_str(), s)))
        .nth(l.index)
        .map(|(n, s)| (n, s, total))
}

/// A playlist item's display name (its own name, else what it holds).
pub fn item_name(data: &ShowData, it: &PlaylistItem) -> String {
    if let Some(n) = it.name.as_ref().filter(|n| !n.is_empty()) {
        return n.clone();
    }
    match &it.kind {
        PlaylistItemKind::Presentation { presentation, .. } => data
            .content
            .presentation(presentation.id)
            .map(|p| p.name.clone())
            .or_else(|| presentation.name.clone())
            .unwrap_or_default(),
        PlaylistItemKind::Media(t) => data.content.asset_name(t.asset),
        PlaylistItemKind::Header { name, .. } => name.clone(),
        _ => String::new(),
    }
}

fn playlist_info(data: &ShowData, l: &LiveSlide) -> PlaylistInfo {
    let Some(ctx) = l.playlist else {
        return PlaylistInfo::default();
    };
    let Some(node) = data.content.playlists.get(&ctx.playlist) else {
        return PlaylistInfo::default();
    };
    let PlaylistNodeKind::Playlist { items, .. } = &node.kind else {
        return PlaylistInfo::default();
    };
    let at = items.iter().position(|i| i.id == ctx.item);
    let is_header = |i: &&PlaylistItem| matches!(i.kind, PlaylistItemKind::Header { .. });
    PlaylistInfo {
        current: at.map(|i| item_name(data, &items[i])),
        next: at.and_then(|i| {
            items[i + 1..]
                .iter()
                .find(|x| x.enabled && !is_header(x))
                .map(|x| item_name(data, x))
        }),
        current_header: at.and_then(|i| {
            items[..i]
                .iter()
                .rev()
                .find(is_header)
                .map(|x| item_name(data, x))
        }),
        next_header: at.and_then(|i| {
            items[i + 1..]
                .iter()
                .find(is_header)
                .map(|x| item_name(data, x))
        }),
        playlist: Some(node.name.clone()),
    }
}

/// The live slide's (place, length) in the play order (the arrangement, see
/// [`crate::show::play_order`]) and the name of the section played next: the first section
/// occurrence after the live one (a repeated section counts again). Off the play order, the
/// count is left to the caller and the next section follows the document.
fn play_position(
    data: &ShowData,
    p: &model::Presentation,
    l: &crate::show::LiveSlide,
) -> (Option<(usize, usize)>, Option<String>) {
    // The section (index into p.sections) of every slide, in document order.
    let section_of: Vec<usize> = p
        .sections
        .iter()
        .enumerate()
        .flat_map(|(i, sec)| std::iter::repeat_n(i, sec.slides.len()))
        .collect();
    let name = |doc: usize| section_of.get(doc).map(|i| p.sections[*i].name.clone());
    let order = crate::show::play_order(data, p, l.playlist);
    let Some(k) = crate::show::step_in(&order, l) else {
        // Document order: the first section with slides after the live slide's.
        let next = section_of
            .get(l.index)
            .and_then(|cur| section_of[l.index..].iter().find(|s| *s != cur))
            .map(|s| p.sections[*s].name.clone());
        return (None, next);
    };
    // A new section occurrence starts where the section changes or the order jumps (the
    // same section played again).
    let next = (k + 1..order.len())
        .find(|j| {
            let (prev, d) = (order[j - 1], order[*j]);
            section_of.get(d) != section_of.get(prev) || d != prev + 1
        })
        .and_then(|j| name(order[j]));
    (Some((k, order.len())), next)
}

/// What data links can show at `clock` (media countdowns are filled in by the caller).
pub fn data_context(data: &ShowData, state: &ShowState, clock: Clock) -> DataContext {
    let mut ctx = DataContext {
        timers: state.timers.values(&data.content.timers, clock),
        last_started: state.timers.last_started,
        wall: Some(crate::timers::wall_time()),
        clock: render::live::BoxClock {
            use_24_hour: state.clock.use_24_hour,
            seconds: state.clock.seconds,
        },
        ..DataContext::default()
    };
    if let Some(l) = state.slide.as_ref() {
        if l.detached {
            if let Some(h) = &state.held_slide {
                ctx.current = Some(SlideInfo::of(&h.slide, &h.section_name));
            }
        } else if let Some((section, s, total)) = slide_at(data, l) {
            ctx.current = Some(SlideInfo::of(s, section));
            ctx.slide_count = Some((l.index, total));
            if let Some(p) = data.content.presentation(l.presentation) {
                let (count, next) = play_position(data, p, l);
                ctx.slide_count = count.or(ctx.slide_count);
                ctx.next_section = next;
            }
        }
        ctx.playlist = playlist_info(data, l);
    }
    if let Some(next) = state.cursor(data)
        && let Some((section, s, _)) = slide_at(data, &next)
    {
        ctx.next = Some(SlideInfo::of(s, section));
    }
    // The stage message (performers only; audience messages are not shown on stage).
    if let Some(m) = &state.stage_message {
        ctx.message = Some(m.text.clone());
        ctx.stage_flash = m.flash_on(clock.now);
    }
    // The live presentation: name, notes, copyright line, artwork.
    let live = |l: Option<&LiveSlide>| l.and_then(|l| data.content.presentation(l.presentation));
    if let Some(p) = live(state.slide.as_ref()) {
        ctx.presentation_name = Some(p.name.clone());
        ctx.presentation_notes = Some(p.notes.plain_text());
        if let Some(song) = &p.song_info {
            ctx.ccli = Some(ccli_line(song, &data.settings.ccli));
            ctx.artwork = song.artwork;
            // Chord charts and key lines (stage layouts add their own chord view).
            ctx.song_key = song.original_key.clone();
            ctx.song_transpose = song.transpose;
        }
    }
    ctx.announcement_name = live(state.announcements.as_ref()).map(|p| p.name.clone());
    ctx.messages = state.messages.iter().map(|m| m.message).collect();
    ctx.video_inputs = live_video_inputs(data, state);
    ctx.layers = layers_with_content(state);
    ctx
}

/// The show layers that have content.
pub fn layers_with_content(state: &ShowState) -> Vec<model::Layer> {
    use model::Layer as L;
    [
        (L::VideoInput, state.video_input.is_some()),
        (L::Media, state.media.is_some()),
        (
            L::Slide,
            state.slide.is_some() || state.slide_media.is_some(),
        ),
        (L::Announcements, state.announcements.is_some()),
        (L::Props, !state.props.is_empty()),
        (L::Messages, !state.messages.is_empty()),
        (L::Audio, state.audio.is_some()),
    ]
    .into_iter()
    .filter_map(|(l, on)| on.then_some(l))
    .collect()
}

/// A song's copyright line (CCLI text links): the settings' `layout` tokens filled from the
/// song and the licence, or, with no layout, the built-in one: title, author,
/// `© year publisher`, `CCLI Song # n`, `CCLI Licence # n`, one per line, empty parts left
/// out.
pub fn ccli_line(song: &model::SongInfo, ccli: &model::CcliSettings) -> String {
    use model::CcliToken as T;
    let some = |v: &Option<String>| v.clone().filter(|v| !v.trim().is_empty());
    let year = song.copyright_year.map(|y| y.to_string());
    if ccli.layout.is_empty() {
        let copyright = [year, some(&song.publisher)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        return [
            some(&song.title),
            some(&song.authors),
            (!copyright.is_empty()).then(|| format!("© {copyright}")),
            some(&song.ccli_number).map(|n| format!("CCLI Song # {n}")),
            some(&ccli.license_number).map(|n| format!("CCLI Licence # {n}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    }
    ccli.layout
        .iter()
        .map(|t| match t {
            T::Text { text } => text.clone(),
            T::Artist => some(&song.artist).unwrap_or_default(),
            T::Author => some(&song.authors).unwrap_or_default(),
            T::Publisher => some(&song.publisher).unwrap_or_default(),
            T::Title => some(&song.title).unwrap_or_default(),
            T::CopyrightYear => year.clone().unwrap_or_default(),
            T::LicenseNumber => some(&ccli.license_number).unwrap_or_default(),
            T::SongNumber => some(&song.ccli_number).unwrap_or_default(),
            T::Unknown => String::new(),
        })
        .collect()
}

/// Whether the slide at `place` of a song's `len` slides (play order) shows the copyright line
/// on its own, as `show_on` says.
pub fn ccli_on_slide(show_on: model::CcliDisplay, place: usize, len: usize) -> bool {
    use model::CcliDisplay as D;
    let (first, last) = (place == 0, len > 0 && place + 1 == len);
    match show_on {
        D::FirstSlide | D::Unknown => first,
        D::LastSlide => last,
        D::FirstAndLast => first || last,
        D::AllSlides => true,
    }
}

/// Whether the slide layer's live slide `l` of `p` shows the copyright line on its own: the
/// show's CCLI display is on, the song asks for it (`songInfo.display`) and the slide is one
/// `ccli.showOn` names, counted in the play order (the arrangement; off it, document order).
pub fn ccli_shown(data: &ShowData, p: &model::Presentation, l: &LiveSlide) -> bool {
    let ccli = &data.settings.ccli;
    if !ccli.enabled || l.detached || !p.song_info.as_ref().is_some_and(|s| s.display) {
        return false;
    }
    let (place, len) = play_position(data, p, l)
        .0
        .unwrap_or_else(|| (l.index, p.slides().count()));
    ccli_on_slide(ccli.show_on, place, len)
}

/// The built-in CCLI template (no `ccli.template`): the copyright line in small white text in
/// the bottom-left corner of a `canvas`-sized slide. Its IDs are constants, so a slide planned
/// with it again keeps the same content key (a re-plan mid-dissolve must not restart the slide).
pub fn ccli_builtin(canvas: Size) -> Slide {
    const SLIDE: uuid::Uuid = uuid::uuid!("5e1d0c11-0000-4000-8000-000000000001");
    const TEXT: uuid::Uuid = uuid::uuid!("5e1d0c11-0000-4000-8000-000000000002");
    let (w, h) = (canvas.width, canvas.height);
    let mut t = Element::text(
        Rect::new(w * 0.03, h * 0.78, w * 0.6, h * 0.18),
        TextBox {
            paragraphs: Paragraph::from_lines(""),
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                size: Some((h * 0.026).max(8.0)),
                color: Some(Color::WHITE),
                ..CharStyle::default()
            },
            default_paragraph: ParagraphStyle {
                alignment: Some(TextAlign::Left),
                ..ParagraphStyle::default()
            },
            vertical_align: VerticalAlign::Bottom,
            fit: FitMode::ShrinkToFit,
            ..TextBox::default()
        },
    );
    t.id = model::ElementId::from_uuid(TEXT);
    t.name = "CCLI Copyright".into();
    t.data_link = Some(model::DataLink::CcliText);
    let mut s = Slide::new();
    s.id = model::SlideId::from_uuid(SLIDE);
    s.elements = vec![t];
    s
}

/// `slide` (on a `canvas`-sized slide layer) with the CCLI template's objects drawn on top:
/// the settings' template (drawn at the show's default canvas size) or the built-in one,
/// scaled to `canvas`. Its CCLI text links show the live song's copyright line once the slide
/// is resolved against the live data.
pub fn with_ccli<'a>(
    data: &ShowData,
    slide: std::borrow::Cow<'a, Slide>,
    canvas: Size,
) -> std::borrow::Cow<'a, Slide> {
    let from = data.settings.default_canvas_size;
    let template = match &data.settings.ccli.template {
        Some(t) => t.clone(),
        None => ccli_builtin(from),
    };
    let (sx, sy) = if from.width > 0.0 && from.height > 0.0 {
        (canvas.width / from.width, canvas.height / from.height)
    } else {
        (1.0, 1.0)
    };
    let mut slide = slide.into_owned();
    slide
        .elements
        .extend(template.elements.into_iter().map(|mut e| {
            if (sx, sy) != (1.0, 1.0) {
                let b = e.bounds;
                e.bounds = Rect::new(b.x * sx, b.y * sy, b.width * sx, b.height * sy);
                if let Some(tb) = e.text_box_mut() {
                    scale_text(tb, sx.min(sy));
                }
            }
            e
        }));
    std::borrow::Cow::Owned(slide)
}

/// Scales a text box's font sizes by `f`.
fn scale_text(tb: &mut TextBox, f: f64) {
    let scale = |s: &mut CharStyle| {
        if let Some(v) = s.size.as_mut() {
            *v *= f;
        }
    };
    scale(&mut tb.default_style);
    for p in &mut tb.paragraphs {
        scale(&mut p.end_style);
        for r in &mut p.runs {
            scale(&mut r.style);
        }
    }
}

/// The logical video inputs on the video-input layer (what `VideoInput` visibility conditions
/// test): the input triggered by ID, or, for a source triggered by name or address, every
/// input bound to that source on this machine.
fn live_video_inputs(data: &ShowData, state: &ShowState) -> Vec<model::VideoInputId> {
    let Some(v) = &state.video_input else {
        return Vec::new();
    };
    if let Some(input) = v.input {
        return vec![input];
    }
    data.video_inputs
        .iter()
        .map(|i| i.id)
        .filter(|&id| data.input_source(id).is_ok_and(|s| s == v.source))
        .collect()
}

/// A message's text with its tokens substituted (see the module docs).
pub fn message_text(m: &Message, values: &[TokenValue], ctx: &DataContext) -> String {
    let given = |name: &str| {
        values
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.value.clone())
    };
    let mut subs: Vec<(String, String)> = m
        .tokens
        .iter()
        .map(|t| {
            let v = match &t.kind {
                MessageTokenKind::Text { default_value } => {
                    given(&t.name).unwrap_or_else(|| default_value.clone())
                }
                MessageTokenKind::Timer { timer, format } => ctx
                    .timer(timer.as_ref())
                    .map(|v| format_timer(v.seconds, v.countdown, format))
                    .unwrap_or_default(),
                MessageTokenKind::Clock { format } => ctx
                    .wall
                    .as_ref()
                    .map(|w| format_clock(w, &ctx.clock.apply(format)))
                    .unwrap_or_default(),
                _ => given(&t.name).unwrap_or_default(),
            };
            (t.name.clone(), v)
        })
        .collect();
    // Values for names the message does not declare are still used as text.
    for v in values {
        if !subs.iter().any(|(n, _)| *n == v.name) {
            subs.push((v.name.clone(), v.value.clone()));
        }
    }
    let mut out = m.text.clone();
    for (name, value) in &subs {
        out = out.replace(&format!("${{{name}}}"), value);
    }
    for (name, value) in &subs {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Canvas of the built-in message banner.
pub const BANNER_CANVAS: Size = Size::HD;

/// The built-in lower-third banner showing `text`.
pub fn banner(text: &str) -> Slide {
    let mut bar = Element::new(Rect::new(0.0, 850.0, 1920.0, 180.0));
    bar.name = "Message Bar".into();
    bar.fill = model::Fill::color(Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.72,
    });
    let mut t = Element::text(
        Rect::new(80.0, 850.0, 1760.0, 180.0),
        TextBox {
            paragraphs: Paragraph::from_lines(text),
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                font_weight: Some(700),
                size: Some(64.0),
                color: Some(Color::WHITE),
                ..CharStyle::default()
            },
            default_paragraph: ParagraphStyle {
                alignment: Some(TextAlign::Center),
                ..ParagraphStyle::default()
            },
            vertical_align: VerticalAlign::Middle,
            fit: FitMode::ShrinkToFit,
            ..TextBox::default()
        },
    );
    t.name = "Message".into();
    let mut s = Slide::new();
    s.elements = vec![bar, t];
    s
}

/// The slide (and its canvas) drawing a message with `text` (see the module docs).
pub fn message_slide(data: &ShowData, m: &Message, text: &str) -> (Slide, Size) {
    let template = m.template.as_ref().and_then(|r| {
        let theme = data.content.themes.get(&r.theme.id)?;
        let ts = theme.slides.iter().find(|s| s.id == r.slide)?;
        Some((ts.slide.clone(), theme.canvas_size))
    });
    let Some((mut slide, canvas)) = template else {
        return (banner(text), BANNER_CANVAS);
    };
    let slot = |e: &Element| {
        e.text_box().is_some() && e.placeholder.as_ref().is_some_and(|p| p.takes_text())
    };
    let target = slide.elements.iter().position(slot).or_else(|| {
        slide
            .elements
            .iter()
            .position(|e| e.text_box().is_some() && e.data_link.is_none())
    });
    match target {
        Some(i) => {
            for (k, e) in slide.elements.iter_mut().enumerate() {
                if k != i
                    && e.text_box().is_some()
                    && e.placeholder.as_ref().is_some_and(|p| p.takes_text())
                    && let Some(tb) = e.text_box_mut()
                {
                    render::live::replace_text(tb, "");
                }
            }
            if let Some(tb) = slide.elements[i].text_box_mut() {
                render::live::replace_text(tb, text);
            }
        }
        None => {
            let b = banner(text);
            slide.elements.extend(b.elements);
        }
    }
    slide.actions.clear();
    (slide, canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Link, MessageToken, TimerFormat, TimerId};

    fn msg(text: &str, tokens: Vec<MessageToken>) -> Message {
        Message {
            text: text.into(),
            tokens,
            ..Message::default()
        }
    }

    #[test]
    fn tokens_are_substituted() {
        let timer = TimerId::new();
        let mut ctx = DataContext::default();
        ctx.timers.insert(
            timer,
            render::live::TimerValue {
                seconds: 125.0,
                countdown: true,
                running: true,
                expired: false,
                warning: None,
                total: None,
            },
        );
        let m = msg(
            "Parent of ${child}, please come to ${room} ({mins} left)",
            vec![
                MessageToken {
                    name: "child".into(),
                    kind: MessageTokenKind::Text {
                        default_value: "?".into(),
                    },
                },
                MessageToken {
                    name: "room".into(),
                    kind: MessageTokenKind::Text {
                        default_value: "the nursery".into(),
                    },
                },
                MessageToken {
                    name: "mins".into(),
                    kind: MessageTokenKind::Timer {
                        timer: Some(Link::to(timer)),
                        format: TimerFormat::default(),
                    },
                },
            ],
        );
        let t = message_text(
            &m,
            &[TokenValue {
                name: "child".into(),
                value: "A42".into(),
            }],
            &ctx,
        );
        assert_eq!(t, "Parent of A42, please come to the nursery (02:05 left)");
        // Undeclared values still substitute; unknown placeholders stay.
        let m = msg("${x} and ${y}", vec![]);
        let t = message_text(
            &m,
            &[TokenValue {
                name: "x".into(),
                value: "1".into(),
            }],
            &ctx,
        );
        assert_eq!(t, "1 and ${y}");
    }

    #[test]
    fn built_in_banner_carries_the_text() {
        let s = banner("Hello\nWorld");
        let t = s.elements.iter().find_map(|e| e.text_box()).unwrap();
        assert_eq!(t.plain_text(), "Hello\nWorld");
    }

    #[test]
    fn copyright_line_built_in_and_from_tokens() {
        use model::{CcliSettings, CcliToken, SongInfo};
        let song = SongInfo {
            title: Some("Morning Song".into()),
            authors: Some("A. Writer".into()),
            publisher: Some("Example Music".into()),
            copyright_year: Some(2001),
            ccli_number: Some("1234567".into()),
            ..SongInfo::default()
        };
        let mut ccli = CcliSettings {
            license_number: Some("7654321".into()),
            ..CcliSettings::default()
        };
        assert_eq!(
            ccli_line(&song, &ccli),
            "Morning Song\nA. Writer\n© 2001 Example Music\nCCLI Song # 1234567\nCCLI Licence # 7654321"
        );
        // Empty parts are left out.
        let bare = SongInfo {
            title: Some("Morning Song".into()),
            publisher: Some(" ".into()),
            ..SongInfo::default()
        };
        assert_eq!(ccli_line(&bare, &CcliSettings::default()), "Morning Song");
        // A layout of tokens.
        ccli.layout = vec![
            CcliToken::Title,
            CcliToken::Text { text: " (".into() },
            CcliToken::CopyrightYear,
            CcliToken::Text { text: ") #".into() },
            CcliToken::LicenseNumber,
            CcliToken::Unknown,
        ];
        assert_eq!(ccli_line(&song, &ccli), "Morning Song (2001) #7654321");
    }

    #[test]
    fn ccli_line_slides_follow_show_on() {
        use model::CcliDisplay as D;
        let on = |d, len| {
            (0..len)
                .filter(|&k| ccli_on_slide(d, k, len))
                .collect::<Vec<_>>()
        };
        assert_eq!(on(D::FirstSlide, 4), [0]);
        assert_eq!(on(D::Unknown, 4), [0]);
        assert_eq!(on(D::LastSlide, 4), [3]);
        assert_eq!(on(D::FirstAndLast, 4), [0, 3]);
        assert_eq!(on(D::FirstAndLast, 1), [0]);
        assert_eq!(on(D::AllSlides, 3), [0, 1, 2]);
        // The built-in template: one CCLI text link in the bottom-left corner.
        let s = ccli_builtin(Size::HD);
        assert_eq!(s.elements.len(), 1);
        let e = &s.elements[0];
        assert_eq!(e.data_link, Some(model::DataLink::CcliText));
        assert!(e.bounds.x < 100.0 && e.bounds.y + e.bounds.height > 1000.0);
        // F01-1: built again, it is the same slide (its content key must not change per plan).
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::to_value(ccli_builtin(Size::HD)).unwrap()
        );
    }
}
