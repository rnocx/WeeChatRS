//! Pasting several lines into the input line: instead of joining them into one,
//! ask whether to send them as messages (a few at a time, so the server doesn't
//! disconnect us for flooding), join them, or upload them and send the link.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{Color32, RichText};

use crate::ui::app::{BackendType, WeeChatApp};

/// Lines sent at once before the rest are spaced out.
const BURST: usize = 4;
const SPACING: Duration = Duration::from_secs(2);
const PREVIEW_LINES: usize = 12;

pub(crate) struct PendingPaste {
    buffer_id: String,
    buffer_name: String,
    text: String,
    lines: Vec<String>,
    injected: bool,
    uploaded_url: Option<String>,
    error: Option<String>,
}

/// Dropping a pending upload cancels its request and prevents posting a link.
pub(crate) struct InjectedPasteUpload {
    receiver: std::sync::mpsc::Receiver<Result<String, String>>,
    abort: tokio::task::AbortHandle,
}
impl Drop for InjectedPasteUpload {
    fn drop(&mut self) { self.abort.abort(); }
}

/// Pasted lines still to send.
pub(crate) struct PasteQueue {
    buffer_id: String,
    lines: VecDeque<String>,
    sent: usize,
    total: usize,
    next_at: Instant,
    /// WeeChat spaces lines out itself (its anti-flood), so they all go at once.
    paced: bool,
}

/// The lines of a paste worth sending: without blank ones or trailing spaces.
pub fn paste_lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).map(str::to_string).collect()
}

/// Native paste uploads keep whitespace and blank lines intact. Inline sending
/// uses the same preview; only the final newline is treated as a terminator.
fn injected_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    let mut lines: Vec<_> = normalized.split('\n').map(str::to_string).collect();
    if lines.last().is_some_and(String::is_empty) { lines.pop(); }
    lines
}

/// A pasted line sent as text even when it starts with a slash: `//` sends one `/`.
pub fn as_message(line: &str) -> String {
    if line.starts_with('/') { format!("/{line}") } else { line.to_string() }
}

/// Takes a paste of more than one line out of this frame's input, before the input
/// line joins it.
pub fn take_multiline_paste(ctx: &egui::Context) -> Option<String> {
    ctx.input_mut(|i| {
        let at = i.events.iter().position(|e| matches!(e, egui::Event::Paste(t) if paste_lines(t).len() > 1))?;
        match i.events.remove(at) {
            egui::Event::Paste(text) => Some(text),
            _ => None,
        }
    })
}

impl WeeChatApp {
    pub(crate) fn ask_about_paste(&mut self, buffer_id: String, text: String) {
        let buffer_name = self.buffer_by_id(&buffer_id).map(|b| b.name.clone()).unwrap_or_default();
        let injected = self.client_for_buffer(&buffer_id).is_some_and(|(client,_)| client.supports_paste_upload());
        let lines = if injected { injected_lines(&text) } else { paste_lines(&text) };
        self.pending_paste = Some(PendingPaste { buffer_id, buffer_name, text, lines, injected, uploaded_url: None, error: None });
    }

