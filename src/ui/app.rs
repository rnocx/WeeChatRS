use crate::relay::client::{RelayClient, RelayEvent};
use crate::relay::models::*;
use crate::ui::ansi::ANSIParser;
use crate::ui::theme::AppTheme;
use egui::{FontId, ScrollArea, Label, Key, Visuals, TextStyle, FontFamily, Color32, text::LayoutJob, Margin, Frame, Rounding, Stroke, Vec2, Modifiers, text_edit::TextEditState, Rect, Painter};
use tokio::sync::mpsc;
use chrono::{Utc, DateTime};
use serde_json::Value;
use serde::{Deserialize, Serialize};

const MAX_MESSAGES: usize = 400;

#[derive(Serialize, Deserialize)]
struct AppSettings {
    host: String,
    port: String,
    use_ssl: bool,
    show_filtered_lines: bool,
    colored_nicks: bool,
    theme: AppTheme,
    font_size: f32,
    use_monospace: bool,
    show_timestamps: bool,
    show_buffers: bool,
    show_nicklist: bool,
    auto_reconnect: bool,
    show_titlebar: bool,
    opacity: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: "9001".to_string(),
            use_ssl: true,
            show_filtered_lines: false,
            colored_nicks: true,
            theme: AppTheme::default(),
            font_size: 14.0,
            use_monospace: true,
            show_timestamps: true,
            show_buffers: true,
            show_nicklist: true,
            auto_reconnect: true,
            show_titlebar: true,
            opacity: 1.0,
        }
    }
}

pub struct WeeChatApp {
    host: String,
    port: String,
    password: String,
    use_ssl: bool,
    
    client: Option<RelayClient>,
    event_rx: mpsc::UnboundedReceiver<RelayEvent>,
    event_tx: mpsc::UnboundedSender<RelayEvent>,
    
    connection_status: String,
    is_connecting: bool,
    buffers: Vec<Buffer>,
    selected_buffer_id: Option<String>,
    input_text: String,
    #[allow(dead_code)]
    debug_log: Vec<String>,

    // Settings
    show_settings: bool,
    show_filtered_lines: bool,
    colored_nicks: bool,
    theme: AppTheme,
    font_size: f32,
    use_monospace: bool,
    show_timestamps: bool,
    auto_reconnect: bool,
    show_titlebar: bool,
    opacity: f32,

    // UI visibility
    show_buffers: bool,
    show_nicklist: bool,

    // Completion state
    completion: Option<CompletionState>,

    // Command History
    command_history: Vec<String>,
    history_index: Option<usize>,

    // Search state
    show_search: bool,
    search_text: String,

    // Navigation
    pending_buffer_switch: Option<String>,
}

struct CompletionState {
    #[allow(dead_code)]
    original_word: String,
    matches: Vec<String>,
    index: usize,
    word_start_idx: usize,
}

