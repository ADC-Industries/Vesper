//! The server's side of the Codex: the server's state as the Codex's [`Server`], the show
//! actor as its [`Show`], and the writes the server plans for it (content edits through
//! [`crate::content`], timers, messages, props and stage layouts through [`crate::showdocs`]).

use super::writer::{Planned, decode};
use super::{CollectionUpdate, ContentPlanned, Server, Show, ShowFuture};
use crate::Shared;
use crate::actor::ShowHandle;
use crate::auth::Auth;
use crate::config::ConfigError;
use crate::content::{ContentOp, ContentResult};
use crate::data::{ChangeEvent, CollectionContent, ShowData};
use crate::showdocs::{DocKind, DocOp, DocResult};
use crate::site::Site;
use idn_open_protocol::channel::Reply;
use model::{ChangeBy, CollectionId, Document, DocumentKind};
use protocol::{EditOps, ServerMessage};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

impl Shared {
    /// The server's state as the Codex's [`Server`] (what the Codex's functions take).
    pub fn codex_server(self: &Arc<Self>) -> Arc<dyn Server> {
        self.clone()
    }
}

impl Server for Shared {
    fn codex_writer(&self) -> &super::writer::Writer {
        &self.codex_writer
    }
    fn uploads(&self) -> &super::uploads::Uploads {
        &self.uploads
    }
    fn library(&self) -> &Arc<super::library::MediaLibrary> {
        &self.library
    }
    fn codex_link(&self) -> &Arc<super::link::Link> {
        &self.codex_link
    }
    fn mirror(&self) -> &Arc<super::mirror::Mirror> {
        &self.mirror
    }
    fn search(&self) -> &super::search::SearchIndex {
        &self.search
    }
    fn backups(&self) -> &super::backups::Backups {
        &self.backups
    }
    fn exports(&self) -> &super::export::Exports {
        &self.exports
    }
    fn adopt(&self) -> &super::adopt::Adopter {
        &self.adopt
    }
    fn version(&self) -> &'static str {
        crate::VERSION
    }
    fn site(&self) -> &Arc<Site> {
        &self.site
    }
    fn auth(&self) -> &Auth {
        &self.auth
    }
    fn events(&self) -> &broadcast::Sender<Arc<ServerMessage>> {
        &self.events
    }
    fn data(&self) -> Arc<ShowData> {
        Shared::data(self)
    }
    fn codex(&self) -> Arc<super::CodexState> {
        Shared::codex(self)
    }
    fn set_codex(&self, codex: Arc<super::CodexState>) {
        Shared::set_codex(self, codex)
    }
    fn collection_content(&self, id: CollectionId) -> Option<Arc<CollectionContent>> {
        Shared::collection_content(self, id)
    }
    fn codex_is_host(&self) -> bool {
        Shared::codex_is_host(self)
    }
    fn live_items(&self) -> Vec<String> {
        live_items(self)
    }
    fn forwarded_collection(&self) -> Option<CollectionId> {
        crate::api::forwarded_collection()
    }
    fn recreate(
        &self,
        content: &CollectionContent,
        kind: DocumentKind,
        doc: Value,
    ) -> Result<ContentPlanned, ConfigError> {
        recreate(content, kind, doc)
    }
    fn presence_serve(&self, fingerprint: &str, kind: &str, payload: &[u8]) -> Reply {
        super::site_presence::serve(self, fingerprint, kind, payload)
    }
    fn presence_pushed(&self, payload: &[u8]) -> Reply {
        super::site_presence::pushed(self, payload)
    }
}

impl Show for ShowHandle {
    fn collection(&self, update: CollectionUpdate) -> ShowFuture<'_, ()> {
        Box::pin(ShowHandle::collection(self, update))
    }
    fn reload_data(&self) -> ShowFuture<'_, u64> {
        Box::pin(self.apply(protocol::Command::ReloadData))
    }
}

/// What is live on this box: the slide and announcement layers, the media, foreground video and
/// audio layers, the props and the messages (named, for the `liveShow` refusal).
pub fn live_items(sh: &Shared) -> Vec<String> {
    let (state, data) = (sh.state(), sh.data());
    let c = &data.content;
    let mut out = Vec::new();
    let pres = |p: model::PresentationId| {
        c.presentation(p)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| p.0.to_string())
    };
    if let Some(s) = &state.slide {
        out.push(format!("a slide of “{}”", pres(s.presentation)));
    }
    if let Some(s) = &state.announcements {
        out.push(format!("an announcement of “{}”", pres(s.presentation)));
    }
    for (m, what) in [
        (&state.media, "media"),
        (&state.slide_media, "a foreground video"),
        (&state.audio, "audio"),
    ] {
        if let Some(m) = m {
            out.push(format!("{what} “{}”", c.asset_name(m.asset)));
        }
    }
    for p in &state.props {
        let name = c
            .props
            .iter()
            .find(|x| x.id == *p)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| p.0.to_string());
        out.push(format!("the prop “{name}”"));
    }
    for m in &state.messages {
        let name = c
            .messages
            .iter()
            .find(|x| x.id == m.message)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| m.message.0.to_string());
        out.push(format!("the message “{name}”"));
    }
    out
}

