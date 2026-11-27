//! Stage display: what stage screens draw.
//!
//! A stage screen (`ScreenKind::Stage`) never shows the audience layers (media, slide, props,
//! messages, announcements). It draws one **stage layout**: a slide whose elements carry data
//! links (current / next slide text, notes, clock, timers, video countdown, stage message,
//! slide pictures), resolved against the frame's [`render::live::DataContext`] like any live
//! slide, plus the **stage message**.
//!
//! - **Which layout** ([`layout_for`]): the most specific of a choice in
//!   [`ShowState::stage_layouts`] for the screen (a stage-layout slide action, or
//!   `setStageLayout`), one for every stage screen (an action without a screen), the screen
//!   document's stored layout (`kind.layout`, written by `setStageLayout`), the first stage
//!   layout in sort order, and the built-in "Current / Next" layout ([`Template::CurrentNext`])
//!   when the data folder has none. A choice naming a deleted layout is skipped.
//! - **Slide pictures** (`DataLink::SlideImage`): the **slide layer only** — the current or
//!   next slide drawn on black at the element's size, with its live data, without background
//!   media, props or messages (the slide the performers read, not the audience composite).
//!   The render thread draws them ([`crate::engine`]) and fills the element with the picture;
//!   the element's own text is not drawn. They change with the slide.
//! - **Stage message** ([`StageMessage`]): shown in the layout's stage-message elements
//!   (`DataLink::Message`), else in a built-in banner along the top ([`banner`]: large white
//!   text on an opaque black bar). Shown with `flash`, it flashes for [`FLASH_FOR`]: the banner
//!   (or a message element whose link has `flash`) turns red and back every
//!   [`FLASH_PERIOD`] / 2.
//! - **Built-in templates** ([`Template`]): "Current / Next", "Lyrics + Clock + Timer" and
//!   "Notes", on a 1920 × 1080 canvas. New layouts start from them
//!   (`POST /api/v1/stage-layouts` with `template`).
//!
//! Stage screens cut: layout changes and new slides never dissolve.

use crate::data::ShowData;
use crate::show::{LiveSlide, ShowState};
use model::{
    CharStyle, ClockFormat, Color, CountdownSource, DataLink, Element, ElementId, FitMode,
    Paragraph, ParagraphStyle, Rect, Screen, ScreenKind, Size, Slide, SlideTextSource, SlideWhich,
    StageLayout, StageLayoutId, TextAlign, TextBox, TimeUnitStyle, TimerFormat, VerticalAlign,
};
use std::time::{Duration, Instant};

/// How long a flashing stage message flashes.
pub const FLASH_FOR: Duration = Duration::from_secs(4);
/// One on + off cycle of the flash.
pub const FLASH_PERIOD: Duration = Duration::from_millis(800);

/// The stage message on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct StageMessage {
    /// The text.
    pub text: String,
    /// Shown with `flash`.
    pub flash: bool,
    /// When it was shown (the flash runs from here).
    pub shown_at: Instant,
}

impl StageMessage {
    /// In an "on" phase of its flash at `now`.
    pub fn flash_on(&self, now: Instant) -> bool {
        if !self.flash {
            return false;
        }
        let t = now.saturating_duration_since(self.shown_at);
        if t >= FLASH_FOR {
            return false;
        }
        let half = FLASH_PERIOD.as_nanos() / 2;
        (t.as_nanos() / half.max(1)).is_multiple_of(2)
    }

    /// When the flash next changes phase (`None` = it does not flash any more).
    pub fn next_flash_change(&self, now: Instant) -> Option<Instant> {
        if !self.flash {
            return None;
        }
        let t = now.saturating_duration_since(self.shown_at);
        if t >= FLASH_FOR {
            return None;
        }
        let half = FLASH_PERIOD / 2;
        let k = t.as_nanos() / half.as_nanos().max(1) + 1;
        let next = self.shown_at + half * k as u32;
        Some(next.min(self.shown_at + FLASH_FOR))
    }
}

/// A built-in stage layout template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    /// Current slide text on top, next slide text below, clock / video countdown / timer
    /// along the bottom.
    CurrentNext,
    /// The current lyrics as large as possible, clock and timer along the bottom.
    LyricsClockTimer,
    /// The current slide's notes, its text in a strip above, clock and timer below.
    Notes,
    /// For the band: the song's key, the current and next slide's lyrics with chords above the
    /// words, the next section's name, clock and timer.
    Musician,
}

