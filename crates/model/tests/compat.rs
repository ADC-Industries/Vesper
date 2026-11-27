//! Schema safety and forward compatibility: migrations, the TooNew rule, `Unsupported`
//! payloads that survive round trips, and `Unknown` fallbacks.

use model::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fmt::Debug;

#[test]
fn migrate_refuses_newer_documents() {
    let mut doc = json!({"schemaVersion": NEWEST_SCHEMA_VERSION + 1, "id": PresentationId::new()});
    let err = migrate(DocumentKind::Presentation, &mut doc).unwrap_err();
    assert_eq!(
        err,
        MigrationError::TooNew {
            found: NEWEST_SCHEMA_VERSION + 1,
            supported: NEWEST_SCHEMA_VERSION
        }
    );
    assert!(err.to_string().contains("refusing"));
    assert!(matches!(
        ensure_writable(&doc),
        Err(MigrationError::TooNew { .. })
    ));
    assert!(matches!(
        decode::<Presentation>(doc),
        Err(DecodeError::Migration(MigrationError::TooNew { .. }))
    ));
}

#[test]
fn migrate_current_and_unversioned_documents() {
    let mut current = serde_json::to_value(Presentation::new("x")).unwrap();
    let before = current.clone();
    migrate(DocumentKind::Presentation, &mut current).unwrap();
    assert_eq!(current, before);
    ensure_writable(&current).unwrap();

    let mut legacy = json!({"id": TimerId::new(), "name": "Countdown"});
    migrate(DocumentKind::Timer, &mut legacy).unwrap();
    assert_eq!(legacy["schemaVersion"], json!(SCHEMA_VERSION));

    for bad in [
        json!({"schemaVersion": 0}),
        json!({"schemaVersion": "1"}),
        json!({"schemaVersion": -1}),
    ] {
        let mut bad = bad;
        assert_eq!(
            migrate(DocumentKind::Timer, &mut bad),
            Err(MigrationError::InvalidVersion)
        );
    }
    assert_eq!(
        migrate(DocumentKind::Timer, &mut json!([])),
        Err(MigrationError::NotAnObject)
    );
    assert_eq!(
        migrate(DocumentKind::Unknown, &mut json!({"schemaVersion": 1})),
        Err(MigrationError::UnknownKind)
    );
}

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) -> Value {
    let v = serde_json::to_value(value).unwrap();
    let back: T = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(&back, value);
    v
}

#[test]
fn unsupported_payloads_round_trip() {
    let payload = json!({"communication": {"deviceUuid": "abc", "commands": [1, 2, 3]}, "x": null});

    let action = SlideAction::new(ActionKind::Unsupported {
        source_type: "ACTION_TYPE_COMMUNICATION".into(),
        payload: payload.clone(),
    });
    let v = round_trip(&action);
    assert_eq!(
        v["kind"],
        json!({"type": "unsupported", "sourceType": "ACTION_TYPE_COMMUNICATION", "payload": payload})
    );

    round_trip(&Fill::enabled(FillKind::Unsupported {
        source_type: "fill.x".into(),
        payload: payload.clone(),
    }));
    round_trip(&Effect::new(EffectKind::Unsupported {
        source_type: "fx".into(),
        payload: payload.clone(),
    }));
    round_trip(&Transition {
        kind: TransitionKind::Unsupported {
            source_type: "tr".into(),
            payload: payload.clone(),
        },
        duration: 1.5,
        ..Default::default()
    });
    round_trip(&DataLink::Unsupported {
        source_type: "rssFeed".into(),
        payload: payload.clone(),
    });
    round_trip(&VisibilityCondition::Unsupported {
        source_type: "cond".into(),
        payload: payload.clone(),
    });
    round_trip(&PlaylistItem::new(PlaylistItemKind::Unsupported {
        source_type: "planningCenter".into(),
        payload: payload.clone(),
    }));
    round_trip(&MessageTokenKind::Unsupported {
        source_type: "tok".into(),
        payload: payload.clone(),
    });

    // Vendor effects keep every parameter and render as a dissolve.
    let warp = Transition {
        kind: TransitionKind::Vendor(VendorEffect {
            name: "Color Warp".into(),
            vendor_id: Some("00000000-0000-0000-0000-00000000abcd".into()),
            category: Some("Color".into()),
            params: vec![
                EffectParam {
                    name: "Zoom".into(),
                    value: ParamValue::Number { value: 0.25 },
                },
                EffectParam {
                    name: "Direction".into(),
                    value: ParamValue::Direction {
                        value: Direction::Left,
                    },
                },
            ],
        }),
        duration: 1.0,
        ..Default::default()
    };
    let v = round_trip(&warp);
    assert_eq!(v["kind"]["type"], json!("vendor"));
    assert_eq!(v["kind"]["name"], json!("Color Warp"));
    assert_eq!(warp.renderable(), Transition::dissolve(1.0));

    // Import provenance keeps unmapped source data verbatim.
    let el = Element {
        import: Some(ImportInfo {
            unmapped: json!({"info": 7, "buildIn": {"duration": 1}}),
            ..ImportInfo::default()
        }),
        ..Element::default()
    };
    let v = round_trip(&el);
    assert_eq!(
        v["import"],
        json!({"unmapped": {"info": 7, "buildIn": {"duration": 1}}})
    );
}

