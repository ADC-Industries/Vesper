//! Shared helpers for the server integration tests: a synthetic data directory built from
//! `model` types, a tiny HTTP/1.1 client and a WebSocket client.

#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use model::{
    AlphaMode, AssetLocation, CharStyle, ClearGroup, ClearLayer, ClearTarget, Color, Element,
    FrameRate, LayerVisibility, Library, Link, Look, MediaAsset, MediaLayer, MediaTrigger,
    Paragraph, Placeholder, PlaylistItem, PlaylistItemKind, PlaylistNode, PlaylistNodeKind,
    PlaylistPurpose, Presentation, Rect, Screen, ScreenLook, Section, Size, Slide, SlideAction,
    TemplateOverride, TemplateSlide, TemplateSlideId, TextBox, Theme,
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use vesper_server::{RunningServer, ServerConfig, start};

pub const W: u32 = 640;
pub const H: u32 = 360;

pub fn write_doc<T: serde::Serialize>(dir: &Path, section: &str, kind: &str, id: &str, doc: &T) {
    let d = dir.join(section).join(kind);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join(format!("{id}.json")),
        serde_json::to_vec_pretty(doc).unwrap(),
    )
    .unwrap();
}

/// The content root of a started server's data folder: its Codex's active collection
/// (`codex/collections/<id>`, from `machine/codex.json`), where presentations, playlists and
/// the other collection documents are stored once the server has migrated the fixture.
pub fn content_dir(dir: &Path) -> PathBuf {
    let config: Value =
        serde_json::from_slice(&std::fs::read(dir.join("machine/codex.json")).unwrap()).unwrap();
    let id = config["collection"].as_str().expect("an active collection");
    dir.join("codex/collections").join(id)
}

pub fn text_element(bounds: Rect, text: &str, color: Color) -> Element {
    Element::text(
        bounds,
        TextBox {
            paragraphs: Paragraph::from_lines(text),
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                font_weight: Some(700),
                size: Some(90.0),
                color: Some(color),
                ..CharStyle::default()
            },
            ..TextBox::default()
        },
    )
}

pub struct Fixture {
    pub dir: PathBuf,
    pub library: Library,
    pub song: Presentation,
    pub second: Presentation,
    pub audience: Screen,
    pub key: Screen,
    pub default_look: Look,
    pub plain_look: Look,
    pub folder: PlaylistNode,
    pub playlist: PlaylistNode,
    pub clear_slide: ClearGroup,
    pub asset: MediaAsset,
}

impl Fixture {
    /// Items of the Sunday playlist: header, song, placeholder, second song.
    pub fn items(&self) -> Vec<PlaylistItem> {
        match &self.playlist.kind {
            PlaylistNodeKind::Playlist { items, .. } => items.clone(),
            _ => unreachable!(),
        }
    }
}

fn slide(text: &str) -> Slide {
    Slide {
        elements: vec![text_element(
            Rect::new(20.0, 80.0, W as f64 - 40.0, 200.0),
            text,
            Color::WHITE,
        )],
        ..Slide::new()
    }
}

