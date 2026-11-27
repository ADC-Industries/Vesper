//! Rust `Default` and serde defaults agree for every document and value type: the default
//! serializes to minimal JSON and that JSON (or `{}`) deserializes back to the default.

use model::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fmt::Debug;

/// `T::default()` writes only `schemaVersion` plus the listed required keys, and decodes back
/// (also without `schemaVersion`, which means version 1).
fn check_doc<T: Document + Default + PartialEq + Debug>(required: &[&str]) {
    let d = T::default();
    assert_eq!(d.schema_version(), SCHEMA_VERSION, "{:?}", T::KIND);
    let v = serde_json::to_value(&d).unwrap();
    let obj = v.as_object().expect("documents are objects");
    for k in obj.keys() {
        assert!(
            k == "schemaVersion" || required.contains(&k.as_str()),
            "{:?}::default() wrote non-default field `{k}`: {v}",
            T::KIND
        );
    }
    for k in required {
        assert!(obj.contains_key(*k), "{:?} must write `{k}`", T::KIND);
    }
    assert_eq!(
        obj["schemaVersion"],
        json!(SCHEMA_VERSION),
        "schemaVersion is always written"
    );
    let back: T = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(back, d, "{:?} round trip", T::KIND);

    let mut without_version = v.clone();
    without_version
        .as_object_mut()
        .unwrap()
        .remove("schemaVersion");
    let back: T = decode(without_version).unwrap();
    assert_eq!(back, d, "{:?}: missing schemaVersion = 1", T::KIND);
    assert_eq!(back.id(), d.id());
}

/// `T::default()` serializes to `{}` and `{}` deserializes to `T::default()`.
fn check_value<T: Default + Serialize + DeserializeOwned + PartialEq + Debug>(name: &str) {
    let d = T::default();
    assert_eq!(
        serde_json::to_value(&d).unwrap(),
        json!({}),
        "{name}::default() is not {{}}"
    );
    let back: T = serde_json::from_value(json!({})).unwrap();
    assert_eq!(back, d, "{{}} does not decode to {name}::default()");
}

/// Like [`check_value`] for value types that carry an ID: only `id` is written.
fn check_identified<T: Default + Serialize + DeserializeOwned + PartialEq + Debug>(name: &str) {
    let d = T::default();
    let v = serde_json::to_value(&d).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["id"], "{name}::default() wrote {v}");
    let back: T = serde_json::from_value(v).unwrap();
    assert_eq!(back, d);
}

#[test]
fn every_document_type_defaults_to_minimal_json() {
    check_doc::<Library>(&["id"]);
    check_doc::<Presentation>(&["id"]);
    check_doc::<GlobalGroup>(&["id"]);
    check_doc::<PlaylistNode>(&["id"]);
    check_doc::<MediaBin>(&["id"]);
    check_doc::<MediaAsset>(&["id"]);
    check_doc::<MediaRoot>(&["id"]);
    check_doc::<Theme>(&["id"]);
    check_doc::<Screen>(&["id"]);
    check_doc::<Look>(&["id"]);
    check_doc::<Mask>(&["id", "slide"]);
    check_doc::<VideoInput>(&["id"]);
    check_doc::<AudioInput>(&["id"]);
    check_doc::<Timer>(&["id"]);
    check_doc::<StageLayout>(&["id", "slide"]);
    check_doc::<ClearGroup>(&["id"]);
    check_doc::<Prop>(&["id", "slide"]);
    check_doc::<Message>(&["id"]);
    check_doc::<Macro>(&["id"]);
    check_doc::<ShowSettings>(&[]);
    check_doc::<OutputBinding>(&["id", "screen"]);
    check_doc::<InputBinding>(&["input"]);
    check_doc::<AudioInputBinding>(&["input"]);
    check_doc::<MediaRootBinding>(&["root"]);
    check_doc::<AudioDeviceConfig>(&[]);
    check_doc::<FontSubstitutionTable>(&[]);
    check_doc::<CaptionsConfig>(&[]);
    check_doc::<CodexInfo>(&["id"]);
    check_doc::<Collection>(&["id"]);
    check_doc::<CodexConfig>(&["role"]);
}

#[test]
fn singletons_decode_from_empty_object() {
    // Stored documents always go through `decode` (migration): `{}` is a schema-1 document.
    let empty = || serde_json::json!({});
    assert_eq!(
        decode::<ShowSettings>(empty()).unwrap(),
        ShowSettings::default()
    );
    assert_eq!(
        decode::<AudioDeviceConfig>(empty()).unwrap(),
        AudioDeviceConfig::default()
    );
    assert_eq!(
        decode::<FontSubstitutionTable>(empty()).unwrap(),
        FontSubstitutionTable::default()
    );
}

