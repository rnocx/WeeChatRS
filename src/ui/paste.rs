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
        let lines = paste_lines(&text);
        self.pending_paste = Some(PendingPaste { buffer_id, buffer_name, text, lines });
    }

    pub(crate) fn show_paste_dialog(&mut self, ctx: &egui::Context, accent: Color32, muted: Color32) {
        let Some(paste) = self.pending_paste.as_ref() else { return };
        let uploading = self.file_share_progress.is_some();
        let (mut send, mut join, mut upload, mut cancel) = (false, false, false, false);
        let modal = egui::Modal::new(egui::Id::unique("paste_guard")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new(format!("Paste {} lines into {}?", paste.lines.len(), paste.buffer_name)).strong());
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for line in paste.lines.iter().take(PREVIEW_LINES) {
                        ui.label(RichText::new(line).monospace());
                    }
                    if paste.lines.len() > PREVIEW_LINES {
                        ui.label(RichText::new(format!("… and {} more", paste.lines.len() - PREVIEW_LINES)).color(muted));
                    }
                });
            });
            if paste.lines.len() > BURST {
                ui.label(RichText::new(format!(
                    "Sent one line every {} seconds after the first {}, so the server doesn't disconnect you for flooding.",
                    SPACING.as_secs(), BURST,
                )).small().color(muted));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = RichText::new(format!("Send {} lines", paste.lines.len())).color(Color32::WHITE).strong();
                send = ui.add(egui::Button::new(label).fill(accent)).clicked();
                join = ui.button("Paste as one line").clicked();
                upload = ui.add_enabled(!uploading, egui::Button::new("Upload as a file"))
                    .on_hover_text("Send a link to the text instead, with the service in Settings → File sharing")
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if !(send || join || upload) {
            if cancel || modal.should_close() {
                self.pending_paste = None;
            }
            return;
        }
        let paste = self.pending_paste.take().expect("shown above");
        if send {
            let paced = self.connections.iter()
                .find(|c| paste.buffer_id.starts_with(&format!("{}/", c.prefix)))
                .is_none_or(|c| c.backend_type != BackendType::WeeChat);
            self.paste_queue = Some(PasteQueue {
                buffer_id: paste.buffer_id,
                total: paste.lines.len(),
                lines: paste.lines.iter().map(|l| as_message(l)).collect(),
                sent: 0,
                next_at: Instant::now(),
                paced,
            });
        } else if join {
            if !self.input_text.is_empty() && !self.input_text.ends_with(' ') {
                self.input_text.push(' ');
            }
            self.input_text.push_str(&paste.lines.join(" "));
            self.focus_input = true;
            crate::ui::input::set_cursor(ctx, crate::ui::input::input_id(), self.input_text.chars().count());
        } else if upload {
            self.upload_paste(paste.buffer_id, &paste.text);
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

    #[test]
    fn pastes_lose_blank_lines_and_keep_slashes_as_text() {
        assert_eq!(paste_lines("fn main() {\r\n\n    run();  \n}\n"), ["fn main() {", "    run();", "}"]);
        assert_eq!(paste_lines("one line\n").len(), 1);
        assert_eq!(as_message("/quit now"), "//quit now");
        assert_eq!(as_message("hello"), "hello");
    }
}
