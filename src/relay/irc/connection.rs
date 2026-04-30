use super::IrcConfig;
use crate::relay::backend::BackendEvent;
use egui::Context as EguiContext;
use tokio::sync::mpsc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub enum IrcCommand {
    SendMessage { buffer_id: String, text: String },
    FetchNicks { buffer_id: String },
    FetchBufferList,
    Disconnect,
}

pub fn spawn(
    config: Arc<IrcConfig>,
    event_tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: EguiContext,
    mut cmd_rx: mpsc::UnboundedReceiver<IrcCommand>,
    connected: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        macro_rules! send {
            ($ev:expr) => {{
                let _ = event_tx.send($ev);
                ctx.request_repaint();
            }};
        }

        // Phase 4 stub: signal that the backend is not yet implemented
        send!(BackendEvent::Error(format!(
            "Soju/IRC backend not yet implemented ({}:{})",
            config.host, config.port
        )));
        connected.store(false, Ordering::Relaxed);

        // Drain commands so the channel doesn't block
        while let Ok(_cmd) = cmd_rx.try_recv() {}
    });
}