impl WeeChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        let settings: AppSettings = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            AppSettings::default()
        };

        Self {
            host: settings.host,
            port: settings.port,
            password: "".to_string(),
            use_ssl: settings.use_ssl,
            client: None,
            event_rx,
            event_tx,
            connection_status: "".to_string(),
            is_connecting: false,
            buffers: Vec::new(),
            selected_buffer_id: None,
            input_text: String::new(),
            debug_log: Vec::new(),
            show_settings: false,
            show_filtered_lines: settings.show_filtered_lines,
            colored_nicks: settings.colored_nicks,
            theme: settings.theme,
            font_size: settings.font_size,
            use_monospace: settings.use_monospace,
            show_timestamps: settings.show_timestamps,
            show_buffers: settings.show_buffers,
            show_nicklist: settings.show_nicklist,
            auto_reconnect: settings.auto_reconnect,
            show_titlebar: settings.show_titlebar,
            opacity: settings.opacity,
            completion: None,
            command_history: Vec::new(),
            history_index: None,
            show_search: false,
            search_text: String::new(),
            pending_buffer_switch: None,
        }
    }

    fn handle_event(&mut self, event: RelayEvent) {
        match event {
            RelayEvent::Connecting => {
                self.is_connecting = true;
                self.connection_status = "Connecting...".to_string();
            }
            RelayEvent::Connected => {
                self.is_connecting = false;
                self.connection_status = "Connected".to_string();
                if let Some(client) = &self.client {
                    client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                    client.send_api("POST /api/sync", None, Some(serde_json::json!({"colors": "ansi", "buffers": "all"})));
                    client.send_api("GET /api/hotlist", Some("_hotlist"), None);
                }
            }
            RelayEvent::Disconnected => {
                self.is_connecting = false;
                if !self.auto_reconnect {
                    self.client = None;
                }
                self.connection_status = "Disconnected".to_string();
            }
            RelayEvent::Error(e) => {
                self.connection_status = format!("Error: {}", e);
                if !self.auto_reconnect {
                    self.client = None;
                }
            }
            RelayEvent::Message(resp) => {
                self.process_response(resp);
            }
        }
    }

    fn process_response(&mut self, resp: WeeChatResponse) {
        if let Some(id) = &resp.request_id {
            if id == "_list_buffers" {
                self.handle_buffer_list(resp);
                return;
            } else if id == "_hotlist" {
                self.handle_hotlist(resp);
                return;
            } else if id.starts_with("_buffer_lines:") {
                let buffer_id = id[14..].to_string();
                self.handle_buffer_lines(&buffer_id, resp);
                return;
            } else if id.starts_with("_nicks:") {
                let buffer_id = id[7..].to_string();
                self.handle_nick_list(&buffer_id, resp);
                return;
            } else if id.starts_with("_buffer_info:") {
                let buffer_id = id[13..].to_string();
                self.handle_buffer_info(&buffer_id, resp);
                return;
            }
        }

        if let Some(event) = &resp.event_name {
            match event.as_str() {
                "buffer_line_added" => self.handle_line_added(resp),
                "buffer_line_changed" => self.handle_line_changed(resp),
                "buffer_opened" | "buffer_closed" | "buffer_renamed" | "buffer_localvar_added" | "buffer_localvar_changed" | "buffer_localvar_removed" => {
                    if let Some(client) = &self.client {
                        client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                    }
                }
                _ => {}
            }
        }
    }

    fn body_as_vec(resp: &WeeChatResponse) -> Vec<&Value> {
        match &resp.body {
            Some(Value::Array(a)) => a.iter().collect(),
            Some(obj) => vec![obj],
            None => vec![],
        }
    }

    fn parse_date(val: Option<&Value>) -> DateTime<Utc> {
        if let Some(s) = val.and_then(|v| v.as_str()) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return dt.with_timezone(&Utc);
            }
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                return dt.and_utc();
            }
        }
        Utc::now()
    }

    fn extract_metadata(obj: &serde_json::Map<String, Value>, topic: &mut String, modes: &mut String, kind: &mut String, server: &mut String, full_name: &str, plugin: &str) {
        // 1. Initial Source of Truth
        if full_name == "weechat" || plugin == "core" {
            *kind = "core".to_string();
            *server = "00_core".to_string();
            return;
        }

        if let Some(vars) = obj.get("local_variables").and_then(|v| v.as_object()) {
            if let Some(t) = vars.get("topic").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            if let Some(m) = vars.get("modes").and_then(|v| v.as_str()) { *modes = m.to_string(); }
            if let Some(k) = vars.get("type").and_then(|v| v.as_str()) { *kind = k.to_string(); }
            if let Some(s) = vars.get("server").and_then(|v| v.as_str()) { *server = s.to_lowercase(); }
        }
        
        // 2. Topic Fallbacks
        if topic.is_empty() {
            if let Some(t) = obj.get("title").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            else if let Some(t) = obj.get("topic").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            else if let Some(t) = obj.get("topic_string").and_then(|v| v.as_str()) { *topic = t.to_string(); }
        }

        // 3. Robust Hierarchy Restoration
        if server.is_empty() {
            let parts: Vec<&str> = full_name.split('.').collect();
            if parts.len() >= 2 {
                if parts[0] == "irc" { *server = parts[1].to_lowercase(); }
                else { *server = parts[0].to_lowercase(); }
            } else {
                *server = if !plugin.is_empty() { plugin.to_lowercase() } else { "z_orphans".to_string() };
            }
        }

        if kind.is_empty() {
            let parts: Vec<&str> = full_name.split('.').collect();
            if parts.len() <= 2 || parts.contains(&"server") { *kind = "server".to_string(); }
            else { *kind = "channel".to_string(); }
        }
    }

    fn sort_buffers(buffers: &mut Vec<Buffer>) {
        buffers.sort_by(|a, b| {
            // Group by normalized server key
            if a.server != b.server {
                return a.server.cmp(&b.server);
            }
            // Same group: Server root always first
            if a.kind == "server" && b.kind != "server" { return std::cmp::Ordering::Less; }
            if b.kind == "server" && a.kind != "server" { return std::cmp::Ordering::Greater; }
            // Then by buffer number
            a.number.cmp(&b.number)
        });
    }

    fn handle_buffer_list(&mut self, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        let mut new_buffers = Vec::new();
        for val in body {
            if let Some(obj) = val.as_object() {
                let id = obj.get("id")
                    .and_then(|v| v.as_i64().map(|i| i.to_string())
                        .or_else(|| v.as_str().map(|s| s.to_string())));
                
                let number = obj.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                
                let name = obj.get("short_name").and_then(|v| v.as_str())
                    .or_else(|| obj.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("unknown")
                    .to_string();
                    
                let full_name = obj.get("name").and_then(|v| v.as_str())
                    .unwrap_or(&name)
                    .to_string();

                let plugin = obj.get("plugin").and_then(|v| v.as_str()).unwrap_or("").to_string();

                if let Some(id) = id {
                    let mut topic = String::new();
                    let mut modes = String::new();
                    let mut kind = String::new();
                    let mut server = String::new();
                    
                    let mut messages = Vec::new();
                    let mut nicks = Vec::new();
                    let mut activity = BufferActivity::None;
                    let mut last_read_id = None;
                    
                    if let Some(existing) = self.buffers.iter().find(|b| b.id == id) {
                        messages = existing.messages.clone();
                        nicks = existing.nicks.clone();
                        activity = existing.activity;
                        last_read_id = existing.last_read_id.clone();
                    }

                    Self::extract_metadata(obj, &mut topic, &mut modes, &mut kind, &mut server, &full_name, &plugin);

                    new_buffers.push(Buffer {
                        id,
                        number,
                        name,
                        full_name,
                        plugin,
                        kind,
                        server,
                        messages,
                        nicks,
                        activity,
                        last_read_id,
                        topic,
                        modes,
                    });
                }
            }
        }
        if !new_buffers.is_empty() {
            Self::sort_buffers(&mut new_buffers);
            self.buffers = new_buffers;

            if let Some(target) = self.pending_buffer_switch.take() {
                if let Some(found) = self.buffers.iter().find(|b| b.name == target || b.full_name.ends_with(&target)) {
                    let id = found.id.clone();
                    self.select_buffer(id);
                } else {
                    self.pending_buffer_switch = Some(target);
                }
            }

            if self.selected_buffer_id.is_none() {
                if let Some(first) = self.buffers.first() {
                    let id = first.id.clone();
                    self.select_buffer(id);
                }
            }
        }
    }

    fn handle_hotlist(&mut self, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                let buffer_id = obj.get("buffer_id").and_then(|v| v.as_i64()).map(|i| i.to_string());
                let priority = obj.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
                
                if let Some(buffer_id) = buffer_id {
                    if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                        buffer.activity = match priority {
                            3 => BufferActivity::Highlight,
                            2 => BufferActivity::Message,
                            1 => BufferActivity::Metadata,
                            _ => BufferActivity::None,
                        };
                    }
                }
            }
        }
    }

    fn handle_buffer_lines(&mut self, buffer_id: &str, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        let lines: Vec<Line> = body.iter().filter_map(|val| {
            let obj = val.as_object()?;
            let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);
            let id = obj.get("id").and_then(|v| v.as_i64()).map(|i| i.to_string()).unwrap_or_else(|| "unknown".to_string());
            let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
            let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let timestamp = Self::parse_date(obj.get("date"));
            
            Some(Line {
                id,
                timestamp,
                prefix: prefix.to_string(),
                message: message.to_string(),
                displayed,
            })
        }).collect();

        if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
            buffer.messages = lines;
            if buffer.messages.len() > MAX_MESSAGES {
                let start = buffer.messages.len() - MAX_MESSAGES;
                buffer.messages.drain(0..start);
            }
            if self.selected_buffer_id.as_ref() == Some(&buffer_id.to_string()) {
                if let Some(last) = buffer.messages.last() {
                    buffer.last_read_id = Some(last.id.clone());
                }
            }
        }
    }

    fn handle_buffer_info(&mut self, buffer_id: &str, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                    let full_name = buffer.full_name.clone();
                    let plugin = buffer.plugin.clone();
                    Self::extract_metadata(obj, &mut buffer.topic, &mut buffer.modes, &mut buffer.kind, &mut buffer.server, &full_name, &plugin);
                }
            }
        }
    }

    fn handle_nick_list(&mut self, buffer_id: &str, resp: WeeChatResponse) {
        if let Some(body) = &resp.body {
            let mut nicks = Vec::new();
            self.extract_nicks(body, &mut nicks);
            if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                buffer.nicks = nicks;
            }
        }
    }

    fn extract_nicks(&self, val: &Value, nicks: &mut Vec<Nick>) {
        if let Some(obj) = val.as_object() {
            if let Some(Value::Array(nick_arr)) = obj.get("nicks") {
                for n in nick_arr {
                    if let Some(no) = n.as_object() {
                        let name = no.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let prefix = no.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                        let color = no.get("color").and_then(|v| v.as_str()).unwrap_or("");
                        nicks.push(Nick {
                            name: name.to_string(),
                            prefix: prefix.to_string(),
                            color_ansi: color.to_string(),
                        });
                    }
                }
            }
            if let Some(Value::Array(groups)) = obj.get("groups") {
                for g in groups {
                    self.extract_nicks(g, nicks);
                }
            }
        }
    }

    fn strip_ansi(text: &str) -> String {
        let re = regex::Regex::new(r"\x1B\[[0-9;]*[A-Za-z]").unwrap();
        re.replace_all(text, "").to_string()
    }

    fn handle_line_added(&mut self, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);
                let buffer_id = resp.buffer_id.map(|i| i.to_string())
                    .or_else(|| obj.get("buffer_id").and_then(|v| v.as_i64()).map(|i| i.to_string()));
                
                let is_highlight = obj.get("highlight").and_then(|v| v.as_bool()).unwrap_or(false);
                let notify_level = obj.get("notify_level").and_then(|v| v.as_i64()).unwrap_or(0);

                if let Some(buffer_id) = buffer_id {
                    let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                    let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let id = obj.get("id").and_then(|v| v.as_i64()).map(|i| i.to_string()).unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string());
                    let timestamp = Self::parse_date(obj.get("date"));
                    
                    let line = Line {
                        id,
                        timestamp,
                        prefix: prefix.to_string(),
                        message: message.to_string(),
                        displayed,
                    };

                    if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                        if !buffer.messages.iter().any(|m| m.id == line.id) {
                            buffer.messages.push(line.clone());
                            if buffer.messages.len() > MAX_MESSAGES {
                                buffer.messages.remove(0);
                            }
                            
                            if self.selected_buffer_id.as_ref() == Some(&buffer_id) {
                                buffer.last_read_id = Some(line.id.clone());
                            } else if displayed {
                                 let activity = if is_highlight || notify_level == 3 {
                                     BufferActivity::Highlight
                                 } else if notify_level == 2 {
                                     BufferActivity::Message
                                     } else if notify_level == 1 {
                                     BufferActivity::Metadata
                                 } else {
                                     buffer.activity
                                 };
                                 
                                 if activity > buffer.activity {
                                     buffer.activity = activity;
                                 }
                                 
                                 if is_highlight || notify_level == 3 {
                                     let sender = Self::strip_ansi(prefix);
                                     let text = Self::strip_ansi(message);
                                     let body = if sender.is_empty() { text } else { format!("{}: {}", sender, text) };

                                     let _ = notify_rust::Notification::new()
                                         .summary(&buffer.name)
                                         .body(&body)
                                         .show();
                                 }
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_line_changed(&mut self, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                let buffer_id = resp.buffer_id.map(|i| i.to_string())
                    .or_else(|| obj.get("buffer_id").and_then(|v| v.as_i64()).map(|i| i.to_string()));
                
                let line_id = obj.get("id").and_then(|v| v.as_i64()).map(|i| i.to_string());
                let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);

                if let (Some(buffer_id), Some(line_id)) = (buffer_id, line_id) {
                    if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                        if let Some(line) = buffer.messages.iter_mut().find(|m| m.id == line_id) {
                            line.displayed = displayed;
                        } else if displayed {
                            let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                            let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
                            let timestamp = Self::parse_date(obj.get("date"));
                            let line = Line {
                                id: line_id,
                                timestamp,
                                prefix: prefix.to_string(),
                                message: message.to_string(),
                                displayed,
                            };
                            buffer.messages.push(line);
                            if buffer.messages.len() > MAX_MESSAGES {
                                buffer.messages.remove(0);
                            }
                        }
                    }
                }
            }
        }
    }

    fn select_buffer(&mut self, id: String) {
        self.selected_buffer_id = Some(id.clone());
        if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == id) {
            buffer.activity = BufferActivity::None;
            if let Some(client) = &self.client {
                client.send_api(&format!("GET /api/buffers/{}", id), Some(&format!("_buffer_info:{}", id)), None);
                client.send_api(&format!("GET /api/buffers/{}/lines?lines=-{}", id, MAX_MESSAGES), Some(&format!("_buffer_lines:{}", id)), None);
                client.send_api(&format!("GET /api/buffers/{}/nicks", id), Some(&format!("_nicks:{}", id)), None);
                
                client.send_api("POST /api/input", None, Some(serde_json::json!({
                    "buffer_id": id.parse::<i64>().unwrap_or(0),
                    "command": "/buffer set localvar_set_read_marker 1"
                })));
            }
        }
    }

    fn hash_nick(name: &str) -> u8 {
        let mut h: u32 = 0;
        for b in name.as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(*b as u32);
        }
        ((h % 15) + 1) as u8
    }

    fn send_current_message(&mut self) {
        if !self.input_text.is_empty() {
            let msg = self.input_text.clone();
            let is_command = msg.starts_with('/');
            
            if let Some(client) = &self.client {
                if let Some(buffer) = self.selected_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)) {
                    if let Ok(numeric_id) = buffer.id.parse::<i64>() {
                        client.send_api("POST /api/input", None, Some(serde_json::json!({
                            "buffer_id": numeric_id,
                            "command": msg.clone()
                        })));
                    }
                }
                
                if is_command {
                    if msg.starts_with("/query ") {
                        let nick = msg[7..].split_whitespace().next().map(|s| s.to_string());
                        self.pending_buffer_switch = nick;
                    } else if msg.starts_with("/join ") {
                        let chan = msg[6..].split_whitespace().next().map(|s| s.to_string());
                        self.pending_buffer_switch = chan;
                    }
                    client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                }
            }
            
            if self.command_history.last() != Some(&msg) {
                self.command_history.push(msg);
                if self.command_history.len() > 100 {
                    self.command_history.remove(0);
                }
            }
            
            self.input_text.clear();
            self.completion = None;
            self.history_index = None;
        }
    }

    fn perform_completion(&mut self, ctx: &egui::Context, id: egui::Id) {
        let nicks = match self.selected_buffer_id.as_ref()
            .and_then(|id| self.buffers.iter().find(|b| &b.id == id))
            .map(|b| b.nicks.clone()) {
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
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(new_cursor_pos))));
                state.store(ctx, id);
            }
        }
    }

    fn cycle_buffer(&mut self, delta: i32) {
        if self.buffers.is_empty() { return; }
        let current_id = match &self.selected_buffer_id {
            Some(id) => id,
            None => {
                if let Some(first) = self.buffers.first() {
                    let id = first.id.clone();
                    self.select_buffer(id);
                }
                return;
            }
        };

        if let Some(idx) = self.buffers.iter().position(|b| &b.id == current_id) {
            let new_idx = (idx as i32 + delta).rem_euclid(self.buffers.len() as i32) as usize;
            let new_id = self.buffers[new_idx].id.clone();
            self.select_buffer(new_id);
        }
    }

    fn cycle_history(&mut self, delta: i32, ctx: &egui::Context, id: egui::Id) {
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
                if delta < 0 {
                    Some(self.command_history.len() - 1)
                } else {
                    None
                }
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
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(pos))));
            state.store(ctx, id);
        }
    }

    fn send_command(&mut self, command: &str) {
        if command.starts_with("/query ") {
            self.pending_buffer_switch = Some(command[7..].trim().to_string());
        } else if command.starts_with("/join ") {
            self.pending_buffer_switch = Some(command[6..].trim().to_string());
        }

        if let Some(client) = &self.client {
            if let Some(buffer) = self.selected_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)) {
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

    fn draw_sidebar_icon(painter: &Painter, rect: Rect, color: Color32, is_right: bool) {
        let stroke = Stroke::new(1.5, color);
        let rounding = Rounding::same(2.0);
        painter.rect_stroke(rect.shrink(4.0), rounding, stroke);
        
        let split_x = if is_right { rect.right() - 8.0 } else { rect.left() + 8.0 };
        painter.line_segment(
            [egui::pos2(split_x, rect.top() + 4.0), egui::pos2(split_x, rect.bottom() - 4.0)],
            stroke
        );
    }
}

