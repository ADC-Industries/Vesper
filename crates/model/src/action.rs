//! Actions: what happens when a slide (or macro) is triggered, besides showing the slide.
//!
//! In ProPresenter a cue holds a list of actions, one of which is "show this slide". Here showing
//! the slide is implicit and [`SlideAction`]s are the *extra* effects. The reference library
//! only uses media and prop actions; the other kinds have minimal payloads with stable links so
//! they can grow without a breaking change. Imported action types we do not model are kept
//! verbatim in [`ActionKind::Unsupported`].

use crate::common::{ImportInfo, Link, is_default, is_true, is_zero, yes};
use crate::ids::{
    ActionId, ClearGroupId, LookId, MacroId, MessageId, PlaylistId, PlaylistItemId, PresentationId,
    PropId, ScreenId, StageLayoutId, TimerId,
};
use crate::media::{MediaLayer, MediaTrigger, VideoInputTrigger};
use crate::show::ClearTarget;
use serde::{Deserialize, Serialize};

/// An action with its common envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SlideAction {
    /// Action ID (imported UUIDs are reused).
    pub id: ActionId,
    /// Disabled actions are kept but not run.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Display name (ProPresenter uses the media file name for media actions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    /// Delay after the slide is triggered, seconds.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay: f64,
    /// What the action does.
    pub kind: ActionKind,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl SlideAction {
    /// An enabled action with a fresh ID.
    pub fn new(kind: ActionKind) -> Self {
        SlideAction {
            id: ActionId::new(),
            enabled: true,
            name: None,
            delay: 0.0,
            kind,
            import: None,
        }
    }

    /// Put media on its layer.
    pub fn media(trigger: MediaTrigger) -> Self {
        SlideAction::new(ActionKind::Media(trigger))
    }

    /// Copy with a fresh ID.
    pub fn duplicate(&self) -> Self {
        SlideAction {
            id: ActionId::new(),
            ..self.clone()
        }
    }

    /// The media layer this action targets, if it is a media action.
    pub fn media_layer(&self) -> Option<MediaLayer> {
        match &self.kind {
            ActionKind::Media(t) => Some(t.layer),
            _ => None,
        }
    }
}

/// The action payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ActionKind {
    /// Put media on a layer (background image / video, foreground video, audio).
    Media(MediaTrigger),
    /// Show a live video input on the video-input layer.
    VideoInput(VideoInputTrigger),
    /// Clear layers.
    ClearLayer {
        /// What to clear.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<ClearTarget>,
    },
    /// Run a clear group.
    ClearGroup {
        /// The clear group.
        group: Link<ClearGroupId>,
    },
    /// Control a timer.
    Timer {
        /// The timer.
        timer: Link<TimerId>,
        /// Operation.
        #[serde(default, skip_serializing_if = "is_default")]
        operation: TimerOperation,
    },
    /// Show or hide a message.
    Message {
        /// The message.
        message: Link<MessageId>,
        /// `true` = show, `false` = hide.
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        show: bool,
        /// Token values for this showing (unlisted tokens keep their defaults).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        values: Vec<TokenValue>,
    },
    /// Show or hide a prop.
    Prop {
        /// The prop.
        prop: Link<PropId>,
        /// `true` = show, `false` = hide.
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        show: bool,
    },
    /// Switch the live look.
    Look {
        /// The look.
        look: Link<LookId>,
    },
    /// Set the stage layout of stage screens.
    StageLayout {
        /// Assignments.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        assignments: Vec<StageAssignment>,
    },
    /// Run a macro.
    Macro {
        /// The macro.
        target: Link<MacroId>,
    },
    /// Trigger another presentation or playlist item.
    Trigger {
        /// What to trigger.
        target: TriggerTarget,
    },
    /// An imported action kind this build does not model (skipped at runtime, kept verbatim).
    Unsupported {
        /// Source action type (e.g. `"ACTION_TYPE_COMMUNICATION"`).
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag from a newer schema (skipped; lossy, see [`crate::migrate()`]).
    #[serde(other)]
    Unknown,
}

/// Timer operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TimerOperation {
    /// Start or resume.
    #[default]
    Start,
    /// Stop (pause).
    Stop,
    /// Reset to the configured value.
    Reset,
    /// Reset, then start.
    Restart,
    /// Set the current value.
    Set {
        /// New value, seconds.
        seconds: f64,
    },
    /// Add to the current value.
    Adjust {
        /// Seconds to add (negative subtracts).
        seconds: f64,
    },
    /// Unknown tag (ignored).
    #[serde(other)]
    Unknown,
}

/// A message token value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TokenValue {
    /// Token name (as in `${name}`).
    pub name: String,
    /// Text value.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
}

/// One stage layout assignment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StageAssignment {
    /// Stage screen (`None` = every stage screen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub screen: Option<Link<ScreenId>>,
    /// Layout to show.
    pub layout: Link<StageLayoutId>,
}

/// What a trigger action triggers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TriggerTarget {
    /// First slide of a presentation.
    Presentation {
        /// The presentation.
        presentation: Link<PresentationId>,
    },
    /// An item of a playlist.
    PlaylistItem {
        /// The playlist.
        playlist: Link<PlaylistId>,
        /// The item.
        item: PlaylistItemId,
    },
    /// Unknown tag (ignored).
    #[serde(other)]
    Unknown,
}
