//! Renders imported slides to PNG.
//!
//! ```text
//! render-slide --data <import-dir> --presentation <id or name substring> --slide <index> --out <png>
//!              [--background] [--background-image <file>] [--size 1920x1080] [--transparent]
//!              [--fonts <dir>]... [--software] [--no-substitutions] [--no-system-fonts] [--front-to-back]
//! render-slide --data <import-dir> --all-in <id or name substring> --out-dir <dir> [--background] ...
//! render-slide --data <import-dir> --list <name substring>
//! ```
//!
//! `<import-dir>` is the output of `pro-import` (`portable/<kind>/<id>.json`,
//! `machine/<kind>/<id>.json`). Media is resolved through the machine's media-root bindings.
//! `--background` puts the most recent background media action (at or before the slide, in
//! document order) on the media layer, as ProPresenter would show it. `--slide` is the 0-based
//! index over all slides in document order. Font fallbacks, missing media and unsupported
//! features are printed after rendering.
//!
//! `--front-to-back` reverses each slide's element list before rendering. The model stores
//! elements back to front (first = bottom); use the flag to view imports whose element lists
//! are still in ProPresenter's front-to-back order.

use anyhow::{Context, bail};
use model::{
    ActionKind, AssetLocation, FontSubstitutionTable, MediaAsset, MediaAssetId, MediaDisplay,
    MediaLayer, MediaRootBinding, MediaRootId, Presentation, Slide, Theme, ThemeId,
};
use render::{ImageSource, LayerContent, Renderer, RendererOptions, ScreenConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct StderrLogger;
impl log::Log for StderrLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info && m.target().starts_with("render")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{}] {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

#[derive(Default)]
struct Args {
    data: Option<PathBuf>,
    presentation: Option<String>,
    slide: Option<usize>,
    out: Option<PathBuf>,
    all_in: Option<String>,
    out_dir: Option<PathBuf>,
    list: Option<String>,
    background: bool,
    background_image: Option<PathBuf>,
    size: (u32, u32),
    transparent: bool,
    fonts: Vec<PathBuf>,
    software: bool,
    no_substitutions: bool,
    front_to_back: bool,
    no_system_fonts: bool,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut a = Args {
        size: (1920, 1080),
        ..Args::default()
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--data" => a.data = Some(val()?.into()),
            "--presentation" => a.presentation = Some(val()?),
            "--slide" => a.slide = Some(val()?.parse().context("--slide")?),
            "--out" => a.out = Some(val()?.into()),
            "--all-in" => a.all_in = Some(val()?),
            "--out-dir" => a.out_dir = Some(val()?.into()),
            "--list" => a.list = Some(val()?),
            "--background" => a.background = true,
            "--background-image" => a.background_image = Some(val()?.into()),
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').context("--size WxH")?;
                a.size = (w.parse()?, h.parse()?);
            }
            "--transparent" => a.transparent = true,
            "--fonts" => a.fonts.push(val()?.into()),
            "--software" => a.software = true,
            "--no-substitutions" => a.no_substitutions = true,
            "--front-to-back" => a.front_to_back = true,
            "--no-system-fonts" => a.no_system_fonts = true,
            "-h" | "--help" => {
                let usage: Vec<&str> = include_str!("render-slide.rs")
                    .lines()
                    .take_while(|l| l.starts_with("//!"))
                    .map(|l| l.trim_start_matches("//!").trim_start_matches(' '))
                    .collect();
                println!("{}", usage.join("\n"));
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(a)
}

fn read_dir_json<T: serde::de::DeserializeOwned>(dir: &Path) -> Vec<T> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            match serde_json::from_str(&text) {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("skipping {}: {e}", p.display());
                    None
                }
            }
        })
        .collect()
}

struct Data {
    root: PathBuf,
    assets: HashMap<MediaAssetId, MediaAsset>,
    bindings: HashMap<MediaRootId, PathBuf>,
    themes: HashMap<ThemeId, Theme>,
}

impl Data {
    fn load(root: &Path) -> Self {
        let assets: Vec<MediaAsset> = read_dir_json(&root.join("portable/mediaAsset"));
        let bindings: Vec<MediaRootBinding> = read_dir_json(&root.join("machine/mediaRootBinding"));
        let themes: Vec<Theme> = read_dir_json(&root.join("portable/theme"));
        Data {
            root: root.to_path_buf(),
            assets: assets.into_iter().map(|a| (a.id, a)).collect(),
            bindings: bindings
                .into_iter()
                .map(|b| (b.root, PathBuf::from(b.local_path)))
                .collect(),
            themes: themes.into_iter().map(|t| (t.id, t)).collect(),
        }
    }

    fn presentations(&self) -> Vec<Presentation> {
        read_dir_json(&self.root.join("portable/presentation"))
    }

    fn substitutions(&self) -> FontSubstitutionTable {
        read_dir_json::<FontSubstitutionTable>(&self.root.join("machine/fontSubstitutionTable"))
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// Local file of an asset, if it can be located.
    fn asset_path(&self, id: MediaAssetId) -> Option<PathBuf> {
        let a = self.assets.get(&id)?;
        let p = match &a.location {
            AssetLocation::Linked { root, path } => self.bindings.get(root)?.join(path),
            AssetLocation::ThemeAsset { theme, path } => {
                // ProPresenter keeps theme assets next to the theme document.
                let doc = self
                    .themes
                    .get(theme)?
                    .import
                    .as_ref()?
                    .document_path
                    .clone()?;
                let dir = Path::new(&doc).parent()?.to_path_buf();
                self.bindings
                    .values()
                    .map(|b| b.join(&dir).join(path))
                    .find(|p| p.exists())?
            }
            AssetLocation::Managed { sha256, extension } => self
                .root
                .join("media")
                .join(format!("{sha256}.{extension}")),
            _ => return None,
        };
        p.exists().then_some(p)
    }
}

fn find_presentation<'a>(ps: &'a [Presentation], q: &str) -> anyhow::Result<&'a Presentation> {
    if let Some(p) = ps.iter().find(|p| p.id.0.to_string() == q) {
        return Ok(p);
    }
    let ql = q.to_lowercase();
    let mut hits: Vec<&Presentation> = ps
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&ql))
        .collect();
    hits.sort_by_key(|p| (p.name.to_lowercase() != ql, p.name.len()));
    match hits.first() {
        Some(p) => {
            if hits.len() > 1 {
                eprintln!(
                    "{} presentations match {q:?}; using {:?} ({})",
                    hits.len(),
                    p.name,
                    p.id.0
                );
            }
            Ok(p)
        }
        None => bail!("no presentation matches {q:?}"),
    }
}

