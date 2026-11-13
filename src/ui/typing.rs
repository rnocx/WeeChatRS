//! Typing indicators: who is typing in the open buffer, and telling the others
//! when we are. IRC takes them as IRCv3 `+typing` tags, Injected has its own
//! events, and the WeeChat relay carries neither.

use std::time::{Duration, Instant};

use crate::relay::backend::TypingState;
use crate::ui::app::WeeChatApp;

/// An active indicator lapses after this long without a refresh, so one whose
/// "done" never arrived doesn't stay forever.
const ACTIVE_FOR: Duration = Duration::from_secs(6);
/// A paused one (text left in the input) lasts longer.
const PAUSED_FOR: Duration = Duration::from_secs(30);
/// How often we repeat "active" while typing goes on.
const RESEND_AFTER: Duration = Duration::from_secs(3);
/// No keystroke for this long and we say "paused".
const PAUSE_AFTER: Duration = Duration::from_secs(5);

/// What we last told the others about our typing.
pub(crate) struct TypingSent {
    buffer_id: String,
    state: TypingState,
    /// When "active" last went out.
    last_active: Instant,
}

impl WeeChatApp {
    /// `nick` typing, paused or done in `buffer_id`.
    pub(crate) fn typing_seen(&mut self, buffer_id: String, nick: String, state: TypingState) {
        let list = self.typing.entry(buffer_id).or_default();
        list.retain(|(n, _)| !n.eq_ignore_ascii_case(&nick));
        let lasts = match state {
            TypingState::Active => ACTIVE_FOR,
            TypingState::Paused => PAUSED_FOR,
            TypingState::Done | TypingState::Sent => return,
        };
        list.push((nick, Instant::now() + lasts));
    }

    /// The line to show above the input of `buffer_id`, if anyone is typing there.
    /// Asks for a repaint when the next indicator lapses.
    pub(crate) fn typing_line(&mut self, ctx: &egui::Context, buffer_id: &str) -> Option<String> {
        if !self.show_typing {
            return None;
        }
        let now = Instant::now();
        let list = self.typing.get_mut(buffer_id)?;
        list.retain(|(_, until)| *until > now);
        if list.is_empty() {
            return None;
        }
        if let Some(next) = list.iter().map(|(_, until)| *until).min() {
            ctx.request_repaint_after(next.saturating_duration_since(now) + Duration::from_millis(50));
        }
        Some(typing_text(&list.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()))
    }

    /// Called every frame: tells the others when we start, pause and stop typing in
    /// the open buffer.
    pub(crate) fn update_typing(&mut self, ctx: &egui::Context) {
        let open = self.selected_buffer_id.clone().filter(|id| {
            self.buffer_by_id(id).is_some_and(|b| matches!(b.kind.as_str(), "channel" | "private"))
        });
        let wants = self.send_typing.then_some(()).and(open.clone());
        // Switched buffer or switched it off: close out the one we were typing in.
        if let Some(sent) = &self.typing_sent {
            if wants.as_deref() != Some(sent.buffer_id.as_str()) {
                let id = sent.buffer_id.clone();
                self.typing_sent = None;
                self.send_typing_state(&id, TypingState::Done);
            }
        }
        let Some(id) = wants else {
            self.typing_text = self.input_text.clone();
            return;
        };
        let now = Instant::now();
        // A command isn't something to announce.
        let typing = !self.input_text.is_empty() && !self.input_text.starts_with('/');
        if self.input_text != self.typing_text {
            self.typing_text = self.input_text.clone();
            self.typing_edit = now;
            if typing {
                let due = self.typing_sent.as_ref().is_none_or(|s| {
                    s.state != TypingState::Active || now.duration_since(s.last_active) >= RESEND_AFTER
                });
                if due {
                    self.typing_sent = Some(TypingSent { buffer_id: id.clone(), state: TypingState::Active, last_active: now });
                    self.send_typing_state(&id, TypingState::Active);
                }
            } else if self.typing_sent.take().is_some() {
                let state = if self.typing_submitted { TypingState::Sent } else { TypingState::Done };
                self.send_typing_state(&id, state);
            }
            self.typing_submitted = false;
        } else if let Some(sent) = self.typing_sent.as_mut() {
            if sent.state == TypingState::Active {
                let idle = now.duration_since(self.typing_edit);
                if idle >= PAUSE_AFTER {
                    sent.state = TypingState::Paused;
                    self.send_typing_state(&id, TypingState::Paused);
                } else {
                    ctx.request_repaint_after(PAUSE_AFTER - idle + Duration::from_millis(50));
                }
            }
        }
    }

    fn send_typing_state(&self, buffer_id: &str, state: TypingState) {
        if let Some((client, raw_id)) = self.client_for_buffer(buffer_id) {
            client.send_typing(&raw_id, state);
        }
    }
}

/// "alice is typing…", "alice and bob are typing…", "Several people are typing…".
fn typing_text(nicks: &[&str]) -> String {
    match nicks {
        [] => String::new(),
        [one] => format!("{one} is typing…"),
        [a, b] => format!("{a} and {b} are typing…"),
        [a, b, c] => format!("{a}, {b} and {c} are typing…"),
        _ => "Several people are typing…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::typing_text;

    #[test]
    fn the_line_names_up_to_three() {
        assert_eq!(typing_text(&["alice"]), "alice is typing…");
        assert_eq!(typing_text(&["alice", "bob"]), "alice and bob are typing…");
        assert_eq!(typing_text(&["a", "b", "c"]), "a, b and c are typing…");
        assert_eq!(typing_text(&["a", "b", "c", "d"]), "Several people are typing…");
    }
}
