//! What a file needs (pure): the facts that make a video hard to play live, the reasons to
//! convert it, and what IDN's converter is asked for.

use protocol::{HouseVideoCodec, PrepMaxSize, PrepQuality};
use std::path::Path;

/// What is known about a file, from IDN's probe (Beta14's facts: the average and base frame
/// rates, the field order, the bit rates).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// Has a picture (a video; not a sound file).
    pub video: bool,
    /// The video codec (`h264`, `hevc`, `prores`).
    pub codec: String,
    /// The stored size (before `rotation`).
    pub width: u32,
    /// The stored height.
    pub height: u32,
    /// Degrees the picture is turned to show upright (a phone's portrait video: stored
    /// landscape, 90 or 270).
    pub rotation: u16,
    /// The frame rate it plays at: the average over the stream (IDN's `frame_rate`).
    pub fps: f64,
    /// The lowest rate all its timestamps fit (FFmpeg's `r_frame_rate`), when known: a
    /// constant-rate file has it equal to the average.
    pub base_fps: Option<f64>,
    /// IDN's probe takes it for a variable frame rate (its average and base rates differ by
    /// more than 1 %).
    pub variable_rate: bool,
    /// The pixel format (`yuv420p`, `yuv422p10le`).
    pub pixel_format: String,
    /// Bits per sample.
    pub bit_depth: u8,
    /// Has an alpha channel.
    pub alpha: bool,
    /// Interlaced (a field order other than progressive), when known.
    pub interlaced: Option<bool>,
    /// The whole file's bit rate, bits a second, when known.
    pub bit_rate: Option<u64>,
    /// This box's GPU decodes the codec.
    pub gpu_decodes: bool,
    /// This box has a GPU decoder at all.
    pub gpu: bool,
    /// The first sound stream's codec, if any.
    pub audio_codec: Option<String>,
    /// Its sample rate.
    pub audio_rate: Option<u32>,
    /// Its channels.
    pub audio_channels: Option<u32>,
}

/// What a conversion does to the picture (IDN's converter always encodes it again: it has no
/// copy of the picture).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoWork {
    /// The file says it is interlaced (IDN deinterlaces what the file or the picture says is).
    pub deinterlace: bool,
    /// The constant frame rate written (numerator, denominator).
    pub rate: (u32, u32),
    /// Fitted inside this size (width, height; a portrait picture's box turned to match):
    /// it is larger than the size limit.
    pub max_size: Option<(u32, u32)>,
}

/// The conversion a file needs, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Why it is converted (empty: it isn't).
    pub reasons: Vec<String>,
    /// Why it is kept as it is although something is unusual.
    pub notes: Vec<String>,
    /// The picture is encoded again (`None`: no picture, or nothing converted). A video is
    /// converted whole: for its sound alone too.
    pub video: Option<VideoWork>,
    /// The sound needs work (encoded as AAC 48 kHz stereo; a conversion always does that).
    pub audio: bool,
}

