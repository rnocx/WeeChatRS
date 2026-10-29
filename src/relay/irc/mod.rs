pub mod connection;
pub mod parser;

use crate::relay::backend::{BackendClient, BackendEvent};
use egui::Context as EguiContext;
use tokio::sync::mpsc;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

use connection::{IrcCommand, NetworkUpdate};

#[derive(Clone)]
pub struct IrcConfig {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub nick: String,
    pub username: String,
    pub sasl_username: String,
    pub password: String,
    pub use_ssl: bool,
    pub accept_invalid_certs: bool,
    pub channel: String,
    pub tunnel_port: Option<u16>,
    pub auto_reconnect: bool,
    /// soju network to bind this connection to (BOUNCER BIND), for the per-network
    /// connections the router opens.
    pub bind_netid: Option<String>,
    /// On a soju login without a network, list the networks and open a connection
    /// per network. Off for the per-network connections themselves.
    pub list_networks: bool,
}

/// Command senders of the per-network connections, by buffer-id prefix (`net<id>`).
type NetworkSenders = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<IrcCommand>>>>;

pub struct IrcClient {
    config: Arc<IrcConfig>,
    event_tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: EguiContext,
    cmd_tx: mpsc::UnboundedSender<IrcCommand>,
    connected: Arc<AtomicBool>,
    networks: NetworkSenders,
    /// Set once any of the connections has negotiated soju.im/search.
    search_supported: Arc<AtomicBool>,
}

impl IrcClient {
    pub fn new(
        config: IrcConfig,
        event_tx: mpsc::UnboundedSender<BackendEvent>,
        ctx: EguiContext,
    ) -> Self {
        let (cmd_tx, _) = mpsc::unbounded_channel();
        Self {
            config: Arc::new(config),
            event_tx,
            ctx,
            cmd_tx,
            connected: Arc::new(AtomicBool::new(false)),
            networks: Arc::default(),
            search_supported: Arc::default(),
        }
    }

    /// The connection that owns `buffer_id`, and the buffer's id on that connection.
    /// Buffers of a bouncer network are `net<id>/<buffer>`; the rest belong to the
    /// main connection.
    fn route(&self, buffer_id: &str, f: impl FnOnce(&str) -> IrcCommand) {
        if let Some((key, rest)) = buffer_id.split_once('/') {
            if let Some(tx) = self.networks.lock().unwrap().get(key) {
                let _ = tx.send(f(rest));
                return;
            }
        }
        let _ = self.cmd_tx.send(f(buffer_id));
    }
}

impl BackendClient for IrcClient {
    fn connect(&mut self) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        self.cmd_tx = cmd_tx;
        let (main_tx, main_rx) = mpsc::unbounded_channel();
        let (network_tx, network_rx) = mpsc::unbounded_channel();
        connection::spawn(
            Arc::clone(&self.config),
            main_tx,
            self.ctx.clone(),
            cmd_rx,
            Arc::clone(&self.connected),
            Some(network_tx),
            Arc::clone(&self.search_supported),
        );
        tokio::spawn(run_router(Router {
            config: Arc::clone(&self.config),
            event_tx: self.event_tx.clone(),
            ctx: self.ctx.clone(),
            senders: Arc::clone(&self.networks),
            search_supported: Arc::clone(&self.search_supported),
            networks: HashMap::new(),
            line_counter: 0,
        }, main_rx, network_rx));
    }

    fn disconnect(&mut self) {
        let _ = self.cmd_tx.send(IrcCommand::Disconnect);
    }

    fn send_message(&self, buffer_id: &str, text: &str) {
        self.route(buffer_id, |id| IrcCommand::SendMessage {
            buffer_id: id.to_string(),
            text: text.to_string(),
        });
    }

    fn fetch_lines(&self, _buffer_id: &str, _count: usize) {
        // chathistory is requested automatically on JOIN
    }

    fn fetch_nicks(&self, buffer_id: &str) {
        self.route(buffer_id, |id| IrcCommand::FetchNicks { buffer_id: id.to_string() });
    }

    fn mark_read(&self, buffer_id: &str) {
        self.route(buffer_id, |id| IrcCommand::MarkRead { buffer_id: id.to_string() });
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    fn fetch_buffer_list(&self) {
        // IRC buffers are discovered passively via JOIN
    }

    fn supports_search(&self) -> bool {
        self.search_supported.load(Ordering::Relaxed)
    }

    fn search(&self, buffer_id: &str, text: &str, request: &str) {
        self.route(buffer_id, |id| IrcCommand::Search {
            buffer_id: id.to_string(),
            text: text.to_string(),
            request: request.to_string(),
        });
    }

    fn fetch_context(&self, buffer_id: &str, timestamp: &str, request: &str) {
        self.route(buffer_id, |id| IrcCommand::Context {
            buffer_id: id.to_string(),
            timestamp: timestamp.to_string(),
            request: request.to_string(),
        });
    }

    fn fetch_lines_before(&self, buffer_id: &str, before_ts: &str) {
        self.route(buffer_id, |id| IrcCommand::FetchBefore {
            buffer_id: id.to_string(),
            before_ts: before_ts.to_string(),
        });
    }
}

