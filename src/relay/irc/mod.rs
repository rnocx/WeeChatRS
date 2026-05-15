pub mod connection;
pub mod parser;

use crate::relay::backend::{BackendClient, BackendEvent};
use egui::Context as EguiContext;
use tokio::sync::mpsc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use connection::IrcCommand;

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
}

pub struct IrcClient {
    config: Arc<IrcConfig>,
    event_tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: EguiContext,
    cmd_tx: mpsc::UnboundedSender<IrcCommand>,
    connected: Arc<AtomicBool>,
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
        }
    }
}

impl BackendClient for IrcClient {
    fn connect(&mut self) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        self.cmd_tx = cmd_tx;
        connection::spawn(
            Arc::clone(&self.config),
            self.event_tx.clone(),
            self.ctx.clone(),
            cmd_rx,
            Arc::clone(&self.connected),
        );
    }

    fn disconnect(&mut self) {
        let _ = self.cmd_tx.send(IrcCommand::Disconnect);
    }

    fn send_message(&self, buffer_id: &str, text: &str) {
        let _ = self.cmd_tx.send(IrcCommand::SendMessage {
            buffer_id: buffer_id.to_string(),
            text: text.to_string(),
        });
    }

    fn fetch_lines(&self, _buffer_id: &str, _count: usize) {
        // chathistory is requested automatically on JOIN
    }

    fn fetch_nicks(&self, buffer_id: &str) {
        let _ = self.cmd_tx.send(IrcCommand::FetchNicks {
            buffer_id: buffer_id.to_string(),
        });
    }

    fn mark_read(&self, buffer_id: &str) {
        let _ = self.cmd_tx.send(IrcCommand::MarkRead {
            buffer_id: buffer_id.to_string(),
        });
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    fn fetch_buffer_list(&self) {
        // IRC buffers are discovered passively via JOIN
    }

    fn fetch_lines_before(&self, buffer_id: &str, before_ts: &str) {
        let _ = self.cmd_tx.send(IrcCommand::FetchBefore {
            buffer_id: buffer_id.to_string(),
            before_ts: before_ts.to_string(),
        });
    }
}