#[test]
fn unknown_tags_fall_back() {
    let a: ActionKind = serde_json::from_value(json!({"type": "teleport", "target": 1})).unwrap();
    assert_eq!(a, ActionKind::Unknown);
    let k: TransitionKind = serde_json::from_value(json!({"type": "spin"})).unwrap();
    assert_eq!(k, TransitionKind::Unknown);
    assert_eq!(
        Transition {
            kind: k,
            duration: 0.7,
            ..Default::default()
        }
        .renderable(),
        Transition::dissolve(0.7)
    );
    let l: Layer = serde_json::from_value(json!("hologram")).unwrap();
    assert_eq!(l, Layer::Unknown);
    let s: MediaStatus = serde_json::from_value(json!("quantum")).unwrap();
    assert_eq!(s, MediaStatus::Unknown);
    let loc: AssetLocation = serde_json::from_value(json!({"type": "ipfs", "cid": "x"})).unwrap();
    assert_eq!(loc, AssetLocation::Unknown);
}

#[test]
fn media_asset_relink_state_round_trips() {
    let root = MediaRoot {
        name: "Shared backgrounds".into(),
        hints: vec![
            VolumeHint {
                drive_letter: Some("X".into()),
                volume_name: Some("Shared Drive".into()),
                ..VolumeHint::default()
            },
            VolumeHint {
                unc_prefix: Some("\\\\nas\\Media".into()),
                ..VolumeHint::default()
            },
        ],
        ..MediaRoot::default()
    };
    let path = "Loops/Sunrise.mp4";
    let asset = MediaAsset {
        id: MediaAssetId::for_linked(root.id, path),
        kind: MediaKind::Video,
        name: "Sunrise".into(),
        location: AssetLocation::Linked {
            root: root.id,
            path: path.into(),
        },
        origin: Some(MediaOrigin {
            platform: SourcePlatform::Windows,
            absolute_hint: Some("X:\\Loops\\Sunrise.mp4".into()),
            root_kind: Some("ROOT_SHOW".into()),
            relative_path: Some(path.into()),
            drive_letter: Some("X".into()),
            ..MediaOrigin::default()
        }),
        status: MediaStatus::Missing,
        checked_at: Some("2026-10-18T09:30:00Z".into()),
        relinks: vec![RelinkRecord {
            method: RelinkMethod::Manual,
            previous: AssetLocation::Unresolved,
            at: None,
        }],
        probe: Some(MediaProbe {
            width: Some(1920),
            height: Some(1080),
            duration: Some(12.5),
            frame_rate: Some(FrameRate::FPS29_97),
            from_import_hints: true,
            ..MediaProbe::default()
        }),
        ..MediaAsset::default()
    };
    let v = round_trip(&asset);
    assert_eq!(v["status"], json!("missing"));
    assert_eq!(
        v["location"],
        json!({"type": "linked", "root": root.id.to_string(), "path": path})
    );
    round_trip(&root);

    // A watched bin caches its last scan with deterministic item IDs.
    let bin_id = MediaBinId::new();
    let defaults = TriggerDefaults {
        playback: Playback {
            repeat: Repeat::Loop,
            ..Playback::default()
        },
        ..TriggerDefaults::default()
    };
    let bin = MediaBin {
        id: bin_id,
        name: "Loops".into(),
        kind: MediaBinKind::Watched {
            folder: WatchedFolder {
                root: root.id,
                path: "Loops".into(),
                recursive: false,
            },
            last_scan: vec![MediaBinItem {
                id: MediaBinItemId::for_watched(bin_id, "Sunrise.mp4"),
                name: None,
                enabled: true,
                trigger: defaults.trigger(asset.id),
            }],
            import_behavior: defaults,
            scanned_at: None,
        },
        ..MediaBin::default()
    };
    round_trip(&bin);
}

