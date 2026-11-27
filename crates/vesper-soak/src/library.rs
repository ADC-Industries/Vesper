//! The synthetic library the soak runs on, written as a data folder in the layout `pro-import`
//! writes (`portable/`, `machine/`, `media/`), which the server moves into its Codex on first
//! start. Everything is made up here: songs with lorem-ipsum lyrics, generated images and
//! videos (system FFmpeg). Never real church data.

use anyhow::{Context, Result, bail};
use model::{
    AssetLocation, CharStyle, Color, Element, FrameRate, Link, MediaAsset, MediaBin, MediaBinItem,
    MediaBinItemId, MediaBinKind, MediaKind, MediaLayer, MediaTrigger, OutputBinding, OutputId,
    OutputTransport, Paragraph, Playback, PlaylistItem, PlaylistItemKind, PlaylistNode,
    PlaylistNodeKind, PlaylistPurpose, Presentation, Rect, Repeat, Screen, ScreenKind, Section,
    Size, Slide, SlideAction, TextBox,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Seconds between beeps (and flashes) in the A/V sync clip.
pub const BEEP_EVERY: f64 = 2.0;
/// Length of the A/V sync clip, s.
pub const AV_SECS: u32 = 60;
/// How long each flash (and beep) lasts, s.
pub const FLASH_SECS: f64 = 0.1;

/// What the soak plays, by ID.
pub struct Library {
    /// Audience screens (each with an IDN output; the first embeds the program audio).
    pub screens: Vec<Screen>,
    /// The IDN source name of each audience screen's output.
    pub outputs: Vec<String>,
    /// The songs.
    pub songs: Vec<Presentation>,
    /// The service playlist: announcements, songs, scripture, sermon, last song.
    pub playlist: PlaylistNode,
    /// Background images.
    pub images: Vec<MediaAsset>,
    /// A looping background video (no sound).
    pub loop_video: MediaAsset,
    /// The A/V sync clip: white flashes with beeps at the same media times.
    pub av_video: MediaAsset,
}

impl Library {
    /// The playlist's items.
    pub fn items(&self) -> Vec<PlaylistItem> {
        match &self.playlist.kind {
            PlaylistNodeKind::Playlist { items, .. } => items.clone(),
            _ => Vec::new(),
        }
    }
}

const WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "enim",
    "ad",
    "minim",
    "veniam",
    "quis",
    "nostrud",
    "exercitation",
    "ullamco",
    "laboris",
    "nisi",
    "aliquip",
    "ex",
    "ea",
    "commodo",
    "consequat",
];

/// A small deterministic random source (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// A lyric line of 4–7 lorem-ipsum words, capitalised.
pub fn line(rng: &mut Rng) -> String {
    let n = 4 + rng.below(4);
    let mut s: Vec<String> = (0..n)
        .map(|_| WORDS[rng.below(WORDS.len())].to_string())
        .collect();
    if let Some(c) = s[0].get(..1) {
        s[0] = c.to_uppercase() + &s[0][1..];
    }
    s.join(" ")
}

fn text_slide(text: &str, canvas: Size, size: f64) -> Slide {
    let el = Element::text(
        Rect::new(
            canvas.width * 0.05,
            canvas.height * 0.2,
            canvas.width * 0.9,
            canvas.height * 0.6,
        ),
        TextBox {
            paragraphs: Paragraph::from_lines(text),
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                font_weight: Some(700),
                size: Some(size),
                color: Some(Color::WHITE),
                ..CharStyle::default()
            },
            ..TextBox::default()
        },
    );
    Slide {
        elements: vec![el],
        ..Slide::new()
    }
}

fn write_doc<T: serde::Serialize>(
    dir: &Path,
    section: &str,
    kind: &str,
    id: &str,
    doc: &T,
) -> Result<()> {
    let d = dir.join(section).join(kind);
    std::fs::create_dir_all(&d)?;
    std::fs::write(
        d.join(format!("{id}.json")),
        serde_json::to_vec_pretty(doc)?,
    )?;
    Ok(())
}