fn slides(p: &Presentation) -> Vec<(&str, &Slide)> {
    p.sections
        .iter()
        .flat_map(|s| s.slides.iter().map(move |sl| (s.name.as_str(), sl)))
        .collect()
}

/// The most recent background media trigger at or before slide `index`.
fn background_for(all: &[(&str, &Slide)], index: usize) -> Option<(MediaAssetId, MediaDisplay)> {
    for (_, s) in all[..=index].iter().rev() {
        for a in s.actions.iter().rev().filter(|a| a.enabled) {
            if let ActionKind::Media(t) = &a.kind
                && matches!(t.layer, MediaLayer::Background | MediaLayer::Unknown)
            {
                return Some((t.asset, t.display.clone()));
            }
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    log::set_logger(&StderrLogger).ok();
    log::set_max_level(log::LevelFilter::Info);
    let args = parse_args()?;
    let data_dir = args
        .data
        .clone()
        .context("--data <import-dir> is required")?;
    let data = Data::load(&data_dir);
    let presentations = data.presentations();

    if let Some(q) = &args.list {
        let ql = q.to_lowercase();
        for p in presentations
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&ql))
        {
            println!("{}  {:4} slides  {}", p.id.0, slides(p).len(), p.name);
        }
        return Ok(());
    }

    let mut opts = RendererOptions {
        font_dirs: args.fonts.clone(),
        system_fonts: !args.no_system_fonts,
        ..RendererOptions::default()
    };
    if args.software {
        opts.adapter = render::AdapterPreference::Software;
    }
    if !args.no_substitutions {
        opts.font_substitutions = data.substitutions();
    }
    let t0 = Instant::now();
    let mut renderer = Renderer::new(opts)?;
    eprintln!(
        "adapter: {} (init {:.0} ms)",
        renderer.adapter(),
        t0.elapsed().as_secs_f64() * 1000.0
    );
    let screen = if args.transparent {
        ScreenConfig::transparent(args.size.0, args.size.1)
    } else {
        ScreenConfig::opaque(args.size.0, args.size.1)
    };
    let resolver = |id: MediaAssetId| data.asset_path(id).map(ImageSource::Path);

    let jobs: Vec<(usize, PathBuf)>;
    let pres;
    if let Some(q) = &args.all_in {
        pres = find_presentation(&presentations, q)?;
        let dir = args.out_dir.clone().context("--all-in needs --out-dir")?;
        std::fs::create_dir_all(&dir)?;
        jobs = (0..slides(pres).len())
            .map(|i| (i, dir.join(format!("slide-{i:03}.png"))))
            .collect();
    } else {
        let q = args
            .presentation
            .as_deref()
            .context("--presentation is required")?;
        pres = find_presentation(&presentations, q)?;
        let i = args.slide.unwrap_or(0);
        jobs = vec![(i, args.out.clone().context("--out <png> is required")?)];
    }
    let all = slides(pres);
    eprintln!(
        "presentation {:?} ({}), {} slides, canvas {}x{}",
        pres.name,
        pres.id.0,
        all.len(),
        pres.canvas_size.width,
        pres.canvas_size.height
    );
    for (i, out) in jobs {
        let Some((section, slide)) = all.get(i) else {
            bail!("slide {i} out of range (0..{})", all.len());
        };
        let mut layers = Vec::new();
        if let Some(img) = &args.background_image {
            layers.push(LayerContent::media(
                ImageSource::path(img.clone()),
                MediaDisplay {
                    scale: model::ScaleMode::Fill,
                    ..MediaDisplay::default()
                },
            ));
        } else if args.background {
            match background_for(&all, i) {
                Some((asset, display)) => match data.asset_path(asset) {
                    Some(p) => layers.push(LayerContent::media(ImageSource::Path(p), display)),
                    None => eprintln!(
                        "slide {i}: background media {} is missing",
                        data.assets
                            .get(&asset)
                            .map(|a| a.name.as_str())
                            .unwrap_or("?")
                    ),
                },
                None => eprintln!("slide {i}: no background media action before this slide"),
            }
        }
        let reversed;
        let slide: &Slide = if args.front_to_back {
            let mut s = (*slide).clone();
            s.elements.reverse();
            reversed = s;
            &reversed
        } else {
            slide
        };
        layers.push(LayerContent::slide_with_media(
            slide,
            pres.canvas_size,
            &resolver,
        ));
        let t = Instant::now();
        let frame = renderer.render_layers(screen, &layers)?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        frame.save_png(&out)?;
        let text = slide.plain_text().replace('\n', " / ");
        eprintln!(
            "slide {i} [{section}] -> {} ({ms:.0} ms) {}",
            out.display(),
            text.chars().take(70).collect::<String>()
        );
    }
    let report = renderer.take_report();
    if !report.is_clean() {
        eprint!("{report}");
    }
    Ok(())
}
