//! Wire format checks: serde round-trips, the JSON shapes the README documents, and agreement
//! between what serde emits and the committed TypeScript bindings (field names, tag values,
//! required vs optional fields).

use model::{
    ClearGroupId, FrameRate, Layer, LookId, MediaAssetId, MediaKind, PlaylistId, PlaylistItemId,
    PresentationId, ScreenId, SectionId, SlideId, TimerId, Transition, TransitionKind,
};
use protocol::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::path::Path;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(v: &T) -> Value {
    let json = serde_json::to_value(v).unwrap();
    let back: T = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(&back, v, "round trip of {json}");
    json
}

fn sample_position(playlist: bool) -> SlidePosition {
    SlidePosition {
        presentation: PresentationId::new(),
        presentation_name: "Morning Light".into(),
        slide: SlideId::new(),
        slide_index: 3,
        section: SectionId::new(),
        section_name: "Chorus".into(),
        playlist: playlist.then(|| PlaylistContext {
            playlist: PlaylistId::new(),
            item: PlaylistItemId::new(),
        }),
        step: Some(5),
    }
}

fn sample_presentation_ops() -> Vec<PresentationOp> {
    use model::*;
    let slide = Slide::new();
    let el = Element::text(Rect::new(0.0, 0.0, 10.0, 10.0), TextBox::plain("a"));
    vec![
        PresentationOp::SetPresentation {
            name: Some("Song".into()),
            song_info: Some(None),
            scripture: None,
            transition: Some(None),
            canvas_size: Some(Size::HD),
            notes: None,
            category: Some(Some("Hymns".into())),
            language: Some(Some("es".into())),
        },
        PresentationOp::AddSection {
            after: Some(SectionId::new()),
            section: Section::new("Verse 1"),
        },
        PresentationOp::DeleteSection {
            section: SectionId::new(),
        },
        PresentationOp::MoveSection {
            section: SectionId::new(),
            after: None,
        },
        PresentationOp::SetSection {
            section: SectionId::new(),
            name: Some("Chorus".into()),
            color: Some(Some(Color::WHITE)),
            hotkey: Some(None),
            global_group: None,
        },
        PresentationOp::AddSlide {
            section: SectionId::new(),
            after: None,
            slide: Some(slide.clone()),
        },
        PresentationOp::DeleteSlide { slide: slide.id },
        PresentationOp::MoveSlide {
            slide: slide.id,
            to_section: SectionId::new(),
            after: Some(SlideId::new()),
        },
        PresentationOp::DuplicateSlide { slide: slide.id },
        PresentationOp::SetSlide {
            slide: slide.id,
            label: Some(Some(SlideLabel {
                text: "3X".into(),
                color: None,
            })),
            enabled: Some(false),
            notes: None,
            transition: None,
            background: Some(None),
            hotkey: None,
            builds: None,
        },
        PresentationOp::SetSlideText {
            slide: slide.id,
            element: None,
            text: "a\nb".into(),
            expect_text: Some("a".into()),
        },
        PresentationOp::SplitSlide {
            slide: slide.id,
            at_paragraph: 1,
        },
        PresentationOp::MergeSlides {
            first: slide.id,
            second: SlideId::new(),
        },
        PresentationOp::AddElement {
            slide: slide.id,
            element: el.clone(),
            index: Some(0),
        },
        PresentationOp::DeleteElement {
            slide: slide.id,
            element: el.id,
        },
        PresentationOp::ReorderElement {
            slide: slide.id,
            element: el.id,
            index: 2,
        },
        PresentationOp::SetElement {
            slide: slide.id,
            element: el.id,
            patch: ElementPatch {
                bounds: Some(Rect::new(1.0, 2.0, 3.0, 4.0)),
                opacity: Some(0.5),
                ..ElementPatch::default()
            },
        },
        PresentationOp::SetTextStyle {
            slide: slide.id,
            element: el.id,
            range: Some(TextRange { start: 0, end: 1 }),
            character: Some(CharStyle {
                size: Some(40.0),
                ..CharStyle::default()
            }),
            paragraph: None,
            clear: vec!["color".into()],
        },
        PresentationOp::SetTextBox {
            slide: slide.id,
            element: el.id,
            vertical_align: Some(VerticalAlign::Top),
            fit: None,
            margins: None,
            outline: None,
            text_shadow: None,
            line_mask: None,
        },
        PresentationOp::ReplaceTextBox {
            slide: slide.id,
            element: el.id,
            text_box: TextBox::plain("x"),
        },
        PresentationOp::SetArrangements {
            arrangements: vec![Arrangement::default()],
            selected: Some(None),
        },
        PresentationOp::AddSlideAction {
            slide: slide.id,
            after: None,
            action: SlideAction::new(ActionKind::Timer {
                timer: Link::to(TimerId::new()),
                operation: TimerOperation::Set { seconds: 30.0 },
            }),
        },
        PresentationOp::AddSlideAction {
            slide: slide.id,
            after: Some(ActionId::new()),
            action: SlideAction::new(ActionKind::Message {
                message: Link::to(MessageId::new()),
                show: false,
                values: vec![TokenValue {
                    name: "a".into(),
                    value: "b".into(),
                }],
            }),
        },
        PresentationOp::RemoveSlideAction {
            slide: slide.id,
            action: ActionId::new(),
        },
        PresentationOp::MoveSlideAction {
            slide: slide.id,
            action: ActionId::new(),
            after: Some(ActionId::new()),
        },
        PresentationOp::SetSlideAction {
            slide: slide.id,
            action: SlideAction::new(ActionKind::Prop {
                prop: Link::to(PropId::new()),
                show: true,
            }),
        },
    ]
}

fn sample_playlist_ops() -> Vec<PlaylistOp> {
    use model::*;
    let item = PlaylistItem::new(PlaylistItemKind::Header {
        name: "Worship".into(),
        color: Some(Color::WHITE),
    });
    vec![
        PlaylistOp::AddItem {
            after: None,
            item: item.clone(),
        },
        PlaylistOp::RemoveItem { item: item.id },
        PlaylistOp::MoveItem {
            item: item.id,
            after: Some(PlaylistItemId::new()),
        },
        PlaylistOp::SetItem {
            item: item.id,
            arrangement: Some(None),
            enabled: Some(true),
            name: Some(Some("Opening".into())),
            color: None,
        },
    ]
}