impl Template {
    /// Every template, in menu order.
    pub const ALL: [Template; 4] = [
        Template::CurrentNext,
        Template::LyricsClockTimer,
        Template::Notes,
        Template::Musician,
    ];

    /// The wire ID (`currentNext`, …).
    pub fn id(self) -> &'static str {
        match self {
            Template::CurrentNext => "currentNext",
            Template::LyricsClockTimer => "lyricsClockTimer",
            Template::Notes => "notes",
            Template::Musician => "musician",
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Template::CurrentNext => "Current / Next",
            Template::LyricsClockTimer => "Lyrics + Clock + Timer",
            Template::Notes => "Notes",
            Template::Musician => "Musician (Chords)",
        }
    }

    /// The template with this wire ID.
    pub fn from_id(id: &str) -> Option<Template> {
        Template::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The template's slide (1920 × 1080 canvas, [`TEMPLATE_CANVAS`]).
    pub fn slide(self) -> Slide {
        let (w, h) = (TEMPLATE_CANVAS.width, TEMPLATE_CANVAS.height);
        let mut els = Vec::new();
        let grey = Color::rgb(0.55, 0.55, 0.55);
        let yellow = Color::rgb(1.0, 0.84, 0.25);
        // Bottom strip: clock, (video countdown), timer.
        let strip = |els: &mut Vec<Element>, countdown: bool| {
            let y = h - 150.0;
            els.push(rule(Rect::new(0.0, y - 12.0, w, 4.0), grey));
            els.push(named(
                linked(
                    Rect::new(40.0, y, 560.0, 130.0),
                    "12:00 PM",
                    96.0,
                    Color::WHITE,
                    TextAlign::Left,
                    DataLink::Clock {
                        format: ClockFormat::default(),
                    },
                ),
                "Clock",
            ));
            if countdown {
                els.push(named(
                    linked(
                        Rect::new(680.0, y, 560.0, 130.0),
                        "0:00",
                        96.0,
                        Color::rgb(0.45, 0.8, 1.0),
                        TextAlign::Center,
                        DataLink::VideoCountdown {
                            format: short_timer(),
                            source: CountdownSource::Presentation,
                            ignore_looping: true,
                        },
                    ),
                    "Video Countdown",
                ));
            }
            els.push(named(
                linked(
                    Rect::new(w - 600.0, y, 560.0, 130.0),
                    "00:00",
                    96.0,
                    yellow,
                    TextAlign::Right,
                    DataLink::Timer {
                        timer: None,
                        format: TimerFormat::default(),
                        color_triggers: vec![model::ColorTrigger {
                            remaining: 60.0,
                            color: Color::rgb(1.0, 0.25, 0.2),
                        }],
                        show: Default::default(),
                    },
                ),
                "Timer",
            ));
        };
        let current = |r: Rect, size: f64| {
            named(
                linked(
                    r,
                    "Current Slide",
                    size,
                    Color::WHITE,
                    TextAlign::Center,
                    DataLink::SlideText {
                        which: SlideWhich::Current,
                        source: SlideTextSource::AllText,
                        keep_formatting: false,
                    },
                ),
                "Current Slide",
            )
        };
        match self {
            Template::CurrentNext => {
                els.push(current(Rect::new(40.0, 30.0, w - 80.0, 520.0), 110.0));
                els.push(rule(Rect::new(0.0, 570.0, w, 4.0), grey));
                els.push(named(
                    linked(
                        Rect::new(40.0, 590.0, w - 80.0, 310.0),
                        "Next Slide",
                        80.0,
                        Color::rgb(0.7, 0.7, 0.7),
                        TextAlign::Center,
                        DataLink::SlideText {
                            which: SlideWhich::Next,
                            source: SlideTextSource::AllText,
                            keep_formatting: false,
                        },
                    ),
                    "Next Slide",
                ));
                strip(&mut els, true);
            }
            Template::LyricsClockTimer => {
                els.push(current(Rect::new(30.0, 20.0, w - 60.0, h - 200.0), 150.0));
                strip(&mut els, false);
            }
            Template::Notes => {
                els.push(current(Rect::new(40.0, 20.0, w - 80.0, 230.0), 64.0));
                els.push(rule(Rect::new(0.0, 262.0, w, 4.0), grey));
                let mut notes = linked(
                    Rect::new(60.0, 290.0, w - 120.0, h - 470.0),
                    "Slide Notes",
                    72.0,
                    yellow,
                    TextAlign::Left,
                    DataLink::SlideNotes {
                        which: SlideWhich::Current,
                    },
                );
                if let Some(tb) = notes.text_box_mut() {
                    tb.vertical_align = VerticalAlign::Top;
                }
                els.push(named(notes, "Notes"));
                strip(&mut els, false);
            }
            Template::Musician => {
                let top = |r: Rect, preview: &str, size: f64, c: Color, a: TextAlign, l| {
                    let mut e = linked(r, preview, size, c, a, l);
                    if let Some(tb) = e.text_box_mut() {
                        tb.fit = FitMode::ShrinkToFit;
                    }
                    e
                };
                els.push(named(
                    top(
                        Rect::new(40.0, 16.0, 900.0, 90.0),
                        "Key G",
                        64.0,
                        yellow,
                        TextAlign::Left,
                        DataLink::SongKey,
                    ),
                    "Song Key",
                ));
                els.push(named(
                    top(
                        Rect::new(960.0, 16.0, 920.0, 90.0),
                        "Verse 1",
                        64.0,
                        Color::WHITE,
                        TextAlign::Right,
                        DataLink::SectionName {
                            which: model::SectionWhich::CurrentSlide,
                        },
                    ),
                    "Section",
                ));
                els.push(rule(Rect::new(0.0, 118.0, w, 4.0), grey));
                let chart = |r: Rect, preview: &str, size: f64, c: Color, which| {
                    let mut e = linked(
                        r,
                        preview,
                        size,
                        c,
                        TextAlign::Left,
                        DataLink::ChordChart { which },
                    );
                    if let Some(tb) = e.text_box_mut() {
                        tb.vertical_align = VerticalAlign::Top;
                        tb.chords = model::ChordDisplay::shown();
                        // Design-time chords, so the layout's picture shows where they go.
                        if let Some(p) = tb.paragraphs.first_mut() {
                            p.chords = vec![
                                model::ChordMark::new(0, "G"),
                                model::ChordMark::new(8, "D/F#"),
                            ];
                        }
                    }
                    e
                };
                els.push(named(
                    chart(
                        Rect::new(40.0, 136.0, w - 80.0, 520.0),
                        "Current Slide",
                        96.0,
                        Color::WHITE,
                        SlideWhich::Current,
                    ),
                    "Chord Chart",
                ));
                els.push(rule(Rect::new(0.0, 668.0, w, 4.0), grey));
                els.push(named(
                    linked(
                        Rect::new(40.0, 680.0, 380.0, 70.0),
                        "NEXT",
                        44.0,
                        grey,
                        TextAlign::Left,
                        DataLink::Unknown,
                    ),
                    "Next Label",
                ));
                if let Some(e) = els.last_mut() {
                    e.data_link = None;
                }
                els.push(named(
                    top(
                        Rect::new(420.0, 680.0, w - 460.0, 70.0),
                        "Chorus",
                        44.0,
                        Color::rgb(0.45, 0.8, 1.0),
                        TextAlign::Right,
                        DataLink::SectionName {
                            which: model::SectionWhich::NextSection,
                        },
                    ),
                    "Next Section",
                ));
                els.push(named(
                    chart(
                        Rect::new(40.0, 752.0, w - 80.0, 150.0),
                        "Next Slide",
                        60.0,
                        Color::rgb(0.7, 0.7, 0.7),
                        SlideWhich::Next,
                    ),
                    "Next Chord Chart",
                ));
                strip(&mut els, false);
            }
        }
        let mut s = Slide::new();
        s.elements = els;
        s
    }

    /// A new stage layout document made from the template.
    pub fn layout(self) -> StageLayout {
        StageLayout {
            name: self.name().into(),
            size: TEMPLATE_CANVAS,
            slide: self.slide(),
            ..StageLayout::default()
        }
    }
}

