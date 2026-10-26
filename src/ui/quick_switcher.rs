//! `Cmd/Ctrl+P` quick switcher: fuzzy-find across all buffers.

use crate::relay::models::BufferActivity;
use crate::ui::app::WeeChatApp;
use crate::ui::icons;
use egui::{Color32, Key, Modifiers, RichText};

const MAX_RESULTS: usize = 12;

#[derive(Default)]
pub(crate) struct QuickSwitcherState {
    pub open: bool,
    query: String,
    selected: usize,
}

impl QuickSwitcherState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.selected = 0;
    }
}

/// Subsequence match of `query` in `candidate` (both expected lowercase).
/// Higher is better; `None` when not every query char is found in order.
/// Rewards matches at the start, after a separator (`#`, `.`, `-`, `_`, space)
/// and consecutive runs, and lightly penalises long candidates.
pub(crate) fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.chars().collect();
    let mut score = 0;
    let mut ci = 0;
    let mut prev_match: Option<usize> = None;
    for qc in query.chars() {
        let pos = (ci..cand.len()).find(|&i| cand[i] == qc)?;
        score += 1;
        if pos == 0 {
            score += 8;
        } else if matches!(cand[pos - 1], '#' | '.' | '-' | '_' | ' ' | '&') {
            score += 5;
        }
        if prev_match == Some(pos.wrapping_sub(1)) {
            score += 4;
        }
        prev_match = Some(pos);
        ci = pos + 1;
    }
    Some(score * 10 - cand.len() as i32)
}

impl WeeChatApp {
    pub(crate) fn show_quick_switcher(&mut self, ctx: &egui::Context, accent: Color32, muted: Color32) {
        if !self.quick_switcher.open {
            return;
        }

        let query = self.quick_switcher.query.to_lowercase();
        let mut results: Vec<(i32, usize)> = self.buffers.iter().enumerate()
            .filter(|(_, b)| !b.hidden || self.show_hidden_buffers)
            .filter_map(|(idx, b)| {
                let by_name = fuzzy_score(&query, &b.name.to_lowercase());
                // Full name ("irc.libera.#rust") also matches, at a discount.
                let by_full = fuzzy_score(&query, &b.full_name.to_lowercase()).map(|s| s - 15);
                let mut score = by_name.max(by_full)?;
                score += match b.activity {
                    BufferActivity::Highlight => 30,
                    BufferActivity::Message => 10,
                    _ => 0,
                };
                Some((score, idx))
            })
            .collect();
        if !query.is_empty() {
            // Stable sort keeps buffer-list order among equal scores.
            results.sort_by(|a, b| b.0.cmp(&a.0));
        }
        results.truncate(MAX_RESULTS);

        let state = &mut self.quick_switcher;
        if !results.is_empty() {
            state.selected = state.selected.min(results.len() - 1);
        }
        let (up, down, enter) = ctx.input_mut(|i| (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
        ));
        if up { state.selected = state.selected.saturating_sub(1); }
        if down && state.selected + 1 < results.len() { state.selected += 1; }

        let mut chosen: Option<usize> = if enter { results.get(state.selected).map(|r| r.1) } else { None };

        let modal = egui::Modal::new(egui::Id::unique("quick_switcher")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(muted));
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .hint_text("Jump to buffer…")
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if edit.changed() { state.selected = 0; }
                edit.request_focus();
            });
            ui.separator();

            if results.is_empty() {
                ui.label(RichText::new("No matching buffers").color(muted));
            }
            for (row, &(_, idx)) in results.iter().enumerate() {
                let b = &self.buffers[idx];
                let is_sel = row == state.selected;
                let frame = egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .corner_radius(6)
                    .fill(if is_sel { accent.linear_multiply(0.25) } else { Color32::TRANSPARENT });
                let resp = frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&b.name).strong());
                        ui.label(RichText::new(&b.full_name).color(muted).small());
                        if b.unread_count > 0 {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let color = if b.activity == BufferActivity::Highlight {
                                    Color32::from_rgb(200, 50, 50)
                                } else {
                                    accent
                                };
                                ui.label(RichText::new(b.unread_count.to_string()).color(color).small().strong());
                            });
                        }
                    });
                }).response.interact(egui::Sense::click());
                if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                    state.selected = row;
                }
                if resp.clicked() { chosen = Some(idx); }
            }
        });

        if let Some(idx) = chosen {
            let id = self.buffers[idx].id.clone();
            self.quick_switcher.open = false;
            self.select_buffer(id);
        } else if modal.should_close() {
            self.quick_switcher.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn rejects_out_of_order() {
        assert_eq!(fuzzy_score("tsur", "#rust"), None);
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(fuzzy_score("", "#anything"), Some(0));
    }

    #[test]
    fn prefers_word_start_and_contiguous() {
        let rust = fuzzy_score("rust", "#rust").unwrap();
        let scattered = fuzzy_score("rust", "#random-useless-stuff-t").unwrap();
        assert!(rust > scattered);
    }

    #[test]
    fn prefers_shorter_candidate_on_tie() {
        let short = fuzzy_score("dev", "#dev").unwrap();
        let long = fuzzy_score("dev", "#devops-and-more").unwrap();
        assert!(short > long);
    }
}
