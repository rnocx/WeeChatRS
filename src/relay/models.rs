use serde::{Deserialize, Serialize};
use serde_json::Value;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::ui::ansi::{ANSIParser, ANSISection};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WeeChatResponse {
    pub request_id: Option<String>,
    pub event_name: Option<String>,
    pub code: Option<i64>,
    pub message: Option<String>,
    pub body_type: Option<String>,
    pub buffer_id: Option<i64>,
    pub body: Option<Value>,
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
    pub messages: VecDeque<Line>,
    pub nicks: Vec<Nick>,
    pub activity: BufferActivity,
    pub unread_count: u32,
    pub last_read_id: Option<String>,
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
