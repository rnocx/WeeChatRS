//! Native Injected cloud client. Protocol mapped from the shipped 1.1.3 client.
//! This connects to the account's saved networks; it does not embed Electron.
pub(crate) mod login;
#[cfg(feature = "injected-web-login")]
pub(crate) mod web_login;
mod protocol;
mod session;

use crate::relay::backend::{BackendClient, BackendEvent};
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
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}