/// Two screens (opaque "Audience", straight-alpha "Key"), two looks ("Default": Key without
/// media and the slide text restyled through a red lower-third template; "Plain": everything
/// everywhere), a blue background image, a 4-slide song (slide 2 disabled) and a 2-slide song in
/// one library, a folder with a "Sunday" playlist [header, song, placeholder, second song], a
/// "Clear Slide" clear group, plus one broken and one too-new document.
pub fn fixture(tag: &str) -> Fixture {
    // FFmpeg is probed once for the whole test binary, before this test's server renders
    // (M-4: a probe while another server renders can crash it in the Vulkan loader).
    media::warm_up();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "server-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("media")).unwrap();

    image::RgbImage::from_pixel(64, 36, image::Rgb([20, 40, 230]))
        .save(dir.join("media").join("bg0001.png"))
        .unwrap();
    let asset = MediaAsset {
        name: "Blue background".into(),
        location: AssetLocation::Managed {
            sha256: "bg0001".into(),
            extension: "png".into(),
        },
        ..MediaAsset::default()
    };
    write_doc(
        &dir,
        "portable",
        "mediaAsset",
        &asset.id.0.to_string(),
        &asset,
    );

    let canvas = Size::new(W as f64, H as f64);
    let audience = Screen {
        name: "Audience".into(),
        sort_key: "a".into(),
        size: canvas,
        frame_rate: FrameRate::FPS30,
        ..Screen::default()
    };
    let key = Screen {
        name: "Key".into(),
        sort_key: "b".into(),
        size: canvas,
        frame_rate: FrameRate::FPS25,
        alpha: AlphaMode::Straight,
        ..Screen::default()
    };
    for s in [&audience, &key] {
        write_doc(&dir, "portable", "screen", &s.id.0.to_string(), s);
    }

    let mut slot = text_element(
        Rect::new(0.0, 290.0, W as f64, 70.0),
        "",
        Color::rgb(1.0, 0.0, 0.0),
    );
    slot.placeholder = Some(Placeholder::text("text1"));
    if let Some(tb) = slot.text_box_mut() {
        tb.default_style.size = Some(40.0);
    }
    let tslide_id = TemplateSlideId::new();
    let theme = Theme {
        name: "Lower Thirds".into(),
        canvas_size: canvas,
        slides: vec![TemplateSlide {
            id: tslide_id,
            name: "Lyrics".into(),
            slide: Slide {
                elements: vec![slot],
                ..Slide::new()
            },
            guides: Vec::new(),
            import: None,
        }],
        ..Theme::default()
    };
    write_doc(&dir, "portable", "theme", &theme.id.0.to_string(), &theme);

    let mut key_layers = LayerVisibility::ALL;
    key_layers.media = false;
    let default_look = Look {
        name: "Default".into(),
        sort_key: "a".into(),
        screens: vec![ScreenLook {
            screen: key.id,
            layers: key_layers,
            template_override: Some(TemplateOverride {
                theme: Link::named(theme.id, "Lower Thirds"),
                slide: tslide_id,
                mapping: Default::default(),
                keep_inline_formatting: true,
                hide_when_empty: true,
            }),
            mask: None,
            language: Default::default(),
        }],
        ..Look::default()
    };
    let plain_look = Look {
        name: "Plain".into(),
        sort_key: "b".into(),
        ..Look::default()
    };
    for l in [&default_look, &plain_look] {
        write_doc(&dir, "portable", "look", &l.id.0.to_string(), l);
    }

    // Changes cut by default here, so frame checks see the new picture at once; transition
    // tests ask for theirs.
    let cut = Some(model::Transition::CUT);
    let settings = model::ShowSettings {
        default_transitions: model::LayerTransitions {
            media: cut.clone(),
            slide: cut.clone(),
            announcements: cut,
            ..Default::default()
        },
        startup_look: Some(default_look.id),
        ..Default::default()
    };
    write_doc(
        &dir,
        "portable",
        "showSettings",
        "00000000-0000-0000-0000-000000000000",
        &settings,
    );

    let library = Library {
        name: "Songs".into(),
        sort_key: "a".into(),
        ..Library::default()
    };
    write_doc(
        &dir,
        "portable",
        "library",
        &library.id.0.to_string(),
        &library,
    );

    let mut s0 = slide("ONE");
    s0.actions.push(SlideAction::media(MediaTrigger::new(
        asset.id,
        MediaLayer::Background,
    )));
    let mut s2 = slide("SKIP");
    s2.enabled = false;
    let mut verse = Section::new("Verse 1");
    verse.slides = vec![s0, slide("TWO")];
    let mut chorus = Section::new("Chorus");
    chorus.slides = vec![s2, slide("FOUR")];
    let song = Presentation {
        library: Some(library.id),
        canvas_size: canvas,
        sections: vec![verse, chorus],
        ..Presentation::new("Test Song")
    };
    let mut b = Section::new("Verse");
    b.slides = vec![slide("B1"), slide("B2")];
    let second = Presentation {
        library: Some(library.id),
        canvas_size: canvas,
        sections: vec![b],
        ..Presentation::new("Second Song")
    };
    for p in [&song, &second] {
        write_doc(&dir, "portable", "presentation", &p.id.0.to_string(), p);
    }

    let folder = PlaylistNode {
        name: "Services".into(),
        kind: PlaylistNodeKind::Folder,
        ..PlaylistNode::default()
    };
    let item = |p: &Presentation| {
        PlaylistItem::new(PlaylistItemKind::Presentation {
            presentation: Link::named(p.id, p.name.clone()),
            arrangement: None,
            unresolved: None,
        })
    };
    let playlist = PlaylistNode {
        name: "Sunday".into(),
        parent: Some(folder.id),
        kind: PlaylistNodeKind::Playlist {
            purpose: PlaylistPurpose::Service,
            items: vec![
                PlaylistItem::new(PlaylistItemKind::Header {
                    name: "Worship".into(),
                    color: None,
                }),
                item(&song),
                PlaylistItem::new(PlaylistItemKind::Placeholder),
                item(&second),
            ],
        },
        ..PlaylistNode::default()
    };
    for n in [&folder, &playlist] {
        write_doc(&dir, "portable", "playlistNode", &n.id.0.to_string(), n);
    }

    let clear_slide = ClearGroup {
        name: "Clear Slide".into(),
        targets: vec![ClearTarget {
            layer: ClearLayer::Slide,
            destination: None,
        }],
        ..ClearGroup::default()
    };
    write_doc(
        &dir,
        "portable",
        "clearGroup",
        &clear_slide.id.0.to_string(),
        &clear_slide,
    );

    std::fs::write(dir.join("portable/presentation/broken.json"), b"{ not json").unwrap();
    let mut future = serde_json::to_value(Presentation::new("Future")).unwrap();
    future["schemaVersion"] = json!(999);
    std::fs::write(
        dir.join("portable/presentation/future.json"),
        serde_json::to_vec(&future).unwrap(),
    )
    .unwrap();

    Fixture {
        dir,
        library,
        song,
        second,
        audience,
        key,
        default_look,
        plain_look,
        folder,
        playlist,
        clear_slide,
        asset,
    }
}