/// Canvas of the built-in templates and the stage banner.
pub const TEMPLATE_CANVAS: Size = Size::HD;

fn short_timer() -> TimerFormat {
    TimerFormat {
        hours: TimeUnitStyle::Hidden,
        minutes: TimeUnitStyle::Short,
        ..TimerFormat::default()
    }
}

fn named(mut e: Element, name: &str) -> Element {
    e.name = name.into();
    e
}

fn rule(r: Rect, c: Color) -> Element {
    let mut e = Element::new(r);
    e.name = "Divider".into();
    e.fill = model::Fill::color(c);
    e
}

/// A text element showing `link` (design-time text `preview`), bold DejaVu Sans, shrinking to
/// fit.
pub fn linked(
    r: Rect,
    preview: &str,
    size: f64,
    color: Color,
    align: TextAlign,
    link: DataLink,
) -> Element {
    let mut e = Element::text(
        r,
        TextBox {
            paragraphs: Paragraph::from_lines(preview),
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                font_weight: Some(700),
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
    );
    e.data_link = Some(link);
    e
}

/// The built-in stage message banner (see the module docs), on [`TEMPLATE_CANVAS`].
pub fn banner(text: &str, flash_on: bool) -> Slide {
    let (w, _) = (TEMPLATE_CANVAS.width, TEMPLATE_CANVAS.height);
    let mut bar = Element::new(Rect::new(0.0, 0.0, w, 230.0));
    bar.name = "Stage Message Bar".into();
    bar.fill = model::Fill::color(if flash_on {
        render::live::FLASH_COLOR
    } else {
        Color::BLACK
    });
    let mut edge = Element::new(Rect::new(0.0, 226.0, w, 8.0));
    edge.name = "Stage Message Edge".into();
    edge.fill = model::Fill::color(render::live::FLASH_COLOR);
    let mut t = linked(
        Rect::new(40.0, 10.0, w - 80.0, 210.0),
        text,
        110.0,
        Color::WHITE,
        TextAlign::Center,
        DataLink::Message {
            flash: false,
            flash_color: None,
        },
    );
    // Plain text: the banner is built with the message already in it.
    t.data_link = None;
    t.name = "Stage Message".into();
    let mut s = Slide::new();
    s.elements = vec![bar, edge, t];
    s
}

/// The layout has an element showing the stage message.
pub fn has_message_element(slide: &Slide) -> bool {
    slide
        .elements
        .iter()
        .any(|e| matches!(e.data_link, Some(DataLink::Message { .. })))
}

/// The ID a screen's chosen layout has (`None` = the built-in default), see the module docs.
pub fn layout_id_for(data: &ShowData, state: &ShowState, screen: &Screen) -> Option<StageLayoutId> {
    let exists = |id: &StageLayoutId| data.content.stage_layouts.iter().any(|l| l.id == *id);
    let chosen = |want: Option<model::ScreenId>| {
        state
            .stage_layouts
            .iter()
            .rev()
            .find(|(s, l)| *s == want && exists(l))
            .map(|(_, l)| *l)
    };
    chosen(Some(screen.id))
        .or_else(|| chosen(None))
        .or(match &screen.kind {
            ScreenKind::Stage { layout: Some(l) } if exists(l) => Some(*l),
            _ => None,
        })
        .or_else(|| data.content.stage_layouts.first().map(|l| l.id))
}

/// The layout a stage screen shows: its slide and canvas (the built-in "Current / Next" when
/// there is none).
pub fn layout_for<'a>(
    data: &'a ShowData,
    state: &ShowState,
    screen: &Screen,
) -> (std::borrow::Cow<'a, Slide>, Size, Option<StageLayoutId>) {
    match layout_id_for(data, state, screen)
        .and_then(|id| data.content.stage_layouts.iter().find(|l| l.id == id))
    {
        Some(l) => (std::borrow::Cow::Borrowed(&l.slide), l.size, Some(l.id)),
        None => (
            std::borrow::Cow::Owned(Template::CurrentNext.slide()),
            TEMPLATE_CANVAS,
            None,
        ),
    }
}