// ── Bouncer networks ─────────────────────────────────────────────────────────

/// One soju network and the connection bound to it.
struct Network {
    name: String,
    state: String,
    cmd_tx: mpsc::UnboundedSender<IrcCommand>,
    /// Set before the connection is told to quit, so nothing it sends afterwards
    /// reaches the UI (a new connection for the same network may already be open).
    stopped: Arc<AtomicBool>,
    /// Buffers it has open, by their id on that connection.
    buffers: Arc<Mutex<HashSet<String>>>,
}

struct Router {
    config: Arc<IrcConfig>,
    event_tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: EguiContext,
    senders: NetworkSenders,
    search_supported: Arc<AtomicBool>,
    networks: HashMap<String, Network>,
    line_counter: u64,
}

/// Forwards the main connection's events, and runs a connection per bouncer network
/// while the main connection lists them.
async fn run_router(
    mut router: Router,
    mut main_rx: mpsc::UnboundedReceiver<BackendEvent>,
    mut network_rx: mpsc::UnboundedReceiver<NetworkUpdate>,
) {
    loop {
        tokio::select! {
            event = main_rx.recv() => {
                let Some(event) = event else { break };
                // The UI drops all of this connection's buffers on either; the networks
                // are listed again after registration.
                if matches!(event, BackendEvent::Connected | BackendEvent::Disconnected) {
                    router.stop_all();
                }
                router.send(event);
            }
            Some(update) = network_rx.recv() => router.update(update),
        }
    }
    router.stop_all();
}

impl Router {
    fn send(&self, event: BackendEvent) {
        let _ = self.event_tx.send(event);
        self.ctx.request_repaint();
    }

    fn update(&mut self, update: NetworkUpdate) {
        match update {
            NetworkUpdate::Changed { netid, attrs } => {
                let key = network_key(&netid);
                if let Some(network) = self.networks.get_mut(&key) {
                    let mut notes = Vec::new();
                    if let Some(name) = attrs.get("name").filter(|n| **n != network.name) {
                        notes.push(format!("Network renamed to {}", name));
                        network.name = name.clone();
                    }
                    if let Some(state) = attrs.get("state").filter(|s| **s != network.state) {
                        network.state = state.clone();
                        notes.push(format!("Bouncer is {} to {}", state, network.name));
                    }
                    if let Some(error) = attrs.get("error").filter(|e| !e.is_empty()) {
                        notes.push(format!("Bouncer error: {}", error));
                    }
                    for note in notes {
                        self.status_line(&key, note);
                    }
                } else {
                    self.start(netid, &attrs);
                }
            }
            NetworkUpdate::Removed { netid } => {
                let key = network_key(&netid);
                if let Some(network) = self.networks.remove(&key) {
                    self.senders.lock().unwrap().remove(&key);
                    stop(&network);
                    let buffers: Vec<String> = network.buffers.lock().unwrap().drain().collect();
                    for id in buffers {
                        self.send(BackendEvent::BufferClosed { buffer_id: format!("{}/{}", key, id) });
                    }
                    self.send(BackendEvent::ConnLog(format!("Bouncer network {} was removed", network.name)));
                }
            }
        }
    }