#[test]
fn edit_ops_round_trip_and_match_typescript() {
    for op in sample_presentation_ops() {
        round_trip(&op);
        agree("PresentationOp", &op);
    }
    for op in sample_playlist_ops() {
        round_trip(&op);
        agree("PlaylistOp", &op);
    }
    // `null` clears, an absent field is left alone.
    let op: PresentationOp = serde_json::from_value(
        json!({"type": "setSection", "section": SectionId::new(), "color": null}),
    )
    .unwrap();
    match op {
        PresentationOp::SetSection { color, hotkey, .. } => {
            assert_eq!(color, Some(None));
            assert_eq!(hotkey, None);
        }
        _ => unreachable!(),
    }
    let r = PresentationEditResult {
        revision: "r".into(),
        applied: sample_presentation_ops(),
        inverse: vec![],
    };
    agree("PresentationEditResult", &round_trip(&r));
    agree(
        "EditConflict",
        &EditConflict {
            revision: Some("r".into()),
            op_index: 0,
            slide: Some(SlideId::new()),
            element: None,
            text: Some("old".into()),
        },
    );
    agree(
        "CreatePlaylistNode",
        &CreatePlaylistNode {
            name: Some("Sunday".into()),
            kind: Some(PlaylistNodeType::Playlist),
            purpose: Some(model::PlaylistPurpose::Audio),
            parent: None,
            after: None,
            document: None,
        },
    );
    agree(
        "UpdatePlaylistNode",
        &UpdatePlaylistNode {
            name: None,
            parent: Some(None),
            after: Some(Some(PlaylistId::new())),
        },
    );
    agree(
        "DeletedDocuments",
        &DeletedDocuments {
            kind: model::DocumentKind::Library,
            documents: vec![],
        },
    );
}

fn sample_state() -> ShowState {
    ShowState {
        layers: LayerStates {
            video_input: Some(VideoInputLayerState {
                input: Some(model::VideoInputId::new()),
                name: "Camera 1".into(),
                source: "BOOTH (Camera 1)".into(),
            }),
            media: Some(MediaLayerState {
                asset: MediaAssetId::new(),
                name: "Blue".into(),
                kind: MediaKind::Image,
                online: true,
                playback: MediaPlayback {
                    state: PlaybackState::Playing,
                    position: 1.5,
                    duration: Some(10.0),
                    looping: true,
                    rate: 1.0,
                    hw: true,
                },
            }),
            foreground: None,
            slide: Some(sample_position(true)),
            announcements: Some(sample_position(false)),
            props: vec![PropLayerState {
                prop: model::PropId::new(),
                name: "Logo".into(),
            }],
            messages: vec![MessageLayerState {
                message: model::MessageId::new(),
                name: "Nursery".into(),
                text: "Parent of 123, please come".into(),
                tokens: vec![model::TokenValue {
                    name: "child".into(),
                    value: "123".into(),
                }],
                visible: true,
            }],
            audio: None,
        },
        active_look: Some(LookId::new()),
        transitions: vec![TransitionState {
            layer: Layer::Slide,
            kind: TransitionKind::Dissolve,
            progress: 0.5,
            duration: 1.0,
        }],
        transport: Some(MediaTransport {
            layer: Layer::Media,
            asset: MediaAssetId::new(),
            name: "Clouds".into(),
            state: PlaybackState::Paused,
            position: 3.25,
            duration: Some(30.0),
            looping: true,
            rate: 1.0,
            hw: true,
        }),
        timers: vec![TimerState {
            id: TimerId::new(),
            name: "Countdown".into(),
            kind: model::TimerKind::Countdown { seconds: 300.0 },
            allow_overrun: false,
            state: TimerRunState::Stopped,
            current_seconds: 300.0,
            display: "05:00".into(),
            warning_seconds: Some(60.0),
        }],
        stage_layouts: vec![StageLayoutState {
            screen: None,
            layout: model::StageLayoutId::new(),
        }],
        stage_screens: vec![StageScreenState {
            screen: ScreenId::new(),
            name: "Stage".into(),
            layout: Some(model::StageLayoutId::new()),
            layout_name: "Current / Next".into(),
        }],
        stage_message: Some(StageMessageState {
            text: "Wrap up".into(),
            flash: true,
        }),
        screens: vec![ScreenState {
            id: ScreenId::new(),
            name: "Audience".into(),
            role: ScreenRole::Audience,
            enabled: true,
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::FPS30,
            measured_fps: 29.9,
            last_render_ms: Some(4.2),
            test_pattern: Some(model::TestPattern::Solid {
                color: model::Color::WHITE,
            }),
            outputs: vec![OutputState {
                name: "IDN: Audience".into(),
                kind: "idn".into(),
                origin: "binding".into(),
                binding: Some(model::OutputId::new()),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::FPS25,
                delay_frames: 2,
                status: OutputStatus::Ok,
                detail: None,
                frames_sent: 100,
                frames_dropped: 1,
                errors: 0,
                embed_audio: true,
                audio_db: -3.0,
                audio_muted: false,
            }],
            signage: None,
        }],
        screens_enabled: ScreensEnabled {
            audience: true,
            stage: false,
        },
        identify: true,
        options: ShowOptions {
            continuous_playlist: false,
        },
        cursor: Some(sample_position(true)),
        diagnostics: DiagnosticsSummary {
            font_substitutions: 2,
            ..DiagnosticsSummary::default()
        },
        audio: AudioState {
            device: "hw:1,0".into(),
            output_state: AudioOutputState::Ok,
            detail: None,
            master_db: -3.0,
            muted: false,
            levels: vec![-12.0, -60.0],
            xruns: 0,
            layers: AudioLayerVolumes::default(),
            shuffle: true,
            repeat: false,
            playlist: None,
        },
        restorable_layers: vec![Layer::Slide, Layer::Props],
        clock_format: ClockFormatState {
            use_24_hour: true,
            seconds: false,
        },
        displays: vec![protocol::DisplayInfo {
            id: "HDMI-A-1".into(),
            index: 0,
            name: "Example Monitor".into(),
            manufacturer: Some("XYZ".into()),
            connected: true,
            modes: vec![model::DisplayMode {
                width: 1920,
                height: 1080,
                refresh_mhz: 60000,
            }],
            preferred: Some(model::DisplayMode {
                width: 1920,
                height: 1080,
                refresh_mhz: 60000,
            }),
            source: Some("window".into()),
            position: Some(protocol::DisplayPosition { x: 1920, y: 0 }),
            primary: false,
            bound_to: vec![],
        }],
        build: Some(protocol::BuildProgress {
            layer: Layer::Slide,
            presentation: PresentationId::new(),
            slide: SlideId::new(),
            step: 1,
            steps: 3,
        }),
        captions: protocol::CaptionsState {
            enabled: true,
            status: protocol::CaptionsStatus::Running,
            detail: None,
            progress: None,
            downloaded: None,
            download_size: None,
            model: Some("base.en".into()),
            device: Some("CPU (6 threads)".into()),
            load: Some(0.31),
            lines: vec![protocol::CaptionLineState {
                line: 4,
                text: "Welcome, everyone.".into(),
                is_final: false,
            }],
        },
    }
}

