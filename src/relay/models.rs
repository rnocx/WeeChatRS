use serde::{Deserialize, Serialize};
use serde_json::Value;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Arc;
use crate::ui::ansi::{ANSIParser, ANSISection};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WeeChatResponse {
    pub request_id: Option<String>,
    pub event_name: Option<String>,
    pub code: Option<i64>,
    pub message: Option<String>,
    pub body_type: Option<String>,
    #[serde(default, deserialize_with = "lenient_id")]
    pub buffer_id: Option<i64>,
    pub body: Option<Value>,
}

/// WeeChat writes ids ending in 0 in float notation (`1.79309023254937e+15` for
/// 1793090232549370); accept both so those events aren't dropped.
fn lenient_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)),
        None => None,
    })
}

/// Reply to a completion request (WeeChat `POST /api/completion`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompletionReply {
    /// False when the relay rejected the request (e.g. a WeeChat without the endpoint).
    pub ok: bool,
    /// "null", "command", "command_arg" or "auto".
    pub context: String,
    pub base_word: String,
    /// Character index in the input where the completed word starts.
    pub position_replace: usize,
    pub add_space: bool,
    pub list: Vec<String>,
}

/// What a buffer's messages do, besides muting it: chosen per buffer in its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NotifyLevel {
    /// Every message from someone else notifies.
    All,
    /// Messages count as unread; mentions and private messages notify.
    #[default]
    Default,
    /// Only mentions and private messages count as unread, and notify.
    Mentions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum BufferActivity {
    #[default]
    None = 0,
    Metadata = 1,
    Message = 2,
    Highlight = 3,
}

#[derive(Debug, Clone, Default)]
pub struct Buffer {
    pub id: String,
    pub number: i32,
    pub name: String,
    pub full_name: String,
    pub plugin: String,
    pub kind: String,
    pub server: String,
    /// Behind `Arc` so the render pass can take a cheap handle each frame;
    /// mutate via `Arc::make_mut` (no copy once that handle is dropped).
    pub messages: Arc<VecDeque<Line>>,
    pub nicks: Arc<Vec<Nick>>,
    pub activity: BufferActivity,
    pub unread_count: u32,
    pub last_read_id: Option<String>,
    pub last_markread_ts: Option<DateTime<Utc>>,
    pub topic: String,
    pub modes: String,
    pub hidden: bool,
    pub muted: bool,
    pub notify: NotifyLevel,
    pub has_nicklist: bool,
    /// Snapshot of last_read_id taken when the buffer was first entered this session.
    /// Used to anchor the unread divider while the user views the buffer.
    pub visit_start_marker_id: Option<String>,
}

/// Rank of a nick prefix: `~ & @ % +` first, plain users last. WeeChat puts the
/// highest mode first in the prefix, so only the first non-blank char counts.
fn nick_rank(prefix: &str) -> u8 {
    match prefix.chars().find(|c| !c.is_whitespace()) {
        Some('~') => 0,
        Some('&') => 1,
        Some('@') => 2,
        Some('%') => 3,
        Some('+') => 4,
        _ => 5,
    }
}

/// Nick list order: by rank, then case-insensitive name.
fn nick_cmp(a: &Nick, b: &Nick) -> std::cmp::Ordering {
    nick_rank(&a.prefix)
        .cmp(&nick_rank(&b.prefix))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
}

impl Buffer {
    /// Replace the whole nick list (sorted).
    pub fn set_nicks(&mut self, mut nicks: Vec<Nick>) {
        nicks.sort_by(nick_cmp);
        self.nicks = Arc::new(nicks);
    }

    /// Insert or replace a nick, keeping the list sorted. Use this for every
    /// incremental nick change so no caller has to sort.
    pub fn upsert_nick(&mut self, nick: Nick) {
        let nicks = Arc::make_mut(&mut self.nicks);
        if let Some(i) = nicks.iter().position(|n| n.name == nick.name) {
            nicks.remove(i);
        }
        let pos = nicks.partition_point(|n| nick_cmp(n, &nick).is_lt());
        nicks.insert(pos, nick);
    }

