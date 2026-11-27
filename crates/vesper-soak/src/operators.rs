//! Operators: WebSocket clients of the server (`/api/v1/ws`), the way Oriel talks to it. Each
//! runs on a thread of its own that reads everything the server sends (states, events) without
//! falling behind, sends commands and heartbeats, and measures round trips.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

/// Round trips and failures, shared by every operator; the watcher takes them every sample.
#[derive(Default)]
pub struct WsMetrics {
    /// Heartbeat (`ping` → `pong`) round trips since the last sample, ms.
    pub pings: Mutex<Vec<f64>>,
    /// Command → result round trips since the last sample, ms.
    pub commands: Mutex<Vec<f64>>,
    /// Commands the server refused (`ok: false`) or never answered.
    pub command_errors: AtomicU64,
    /// Commands sent.
    pub commands_sent: AtomicU64,
    /// Planned disconnects (network drops the soak makes).
    pub drops: AtomicU64,
    /// When the server (or the network) closed a connection on its own (the summary leaves
    /// out those its shutdown closed).
    pub lost_at: Mutex<Vec<Instant>>,
    /// The last refusals, for the summary.
    pub last_errors: Mutex<Vec<String>>,
}

impl WsMetrics {
    fn error(&self, what: String) {
        self.command_errors.fetch_add(1, Ordering::Relaxed);
        let mut l = self.last_errors.lock().unwrap();
        if l.len() >= 20 {
            l.remove(0);
        }
        l.push(what);
    }
}

type Pending = Arc<Mutex<HashMap<u64, (Instant, Sender<Value>)>>>;

/// One connected operator.
pub struct Operator {
    pub name: String,
    out: Sender<String>,
    pending: Pending,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
    metrics: Arc<WsMetrics>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Operator {
    /// Connects to the server at `addr` as `name` (the server machine needs no key).
    pub fn connect(addr: SocketAddr, name: &str, metrics: Arc<WsMetrics>) -> Result<Operator> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        stream.set_nodelay(true)?;
        let url = format!("ws://{addr}/api/v1/ws?protocolVersion=1");
        let (mut ws, _) = tungstenite::client(url.as_str(), stream)
            .map_err(|e| anyhow!("WebSocket handshake: {e}"))?;
        ws.send(Message::text(
            json!({"type": "hello", "protocolVersion": 1, "clientName": name}).to_string(),
        ))?;
        ws.get_mut()
            .set_read_timeout(Some(Duration::from_millis(20)))?;
        let (tx, rx) = channel::<String>();
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let thread = {
            let (pending, closed, metrics) = (pending.clone(), closed.clone(), metrics.clone());
            let name = name.to_string();
            std::thread::Builder::new()
                .name(format!("op {name}"))
                .spawn(move || run(ws, rx, pending, closed, metrics))?
        };
        Ok(Operator {
            name: name.to_string(),
            out: tx,
            pending,
            next_id: AtomicU64::new(1),
            closed,
            metrics,
            thread: Some(thread),
        })
    }

    /// The connection is gone.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// Sends a command and waits for its result (at most 10 s). A refusal is an error.
    pub fn command(&self, cmd: Value) -> Result<Value> {
        let kind = cmd["type"].as_str().unwrap_or("?").to_string();
        let r = self.command_raw(cmd)?;
        if r["ok"] != true {
            let e = format!("{} {kind}: {}", self.name, r["error"]);
            self.metrics.error(e.clone());
            bail!(e);
        }
        Ok(r)
    }

    /// Sends a command; the result as the server sent it.
    pub fn command_raw(&self, cmd: Value) -> Result<Value> {
        if self.is_closed() {
            bail!("{}: not connected", self.name);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending
            .lock()
            .unwrap()
            .insert(id, (Instant::now(), tx));
        self.metrics.commands_sent.fetch_add(1, Ordering::Relaxed);
        self.out
            .send(json!({"type": "command", "id": id, "command": cmd}).to_string())
            .context("operator thread gone")?;
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                let e = format!("{}: no result for {} in 10 s", self.name, cmd_type(&cmd));
                self.metrics.error(e.clone());
                bail!(e)
            }
        }
    }

    /// Closes the connection (a planned network drop).
    pub fn drop_connection(mut self) {
        self.metrics.drops.fetch_add(1, Ordering::Relaxed);
        self.close();
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Operator {
    fn drop(&mut self) {
        self.close();
    }
}

fn cmd_type(c: &Value) -> String {
    c["type"].as_str().unwrap_or("?").to_string()
}

/// The operator's thread: drains the socket, sends what is queued, a heartbeat every 2 s.
fn run(
    mut ws: WebSocket<TcpStream>,
    rx: Receiver<String>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    metrics: Arc<WsMetrics>,
) {
    let mut last_ping = Instant::now();
    let mut pings: HashMap<u64, Instant> = HashMap::new();
    let mut ping_id = 0u64;
    loop {
        if closed.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            let _ = ws.flush();
            return;
        }
        while let Ok(m) = rx.try_recv() {
            if ws.send(Message::text(m)).is_err() {
                break;
            }
        }
        if last_ping.elapsed() >= Duration::from_secs(2) {
            last_ping = Instant::now();
            ping_id += 1;
            pings.insert(ping_id, Instant::now());
            pings.retain(|_, t| t.elapsed() < Duration::from_secs(60));
            let _ = ws.send(Message::text(
                json!({"type": "ping", "id": ping_id}).to_string(),
            ));
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else {
                    continue;
                };
                match v["type"].as_str() {
                    Some("result") => {
                        if let Some(id) = v["id"].as_u64()
                            && let Some((at, tx)) = pending.lock().unwrap().remove(&id)
                        {
                            metrics
                                .commands
                                .lock()
                                .unwrap()
                                .push(at.elapsed().as_secs_f64() * 1000.0);
                            let _ = tx.send(v);
                        }
                    }
                    Some("pong") => {
                        if let Some(at) = v["id"].as_u64().and_then(|id| pings.remove(&id)) {
                            metrics
                                .pings
                                .lock()
                                .unwrap()
                                .push(at.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                    _ => {}
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                if !closed.swap(true, Ordering::Relaxed) {
                    metrics.lost_at.lock().unwrap().push(Instant::now());
                }
                pending.lock().unwrap().clear();
                return;
            }
        }
    }
}
