//! Regression tests for JSON hygiene: exact float round trips, compact defaults, required IDs,
//! finite gains, and the shapes added for audio input bindings, bin folders and CCLI layout.

use model::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fmt::Debug;

/// Round trip through JSON **text** (not `Value`), as storage does.
fn text_round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) -> String {
    let s = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&s).unwrap();
    assert_eq!(&back, value, "{s}");
    s
}

#[test]
fn geometry_round_trips_bit_exact() {
    for x in [
        100.50000000000001,
        1622.3980000000001,
        1470.3230000000005,
        3822.3590000000004,
    ] {
        let r = Rect::new(x, 0.0, 0.0, 0.0);
        let s = serde_json::to_string(&r).unwrap();
        let back: Rect = serde_json::from_str(&s).unwrap();
        assert_eq!(back.x.to_bits(), x.to_bits(), "{s}");
    }
    let el = Element::new(Rect::new(1622.3980000000001, 0.0, 10.0, 10.0));
    text_round_trip(&el);

    // Verbatim payload numbers inside a slide survive too.
    let mut slide = Slide::new();
    slide
        .actions
        .push(SlideAction::new(ActionKind::Unsupported {
            source_type: "S".into(),
            payload: json!({"num": 100.50000000000001}),
        }));
    text_round_trip(&slide);
}

#[test]
fn empty_payloads_and_default_anchors_are_omitted() {
    let kind = ActionKind::Unsupported {
        source_type: "S".into(),
        payload: Value::Null,
    };
    let s = text_round_trip(&kind);
    assert_eq!(s, r#"{"type":"unsupported","sourceType":"S"}"#);

    let fill = Fill::enabled(FillKind::Unsupported {
        source_type: "f".into(),
        payload: Value::Null,
    });
    assert!(!text_round_trip(&fill).contains("payload"));

    assert_eq!(serde_json::to_string(&PathPoint::default()).unwrap(), "{}");
    let p: PathPoint = serde_json::from_str("{}").unwrap();
    assert_eq!(p, PathPoint::default());
}

#[test]
fn watched_folder_requires_its_root() {
    assert!(serde_json::from_str::<WatchedFolder>("{}").is_err());
    assert!(serde_json::from_value::<WatchedFolder>(json!({"path": "Loops"})).is_err());
    let bin = json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "kind": {"type": "watched", "folder": {"path": "Loops"}}
    });
    assert!(serde_json::from_value::<MediaBin>(bin).is_err());

    let folder = WatchedFolder {
        root: MediaRootId::new(),
        path: String::new(),
        recursive: false,
    };
    let v = serde_json::to_value(&folder).unwrap();
    assert_eq!(v, json!({"root": folder.root.to_string()}));
    assert_eq!(serde_json::from_value::<WatchedFolder>(v).unwrap(), folder);
}

#[test]
fn audio_settings_can_say_no_audio_and_stay_finite() {
    let none = VideoInput {
        name: "Captions".into(),
        audio: AudioSettings {
            muted: true,
            ..AudioSettings::default()
        },
        ..VideoInput::default()
    };
    let s = text_round_trip(&none);
    assert!(s.contains(r#""audio":{"muted":true}"#), "{s}");

    // A -inf gain never writes `null` (which would fail to load).
    let silent = AudioSettings {
        volume_db: f64::NEG_INFINITY,
        channels: vec![ChannelRoute {
            gain_db: f64::NAN,
            ..ChannelRoute::default()
        }],
        ..AudioSettings::default()
    };
    let s = serde_json::to_string(&silent).unwrap();
    assert!(!s.contains("null"), "{s}");
    let back: AudioSettings = serde_json::from_str(&s).unwrap();
    assert_eq!(back.volume_db, SILENCE_DB);
    assert_eq!(back.channels[0].gain_db, SILENCE_DB);

    // A `null` gain from an older writer reads as silence.
    let old: AudioSettings = serde_json::from_str(r#"{"volumeDb":null}"#).unwrap();
    assert_eq!(old.volume_db, SILENCE_DB);
    let cfg = AudioDeviceConfig {
        master_gain_db: f64::NEG_INFINITY,
        ..AudioDeviceConfig::default()
    };
    let back: AudioDeviceConfig =
        serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
    assert_eq!(back.master_gain_db, SILENCE_DB);
    // Finite gains are unchanged.
    let quiet = AudioSettings {
        volume_db: -12.5,
        ..AudioSettings::default()
    };
    text_round_trip(&quiet);
}

#[test]
fn audio_inputs_bind_to_a_machine_source() {
    let input = AudioInput {
        name: "Program feed".into(),
        ..AudioInput::default()
    };
    let binding = AudioInputBinding {
        input: input.id,
        source: InputSource::Idn {
            source_name: "HOST (Mix Output)".into(),
        },
        ..AudioInputBinding::default()
    };
    let s = text_round_trip(&binding);
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["input"], json!(input.id.to_string()));
    assert_eq!(v["source"]["type"], json!("idn"));
    assert_eq!(binding.id(), input.id.0);
    assert_eq!(AudioInputBinding::KIND, DocumentKind::AudioInputBinding);
    assert_eq!(
        serde_json::to_value(DocumentKind::AudioInputBinding).unwrap(),
        json!("audioInputBinding")
    );
}

#[test]
fn media_bins_keep_their_folder() {
    let bin = |folder: &str| MediaBin {
        name: "Scenery".into(),
        folder: vec![folder.into()],
        import: Some(ImportInfo {
            original_name: Some("Scenery".into()),
            ..ImportInfo::default()
        }),
        ..MediaBin::default()
    };
    let stills = bin("Stills");
    let loops = bin("Loops");
    let v: Value = serde_json::from_str(&text_round_trip(&stills)).unwrap();
    assert_eq!(v["folder"], json!(["Stills"]));
    assert_ne!(stills.folder, loops.folder, "same name, different folder");
    text_round_trip(&loops);
}

#[test]
fn ccli_template_is_an_inline_slide_with_a_token_layout() {
    let mut line = Element::text(Rect::new(240.0, 135.0, 1440.0, 810.0), TextBox::plain(""));
    line.data_link = Some(DataLink::CcliText);
    let ccli = CcliSettings {
        license_number: Some("12345".into()),
        enabled: true,
        show_on: CcliDisplay::FirstAndLast,
        layout: vec![
            CcliToken::Title,
            CcliToken::Text { text: " / ".into() },
            CcliToken::Author,
            CcliToken::CopyrightYear,
            CcliToken::LicenseNumber,
        ],
        template: Some(Slide {
            background: Some(Color::BLACK),
            elements: vec![line],
            ..Slide::new()
        }),
    };
    let s = text_round_trip(&ccli);
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["layout"][0], json!({"type": "title"}));
    assert_eq!(v["layout"][1], json!({"type": "text", "text": " / "}));
    assert_eq!(
        v["template"]["elements"][0]["dataLink"]["type"],
        json!("ccliText")
    );
    let unknown: CcliToken = serde_json::from_value(json!({"type": "albumArt"})).unwrap();
    assert_eq!(unknown, CcliToken::Unknown);
}

#[test]
fn integer_params_are_32_bit() {
    let p = ParamValue::Integer { value: i32::MAX };
    text_round_trip(&p);
    assert!(
        serde_json::from_value::<ParamValue>(json!({"type": "integer", "value": 1i64 << 40}))
            .is_err()
    );
}