/// Plans re-adding one content document as it was stored ([`Server::recreate`]): a create
/// with its ID, through [`crate::content::plan`].
fn recreate(
    content: &CollectionContent,
    kind: DocumentKind,
    v: Value,
) -> Result<ContentPlanned, ConfigError> {
    let op = match kind {
        DocumentKind::Library => ContentOp::CreateLibrary(Box::new(protocol::CreateLibrary {
            name: None,
            document: Some(decode(v)?),
        })),
        DocumentKind::Presentation => {
            ContentOp::CreatePresentation(Box::new(protocol::CreatePresentation {
                document: Some(decode(v)?),
                ..Default::default()
            }))
        }
        DocumentKind::Theme => ContentOp::CreateTheme(
            Box::new(protocol::CreateTheme {
                name: None,
                document: Some(decode(v)?),
            }),
            model::Size::HD,
        ),
        DocumentKind::MediaBin => ContentOp::CreateMediaBin(Box::new(protocol::CreateMediaBin {
            document: Some(decode(v)?),
            ..Default::default()
        })),
        DocumentKind::GlobalGroup => {
            ContentOp::CreateGlobalGroup(Box::new(protocol::CreateGlobalGroup {
                document: Some(decode(v)?),
                ..Default::default()
            }))
        }
        _ => ContentOp::CreatePlaylist(Box::new(protocol::CreatePlaylistNode {
            name: None,
            kind: None,
            purpose: None,
            parent: None,
            after: None,
            document: Some(decode(v)?),
        })),
    };
    let plan = crate::content::plan(content, op)?;
    Ok(ContentPlanned {
        change: plan.change,
        writes: plan.writes,
        events: plan.events,
    })
}

/// A content change (presentations, libraries, playlists; see [`crate::content`]) of this
/// box's active collection, through the Codex writer. `client_id` is echoed in
/// `documentChanged`.
pub async fn content(
    sh: &Arc<Shared>,
    show: &ShowHandle,
    op: ContentOp,
    client_id: Option<String>,
    by: ChangeBy,
) -> Result<ContentResult, ConfigError> {
    let plan = |content: &Arc<CollectionContent>| {
        let recreated = match &op {
            ContentOp::CreateLibrary(b) => b.document.as_ref().map(|d| d.id.0),
            ContentOp::CreatePresentation(b) => b.document.as_ref().map(|d| d.id.0),
            ContentOp::CreatePlaylist(b) => b.document.as_ref().map(|d| d.id.0),
            ContentOp::CreateTheme(b, _) => b.document.as_ref().map(|d| d.id.0),
            ContentOp::CreateMediaBin(b) => b.document.as_ref().map(|d| d.id.0),
            ContentOp::CreateGlobalGroup(b) => b.document.as_ref().map(|d| d.id.0),
            _ => None,
        };
        let plan = crate::content::plan(content, op)?;
        let mut planned = Planned::new(
            plan.writes,
            CollectionUpdate::Content(plan.change),
            plan.events,
        );
        planned.recreated = recreated;
        if let ContentResult::Deleted(d) = &plan.result {
            planned.trashed = Some((d.kind, d.documents.clone()));
        }
        Ok((planned, plan.result))
    };
    sh.codex_writer
        .change(&sh.codex_server(), show, plan, client_id, by)
        .await
}

/// A change of a timer, message, prop or stage layout (see [`crate::showdocs`]) of this
/// box's active collection, through the Codex writer.
pub async fn docs(
    sh: &Arc<Shared>,
    show: &ShowHandle,
    op: DocOp,
    client_id: Option<String>,
    by: ChangeBy,
) -> Result<DocResult, ConfigError> {
    let plan = |content: &Arc<CollectionContent>| {
        let canvas = sh.data().settings.default_canvas_size;
        let trashed = match &op {
            DocOp::Delete { kind, id, .. } => {
                show_doc(content, *kind, *id).map(|v| (kind.kind(), vec![v]))
            }
            _ => None,
        };
        let recreated = match &op {
            DocOp::Create(_, body) => body
                .get("id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok()),
            _ => None,
        };
        let plan = crate::showdocs::plan(content, canvas, op)?;
        let ops = plan.ops.map(EditOps::Presentation);
        let events = plan
            .changes
            .into_iter()
            .map(|(kind, id, change, revision)| ChangeEvent {
                kind,
                id,
                change,
                revision,
                ops: ops.clone(),
            })
            .collect();
        let mut planned = Planned::new(
            plan.writes,
            CollectionUpdate::ShowDocs {
                timers: plan.timers,
                props: plan.props,
                messages: plan.messages,
                stage_layouts: plan.stage_layouts,
            },
            events,
        );
        planned.trashed = trashed;
        planned.recreated = recreated;
        Ok((planned, plan.result))
    };
    sh.codex_writer
        .change(&sh.codex_server(), show, plan, client_id, by)
        .await
}

/// A show document (timer, message, prop, stage layout) as stored, by ID.
fn show_doc(content: &CollectionContent, kind: DocKind, id: uuid::Uuid) -> Option<Value> {
    fn find<T: Document + serde::Serialize>(list: &[T], id: uuid::Uuid) -> Option<Value> {
        list.iter()
            .find(|d| d.id() == id)
            .and_then(|d| serde_json::to_value(d).ok())
    }
    match kind {
        DocKind::Timer => find(&content.timers, id),
        DocKind::Message => find(&content.messages, id),
        DocKind::Prop => find(&content.props, id),
        DocKind::StageLayout => find(&content.stage_layouts, id),
    }
}