fn all_commands() -> Vec<Command> {
    let p = PresentationId::new();
    vec![
        Command::TriggerSlide {
            presentation: p,
            slide: Some(SlideId::new()),
            index: None,
            playlist_item: Some(PlaylistItemRef {
                playlist: PlaylistId::new(),
                item: PlaylistItemId::new(),
            }),
            step: Some(4),
            transition: None,
        },
        Command::TriggerSlide {
            presentation: p,
            slide: None,
            index: Some(2),
            playlist_item: None,
            step: None,
            transition: Some(Transition::dissolve(1.5)),
        },
        Command::Next,
        Command::Prev,
        Command::TriggerPlaylistItem {
            playlist: PlaylistId::new(),
            item: PlaylistItemId::new(),
        },
        Command::ClearLayer {
            layer: Layer::Media,
        },
        Command::ClearAll,
        Command::ClearGroup {
            id: ClearGroupId::new(),
        },
        Command::SetLook {
            look: Some(LookId::new()),
        },
        Command::SetLook { look: None },
        Command::TriggerMedia {
            asset: MediaAssetId::new(),
            layer: Some(Layer::Media),
            transition: Some(Transition::CUT),
        },
        Command::TriggerMedia {
            asset: MediaAssetId::new(),
            layer: None,
            transition: None,
        },
        Command::SetScreensEnabled {
            role: ScreenRole::Stage,
            enabled: false,
        },
        Command::SetOption {
            continuous_playlist: Some(false),
        },
        Command::SetOption {
            continuous_playlist: None,
        },
        Command::TimerStart {
            timer: TimerId::new(),
        },
        Command::TimerStop {
            timer: TimerId::new(),
        },
        Command::TimerReset {
            timer: TimerId::new(),
        },
        Command::TimerSet {
            timer: TimerId::new(),
            seconds: Some(60.0),
            time_of_day: None,
        },
        Command::TimerSet {
            timer: TimerId::new(),
            seconds: None,
            time_of_day: Some(36000.0),
        },
        Command::TimerAdjust {
            timer: TimerId::new(),
            delta_seconds: -30.0,
        },
        Command::PropShow {
            prop: model::PropId::new(),
        },
        Command::PropHide {
            prop: model::PropId::new(),
        },
        Command::PropToggle {
            prop: model::PropId::new(),
        },
        Command::MessageShow {
            message: model::MessageId::new(),
            tokens: vec![model::TokenValue {
                name: "child".into(),
                value: "123".into(),
            }],
        },
        Command::MessageShow {
            message: model::MessageId::new(),
            tokens: vec![],
        },
        Command::MessageHide {
            message: Some(model::MessageId::new()),
        },
        Command::MessageHide { message: None },
        Command::MediaPlay { layer: None },
        Command::MediaPause {
            layer: Some(Layer::Media),
        },
        Command::MediaToggle { layer: None },
        Command::MediaSeek {
            seconds: 12.5,
            layer: None,
        },
        Command::MediaSkip {
            seconds: -15.0,
            layer: Some(Layer::Slide),
        },
        Command::MediaRestart { layer: None },
        Command::MediaStop { layer: None },
        Command::MediaSetLoop {
            looping: false,
            layer: Some(Layer::Audio),
        },
        Command::RestoreLayer {
            layer: Layer::Media,
        },
        Command::SetClockFormat {
            use_24_hour: Some(true),
            seconds: None,
        },
        Command::TriggerVideoInput {
            input: model::VideoInputId::new(),
        },
        Command::SetTransitionOverride {
            layer: Layer::Slide,
            transition: Some(Transition {
                kind: TransitionKind::Dissolve,
                duration: 0.5,
                ..Default::default()
            }),
        },
        Command::SubscribePreview {
            screen: ScreenId::new(),
            fps: 10.0,
            width: 480,
        },
        Command::UnsubscribePreview { screen: None },
        Command::PrefetchThumbnails {
            presentation: p,
            width: Some(320),
        },
        Command::ReloadData,
        Command::AudioNext,
        Command::AudioPrev,
        Command::SetAudioOptions {
            shuffle: Some(true),
            repeat: None,
        },
        Command::SetVolume {
            target: VolumeTarget::Layer,
            layer: Some(model::Layer::Audio),
            output: None,
            db: -6.0,
        },
        Command::SetMute {
            target: VolumeTarget::Master,
            layer: None,
            output: None,
            muted: true,
        },
        Command::PlayTestTone {
            frequency: Some(1000.0),
            seconds: Some(2.0),
        },
        Command::IdentifyScreens { enabled: true },
        Command::SetStageLayout {
            screen: ScreenId::new(),
            layout: Some(model::StageLayoutId::new()),
        },
        Command::SetStageLayout {
            screen: ScreenId::new(),
            layout: None,
        },
        Command::StageMessage {
            text: "5 minutes".into(),
            flash: true,
        },
        Command::StageMessage {
            text: "Look at me".into(),
            flash: false,
        },
        Command::StageMessageClear,
        Command::SetTestPattern {
            screen: ScreenId::new(),
            pattern: Some(model::TestPattern::ColorBars),
        },
        Command::SetTestPattern {
            screen: ScreenId::new(),
            pattern: Some(model::TestPattern::Solid {
                color: model::Color::BLACK,
            }),
        },
        Command::SetTestPattern {
            screen: ScreenId::new(),
            pattern: None,
        },
    ]
}

