//! mDNS / DNS-SD discovery (feature `discover`).
//!
//! The server advertises itself as [`MDNS_SERVICE_TYPE`] (`_vesper._tcp.local.`) with the TXT
//! keys in [`crate::txt`] (`id`, `name`, `version`, `proto`). Clients browse for that type:
//!
//! ```no_run
//! let servers = protocol::discover::discover(std::time::Duration::from_secs(2))?;
//! for s in &servers {
//!     println!("{} at {}", s.name, s.base_url().unwrap_or_default());
//! }
//! # Ok::<(), mdns_sd::Error>(())
//! ```
//!
//! Uses the pure-Rust `mdns-sd` responder, which shares UDP port 5353 with a system responder
//! (Avahi, Bonjour) through `SO_REUSEPORT` / `SO_REUSEADDR`.

use crate::{MDNS_SERVICE_TYPE, PROTOCOL_VERSION, txt};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub use mdns_sd::Error;

/// A server found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// Server ID (TXT `id`); the key to remember a server by (names and addresses change).
    pub id: String,
    /// Display name (TXT `name`).
    pub name: String,
    /// Server software version (TXT `version`).
    pub version: String,
    /// Protocol major version (TXT `proto`; 0 when missing).
    pub protocol_version: u32,
    /// mDNS host name (`box.local.`).
    pub host: String,
    /// Addresses, IPv4 first.
    pub addresses: Vec<IpAddr>,
    /// TCP port.
    pub port: u16,
    /// DNS-SD instance full name (for matching removals).
    pub fullname: String,
}

impl DiscoveredServer {
    /// Whether this client can talk to it.
    pub fn compatible(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }

    /// `http://<first address>:<port>` (IPv6 in brackets), or `None` without addresses.
    pub fn base_url(&self) -> Option<String> {
        self.addresses.first().map(|a| match a {
            IpAddr::V4(v4) => format!("http://{v4}:{}", self.port),
            IpAddr::V6(v6) => format!("http://[{v6}]:{}", self.port),
        })
    }

    fn from_resolved(r: &mdns_sd::ResolvedService) -> Option<Self> {
        let get = |k: &str| r.get_property_val_str(k).map(str::to_owned);
        let id = get(txt::ID)?;
        // IPv6 link-local addresses are reported once per interface and are unusable without
        // their scope ID, so they are left out; the rest is deduplicated, IPv4 first.
        let mut addresses: Vec<IpAddr> = r
            .get_addresses()
            .iter()
            .map(|a| a.to_ip_addr())
            .filter(|a| !matches!(a, IpAddr::V6(v6) if v6.is_unicast_link_local()))
            .collect();
        addresses.sort_by_key(|a| (a.is_ipv6(), a.is_loopback(), *a));
        addresses.dedup();
        Some(DiscoveredServer {
            name: get(txt::NAME).unwrap_or_else(|| id.clone()),
            id,
            version: get(txt::VERSION).unwrap_or_default(),
            protocol_version: get(txt::PROTOCOL).and_then(|p| p.parse().ok()).unwrap_or(0),
            host: r.get_hostname().to_owned(),
            addresses,
            port: r.get_port(),
            fullname: r.get_fullname().to_owned(),
        })
    }
}

/// A discovery event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A server appeared or its record changed.
    Found(DiscoveredServer),
    /// A server went away (instance full name).
    Lost(String),
}

/// A running browse: poll it for events (e.g. from a UI thread or a Tauri command).
pub struct Browser {
    daemon: ServiceDaemon,
    rx: mdns_sd::Receiver<ServiceEvent>,
}

impl Browser {
    /// Starts browsing for servers.
    pub fn start() -> Result<Browser, Error> {
        let daemon = ServiceDaemon::new()?;
        let rx = daemon.browse(MDNS_SERVICE_TYPE)?;
        Ok(Browser { daemon, rx })
    }

    /// The next event, waiting at most `timeout` (`None` on timeout).
    pub fn next_event(&self, timeout: Duration) -> Option<DiscoveryEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left).ok()? {
                ServiceEvent::ServiceResolved(r) => {
                    if let Some(s) = DiscoveredServer::from_resolved(&r) {
                        return Some(DiscoveryEvent::Found(s));
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    return Some(DiscoveryEvent::Lost(fullname));
                }
                _ => {}
            }
        }
    }

    /// Stops browsing.
    pub fn stop(self) {
        let _ = self.daemon.stop_browse(MDNS_SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

/// Browses for `timeout` and returns every server found (deduplicated by ID, sorted by name).
pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredServer>, Error> {
    let browser = Browser::start()?;
    let deadline = Instant::now() + timeout;
    let mut found: BTreeMap<String, DiscoveredServer> = BTreeMap::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match browser.next_event(left) {
            Some(DiscoveryEvent::Found(s)) => {
                found.insert(s.id.clone(), s);
            }
            Some(DiscoveryEvent::Lost(name)) => found.retain(|_, s| s.fullname != name),
            None => break,
        }
    }
    browser.stop();
    let mut v: Vec<DiscoveredServer> = found.into_values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(v)
}

/// What the server advertises.
#[derive(Clone, Debug)]
pub struct Advertisement {
    /// Server ID.
    pub id: String,
    /// Display name (also the DNS-SD instance name).
    pub name: String,
    /// Software version.
    pub version: String,
    /// TCP port.
    pub port: u16,
}

/// A running advertisement; unregisters when stopped or dropped.
pub struct Advertiser {
    daemon: Option<ServiceDaemon>,
    fullname: String,
}

impl Advertiser {
    /// Registers the service on every interface (addresses follow interface changes).
    pub fn start(ad: &Advertisement) -> Result<Advertiser, Error> {
        let daemon = ServiceDaemon::new()?;
        let host = format!("{}.local.", host_label(&ad.name, &ad.id));
        let proto = PROTOCOL_VERSION.to_string();
        let props = [
            (txt::ID, ad.id.as_str()),
            (txt::NAME, ad.name.as_str()),
            (txt::VERSION, ad.version.as_str()),
            (txt::PROTOCOL, proto.as_str()),
        ];
        let info = ServiceInfo::new(MDNS_SERVICE_TYPE, &ad.name, &host, (), ad.port, &props[..])?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        daemon.register(info)?;
        Ok(Advertiser {
            daemon: Some(daemon),
            fullname,
        })
    }

    /// The DNS-SD instance full name.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Unregisters (sends goodbye packets) and stops the responder.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(d) = self.daemon.take() {
            if let Ok(rx) = d.unregister(&self.fullname) {
                let _ = rx.recv_timeout(Duration::from_secs(1));
            }
            let _ = d.shutdown();
        }
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A DNS host label for the advertisement: the name reduced to `[a-z0-9-]` plus a short ID
/// suffix, so two servers named alike never claim the same host name (and never clash with
/// the machine's own host name, which Avahi owns).
fn host_label(name: &str, id: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s = s.trim_matches('-').chars().take(40).collect();
    if s.is_empty() {
        s = crate::PRODUCT_ID.to_owned();
    }
    let suffix: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!("{s}-{}-{suffix}", crate::PRODUCT_ID)
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_labels_are_dns_safe() {
        let l = super::host_label("St. Mark's Booth", "1b2c3d4e-0000");
        assert_eq!(l, "st--mark-s-booth-vesper-1b2c3d4e");
    }
}
