//! TypeScript bindings export.
//!
//! The committed bindings live in `crates/protocol/bindings/` (one `.ts` file per type plus
//! `index.ts`). The `bindings` test regenerates them into a temporary directory and fails when
//! they differ. To regenerate after changing a wire type:
//!
//! ```text
//! UPDATE_BINDINGS=1 cargo test -p protocol --test bindings
//! ```

use std::path::Path;
use ts_rs::{Config, ExportError, TS};

/// Command that regenerates the committed bindings.
pub const REGENERATE: &str = "UPDATE_BINDINGS=1 cargo test -p protocol --test bindings";

/// The ts-rs configuration: 64-bit integers are plain `number`s (serde_json writes them as
/// JSON numbers; revisions and counters stay far below 2^53).
pub fn config(out_dir: &Path) -> Config {
    Config::new().with_out_dir(out_dir).with_large_int("number")
}

/// Writes every wire type (and every model type they reach) into `out_dir`, one file per type,
/// plus `index.ts` re-exporting all of them. Returns the type names, sorted.
pub fn export_all(out_dir: &Path) -> Result<Vec<String>, ExportError> {
    let cfg = config(out_dir);
    macro_rules! export {
        ($($t:ty),* $(,)?) => {$( <$t>::export_all(&cfg)?; )*};
    }
    export!(
        // WebSocket
        crate::ClientMessage,
        crate::ServerMessage,
        crate::Command,
        crate::ShowState,
        // REST
        crate::ServerInfo,
        crate::PairRequest,
        crate::PairResponse,
        crate::PairPending,
        crate::UpgradeCredential,
        crate::PairingCode,
        crate::ClientInfo,
        crate::StateSnapshot,
        crate::CommandResponse,
        crate::ErrorResponse,
        crate::LibrarySummary,
        crate::PresentationSummary,
        crate::PlaylistTreeNode,
        crate::PlaylistDetail,
        crate::ThemeSummary,
        crate::ThemeEntry,
        crate::MediaAssetInfo,
        crate::DocumentUse,
        crate::MediaBinNode,
        crate::MediaBinDetail,
        crate::ThumbnailPrefetch,
        crate::DiagnosticsReport,
        crate::ServerDetails,
        crate::FontList,
        crate::ScreenEntry,
        crate::OutputEntry,
        crate::ScreenDeleted,
        crate::LookEntry,
        crate::DuplicateLook,
        crate::MaskEntry,
        crate::VideoInputEntry,
        crate::MaskDeleted,
        crate::TimerEntry,
        crate::MessageEntry,
        crate::PropEntry,
        crate::StageLayoutEntry,
        crate::StageLayoutTemplate,
        crate::SlideRef,
        crate::OutputTransportInfo,
        crate::AudioDeviceInfo,
        model::AudioDeviceConfig,
        model::CaptionsConfig,
        crate::PushCaption,
        crate::CaptionModel,
        model::Macro,
        crate::RemoteCompatSettings,
        crate::UpdateRemoteCompat,
        crate::RemoteCompatStatus,
        // Scripture
        crate::ScriptureTranslation,
        crate::ScriptureBook,
        crate::ScriptureParse,
        crate::ScripturePassage,
        crate::ScriptureSearch,
        crate::ScriptureSlideRequest,
        crate::ScriptureSlides,
        crate::ScriptureCreate,
        crate::ScriptureCreated,
        crate::ScriptureResplit,
        crate::ScriptureResplitResult,
        // Presence
        crate::PresenceList,
        crate::LiveSourceList,
        // Recording and streaming
        crate::StreamingSettings,
        crate::StreamingStatus,
        crate::StartGateway,
        crate::TakeLock,
        // Site
        crate::SiteStatus,
        crate::SiteSettings,
        crate::JoinSite,
        crate::PairingRequest,
        crate::DenyPairing,
        crate::SiteApiKey,
        crate::MembershipState,
        crate::SiteMembership,
        crate::SiteSession,
        crate::SiteSignIn,
        crate::SiteUnlock,
        crate::SiteAccess,
        crate::JoinRequestInfo,
        crate::ApproveJoin,
        crate::OpenPairing,
        crate::JoinCodeInfo,
        crate::SiteDevice,
        crate::SiteDeviceMachine,
        crate::IdentifyDevice,
        crate::SetDeviceRelay,
        crate::RestartDevice,
        crate::LogSnapshot,
        crate::SiteUser,
        crate::PutSiteUser,
        crate::SetSitePassword,
        crate::IssueToken,
        crate::IssuedToken,
        crate::SiteAuditEntry,
        crate::SiteEvent,
        crate::SiteEventName,
        // Codex
        crate::CodexStatus,
        crate::MediaLibraryStatus,
        crate::CodexSync,
        crate::CodexBoxSync,
        crate::CollectionBoxes,
        crate::CodexBox,
        crate::LibraryItems,
        crate::LibraryItem,
        crate::CodexCacheStatus,
        crate::CollectionSummary,
        crate::CreateCollection,
        crate::UpdateCollection,
        crate::SetAssignment,
        crate::CodexPart,
        crate::CollectionChanges,
        // Updates (the site's update channel, Vesper Launchers)
        crate::UpdatePackage,
        crate::LauncherReport,
        crate::LauncherEntry,
        crate::LauncherAction,
        crate::LauncherActions,
        crate::LauncherJoinCode,
        crate::GithubSettings,
        crate::GithubReleases,
        crate::Catalogue,
        // Mosaic (the site's display boxes and channels)
        crate::MosaicState,
        crate::PutMosaicBox,
        crate::PutMosaicChannel,
        crate::MosaicIdentify,
        crate::MosaicTestPattern,
        crate::PutMosaicWall,
        crate::PutMosaicService,
        crate::TrashItem,
        crate::TrashRestored,
        crate::TrashPurged,
        crate::DeletedCollection,
        crate::CreateUpload,
        crate::UploadInfo,
        crate::UploadConflict,
        crate::StartDormantUpload,
        crate::DormantUploadState,
        crate::TransferCount,
        crate::DormantUpload,
        crate::FinishUpload,
        crate::UploadFinished,
        crate::CodexStorage,
        crate::CollectionStorage,
        crate::DiskUsage,
        crate::FileUsage,
        crate::TrashUsage,
        crate::LibraryUsage,
        crate::CollectionMedia,
        crate::RelinkTarget,
        crate::RelinkedAsset,
        crate::RelinkSearch,
        crate::RelinkFound,
        crate::RelinkAmbiguous,
        crate::RelinkReport,
        crate::ClientFile,
        crate::MediaMatchRequest,
        crate::MatchBy,
        crate::MediaMatch,
        crate::MediaMatchReport,
        crate::RerootRequest,
        crate::RerootFrom,
        crate::RerootTo,
        crate::SearchResults,
        crate::SearchHit,
        crate::SearchField,
        crate::SnippetPart,
        crate::BackupSetSummary,
        crate::SnapshotSummary,
        crate::SnapshotContents,
        crate::SnapshotItemContent,
        crate::SnapshotTaken,
        crate::RestoreChange,
        crate::RestorePreview,
        crate::RestoreRequest,
        crate::RestoreResult,
        crate::RestoreState,
        crate::QueuedRestore,
        crate::OrielPreferences,
        crate::OrielPreferencesBackup,
        crate::CodexExportSettings,
        crate::CodexExportStatus,
        crate::ScheduledExport,
        crate::UpdateExportSettings,
        crate::SetExportPassphrase,
        crate::CreateExport,
        crate::ExportManifest,
        crate::ExportedCollection,
        crate::ImportUpload,
        crate::OpenImport,
        crate::ImportComparison,
        crate::ImportCollection,
        crate::ImportBackupSet,
        crate::ImportPreview,
        crate::ImportRequest,
        crate::ImportResult,
        // Content editing
        crate::PresentationEdit,
        crate::PresentationEditResult,
        crate::PlaylistEdit,
        crate::PlaylistEditResult,
        crate::EditConflict,
        crate::CreateLibrary,
        crate::UpdateLibrary,
        crate::CreatePresentation,
        crate::DuplicatePresentation,
        crate::CreateTheme,
        crate::UpdateTheme,
        crate::DuplicateTheme,
        crate::MovePresentation,
        crate::CreatePlaylistNode,
        crate::UpdatePlaylistNode,
        crate::DuplicatePlaylist,
        crate::MediaBinOp,
        crate::MediaBinEdit,
        crate::MediaBinEditResult,
        crate::CreateMediaBin,
        crate::UpdateMediaBin,
        crate::CreateGlobalGroup,
        crate::UpdateGlobalGroup,
        crate::DocumentRevision,
        crate::DeletedDocuments,
        model::GlobalGroup,
        // Model documents served as they are stored
        model::Presentation,
        model::Library,
        model::PlaylistNode,
        model::Look,
        model::Mask,
        model::Screen,
        model::OutputBinding,
        model::TestPattern,
        model::ClearGroup,
        model::MediaAsset,
        model::DocumentKind,
        // Signage (channels, schedules, playlists, feeds, the override)
        crate::SignageState,
        crate::PutSignageChannel,
        crate::SignageFeedPreview,
        crate::SignagePlaces,
        crate::ServerStats,
        // Media preparation
        crate::MediaPrepStatus,
        crate::PrepareMedia,
        crate::SetMediaTrim,
        // Reports
        crate::CcliReport,
        crate::ChordProParse,
        crate::ChordProParsed,
        // SongSelect files and pasted lyrics
        crate::SongSelectParse,
        crate::SongSelectParsed,
        // PowerPoint / Keynote / Google Slides import
        crate::PptxImportJob,
        // Machine translation (second-language slides)
        crate::TranslationModels,
        crate::TranslatePreview,
        crate::TranslateJob,
    );
    let mut names: Vec<String> = std::fs::read_dir(out_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".ts")
                .filter(|n| *n != "index")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    let mut index = String::from(
        "// Generated by the `protocol` crate (ts-rs). Do not edit by hand. Regenerate with:\n",
    );
    index.push_str(&format!("//   {REGENERATE}\n"));
    for n in &names {
        index.push_str(&format!("export type {{ {n} }} from \"./{n}\";\n"));
    }
    index.push_str(&format!(
        "\n/** Protocol major version these bindings describe. */\nexport const PROTOCOL_VERSION = {};\n",
        crate::PROTOCOL_VERSION
    ));
    index.push_str(&format!(
        "/** Size of the binary preview frame header (see README). */\nexport const PREVIEW_HEADER_LEN = {};\n",
        crate::PREVIEW_HEADER_LEN
    ));
    std::fs::write(out_dir.join("index.ts"), index)?;
    Ok(names)
}