fn server_messages() -> Vec<ServerMessage> {
    vec![
        ServerMessage::Hello {
            server: ServerHello {
                id: "6a1d1c8e-5f53-4c39-9a51-7d1b5a1b2f10".into(),
                name: "Booth".into(),
                version: "0.1.0".into(),
                protocol_version: PROTOCOL_VERSION,
            },
        },
        ServerMessage::State {
            revision: 7,
            state: sample_state(),
        },
        ServerMessage::Result(CommandResult::ok(1, 8)),
        ServerMessage::Result(CommandResult::err(
            2,
            ApiError::new(ErrorCode::NotImplemented, "timers arrive in M4"),
        )),
        ServerMessage::DocumentChanged {
            kind: model::DocumentKind::Presentation,
            id: PresentationId::new().to_string(),
            change: DocumentChange::Changed,
            revision: None,
            client_id: None,
            ops: None,
            collection: None,
        },
        ServerMessage::DocumentChanged {
            kind: model::DocumentKind::Presentation,
            id: PresentationId::new().to_string(),
            change: DocumentChange::Changed,
            revision: Some("0123456789abcdef".into()),
            client_id: Some("tab-1".into()),
            ops: Some(EditOps::Presentation(sample_presentation_ops())),
            collection: Some(model::CollectionId::new()),
        },
        ServerMessage::DocumentChanged {
            kind: model::DocumentKind::PlaylistNode,
            id: PlaylistId::new().to_string(),
            change: DocumentChange::Changed,
            revision: Some("0123456789abcdef".into()),
            client_id: None,
            ops: Some(EditOps::Playlist(sample_playlist_ops())),
            collection: None,
        },
        ServerMessage::CodexChanged {
            parts: vec![
                protocol::CodexPart::Status,
                protocol::CodexPart::Collections,
            ],
            collection: Some(model::CollectionId::new()),
        },
        ServerMessage::CodexChanged {
            parts: vec![protocol::CodexPart::Collections],
            collection: None,
        },
        ServerMessage::Notice {
            level: NoticeLevel::Warning,
            message: "media missing".into(),
            key: None,
        },
        ServerMessage::Notice {
            level: NoticeLevel::Warning,
            message: "3 media files missing in 'Song'".into(),
            key: Some("missingMedia:x".into()),
        },
        ServerMessage::TimerTick {
            timers: vec![TimerTick {
                id: TimerId::new(),
                state: TimerRunState::Running,
                current_seconds: 299.5,
                display: "05:00".into(),
            }],
        },
        ServerMessage::Pong { id: 3 },
        ServerMessage::Error {
            error: ApiError::new(ErrorCode::IncompatibleProtocol, "server speaks 1"),
        },
        ServerMessage::SiteEvent {
            event: sample_site_event(),
        },
        ServerMessage::SiteChanged {
            parts: vec![SitePart::JoinRequests, SitePart::Devices],
        },
    ]
}

#[test]
fn messages_round_trip() {
    for c in all_commands() {
        round_trip(&c);
        round_trip(&ClientMessage::Command { id: 9, command: c });
    }
    round_trip(&ClientMessage::Hello {
        protocol_version: 1,
        client_name: Some("Booth".into()),
    });
    round_trip(&ClientMessage::Ping { id: 5 });
    for m in server_messages() {
        round_trip(&m);
    }
    round_trip(&sample_state());
    round_trip(&ShowState::default());
}

#[test]
fn json_shapes_match_the_readme() {
    // Client → server.
    let m: ClientMessage =
        serde_json::from_value(json!({"type":"command","id":1,"command":{"type":"next"}})).unwrap();
    assert_eq!(
        m,
        ClientMessage::Command {
            id: 1,
            command: Command::Next
        }
    );
    let p = PresentationId::new();
    let m: Command = serde_json::from_value(
        json!({"type":"triggerSlide","presentation":p.to_string(),"index":0}),
    )
    .unwrap();
    assert_eq!(
        m,
        Command::TriggerSlide {
            presentation: p,
            slide: None,
            index: Some(0),
            playlist_item: None,
            step: None,
            transition: None,
        }
    );
    let m: Command = serde_json::from_value(json!({"type":"setLook"})).unwrap();
    assert_eq!(m, Command::SetLook { look: None });
    let m: Command = serde_json::from_value(
        json!({"type":"setScreensEnabled","role":"audience","enabled":false}),
    )
    .unwrap();
    assert_eq!(
        m,
        Command::SetScreensEnabled {
            role: ScreenRole::Audience,
            enabled: false
        }
    );
    let m: Command =
        serde_json::from_value(json!({"type":"setOption","continuousPlaylist":false})).unwrap();
    assert_eq!(
        m,
        Command::SetOption {
            continuous_playlist: Some(false)
        }
    );
    let m: ClientMessage = serde_json::from_value(json!({"type":"ping","id":7})).unwrap();
    assert_eq!(m, ClientMessage::Ping { id: 7 });
    assert_eq!(
        serde_json::to_value(ServerMessage::Pong { id: 7 }).unwrap(),
        json!({"type":"pong","id":7})
    );
    assert_eq!(
        serde_json::to_value(ServerMessage::Notice {
            level: NoticeLevel::Info,
            message: "m".into(),
            key: None
        })
        .unwrap(),
        json!({"type":"notice","level":"info","message":"m"}),
        "no key: omitted"
    );

    // Server → client.
    assert_eq!(
        serde_json::to_value(ServerMessage::Result(CommandResult::ok(4, 12))).unwrap(),
        json!({"type":"result","id":4,"ok":true,"revision":12})
    );
    assert_eq!(
        serde_json::to_value(ServerMessage::Result(CommandResult::err(
            5,
            ApiError::new(ErrorCode::NotImplemented, "x")
        )))
        .unwrap(),
        json!({"type":"result","id":5,"ok":false,"error":{"code":"notImplemented","message":"x"}})
    );
    let st = serde_json::to_value(ServerMessage::State {
        revision: 1,
        state: ShowState::default(),
    })
    .unwrap();
    assert_eq!(st["type"], "state");
    // ShowState has a fixed shape: empty parts are null / [] rather than missing.
    assert_eq!(st["state"]["layers"]["slide"], Value::Null);
    assert_eq!(st["state"]["activeLook"], Value::Null);
    assert_eq!(st["state"]["cursor"], Value::Null);
    assert_eq!(st["state"]["timers"], json!([]));
    assert_eq!(
        st["state"]["screensEnabled"],
        json!({"audience": true, "stage": true})
    );
    assert_eq!(st["state"]["options"], json!({"continuousPlaylist": true}));
    let media = serde_json::to_value(sample_state().layers.media).unwrap();
    assert_eq!(media["playback"]["loop"], true, "`loop`, not `looping`");
    assert_eq!(ErrorCode::NotImplemented.http_status(), 501);
}

