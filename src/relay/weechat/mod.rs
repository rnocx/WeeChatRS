pub mod event_handler;

use crate::relay::backend::{BackendClient, BackendEvent};
use crate::relay::models::*;
use egui::Context as EguiContext;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector,
    tungstenite::client::IntoClientRequest,
    tungstenite::protocol::Message,
};
use url::Url;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

// ── Internal command channel ──────────────────────────────────────────────────

enum ClientCommand {
    Text(String),
    Disconnect,
}

// ── Config (cheaply cloneable, passed into the reconnect loop) ────────────────

#[derive(Clone)]
pub struct WeeChatConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
    pub use_ssl: bool,
    pub accept_invalid_certs: bool,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct WeeChatClient {
    config: Arc<WeeChatConfig>,
    event_tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: EguiContext,
    /// Channel to send commands into the running connection task.
    cmd_tx: mpsc::UnboundedSender<ClientCommand>,
    /// Tracks whether the task has signalled Connected at least once.
    connected: Arc<AtomicBool>,
}

impl WeeChatClient {
    pub fn new(
        config: WeeChatConfig,
        event_tx: mpsc::UnboundedSender<BackendEvent>,
        ctx: EguiContext,
    ) -> Self {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel::<ClientCommand>();
        Self {
            config: Arc::new(config),
            event_tx,
            ctx,
            cmd_tx,
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send a WeeChat relay API request directly.
    pub fn send_api(
        &self,
        request: &str,
        id: Option<&str>,
        body: Option<serde_json::Value>,
    ) {
        let mut payload = json!({ "request": request });
        if let Some(id) = id {
            payload["request_id"] = json!(id);
        }
        if let Some(body) = body {
            payload["body"] = body;
        }
        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = self.cmd_tx.send(ClientCommand::Text(json));
        }
    }
}

impl BackendClient for WeeChatClient {
    fn connect(&mut self) {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ClientCommand>();
        self.cmd_tx = cmd_tx;

        let config = Arc::clone(&self.config);
        let event_tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        let connected = Arc::clone(&self.connected);

        macro_rules! send {
            ($ev:expr) => {{
                let _ = event_tx.send($ev);
                ctx.request_repaint();
            }};
        }

        let host_clean = config.host.trim()
            .replace("https://", "").replace("http://", "")
            .replace("wss://", "").replace("ws://", "");
        let scheme = if config.use_ssl { "wss" } else { "ws" };
        let url_str = format!("{}://{}:{}/api", scheme, host_clean, config.port);

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);

            loop {
                let url = match Url::parse(&url_str) {
                    Ok(u) => u,
                    Err(e) => {
                        send!(BackendEvent::Error(format!("Invalid URL: {}", e)));
                        return;
                    }
                };

                let auth_string = format!("plain:{}", config.password);
                let base64_auth = URL_SAFE_NO_PAD.encode(auth_string.as_bytes());
                let auth_protocol = format!(
                    "api.weechat, base64url.bearer.authorization.weechat.{}",
                    base64_auth
                );

                let mut request = match url.into_client_request() {
                    Ok(r) => r,
                    Err(e) => {
                        send!(BackendEvent::Error(format!("Request error: {}", e)));
                        return;
                    }
                };

                let origin = format!("https://{}", host_clean);
                let basic_auth = format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(auth_string.as_bytes())
                );
                let header_result: Result<(), String> = (|| {
                    let origin_v = origin.parse().map_err(|e: http::header::InvalidHeaderValue|
                        format!("invalid Origin header (bad host?): {}", e))?;
                    let auth_proto_v = auth_protocol.parse().map_err(|e: http::header::InvalidHeaderValue|
                        format!("invalid Sec-WebSocket-Protocol header: {}", e))?;
                    let basic_v = basic_auth.parse().map_err(|e: http::header::InvalidHeaderValue|
                        format!("invalid Authorization header: {}", e))?;
                    request.headers_mut().insert("Origin", origin_v);
                    request.headers_mut().insert("Sec-WebSocket-Protocol", auth_proto_v);
                    request.headers_mut().insert("Authorization", basic_v);
                    Ok(())
                })();
                if let Err(e) = header_result {
                    send!(BackendEvent::AuthError(format!("Connection setup failed: {}", e)));
                    return;
                }

                let connector = if config.use_ssl {
                    native_tls::TlsConnector::builder()
                        .danger_accept_invalid_certs(config.accept_invalid_certs)
                        .build()
                        .ok()
                        .map(Connector::NativeTls)
                } else {
                    None
                };