#[test]
fn value_types_default_to_empty_object() {
    check_value::<LayerVisibility>("LayerVisibility");
    check_value::<LayerTransitions>("LayerTransitions");
    check_value::<Fill>("Fill");
    check_value::<Stroke>("Stroke");
    check_value::<Shadow>("Shadow");
    check_value::<Feather>("Feather");
    check_value::<LineMask>("LineMask");
    check_value::<TextOutline>("TextOutline");
    check_value::<TextBox>("TextBox");
    check_value::<Paragraph>("Paragraph");
    check_value::<TextRun>("TextRun");
    check_value::<RichText>("RichText");
    check_value::<CharStyle>("CharStyle");
    check_value::<ParagraphStyle>("ParagraphStyle");
    check_value::<Gradient>("Gradient");
    check_value::<ShapePath>("ShapePath");
    check_value::<Playback>("Playback");
    check_value::<Fade>("Fade");
    check_value::<AudioSettings>("AudioSettings");
    check_value::<ChannelRoute>("ChannelRoute");
    check_value::<MediaDisplay>("MediaDisplay");
    check_value::<MediaOrigin>("MediaOrigin");
    check_value::<MediaProbe>("MediaProbe");
    check_value::<VolumeHint>("VolumeHint");
    check_value::<TriggerDefaults>("TriggerDefaults");
    check_value::<Transition>("Transition");
    check_value::<VendorEffect>("VendorEffect");
    check_value::<TimerFormat>("TimerFormat");
    check_value::<ClockFormat>("ClockFormat");
    check_value::<CcliSettings>("CcliSettings");
    check_value::<ClearTarget>("ClearTarget");
    check_value::<ImportInfo>("ImportInfo");
    check_value::<HotKey>("HotKey");
    check_value::<SlideLabel>("SlideLabel");
    check_value::<SongInfo>("SongInfo");
    check_value::<AutoAdvance>("AutoAdvance");
    check_value::<BibleBook>("BibleBook");
    check_value::<EdgeInsets>("EdgeInsets");
    check_value::<Flip>("Flip");
    check_value::<Rect>("Rect");
    check_value::<Point>("Point");
    check_value::<DisplaySelector>("DisplaySelector");
    check_value::<FontSubstitution>("FontSubstitution");
    check_value::<Placeholder>("Placeholder");
}

#[test]
fn identified_value_types_default_to_id_only() {
    check_identified::<Slide>("Slide");
    check_identified::<Element>("Element");
    check_identified::<Section>("Section");
    check_identified::<Arrangement>("Arrangement");
    check_identified::<Guide>("Guide");
}

#[test]
fn layer_visibility_empty_object_means_all_visible() {
    let v: LayerVisibility = serde_json::from_str("{}").unwrap();
    assert_eq!(v, LayerVisibility::ALL);
    for layer in Layer::COMPOSITE_ORDER {
        assert!(v.contains(layer), "{layer:?} must be visible");
    }
    // Only hidden layers are written, so a layer added later appears on old looks.
    let mut v = LayerVisibility::ALL;
    v.set(Layer::Props, false);
    assert_eq!(serde_json::to_value(v).unwrap(), json!({"props": false}));
    let back: LayerVisibility = serde_json::from_value(json!({"props": false})).unwrap();
    assert!(!back.props && back.slide && back.messages && back.video_input);
    // A screen look without `layers` is all visible too.
    let sl: ScreenLook =
        serde_json::from_value(json!({"screen": "00000000-0000-0000-0000-000000000001"})).unwrap();
    assert_eq!(sl.layers, LayerVisibility::ALL);
}

#[test]
fn machine_documents_are_not_portable() {
    fn portable<T: Document>() -> bool {
        T::portable()
    }
    assert!(!portable::<OutputBinding>());
    assert!(!portable::<InputBinding>());
    assert!(!portable::<AudioInputBinding>());
    assert!(!portable::<MediaRootBinding>());
    assert!(!portable::<AudioDeviceConfig>());
    assert!(!portable::<FontSubstitutionTable>());
    assert!(portable::<Screen>() && portable::<Presentation>() && portable::<ShowSettings>());
}

#[test]
fn document_kinds_serialize_camel_case() {
    assert_eq!(
        serde_json::to_value(DocumentKind::PlaylistNode).unwrap(),
        json!("playlistNode")
    );
    let k: DocumentKind = serde_json::from_value(json!("somethingNew")).unwrap();
    assert_eq!(k, DocumentKind::Unknown);
    let _: Value = serde_json::to_value(DocumentKind::FontSubstitutionTable).unwrap();
}
