use crate::relay::models::*;
use egui::Context as EguiContext;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, tungstenite::client::IntoClientRequest};
use url::Url;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::time::Duration;

pub enum RelayEvent {
    Connected,
    Connecting,
    Disconnected,
    Error(String),
    Message(WeeChatResponse),
}

pub struct RelayClient {
    tx: mpsc::UnboundedSender<ClientCommand>,
}

enum ClientCommand {
    Text(String),
    Disconnect,
}

impl RelayClient {
    pub fn connect(
        host: String,
        port: u16,
        password: String,
        use_ssl: bool,
        event_tx: mpsc::UnboundedSender<RelayEvent>,
        ctx: EguiContext,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<ClientCommand>();

        let host_clean = host.trim()
            .replace("https://", "")
            .replace("http://", "")
            .replace("wss://", "")
            .replace("ws://", "");

        let scheme = if use_ssl { "wss" } else { "ws" };
        let url_str = format!("{}://{}:{}/api", scheme, host_clean, port);

        macro_rules! send {
            ($tx:expr, $ctx:expr, $ev:expr) => {{
                let _ = $tx.send($ev);
                $ctx.request_repaint();
            }};
        }

        tokio::spawn(async move {
            let password = password;
            let host_clone = host_clean;
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);

            loop {
                let url = match Url::parse(&url_str) {
                    Ok(url) => url,
                    Err(e) => {
                        send!(event_tx, ctx, RelayEvent::Error(format!("Invalid URL: {}", e)));
                        return;
                    }
                };

                send!(event_tx, ctx, RelayEvent::Connecting);

                let auth_string = format!("plain:{}", password);
                let base64_auth = URL_SAFE_NO_PAD.encode(auth_string.as_bytes());
                let auth_protocol = format!("api.weechat, base64url.bearer.authorization.weechat.{}", base64_auth);

                let mut request = match url.into_client_request() {
                    Ok(req) => req,
                    Err(e) => {
                        send!(event_tx, ctx, RelayEvent::Error(format!("Request error: {}", e)));
                        return;
                    }
                };

                let headers = request.headers_mut();
                headers.insert("Origin", format!("https://{}", host_clone).parse().unwrap());
                headers.insert("Sec-WebSocket-Protocol", auth_protocol.parse().unwrap());
                headers.insert("Authorization", format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(auth_string.as_bytes())).parse().unwrap());

                match connect_async(request).await {
                    Ok((ws_stream, _)) => {
                        send!(event_tx, ctx, RelayEvent::Connected);
                        backoff = Duration::from_secs(1);

                        let (mut ws_tx, mut ws_rx) = ws_stream.split();
                        let mut disconnected_cleanly = false;

                        loop {
                            tokio::select! {
                                Some(msg) = ws_rx.next() => {
                                    match msg {
                                        Ok(Message::Text(text)) => {
                                            if let Ok(resp) = serde_json::from_str::<WeeChatResponse>(&text) {
                                                send!(event_tx, ctx, RelayEvent::Message(resp));
                                            }
                                        }
                                        Ok(Message::Close(_)) => {
                                            send!(event_tx, ctx, RelayEvent::Disconnected);
                                            break;
                                        }
                                        Err(e) => {
                                            send!(event_tx, ctx, RelayEvent::Error(format!("Read error: {}", e)));
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                Some(cmd) = rx.recv() => {
                                    match cmd {
                                        ClientCommand::Text(text) => {
                                            if let Err(e) = ws_tx.send(Message::Text(text)).await {
                                                send!(event_tx, ctx, RelayEvent::Error(format!("Send error: {}", e)));
                                                break;
                                            }
                                        }
                                        ClientCommand::Disconnect => {
                                            let _ = ws_tx.send(Message::Close(None)).await;
                                            send!(event_tx, ctx, RelayEvent::Disconnected);
                                            disconnected_cleanly = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if disconnected_cleanly {
                            return;
                        }
                    }
                    Err(e) => {
                        send!(event_tx, ctx, RelayEvent::Error(format!("Connection failed: {}", e)));
                    }
                }

                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        });

        Self { tx }
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(ClientCommand::Disconnect);
    }

    pub fn send_api(&self, request: &str, id: Option<&str>, body: Option<serde_json::Value>) {
        let mut payload = json!({
            "request": request,
        });
        if let Some(id) = id {
            payload["request_id"] = json!(id);
        }
        if let Some(body) = body {
            payload["body"] = body;
        }

        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = self.tx.send(ClientCommand::Text(json));
        }
    }
}