                match connect_async_tls_with_config(request, None, false, connector).await {
                    Ok((ws_stream, _)) => {
                        connected.store(true, Ordering::Relaxed);
                        send!(BackendEvent::Connected);
                        backoff = Duration::from_secs(1);

                        let (mut ws_tx, mut ws_rx) = ws_stream.split();
                        let mut clean = false;
                        let mut last_received = std::time::Instant::now();

                        // Check for dead connection every 60 s; disconnect if silent for 5 min.
                        // We do NOT send WebSocket Ping frames because many relay setups (nginx
                        // proxies, WeeChat itself) close the connection on receiving an
                        // application-initiated Ping, causing spurious reconnect loops.
                        let mut idle_check = tokio::time::interval(Duration::from_secs(60));
                        idle_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        idle_check.tick().await; // skip the immediate first tick

                        loop {
                            tokio::select! {
                                Some(msg) = ws_rx.next() => match msg {
                                    Ok(Message::Text(text)) => {
                                        last_received = std::time::Instant::now();
                                        if let Ok(resp) = serde_json::from_str::<WeeChatResponse>(&text) {
                                            // WeeChatResponse → BackendEvent translation happens
                                            // inside event_handler (still on WeeChatApp).
                                            // For now we tunnel the raw response through a
                                            // temporary variant; Phase 3 removes this.
                                            let _ = event_tx.send(BackendEvent::_WeeChat(resp));
                                            ctx.request_repaint();
                                        }
                                    }
                                    Ok(Message::Ping(data)) => {
                                        last_received = std::time::Instant::now();
                                        let _ = ws_tx.send(Message::Pong(data)).await;
                                    }
                                    Ok(Message::Pong(_)) => {
                                        last_received = std::time::Instant::now();
                                    }
                                    Ok(Message::Close(_)) => {
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        break;
                                    }
                                    Err(e) => {
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Error(format!("Read error: {}", e)));
                                        break;
                                    }
                                    _ => {}
                                },
                                Some(cmd) = cmd_rx.recv() => match cmd {
                                    ClientCommand::Text(t) => {
                                        if let Err(e) = ws_tx.send(Message::Text(t)).await {
                                            send!(BackendEvent::Error(format!("Send error: {}", e)));
                                            break;
                                        }
                                    }
                                    ClientCommand::Disconnect => {
                                        let _ = ws_tx.send(Message::Close(None)).await;
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        clean = true;
                                        break;
                                    }
                                },
                                _ = idle_check.tick() => {
                                    if last_received.elapsed() > Duration::from_secs(300) {
                                        // No data from server in 5 min — connection is dead
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        break;
                                    }
                                }
                            }
                        }

                        if clean { return; }
                    }
                    Err(e) => {
                        connected.store(false, Ordering::Relaxed);
                        // tungstenite returns Error::Http(resp) on a non-101 handshake response.
                        // Status 401/403 mean wrong password / forbidden — retrying won't help.
                        let is_auth_failure = match &e {
                            tokio_tungstenite::tungstenite::Error::Http(resp) => {
                                let s = resp.status().as_u16();
                                s == 401 || s == 403
                            }
                            _ => false,
                        };
                        let msg = format!("Connection failed: {}", e);
                        if is_auth_failure {
                            send!(BackendEvent::AuthError(msg));
                            return;
                        } else {
                            send!(BackendEvent::Error(msg));
                        }
                    }
                }

                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        });
    }

    fn disconnect(&mut self) {
        let _ = self.cmd_tx.send(ClientCommand::Disconnect);
    }

    fn send_message(&self, buffer_id: &str, text: &str) {
        self.send_api(
            "POST /api/input",
            None,
            Some(json!({ "buffer_id": buffer_id.parse::<i64>().unwrap_or(0), "command": text })),
        );
    }

    fn fetch_lines(&self, buffer_id: &str, count: usize) {
        self.send_api(
            &format!("GET /api/buffers/{}/lines?lines=-{}", buffer_id, count),
            Some(&format!("_buffer_lines:{}", buffer_id)),
            None,
        );
    }

    fn fetch_nicks(&self, buffer_id: &str) {
        self.send_api(
            &format!("GET /api/buffers/{}/nicks", buffer_id),
            Some(&format!("_nicks:{}", buffer_id)),
            None,
        );
    }

    fn mark_read(&self, buffer_id: &str) {
        // REST endpoint (WeeChat 4.3+): sets the persistent last_read_line_ufr marker.
        self.send_api(
            &format!("POST /api/buffers/{}/read", buffer_id),
            None,
            None,
        );
        // Belt-and-suspenders for older relay versions: the input command clears the
        // in-memory hotlist entry immediately and also updates last_read_line_ufr.
        if let Ok(id) = buffer_id.parse::<i64>() {
            self.send_api(
                "POST /api/input",
                None,
                Some(serde_json::json!({
                    "buffer_id": id,
                    "command": "/buffer set hotlist -1"
                })),
            );
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    fn refresh_buffer(&self, buffer_id: &str) {
        self.send_api(
            &format!("GET /api/buffers/{}", buffer_id),
            Some(&format!("_buffer_info:{}", buffer_id)),
            None,
        );
    }

    fn fetch_buffer_list(&self) {
        self.send_api("GET /api/buffers", Some("_list_buffers"), None);
    }

    fn fetch_hotlist(&self) {
        self.send_api("GET /api/hotlist", Some("_hotlist"), None);
    }

    fn sync_subscriptions(&self) {
        self.send_api(
            "POST /api/sync",
            None,
            Some(serde_json::json!({"colors": "ansi", "input": false})),
        );
    }
}
