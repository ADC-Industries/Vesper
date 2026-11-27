//! What the Codex needs from the server it runs in ([`Server`], [`Show`]), and the few types
//! the two share: a change of a collection for the show ([`CollectionUpdate`]) and a show
//! box's request run by the Codex through its own routes ([`Forwarded`]).
//!
//! The server's state implements [`Server`]: it holds the Codex's parts (the writer, the
//! uploads, the media library, the link, the mirror, the search index, backups, exports,
//! adopt), this box's documents and the Codex state, the site, and answers the few questions
//! only the server can (what is live, the request's caller, presence over the link, how a
//! content document is created). The show actor's handle implements [`Show`].

use super::{CodexState, adopt, backups, export, library, link, mirror, search, uploads, writer};
use crate::auth::{Access, Auth};
use crate::data::{ChangeEvent, CollectionContent, ContentChange, ShowData};
use crate::site::Site;
use crate::store::DocWrite;
use idn_open_protocol::channel::Reply;
use model::{CodexRole, CollectionId, DocumentKind};
use protocol::{ApiError, ServerMessage};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;
use vesper_core::change::ConfigError;

/// The server, as the Codex's code sees it (`Shared` in the Codex's modules is `dyn Server`).
pub trait Server: Send + Sync + 'static {
    /// The one serialized writer of every collection ([`writer`]).
    fn codex_writer(&self) -> &writer::Writer;
    /// Media uploads in progress ([`uploads`]).
    fn uploads(&self) -> &uploads::Uploads;
    /// The Codex's media library ([`library`]).
    fn library(&self) -> &Arc<library::MediaLibrary>;
    /// The Codex's side of the box-to-Codex link ([`link`]).
    fn codex_link(&self) -> &Arc<link::Link>;
    /// A show box's mirror of the site's Codex ([`mirror`]).
    fn mirror(&self) -> &Arc<mirror::Mirror>;
    /// The search index of the collections ([`search`]).
    fn search(&self) -> &search::SearchIndex;
    /// Settings backups ([`backups`]).
    fn backups(&self) -> &backups::Backups;
    /// Encrypted exports and imports ([`export`]).
    fn exports(&self) -> &export::Exports;
    /// A show box's upload of its dormant collection ([`adopt`]).
    fn adopt(&self) -> &adopt::Adopter;

    /// The server's software version.
    fn version(&self) -> &'static str;
    /// The site this server is in.
    fn site(&self) -> &Arc<Site>;
    /// The server's identity and pairing.
    fn auth(&self) -> &Auth;
    /// Events for every WebSocket.
    fn events(&self) -> &broadcast::Sender<Arc<ServerMessage>>;

    /// The current documents: this box's and the active collection's content.
    fn data(&self) -> Arc<ShowData>;
    /// The Codex: its collections and this box's active one.
    fn codex(&self) -> Arc<CodexState>;
    /// Replaces the Codex state.
    fn set_codex(&self, codex: Arc<CodexState>);
    /// A collection's current documents (the show's for the active collection). `None` for a
    /// collection that isn't there.
    fn collection_content(&self, id: CollectionId) -> Option<Arc<CollectionContent>>;
    /// This box holds the site's Codex (role `host`).
    fn codex_is_host(&self) -> bool {
        self.codex().config.role == CodexRole::Host
    }

    /// What is live on this box, named (for the `liveShow` refusal of a switch).
    fn live_items(&self) -> Vec<String>;
    /// The collection a show box's forwarded request being handled acts on, if any.
    fn forwarded_collection(&self) -> Option<CollectionId>;
    /// Plans adding one content document (a library, presentation, playlist node, theme,
    /// media bin or global group) as it was stored, validated against `content` like a
    /// create with its ID (restoring it from the trash).
    fn recreate(
        &self,
        content: &CollectionContent,
        kind: DocumentKind,
        doc: Value,
    ) -> Result<ContentPlanned, ConfigError>;
    /// A show box's presence report or lock request over the link (on the Codex box).
    fn presence_serve(&self, fingerprint: &str, kind: &str, payload: &[u8]) -> Reply;
    /// The site's presence list pushed by the Codex (on a show box).
    fn presence_pushed(&self, payload: &[u8]) -> Reply;
}