fn ffmpeg(args: &[&str]) -> Result<()> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .context("run the system ffmpeg")?;
    if !out.status.success() {
        bail!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// Moves a generated file into `media/<sha256>.<ext>` and writes its asset document.
fn add_media(dir: &Path, tmp: &Path, name: &str, ext: &str, kind: MediaKind) -> Result<MediaAsset> {
    let bytes = std::fs::read(tmp)?;
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::rename(tmp, dir.join("media").join(format!("{sha}.{ext}")))?;
    let asset = MediaAsset {
        name: name.into(),
        kind,
        location: AssetLocation::Managed {
            sha256: sha,
            extension: ext.into(),
        },
        ..MediaAsset::default()
    };
    write_doc(
        dir,
        "portable",
        "mediaAsset",
        &asset.id.0.to_string(),
        &asset,
    )?;
    Ok(asset)
}

fn presentation(
    name: &str,
    library: &Library_,
    sections: Vec<Section>,
    canvas: Size,
) -> Presentation {
    Presentation {
        library: Some(library.id),
        canvas_size: canvas,
        sections,
        ..Presentation::new(name)
    }
}

type Library_ = model::Library;

/// Writes the synthetic data folder into `dir` (which must not exist yet).
pub fn build(dir: &Path, outputs: u32, (w, h): (u32, u32), tag: &str) -> Result<Library> {
    if dir.exists() {
        bail!("{} exists already", dir.display());
    }
    std::fs::create_dir_all(dir.join("media"))?;
    let tmp: PathBuf = dir.join("gen");
    std::fs::create_dir_all(&tmp)?;
    let canvas = Size::new(w as f64, h as f64);
    let mut rng = Rng::new(0x5eed_50a4);

    // Screens: N audience screens, each with an IDN output, and a stage screen.
    let mut screens = Vec::new();
    let mut names = Vec::new();
    for i in 0..outputs.max(1) {
        let s = Screen {
            name: if i == 0 {
                "Audience".into()
            } else {
                format!("Audience {}", i + 1)
            },
            sort_key: format!("a{i}"),
            size: canvas,
            frame_rate: FrameRate::FPS30,
            ..Screen::default()
        };
        write_doc(dir, "portable", "screen", &s.id.0.to_string(), &s)?;
        let source = format!("Soak {tag} {}", i + 1);
        let out = OutputBinding {
            id: OutputId::new(),
            name: format!("Soak {}", i + 1),
            screen: s.id,
            transport: OutputTransport::Idn {
                source_name: source.clone(),
                groups: Vec::new(),
            },
            embed_audio: i == 0,
            ..OutputBinding::default()
        };
        write_doc(dir, "machine", "outputBinding", &out.id.0.to_string(), &out)?;
        names.push(source);
        screens.push(s);
    }
    let stage = Screen {
        name: "Stage".into(),
        sort_key: "z".into(),
        kind: ScreenKind::Stage { layout: None },
        size: canvas,
        frame_rate: FrameRate::FPS30,
        ..Screen::default()
    };
    write_doc(dir, "portable", "screen", &stage.id.0.to_string(), &stage)?;

    // Dissolves, as a church would run them (transitions render every tick).
    let settings = model::ShowSettings {
        default_transitions: model::LayerTransitions {
            media: Some(model::Transition::dissolve(1.0)),
            slide: Some(model::Transition::dissolve(0.5)),
            announcements: Some(model::Transition::dissolve(0.5)),
            ..Default::default()
        },
        ..Default::default()
    };
    write_doc(
        dir,
        "portable",
        "showSettings",
        "00000000-0000-0000-0000-000000000000",
        &settings,
    )?;

    // Media: three gradient images, a looping background video, the A/V sync clip.
    let mut images = Vec::new();
    for (i, (a, b)) in [
        ([20u8, 40, 120], [90u8, 20, 110]),
        ([10, 80, 60], [10, 30, 90]),
        ([110, 60, 20], [40, 10, 40]),
    ]
    .into_iter()
    .enumerate()
    {
        let img = image::RgbImage::from_fn(1920, 1080, |x, y| {
            let t = (x + y) as f32 / (1920.0 + 1080.0);
            let c = |k: usize| (a[k] as f32 * (1.0 - t) + b[k] as f32 * t) as u8;
            image::Rgb([c(0), c(1), c(2)])
        });
        let p = tmp.join(format!("bg{i}.png"));
        img.save(&p)?;
        images.push(add_media(
            dir,
            &p,
            &format!("Background {}", i + 1),
            "png",
            MediaKind::Image,
        )?);
    }
    let p = tmp.join("loop.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=s={w}x{h}:r=30:d=20"),
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "30",
        p.to_str().context("path")?,
    ])?;
    let loop_video = add_media(dir, &p, "Motion Loop", "mp4", MediaKind::Video)?;
    let p = tmp.join("av.mp4");
    let on = format!("lt(mod(t,{BEEP_EVERY}),{FLASH_SECS})");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!(
            "color=c=black:s={w}x{h}:r=30:d={AV_SECS},drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='{on}'"
        ),
        "-f",
        "lavfi",
        "-i",
        &format!(
            "aevalsrc='0.5*sin(2*PI*1000*t)*{on}|0.5*sin(2*PI*1000*t)*{on}':s=48000:d={AV_SECS}"
        ),
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "30",
        "-c:a",
        "aac",
        "-b:a",
        "160k",
        p.to_str().context("path")?,
    ])?;
    let av_video = add_media(dir, &p, "Sync Clip", "mp4", MediaKind::Video)?;
    let _ = std::fs::remove_dir_all(&tmp);

    // A bin for the backgrounds: the loop loops, the sync clip plays once.
    let item = |a: &MediaAsset, repeat: Repeat| MediaBinItem {
        id: MediaBinItemId::new(),
        name: None,
        enabled: true,
        trigger: MediaTrigger {
            playback: Playback {
                repeat,
                ..Playback::default()
            },
            ..MediaTrigger::new(a.id, MediaLayer::Background)
        },
    };
    let mut bin_items = vec![
        item(&loop_video, Repeat::Loop),
        item(&av_video, Repeat::Once),
    ];
    bin_items.extend(images.iter().map(|a| item(a, Repeat::Once)));
    let bin = MediaBin {
        name: "Soak Backgrounds".into(),
        kind: MediaBinKind::Manual { items: bin_items },
        ..MediaBin::default()
    };
    write_doc(dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin)?;

    // Content.
    let library = model::Library {
        name: "Songs".into(),
        sort_key: "a".into(),
        ..Library_::default()
    };
    write_doc(
        dir,
        "portable",
        "library",
        &library.id.0.to_string(),
        &library,
    )?;
    let mut songs = Vec::new();
    for n in 0..6 {
        let mut sections = Vec::new();
        for (k, title) in ["Verse 1", "Chorus", "Verse 2", "Chorus", "Bridge"]
            .iter()
            .enumerate()
        {
            let mut s = Section::new(*title);
            let count = if k == 4 { 1 } else { 2 };
            for _ in 0..count {
                let text = format!("{}\n{}", line(&mut rng), line(&mut rng));
                s.slides.push(text_slide(&text, canvas, h as f64 / 12.0));
            }
            sections.push(s);
        }
        // A background image on the first slide of every other song.
        if n % 2 == 0
            && let Some(first) = sections.first_mut().and_then(|s| s.slides.first_mut())
        {
            first.actions.push(SlideAction::media(MediaTrigger::new(
                images[n % images.len()].id,
                MediaLayer::Background,
            )));
        }
        songs.push(presentation(
            &format!("Soak Song {}", n + 1),
            &library,
            sections,
            canvas,
        ));
    }
    let simple = |name: &str, count: usize, size: f64, rng: &mut Rng| {
        let mut s = Section::new("Slides");
        for i in 0..count {
            s.slides.push(text_slide(
                &format!("{name} {}\n{}", i + 1, line(rng)),
                canvas,
                size,
            ));
        }
        presentation(name, &library, vec![s], canvas)
    };
    let announcements = simple("Announcements", 6, h as f64 / 14.0, &mut rng);
    let scripture = simple("Reading", 5, h as f64 / 16.0, &mut rng);
    let sermon = simple("Sermon Point", 5, h as f64 / 12.0, &mut rng);
    for p in songs.iter().chain([&announcements, &scripture, &sermon]) {
        write_doc(dir, "portable", "presentation", &p.id.0.to_string(), p)?;
    }

    let entry = |p: &Presentation| {
        PlaylistItem::new(PlaylistItemKind::Presentation {
            presentation: Link::named(p.id, p.name.clone()),
            arrangement: None,
            unresolved: None,
        })
    };
    let mut items = vec![entry(&announcements)];
    items.extend(songs.iter().take(3).map(entry));
    items.push(entry(&scripture));
    items.push(entry(&sermon));
    items.push(entry(&songs[3]));
    let playlist = PlaylistNode {
        name: "Soak Sunday".into(),
        kind: PlaylistNodeKind::Playlist {
            purpose: PlaylistPurpose::Service,
            items,
        },
        ..PlaylistNode::default()
    };
    write_doc(
        dir,
        "portable",
        "playlistNode",
        &playlist.id.0.to_string(),
        &playlist,
    )?;

    Ok(Library {
        screens,
        outputs: names,
        songs,
        playlist,
        images,
        loop_video,
        av_video,
    })
}