impl Facts {
    /// The picture's size as it shows, upright (IDN's converter turns it upright before it
    /// fits it inside a size): every size decision takes this one.
    pub fn upright(&self) -> (u32, u32) {
        if self.rotation % 180 == 90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

impl Plan {
    /// Something is converted.
    pub fn converts(&self) -> bool {
        self.video.is_some() || self.audio
    }
}

/// Codecs every box's GPU decoder takes, or that decode cheaply in software.
const VIDEO_OK: &[&str] = &["h264", "hevc"];
/// Sound codecs read without trouble.
const AUDIO_OK: &[&str] = &[
    "aac",
    "mp3",
    "opus",
    "vorbis",
    "flac",
    "alac",
    "pcm_s16le",
    "pcm_s24le",
    "pcm_s32le",
    "pcm_f32le",
    "pcm_s16be",
    "pcm_s24be",
];

/// The usual frame rates (numerator, denominator) and their rates.
const RATES: &[((u32, u32), f64)] = &[
    ((24000, 1001), 23.976),
    ((24, 1), 24.0),
    ((25, 1), 25.0),
    ((30000, 1001), 29.97),
    ((30, 1), 30.0),
    ((50, 1), 50.0),
    ((60000, 1001), 59.94),
    ((60, 1), 60.0),
];

/// The constant rate a video is written at: the usual rate nearest `fps` (at most 60).
pub fn house_rate(fps: f64) -> (u32, u32) {
    let fps = if fps.is_finite() && fps > 1.0 {
        fps.min(60.0)
    } else {
        30.0
    };
    RATES
        .iter()
        .min_by(|a, b| (a.1 - fps).abs().total_cmp(&(b.1 - fps).abs()))
        .map(|r| r.0)
        .unwrap_or((30, 1))
}

/// A rate for people (`29.97`, `25`).
fn rate_text(r: (u32, u32)) -> String {
    let v = r.0 as f64 / r.1.max(1) as f64;
    let t = format!("{:.3}", v);
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Bit rate above which a video counts as huge: about 50 Mb/s for 1080p30, scaled by its
/// pixels a second (but at least 25 Mb/s).
fn huge_bit_rate(f: &Facts) -> u64 {
    let px = f.width as f64 * f.height as f64 * f.fps.clamp(1.0, 120.0);
    let per = 50_000_000.0 / (1920.0 * 1080.0 * 30.0);
    (px * per).max(25_000_000.0) as u64
}

fn mbps(b: u64) -> String {
    format!("{:.0} Mb/s", b as f64 / 1e6)
}

/// The box a `width`×`height` picture is fitted inside under `limit` (turned for a portrait
/// picture), when it is larger.
pub fn fit_inside(width: u32, height: u32, limit: PrepMaxSize) -> Option<(u32, u32)> {
    let (lw, lh) = limit.bounds();
    let (bw, bh) = if height > width { (lh, lw) } else { (lw, lh) };
    (width > bw || height > bh).then_some((bw, bh))
}

/// The size of the converted picture: `f` upright, fitted inside `work`'s box with its shape
/// kept (never made larger), as IDN's converter makes it.
pub fn output_size(f: &Facts, work: &VideoWork) -> (u32, u32) {
    let (w, h) = f.upright();
    let Some((bw, bh)) = work.max_size else {
        return (w, h);
    };
    if w == 0 || h == 0 || (w <= bw && h <= bh) {
        return (w, h);
    }
    let scale = (bw as f64 / w as f64).min(bh as f64 / h as f64);
    (
        ((w as f64 * scale).round() as u32).clamp(1, bw),
        ((h as f64 * scale).round() as u32).clamp(1, bh),
    )
}

/// What `f` needs with the default size limit (its own size, at most 4K).
pub fn plan(f: &Facts) -> Plan {
    plan_with(f, PrepMaxSize::Keep)
}

/// What `f` needs: why to convert it, and how (`limit`: the setting `maxSize`).
pub fn plan_with(f: &Facts, limit: PrepMaxSize) -> Plan {
    let mut p = Plan::default();
    let mut video = Vec::new();
    let mut audio = Vec::new();
    if f.video {
        if f.alpha {
            p.notes
                .push("Kept as it is: it has an alpha channel (the house format has none)".into());
        } else {
            if !VIDEO_OK.contains(&f.codec.as_str()) {
                video.push(format!(
                    "{} video isn't decoded by the GPU",
                    upper(&f.codec)
                ));
            } else if f.gpu && !f.gpu_decodes {
                video.push(format!(
                    "this box's GPU doesn't decode {} video",
                    upper(&f.codec)
                ));
            }
            let chroma_422_444 = ["422", "444"].iter().any(|c| f.pixel_format.contains(c));
            if chroma_422_444 && video.is_empty() {
                video.push(format!(
                    "{} colour ({}) isn't decoded by the GPU",
                    if f.pixel_format.contains("444") {
                        "4:4:4"
                    } else {
                        "4:2:2"
                    },
                    f.pixel_format
                ));
            }
            if f.bit_depth > 8 && (f.codec == "h264" || f.bit_depth > 10) && video.is_empty() {
                video.push(format!(
                    "{}-bit {} isn't decoded by the GPU",
                    f.bit_depth,
                    upper(&f.codec)
                ));
            }
            if f.interlaced == Some(true) {
                video.push("interlaced".into());
            }
            let differs = f.base_fps.is_some_and(|base| {
                base > 0.0 && f.fps > 0.0 && (base - f.fps).abs() / f.fps > 0.005
            });
            if f.variable_rate || differs {
                video.push(format!(
                    "variable frame rate (about {:.2} fps on average)",
                    f.fps
                ));
            }
            if f.fps > 61.0 {
                video.push(format!("{:.0} fps (the house format is at most 60)", f.fps));
            }
            let (w, h) = f.upright();
            if fit_inside(w, h, PrepMaxSize::Keep).is_some() {
                video.push(format!("{w}×{h} (larger than 4K)"));
            } else if fit_inside(w, h, limit).is_some() {
                video.push(format!("{w}×{h} (larger than the 1080p limit)"));
            }
            if let Some(b) = f.bit_rate
                && b > huge_bit_rate(f)
            {
                video.push(format!("a very high bit rate ({})", mbps(b)));
            }
        }
        if let Some(c) = &f.audio_codec {
            if !AUDIO_OK.contains(&c.as_str()) {
                audio.push(format!("{} sound", upper(c)));
            } else if let Some(r) = f.audio_rate
                && r != 48_000
            {
                audio.push(format!(
                    "sound at {} kHz (the house rate is 48 kHz)",
                    trim_khz(r)
                ));
            }
        }
    } else if let Some(c) = &f.audio_codec
        && !AUDIO_OK.contains(&c.as_str())
    {
        audio.push(format!("{} sound", upper(c)));
    }
    if f.video && f.alpha && !audio.is_empty() {
        // The picture can't be re-encoded (alpha) and copying it into the house MP4 fails
        // (ProRes, QuickTime RLE, PNG) or loses its alpha (VP8/VP9): the whole file stays.
        p.notes.push(format!(
            "Its sound is kept as it is too ({})",
            audio.join(", ")
        ));
        audio.clear();
    }
    p.audio = !audio.is_empty();
    if f.video && (!video.is_empty() || p.audio) {
        let (w, h) = f.upright();
        p.video = Some(VideoWork {
            deinterlace: f.interlaced == Some(true),
            rate: house_rate(f.fps),
            max_size: fit_inside(w, h, limit),
        });
    }
    p.reasons = video.into_iter().chain(audio).collect();
    p
}

fn trim_khz(r: u32) -> String {
    let k = r as f64 / 1000.0;
    if (k - k.round()).abs() < 1e-9 {
        format!("{k:.0}")
    } else {
        format!("{k}")
    }
}

fn upper(codec: &str) -> String {
    match codec {
        "h264" => "H.264".into(),
        "hevc" => "HEVC".into(),
        "mpeg2video" => "MPEG-2".into(),
        "mpeg4" => "MPEG-4 Part 2".into(),
        "prores" => "ProRes".into(),
        "wmv3" | "wmv2" | "wmv1" => "Windows Media".into(),
        "wmav2" | "wmav1" | "wmapro" => "Windows Media Audio".into(),
        "ac3" => "AC-3".into(),
        "eac3" => "E-AC-3".into(),
        c => c.to_uppercase(),
    }
}

/// IDN's converter's own bit rate for 1080p30 H.264 (its `video_kbps` default: about 0.12 bit
/// a pixel), kbit/s.
const IDN_1080P30_KBPS: f64 = 7_500.0;

/// A quality's bit rate over IDN's own choice by size and rate.
pub fn quality_factor(q: PrepQuality) -> f64 {
    match q {
        PrepQuality::Standard => 1.6,
        PrepQuality::High => 8.0 / 3.0,
        PrepQuality::Maximum => 4.0,
    }
}

/// The video bit rate asked of the converter, kbit/s: IDN's choice by size and rate (1080p30
/// H.264 7.5 Mb/s, scaled by the pixels a second) times the quality's factor (standard: 12
/// Mb/s for 1080p30), 2 to 60 Mb/s; HEVC 0.6 of that. `width`×`height`: the converted
/// picture's size ([`output_size`]), not the original's. `explicit`: the setting `videoKbps`
/// (kept within 1 to 60 Mb/s) instead.
pub fn video_kbps(
    width: u32,
    height: u32,
    rate: (u32, u32),
    house: HouseVideoCodec,
    quality: PrepQuality,
    explicit: Option<u32>,
) -> u32 {
    if let Some(k) = explicit {
        return k.clamp(1_000, 60_000);
    }
    let fps = rate.0 as f64 / rate.1.max(1) as f64;
    let px = width as f64 * height as f64 * fps;
    let b = ((px / (1920.0 * 1080.0 * 30.0)) * IDN_1080P30_KBPS * quality_factor(quality))
        .round()
        .clamp(2_000.0, 60_000.0);
    match house {
        HouseVideoCodec::H264 => b as u32,
        HouseVideoCodec::Hevc => (b * 0.6).round().max(2_000.0) as u32,
    }
}

/// The file name of the converted copy (`Sunrise.mov` → `Sunrise (prepared).mp4`).
pub fn prepared_name(original: &str, video: bool) -> String {
    let stem = Path::new(original)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("media");
    format!("{stem} (prepared).{}", if video { "mp4" } else { "m4a" })
}

/// The copy's format, for the asset ("H.264 1920×1080 29.97 fps, AAC 48 kHz"): what the
/// converter made (`size` and `rate` of its picture, `audio`: it has sound).
pub fn format_line(
    house: HouseVideoCodec,
    picture: Option<((u32, u32), (u32, u32))>,
    audio: bool,
) -> String {
    let mut parts = Vec::new();
    if let Some(((w, h), rate)) = picture.filter(|((w, h), _)| *w > 0 && *h > 0) {
        let c = match house {
            HouseVideoCodec::H264 => "H.264",
            HouseVideoCodec::Hevc => "HEVC",
        };
        parts.push(format!("{c} {w}×{h} {} fps", rate_text(rate)));
    }
    if audio {
        parts.push("AAC 48 kHz".into());
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Facts {
        Facts {
            video: true,
            codec: "h264".into(),
            width: 1920,
            height: 1080,
            rotation: 0,
            fps: 29.97,
            base_fps: Some(29.97),
            variable_rate: false,
            pixel_format: "yuv420p".into(),
            bit_depth: 8,
            alpha: false,
            interlaced: Some(false),
            bit_rate: Some(12_000_000),
            gpu_decodes: true,
            gpu: true,
            audio_codec: Some("aac".into()),
            audio_rate: Some(48_000),
            audio_channels: Some(2),
        }
    }

    #[test]
    fn a_house_format_file_is_left_alone() {
        let p = plan(&good());
        assert!(!p.converts(), "{p:?}");
        assert!(p.reasons.is_empty() && p.notes.is_empty());
        // HEVC 10-bit decodes on the GPU: fine too.
        let f = Facts {
            codec: "hevc".into(),
            pixel_format: "yuv420p10le".into(),
            bit_depth: 10,
            ..good()
        };
        assert!(!plan(&f).converts());
        // A base rate IDN doesn't know.
        let f = Facts {
            base_fps: None,
            ..good()
        };
        assert!(!plan(&f).converts());
    }

    #[test]
    fn hard_to_play_videos_are_converted_with_their_reasons() {
        let cases: Vec<(Facts, &str)> = vec![
            (
                Facts {
                    codec: "prores".into(),
                    pixel_format: "yuv422p10le".into(),
                    bit_depth: 10,
                    gpu_decodes: false,
                    ..good()
                },
                "ProRes video isn't decoded by the GPU",
            ),
            (
                Facts {
                    pixel_format: "yuv422p".into(),
                    ..good()
                },
                "4:2:2 colour (yuv422p) isn't decoded by the GPU",
            ),
            (
                Facts {
                    pixel_format: "yuv420p10le".into(),
                    bit_depth: 10,
                    ..good()
                },
                "10-bit H.264 isn't decoded by the GPU",
            ),
            (
                Facts {
                    interlaced: Some(true),
                    fps: 25.0,
                    base_fps: Some(25.0),
                    ..good()
                },
                "interlaced",
            ),
            (
                Facts {
                    fps: 28.71,
                    base_fps: Some(30.0),
                    ..good()
                },
                "variable frame rate (about 28.71 fps on average)",
            ),
            (
                // IDN's own guess, its rates within 0.5 %.
                Facts {
                    fps: 29.9,
                    base_fps: Some(29.97),
                    variable_rate: true,
                    ..good()
                },
                "variable frame rate (about 29.90 fps on average)",
            ),
            (
                // A phone's: its timestamps on a fine clock.
                Facts {
                    fps: 29.5,
                    base_fps: Some(90_000.0),
                    ..good()
                },
                "variable frame rate (about 29.50 fps on average)",
            ),
            (
                Facts {
                    fps: 119.88,
                    base_fps: Some(119.88),
                    ..good()
                },
                "120 fps (the house format is at most 60)",
            ),
            (
                Facts {
                    bit_rate: Some(150_000_000),
                    ..good()
                },
                "a very high bit rate (150 Mb/s)",
            ),
            (
                Facts {
                    width: 7680,
                    height: 4320,
                    bit_rate: None,
                    ..good()
                },
                "7680×4320 (larger than 4K)",
            ),
        ];
        for (f, why) in cases {
            let p = plan(&f);
            assert!(p.video.is_some(), "{why}: {p:?}");
            assert!(!p.audio, "{why}");
            assert!(p.reasons.iter().any(|r| r == why), "{why}: {:?}", p.reasons);
        }
        // Deinterlaced at its own rate; the VFR clip at the usual rate nearest its average.
        let p = plan(&Facts {
            interlaced: Some(true),
            fps: 25.0,
            base_fps: Some(25.0),
            ..good()
        });
        assert_eq!(
            p.video,
            Some(VideoWork {
                deinterlace: true,
                rate: (25, 1),
                max_size: None
            })
        );
        let p = plan(&Facts {
            fps: 29.6,
            base_fps: Some(30.0),
            ..good()
        });
        assert_eq!(p.video.unwrap().rate, (30000, 1001));
        let p = plan(&Facts {
            width: 7680,
            height: 4320,
            ..good()
        });
        assert_eq!(p.video.unwrap().max_size, Some((3840, 2160)));
    }

    #[test]
    fn the_size_limit_is_a_reason_and_fits_portrait_pictures_the_other_way() {
        let uhd = Facts {
            width: 3840,
            height: 2160,
            ..good()
        };
        assert!(!plan(&uhd).converts());
        assert!(!plan_with(&uhd, PrepMaxSize::P2160).converts());
        let p = plan_with(&uhd, PrepMaxSize::P1080);
        assert_eq!(p.reasons, vec!["3840×2160 (larger than the 1080p limit)"]);
        assert_eq!(p.video.unwrap().max_size, Some((1920, 1080)));
        let portrait = Facts {
            width: 2160,
            height: 3840,
            ..good()
        };
        let p = plan_with(&portrait, PrepMaxSize::P1080);
        assert_eq!(p.video.unwrap().max_size, Some((1080, 1920)));
        // Converted for another reason: fitted to the limit too.
        let p = plan_with(
            &Facts {
                interlaced: Some(true),
                ..uhd.clone()
            },
            PrepMaxSize::P1080,
        );
        assert_eq!(p.video.unwrap().max_size, Some((1920, 1080)));
        // 1080p itself (either way round) is within it.
        assert_eq!(fit_inside(1920, 1080, PrepMaxSize::P1080), None);
        assert_eq!(fit_inside(1080, 1920, PrepMaxSize::P1080), None);
    }

    #[test]
    fn a_turned_picture_is_sized_upright() {
        // A phone's portrait video: stored landscape, turned 90° (or 270°) to show. IDN fits
        // the upright picture, so its box is portrait.
        for rotation in [90, 270] {
            let phone = Facts {
                width: 3840,
                height: 2160,
                rotation,
                ..good()
            };
            assert_eq!(phone.upright(), (2160, 3840));
            let p = plan_with(&phone, PrepMaxSize::P1080);
            assert_eq!(p.reasons, vec!["2160×3840 (larger than the 1080p limit)"]);
            assert_eq!(p.video.unwrap().max_size, Some((1080, 1920)), "{rotation}");
            // Turned upside down: as stored.
            let upside = Facts {
                rotation: 180,
                ..phone.clone()
            };
            assert_eq!(
                plan_with(&upside, PrepMaxSize::P1080)
                    .video
                    .unwrap()
                    .max_size,
                Some((1920, 1080))
            );
        }
        // Wider than 4K: fitted into a portrait 4K box (IDN keeps its shape: about 2025×3840).
        let tall = Facts {
            width: 4096,
            height: 2160,
            rotation: 90,
            ..good()
        };
        let p = plan(&tall);
        assert_eq!(p.reasons, vec!["2160×4096 (larger than 4K)"]);
        assert_eq!(p.video.unwrap().max_size, Some((2160, 3840)));
        // A 1080p portrait phone video is within the 1080p limit.
        let hd = Facts {
            width: 1920,
            height: 1080,
            rotation: 90,
            ..good()
        };
        assert!(!plan_with(&hd, PrepMaxSize::P1080).converts());
    }

    #[test]
    fn quality_presets_scale_idns_bit_rate() {
        use HouseVideoCodec::{H264, Hevc};
        use PrepQuality::{High, Maximum, Standard};
        let k = |w, h, r, c, q| video_kbps(w, h, r, c, q, None);
        // 1080p30 H.264: IDN's own 7.5 Mb/s × 1.6, × 2.67, × 4.
        assert_eq!(k(1920, 1080, (30, 1), H264, Standard), 12_000);
        assert_eq!(k(1920, 1080, (30, 1), H264, High), 20_000);
        assert_eq!(k(1920, 1080, (30, 1), H264, Maximum), 30_000);
        assert_eq!(k(1920, 1080, (30, 1), Hevc, High), 12_000);
        // Scaled by the pixels a second, within 2–60 Mb/s.
        assert_eq!(k(1280, 720, (60, 1), H264, Standard), 10_667);
        assert_eq!(k(3840, 2160, (60, 1), H264, Maximum), 60_000);
        assert_eq!(k(320, 180, (25, 1), H264, Maximum), 2_000);
        // A portrait picture counts all its pixels.
        assert_eq!(k(1080, 1920, (30, 1), H264, Standard), 12_000);
        assert_eq!(
            k(2160, 3840, (30, 1), H264, Standard),
            k(3840, 2160, (30, 1), H264, Standard)
        );
        // An explicit rate wins, within 1–60 Mb/s.
        assert_eq!(
            video_kbps(1920, 1080, (30, 1), H264, Maximum, Some(8_000)),
            8_000
        );
        assert_eq!(
            video_kbps(1920, 1080, (30, 1), Hevc, Standard, Some(500)),
            1_000
        );
        assert_eq!(
            video_kbps(1920, 1080, (30, 1), H264, Standard, Some(99_000)),
            60_000
        );
    }

    #[test]
    fn the_bit_rate_follows_the_converted_size() {
        let size = |f: &Facts, limit| output_size(f, &plan_with(f, limit).video.unwrap());
        let kbps = |f: &Facts, limit| {
            let v = plan_with(f, limit).video.unwrap();
            let (w, h) = output_size(f, &v);
            video_kbps(
                w,
                h,
                v.rate,
                HouseVideoCodec::H264,
                PrepQuality::Standard,
                None,
            )
        };
        // 4K at 29.97 with the 1080p limit: a 1080p copy, at 1080p's rate (not 4K's).
        let uhd = Facts {
            width: 3840,
            height: 2160,
            ..good()
        };
        assert_eq!(size(&uhd, PrepMaxSize::P1080), (1920, 1080));
        assert_eq!(kbps(&uhd, PrepMaxSize::P1080), 11_988);
        // Kept at its size: 4K's rate.
        let uhd_interlaced = Facts {
            interlaced: Some(true),
            ..uhd.clone()
        };
        assert_eq!(size(&uhd_interlaced, PrepMaxSize::Keep), (3840, 2160));
        assert_eq!(kbps(&uhd_interlaced, PrepMaxSize::Keep), 47_952);
        // A phone's portrait 4K (stored landscape, turned): an upright 1080×1920 copy.
        let phone = Facts {
            rotation: 90,
            ..uhd.clone()
        };
        assert_eq!(size(&phone, PrepMaxSize::P1080), (1080, 1920));
        assert_eq!(kbps(&phone, PrepMaxSize::P1080), 11_988);
        // Larger than 4K: its shape kept inside 4K.
        let wide = Facts {
            width: 4096,
            height: 2160,
            ..good()
        };
        assert_eq!(size(&wide, PrepMaxSize::Keep), (3840, 2025));
        assert_eq!(
            size(
                &Facts {
                    rotation: 270,
                    ..wide
                },
                PrepMaxSize::Keep
            ),
            (2025, 3840)
        );
        // Smaller than the box: its own size.
        let small = Facts {
            width: 640,
            height: 360,
            interlaced: Some(true),
            ..good()
        };
        assert_eq!(size(&small, PrepMaxSize::P1080), (640, 360));
    }

    #[test]
    fn sound_problems_convert_the_whole_video() {
        // IDN's converter encodes the picture again too (it has no copy of the picture).
        let p = plan(&Facts {
            audio_rate: Some(44_100),
            ..good()
        });
        assert!(p.audio && p.converts());
        assert_eq!(p.video.as_ref().map(|v| v.rate), Some((30000, 1001)));
        assert_eq!(
            p.reasons,
            vec!["sound at 44.1 kHz (the house rate is 48 kHz)".to_string()]
        );
        let p = plan(&Facts {
            video: false,
            audio_codec: Some("wmav2".into()),
            ..Facts::default()
        });
        assert!(p.audio && p.video.is_none());
        assert_eq!(p.reasons, vec!["Windows Media Audio sound".to_string()]);
        // A sound file at 44.1 kHz is fine (read at 48 kHz as it plays).
        let p = plan(&Facts {
            video: false,
            audio_codec: Some("mp3".into()),
            audio_rate: Some(44_100),
            ..Facts::default()
        });
        assert!(!p.converts());
    }

    #[test]
    fn alpha_is_kept_with_a_note() {
        let p = plan(&Facts {
            codec: "prores".into(),
            alpha: true,
            ..good()
        });
        assert!(p.video.is_none());
        assert_eq!(p.notes.len(), 1);
        // Its sound needing work doesn't make it a conversion (the house format has no alpha).
        for (codec, rate) in [("pcm_s16le", 44_100), ("vorbis", 44_100), ("aac", 44_100)] {
            let p = plan(&Facts {
                codec: "prores".into(),
                alpha: true,
                audio_codec: Some(codec.into()),
                audio_rate: Some(rate),
                ..good()
            });
            assert!(!p.converts(), "{codec}: {p:?}");
            assert_eq!(p.notes.len(), 2, "{p:?}");
        }
    }

    #[test]
    fn rates_bit_rates_names_and_formats() {
        assert_eq!(house_rate(29.97), (30000, 1001));
        assert_eq!(house_rate(23.98), (24000, 1001));
        assert_eq!(house_rate(120.0), (60, 1));
        assert_eq!(house_rate(f64::NAN), (30, 1));
        assert_eq!(rate_text((30000, 1001)), "29.97");
        assert_eq!(rate_text((24000, 1001)), "23.976");
        assert_eq!(rate_text((25, 1)), "25");
        assert_eq!(
            video_kbps(
                1920,
                1080,
                (30, 1),
                HouseVideoCodec::H264,
                PrepQuality::Standard,
                None
            ),
            12_000
        );
        assert_eq!(
            video_kbps(
                1920,
                1080,
                (30, 1),
                HouseVideoCodec::Hevc,
                PrepQuality::Standard,
                None
            ),
            7_200
        );
        assert_eq!(
            video_kbps(
                320,
                180,
                (25, 1),
                HouseVideoCodec::H264,
                PrepQuality::Standard,
                None
            ),
            2_000
        );
        assert_eq!(
            video_kbps(
                7680,
                4320,
                (60, 1),
                HouseVideoCodec::H264,
                PrepQuality::Standard,
                None
            ),
            60_000
        );
        assert_eq!(prepared_name("Sunrise.mov", true), "Sunrise (prepared).mp4");
        assert_eq!(prepared_name("Bells.wma", false), "Bells (prepared).m4a");
        assert_eq!(
            format_line(HouseVideoCodec::H264, Some(((1920, 1080), (25, 1))), true),
            "H.264 1920×1080 25 fps, AAC 48 kHz"
        );
        assert_eq!(
            format_line(
                HouseVideoCodec::Hevc,
                Some(((1280, 720), (30000, 1001))),
                false
            ),
            "HEVC 1280×720 29.97 fps"
        );
        assert_eq!(format_line(HouseVideoCodec::H264, None, true), "AAC 48 kHz");
    }
}