impl eframe::App for WeeChatApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = AppSettings {
            host: self.host.clone(),
            port: self.port.clone(),
            use_ssl: self.use_ssl,
            show_filtered_lines: self.show_filtered_lines,
            colored_nicks: self.colored_nicks,
            theme: self.theme.clone(),
            font_size: self.font_size,
            use_monospace: self.use_monospace,
            show_timestamps: self.show_timestamps,
            show_buffers: self.show_buffers,
            show_nicklist: self.show_nicklist,
            auto_reconnect: self.auto_reconnect,
            show_titlebar: self.show_titlebar,
            opacity: self.opacity,
        };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event);
        }

        let mut tab_pressed = false;
        let mut arrow_up_shortcut = false;
        let mut arrow_down_shortcut = false;
        let mut history_up = false;
        let mut history_down = false;
        let mut search_shortcut = false;

        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Tab) {
                tab_pressed = true;
            }
            
            let meta = i.modifiers.command || i.modifiers.alt || i.modifiers.mac_cmd || i.modifiers.ctrl;
            if meta {
                if i.consume_key(i.modifiers, Key::ArrowUp) || i.key_pressed(Key::ArrowUp) { arrow_up_shortcut = true; }
                if i.consume_key(i.modifiers, Key::ArrowDown) || i.key_pressed(Key::ArrowDown) { arrow_down_shortcut = true; }
                if i.consume_key(i.modifiers, Key::F) { search_shortcut = true; }
                if i.consume_key(i.modifiers, Key::B) { self.show_buffers = !self.show_buffers; }
                if i.consume_key(i.modifiers, Key::N) { self.show_nicklist = !self.show_nicklist; }
            } else {
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) { history_up = true; }
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) { history_down = true; }
            }
        });

        if arrow_up_shortcut { self.cycle_buffer(-1); }
        if arrow_down_shortcut { self.cycle_buffer(1); }
        if search_shortcut { self.show_search = !self.show_search; }

        let mut style = (*ctx.style()).clone();
        let font_family = if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional };
        
        style.text_styles = [
            (TextStyle::Small, FontId::new(self.font_size * 0.8, font_family.clone())),
            (TextStyle::Body, FontId::new(self.font_size, font_family.clone())),
            (TextStyle::Button, FontId::new(self.font_size, font_family.clone())),
            (TextStyle::Heading, FontId::new(self.font_size * 1.4, font_family.clone())),
            (TextStyle::Monospace, FontId::new(self.font_size, FontFamily::Monospace)),
        ].into();
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
        style.spacing.window_margin = Margin::same(12.0);
        style.visuals.window_rounding = Rounding::same(12.0);
        style.visuals.widgets.noninteractive.rounding = Rounding::same(8.0);
        style.visuals.widgets.inactive.rounding = Rounding::same(8.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(8.0);
        style.visuals.widgets.active.rounding = Rounding::same(8.0);
        ctx.set_style(style);

        let mut visuals = Visuals::dark();
        let accent_color = Color32::from_rgb(100, 149, 237);
        let base_bg = self.theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18));
        let alpha = (self.opacity * 255.0) as u8;
        let bg_color = Color32::from_rgba_unmultiplied(base_bg.r(), base_bg.g(), base_bg.b(), alpha);
        let surface_color = Color32::from_rgba_unmultiplied(30, 30, 30, alpha);
        
        visuals.panel_fill = bg_color;
        visuals.window_fill = bg_color;
        visuals.extreme_bg_color = Color32::from_rgba_unmultiplied(10, 10, 10, alpha);
        visuals.widgets.active.bg_fill = accent_color;
        visuals.selection.bg_fill = accent_color.linear_multiply(0.5);
        
        if let Some(fg) = self.theme.foreground {
            visuals.override_text_color = Some(fg.into());
        }
        ctx.set_visuals(visuals);

        let mut next_selected_buffer_id = None;

        egui::TopBottomPanel::top("top_panel")
            .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(12.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    
                    let icon_size = Vec2::splat(24.0);
                    
                    let (rect, res) = ui.allocate_at_least(icon_size, egui::Sense::click());
                    if res.clicked() { self.show_buffers = !self.show_buffers; }
                    let color = if self.show_buffers { accent_color } else { Color32::GRAY };
                    Self::draw_sidebar_icon(ui.painter(), rect, color, false);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, res) = ui.allocate_at_least(icon_size, egui::Sense::click());
                        if res.clicked() { self.show_nicklist = !self.show_nicklist; }
                        let color = if self.show_nicklist { accent_color } else { Color32::GRAY };
                        Self::draw_sidebar_icon(ui.painter(), rect, color, true);
                        ui.add_space(8.0);

                        if ui.button(egui::RichText::new("⚙").size(16.0)).on_hover_text("Settings").clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        if self.client.is_some() {
                            let status_text = if self.is_connecting { "● Connecting" } else { "● Connected" };
                            let status_color = if self.is_connecting { Color32::from_rgb(255, 165, 0) } else { Color32::from_rgb(50, 205, 50) };
                            ui.label(egui::RichText::new(status_text).color(status_color).small());
                            
                            if ui.button("Disconnect").clicked() {
                                if let Some(client) = &self.client {
                                    client.disconnect();
                                }
                                self.client = None;
                                self.connection_status = "Disconnected".to_string();
                            }
                        }
                    });
                });
            });

        if self.show_buffers {
            egui::SidePanel::left("buffers_panel")
                .resizable(true)
                .default_width(180.0)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("BUFFERS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);
                    
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for buffer in &self.buffers {
                            let is_selected = self.selected_buffer_id.as_ref() == Some(&buffer.id);
                            
                            let (bg, fg) = if is_selected {
                                (accent_color.linear_multiply(0.2), Color32::WHITE)
                            } else {
                                match buffer.activity {
                                    BufferActivity::Highlight => (Color32::from_rgb(150, 50, 50).linear_multiply(0.3), Color32::from_rgb(255, 100, 100)),
                                    BufferActivity::Message => (Color32::TRANSPARENT, Color32::WHITE),
                                    BufferActivity::Metadata => (Color32::TRANSPARENT, Color32::from_rgb(130, 130, 130)),
                                    BufferActivity::None => (Color32::TRANSPARENT, Color32::from_rgb(100, 100, 100)),
                                }
                            };

                            let is_child = buffer.kind == "channel" || buffer.kind == "private";
                            let indent = if is_child { 12.0 } else { 0.0 };

                            ui.horizontal(|ui| {
                                ui.add_space(indent);
                                let response = Frame::none()
                                    .fill(bg)
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(Margin::symmetric(8.0, 4.0))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        let text = if buffer.activity == BufferActivity::Highlight {
                                            format!("• {}", buffer.name)
                                        } else {
                                            buffer.name.clone()
                                        };
                                        ui.label(egui::RichText::new(text).color(fg).strong());
                                    }).response;

                                let response = ui.interact(response.rect, response.id, egui::Sense::click());
                                if response.clicked() {
                                    next_selected_buffer_id = Some(buffer.id.clone());
                                }
                            });
                        }
                    });
                });
        }

        if let Some(id) = next_selected_buffer_id {
            self.select_buffer(id);
        }

        if self.show_settings {
            let mut show_settings = self.show_settings;
            let mut show_filtered_lines = self.show_filtered_lines;
            let mut colored_nicks = self.colored_nicks;
            let mut font_size = self.font_size;
            let mut use_monospace = self.use_monospace;
            let mut show_timestamps = self.show_timestamps;
            let mut auto_reconnect = self.auto_reconnect;
            let mut show_titlebar = self.show_titlebar;
            let mut opacity = self.opacity;
            let mut close_clicked = false;
            let mut reset_theme = false;
            
            egui::Window::new("Settings").open(&mut show_settings).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.add_space(8.0);
                ui.checkbox(&mut show_filtered_lines, "Show filtered lines");
                ui.checkbox(&mut colored_nicks, "Colored nicknames in list");
                ui.checkbox(&mut show_timestamps, "Show timestamps");
                ui.checkbox(&mut auto_reconnect, "Auto-reconnect on drop");
                ui.checkbox(&mut show_titlebar, "Show Topic/Modes Titlebar");
                
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Appearance").strong());
                ui.horizontal(|ui| {
                    ui.label("Font size:");
                    ui.add(egui::Slider::new(&mut font_size, 8.0..=32.0));
                });
                ui.checkbox(&mut use_monospace, "Use Monospace font everywhere");
                ui.horizontal(|ui| {
                    ui.label("Opacity:");
                    ui.add(egui::Slider::new(&mut opacity, 0.1..=1.0));
                });

                ui.separator();
                ui.label(egui::RichText::new("Theme").strong());
                ui.label(format!("Current: {}", self.theme.name));
                ui.horizontal(|ui| {
                    if ui.button("Import .itermcolors").clicked() {
                       if let Some(path) = rfd::FileDialog::new().add_filter("itermcolors", &["itermcolors"]).pick_file() {
                            if let Ok(data) = std::fs::read(&path) {
                                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                                if let Ok(new_theme) = AppTheme::parse_itermcolors(&data, name) {
                                    self.theme = new_theme;
                                }
                            }
                        }
                    }
                    if ui.button("Reset to Default").clicked() { reset_theme = true; }
                });

                ui.add_space(20.0);
                ui.vertical_centered_justified(|ui| {
                    if ui.button("Close").clicked() { close_clicked = true; }
                });
            });
            
            self.show_settings = if close_clicked { false } else { show_settings };
            self.show_filtered_lines = show_filtered_lines;
            self.colored_nicks = colored_nicks;
            self.font_size = font_size;
            self.use_monospace = use_monospace;
            self.show_timestamps = show_timestamps;
            self.auto_reconnect = auto_reconnect;
            self.show_titlebar = show_titlebar;
            self.opacity = opacity;
            if reset_theme { self.theme = AppTheme::default(); }
        }

        let current_buffer_id = self.selected_buffer_id.clone();
        let current_buffer_nicks = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).map(|b| b.nicks.clone());
        let current_buffer_full_name = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).map(|b| b.full_name.clone());
        let current_buffer_messages = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).map(|b| b.messages.clone());
        let current_buffer_last_read_id = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).and_then(|b| b.last_read_id.clone());
        let current_buffer_topic = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).map(|b| b.topic.clone()).unwrap_or_default();
        let current_buffer_modes = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).map(|b| b.modes.clone()).unwrap_or_default();
        let current_buffer_kind = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id)).map(|b| b.kind.clone()).unwrap_or_default();

        let font_id = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });

        let is_query_or_core = current_buffer_kind == "private" || current_buffer_kind == "server" || current_buffer_full_name.as_ref().map(|n| n == "weechat").unwrap_or(false);

        if self.show_nicklist && !is_query_or_core && self.client.is_some() && current_buffer_id.is_some() {
            egui::SidePanel::right("nicks_panel")
                .resizable(true)
                .default_width(140.0)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("NICKS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);
                    ScrollArea::vertical().show(ui, |ui| {
                        if let Some(nicks) = &current_buffer_nicks {
                            for nick in nicks {
                                let text = format!("{}{}", nick.prefix, nick.name);
                                let input = if self.colored_nicks {
                                    if self.theme.name == "Default" { format!("{}{}", nick.color_ansi, text) }
                                    else {
                                        let idx = Self::hash_nick(&nick.name);
                                        let esc = if idx < 8 { format!("\x1B[{}m", 30 + idx) } else { format!("\x1B[{}m", 90 + idx - 8) };
                                        format!("{}{}", esc, text)
                                    }
                                } else { text };
                                let sections = ANSIParser::parse(&input, font_id.clone(), &self.theme);
                                let mut job = LayoutJob::default();
                                for s in sections { job.append(&s.text, 0.0, s.format); }
                                
                                let label_res = ui.add(Label::new(job).wrap(false).sense(egui::Sense::click()));
                                label_res.context_menu(|ui| {
                                    if ui.button(format!("Query {}", nick.name)).clicked() {
                                        self.send_command(&format!("/query {}", nick.name));
                                        ui.close_menu();
                                    }
                                    if ui.button(format!("Whois {}", nick.name)).clicked() {
                                        self.send_command(&format!("/whois {}", nick.name));
                                        ui.close_menu();
                                    }
                                });
                            }
                        }
                    });
                });
        }

        if self.client.is_some() && current_buffer_id.is_some() {
            egui::TopBottomPanel::bottom("input_panel")
                .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(16.0, 10.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let text_edit = egui::TextEdit::singleline(&mut self.input_text)
                            .hint_text("Type a message...")
                            .margin(Margin::symmetric(8.0, 4.0))
                            .lock_focus(true) 
                            .desired_width(ui.available_width() - 80.0);
                        
                        let res = ui.add(text_edit);
                        
                        if res.has_focus() {
                            if tab_pressed { 
                                self.perform_completion(ctx, res.id); 
                                res.request_focus();
                            } else if history_up {
                                self.cycle_history(-1, ctx, res.id);
                                res.request_focus();
                            } else if history_down {
                                self.cycle_history(1, ctx, res.id);
                                res.request_focus();
                            } else {
                                let any_other_key = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
                                if any_other_key && !tab_pressed && !history_up && !history_down {
                                    self.completion = None;
                                }
                            }
                        }

                        if ui.add(egui::Button::new("Send").min_size(Vec2::new(60.0, 0.0))).clicked() || (res.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter))) {
                            self.send_current_message();
                            res.request_focus();
                        }
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
            if self.client.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(ctx.available_rect().height() * 0.2);
                    
                    Frame::group(ui.style())
                        .fill(surface_color)
                        .rounding(Rounding::same(12.0))
                        .stroke(Stroke::new(1.0, Color32::from_gray(60)))
                        .inner_margin(Margin::same(30.0))
                        .show(ui, |ui| {
                            ui.set_max_width(400.0);
                            ui.heading(egui::RichText::new("Connect to WeeChatRS").strong().size(24.0));
                            ui.add_space(20.0);
                            
                            egui::Grid::new("login_grid").num_columns(2).spacing([15.0, 15.0]).show(ui, |ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Host:"); });
                                ui.add(egui::TextEdit::singleline(&mut self.host).desired_width(240.0));
                                ui.end_row();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Port:"); });
                                ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(240.0));
                                ui.end_row();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Password:"); });
                                ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(240.0));
                                ui.end_row();
                            });
                            
                            ui.add_space(15.0);
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.use_ssl, "Use SSL");
                                ui.add_space(20.0);
                                ui.checkbox(&mut self.auto_reconnect, "Auto-reconnect");
                            });
                            ui.add_space(25.0);
                            
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(egui::RichText::new("Connect").strong()).min_size(Vec2::new(120.0, 40.0))).clicked() {
                                    let port = self.port.parse().unwrap_or(9001);
                                    self.client = Some(RelayClient::connect(self.host.clone(), port, self.password.clone(), self.use_ssl, self.event_tx.clone()));
                                    self.connection_status = "Connecting...".to_string();
                                }
                                if ui.button("Save Profile").clicked() {
                                    ctx.memory_mut(|m| m.data.insert_persisted(egui::Id::NULL, ())); 
                                }
                            });
                            
                            ui.add_space(15.0);
                            if !self.connection_status.is_empty() {
                                ui.label(egui::RichText::new(&self.connection_status).color(if self.connection_status.starts_with("Error") { Color32::from_rgb(255, 100, 100) } else { accent_color }));
                            }
                        });
                });
            } else if let Some(_full_name) = current_buffer_full_name {
                ui.vertical(|ui| {
                    if self.show_search {
                        Frame::none()
                            .fill(surface_color.linear_multiply(0.8))
                            .inner_margin(Margin::symmetric(16.0, 8.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("🔍");
                                    let res = ui.add(egui::TextEdit::singleline(&mut self.search_text)
                                        .hint_text("Search scrollback...")
                                        .desired_width(ui.available_width() - 40.0));
                                    if ui.button("❌").clicked() {
                                        self.show_search = false;
                                        self.search_text.clear();
                                    }
                                    if self.show_search { res.request_focus(); }
                                });
                            });
                        ui.separator();
                    }

                    if self.show_titlebar && (!current_buffer_topic.is_empty() || !current_buffer_modes.is_empty()) {
                        Frame::none()
                            .fill(surface_color.linear_multiply(0.3))
                            .inner_margin(Margin::symmetric(16.0, 6.0))
                            .stroke(Stroke::new(1.0, Color32::from_white_alpha(10)))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    if !current_buffer_modes.is_empty() {
                                        ui.label(egui::RichText::new(format!("[{}]", current_buffer_modes)).color(accent_color).small());
                                    }
                                    if !current_buffer_topic.is_empty() {
                                        let topic_font = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });
                                        let sections = ANSIParser::parse(&current_buffer_topic, topic_font, &self.theme);
                                        let mut job = LayoutJob::default();
                                        for s in sections { job.append(&s.text, 0.0, s.format); }
                                        ui.add(Label::new(job).wrap(true)); 
                                    }
                                });
                            });
                        ui.add_space(-1.0);
                        ui.separator();
                    }
                    
                    ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        Frame::none().inner_margin(Margin::same(16.0)).show(ui, |ui| {
                            if let Some(messages) = &current_buffer_messages {
                                let mut marker_shown = false;
                                for line in messages {
                                    if !self.show_filtered_lines && !line.displayed { continue; }
                                    
                                    if !self.search_text.is_empty() {
                                        let clean_prefix = Self::strip_ansi(&line.prefix).to_lowercase();
                                        let clean_msg = Self::strip_ansi(&line.message).to_lowercase();
                                        let q = self.search_text.to_lowercase();
                                        if !clean_prefix.contains(&q) && !clean_msg.contains(&q) { continue; }
                                    }

                                    if !marker_shown && current_buffer_last_read_id.is_some() && &line.id > current_buffer_last_read_id.as_ref().unwrap() {
                                        ui.add_space(8.0);
                                        ui.horizontal(|ui| {
                                            ui.add_space(20.0);
                                            ui.separator();
                                            ui.label(egui::RichText::new(" NEW MESSAGES ").color(Color32::from_rgb(255, 100, 100)).size(10.0).strong());
                                            ui.separator();
                                        });
                                        ui.add_space(8.0);
                                        marker_shown = true;
                                    }

                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        if self.show_timestamps {
                                            ui.label(egui::RichText::new(line.timestamp.format("%H:%M:%S").to_string()).font(font_id.clone()).color(Color32::from_gray(100)));
                                        }
                                        let prefix_sections = ANSIParser::parse(&line.prefix, font_id.clone(), &self.theme);
                                        let mut prefix_job = LayoutJob::default();
                                        for s in prefix_sections { prefix_job.append(&s.text, 0.0, s.format); }
                                        ui.label(prefix_job);
                                        let msg_sections = ANSIParser::parse(&line.message, font_id.clone(), &self.theme);
                                        for s in msg_sections {
                                            if let Some(url) = s.url {
                                                if ui.link(egui::RichText::new(&s.text).font(font_id.clone())).clicked() { ui.ctx().output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url))); }
                                            } else {
                                                let mut job = LayoutJob::default();
                                                job.append(&s.text, 0.0, s.format);
                                                ui.add(Label::new(job).wrap(true));
                                            }
                                        }
                                    });
                                }
                            }
                        });
                    });
                });
            } else {
                ui.centered_and_justified(|ui| { ui.label(egui::RichText::new("Select a buffer to start chatting").color(Color32::from_gray(100)).size(16.0)); });
            }
        });

        if self.client.is_some() { ctx.request_repaint(); }
    }
}