// ---- TypeScript agreement ---------------------------------------------------------------------

struct TsType {
    /// Body after `export type X = ` (comments removed).
    body: String,
    /// Types it imports.
    imports: Vec<String>,
}

fn bindings_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/bindings"))
}

fn load_ts(name: &str) -> TsType {
    let text = std::fs::read_to_string(bindings_dir().join(format!("{name}.ts")))
        .unwrap_or_else(|e| panic!("bindings/{name}.ts: {e}"));
    let mut imports = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("import type { ")
            && let Some(end) = rest.find(' ')
        {
            imports.push(rest[..end].to_string());
        }
    }
    let mut body = String::new();
    let mut rest = text.as_str();
    // strip /* */ comments
    while let Some(start) = rest.find("/*") {
        body.push_str(&rest[..start]);
        let end = rest[start..]
            .find("*/")
            .map(|e| start + e + 2)
            .unwrap_or(rest.len());
        rest = &rest[end..];
    }
    body.push_str(rest);
    let marker = format!("export type {name}");
    let at = body
        .find(&marker)
        .unwrap_or_else(|| panic!("{name}.ts has no `{marker}`"));
    let body = body[at + marker.len()..].to_string();
    TsType { body, imports }
}

/// Property names (with `?` flag) declared anywhere in a TS type body, and string literals.
fn props_and_literals(body: &str) -> (BTreeMap<String, bool>, BTreeSet<String>) {
    let mut props = BTreeMap::new();
    let mut lits = BTreeSet::new();
    let b: Vec<char> = body.chars().collect();
    let mut i = 0;
    let mut last_sig = ' ';
    while i < b.len() {
        let c = b[i];
        if c == '"' {
            let end = b[i + 1..]
                .iter()
                .position(|&x| x == '"')
                .map(|p| i + 1 + p)
                .unwrap_or(b.len());
            let s: String = b[i + 1..end].iter().collect();
            let mut j = end + 1;
            while j < b.len() && b[j].is_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == ':' && matches!(last_sig, '{' | ',') {
                props.insert(s, false);
            } else {
                lits.insert(s);
            }
            last_sig = '"';
            i = end + 1;
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let ident: String = b[start..i].iter().collect();
            let mut j = i;
            let optional = j < b.len() && b[j] == '?';
            if optional {
                j += 1;
            }
            if j < b.len() && b[j] == ':' && matches!(last_sig, '{' | ',') {
                props.insert(ident, optional);
            }
            last_sig = 'a';
            continue;
        }
        if !c.is_whitespace() {
            last_sig = c;
        }
        i += 1;
    }
    (props, lits)
}

/// Every property and literal reachable from a TS type (following imports).
fn reachable(name: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut seen = BTreeSet::new();
    let mut todo = vec![name.to_string()];
    let (mut props, mut lits) = (BTreeSet::new(), BTreeSet::new());
    while let Some(n) = todo.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        let t = load_ts(&n);
        let (p, l) = props_and_literals(&t.body);
        props.extend(p.into_keys());
        lits.extend(l);
        todo.extend(t.imports);
    }
    (props, lits)
}

fn json_keys(v: &Value, keys: &mut BTreeSet<String>, tags: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                keys.insert(k.clone());
                if k == "type"
                    && let Value::String(s) = x
                {
                    tags.insert(s.clone());
                }
                json_keys(x, keys, tags);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| json_keys(x, keys, tags)),
        _ => {}
    }
}

/// Checks a serialized sample against TS type `ts`: every key it contains is declared in the
/// type (or a type it reaches), every `type` tag is a declared literal, and, for object types,
/// every required top-level property is present.
fn agree<T: Serialize>(ts: &str, sample: &T) {
    let v = serde_json::to_value(sample).unwrap();
    let (props, lits) = reachable(ts);
    let (mut keys, mut tags) = (BTreeSet::new(), BTreeSet::new());
    json_keys(&v, &mut keys, &mut tags);
    let unknown: Vec<_> = keys.difference(&props).collect();
    assert!(
        unknown.is_empty(),
        "{ts}: JSON keys not in the TS bindings: {unknown:?}\n{v}"
    );
    let bad_tags: Vec<_> = tags.difference(&lits).collect();
    assert!(
        bad_tags.is_empty(),
        "{ts}: tags not in the TS bindings: {bad_tags:?}"
    );
    let body = load_ts(ts).body;
    let trimmed = body.trim_start_matches([' ', '=']).trim_start();
    if trimmed.starts_with('{')
        && !trimmed.contains("} |")
        && let Value::Object(m) = &v
    {
        // Top-level properties only: cut at the first nested `{`.
        let depth1: String = {
            let mut d = 0;
            trimmed
                .chars()
                .filter(|&c| {
                    match c {
                        '{' => d += 1,
                        '}' => d -= 1,
                        _ => {}
                    }
                    d <= 1 && c != '{' && c != '}'
                })
                .collect()
        };
        let (top, _) = props_and_literals(&format!("{{{depth1}}}"));
        for (p, optional) in top {
            if !optional {
                assert!(
                    m.contains_key(&p),
                    "{ts}: required TS field `{p}` missing in {v}"
                );
            }
        }
    }
}