#[test]
fn video_input_items_cannot_target_other_layers() {
    let trigger = VideoInputTrigger {
        input: Link::named(VideoInputId::new(), "Camera 1"),
        display: MediaDisplay::default(),
        audio: AudioSettings {
            volume_db: -12.0,
            channels: vec![ChannelRoute {
                source: 2,
                dest: 0,
                gain_db: -9.0,
                muted: true,
            }],
            ..AudioSettings::default()
        },
        transition: None,
    };
    assert_eq!(trigger.layer(), Layer::VideoInput);
    let item = PlaylistItem::new(PlaylistItemKind::VideoInput(trigger));
    let v = round_trip(&item);
    assert_eq!(v["kind"]["type"], json!("videoInput"));
    assert!(v["kind"].get("layer").is_none());
}

#[test]
fn idn_transport_round_trips_and_unknown_transports_fall_back() {
    let t = OutputTransport::Idn {
        source_name: "Lyrics".into(),
        groups: vec![],
    };
    assert_eq!(
        round_trip(&t),
        json!({"type": "idn", "sourceName": "Lyrics"})
    );
    let g: OutputTransport =
        serde_json::from_value(json!({"type": "idn", "sourceName": "Key", "groups": ["booth"]}))
            .unwrap();
    assert_eq!(
        g,
        OutputTransport::Idn {
            source_name: "Key".into(),
            groups: vec!["booth".into()]
        }
    );
    // A transport from a newer build leaves the output off rather than failing the binding.
    let u: OutputTransport =
        serde_json::from_value(json!({"type": "srt", "url": "srt://x"})).unwrap();
    assert_eq!(u, OutputTransport::Unknown);
}

