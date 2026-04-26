use serde::{Deserialize, Serialize};
use serde_json::Value;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;

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
    /// Snapshot of last_read_id taken when the buffer was first entered this session.
    /// Used to anchor the unread divider while the user views the buffer.
    pub visit_start_marker_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub prefix: String,
    pub message: String,
    pub displayed: bool,
}

#[derive(Debug, Clone)]
pub struct Nick {
    pub name: String,
    pub prefix: String,
    pub color_ansi: String,
}