/// The chord view of a stage layout (`None` = the built-in layout: the default view).
pub fn chord_view_for(data: &ShowData, id: Option<StageLayoutId>) -> model::ChordView {
    id.and_then(|id| data.content.stage_layouts.iter().find(|l| l.id == id))
        .map(|l| l.chords.clone())
        .unwrap_or_default()
}

/// The layout has a chord chart or a song key element.
pub fn has_chord_links(slide: &Slide) -> bool {
    slide.elements.iter().any(|e| {
        matches!(
            e.data_link,
            Some(DataLink::ChordChart { .. } | DataLink::SongKey)
        )
    })
}

/// The slide (and canvas) a slide-picture link shows: the live slide (`Current`, at its build
/// step) or the one `next` would trigger (with only the builds that play with it).
pub fn picture_source<'a>(
    data: &'a ShowData,
    state: &ShowState,
    which: SlideWhich,
    cursor: Option<LiveSlide>,
) -> Option<(std::borrow::Cow<'a, Slide>, Size)> {
    let live = match which {
        SlideWhich::Next => cursor?,
        _ => state.slide?,
    };
    if live.detached && which != SlideWhich::Next {
        let h = state.held_slide.as_ref()?;
        return Some((std::borrow::Cow::Owned(h.slide.clone()), h.canvas));
    }
    let p = data.content.presentation(live.presentation)?;
    let s = p.slides().nth(live.index)?;
    // Object builds: the current slide at its build step, the next one as it goes live.
    let step = match which {
        SlideWhich::Next => 0,
        _ => state
            .layer_build(model::Layer::Slide)
            .filter(|b| b.slide == s.id)
            .map_or(0, |b| b.step),
    };
    Some((model::build::settled(s, p.canvas_size, step), p.canvas_size))
}