    /// Open a connection bound to the network, its buffers grouped under `net<id>/`.
    fn start(&mut self, netid: String, attrs: &HashMap<String, String>) {
        let key = network_key(&netid);
        let name = attrs.get("name").filter(|n| !n.is_empty())
            .or_else(|| attrs.get("host"))
            .cloned()
            .unwrap_or_else(|| netid.clone());
        let config = IrcConfig {
            label: name.clone(),
            channel: String::new(),
            auto_reconnect: true,
            bind_netid: Some(netid.clone()),
            list_networks: false,
            ..(*self.config).clone()
        };
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        connection::spawn(Arc::new(config), event_tx, self.ctx.clone(), cmd_rx, Arc::default(), None,
            Arc::clone(&self.search_supported));

        let stopped = Arc::new(AtomicBool::new(false));
        let buffers: Arc<Mutex<HashSet<String>>> = Arc::default();
        // Group key for the buffer list, unique per bouncer and network.
        let group = format!("{}/{}", self.config.host.trim(), key);
        {
            let (key, name, stopped, buffers) = (key.clone(), name.clone(), Arc::clone(&stopped), Arc::clone(&buffers));
            let (ui_tx, ctx) = (self.event_tx.clone(), self.ctx.clone());
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    if stopped.load(Ordering::Relaxed) { continue; }
                    for event in network_event(event, &key, &name, &group, &buffers) {
                        let _ = ui_tx.send(event);
                    }
                    ctx.request_repaint();
                }
            });
        }
        self.senders.lock().unwrap().insert(key.clone(), cmd_tx.clone());
        self.networks.insert(key, Network {
            name,
            state: attrs.get("state").cloned().unwrap_or_default(),
            cmd_tx,
            stopped,
            buffers,
        });
    }

    fn stop_all(&mut self) {
        self.senders.lock().unwrap().clear();
        for (_, network) in self.networks.drain() {
            stop(&network);
        }
    }

    fn status_line(&mut self, key: &str, text: String) {
        self.line_counter += 1;
        let line = crate::relay::models::Line::new(
            format!("bouncer-{}", self.line_counter), chrono::Utc::now(),
            "--".to_string(), text, true, false,
        );
        self.send(BackendEvent::LineAdded { buffer_id: format!("{}/status", key), line });
    }
}

fn network_key(netid: &str) -> String {
    format!("net{}", netid)
}

fn stop(network: &Network) {
    network.stopped.store(true, Ordering::Relaxed);
    let _ = network.cmd_tx.send(IrcCommand::Disconnect);
}

