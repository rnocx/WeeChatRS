//! The mentions view: the lines that highlighted us, from every buffer, newest
//! first, like WeeChat's highmon. A click goes to the buffer.

use chrono::{DateTime, Local, Utc};
use egui::{Color32, RichText};

use crate::relay::models::Buffer;
use crate::ui::app::WeeChatApp;
use crate::ui::icons;

const MAX_SHOWN: usize = 300;

#[derive(Default)]
pub(crate) struct MentionsState {
    pub open: bool,
    query: String,
    /// Mentions up to then are hidden by Clear.
    cleared_at: Option<DateTime<Utc>>,
    /// Mentions since the view was last opened, for the toolbar.
    pub unseen: usize,
    hovered: Option<usize>,
}

impl MentionsState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        if self.open {
            self.unseen = 0;
        }
    }
}

/// WeeChat's highmon buffer repeats the mentions shown here.
fn is_highmon(buffer: &Buffer) -> bool {
    buffer.name == "highmon"
}

/// The mentions to show, as (buffer index, line index), newest first.
fn collect(buffers: &[Buffer], query: &str, cleared_at: Option<DateTime<Utc>>) -> Vec<(usize, usize)> {
    let mut found: Vec<(DateTime<Utc>, usize, usize)> = buffers.iter().enumerate()
        .filter(|(_, b)| !is_highmon(b))
        .flat_map(|(bi, b)| {
            let name_matches = b.name.to_lowercase().contains(query);
            b.messages.iter().enumerate()
                .filter(move |(_, l)| l.highlight && l.displayed && cleared_at.is_none_or(|c| l.timestamp > c))
                .filter(move |(_, l)| name_matches || l.plain_message_lower.contains(query) || l.plain_prefix_lower.contains(query))
                .map(move |(li, l)| (l.timestamp, bi, li))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.truncate(MAX_SHOWN);
    found.into_iter().map(|(_, bi, li)| (bi, li)).collect()
}

/// The time of a mention: the hour today, with the day before that.
fn when(timestamp: DateTime<Utc>) -> String {
    let local = timestamp.with_timezone(&Local);
    if local.date_naive() == Local::now().date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%a %d %b %H:%M").to_string()
    }
}

impl WeeChatApp {
    pub(crate) fn show_mentions(&mut self, ctx: &egui::Context, accent: Color32, muted: Color32) {
        if !self.mentions.open {
            return;
        }
        let query = self.mentions.query.trim().to_lowercase();
        let found = collect(&self.buffers, &query, self.mentions.cleared_at);
        let mut chosen: Option<String> = None;
        let mut clear = false;
        let mut hovered = None;
        let modal = egui::Modal::new(egui::Id::unique("mentions")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::AT).color(muted));
                ui.label(RichText::new("Mentions").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!found.is_empty(), egui::Button::new("Clear"))
                        .on_hover_text("Hide these mentions; new ones still show")
                        .clicked()
                    {
                        clear = true;
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mentions.query)
                            .hint_text("Filter…")
                            .desired_width(200.0),
                    );
                });
            });
            ui.separator();
            if found.is_empty() {
                ui.add_space(8.0);
                let text = if query.is_empty() { "No mentions yet" } else { "No matching mentions" };
                ui.label(RichText::new(text).color(muted));
                ui.add_space(8.0);
                return;
            }
            egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                for (row, &(bi, li)) in found.iter().enumerate() {
                    let buffer = &self.buffers[bi];
                    let line = &buffer.messages[li];
                    let fill = if self.mentions.hovered == Some(row) { accent.linear_multiply(0.15) } else { Color32::TRANSPARENT };
                    let frame = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).corner_radius(6).fill(fill);
                    let resp = frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&buffer.name).color(accent).strong());
                            ui.label(RichText::new(when(line.timestamp)).color(muted).small());
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&line.plain_prefix).strong());
                            ui.label(&line.plain_message);
                        });
                    }).response.interact(egui::Sense::click());
                    if resp.hovered() {
                        hovered = Some(row);
                    }
                    if resp.on_hover_text(&buffer.full_name).clicked() {
                        chosen = Some(buffer.id.clone());
                    }
                }
            });
        });
        self.mentions.hovered = hovered;
        if clear {
            self.mentions.cleared_at = Some(Utc::now());
        }
        if let Some(id) = chosen {
            self.mentions.open = false;
            self.select_buffer(id);
        } else if modal.should_close() {
            self.mentions.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::models::Line;
    use std::sync::Arc;

    fn buffer(name: &str, lines: Vec<Line>) -> Buffer {
        Buffer { name: name.into(), messages: Arc::new(lines.into()), ..Default::default() }
    }

    fn line(at: i64, text: &str, highlight: bool) -> Line {
        let ts = DateTime::from_timestamp(at, 0).unwrap();
        Line::new(at.to_string(), ts, "bob".into(), text.into(), true, highlight)
    }

    #[test]
    fn collects_highlights_from_every_buffer_newest_first() {
        let buffers = vec![
            buffer("#rust", vec![line(10, "hi me", true), line(20, "other", false), line(30, "me again", true)]),
            buffer("#go", vec![line(25, "hey me", true)]),
            buffer("highmon", vec![line(40, "#rust bob hi me", true)]),
        ];
        assert_eq!(collect(&buffers, "", None), [(0, 2), (1, 0), (0, 0)]);
        assert_eq!(collect(&buffers, "again", None), [(0, 2)]);
        assert_eq!(collect(&buffers, "#go", None), [(1, 0)]);
        assert_eq!(collect(&buffers, "", DateTime::from_timestamp(25, 0)), [(0, 2)]);
    }
}