/// Elements of a layout that show a slide picture, with the slide they show.
pub fn picture_links(slide: &Slide) -> Vec<(ElementId, SlideWhich)> {
    slide
        .elements
        .iter()
        .filter(|e| !e.hidden)
        .filter_map(|e| match e.data_link {
            Some(DataLink::SlideImage { which }) => Some((e.id, which)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_phases_and_end() {
        let t0 = Instant::now();
        let m = StageMessage {
            text: "x".into(),
            flash: true,
            shown_at: t0,
        };
        assert!(m.flash_on(t0));
        assert!(!m.flash_on(t0 + FLASH_PERIOD / 2 + Duration::from_millis(1)));
        assert!(m.flash_on(t0 + FLASH_PERIOD + Duration::from_millis(1)));
        assert!(!m.flash_on(t0 + FLASH_FOR));
        assert_eq!(m.next_flash_change(t0), Some(t0 + FLASH_PERIOD / 2));
        assert_eq!(m.next_flash_change(t0 + FLASH_FOR), None);
        let still = StageMessage { flash: false, ..m };
        assert!(!still.flash_on(t0));
        assert_eq!(still.next_flash_change(t0), None);
    }

    #[test]
    fn templates_link_what_they_promise() {
        for t in Template::ALL {
            assert_eq!(Template::from_id(t.id()), Some(t));
            let s = t.slide();
            let links: Vec<&DataLink> = s
                .elements
                .iter()
                .filter_map(|e| e.data_link.as_ref())
                .collect();
            assert!(
                links.iter().any(|l| matches!(l, DataLink::Clock { .. })),
                "{t:?}"
            );
            assert!(
                links.iter().any(|l| matches!(l, DataLink::Timer { .. })),
                "{t:?}"
            );
            assert!(!has_message_element(&s));
        }
        let cn = Template::CurrentNext.slide();
        assert!(cn.elements.iter().any(|e| matches!(
            e.data_link,
            Some(DataLink::SlideText {
                which: SlideWhich::Next,
                ..
            })
        )));
        assert!(
            Template::Notes
                .slide()
                .elements
                .iter()
                .any(|e| matches!(e.data_link, Some(DataLink::SlideNotes { .. })))
        );
    }

    #[test]
    fn banner_flashes_red() {
        let b = banner("Wrap up", false);
        assert_eq!(b.elements[0].fill, model::Fill::color(Color::BLACK));
        let f = banner("Wrap up", true);
        assert_eq!(
            f.elements[0].fill,
            model::Fill::color(render::live::FLASH_COLOR)
        );
        assert_eq!(f.elements[2].text_box().unwrap().plain_text(), "Wrap up");
    }

    // ---- planning ------------------------------------------------------------------------

    use crate::show::{ShowState, plan_screen};
    use model::{
        Library, Link, PlaylistItem, PlaylistItemKind, PlaylistNode, PlaylistNodeKind,
        Presentation, Section,
    };

    struct Fx {
        data: ShowData,
        stage: Screen,
        song: Presentation,
        other: Presentation,
        a: StageLayout,
        b: StageLayout,
        playlist: PlaylistNode,
    }

    fn put<T: serde::Serialize>(dir: &std::path::Path, kind: &str, id: uuid::Uuid, doc: &T) {
        let d = dir.join("portable").join(kind);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(format!("{id}.json")),
            serde_json::to_vec(doc).unwrap(),
        )
        .unwrap();
    }

    fn text_slide(t: &str) -> Slide {
        let mut s = Slide::new();
        s.elements = vec![Element::text(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            TextBox::plain(t),
        )];
        s
    }

    fn fx(with_layouts: bool) -> Fx {
        let dir = std::env::temp_dir().join(format!("stage-{}", uuid::Uuid::new_v4()));
        let lib = Library::default();
        put(&dir, "library", lib.id.0, &lib);
        let mut sec = Section::new("Verse");
        sec.slides = vec![text_slide("ONE"), text_slide("TWO")];
        let song = Presentation {
            library: Some(lib.id),
            sections: vec![sec],
            ..Presentation::new("Song")
        };
        let mut sec = Section::new("Chorus");
        sec.slides = vec![text_slide("B1")];
        let other = Presentation {
            library: Some(lib.id),
            sections: vec![sec],
            ..Presentation::new("Other")
        };
        put(&dir, "presentation", song.id.0, &song);
        put(&dir, "presentation", other.id.0, &other);
        let item = |p: &Presentation| {
            PlaylistItem::new(PlaylistItemKind::Presentation {
                presentation: Link::named(p.id, p.name.clone()),
                arrangement: None,
                unresolved: None,
            })
        };
        let playlist = PlaylistNode {
            name: "Sunday".into(),
            kind: PlaylistNodeKind::Playlist {
                purpose: model::PlaylistPurpose::Service,
                items: vec![item(&song), item(&other)],
            },
            ..PlaylistNode::default()
        };
        put(&dir, "playlistNode", playlist.id.0, &playlist);
        let stage = Screen {
            name: "Stage".into(),
            kind: ScreenKind::Stage { layout: None },
            ..Screen::default()
        };
        put(&dir, "screen", stage.id.0, &stage);
        let mut a = Template::CurrentNext.layout();
        a.sort_key = "a".into();
        let mut b = Template::Notes.layout();
        b.sort_key = "b".into();
        let mut msg = linked(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            "",
            40.0,
            Color::WHITE,
            TextAlign::Center,
            DataLink::Message {
                flash: true,
                flash_color: None,
            },
        );
        msg.name = "Msg".into();
        b.slide.elements.push(msg);
        if with_layouts {
            put(&dir, "stageLayout", a.id.0, &a);
            put(&dir, "stageLayout", b.id.0, &b);
        }
        let data = ShowData::load(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Fx {
            data,
            stage,
            song,
            other,
            a,
            b,
            playlist,
        }
    }

    fn texts(plan: &crate::show::ScreenPlan<'_>) -> Vec<(String, String)> {
        plan.slides[0]
            .slide
            .elements
            .iter()
            .filter_map(|e| Some((e.name.clone(), e.text_box()?.plain_text())))
            .collect()
    }

    fn text_of(plan: &crate::show::ScreenPlan<'_>, name: &str) -> String {
        texts(plan)
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t)
            .unwrap_or_default()
    }

    fn plan_now<'a>(f: &'a Fx, state: &ShowState) -> crate::show::ScreenPlan<'a> {
        let ctx = crate::live::data_context(&f.data, state, crate::timers::Clock::now());
        plan_screen(&f.data, state, &f.stage, Some(&ctx))
    }

    #[test]
    fn stage_screens_show_current_and_next_and_no_audience_layers() {
        let f = fx(false);
        let mut st = ShowState::default();
        // No layouts in the data: the built-in "Current / Next".
        assert_eq!(layout_id_for(&f.data, &st, &f.stage), None);
        let plan = plan_now(&f, &st);
        assert!(plan.stage);
        assert_eq!(text_of(&plan, "Current Slide"), "");
        st.trigger(&f.data, f.song.id, None, Some(0), None, None, None)
            .unwrap();
        let plan = plan_now(&f, &st);
        assert_eq!(text_of(&plan, "Current Slide"), "ONE");
        assert_eq!(text_of(&plan, "Next Slide"), "TWO");
        assert!(plan.media.is_none() && plan.props.is_empty() && plan.message.is_none());
        assert_eq!(plan.slides.len(), 1);
        assert!(plan.live.clock && plan.live.timers);
        // Next follows the playlist into the next presentation.
        let ctx_item = protocol::PlaylistItemRef {
            playlist: f.playlist.id,
            item: match &f.playlist.kind {
                PlaylistNodeKind::Playlist { items, .. } => items[0].id,
                _ => unreachable!(),
            },
        };
        st.trigger(
            &f.data,
            f.song.id,
            None,
            Some(1),
            Some(ctx_item),
            None,
            None,
        )
        .unwrap();
        let plan = plan_now(&f, &st);
        assert_eq!(text_of(&plan, "Current Slide"), "TWO");
        assert_eq!(text_of(&plan, "Next Slide"), "B1");
        let _ = &f.other;
        // Switched off: blank.
        st.screens_enabled.set(protocol::ScreenRole::Stage, false);
        assert!(plan_now(&f, &st).blank);
    }

    #[test]
    fn layout_choice_and_stage_message() {
        let f = fx(true);
        let mut st = ShowState::default();
        // The first layout by default.
        assert_eq!(layout_id_for(&f.data, &st, &f.stage), Some(f.a.id));
        // The screen document's choice.
        let mut screen = f.stage.clone();
        screen.kind = ScreenKind::Stage {
            layout: Some(f.b.id),
        };
        assert_eq!(layout_id_for(&f.data, &st, &screen), Some(f.b.id));
        // A show choice for every stage screen, then one for this screen, win.
        st.stage_layouts.push((None, f.b.id));
        assert_eq!(layout_id_for(&f.data, &st, &f.stage), Some(f.b.id));
        st.stage_layouts.push((Some(f.stage.id), f.a.id));
        assert_eq!(layout_id_for(&f.data, &st, &f.stage), Some(f.a.id));
        // A deleted layout is skipped.
        st.stage_layouts
            .push((Some(f.stage.id), StageLayoutId::new()));
        assert_eq!(layout_id_for(&f.data, &st, &f.stage), Some(f.a.id));

        // Layout A has no message element: the banner.
        st.stage_message = Some(StageMessage {
            text: "Wrap up".into(),
            flash: true,
            shown_at: Instant::now(),
        });
        let plan = plan_now(&f, &st);
        let banner = plan.message.as_ref().expect("banner");
        assert_eq!(banner.layer, model::Layer::Messages);
        assert!(
            banner
                .slide
                .elements
                .iter()
                .any(|e| e.text_box().is_some_and(|t| t.plain_text() == "Wrap up"))
        );
        assert_eq!(
            banner.slide.elements[0].fill,
            model::Fill::color(render::live::FLASH_COLOR)
        );
        assert!(plan.live.flash);
        // Layout B shows it in its own element (which flashes), no banner.
        st.stage_layouts.push((Some(f.stage.id), f.b.id));
        let plan = plan_now(&f, &st);
        assert!(plan.message.is_none());
        assert_eq!(text_of(&plan, "Msg"), "Wrap up");
        let msg = plan.slides[0]
            .slide
            .elements
            .iter()
            .find(|e| e.name == "Msg")
            .unwrap();
        assert_eq!(msg.fill, model::Fill::color(render::live::FLASH_COLOR));
        // Cleared.
        st.stage_message = None;
        let plan = plan_now(&f, &st);
        assert_eq!(text_of(&plan, "Msg"), "");
    }

    #[test]
    fn chord_charts_follow_each_layouts_chord_view() {
        let f = fx(false);
        // A song written in G, played in A; a line with chords.
        let mut words = Element::text(Rect::new(0.0, 0.0, 100.0, 50.0), TextBox::plain("Sing"));
        if let Some(tb) = words.text_box_mut() {
            tb.paragraphs[0].chords = vec![
                model::ChordMark::new(0, "G"),
                model::ChordMark::new(3, "D/F#"),
            ];
        }
        let mut sec = Section::new("Verse 1");
        sec.slides = vec![Slide {
            elements: vec![words],
            ..Slide::new()
        }];
        let song = Presentation {
            library: f.song.library,
            sections: vec![sec, {
                let mut c = Section::new("Chorus");
                c.slides = vec![text_slide("Chorus words")];
                c
            }],
            song_info: Some(model::SongInfo {
                original_key: Some("G".into()),
                transpose: 2,
                ..Default::default()
            }),
            ..Presentation::new("Band")
        };
        let dir = std::env::temp_dir().join(format!("stage-{}", uuid::Uuid::new_v4()));
        let lib = Library::default();
        put(&dir, "library", lib.id.0, &lib);
        let song = Presentation {
            library: Some(lib.id),
            ..song
        };
        put(&dir, "presentation", song.id.0, &song);
        put(&dir, "screen", f.stage.id.0, &f.stage);
        let mut guitar = Template::Musician.layout();
        guitar.sort_key = "a".into();
        guitar.chords = model::ChordView {
            capo: 2,
            ..Default::default()
        };
        let mut keys = Template::Musician.layout();
        keys.sort_key = "b".into();
        put(&dir, "stageLayout", guitar.id.0, &guitar);
        put(&dir, "stageLayout", keys.id.0, &keys);
        let data = ShowData::load(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let mut st = ShowState::default();
        st.trigger(&data, song.id, None, Some(0), None, None, None)
            .unwrap();
        let chart = |st: &ShowState| {
            let ctx = crate::live::data_context(&data, st, crate::timers::Clock::now());
            let plan = plan_screen(&data, st, &f.stage, Some(&ctx));
            let el = |name: &str| {
                plan.slides[0]
                    .slide
                    .elements
                    .iter()
                    .find(|e| e.name == name)
                    .unwrap()
                    .clone()
            };
            let c = el("Chord Chart");
            let tb = c.text_box().unwrap();
            assert!(tb.chords.show);
            let chords: Vec<String> = tb.paragraphs[0]
                .chords
                .iter()
                .map(|c| c.chord.clone())
                .collect();
            (
                tb.plain_text(),
                chords,
                el("Song Key").text_box().unwrap().plain_text(),
                el("Next Section").text_box().unwrap().plain_text(),
            )
        };
        // The guitarist (capo 2) reads G shapes; the key line names both.
        let (text, chords, key, next) = chart(&st);
        assert_eq!(text, "Sing");
        assert_eq!(chords, ["G", "D/F#"]);
        assert_eq!(key, "Key A · Capo 2 (G shapes)");
        assert_eq!(next, "Chorus");
        // The keys player's layout: concert pitch.
        st.stage_layouts.push((Some(f.stage.id), keys.id));
        let (_, chords, key, _) = chart(&st);
        assert_eq!(chords, ["A", "E/G#"]);
        assert_eq!(key, "Key A");
    }

    #[test]
    fn slide_pictures_name_their_slides() {
        let f = fx(false);
        let mut layout = Template::CurrentNext.slide();
        let mut pic = Element::new(Rect::new(0.0, 0.0, 480.0, 270.0));
        pic.data_link = Some(DataLink::SlideImage {
            which: SlideWhich::Next,
        });
        let pid = pic.id;
        layout.elements.push(pic);
        assert_eq!(picture_links(&layout), vec![(pid, SlideWhich::Next)]);
        let mut st = ShowState::default();
        st.trigger(&f.data, f.song.id, None, Some(0), None, None, None)
            .unwrap();
        let cursor = st.cursor(&f.data);
        let (s, _) = picture_source(&f.data, &st, SlideWhich::Next, cursor).unwrap();
        assert_eq!(s.elements[0].text_box().unwrap().plain_text(), "TWO");
        let (s, _) = picture_source(&f.data, &st, SlideWhich::Current, cursor).unwrap();
        assert_eq!(s.elements[0].text_box().unwrap().plain_text(), "ONE");
    }
}
