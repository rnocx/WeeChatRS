//! Mentions: the lines that highlighted us, from every buffer, in an entry of its
//! own at the top of the buffer list, like WeeChat's highmon on every connection.
//! A click on a line goes to its buffer.

use chrono::{DateTime, Local, Utc};
use egui::{Color32, FontId, Label, RichText, text::LayoutJob};

use crate::relay::models::Buffer;
use crate::ui::ansi::ANSISection;
use crate::ui::app::WeeChatApp;
use crate::ui::icons;
use crate::ui::theme::AppTheme;

/// The id Mentions is selected by, which no connection's buffer has.
pub(crate) const MENTIONS_ID: &str = "@mentions";
const MAX_SHOWN: usize = 300;

#[derive(Default)]
pub(crate) struct MentionsState {
    query: String,
    /// Mentions up to then are hidden by Clear.
    cleared_at: Option<DateTime<Utc>>,
    /// Mentions since Mentions was last viewed, for its badge.
    pub unseen: usize,
    /// The buffer to go back to when Mentions is toggled off.
    return_to: Option<String>,
}

/// The mentions to show, as (buffer index, line index), oldest first like a buffer.
fn collect(buffers: &[Buffer], query: &str, cleared_at: Option<DateTime<Utc>>) -> Vec<(usize, usize)> {
    let mut found: Vec<(DateTime<Utc>, usize, usize)> = buffers.iter().enumerate()
        .filter(|(_, b)| !crate::ui::app::is_highmon_buffer(&b.full_name))
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
    found.into_iter().rev().map(|(_, bi, li)| (bi, li)).collect()
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

/// Appends a line's coloured text, `leading` points after what comes before it.
fn append_sections(job: &mut LayoutJob, sections: &[ANSISection], leading: f32, font: &FontId, theme: &AppTheme) {
    for (i, s) in sections.iter().enumerate() {
        job.append(&s.text, if i == 0 { leading } else { 0.0 }, s.style.to_format(font.clone(), theme));
    }
}

impl WeeChatApp {
    pub(crate) fn mentions_selected(&self) -> bool {
        self.selected_buffer_id.as_deref() == Some(MENTIONS_ID)
    }

    /// Shows Mentions, or goes back to the buffer before it.
    pub(crate) fn toggle_mentions(&mut self) {
        if self.mentions_selected() {
            let back = self.mentions.return_to.take()
                .filter(|id| self.buffer_by_id(id).is_some())
                .or_else(|| self.buffers.first().map(|b| b.id.clone()));
            if let Some(id) = back {
                self.select_buffer(id);
            }
        } else {
            self.mentions.return_to = self.selected_buffer_id.clone();
            self.select_buffer(MENTIONS_ID.to_string());
        }
    }

    /// Called when Mentions is selected.
    pub(crate) fn mentions_viewed(&mut self) {
        self.mentions.unseen = 0;
        self.mentions.query.clear();
    }

    /// A new mention for the badge, unless Mentions is on screen.
    pub(crate) fn count_mention(&mut self) {
        if !self.mentions_selected() {
            self.mentions.unseen += 1;
        }
    }

    /// Mentions' row at the top of the buffer list, with a badge for new mentions.
    /// True when it was clicked.
    pub(crate) fn mentions_row(&self, ui: &mut egui::Ui, accent: Color32, text: Color32, muted: Color32) -> bool {
        let selected = self.mentions_selected();
        let unseen = self.mentions.unseen;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::click());
        let (fill, fg) = if selected {
            (accent.linear_multiply(0.2), text)
        } else if unseen > 0 {
            (Color32::from_rgb(150, 50, 50).linear_multiply(0.3), Color32::from_rgb(255, 100, 100))
        } else {
            (Color32::TRANSPARENT, muted)
        };
        let fill = if resp.hovered() && !selected { accent.linear_multiply(0.1) } else { fill };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(6), fill);
        }
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(egui::vec2(8.0, 4.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if unseen > 0 {
            let badge = if unseen > 99 { "99+".to_string() } else { unseen.to_string() };
            egui::Frame::NONE
                .fill(Color32::from_rgb(200, 50, 50))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(4, 1))
                .show(&mut row, |ui| {
                    ui.label(RichText::new(badge).color(Color32::WHITE).strong().size(self.font_size * 0.72));
                });
        }
        row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let label = RichText::new(format!("{}  Mentions", icons::AT)).color(fg).strong().family(crate::ui::fonts::chat_family());
            ui.add(Label::new(label).truncate().selectable(false));
        });
        ui.add_space(6.0);
        resp.on_hover_text("Lines that mention you, from every buffer").clicked()
    }

    /// Mentions in the chat area: a bar with the filter and Clear, then the lines.
    pub(crate) fn draw_mentions(&mut self, ui: &mut egui::Ui, theme: &AppTheme, accent: Color32, muted: Color32, bar_fill: Color32) {
        let query = self.mentions.query.trim().to_lowercase();
        let found = collect(&self.buffers, &query, self.mentions.cleared_at);
        let mut clear = false;
        egui::Frame::NONE
            .fill(bar_fill)
            .inner_margin(egui::Margin::symmetric(16, 6))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(10)))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icons::AT).color(accent));
                    ui.label(RichText::new("Mentions from every buffer").color(muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clear = ui.add_enabled(!found.is_empty(), egui::Button::new("Clear"))
                            .on_hover_text("Hide these mentions; new ones still show")
                            .clicked();
                        ui.add(egui::TextEdit::singleline(&mut self.mentions.query).hint_text("Filter…").desired_width(200.0));
                    });
                });
            });
        ui.add_space(-1.0);
        ui.separator();
        if clear {
            self.mentions.cleared_at = Some(Utc::now());
        }

        if found.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                let text = if query.is_empty() {
                    "No mentions yet. Lines that mention you, in any buffer, show up here."
                } else {
                    "No matching mentions"
                };
                ui.label(RichText::new(text).color(muted));
            });
            return;
        }

        let font = FontId::new(self.font_size, crate::ui::fonts::chat_family());
        let mut chosen = None;
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                for &(bi, li) in &found {
                    let buffer = &self.buffers[bi];
                    let line = &buffer.messages[li];
                    let mut job = LayoutJob::default();
                    job.append(&when(line.timestamp), 0.0, egui::TextFormat::simple(font.clone(), muted));
                    job.append(&buffer.name, 12.0, egui::TextFormat::simple(font.clone(), accent));
                    append_sections(&mut job, &line.parsed_prefix, 12.0, &font, theme);
                    append_sections(&mut job, &line.parsed_message, 8.0, &font, theme);
                    let resp = ui.add(Label::new(job).wrap().sense(egui::Sense::click()));
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.on_hover_text(format!("Go to {}", buffer.full_name)).clicked() {
                        chosen = Some(buffer.id.clone());
                    }
                }
            });
        });
        if let Some(id) = chosen {
            self.select_buffer(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::models::Line;
    use std::sync::Arc;

    fn buffer(name: &str, lines: Vec<Line>) -> Buffer {
        Buffer { name: name.into(), full_name: format!("c/{name}"), messages: Arc::new(lines.into()), ..Default::default() }
    }

    fn line(at: i64, text: &str, highlight: bool) -> Line {
        let ts = DateTime::from_timestamp(at, 0).unwrap();
        Line::new(at.to_string(), ts, "bob".into(), text.into(), true, highlight)
    }

    #[test]
    fn collects_highlights_from_every_buffer_oldest_first() {
        let mut highmon = buffer("highmon", vec![line(40, "#rust bob hi me", true)]);
        highmon.full_name = "c/perl.highmon".into();
        let buffers = vec![
            buffer("#rust", vec![line(10, "hi me", true), line(20, "other", false), line(30, "me again", true)]),
            buffer("#go", vec![line(25, "hey me", true)]),
            highmon,
        ];
        assert_eq!(collect(&buffers, "", None), [(0, 0), (1, 0), (0, 2)]);
        assert_eq!(collect(&buffers, "again", None), [(0, 2)]);
        assert_eq!(collect(&buffers, "#go", None), [(1, 0)]);
        assert_eq!(collect(&buffers, "", DateTime::from_timestamp(25, 0)), [(0, 2)]);
    }
}
