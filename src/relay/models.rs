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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BufferActivity {
    None = 0,
    Metadata = 1,
    Message = 2,
    Highlight = 3,
}

#[derive(Debug, Clone)]
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
    pub has_nicklist: bool,
    /// Snapshot of last_read_id taken when the buffer was first entered this session.
    /// Used to anchor the unread divider while the user views the buffer.
    pub visit_start_marker_id: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Line {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub prefix: String,
    pub message: String,
    pub displayed: bool,
    pub highlight: bool,
    // Cached: parsed once at insertion. Theme/font-independent — resolved at render.
    pub parsed_prefix: Vec<ANSISection>,
    pub parsed_message: Vec<ANSISection>,
    // Cached plain text (ANSI stripped) and lowercased copies for search.
    pub plain_prefix: String,
    pub plain_message: String,
    pub plain_prefix_lower: String,
    pub plain_message_lower: String,
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
            parsed_prefix,
            parsed_message,
            plain_prefix,
            plain_message,
            plain_prefix_lower,
            plain_message_lower,
        }
    }
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
