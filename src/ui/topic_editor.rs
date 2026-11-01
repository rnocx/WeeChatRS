//! A dialog to edit a channel's topic in place, opened from the topic bar or the
//! buffer list, instead of retyping the whole topic after /topic.

use egui::{Key, Modifiers, RichText};

use crate::ui::ansi::ANSIParser;
use crate::ui::app::{BackendType, WeeChatApp};

pub(crate) struct TopicEditor {
    buffer_id: String,
    channel: String,
    text: String,
    original: String,
    /// The topic had colours or formatting, which plain text leaves out.
    had_formatting: bool,
    focused: bool,
}

/// The topic as the server would take it: one line, without trailing spaces.
fn one_line(text: &str) -> String {
    text.split(['\r', '\n']).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

impl WeeChatApp {
    pub(crate) fn open_topic_editor(&mut self, buffer_id: &str) {
        let Some(buffer) = self.buffer_by_id(buffer_id) else { return };
        let plain: String = ANSIParser::parse(&buffer.topic).iter().map(|s| s.text.as_str()).collect();
        self.topic_editor = Some(TopicEditor {
            buffer_id: buffer_id.to_string(),
            channel: buffer.name.clone(),
            text: plain.clone(),
            original: plain,
            had_formatting: buffer.topic.contains('\x1B'),
            focused: false,
        });
    }

    pub(crate) fn show_topic_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = self.topic_editor.as_mut() else { return };
        // Enter saves (the field wraps but a topic is one line); Esc cancels.
        let (enter, escape) = ctx.input_mut(|i| (
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        ));
        let mut save = enter;
        let mut cancel = escape;
        let modal = egui::Modal::new(egui::Id::unique("topic_editor")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new(format!("Topic of {}", editor.channel)).strong());
            ui.add_space(6.0);
            let edit = ui.add(
                egui::TextEdit::multiline(&mut editor.text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("No topic"),
            );
            if !editor.focused {
                edit.request_focus();
                editor.focused = true;
            }
            let weak = ui.visuals().weak_text_color();
            ui.label(RichText::new(format!("{} characters", one_line(&editor.text).chars().count())).small().color(weak));
            if editor.had_formatting {
                ui.label(RichText::new("The topic's colours and formatting aren't kept when you save.").small().color(weak));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let changed = one_line(&editor.text) != one_line(&editor.original);
                if ui.add_enabled(changed, egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if !editor.text.trim().is_empty() && ui.button("Clear").on_hover_text("Remove the topic").clicked() {
                    editor.text.clear();
                }
            });
        });
        // A click outside the dialog cancels too.
        if cancel || modal.should_close() {
            self.topic_editor = None;
        } else if save {
            let editor = self.topic_editor.take().expect("shown above");
            let text = one_line(&editor.text);
            if text != one_line(&editor.original) {
                self.set_topic(&editor.buffer_id, &editor.channel, &text);
            }
        }
    }

    fn set_topic(&mut self, buffer_id: &str, channel: &str, text: &str) {
        let backend = self.connections.iter()
            .find(|c| buffer_id.starts_with(&format!("{}/", c.prefix)))
            .map(|c| c.backend_type.clone());
        // `/topic` without text shows the topic, so removing it takes each backend's own way.
        let command = match (text.is_empty(), backend) {
            (false, _) => format!("/topic {text}"),
            (true, Some(BackendType::WeeChat)) => "/topic -delete".to_string(),
            (true, Some(BackendType::Injected)) => format!("/raw TOPIC {channel} :"),
            (true, _) => format!("/quote TOPIC {channel} :"),
        };
        self.send_command_to_buffer(buffer_id, &command);
    }
}
