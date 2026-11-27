//! The IDN Nodes Mosaic works with (`docs/MOSAIC-PLAN.md`), behind one trait so tests can stand
//! in for them:
//!
//! - **This machine's own Node** ([`Nodes::own`]): its fingerprint says which Node is this
//!   Vesper's (`Node::connect("Vesper")`, then `enrollment().fingerprint()`, as the IDN maintainers
//!   advised), so that machine is never offered as a display box; on the site's server its
//!   routers carry Mosaic's channels ([`Nodes::source_name`]) and it pushes the site's logo to the
//!   boxes with `media_send` ([`Nodes::send_media`], `replace="true"` since IDN Beta14) and
//!   follows the transfer to `sent` ([`Nodes::transfer`]).
//! - **Other machines' Node settings** ([`Nodes::apply`]) through the Site Manager this server
//!   hosts (`SiteCommand::Apply`, run as the server: routers, a box's display service).
//!
//! Blocking: call from `spawn_blocking` or a thread of its own.

use crate::lock;
use crate::site::Site;
use idn_open_protocol::node::Node;
use idn_open_protocol::xml::Element;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The name this server connects to its Node with (the Node's panel shows it).
pub const APP_NAME: &str = "Vesper";
/// A Node not there is looked for again after this.
const RETRY: Duration = Duration::from_secs(30);

/// What Mosaic asks of IDN Nodes.
pub trait Nodes: Send + Sync + std::fmt::Debug {
    /// This machine's IDN Node's fingerprint (`None`: no Node here, or not reached yet).
    fn own(&self) -> Option<String>;
    /// The full name (`HOST (name)`) of a source `name` this machine publishes: a router.
    fn source_name(&self, name: &str) -> String;
    /// Changes machine `machine`'s Node settings through the Site Manager (as the server).
    fn apply(&self, site: &Site, machine: &str, settings: Map<String, Value>)
    -> Result<(), String>;
    /// Machine `machine`'s IDN Node's latest report through the Site Manager (`None`: offline,
    /// not a Node, or not reported yet).
    fn report(&self, site: &Site, machine: &str) -> Option<Element>;
    /// This machine's Node's media folder.
    fn media_dir(&self) -> Result<PathBuf, String>;
    /// Sends `file` (in this machine's media folder) to machine `to`'s Node, which keeps it at
    /// the top of its own media folder (`media_send`). `Ok` means the box accepted the offer
    /// (`offered`), not that the file arrived. `replace` (IDN Beta14, `replace="true"`): the
    /// box replaces a finished file of the name once the new one is whole and checked; without
    /// it a name the box has already gets "name (2)" there. A box's Node from before Beta14
    /// refuses a `replace` offer. `Ok(Some(offer))`: this machine's Node named the offer (Beta14
    /// and later), whose state [`Nodes::transfer`] follows; `Ok(None)`: an older Node here (it
    /// sent the file without `replace`).
    fn send_media(&self, file: &Path, to: &str, replace: bool) -> Result<Option<String>, String>;
    /// How a file this machine's Node offered (`offer`, from [`Nodes::send_media`]) is going
    /// (`None`: not known: no Node, or no longer listed). A box from before Beta14 lets the
    /// stream go early, so its sender may say `failed` though the file arrived: trust a
    /// `failed` only from a box that took a `replace` offer (Beta14 and later).
    fn transfer(&self, offer: &str, to: &str) -> Option<Transfer>;
}

/// A file this machine's Node offered, as its Node's `media` list says (`<transfer … done
/// state/>`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transfer {
    /// `offered`, `sent` (the box fetched all of it), `refused` or `failed: …`.
    pub state: String,
    /// The bytes the box has fetched so far (it moves while the box fetches).
    pub done: u64,
}

impl Transfer {
    /// In state `state`, `done` bytes fetched.
    pub fn new(state: impl Into<String>, done: u64) -> Self {
        Transfer {
            state: state.into(),
            done,
        }
    }
}

/// The real ones: the Site Manager, and (`local`) this machine's IDN Node.
#[derive(Debug)]
pub struct IdnNodes {
    local: bool,
    node: Mutex<Option<Node>>,
    tried: Mutex<Option<Instant>>,
    fingerprint: Mutex<Option<String>>,
}

impl IdnNodes {
    /// With this machine's IDN Node (the server's default), or without (`local: false`: a
    /// server that must not touch the machine's Node, such as a test's).
    pub fn new(local: bool) -> Self {
        IdnNodes {
            local,
            node: Mutex::new(None),
            tried: Mutex::new(None),
            fingerprint: Mutex::new(None),
        }
    }

    /// Runs `f` with the Node, connecting first when it isn't (at most every [`RETRY`]); a
    /// failed request drops the connection (it is made again next time).
    fn with<T>(&self, f: impl FnOnce(&Node) -> std::io::Result<T>) -> Result<T, String> {
        if !self.local {
            return Err("no IDN Node is used on this server".into());
        }
        let mut node = lock(&self.node);
        if node.is_none() {
            let mut tried = lock(&self.tried);
            if tried.is_some_and(|t| t.elapsed() < RETRY) {
                return Err("no IDN Node on this machine".into());
            }
            *tried = Some(Instant::now());
            match Node::connect(APP_NAME) {
                Ok(n) => {
                    *lock(&self.fingerprint) = Some(n.enrollment().fingerprint());
                    *node = Some(n);
                }
                Err(e) => return Err(format!("no IDN Node on this machine ({e})")),
            }
        }
        let r = f(node.as_ref().expect("connected"));
        if r.is_err() {
            *node = None;
            *lock(&self.tried) = None;
        }
        r.map_err(|e| format!("this machine's IDN Node: {e}"))
    }
}

impl Nodes for IdnNodes {
    fn own(&self) -> Option<String> {
        if let Some(fp) = lock(&self.fingerprint).clone() {
            return Some(fp);
        }
        self.with(|_| Ok(())).ok()?;
        lock(&self.fingerprint).clone()
    }

    fn source_name(&self, name: &str) -> String {
        format!("{} ({name})", idn_open_protocol::discovery::hostname())
    }

    fn apply(
        &self,
        site: &Site,
        machine: &str,
        settings: Map<String, Value>,
    ) -> Result<(), String> {
        site.apply_node_settings(machine, settings)
            .map(|_| ())
            .map_err(|e| e.message)
    }

    fn report(&self, site: &Site, machine: &str) -> Option<Element> {
        site.node_report(machine)
    }

    fn media_dir(&self) -> Result<PathBuf, String> {
        self.with(|n| n.media_dir())
    }

    fn send_media(&self, file: &Path, to: &str, replace: bool) -> Result<Option<String>, String> {
        let r = self.with(|n| {
            let mut e = Element::new("media_send")
                .with_attr("path", &file.to_string_lossy())
                .with_attr("to", to);
            if replace {
                e = e.with_attr("replace", "true");
            }
            n.request(e)
        })?;
        if r.name == "error" {
            return Err(r.attr("text").unwrap_or("refused").to_string());
        }
        Ok(r.attr("offer").filter(|o| !o.is_empty()).map(str::to_owned))
    }

    fn transfer(&self, offer: &str, _to: &str) -> Option<Transfer> {
        let media = self.with(|n| n.request(Element::new("media"))).ok()?;
        let t = media
            .children_named("transfer")
            .find(|t| t.attr("way") == Some("out") && t.attr("offer") == Some(offer))?;
        Some(Transfer {
            state: t.attr("state")?.to_owned(),
            done: t.attr("done").and_then(|d| d.parse().ok()).unwrap_or(0),
        })
    }
}