/// Starts a server on a random localhost port for the fixture (no mDNS unless asked, no Site
/// Manager: `tests/server/site.rs` starts its own on loopback ports).
pub async fn start_server(fx: &Fixture, trust_localhost: bool, mdns: bool) -> RunningServer {
    start_server_with(fx, |c| {
        c.trust_localhost = trust_localhost;
        c.mdns = mdns;
    })
    .await
}

/// Starts a server like [`start_server`] (trusting localhost, no mDNS), its configuration
/// changed by `change` first.
pub async fn start_server_with(
    fx: &Fixture,
    change: impl FnOnce(&mut ServerConfig),
) -> RunningServer {
    let dejavu = Path::new("/usr/share/fonts/truetype/dejavu");
    let renderer = if dejavu.is_dir() {
        render::RendererOptions {
            system_fonts: false,
            font_dirs: vec![dejavu.to_path_buf()],
            ..render::RendererOptions::default()
        }
    } else {
        render::RendererOptions::default()
    };
    let mut config = ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        renderer,
        trust_localhost: true,
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        ..ServerConfig::new(&fx.dir)
    };
    change(&mut config);
    start(config).await.expect("server starts")
}

/// Starts a server like [`start_server`] (no mDNS) with the given site module, e.g.
/// `SiteConfig::loopback()`: its own site on free ports, so clients can pair.
pub async fn start_with_site(
    fx: &Fixture,
    trust_localhost: bool,
    site: vesper_server::site::SiteConfig,
) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost,
        mdns: false,
        site,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

