//! Native Injected cloud client. Protocol mapped from the shipped 1.1.3 client.
//! This connects to the account's saved networks; it does not embed Electron.
pub(crate) mod login;
#[cfg(web_login)]
mod bridge;
#[cfg(web_login)]
pub(crate) mod web_login;
#[cfg(feature = "injected-web-login")]
pub(crate) mod web_window;
mod protocol;
mod paste;
mod session;

use crate::relay::backend::{BackendClient, BackendEvent};

/// How a capped read of a response failed.
pub(crate) enum BodyError {
    Read,
    TooLarge,
}

/// The body of a response, up to `max` bytes. The server is the user's own choice,
/// but nothing says its answer has to be small.
pub(crate) async fn read_capped(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, BodyError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| BodyError::Read)? {
        if bytes.len() + chunk.len() > max {
            return Err(BodyError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

use egui::Context;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct InjectedConfig {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub use_ssl: bool,
    pub auto_reconnect: bool,
}

pub(super) enum Command {
    Disconnect,
    Send(String, String),
    History(String, usize, Option<i64>),
    Nicks(String),
    Read(String),
    /// LIST on the server of this buffer, for channels with at least this many users.
    List(String, u32),
    /// Our typing state in a conversation.
    Typing(String, crate::relay::backend::TypingState),
}

pub struct InjectedClient {
    config: Arc<InjectedConfig>,
    tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: Context,
    commands: Option<mpsc::UnboundedSender<Command>>,
    connected: Arc<AtomicBool>,
}

impl InjectedClient {
    pub fn new(
        config: InjectedConfig,
        tx: mpsc::UnboundedSender<BackendEvent>,
        ctx: Context,
    ) -> Self {
        Self {
            config: Arc::new(config),
            tx,
            ctx,
            commands: None,
            connected: Arc::new(AtomicBool::new(false)),
        }
    }
    fn command(&self, c: Command) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(c);
        }
    }
}
impl Drop for InjectedClient {
    fn drop(&mut self) {
        self.command(Command::Disconnect);
    }
}
impl BackendClient for InjectedClient {
    fn connect(&mut self) {
        self.command(Command::Disconnect);
        let (tx, rx) = mpsc::unbounded_channel();
        self.commands = Some(tx);
        session::spawn(
            self.config.clone(),
            self.tx.clone(),
            self.ctx.clone(),
            rx,
            self.connected.clone(),
        );
    }
    fn disconnect(&mut self) {
        self.command(Command::Disconnect);
    }
    fn send_message(&self, buffer_id: &str, text: &str) {
        self.command(Command::Send(buffer_id.into(), text.into()));
    }
    fn supports_paste_upload(&self) -> bool { true }
    fn supports_channel_list(&self) -> bool { true }
    fn list_channels(&self, buffer_id: &str, min_users: u32) {
        self.command(Command::List(buffer_id.into(), min_users));
    }
    fn create_paste(&self, content: String) -> crate::relay::backend::PasteUpload {
        let config = self.config.clone();
        Box::pin(async move { paste::create(&config, content).await })
    }
    fn fetch_lines(&self, buffer_id: &str, count: usize) {
        self.command(Command::History(buffer_id.into(), count, None));
    }
    fn fetch_lines_before(&self, buffer_id: &str, before: &str) {
        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(before) {
            self.command(Command::History(
                buffer_id.into(),
                100,
                Some(ts.timestamp_millis()),
            ));
        }
    }
    fn fetch_nicks(&self, buffer_id: &str) {
        self.command(Command::Nicks(buffer_id.into()));
    }
    fn mark_read(&self, buffer_id: &str) {
        self.command(Command::Read(buffer_id.into()));
    }
    fn send_typing(&self, buffer_id: &str, state: crate::relay::backend::TypingState) {
        use crate::relay::backend::TypingState;
        let state = if state == TypingState::Sent { TypingState::Done } else { state };
        self.command(Command::Typing(buffer_id.into(), state));
    }
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn answer_with(len: usize) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&vec![b'x'; len]).await;
        });
        reqwest::get(format!("http://127.0.0.1:{port}/")).await.unwrap()
    }

    #[tokio::test]
    async fn a_body_is_read_up_to_its_cap() {
        assert_eq!(read_capped(answer_with(1000).await, 1000).await.ok().map(|b| b.len()), Some(1000));
        assert!(matches!(read_capped(answer_with(1001).await, 1000).await, Err(BodyError::TooLarge)));
    }
}