#[test]
fn schema_1_ndi_bindings_migrate_to_idn() {
    let screen = ScreenId::new();
    let id = OutputId::new();
    // A schema-1 output binding as pro-import wrote it (official NDI transport).
    let v1 = json!({
        "schemaVersion": 1,
        "id": id,
        "name": "Key Feed",
        "screen": screen,
        "transport": {"type": "ndi", "sourceName": "Key-Feed", "groups": ["booth"]},
        "embedAudio": true
    });
    let mut migrated = v1.clone();
    migrate(DocumentKind::OutputBinding, &mut migrated).unwrap();
    assert_eq!(migrated["schemaVersion"], json!(SCHEMA_VERSION));
    assert_eq!(
        migrated["transport"],
        json!({"type": "idn", "sourceName": "Key-Feed", "groups": ["booth"]})
    );
    let b: OutputBinding = decode(v1).unwrap();
    assert_eq!(
        b.transport,
        OutputTransport::Idn {
            source_name: "Key-Feed".into(),
            groups: vec!["booth".into()],
        }
    );
    assert!(b.embed_audio);
    assert_eq!(b.screen, screen);

    // Video and audio input bindings: `source`.
    let input = VideoInputId::new();
    let ib: InputBinding = decode(json!({
        "schemaVersion": 1,
        "input": input,
        "source": {"type": "ndi", "sourceName": "CAM (Camera 1)"}
    }))
    .unwrap();
    assert_eq!(
        ib.source,
        InputSource::Idn {
            source_name: "CAM (Camera 1)".into()
        }
    );
    let ab: AudioInputBinding = decode(json!({
        "input": AudioInputId::new(),
        "source": {"type": "ndi", "sourceName": "MIX (Program)"}
    }))
    .unwrap();
    assert_eq!(
        ab.source,
        InputSource::Idn {
            source_name: "MIX (Program)".into()
        }
    );

    // Other transports, other kinds and current documents are untouched.
    let mut omt = json!({"schemaVersion": 1, "id": id, "screen": screen,
        "transport": {"type": "omt", "name": "x"}});
    migrate(DocumentKind::OutputBinding, &mut omt).unwrap();
    assert_eq!(omt["transport"]["type"], json!("omt"));
    let mut screen_doc = json!({"schemaVersion": 1, "id": screen, "transport": {"type": "ndi"}});
    migrate(DocumentKind::Screen, &mut screen_doc).unwrap();
    assert_eq!(screen_doc["transport"]["type"], json!("ndi"));

    // Unmigrated `ndi` (no longer a variant) decodes to Unknown: the output stays off.
    let raw: OutputTransport =
        serde_json::from_value(json!({"type": "ndi", "sourceName": "x"})).unwrap();
    assert_eq!(raw, OutputTransport::Unknown);
}

/// Version 3: only documents holding a media library location are written as 3 (an older
/// server refuses them instead of losing the location); everything else stays 2.
#[test]
fn library_locations_are_written_as_version_3() {
    let mut asset = MediaAsset {
        location: AssetLocation::Library {
            id: "0011223344556677".into(),
            sha256: "ab".repeat(32),
        },
        ..MediaAsset::default()
    };
    let v = serde_json::to_value(&asset).unwrap();
    assert_eq!(version_for(&v), 3);
    let mut stored = v.clone();
    stored["schemaVersion"] = json!(3);
    migrate(DocumentKind::MediaAsset, &mut stored).unwrap();
    assert_eq!(stored["schemaVersion"], json!(3));
    ensure_writable(&stored).unwrap();
    // In a trash entry's document too; not in a relink's record.
    assert_eq!(version_for(&json!({"document": v})), 3);
    asset.relinks.push(RelinkRecord {
        method: RelinkMethod::Manual,
        previous: asset.location.clone(),
        at: None,
    });
    asset.location = AssetLocation::Unresolved;
    let v = serde_json::to_value(&asset).unwrap();
    assert_eq!(version_for(&v), SCHEMA_VERSION);
    let mut old = v.clone();
    old["schemaVersion"] = json!(3);
    migrate(DocumentKind::MediaAsset, &mut old).unwrap();
    assert_eq!(
        old["schemaVersion"],
        json!(2),
        "written as low as it can be"
    );
    assert_eq!(NEWEST_SCHEMA_VERSION, 3);
}

/// A presentation's own language (lane L, 2026-11-26): saved when set, absent from documents
/// that have none, and older files (written before it existed) load without it.
#[test]
fn a_presentations_language_round_trips_and_old_files_load_without_it() {
    let mut p = Presentation::new("Morning Song");
    p.language = Some("pt-BR".into());
    let v = round_trip(&p);
    assert_eq!(v["language"], json!("pt-BR"));

    let none = serde_json::to_value(Presentation::new("Morning Song")).unwrap();
    assert!(none.get("language").is_none());

    let old = json!({"schemaVersion": SCHEMA_VERSION, "id": PresentationId::new(), "name": "Old"});
    let p: Presentation = decode(old).unwrap();
    assert_eq!(p.language, None);
}