/// Waits for the WebSocket to be closed by the server; the close code.
pub async fn closed_by_server(ws: &mut Ws) -> Option<u16> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the server didn't close the WebSocket");
        match tokio::time::timeout(left, futures_util::StreamExt::next(ws)).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(f)))) => {
                return f.map(|f| u16::from(f.code));
            }
            Ok(Some(Ok(_))) => continue,
            Ok(_) => return None,
            Err(_) => panic!("the server didn't close the WebSocket"),
        }
    }
}

/// An HTTP response.
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A minimal HTTP/1.1 client (one request per connection).
pub async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Resp {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    // (A `Host` in `headers` replaces the default, the server's address.)
    let host = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map_or_else(|| addr.to_string(), |(_, v)| v.to_string());
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("host"))
    {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), s.read_to_end(&mut resp))
        .await
        .expect("HTTP response in time")
        .unwrap();
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP header end");
    let head = String::from_utf8_lossy(&resp[..split]).to_string();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut body = resp[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"))
    {
        body = dechunk(&body);
    }
    Resp {
        status,
        headers,
        body,
    }
}

/// [`http`] with a raw body (`application/octet-stream`).
pub async fn http_bytes(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Resp {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut resp = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), s.read_to_end(&mut resp))
        .await
        .expect("HTTP response in time")
        .unwrap();
    parse_response(&resp)
}

fn parse_response(resp: &[u8]) -> Resp {
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP header end");
    let head = String::from_utf8_lossy(&resp[..split]).to_string();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut body = resp[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"))
    {
        body = dechunk(&body);
    }
    Resp {
        status,
        headers,
        body,
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(nl) = b.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let n = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).trim(), 16).unwrap_or(0);
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&b[nl + 2..nl + 2 + n]);
        b = &b[nl + 2 + n + 2..];
    }
}

