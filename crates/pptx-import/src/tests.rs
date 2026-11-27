//! Imports of synthetic decks (built in [`crate::synthetic`]; no real files).

use super::*;
use crate::render::Renderer;
use crate::synthetic::{self, DeckSpec, SlideSpec};
use model::{
    Baseline, Capitalization, FitMode, LineStyle, ListKind, Shape, TextAlign, VerticalAlign,
};

fn off() -> Options {
    Options {
        renderer: Renderer::Off,
        ..Options::default()
    }
}

fn run(d: &DeckSpec, o: &Options) -> Deck {
    import(&synthetic::build(d), o, &mut |_, _, _| {}).expect("imports")
}

fn one(spec: SlideSpec) -> DeckSpec {
    DeckSpec {
        slides: vec![spec],
        ..DeckSpec::default()
    }
}

fn texts(s: &Slide) -> Vec<&Element> {
    s.elements
        .iter()
        .filter(|e| matches!(e.kind, ElementKind::Text { .. }))
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn placeholders_take_their_place_and_style_from_layout_master_and_theme() {
    let d = run(
        &one(SlideSpec {
            shapes: format!(
                "{}{}",
                synthetic::title(2, "Welcome"),
                synthetic::body(3, &["First point", "Second point"])
            ),
            ..SlideSpec::default()
        }),
        &off(),
    );
    let p = &d.presentation;
    assert_eq!(p.name, "Synthetic deck");
    assert_eq!(p.canvas_size, Size::new(1920.0, 1080.0));
    assert_eq!(p.import.as_ref().unwrap().source, ImportSource::PowerPoint);
    assert_eq!(p.sections.len(), 1);
    let s = &p.sections[0].slides[0];
    assert_eq!(s.background, Some(Color::WHITE));
    assert_eq!(s.label.as_ref().unwrap().text, "Welcome");
    // The master's bar first (under the slide's shapes), in accent1.
    let bar = &s.elements[0];
    assert_eq!(bar.name, "Footer bar");
    assert_eq!(bar.fill, Fill::color(theme::hex("2E75B6").unwrap()));
    assert!(near(bar.bounds.y, 1080.0 - 54.0), "{:?}", bar.bounds);
    let t = texts(s);
    assert_eq!(t.len(), 2);
    // Title: the master's place (838200 EMU = 132 px), the layout's centring and bold, the
    // theme's heading font, 44 pt = 88 px on a 960 pt wide slide, dk1 through tx1.
    let title = t[0];
    assert!(near(title.bounds.x, 132.0), "{:?}", title.bounds);
    let tb = title.text_box().unwrap();
    assert_eq!(tb.plain_text(), "Welcome");
    let st = tb.run_style(&tb.paragraphs[0].runs[0]);
    assert_eq!(st.font_family.as_deref(), Some("Georgia"));
    assert_eq!(st.font_weight, Some(700));
    assert_eq!(st.size, Some(88.0));
    assert_eq!(st.color, theme::hex("1A1A1A"));
    assert_eq!(
        tb.paragraph_style(&tb.paragraphs[0]).alignment,
        Some(TextAlign::Center)
    );
    assert_eq!(tb.vertical_align, VerticalAlign::Middle);
    // Body: the body font, 28 pt, bullets with the master's indents, shrink to fit.
    let body = t[1].text_box().unwrap();
    assert_eq!(body.paragraphs.len(), 2);
    let ps = body.paragraph_style(&body.paragraphs[1]);
    assert_eq!(ps.list.as_ref().map(|l| l.kind), Some(ListKind::Disc));
    assert_eq!(ps.indent_start, Some(36.0));
    assert_eq!(ps.indent_first_line, Some(-36.0));
    assert_eq!(ps.space_before, Some(20.0));
    let rs = body.run_style(&body.paragraphs[1].runs[0]);
    assert_eq!(rs.font_family.as_deref(), Some("Verdana"));
    assert_eq!(rs.size, Some(56.0));
    assert_eq!(body.fit, FitMode::ShrinkToFit);
    assert_eq!(d.report.slides[0].editable, 3);
    assert!(
        d.report.renderer_problem.is_none(),
        "no pictures were needed"
    );
}

#[test]
fn a_text_box_keeps_its_run_formatting() {
    let paras = r#"
      <a:p><a:pPr algn="r"/><a:r><a:rPr sz="3200" b="1"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:latin typeface="Arial"/></a:rPr><a:t>Bold red</a:t></a:r><a:r><a:rPr sz="3200" i="1" u="sng" cap="all" baseline="30000"><a:latin typeface="Arial"/></a:rPr><a:t> italic</a:t></a:r><a:br/><a:r><a:rPr sz="3200" strike="sngStrike"><a:latin typeface="Arial"/></a:rPr><a:t>next line</a:t></a:r></a:p>
      <a:p><a:pPr marL="342900" indent="-342900"><a:buAutoNum type="arabicPeriod"/></a:pPr><a:r><a:rPr sz="2000"/><a:t>Numbered</a:t></a:r></a:p>
      <a:p><a:pPr><a:lnSpc><a:spcPct val="150000"/></a:lnSpc></a:pPr><a:endParaRPr sz="2000"/></a:p>"#;
    let d = run(
        &one(SlideSpec {
            shapes: synthetic::text_box(
                4,
                "Caption",
                (1_219_200, 609_600, 6_096_000, 1_828_800),
                r#"<a:bodyPr lIns="0" tIns="0" rIns="0" bIns="0" anchor="b"><a:noAutofit/></a:bodyPr>"#,
                paras,
            ),
            ..SlideSpec::default()
        }),
        &off(),
    );
    let s = &d.presentation.sections[0].slides[0];
    let e = texts(s)[0];
    assert_eq!(e.name, "Caption");
    assert_eq!(e.bounds, Rect::new(192.0, 96.0, 960.0, 288.0));
    assert!(!e.fill.enabled, "noFill");
    let tb = e.text_box().unwrap();
    assert_eq!(tb.vertical_align, VerticalAlign::Bottom);
    assert_eq!(tb.fit, FitMode::None);
    assert_eq!(tb.margins, EdgeInsets::default());
    assert_eq!(tb.paragraphs.len(), 3);
    let p0 = &tb.paragraphs[0];
    assert_eq!(p0.plain_text(), "Bold red italic\u{2028}next line");
    let r = |i: usize| tb.run_style(&p0.runs[i]);
    assert_eq!(r(0).font_weight, Some(700));
    assert_eq!(r(0).color, Some(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(r(0).size, Some(64.0));
    assert_eq!(r(0).font_family.as_deref(), Some("Arial"));
    assert_eq!(r(1).italic, Some(true));
    assert_eq!(r(1).underline, Some(LineStyle::Single));
    assert_eq!(r(1).capitalization, Some(Capitalization::AllCaps));
    assert_eq!(r(1).baseline, Some(Baseline::Superscript));
    assert_eq!(r(1).font_weight, Some(400));
    assert_eq!(r(3).strikethrough, Some(LineStyle::Single));
    assert_eq!(tb.paragraph_style(p0).alignment, Some(TextAlign::Right));
    // Runs keep only what differs from the box's style.
    assert!(p0.runs[0].style.is_empty(), "{:?}", p0.runs[0].style);
    let p1 = tb.paragraph_style(&tb.paragraphs[1]);
    assert_eq!(p1.list.as_ref().map(|l| l.kind), Some(ListKind::Decimal));
    assert_eq!(p1.alignment, Some(TextAlign::Left));
    assert_eq!(p1.indent_start, Some(54.0));
    let p2 = &tb.paragraphs[2];
    assert!(p2.runs.is_empty());
    assert_eq!(tb.paragraph_style(p2).line_height_multiple, Some(1.5));
    assert_eq!(tb.empty_paragraph_style(p2).size, Some(40.0));
}

#[test]
fn pictures_shapes_and_backgrounds() {
    let red = synthetic::png(40, 20, [255, 0, 0]);
    let blue = synthetic::png(16, 9, [0, 0, 255]);
    let shapes = format!(
        "{}{}{}{}",
        synthetic::picture(
            5,
            "Photo",
            "rIdImg",
            (0, 0, 3_048_000, 1_524_000),
            r#"<a:srcRect l="25000" r="25000"/>"#
        ),
        synthetic::picture(
            6,
            "Photo again",
            "rIdImg",
            (3_048_000, 0, 3_048_000, 1_524_000),
            ""
        ),
        synthetic::shape(
            7,
            "Badge",
            "ellipse",
            (6_096_000, 3_048_000, 1_524_000, 1_524_000),
            r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FFFFFF"/></a:gs><a:gs pos="100000"><a:schemeClr val="accent1"/></a:gs></a:gsLst><a:lin ang="5400000"/></a:gradFill>"#
        ),
        synthetic::shape(
            8,
            "Box",
            "roundRect",
            (0, 3_048_000, 1_524_000, 1_524_000),
            r#"<a:solidFill><a:srgbClr val="00FF00"><a:alpha val="50000"/></a:srgbClr></a:solidFill>"#
        ),
    );
    let d = run(
        &one(SlideSpec {
            shapes,
            media: vec![
                ("rIdImg".into(), "image1.png".into(), red),
                ("rIdBg".into(), "image2.png".into(), blue),
            ],
            background: Some(
                r#"<p:bg><p:bgPr><a:blipFill><a:blip r:embed="rIdBg"/><a:stretch><a:fillRect/></a:stretch></a:blipFill><a:effectLst/></p:bgPr></p:bg>"#.into(),
            ),
            ..SlideSpec::default()
        }),
        &off(),
    );
    // One file per picture part, though the photo is used twice.
    assert_eq!(d.media.len(), 2);
    assert!(d.media[0].name.starts_with("Synthetic deck "));
    let s = &d.presentation.sections[0].slides[0];
    // Background picture at the bottom, full canvas, stretched.
    let bg = &s.elements[0];
    assert_eq!(bg.name, "Background");
    assert_eq!(bg.bounds, Rect::full(Size::new(1920.0, 1080.0)));
    let FillKind::Media { asset, display, .. } = &bg.fill.kind else {
        panic!("background picture: {:?}", bg.fill)
    };
    assert_eq!(display.scale, ScaleMode::Stretch);
    assert!(d.media.iter().any(|m| m.asset == *asset));
    let photo = s.elements.iter().find(|e| e.name == "Photo").unwrap();
    let FillKind::Media {
        asset: a1, display, ..
    } = &photo.fill.kind
    else {
        panic!()
    };
    // 25 % of 40 px off each side.
    assert_eq!(display.crop.left, 10.0);
    assert_eq!(display.crop.right, 10.0);
    let again = s.elements.iter().find(|e| e.name == "Photo again").unwrap();
    let FillKind::Media { asset: a2, .. } = &again.fill.kind else {
        panic!()
    };
    assert_eq!(a1, a2);
    let badge = s.elements.iter().find(|e| e.name == "Badge").unwrap();
    assert!(matches!(
        badge.kind,
        ElementKind::Shape {
            shape: Shape::Ellipse
        }
    ));
    let FillKind::Gradient { gradient } = &badge.fill.kind else {
        panic!("{:?}", badge.fill)
    };
    assert_eq!(gradient.angle, 90.0);
    assert_eq!(gradient.stops.len(), 2);
    assert!(badge.stroke.enabled);
    assert_eq!(badge.stroke.width, 4.0);
    let bx = s.elements.iter().find(|e| e.name == "Box").unwrap();
    assert!(matches!(
        bx.kind,
        ElementKind::Shape {
            shape: Shape::RoundedRectangle { .. }
        }
    ));
    assert_eq!(bx.fill, Fill::color(Color::rgba(0.0, 1.0, 0.0, 0.5)));
}

#[test]
fn slide_size_notes_sections_and_hidden_slides() {
    let slide = |t: &str| SlideSpec {
        shapes: synthetic::title(2, t),
        notes: Some(format!("Notes for {t}\nSecond line")),
        ..SlideSpec::default()
    };
    let mut hidden = slide("Gamma");
    hidden.hidden = true;
    let d = run(
        &DeckSpec {
            size: (9_144_000, 6_858_000),
            slides: vec![slide("Alpha"), slide("Beta"), hidden],
            sections: vec![("Opening".into(), vec![0]), ("Main".into(), vec![1, 2])],
            ..DeckSpec::default()
        },
        &Options {
            name: Some("Named here".into()),
            ..off()
        },
    );
    let p = &d.presentation;
    assert_eq!(p.name, "Named here");
    assert_eq!(p.canvas_size, Size::new(1920.0, 1440.0));
    let names: Vec<&str> = p.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Opening", "Main"]);
    assert_eq!(p.sections[1].slides.len(), 2);
    let s = &p.sections[0].slides[0];
    assert_eq!(s.notes.plain_text(), "Notes for Alpha\nSecond line");
    assert!(s.enabled);
    assert!(!p.sections[1].slides[1].enabled);
    assert!(d.report.slides[2].hidden);
}

#[test]
fn what_isnt_editable_without_pictures_is_approximated_or_left_out() {
    let cloud = r#"<p:sp><p:nvSpPr><p:cNvPr id="9" name="Cloud"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1219200" cy="1219200"/></a:xfrm><a:prstGeom prst="cloud"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FFFF00"/></a:solidFill></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Think</a:t></a:r></a:p></p:txBody></p:sp>"#;
    let d = run(
        &one(SlideSpec {
            shapes: format!(
                "{}{cloud}",
                synthetic::chart(
                    8,
                    "Sales chart",
                    (2_000_000, 2_000_000, 4_000_000, 3_000_000)
                )
            ),
            ..SlideSpec::default()
        }),
        &off(),
    );
    let r = &d.report.slides[0];
    assert_eq!(r.picture, None);
    assert_eq!(r.approximated.len(), 2, "{:?}", r.approximated);
    assert_eq!(r.approximated[0].kind, "chart");
    assert!(r.approximated[0].detail.contains("left out"));
    assert!(r.approximated[1].kind.contains("cloud"));
    assert!(r.approximated[1].detail.contains("rectangle"));
    assert_eq!(
        d.report.renderer_problem.as_deref(),
        Some("slide pictures are turned off")
    );
    let s = &d.presentation.sections[0].slides[0];
    let think = texts(s)[0];
    assert_eq!(think.text_box().unwrap().plain_text(), "Think");
    assert!(d.media.is_empty());
}

#[test]
fn the_first_slide_falls_back_to_the_deck_thumbnail() {
    let d = run(
        &DeckSpec {
            slides: vec![
                SlideSpec {
                    shapes: synthetic::chart(8, "Chart", (6_096_000, 0, 6_096_000, 3_429_000)),
                    ..SlideSpec::default()
                },
                SlideSpec {
                    shapes: synthetic::chart(8, "Chart", (0, 0, 6_096_000, 3_429_000)),
                    ..SlideSpec::default()
                },
            ],
            thumbnail: Some(synthetic::png(256, 144, [0, 128, 0])),
            ..DeckSpec::default()
        },
        &off(),
    );
    assert_eq!(d.report.slides[0].picture, Some(PictureSource::Thumbnail));
    assert_eq!(d.report.slides[1].picture, None);
    assert_eq!(d.media.len(), 1);
    let s = &d.presentation.sections[0].slides[0];
    let pic = s
        .elements
        .iter()
        .find(|e| e.name == "Chart (picture)")
        .unwrap();
    // The right half of the canvas, from the right half of the 256 px thumbnail.
    assert_eq!(pic.bounds, Rect::new(960.0, 0.0, 960.0, 540.0));
    let FillKind::Media { display, .. } = &pic.fill.kind else {
        panic!()
    };
    assert_eq!(display.crop.left, 128.0);
    assert_eq!(display.crop.bottom, 72.0);
}

#[test]
fn group_members_are_placed_through_the_group() {
    let inner = synthetic::shape(11, "In group", "rect", (0, 0, 1_000, 1_000), "");
    let group = format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="10" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="6096000" y="0"/><a:ext cx="1219200" cy="1219200"/><a:chOff x="0" y="0"/><a:chExt cx="1000" cy="1000"/></a:xfrm></p:grpSpPr>{inner}</p:grpSp>"#
    );
    let d = run(
        &one(SlideSpec {
            shapes: group,
            ..SlideSpec::default()
        }),
        &off(),
    );
    let s = &d.presentation.sections[0].slides[0];
    let e = s.elements.iter().find(|e| e.name == "In group").unwrap();
    assert_eq!(e.bounds, Rect::new(960.0, 0.0, 192.0, 192.0));
}

/// Runs `f` on a thread with the 2 MiB stack the server's import threads have.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

/// A slide nested far deeper than any real deck is refused, not a stack overflow (which
/// aborted the server); groups nested as deep as allowed still import, and group scales that
/// multiply past `f64::MAX` stay finite (the saved presentation can't hold infinity or NaN).
#[test]
fn deep_nesting_and_huge_group_scales_are_safe() {
    on_small_stack(|| {
        let deep = one(SlideSpec {
            shapes: "<p:sp>".repeat(200_000),
            ..SlideSpec::default()
        });
        let e = import(&synthetic::build(&deep), &off(), &mut |_, _, _| {})
            .err()
            .expect("refused");
        assert!(format!("{e:#}").contains("nested too deeply"), "{e:#}");

        let levels = crate::opc::MAX_DEPTH - 12;
        let open = r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="10" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4611686018427387903" cy="4611686018427387903"/><a:chOff x="0" y="0"/><a:chExt cx="1" cy="1"/></a:xfrm></p:grpSpPr>"#;
        let inner = synthetic::shape(11, "Deep inside", "rect", (1, 1, 1_000, 1_000), "");
        let shapes = format!(
            "{}{inner}{}",
            open.repeat(levels),
            "</p:grpSp>".repeat(levels)
        );
        let d = run(
            &one(SlideSpec {
                shapes,
                ..SlideSpec::default()
            }),
            &off(),
        );
        let s = &d.presentation.sections[0].slides[0];
        assert!(s.elements.iter().any(|e| e.name == "Deep inside"));
        for e in &s.elements {
            assert!(finite_element(e), "{e:?}");
        }
        let json = serde_json::to_string(&d.presentation).unwrap();
        serde_json::from_str::<Presentation>(&json).expect("reads back");
    });
}

/// A slide size one EMU high (canvas 0 px high, pictures cut at infinite scale) is refused.
#[test]
fn a_broken_slide_size_is_refused() {
    for size in [(12_192_000, 1), (1, 6_858_000), (0, 100)] {
        let d = DeckSpec {
            size,
            thumbnail: Some(synthetic::png(8, 8, [1, 2, 3])),
            slides: vec![SlideSpec {
                shapes: synthetic::chart(8, "Chart", (0, 0, 10, 1)),
                ..SlideSpec::default()
            }],
            ..DeckSpec::default()
        };
        let e = import(&synthetic::build(&d), &off(), &mut |_, _, _| {})
            .err()
            .expect("refused");
        assert!(format!("{e:#}").contains("slide size"), "{e:#}");
    }
}

#[test]
fn not_a_presentation() {
    let e = import(b"not a zip at all, just text", &off(), &mut |_, _, _| {})
        .err()
        .unwrap();
    assert!(format!("{e:#}").contains(".pptx"), "{e:#}");
    let mut w = zip::Writer::new();
    w.add("Index/Document.iwa", b"x").unwrap();
    let k = w.finish().unwrap();
    let e = import(&k, &off(), &mut |_, _, _| {}).err().unwrap();
    assert!(format!("{e:#}").contains("Keynote"), "{e:#}");
}

/// LibreOffice is stood in for by a script that writes a picture for each package it gets.
#[cfg(unix)]
#[test]
fn slide_pictures_come_from_libreoffice() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("pptx-import-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pic = dir.join("pic.png");
    std::fs::write(&pic, synthetic::png(1920, 1080, [200, 30, 30])).unwrap();
    let script = dir.join("soffice");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --outdir) out=\"$2\"; shift 2;;\n    *.pptx) cp '{}' \"$out/$(basename \"$1\" .pptx).png\"; shift;;\n    *) shift;;\n  esac\ndone\n",
            pic.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let work = dir.join("work");
    let d = run(
        &DeckSpec {
            slides: vec![
                SlideSpec {
                    shapes: synthetic::title(2, "Plain"),
                    ..SlideSpec::default()
                },
                SlideSpec {
                    shapes: format!(
                        "{}{}",
                        synthetic::chart(8, "Chart", (0, 0, 6_096_000, 3_429_000)),
                        synthetic::title(2, "With a chart")
                    ),
                    media: vec![(
                        "rIdX".into(),
                        "image9.png".into(),
                        synthetic::png(4, 4, [1, 2, 3]),
                    )],
                    ..SlideSpec::default()
                },
            ],
            ..DeckSpec::default()
        },
        &Options {
            renderer: Renderer::Program(script.clone()),
            work_dir: Some(work.clone()),
            ..Options::default()
        },
    );
    assert_eq!(d.report.renderer_problem, None);
    assert_eq!(
        d.report.slides[0].picture, None,
        "slide 1 needed no picture"
    );
    assert_eq!(d.report.slides[1].picture, Some(PictureSource::Renderer));
    assert_eq!(d.report.slides[1].pictured[0].kind, "chart");
    let s = &d.presentation.sections[0].slides[1];
    // The chart's picture sits under the title (its place in the stack), cut from the left
    // quarter of the slide's picture.
    let at = |n: &str| s.elements.iter().position(|e| e.name == n).unwrap();
    assert!(at("Chart (picture)") < at("Title 2"));
    let pic = &s.elements[at("Chart (picture)")];
    let FillKind::Media { display, .. } = &pic.fill.kind else {
        panic!()
    };
    assert_eq!(display.crop.right, 960.0);
    assert_eq!(display.crop.bottom, 540.0);
    // The package LibreOffice got holds only that slide.
    let pkg = std::fs::read(work.join("in/slide-2.pptx")).unwrap();
    let z = zip::Archive::open(&pkg).unwrap();
    let pres = z.text("ppt/presentation.xml").unwrap();
    assert_eq!(pres.matches("<p:sldId ").count(), 1, "{pres}");
    assert!(pres.contains(r#"r:id="rId12""#));
    assert!(z.has("ppt/slides/slide2.xml"));
    assert!(!z.has("ppt/slides/slide1.xml"));
    assert!(z.has("ppt/media/image9.png"));
    assert!(z.has("ppt/slideMasters/slideMaster1.xml"));
    let rels = z.text("ppt/_rels/presentation.xml.rels").unwrap();
    assert!(!rels.contains("slide1.xml"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failing_renderer_doesnt_stop_the_import() {
    let d = run(
        &one(SlideSpec {
            shapes: synthetic::chart(8, "Chart", (0, 0, 6_096_000, 3_429_000)),
            ..SlideSpec::default()
        }),
        &Options {
            renderer: Renderer::Program("/nonexistent/soffice".into()),
            ..Options::default()
        },
    );
    assert!(
        d.report
            .renderer_problem
            .as_deref()
            .is_some_and(|p| p.contains("couldn't start")),
        "{:?}",
        d.report.renderer_problem
    );
    assert_eq!(d.report.slides[0].approximated.len(), 1);
}

#[cfg(unix)]
/// Rebuilds the package `bytes` with `edit(name, data)` applied to each part, plus `extra`.
fn rebuild(
    bytes: &[u8],
    edit: impl Fn(&str, Vec<u8>) -> Vec<u8>,
    extra: &[(&str, Vec<u8>)],
) -> Vec<u8> {
    let a = zip::Archive::open(bytes).unwrap();
    let mut w = zip::Writer::new();
    for e in a.entries() {
        let data = a.read_entry(e).unwrap();
        w.add(&e.name, &edit(&e.name, data)).unwrap();
    }
    for (n, d) in extra {
        w.add(n, d).unwrap();
    }
    w.finish().unwrap()
}

/// What LibreOffice gets: only the slide asked for, even after a slide-list entry whose
/// relationship is missing (indexes used to drift, drawing the slide before); no external
/// targets (web addresses, server files) apart from hyperlinks; no videos; and a part that
/// can't be read is left out instead of costing every slide its picture.
#[cfg(unix)]
#[test]
fn libreoffice_gets_the_right_slide_without_external_links_or_videos() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("pptx-import-test-pkg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pic = dir.join("pic.png");
    std::fs::write(&pic, synthetic::png(1920, 1080, [200, 30, 30])).unwrap();
    let script = dir.join("soffice");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --outdir) out=\"$2\"; shift 2;;\n    *.pptx) cp '{}' \"$out/$(basename \"$1\" .pptx).png\"; shift;;\n    *) shift;;\n  esac\ndone\n",
            pic.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let deck = synthetic::build(&DeckSpec {
        slides: vec![
            SlideSpec {
                shapes: synthetic::title(2, "Plain"),
                ..SlideSpec::default()
            },
            SlideSpec {
                shapes: format!(
                    "{}{}",
                    synthetic::chart(8, "Chart", (0, 0, 6_096_000, 3_429_000)),
                    synthetic::title(2, "With a chart")
                ),
                ..SlideSpec::default()
            },
        ],
        ..DeckSpec::default()
    });
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let deck = rebuild(
        &deck,
        |name, data| {
            let s = String::from_utf8(data.clone()).unwrap_or_default();
            match name {
                "ppt/presentation.xml" => s
                    .replacen("<p:sldId ", r#"<p:sldId id="250" r:id="rIdGone"/><p:sldId "#, 1)
                    .into_bytes(),
                "ppt/slides/_rels/slide2.xml.rels" => s
                    .replace(
                        "</Relationships>",
                        &format!(
                            r#"<Relationship Id="rIdF" Type="{rel}/image" Target="file:///etc/hostname" TargetMode="External"/><Relationship Id="rIdW" Type="{rel}/image" Target="http://192.0.2.7/x.png" TargetMode="External"/><Relationship Id="rIdH" Type="{rel}/hyperlink" Target="https://example.org/" TargetMode="External"/><Relationship Id="rIdV" Type="{rel}/video" Target="../media/clip.mp4"/><Relationship Id="rIdB" Type="{rel}/image" Target="../media/huge.png"/></Relationships>"#
                        ),
                    )
                    .into_bytes(),
                _ => data,
            }
        },
        &[
            ("ppt/media/clip.mp4", vec![5u8; 1000]),
            ("ppt/media/huge.png", vec![6u8; 1000]),
        ],
    );
    // huge.png claims to be larger than any part the import reads.
    let mut deck = deck;
    let name = b"ppt/media/huge.png";
    let at = deck.windows(name.len()).rposition(|w| w == name).unwrap() - 46;
    deck[at + 24..at + 28].copy_from_slice(&(600u32 << 20).to_le_bytes());

    let work = dir.join("work");
    let d = import(
        &deck,
        &Options {
            renderer: Renderer::Program(script.clone()),
            work_dir: Some(work.clone()),
            ..Options::default()
        },
        &mut |_, _, _| {},
    )
    .expect("imports");
    assert_eq!(d.report.renderer_problem, None);
    assert_eq!(d.report.slides[1].picture, Some(PictureSource::Renderer));
    let pkg = std::fs::read(work.join("in/slide-2.pptx")).unwrap();
    let z = zip::Archive::open(&pkg).unwrap();
    assert!(z.has("ppt/slides/slide2.xml"), "the slide asked for");
    assert!(!z.has("ppt/slides/slide1.xml"));
    let pres = z.text("ppt/presentation.xml").unwrap();
    assert_eq!(pres.matches("<p:sldId ").count(), 1, "{pres}");
    assert!(pres.contains(r#"r:id="rId12""#), "{pres}");
    assert!(!z.has("ppt/media/clip.mp4"), "no videos");
    assert!(
        !z.has("ppt/media/huge.png"),
        "an unreadable part is left out"
    );
    let rels = z.text("ppt/slides/_rels/slide2.xml.rels").unwrap();
    assert!(!rels.contains("file:///"), "{rels}");
    assert!(!rels.contains("192.0.2.7"), "{rels}");
    assert!(!rels.contains("clip.mp4"), "{rels}");
    assert!(
        rels.contains("https://example.org/"),
        "hyperlinks stay: {rels}"
    );
    assert!(rels.contains("slideLayout1.xml"), "{rels}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The deck's language: `dc:language` when the file states it, else the language most of the
/// slides' text is marked with (`a:rPr lang`, by characters); none when nothing says.
#[test]
fn the_decks_language_is_what_the_file_states_or_most_of_its_text() {
    let para = |lang: &str, text: &str| {
        format!(r#"<a:p><a:r><a:rPr lang="{lang}"/><a:t>{text}</a:t></a:r></a:p>"#)
    };
    let slide = |paras: String| SlideSpec {
        shapes: synthetic::text_box(
            2,
            "Words",
            (0, 0, 9_000_000, 3_000_000),
            "<a:bodyPr/>",
            &paras,
        ),
        ..SlideSpec::default()
    };
    let deck = DeckSpec {
        slides: vec![
            slide(para("es-ES", "Bienvenidos a la casa") + &para("en-US", "Welcome")),
            slide(para("es-ES", "Luz del alba") + &para("en-US", " ")),
        ],
        ..DeckSpec::default()
    };
    assert_eq!(
        run(&deck, &off()).presentation.language.as_deref(),
        Some("es-ES")
    );

    // What the file states wins.
    let stated = DeckSpec {
        language: Some("pt-br".into()),
        ..deck.clone()
    };
    assert_eq!(
        run(&stated, &off()).presentation.language.as_deref(),
        Some("pt-BR")
    );

    // A stated value that isn't a code is ignored; runs without a language say nothing.
    let unmarked = DeckSpec {
        language: Some("Spanish (Spain)".into()),
        slides: vec![slide(
            r#"<a:p><a:r><a:rPr/><a:t>Plain words</a:t></a:r></a:p>"#.into(),
        )],
        ..DeckSpec::default()
    };
    assert_eq!(run(&unmarked, &off()).presentation.language, None);
}