/// A per-network connection's event as the UI sees it: buffer ids under `key/`,
/// buffers grouped by `group`, and connection state reported in the log only, since
/// the network's connection coming and going isn't the profile's.
fn network_event(
    event: BackendEvent,
    key: &str,
    name: &str,
    group: &str,
    buffers: &Mutex<HashSet<String>>,
) -> Vec<BackendEvent> {
    let id = |buffer_id: String| format!("{}/{}", key, buffer_id);
    vec![match event {
        BackendEvent::Connected => BackendEvent::ConnLog(format!("{}: connected", name)),
        BackendEvent::Disconnected => {
            // It reconnects on its own and opens its buffers again.
            let mut events: Vec<BackendEvent> = buffers.lock().unwrap().drain()
                .map(|b| BackendEvent::BufferClosed { buffer_id: id(b) })
                .collect();
            events.push(BackendEvent::ConnLog(format!("{}: disconnected", name)));
            return events;
        }
        BackendEvent::Error(e) | BackendEvent::AuthError(e) => BackendEvent::ConnLog(format!("{}: {}", name, e)),
        BackendEvent::ConnLog(m) => BackendEvent::ConnLog(format!("{}: {}", name, m)),
        BackendEvent::BufferOpened(mut buffer) => {
            buffers.lock().unwrap().insert(buffer.id.clone());
            buffer.id = id(buffer.id);
            buffer.server = group.to_string();
            // Numbered by network, so network groups keep soju's order however their
            // connections race: network 2's buffers come after network 1's.
            let netid: i32 = key.trim_start_matches("net").parse().unwrap_or(0);
            buffer.number = netid.saturating_mul(10_000).saturating_add(buffer.number);
            BackendEvent::BufferOpened(buffer)
        }
        BackendEvent::BufferClosed { buffer_id } => {
            buffers.lock().unwrap().remove(&buffer_id);
            BackendEvent::BufferClosed { buffer_id: id(buffer_id) }
        }
        BackendEvent::BuffersLoaded(list) => BackendEvent::BuffersLoaded(list.into_iter().map(|mut b| {
            b.id = id(b.id);
            b.server = group.to_string();
            b
        }).collect()),
        BackendEvent::LineAdded { buffer_id, line } => BackendEvent::LineAdded { buffer_id: id(buffer_id), line },
        BackendEvent::NicklistLoaded { buffer_id, nicks } => BackendEvent::NicklistLoaded { buffer_id: id(buffer_id), nicks },
        BackendEvent::NickAdded { buffer_id, nick } => BackendEvent::NickAdded { buffer_id: id(buffer_id), nick },
        BackendEvent::NickRemoved { buffer_id, nick_name } => BackendEvent::NickRemoved { buffer_id: id(buffer_id), nick_name },
        BackendEvent::NickAwayChanged { buffer_id, nick_name, away } => BackendEvent::NickAwayChanged { buffer_id: id(buffer_id), nick_name, away },
        BackendEvent::ActivityChanged { buffer_id, activity, unread_count, markread_ts } => BackendEvent::ActivityChanged { buffer_id: id(buffer_id), activity, unread_count, markread_ts },
        BackendEvent::TopicChanged { buffer_id, topic } => BackendEvent::TopicChanged { buffer_id: id(buffer_id), topic },
        BackendEvent::BufferHidden { buffer_id, hidden } => BackendEvent::BufferHidden { buffer_id: id(buffer_id), hidden },
        BackendEvent::LinesLoaded { buffer_id, lines, is_prepend } => BackendEvent::LinesLoaded { buffer_id: id(buffer_id), lines, is_prepend },
        BackendEvent::HistoryResults { request, lines, error } => BackendEvent::HistoryResults {
            request,
            lines: lines.into_iter().map(|(b, l)| (id(b), l)).collect(),
            error,
        },
        BackendEvent::_WeeChat(r) => BackendEvent::_WeeChat(r),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::models::{Buffer, Line};

    fn buffer(id: &str) -> Buffer {
        Buffer {
            id: id.into(), number: 1, name: id.into(), full_name: format!("irc.{}", id),
            plugin: "irc".into(), kind: "channel".into(), server: "soju.test".into(),
            messages: Default::default(), nicks: Default::default(),
            activity: crate::relay::models::BufferActivity::None, unread_count: 0,
            last_read_id: None, last_markread_ts: None, topic: String::new(), modes: String::new(),
            hidden: false, muted: false, has_nicklist: true, visit_start_marker_id: None,
        }
    }

    #[test]
    fn network_buffers_are_namespaced_and_closed_on_disconnect() {
        let buffers = Mutex::new(HashSet::new());
        let opened = network_event(BackendEvent::BufferOpened(buffer("#rust")), "net7", "Libera", "soju.test/net7", &buffers);
        match &opened[..] {
            [BackendEvent::BufferOpened(b)] => {
                assert_eq!(b.id, "net7/#rust");
                assert_eq!(b.server, "soju.test/net7");
            }
            _ => panic!("expected BufferOpened"),
        }
        let line = Line::new("1".into(), chrono::Utc::now(), "nyx".into(), "hi".into(), true, false);
        match &network_event(BackendEvent::LineAdded { buffer_id: "#rust".into(), line }, "net7", "Libera", "g", &buffers)[..] {
            [BackendEvent::LineAdded { buffer_id, .. }] => assert_eq!(buffer_id, "net7/#rust"),
            _ => panic!("expected LineAdded"),
        }
        // The network's own connection state only goes to the log, and its buffers close.
        let closed = network_event(BackendEvent::Disconnected, "net7", "Libera", "g", &buffers);
        assert!(matches!(&closed[0], BackendEvent::BufferClosed { buffer_id } if buffer_id == "net7/#rust"));
        assert!(matches!(&closed[1], BackendEvent::ConnLog(m) if m == "Libera: disconnected"));
        assert!(buffers.lock().unwrap().is_empty());
    }
}