#[test]
fn json_field_names_match_typescript() {
    for c in all_commands() {
        agree("Command", &c);
        agree(
            "ClientMessage",
            &ClientMessage::Command { id: 1, command: c },
        );
    }
    for m in server_messages() {
        agree("ServerMessage", &m);
    }
    agree("ShowState", &sample_state());
    agree("ShowState", &ShowState::default());
    agree(
        "MediaPlayback",
        &MediaPlayback {
            state: PlaybackState::Paused,
            position: 0.0,
            duration: None,
            looping: false,
            rate: 1.0,
            hw: false,
        },
    );
    agree(
        "ServerInfo",
        &ServerInfo {
            id: "x".into(),
            name: "n".into(),
            version: "v".into(),
            protocol_version: 1,
            capabilities: vec!["ws".into()],
            auth: AuthStatus::Local,
            pairing_code: Some("123456".into()),
            upgrade_credential: Some(true),
        },
    );
    agree(
        "PairResponse",
        &PairResponse {
            token: "t".into(),
            client_id: "c".into(),
        },
    );
    agree(
        "IssueCount",
        &IssueCount {
            what: "w".into(),
            detail: String::new(),
            count: 1,
        },
    );
    agree(
        "FontIssueInfo",
        &FontIssueInfo {
            family: "A".into(),
            weight: 400,
            italic: false,
            used_family: "B".into(),
            matched: "fallback".into(),
            synthetic_bold: false,
            synthetic_italic: false,
            uses: 1,
        },
    );
    let tree = PlaylistTreeNode {
        id: PlaylistId::new(),
        name: "Folder".into(),
        kind: PlaylistNodeType::Folder,
        purpose: None,
        item_count: 0,
        children: vec![PlaylistTreeNode {
            id: PlaylistId::new(),
            name: "Sunday".into(),
            kind: PlaylistNodeType::Playlist,
            purpose: Some(model::PlaylistPurpose::Service),
            item_count: 2,
            children: vec![],
        }],
    };
    agree("PlaylistTreeNode", &tree);
    let bins = MediaBinNode {
        id: None,
        name: "Motion".into(),
        path: vec!["Motion".into()],
        kind: MediaBinNodeType::Folder,
        browser: model::BinBrowser::Media,
        item_count: 3,
        children: vec![MediaBinNode {
            id: Some(model::MediaBinId::new()),
            name: "Loops".into(),
            path: vec!["Motion".into()],
            kind: MediaBinNodeType::Watched,
            browser: model::BinBrowser::Media,
            item_count: 3,
            children: vec![],
        }],
    };
    agree("MediaBinNode", &bins);
    let screen = model::Screen {
        name: "Side".into(),
        frame_rate: FrameRate::FPS25,
        enabled: false,
        ..model::Screen::default()
    };
    agree(
        "ScreenEntry",
        &ScreenEntry {
            revision: "abc".into(),
            screen: screen.clone(),
        },
    );
    let out = model::OutputBinding {
        name: "Side".into(),
        screen: screen.id,
        transport: model::OutputTransport::Idn {
            source_name: "Side".into(),
            groups: vec!["Booth".into()],
        },
        video_delay: 3,
        frame_rate_override: Some(FrameRate::FPS50),
        resolution_override: Some(model::Resolution {
            width: 1280,
            height: 720,
        }),
        alpha_override: Some(model::OutputAlpha::Opaque),
        ..model::OutputBinding::default()
    };
    let entry = OutputEntry {
        revision: "r".into(),
        output: out,
    };
    assert_eq!(round_trip(&entry)["videoDelay"], 3);
    agree("OutputEntry", &entry);
    agree(
        "OutputEntry",
        &OutputEntry {
            revision: "r".into(),
            output: model::OutputBinding::default(),
        },
    );
    agree(
        "ScreenDeleted",
        &ScreenDeleted {
            outputs: vec![model::OutputId::new()],
            looks: vec![],
        },
    );
    agree(
        "OutputTransportInfo",
        &OutputTransportInfo {
            kind: "sdi".into(),
            label: "SDI".into(),
            supported: false,
            available: false,
            version: None,
            status: "not yet supported".into(),
            detail: Some("later".into()),
        },
    );
    agree(
        "ErrorResponse",
        &ErrorResponse {
            error: ApiError::invalid("name", "a name is needed"),
            current: Some(json!(null)),
            codex: None,
        },
    );
    agree(
        "FontList",
        &FontList {
            families: vec![FontFamilyInfo {
                family: "Carlito".into(),
                faces: vec![FontFaceInfo {
                    weight: 700,
                    italic: false,
                    postscript_name: Some("Carlito-Bold".into()),
                }],
            }],
            substitutions: vec![FontSubstitutionInfo {
                family: Some("Calibri".into()),
                postscript_name: None,
                replacement_family: "Carlito".into(),
                installed: true,
            }],
            fallback_family: "DejaVu Sans".into(),
        },
    );
    agree(
        "MediaBinDetail",
        &MediaBinDetail {
            id: model::MediaBinId::new(),
            name: "Backgrounds".into(),
            path: vec![],
            kind: MediaBinNodeType::Manual,
            browser: model::BinBrowser::Media,
            items: vec![MediaBinItemInfo {
                id: model::MediaBinItemId::new(),
                name: "Clouds".into(),
                kind: MediaKind::Image,
                asset: MediaAssetId::new(),
                availability: MediaAvailability::Missing,
                enabled: true,
                duration: Some(12.5),
            }],
        },
    );
}

/// Model documents at their defaults omit every defaulted field: the TS must mark all of those
/// optional (this catches a missing `ts(optional)` annotation in `model`).
#[test]
fn model_defaults_agree_with_typescript() {
    use model::*;
    let mut p = Presentation::new_blank("Song");
    p.sections[0].slides.push(Slide::new());
    agree("Presentation", &p);
    agree("Slide", &Slide::new());
    agree("Section", &Section::new("V1"));
    agree("Library", &Library::default());
    agree("PlaylistNode", &PlaylistNode::default());
    agree(
        "PlaylistNode",
        &PlaylistNode {
            kind: PlaylistNodeKind::Folder,
            ..PlaylistNode::default()
        },
    );
    agree(
        "PlaylistItem",
        &PlaylistItem::new(PlaylistItemKind::Placeholder),
    );
    agree("Look", &Look::default());
    agree("ScreenLook", &ScreenLook::default());
    agree("Screen", &Screen::default());
    agree("OutputBinding", &OutputBinding::default());
    agree("ClearGroup", &ClearGroup::clear_all());
    agree("ClearTarget", &ClearTarget::default());
    agree("MediaAsset", &MediaAsset::default());
    agree("LayerVisibility", &LayerVisibility::default());
    agree("ImportInfo", &ImportInfo::default());
    agree("Transition", &Transition::CUT);
    agree("Rect", &Rect::default());
    agree("Color", &Color::default());
    agree("HotKey", &HotKey::default());
    agree("TextBox", &TextBox::default());
    agree("CharStyle", &CharStyle::default());
    agree(
        "Element",
        &Element::text(Rect::new(0.0, 0.0, 10.0, 10.0), TextBox::default()),
    );
}

