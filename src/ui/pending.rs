//! Messages shown the moment they are sent, waiting for the server's copy.
//!
//! The local line goes in with a temporary id. When the backend reports the same
//! message, the app swaps the authoritative line in for it. Several messages with
//! the same text match in the order they were sent.

use crate::relay::models::Line;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Longer messages are not shown early. The server or WeeChat splits or cuts them,
/// so their echo would not match and the line would show twice.
pub(crate) const MAX_EARLY_ECHO_BYTES: usize = 400;

/// How long a pending message waits for its echo before the app stops expecting it.
/// The line stays on screen, only the bookkeeping goes.
pub(crate) const PENDING_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq)]
struct Pending {
    line_id: String,
    text: String,
    sent: Instant,
}

#[derive(Default)]
pub(crate) struct PendingEchoes {
    by_buffer: HashMap<String, VecDeque<Pending>>,
    counter: u64,
}

impl PendingEchoes {
    /// Remember a message sent to `buffer_id`, as the plain text its local line shows,
    /// and hand back the temporary id that line should carry.
    pub(crate) fn add(&mut self, buffer_id: &str, plain: &str, now: Instant) -> String {
        self.counter += 1;
        let line_id = format!("pending-{}", self.counter);
        self.by_buffer.entry(buffer_id.to_string()).or_default().push_back(Pending {
            line_id: line_id.clone(),
            text: plain.trim().to_string(),
            sent: now,
        });
        line_id
    }

    /// The oldest pending message in `buffer_id` that `plain` (a line's text without
    /// colours) is the echo of. It is removed, and its temporary id returned.
    pub(crate) fn take_match(&mut self, buffer_id: &str, plain: &str) -> Option<String> {
        let queue = self.by_buffer.get_mut(buffer_id)?;
        let plain = plain.trim();
        let i = queue.iter().position(|p| p.text == plain)?;
        let found = queue.remove(i).map(|p| p.line_id);
        if queue.is_empty() {
            self.by_buffer.remove(buffer_id);
        }
        found
    }

    /// `line` came from the backend as our own message in `buffer_id`. If it is the
    /// echo of a pending one, it takes the place of the local line in `messages` and
    /// this returns true.
    pub(crate) fn reconcile(&mut self, buffer_id: &str, messages: &mut VecDeque<Line>, line: &Line) -> bool {
        let Some(temp) = self.take_match(buffer_id, &line.plain_message) else { return false };
        match messages.iter().rposition(|m| m.id == temp) {
            Some(pos) => { messages[pos] = line.clone(); true }
            None => false,
        }
    }

    /// Drop what has waited longer than `ttl`.
    pub(crate) fn expire(&mut self, now: Instant, ttl: Duration) {
        self.by_buffer.retain(|_, queue| {
            queue.retain(|p| now.duration_since(p.sent) < ttl);
            !queue.is_empty()
        });
    }

