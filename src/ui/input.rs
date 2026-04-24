use egui::text_edit::TextEditState;
use egui::text::{CCursorRange, CCursor};
use crate::ui::app::WeeChatApp;

pub(crate) struct CompletionState {
    #[allow(dead_code)]
    pub(crate) original_word: String,
    pub(crate) matches: Vec<String>,
    pub(crate) index: usize,
    pub(crate) word_start_idx: usize,
}

impl WeeChatApp {
    pub(crate) fn perform_completion(&mut self, ctx: &egui::Context, id: egui::Id) {
        let nicks = match self.selected_buffer_id.as_ref()
            .and_then(|id| self.buffers.iter().find(|b| &b.id == id))
            .map(|b| b.nicks.clone())
        {
            Some(n) => n,
            None => return,
        };

        let mut new_cursor_pos = 0;

        if let Some(state) = &mut self.completion {
            if state.matches.is_empty() { return; }
            state.index = (state.index + 1) % state.matches.len();
            let matched_nick = &state.matches[state.index];

            let mut new_text = self.input_text[..state.word_start_idx].to_string();
            new_text.push_str(matched_nick);
            if state.word_start_idx == 0 {
                new_text.push_str(": ");
            } else {
                new_text.push(' ');
            }
            new_cursor_pos = new_text.len();
            self.input_text = new_text;
        } else {
            let last_word_start = self.input_text.rfind(' ').map(|i| i + 1).unwrap_or(0);
            let word_to_complete = self.input_text[last_word_start..].to_string();
            if word_to_complete.is_empty() { return; }

            let matches: Vec<String> = nicks.iter()
                .filter(|n| n.name.to_lowercase().starts_with(&word_to_complete.to_lowercase()))
                .map(|n| n.name.clone())
                .collect();

            if !matches.is_empty() {
                let matched_nick = &matches[0];
                let mut new_text = self.input_text[..last_word_start].to_string();
                new_text.push_str(matched_nick);
                if last_word_start == 0 {
                    new_text.push_str(": ");
                } else {
                    new_text.push(' ');
                }

                new_cursor_pos = new_text.len();
                self.input_text = new_text;
                self.completion = Some(CompletionState {
                    original_word: word_to_complete,
                    matches,
                    index: 0,
                    word_start_idx: last_word_start,
                });
            }
        }

        if new_cursor_pos > 0 {
            if let Some(mut state) = TextEditState::load(ctx, id) {
                state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(new_cursor_pos))));
                state.store(ctx, id);
            }
        }
    }

    pub(crate) fn cycle_history(&mut self, delta: i32, ctx: &egui::Context, id: egui::Id) {
        if self.command_history.is_empty() { return; }

        let new_index = match self.history_index {
            Some(idx) => {
                if delta < 0 && idx == 0 {
                    Some(0)
                } else {
                    let next = idx as i32 + delta;
                    if next >= self.command_history.len() as i32 {
                        None
                    } else {
                        Some(next.max(0) as usize)
                    }
                }
            }
            None => {
                if delta < 0 { Some(self.command_history.len() - 1) } else { None }
            }
        };

        self.history_index = new_index;
        if let Some(idx) = self.history_index {
            self.input_text = self.command_history[idx].clone();
        } else {
            self.input_text.clear();
        }

        let pos = self.input_text.len();
        if let Some(mut state) = TextEditState::load(ctx, id) {
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(pos))));
            state.store(ctx, id);
        }
    }

    pub(crate) fn cycle_buffer(&mut self, delta: i32) {
        if self.buffers.is_empty() { return; }
        let current_id = match self.selected_buffer_id.clone() {
            Some(id) => id,
            None => {
                if let Some(first) = self.buffers.first() {
                    let id = first.id.clone();
                    self.select_buffer(id);
                }
                return;
            }
        };

        if let Some(idx) = self.buffers.iter().position(|b| b.id == current_id) {
            let new_idx = (idx as i32 + delta).rem_euclid(self.buffers.len() as i32) as usize;
            let new_id = self.buffers[new_idx].id.clone();
            self.select_buffer(new_id);
        }
    }

    pub(crate) fn send_current_message(&mut self) {
        if self.input_text.is_empty() { return; }
        let msg = self.input_text.clone();
        let is_command = msg.starts_with('/');

        if let Some(client) = &self.client {
            if let Some(buffer) = self.selected_buffer_id.as_ref()
                .and_then(|id| self.buffers.iter().find(|b| &b.id == id))
            {
                if let Ok(numeric_id) = buffer.id.parse::<i64>() {
                    client.send_api("POST /api/input", None, Some(serde_json::json!({
                        "buffer_id": numeric_id,
                        "command": msg
                    })));
                }
            }

            if is_command {
                if msg.starts_with("/query ") {
                    self.pending_buffer_switch = msg[7..].split_whitespace().next().map(|s| s.to_string());
                } else if msg.starts_with("/join ") {
                    self.pending_buffer_switch = msg[6..].split_whitespace().next().map(|s| s.to_string());
                }
                client.send_api("GET /api/buffers", Some("_list_buffers"), None);
            }
        }

        if self.command_history.last().map(|s| s.as_str()) != Some(&msg) {
            self.command_history.push(msg);
            if self.command_history.len() > 100 {
                self.command_history.remove(0);
            }
        }

        self.input_text.clear();
        self.completion = None;
        self.history_index = None;
    }

    pub(crate) fn send_command(&mut self, command: &str) {
        if command.starts_with("/query ") {
            self.pending_buffer_switch = Some(command[7..].trim().to_string());
        } else if command.starts_with("/join ") {
            self.pending_buffer_switch = Some(command[6..].trim().to_string());
        }

        if let Some(client) = &self.client {
            if let Some(buffer) = self.selected_buffer_id.as_ref()
                .and_then(|id| self.buffers.iter().find(|b| &b.id == id))
            {
                if let Ok(numeric_id) = buffer.id.parse::<i64>() {
                    client.send_api("POST /api/input", None, Some(serde_json::json!({
                        "buffer_id": numeric_id,
                        "command": command
                    })));
                }
            }
            client.send_api("GET /api/buffers", Some("_list_buffers"), None);
        }
    }
}