/// GET returning JSON, asserting 200.
pub async fn get_json(addr: SocketAddr, path: &str) -> Value {
    let r = http(addr, "GET", path, &[], None).await;
    assert_eq!(
        r.status,
        200,
        "GET {path}: {}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A WebSocket client that tracks the newest state.
pub struct Client {
    pub ws: Ws,
    next_id: u32,
    pub hello: Value,
    pub revision: u64,
    pub state: Value,
    /// Binary frames received while waiting for something else.
    pub binary: Vec<Vec<u8>>,
    /// Notices and other events seen.
    pub events: Vec<Value>,
}

impl Client {
    /// Connects (`query` without the leading `?`) and reads `hello` + the first state.
    pub async fn connect(addr: SocketAddr, query: &str) -> Client {
        let url = format!("ws://{addr}/api/v1/ws?{query}");
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("WebSocket connects");
        let mut c = Client {
            ws,
            next_id: 1,
            hello: Value::Null,
            revision: 0,
            state: Value::Null,
            binary: Vec::new(),
            events: Vec::new(),
        };
        let hello = c.next_text().await;
        assert_eq!(hello["type"], "hello", "{hello}");
        c.hello = hello;
        let st = c.next_text().await;
        assert_eq!(st["type"], "state", "{st}");
        c.absorb(&st);
        c
    }

    /// Records a message: states update `revision` / `state`, others go to `events`.
    pub fn absorb(&mut self, m: &Value) {
        if m["type"] == "state" {
            let rev = m["revision"].as_u64().unwrap();
            assert!(
                rev > self.revision,
                "revisions increase ({rev} after {})",
                self.revision
            );
            self.revision = rev;
            self.state = m["state"].clone();
        } else if m["type"] != "result" {
            self.events.push(m.clone());
        }
    }

    /// The next message (text parsed as JSON; binary stored and returned as `{"binary": n}`).
    pub async fn next_any(&mut self) -> Value {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(60), self.ws.next())
                .await
                .expect("a WebSocket message in time")
                .expect("WebSocket open")
                .expect("WebSocket message");
            match m {
                Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
                Message::Binary(b) => {
                    self.binary.push(b.to_vec());
                    return json!({ "binary": self.binary.len() });
                }
                Message::Close(f) => return json!({ "close": f.map(|f| u16::from(f.code)) }),
                _ => continue,
            }
        }
    }

    async fn next_text(&mut self) -> Value {
        loop {
            let v = self.next_any().await;
            if v.get("binary").is_none() {
                return v;
            }
        }
    }

    /// Sends a command and returns its result (states received meanwhile are absorbed).
    pub async fn command(&mut self, command: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "type": "command", "id": id, "command": command });
        self.ws
            .send(Message::Text(msg.to_string().into()))
            .await
            .unwrap();
        loop {
            let m = self.next_text().await;
            if m["type"] == "result" && m["id"] == id {
                return m;
            }
            self.absorb(&m);
        }
    }

    /// Sends a command that must succeed; returns the result's revision. The state of that
    /// revision has arrived by then.
    pub async fn ok(&mut self, command: Value) -> u64 {
        let r = self.command(command.clone()).await;
        assert_eq!(r["ok"], true, "{command} -> {r}");
        let rev = r["revision"].as_u64().unwrap();
        assert!(
            self.revision >= rev,
            "state {rev} arrives before its result"
        );
        rev
    }

    /// Waits until a state satisfies `pred`.
    pub async fn wait_state(&mut self, what: &str, pred: impl Fn(u64, &Value) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !pred(self.revision, &self.state) {
            assert!(
                Instant::now() < deadline,
                "{what}: state never matched: {}",
                self.state
            );
            let m = self.next_text().await;
            self.absorb(&m);
        }
    }

    /// Waits for the next binary message.
    pub async fn next_binary(&mut self) -> Vec<u8> {
        if let Some(b) = self.binary.pop() {
            return b;
        }
        loop {
            let v = self.next_any().await;
            if v.get("binary").is_some() {
                return self.binary.pop().unwrap();
            }
            self.absorb(&v);
        }
    }
}

/// The latest rendered frame of screen `i`, straight alpha.
pub fn latest(server: &RunningServer, i: usize) -> Option<image::RgbaImage> {
    let f = server.shared.screens()[i].latest.lock().unwrap().clone()?;
    Some(f.to_straight_rgba())
}

/// Polls screen `i` until `ok` accepts its frame.
pub async fn wait_frame(
    server: &RunningServer,
    i: usize,
    what: &str,
    ok: impl Fn(&image::RgbaImage) -> bool,
) -> image::RgbaImage {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(img) = latest(server, i)
            && ok(&img)
        {
            return img;
        }
        if Instant::now() > deadline {
            let p = std::env::temp_dir().join("server-api-failed-frame.png");
            if let Some(img) = latest(server, i) {
                img.save(&p).ok();
            }
            panic!(
                "{what}: frame never matched (last one saved to {})",
                p.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

pub fn near(p: &image::Rgba<u8>, c: [u8; 3], tol: i32) -> bool {
    (0..3).all(|k| (p.0[k] as i32 - c[k] as i32).abs() <= tol)
}

/// Number of opaque pixels in the rows `y0..y1` that look like `c`.
pub fn count(img: &image::RgbaImage, y0: u32, y1: u32, c: [u8; 3], tol: i32) -> usize {
    (y0..y1.min(img.height()))
        .flat_map(|y| (0..img.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = img.get_pixel(x, y);
            p.0[3] > 200 && near(p, c, tol)
        })
        .count()
}

pub const BLUE: [u8; 3] = [20, 40, 230];
pub const WHITE: [u8; 3] = [255, 255, 255];
pub const BLACK: [u8; 3] = [0, 0, 0];
pub const RED: [u8; 3] = [255, 0, 0];