    /// Forget everything pending in the buffers of one connection, for a disconnect.
    pub(crate) fn clear_connection(&mut self, conn_prefix: &str) {
        let prefix = format!("{}/", conn_prefix);
        self.by_buffer.retain(|id, _| !id.starts_with(&prefix));
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.by_buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_message_is_matched_once() {
        let mut p = PendingEchoes::default();
        let id = p.add("w/1", "hello", Instant::now());
        assert_eq!(p.take_match("w/1", "hello"), Some(id));
        assert_eq!(p.take_match("w/1", "hello"), None);
        assert!(p.is_empty());
    }

    #[test]
    fn identical_messages_match_in_the_order_sent() {
        let mut p = PendingEchoes::default();
        let (a, b) = (p.add("w/1", "ok", Instant::now()), p.add("w/1", "ok", Instant::now()));
        assert_ne!(a, b);
        assert_eq!(p.take_match("w/1", "ok"), Some(a));
        assert_eq!(p.take_match("w/1", "ok"), Some(b));
    }

    #[test]
    fn rapid_messages_match_whatever_order_they_come_back_in() {
        let mut p = PendingEchoes::default();
        let (a, b, c) = (
            p.add("w/1", "one", Instant::now()),
            p.add("w/1", "two", Instant::now()),
            p.add("w/1", "three", Instant::now()),
        );
        assert_eq!(p.take_match("w/1", "two"), Some(b));
        assert_eq!(p.take_match("w/1", "one"), Some(a));
        assert_eq!(p.take_match("w/1", "three"), Some(c));
    }

    #[test]
    fn buffers_and_text_must_agree() {
        let mut p = PendingEchoes::default();
        p.add("w/1", "hello", Instant::now());
        assert_eq!(p.take_match("w/2", "hello"), None);
        assert_eq!(p.take_match("w/1", "hello there"), None);
        assert!(!p.is_empty());
    }

    #[test]
    fn an_action_matches_by_its_whole_text() {
        let mut p = PendingEchoes::default();
        let id = p.add("w/1", "sam waves", Instant::now());
        // A message that only ends in the same words is a different one.
        assert_eq!(p.take_match("w/1", "waves"), None);
        assert_eq!(p.take_match("w/1", "bob sam waves"), None);
        assert_eq!(p.take_match("w/1", "sam waves"), Some(id));
    }

    #[test]
    fn lost_echoes_expire() {
        let mut p = PendingEchoes::default();
        let t = Instant::now();
        p.add("w/1", "old", t);
        p.add("w/1", "new", t + Duration::from_secs(8));
        p.expire(t + Duration::from_secs(11), PENDING_TTL);
        assert_eq!(p.take_match("w/1", "old"), None);
        assert!(p.take_match("w/1", "new").is_some());
        p.add("w/2", "x", t);
        p.expire(t + Duration::from_secs(30), PENDING_TTL);
        assert!(p.is_empty());
    }

    #[test]
    fn a_disconnect_clears_only_its_connection() {
        let mut p = PendingEchoes::default();
        p.add("w/1", "a", Instant::now());
        p.add("z/1", "b", Instant::now());
        p.clear_connection("w");
        assert_eq!(p.take_match("w/1", "a"), None);
        assert!(p.take_match("z/1", "b").is_some());
    }

    fn line(id: &str, prefix: &str, message: &str) -> Line {
        Line::new(id.into(), chrono::Utc::now(), prefix.into(), message.into(), true, false)
    }

    #[test]
    fn the_relay_line_replaces_the_local_one_in_place() {
        let mut p = PendingEchoes::default();
        let mut messages: VecDeque<Line> = VecDeque::new();
        let a = p.add("w/1", "first", Instant::now());
        messages.push_back(line(&a, "sam", "first"));
        let b = p.add("w/1", "second", Instant::now());
        messages.push_back(line(&b, "sam", "second"));
        // Someone else talks in between, and the relay answers the second first.
        messages.push_back(line("40", "bob", "hi"));
        assert!(p.reconcile("w/1", &mut messages, &line("42", "@sam", "second")));
        assert!(p.reconcile("w/1", &mut messages, &line("41", "@sam", "first")));
        let ids: Vec<_> = messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["41", "42", "40"]);
        assert_eq!(messages[0].prefix, "@sam");
        assert!(p.is_empty());
    }

    #[test]
    fn colours_in_the_relay_text_do_not_stop_a_match() {
        let mut p = PendingEchoes::default();
        let mut messages: VecDeque<Line> = VecDeque::new();
        let a = p.add("w/1", "hello", Instant::now());
        messages.push_back(line(&a, "sam", "hello"));
        assert!(p.reconcile("w/1", &mut messages, &line("7", "sam", "\x1B[31mhello\x1B[0m")));
    }

    #[test]
    fn a_line_nobody_is_waiting_for_is_left_to_the_caller() {
        let mut p = PendingEchoes::default();
        let mut messages: VecDeque<Line> = VecDeque::new();
        assert!(!p.reconcile("w/1", &mut messages, &line("7", "sam", "typed in another client")));
        assert!(messages.is_empty());
    }
}