#[test]
fn agreement_check_catches_mismatches() {
    let wrong_name = json!({"state":"paused","position":0.0,"duration":null,"looping":false,"rate":1.0,"hw":false});
    assert!(std::panic::catch_unwind(|| agree("MediaPlayback", &wrong_name)).is_err());
    let missing = json!({"state":"paused","position":0.0,"duration":null,"rate":1.0,"hw":false});
    assert!(std::panic::catch_unwind(|| agree("MediaPlayback", &missing)).is_err());
    let bad_tag = json!({"type":"nextSlide"});
    assert!(std::panic::catch_unwind(|| agree("Command", &bad_tag)).is_err());
    let ok =
        json!({"state":"paused","position":0.0,"duration":null,"loop":false,"rate":1.0,"hw":false});
    agree("MediaPlayback", &ok);
}

fn sample_site_event() -> SiteEvent {
    SiteEvent {
        name: "join_code_wrong".into(),
        level: SiteEventLevel::Warning,
        text: "a wrong join code from BOX (4 tries left)".into(),
        at: "2026-11-20T09:30:00Z".into(),
        fingerprint: Some("ab".repeat(32)),
        machine: Some("BOX".into()),
        address: Some("192.0.2.7".into()),
        request_id: Some(3),
        code_id: Some(1),
        tries_left: Some(4),
        // Free-form (IDN's fields); empty here so the key check sees only declared fields.
        detail: json!({}),
    }
}