/// A shared handle to the server is the server too (so `&Arc<dyn Server>` passes where
/// `&dyn Server` is taken).
impl<T: Server + ?Sized> Server for Arc<T> {
    fn codex_writer(&self) -> &writer::Writer {
        (**self).codex_writer()
    }
    fn uploads(&self) -> &uploads::Uploads {
        (**self).uploads()
    }
    fn library(&self) -> &Arc<library::MediaLibrary> {
        (**self).library()
    }
    fn codex_link(&self) -> &Arc<link::Link> {
        (**self).codex_link()
    }
    fn mirror(&self) -> &Arc<mirror::Mirror> {
        (**self).mirror()
    }
    fn search(&self) -> &search::SearchIndex {
        (**self).search()
    }
    fn backups(&self) -> &backups::Backups {
        (**self).backups()
    }
    fn exports(&self) -> &export::Exports {
        (**self).exports()
    }
    fn adopt(&self) -> &adopt::Adopter {
        (**self).adopt()
    }
    fn version(&self) -> &'static str {
        (**self).version()
    }
    fn site(&self) -> &Arc<Site> {
        (**self).site()
    }
    fn auth(&self) -> &Auth {
        (**self).auth()
    }
    fn events(&self) -> &broadcast::Sender<Arc<ServerMessage>> {
        (**self).events()
    }
    fn data(&self) -> Arc<ShowData> {
        (**self).data()
    }
    fn codex(&self) -> Arc<CodexState> {
        (**self).codex()
    }
    fn set_codex(&self, codex: Arc<CodexState>) {
        (**self).set_codex(codex)
    }
    fn collection_content(&self, id: CollectionId) -> Option<Arc<CollectionContent>> {
        (**self).collection_content(id)
    }
    fn codex_is_host(&self) -> bool {
        (**self).codex_is_host()
    }
    fn live_items(&self) -> Vec<String> {
        (**self).live_items()
    }
    fn forwarded_collection(&self) -> Option<CollectionId> {
        (**self).forwarded_collection()
    }
    fn recreate(
        &self,
        content: &CollectionContent,
        kind: DocumentKind,
        doc: Value,
    ) -> Result<ContentPlanned, ConfigError> {
        (**self).recreate(content, kind, doc)
    }
    fn presence_serve(&self, fingerprint: &str, kind: &str, payload: &[u8]) -> Reply {
        (**self).presence_serve(fingerprint, kind, payload)
    }
    fn presence_pushed(&self, payload: &[u8]) -> Reply {
        (**self).presence_pushed(payload)
    }
}

/// A planned change of a collection's content documents ([`Server::recreate`]).
#[derive(Debug)]
pub struct ContentPlanned {
    /// New documents.
    pub change: ContentChange,
    /// Files to write.
    pub writes: Vec<DocWrite>,
    /// `documentChanged` events once applied.
    pub events: Vec<ChangeEvent>,
}

/// A future the show actor answers.
pub type ShowFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

/// The show actor, as the Codex sees it.
pub trait Show: Send + Sync + 'static {
    /// Swaps in a change of the active collection that the writer has written (in order with
    /// the show commands).
    fn collection(&self, update: CollectionUpdate) -> ShowFuture<'_, ()>;
    /// `reloadData`: reloads the data folder; the revision of the state that includes it.
    fn reload_data(&self) -> ShowFuture<'_, u64>;
}

/// A shared handle to the show actor is one too.
impl<T: Show + ?Sized> Show for Arc<T> {
    fn collection(&self, update: CollectionUpdate) -> ShowFuture<'_, ()> {
        (**self).collection(update)
    }
    fn reload_data(&self) -> ShowFuture<'_, u64> {
        (**self).reload_data()
    }
}

/// A change of the active collection's content, for the show actor.
#[derive(Debug)]
pub enum CollectionUpdate {
    /// Presentations, libraries or playlists changed: the show follows like after an edit
    /// (an edited live slide re-renders, a deleted one stays on screen until the next
    /// trigger).
    Content(ContentChange),
    /// New lists of timers, props, messages or stage layouts (what was deleted leaves the show).
    ShowDocs {
        /// Timers.
        timers: Option<Vec<model::Timer>>,
        /// Props.
        props: Option<Vec<model::Prop>>,
        /// Messages.
        messages: Option<Vec<model::Message>>,
        /// Stage layouts.
        stage_layouts: Option<Vec<model::StageLayout>>,
    },
    /// Another collection becomes active: its content (from the Codex state) replaces the
    /// show's, and the content the show had goes back to the Codex state, in one step.
    Switch(model::CollectionId),
    /// A show box's copy of the site's collection changed ([`mirror`]): its content, rebuilt,
    /// replaces the show's; the show follows like after an edit.
    Mirror(Arc<CollectionContent>),
}

/// A show box's request run by the Codex through its own routes (write-through,
/// [`super::write_through::execute`]): who made it, through which box, on which collection.
#[derive(Clone, Debug)]
pub struct Forwarded {
    /// The client, as the show box authenticated it.
    pub access: Access,
    /// The show box's server ID (the change log names it).
    pub machine: String,
    /// The collection the show box shows: the content routes act on it.
    pub collection: model::CollectionId,
}