    pub(crate) fn show_paste_dialog(&mut self, ctx: &egui::Context, accent: Color32, muted: Color32) {
        let Some(paste) = self.pending_paste.as_ref() else { return };
        let injected = paste.injected;
        let native_uploading = self.injected_paste_upload.is_some();
        let uploading = self.file_share_progress.is_some();
        let chat_target = self.buffer_by_id(&paste.buffer_id).is_some_and(|b| matches!(b.kind.as_str(), "channel" | "private"));
        let native_connected = chat_target && self.client_for_buffer(&paste.buffer_id)
            .is_some_and(|(client,_)| client.supports_paste_upload() && client.is_connected());
        let (mut send, mut join, mut upload, mut cancel, mut copy) = (false, false, false, false, false);
        let modal = egui::Modal::new(egui::Id::unique("paste_guard")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new(format!("Paste {} lines into {}?", paste.lines.len(), paste.buffer_name)).strong());
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for line in paste.lines.iter().take(PREVIEW_LINES) { ui.label(RichText::new(line).monospace()); }
                    if paste.lines.len() > PREVIEW_LINES {
                        ui.label(RichText::new(format!("… and {} more", paste.lines.len() - PREVIEW_LINES)).color(muted));
                    }
                });
            });
            if injected {
                ui.label(RichText::new("Share the text through Injected and send only its link to this conversation.").small().color(muted));
                if !native_connected { ui.label("Connect to the original channel or private conversation before sharing."); }
            }
            if paste.lines.len() > BURST {
                ui.label(RichText::new(format!(
                    "Sent one line every {} seconds after the first {}, so the server doesn't disconnect you for flooding.",
                    SPACING.as_secs(), BURST,
                )).small().color(muted));
            }
            if let Some(error) = &paste.error { ui.colored_label(Color32::from_rgb(220,80,80), error); }
            if let Some(url) = &paste.uploaded_url { ui.hyperlink(url); copy = ui.button("Copy paste link").clicked(); }
            if native_uploading { ui.horizontal(|ui| { ui.spinner(); ui.label("Creating Injected paste…"); }); ctx.request_repaint_after(Duration::from_millis(100)); }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if injected {
                    let label = if paste.uploaded_url.is_some() { "Send paste link" } else { "Share as Injected paste" };
                    upload = ui.add_enabled(!native_uploading && native_connected, egui::Button::new(RichText::new(label).strong().color(Color32::WHITE)).fill(accent)).clicked();
                }
                let label = format!("Send {} lines", paste.lines.len());
                let button = egui::Button::new(if injected { RichText::new(label) } else { RichText::new(label).color(Color32::WHITE).strong() });
                let button = if injected { button } else { button.fill(accent) };
                send = ui.add_enabled(!native_uploading,button).clicked();
                join = ui.add_enabled(!native_uploading,egui::Button::new("Paste as one line")).clicked();
                if !injected {
                    upload = ui.add_enabled(!uploading, egui::Button::new("Upload as a file"))
                        .on_hover_text("Send a link to the text instead, with the service in Settings → File sharing").clicked();
                }
                cancel = ui.button("Cancel").clicked();
            });
        });
        if copy { if let Some(url) = &paste.uploaded_url { ctx.copy_text(url.clone()); } }
        if cancel || (!(send || join || upload) && modal.should_close()) {
            self.injected_paste_upload = None;
            self.pending_paste = None;
            return;
        }
        if upload && injected { self.share_injected_paste(ctx); return; }
        if !(send || join || upload) { return; }
        let paste = self.pending_paste.take().expect("shown above");
        if send {
            let paced = self.connections.iter()
                .find(|c| paste.buffer_id.starts_with(&format!("{}/", c.prefix)))
                .is_none_or(|c| c.backend_type != BackendType::WeeChat);
            self.paste_queue = Some(PasteQueue {
                buffer_id: paste.buffer_id,
                total: paste.lines.len(),
                lines: paste.lines.iter().map(|l| as_message(if l.is_empty() { " " } else { l })).collect(),
                sent: 0, next_at: Instant::now(), paced,
            });
        } else if join {
            if paste.injected && self.buffer_by_id(&paste.buffer_id).is_some() { self.select_buffer(paste.buffer_id.clone()); }
            if !self.input_text.is_empty() && !self.input_text.ends_with(' ') { self.input_text.push(' '); }
            let text = paste.lines.join(" ");
            self.input_text.push_str(&if paste.injected && self.input_text.is_empty() { as_message(&text) } else { text });
            self.focus_input = true;
            crate::ui::input::set_cursor(ctx, crate::ui::input::input_id(), self.input_text.chars().count());
        } else if upload { self.upload_paste(paste.buffer_id, &paste.text); }
    }

    fn share_injected_paste(&mut self, ctx: &egui::Context) {
        if self.injected_paste_upload.is_some() { return; }
        let Some(paste) = &self.pending_paste else { return; };
        if let Some(url) = &paste.uploaded_url {
            let sent = self.buffer_by_id(&paste.buffer_id).is_some() && self.client_for_buffer(&paste.buffer_id)
                .is_some_and(|(client,raw)| {
                    if !client.supports_paste_upload() || !client.is_connected() { return false; }
                    client.send_message(&raw,url); true
                });
            if sent { self.pending_paste = None; }
            else if let Some(paste) = &mut self.pending_paste { paste.error = Some("Paste created, but its original conversation is disconnected. Reconnect to send the link, or copy it.".into()); }
            return;
        }
        let Some((client,_)) = self.client_for_buffer(&paste.buffer_id).filter(|(c,_)| c.supports_paste_upload() && c.is_connected()) else { return; };
        let future = client.create_paste(paste.text.clone());
        let (tx,receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let task = tokio::spawn(async move { let _ = tx.send(future.await); ctx.request_repaint(); });
        self.injected_paste_upload = Some(InjectedPasteUpload {receiver,abort:task.abort_handle()});
        if let Some(paste) = &mut self.pending_paste { paste.error = None; }
    }

    fn poll_injected_paste(&mut self, ctx: &egui::Context) {
        let Some(upload) = &self.injected_paste_upload else { return; };
        let result = match upload.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => { ctx.request_repaint_after(Duration::from_millis(100)); return; }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("Paste upload stopped. Your text is still available to retry.".into()),
        };
        self.injected_paste_upload = None;
        if let Some(paste) = &mut self.pending_paste {
            match result {
                Ok(url) => { paste.uploaded_url = Some(url); self.share_injected_paste(ctx); }
                Err(error) => paste.error = Some(error),
            }
        }
    }

    /// Writes the paste to a file of its own and uploads it; the link is sent when done.
    fn upload_paste(&mut self, buffer_id: String, text: &str) {
        let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("weechatrs-paste-{stamp}"));
        let path = dir.join("paste.txt");
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, text)) {
            self.file_share_error = Some(format!("Couldn't save the paste: {e}"));
            return;
        }
        self.remove_paste_upload();
        self.paste_upload_dir = Some(dir);
        self.start_upload(buffer_id, Some(path));
    }

    /// Removes the file of an uploaded paste, once the upload is over.
    pub(crate) fn remove_paste_upload(&mut self) {
        if let Some(dir) = self.paste_upload_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Sends the pasted lines that are due; called every frame.
    pub(crate) fn send_paste_queue(&mut self, ctx: &egui::Context) {
        self.poll_injected_paste(ctx);
        let now = Instant::now();
        loop {
            let Some(queue) = self.paste_queue.as_mut() else { return };
            if queue.next_at > now {
                ctx.request_repaint_after(queue.next_at - now);
                return;
            }
            let Some(line) = queue.lines.pop_front() else {
                self.paste_queue = None;
                return;
            };
            queue.sent += 1;
            if queue.paced && queue.sent >= BURST {
                queue.next_at = now + SPACING;
            }
            let (buffer_id, done) = (queue.buffer_id.clone(), queue.lines.is_empty());
            match self.client_for_buffer(&buffer_id).filter(|(client, _)| client.is_connected()) {
                Some((client, raw_id)) => client.send_message(&raw_id, &line),
                None => {
                    self.paste_error = Some("The rest of the paste wasn't sent: disconnected".to_string());
                    self.paste_queue = None;
                    return;
                }
            }
            if done {
                self.paste_queue = None;
                return;
            }
        }
    }

    /// "Sending pasted lines: 3 of 12", with Stop, above the input line.
    pub(crate) fn paste_progress(&mut self, ui: &mut egui::Ui, muted: Color32) {
        if let Some(err) = self.paste_error.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(err).color(Color32::from_rgb(220, 80, 80)).small());
                if ui.small_button(crate::ui::icons::X).clicked() {
                    self.paste_error = None;
                }
            });
        }
        let Some(queue) = self.paste_queue.as_ref() else { return };
        let mut stop = false;
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(format!("Sending pasted lines: {} of {}", queue.sent, queue.total)).small().color(muted));
            stop = ui.small_button("Stop").clicked();
        });
        if stop {
            self.paste_queue = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_native_upload_stops_its_completion_task() {
        let (tx,receiver) = std::sync::mpsc::channel::<Result<String,String>>();
        let (started,ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started.send(());
            std::future::pending::<()>().await;
            let _ = tx.send(Ok("https://injected-irc.com/paste?id=never-posted".into()));
        });
        let upload = InjectedPasteUpload { receiver, abort: task.abort_handle() };
        ready.await.unwrap();
        drop(upload);
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[test]
    fn native_preview_keeps_code_indentation_blank_lines_and_literal_commands() {
        assert_eq!(injected_lines("  code  \r\n\r\n/quit\n"), ["  code  ", "", "/quit"]);
        assert_eq!(as_message(&injected_lines("/quit\n")[0]), "//quit");
    }

    #[test]
    fn pastes_lose_blank_lines_and_keep_slashes_as_text() {
        assert_eq!(paste_lines("fn main() {\r\n\n    run();  \n}\n"), ["fn main() {", "    run();", "}"]);
        assert_eq!(paste_lines("one line\n").len(), 1);
        assert_eq!(as_message("/quit now"), "//quit now");
        assert_eq!(as_message("hello"), "hello");
    }
}