#[test]
fn site_wire_shapes() {
    let v = round_trip(&ServerMessage::SiteEvent {
        event: sample_site_event(),
    });
    assert_eq!(v["type"], "siteEvent");
    assert_eq!(v["event"]["name"], "join_code_wrong");
    assert_eq!(v["event"]["triesLeft"], 4);
    let v = round_trip(&ServerMessage::SiteChanged {
        parts: vec![SitePart::JoinCodes],
    });
    assert_eq!(v, json!({"type": "siteChanged", "parts": ["joinCodes"]}));
    // Join modes, roles and event names keep IDN's spelling.
    for (m, s) in [
        (JoinMode::Approval, "approval"),
        (JoinMode::CodeOrApproval, "code_or_approval"),
        (JoinMode::CodeAndApproval, "code_and_approval"),
    ] {
        assert_eq!(round_trip(&m), json!(s));
        assert_eq!(m.as_str(), s);
    }
    assert_eq!(JoinMode::default(), JoinMode::CodeOrApproval);
    assert_eq!(round_trip(&SiteRole::Operator), json!("operator"));
    assert!(SiteRole::Viewer < SiteRole::Operator && SiteRole::Operator < SiteRole::Admin);
    assert_eq!(
        round_trip(&SiteEventName::SiteManagerUnreachable),
        json!("site_manager_unreachable")
    );
    let status = SiteStatus {
        enabled: true,
        running: true,
        error: None,
        api_error: None,
        settings_damaged: false,
        join_mode: JoinMode::CodeOrApproval,
        pairing_open: false,
        site_id: Some("0123456789abcdef".into()),
        ca_fingerprint: Some("ab".repeat(32)),
        discovery_port: Some(5959),
        api_port: Some(29202),
        api_certificate_sha256: Some("AB:CD".into()),
        you: SiteSession {
            access: SiteAccess::Pin,
            user: Some("admin".into()),
            role: Some(SiteRole::Admin),
            signed_in: false,
            unlocked: true,
            unlock_seconds_left: Some(1800),
        },
        revision: "0011223344556677".into(),
        membership: SiteMembership {
            state: MembershipState::WaitingForApproval,
            server: Some("192.0.2.10:5959".into()),
            site_id: Some("0123456789abcdef".into()),
            fingerprint: Some("cd".repeat(32)),
            code: Some("482 913".into()),
            connected: true,
            tries_left: Some(3),
            error: Some("e".into()),
        },
    };
    agree("SiteStatus", &status);
    assert_eq!(
        round_trip(&status)["membership"]["state"],
        "waitingForApproval"
    );
    assert!(round_trip(&status).get("apiError").is_none());
    assert!(round_trip(&status).get("settingsDamaged").is_none());
    let damaged = SiteStatus {
        settings_damaged: true,
        ..status.clone()
    };
    agree("SiteStatus", &damaged);
    assert_eq!(round_trip(&damaged)["settingsDamaged"], true);
    let without_api = SiteStatus {
        api_port: None,
        api_certificate_sha256: None,
        api_error: Some("port in use (API port 29202)".into()),
        ..status.clone()
    };
    agree("SiteStatus", &without_api);
    assert_eq!(
        round_trip(&without_api)["apiError"],
        "port in use (API port 29202)"
    );
    agree(
        "JoinSite",
        &JoinSite {
            server: Some("192.0.2.10".into()),
            code: Some("482913".into()),
        },
    );
    assert_eq!(round_trip(&SitePart::Membership), json!("membership"));
    agree(
        "PairingRequest",
        &PairingRequest {
            id: "k3".into(),
            client: "Oriel – Booth laptop".into(),
            via: "BOX-B (Vesper)".into(),
            via_fingerprint: Some("ab".repeat(32)),
            address: Some("192.0.2.7".into()),
            relayed: true,
            asked_at: "2026-11-20T09:30:00Z".into(),
            launcher: None,
        },
    );
    agree(
        "PairingRequest",
        &PairingRequest {
            id: "p4".into(),
            client: "Vesper Launcher – Chapel Box".into(),
            via: "Main (Vesper)".into(),
            via_fingerprint: None,
            address: Some("192.0.2.8".into()),
            relayed: false,
            asked_at: "2026-11-20T09:31:00Z".into(),
            launcher: Some("6d6f636b-0000-4000-8000-000000000001".into()),
        },
    );
    agree(
        "DenyPairing",
        &DenyPairing {
            reason: Some("not now".into()),
        },
    );
    agree(
        "SiteApiKey",
        &SiteApiKey {
            id: "ab12".into(),
            name: "Oriel – Booth laptop".into(),
            engine: "app:vesper".into(),
            machine: "*".into(),
            scope: "control".into(),
            created_at: "2026-11-20T09:30:00Z".into(),
            by: "admin (via BOX-B (Vesper))".into(),
            expires_at: "2036-11-18T09:30:00Z".into(),
            revoked: true,
            revoked_at: Some("2026-11-21T09:30:00Z".into()),
            last_used: Some("2026-11-20T10:00:00Z".into()),
            last_machine: Some("cd".repeat(32)),
        },
    );
    agree(
        "PairPending",
        &PairPending {
            pairing_id: "0123456789abcdef".into(),
            expires_in_seconds: 600,
            code_wrong: true,
        },
    );
    agree(
        "UpgradeCredential",
        &UpgradeCredential {
            browser: Some(true),
        },
    );
    agree(
        "PairRequest",
        &PairRequest {
            code: Some("482913".into()),
            client_name: "Booth laptop".into(),
            browser: Some(true),
            launcher: None,
        },
    );
    agree(
        "PairRequest",
        &PairRequest {
            code: None,
            client_name: "Chapel Box".into(),
            browser: None,
            launcher: Some("6d6f636b-0000-4000-8000-000000000001".into()),
        },
    );
    assert_eq!(
        serde_json::from_value::<PairRequest>(json!({"clientName": "x"}))
            .unwrap()
            .code,
        None,
        "no code: waits for approval"
    );
    for p in [SitePart::Pairings, SitePart::ApiKeys] {
        round_trip(&p);
    }
    let v = round_trip(&status);
    assert_eq!(v["joinMode"], "code_or_approval");
    assert_eq!(v["you"]["access"], "pin");
    assert_eq!(v["you"]["unlockSecondsLeft"], 1800);
    // Access levels; open is the default.
    assert_eq!(SiteAccess::default(), SiteAccess::Open);
    for (a, s) in [
        (SiteAccess::Open, "open"),
        (SiteAccess::Pin, "pin"),
        (SiteAccess::Users, "users"),
    ] {
        assert_eq!(round_trip(&a), json!(s));
        assert_eq!(a.as_str(), s);
    }
    // The PIN goes in (requests) but never out: responses carry `pinSet`.
    let settings: SiteSettings = serde_json::from_value(
        json!({"enabled": true, "joinMode": "approval", "access": "pin", "pin": "4321"}),
    )
    .unwrap();
    assert_eq!(settings.pin.as_deref(), Some("4321"));
    assert!(!settings.pin_set);
    let shown = SiteSettings {
        pin: None,
        pin_set: true,
        revision: Some("0011".into()),
        ..settings
    };
    agree("SiteSettings", &shown);
    let v = serde_json::to_value(&shown).unwrap();
    assert!(v.get("pin").is_none(), "{v}");
    assert_eq!(v["pinSet"], true);
    assert!(
        serde_json::from_value::<SiteSettings>(json!({"enabled": true, "joinMode": "approval"}))
            .is_err(),
        "access is part of the document"
    );
    agree("SiteUnlock", &SiteUnlock { pin: "1234".into() });
    agree(
        "SiteDevice",
        &SiteDevice {
            fingerprint: "ab".repeat(32),
            name: "BOX".into(),
            kind: "app".into(),
            admin: false,
            health: DeviceHealth::NeedsAttention,
            reason: Some("clock not locked".into()),
            problems: vec![DeviceProblem {
                problem: "clock_unlocked".into(),
                level: SiteEventLevel::Warning,
                text: "clock not locked".into(),
                // Free-form (IDN's fields); empty so the key check sees only declared fields.
                fields: json!({}),
            }],
            last_seen: Some("2026-11-20T09:30:00Z".into()),
            version: Some("0.2.0-Alpha93".into()),
            restarting: vec!["display".into()],
            services: vec![DeviceService {
                id: "display".into(),
                name: "Display".into(),
                state: "restarting".into(),
            }],
            updating: Some("staged".into()),
            update_managed_by: Some("Vesper".into()),
            node: true,
            node_started_at: Some("2026-11-20T08:00:00Z".into()),
            machine: Some(SiteDeviceMachine {
                cpu: Some("Example CPU 8-Core".into()),
                cpus: Some(16),
                gpus: vec!["Example GPU".into()],
                os: Some("Ubuntu".into()),
                os_version: Some("24.04".into()),
                kernel: Some("6.8.0".into()),
                booted_at: Some("2026-11-19T07:00:00Z".into()),
                memory_total: Some(32_000_000_000),
                memory_free: Some(20_000_000_000),
                reported_at: Some("2026-11-20T09:29:50Z".into()),
            }),
            built_in: true,
            relay: true,
        },
    );
    agree("SetDeviceRelay", &SetDeviceRelay { on: true });
    let d: SiteDevice = serde_json::from_value(json!({
        "fingerprint": "ab".repeat(32), "name": "BOX", "kind": "app", "admin": false,
        "health": "online", "problems": [], "restarting": [], "node": false,
    }))
    .unwrap();
    assert!(
        !d.built_in,
        "a machine of the site is not built in by default"
    );
    assert!(
        !d.relay,
        "a machine of the site relays no pairings by default"
    );
    assert!(d.services.is_empty(), "services default to none");
    assert!(
        d.machine.is_none() && d.node_started_at.is_none(),
        "no machine details from an older Node"
    );
    assert!(
        serde_json::to_value(&d).unwrap().get("builtIn").is_none(),
        "builtIn is left out when false"
    );
    assert_eq!(round_trip(&DeviceHealth::Updating), json!("updating"));
    agree(
        "JoinRequestInfo",
        &JoinRequestInfo {
            id: 1,
            name: "BOX".into(),
            kind: "app".into(),
            fingerprint: "ab".repeat(32),
            code: Some("482 913".into()),
            address: Some("192.0.2.7".into()),
            by_code: false,
            asked_at: "2026-11-20T09:30:00Z".into(),
        },
    );
    agree(
        "JoinCodeInfo",
        &JoinCodeInfo {
            id: 1,
            code: Some("482 913".into()),
            expires_at: "2026-11-20T09:40:00Z".into(),
            expires_in_seconds: 600,
            tries_left: 5,
            issuer: "admin".into(),
        },
    );
    agree(
        "IssueToken",
        &IssueToken {
            engine: "desk".into(),
            machine: None,
            scope: "iem:3".into(),
            ttl_seconds: Some(3600),
        },
    );
}