    /// Insert a nick only if it isn't present yet. Returns whether it was added.
    pub fn add_nick(&mut self, nick: Nick) -> bool {
        if self.nicks.iter().any(|n| n.name == nick.name) {
            return false;
        }
        self.upsert_nick(nick);
        true
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Line {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub prefix: String,
    pub message: String,
    pub displayed: bool,
    /// The line mentions us: tinted, and it notifies.
    pub highlight: bool,
    /// It notifies and marks the buffer like a highlight without being one: a
    /// private message, as WeeChat's `notify_private`.
    pub notify: bool,
    // Cached: parsed once at insertion. Theme/font-independent — resolved at render.
    pub parsed_prefix: Vec<ANSISection>,
    pub parsed_message: Vec<ANSISection>,
    // Cached plain text (ANSI stripped) and lowercased copies for search.
    pub plain_prefix: String,
    pub plain_message: String,
    pub plain_prefix_lower: String,
    pub plain_message_lower: String,
    /// Set on a join, part, quit, kick or nick-change line, for the smart filter.
    pub presence: Option<Presence>,
}

/// What a presence line is about, as the smart filter needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Presence {
    pub kind: PresenceKind,
    /// Who joined, left, was kicked or changed nick (the old nick).
    pub nick: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PresenceKind {
    Join,
    Part,
    Quit,
    Kick,
    /// A nick change, to this nick.
    Nick(String),
    /// A host or services account change.
    Account,
}

impl Line {
    pub fn new(
        id: String,
        timestamp: DateTime<Utc>,
        prefix: String,
        message: String,
        displayed: bool,
        highlight: bool,
    ) -> Self {
        let parsed_prefix = ANSIParser::parse(&prefix);
        let parsed_message = ANSIParser::parse(&message);
        let plain_prefix: String = parsed_prefix.iter().map(|s| s.text.as_str()).collect();
        let plain_message: String = parsed_message.iter().map(|s| s.text.as_str()).collect();
        let plain_prefix_lower = plain_prefix.to_lowercase();
        let plain_message_lower = plain_message.to_lowercase();
        Self {
            id,
            timestamp,
            prefix,
            message,
            displayed,
            highlight,
            notify: highlight,
            parsed_prefix,
            parsed_message,
            plain_prefix,
            plain_message,
            plain_prefix_lower,
            plain_message_lower,
            presence: None,
        }
    }

    /// The same line, marked as a join, part, quit, kick or nick change.
    pub fn with_presence(mut self, kind: PresenceKind, nick: &str) -> Self {
        self.presence = Some(Presence { kind, nick: nick.to_string() });
        self
    }

    /// The same line, notifying when `notify` is set even if it isn't a highlight.
    pub fn notifying(mut self, notify: bool) -> Self {
        self.notify = self.highlight || notify;
        self
    }
}

/// A channel from the server's `LIST`: its name, how many are in it, and its topic
/// as ANSI (from the IRC formatting).
#[derive(Debug, Clone, PartialEq)]
pub struct ListedChannel {
    pub name: String,
    pub users: u32,
    pub topic: String,
}

#[derive(Debug, Clone)]
pub struct Nick {
    pub name: String,
    pub prefix: String,
    pub color_ansi: String,
    pub away: bool,
}

/// The ANSI colour slot (1–6, 9–14) for a nick the server didn't colour. Black, white
/// and the greys are left out: they read as uncoloured, and are pale on light themes.
pub fn nick_color_slot(name: &str) -> u8 {
    const SLOTS: [u8; 12] = [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14];
    let mut h: u32 = 0;
    for b in name.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as u32);
    }
    SLOTS[(h % SLOTS.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::WeeChatResponse;

    #[test]
    fn buffer_id_in_float_notation_is_accepted() {
        let resp: WeeChatResponse = serde_json::from_str(
            r#"{"code":0,"event_name":"buffer_cleared","buffer_id":1.79309023254937e+15,"body":null}"#,
        ).unwrap();
        assert_eq!(resp.buffer_id, Some(1_793_090_232_549_370));
        let resp: WeeChatResponse = serde_json::from_str(r#"{"code":0,"buffer_id":1793090232226898}"#).unwrap();
        assert_eq!(resp.buffer_id, Some(1_793_090_232_226_898));
        let resp: WeeChatResponse = serde_json::from_str(r#"{"code":204}"#).unwrap();
        assert_eq!(resp.buffer_id, None);
    }
}