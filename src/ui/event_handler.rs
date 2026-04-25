use crate::relay::client::RelayEvent;
use crate::relay::models::*;
use crate::ui::app::{WeeChatApp, MAX_MESSAGES};
use chrono::{Utc, DateTime, Local};
use serde_json::Value;
use std::sync::OnceLock;

static ANSI_RE: OnceLock<regex::Regex> = OnceLock::new();

fn ansi_re() -> &'static regex::Regex {
    ANSI_RE.get_or_init(|| regex::Regex::new(r"\x1B\[[0-9;]*[A-Za-z]").unwrap())
}

impl WeeChatApp {
    pub(crate) fn handle_event(&mut self, event: RelayEvent) {
        match event {
            RelayEvent::Connecting => {
                self.is_connecting = true;
                self.connection_status = "Connecting...".to_string();
            }
            RelayEvent::Connected => {
                self.is_connecting = false;
                self.connection_status = "Connected".to_string();
                self.buffers.clear(); 
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
                self.buffers.clear();
                self.selected_buffer_id = None;
                self.connection_status = "Disconnected".to_string();
            }
            RelayEvent::Error(e) => {
                self.connection_status = format!("Error: {}", e);
                if !self.auto_reconnect {
                    self.client = None;
                    self.buffers.clear();
                }
            }
            RelayEvent::Message(resp) => {
                self.process_response(resp);
            }
        }
    }

    pub(crate) fn process_response(&mut self, resp: WeeChatResponse) {
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
                "buffer_opened" | "buffer_closed" | "buffer_renamed"
                | "buffer_localvar_added" | "buffer_localvar_changed"
                | "buffer_localvar_removed" => {
                    if let Some(client) = &self.client {
                        client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_id(v: &Value) -> Option<String> {
        v.as_i64().map(|i| i.to_string())
            .or_else(|| v.as_f64().map(|f| (f as i64).to_string()))
            .or_else(|| v.as_str().map(|s| s.to_string()))
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
            // No timezone in string — treat as Unix timestamp (seconds) if numeric,
            // otherwise assume the relay's local time and convert to UTC via the local offset.
            if let Ok(secs) = s.parse::<i64>() {
                if let Some(dt) = DateTime::from_timestamp(secs, 0) {
                    return dt;
                }
            }
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                // Interpret as local time so we don't offset by the relay server's timezone.
                return chrono::TimeZone::from_local_datetime(&Local, &dt)
                    .single()
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);
            }
        }
        Utc::now()
    }

    fn extract_metadata(
        obj: &serde_json::Map<String, Value>,
        topic: &mut String,
        modes: &mut String,
        kind: &mut String,
        server: &mut String,
        full_name: &str,
        plugin: &str,
    ) {
        *kind = "unknown".to_string();
        *server = "orphans".to_string();

        if full_name == "weechat" || plugin == "core" || full_name == "core.weechat" {
            *kind = "core".to_string();
            *server = "!00_core".to_string(); // Forced to top
            return;
        }

        if let Some(vars) = obj.get("local_variables").and_then(|v| v.as_object()) {
            if let Some(t) = vars.get("topic").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            if let Some(m) = vars.get("modes").and_then(|v| v.as_str()) { *modes = m.to_string(); }
            if let Some(k) = vars.get("type").and_then(|v| v.as_str()) { *kind = k.to_string(); }
            if let Some(s) = vars.get("server").and_then(|v| v.as_str()) { *server = s.to_string().to_lowercase(); }
        }
        
        if topic.is_empty() {
            if let Some(t) = obj.get("title").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            else if let Some(t) = obj.get("topic").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            else if let Some(t) = obj.get("topic_string").and_then(|v| v.as_str()) { *topic = t.to_string(); }
        }

        if plugin == "irc" {
            let parts: Vec<&str> = full_name.split('.').collect();
            if parts.len() >= 2 {
                let net_candidate = if parts[1] == "server" && parts.len() >= 3 {
                    if *kind == "unknown" { *kind = "server".to_string(); }
                    parts[2]
                } else {
                    parts[1]
                };
                // local_variables.server is authoritative — only fall back to
                // full_name parsing when it wasn't present in the relay data.
                if *server == "orphans" {
                    *server = net_candidate.to_string().to_lowercase();
                }
            }
            if *kind == "unknown" {
                if parts.len() <= 2 || (parts.len() == 3 && parts[1] == "server") { *kind = "server".to_string(); }
                else { *kind = "channel".to_string(); }
            }
        } else if !plugin.is_empty() {
             if *server == "orphans" { *server = plugin.to_lowercase(); }
             if *kind == "unknown" { *kind = "server".to_string(); }
        }
    }

    fn sort_buffers(buffers: &mut Vec<Buffer>) {
        buffers.sort_by(|a, b| {
            // Group by server key
            if a.server != b.server {
                return a.server.cmp(&b.server);
            }
            
            // Inside group: Roots (core/server) always first
            let a_is_root = a.kind == "core" || a.kind == "server";
            let b_is_root = b.kind == "core" || b.kind == "server";
            
            if a_is_root && !b_is_root { return std::cmp::Ordering::Less; }
            if b_is_root && !a_is_root { return std::cmp::Ordering::Greater; }

            // Then by buffer number
            a.number.cmp(&b.number)
        });
    }

    fn handle_buffer_list(&mut self, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        let mut new_buffers = Vec::new();

        for val in body {
            if let Some(obj) = val.as_object() {
                let id = obj.get("id").and_then(|v| Self::parse_id(v));

                let number = obj.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let name = obj.get("short_name").and_then(|v| v.as_str())
                    .or_else(|| obj.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("unknown").to_string();
                let full_name = obj.get("name").and_then(|v| v.as_str()).unwrap_or(&name).to_string();
                let plugin = obj.get("plugin").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let hidden = obj.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false);
                // Relay sends the server-side read marker; use it so the unread divider
                // is correct immediately on connect without needing any prior local state.
                let relay_last_read_id = obj.get("last_read_line_id").and_then(|v| Self::parse_id(v))
                    .or_else(|| obj.get("last_read_line").and_then(|v| v.as_object()).and_then(|o| o.get("id")).and_then(|v| Self::parse_id(v)));

                if let Some(id) = id {
                    let mut topic = String::new();
                    let mut modes = String::new();
                    let mut kind = String::new();
                    let mut server = String::new();

                    let mut messages = std::collections::VecDeque::new();
                    let mut nicks = Vec::new();
                    let mut activity = BufferActivity::None;
                    let mut last_read_id = None;

                    if let Some(existing) = self.buffers.iter().find(|b| b.id == id) {
                        messages = existing.messages.clone();
                        nicks = existing.nicks.clone();
                        activity = existing.activity;
                        last_read_id = existing.last_read_id.clone();
                    }
                    // Relay's read marker takes priority — it's the authoritative baseline
                    // for showing the unread divider after a fresh connect.
                    if relay_last_read_id.is_some() {
                        last_read_id = relay_last_read_id.clone();
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
                        hidden,
                    });
                }
            }
        }

        if !new_buffers.is_empty() {
            Self::sort_buffers(&mut new_buffers);
            self.buffers = new_buffers;

            // Re-apply user's custom ordering. Buffers not in the order (e.g. newly
            // opened query windows) are anchored just after their server header so they
            // appear in the right group rather than being dumped at the end.
            if !self.buffer_order.is_empty() {
                let order = &self.buffer_order;
                let max_order = order.len();

                // Position of each server group's header in buffer_order.
                let server_header_pos: std::collections::HashMap<String, usize> = self.buffers.iter()
                    .filter(|b| b.kind == "server" || b.kind == "core")
                    .filter_map(|b| {
                        order.iter().position(|id| id == &b.id)
                            .map(|pos| (b.server.clone(), pos))
                    })
                    .collect();

                self.buffers.sort_by_key(|b| {
                    if let Some(pos) = order.iter().position(|id| id == &b.id) {
                        pos * 10_000
                    } else {
                        // Anchor to the server header slot; fall back to end if server unknown.
                        let base = server_header_pos.get(&b.server)
                            .map(|&p| p * 10_000 + 1)
                            .unwrap_or(max_order * 10_000 + 1);
                        base + b.number as usize
                    }
                });
            }

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
                let buffer_id = obj.get("buffer_id").and_then(|v| Self::parse_id(v));
                let priority = obj.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);

                if let Some(buffer_id) = buffer_id {
                    // Skip the buffer the user is currently viewing.
                    if self.selected_buffer_id.as_deref() == Some(&buffer_id) {
                        continue;
                    }
                    // Skip buffers the user has explicitly read in this or a previous session.
                    // This suppresses stale hotlist entries when the server-side read call is
                    // unavailable (older WeeChat versions).
                    if self.cleared_buffer_ids.contains(&buffer_id) {
                        continue;
                    }
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
            let id = obj.get("id").and_then(|v| Self::parse_id(v))
                .unwrap_or_else(|| "unknown".to_string());
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
            let mut deque: std::collections::VecDeque<Line> = lines.into();
            if deque.len() > MAX_MESSAGES {
                let excess = deque.len() - MAX_MESSAGES;
                deque.drain(0..excess);
            }
            buffer.messages = deque;
            // Only mark everything as read when loading for the first time (no prior
            // read marker). On subsequent reloads the existing marker must be kept so
            // the unread divider stays visible for messages that arrived since the user
            // last viewed this buffer.
            if buffer.last_read_id.is_none() {
                if let Some(last) = buffer.messages.back() {
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

    pub(crate) fn strip_ansi(text: &str) -> String {
        ansi_re().replace_all(text, "").to_string()
    }

    fn handle_line_added(&mut self, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);
                let buffer_id = resp.buffer_id.map(|i| i.to_string())
                    .or_else(|| obj.get("buffer_id").and_then(|v| Self::parse_id(v)));

                let is_highlight = obj.get("highlight").and_then(|v| v.as_bool()).unwrap_or(false);
                let notify_level = obj.get("notify_level").and_then(|v| v.as_i64()).unwrap_or(0);

                if let Some(buffer_id) = buffer_id {
                    let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                    let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let id = obj.get("id").and_then(|v| Self::parse_id(v))
                        .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string());
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
                            buffer.messages.push_back(line.clone());
                            if buffer.messages.len() > MAX_MESSAGES {
                                buffer.messages.pop_front();
                            }

                            if self.selected_buffer_id.as_deref() == Some(&buffer_id) {
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
                                    // A real new message arrived while the user isn't watching —
                                    // evict from cleared set so it shows as unread on next reconnect.
                                    self.cleared_buffer_ids.remove(&buffer_id);
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
                    .or_else(|| obj.get("buffer_id").and_then(|v| Self::parse_id(v)));
                let line_id = obj.get("id").and_then(|v| Self::parse_id(v));
                let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);

                if let (Some(buffer_id), Some(line_id)) = (buffer_id, line_id) {
                    if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                        if let Some(line) = buffer.messages.iter_mut().find(|m| m.id == line_id) {
                            line.displayed = displayed;
                        } else if displayed {
                            let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                            let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
                            let timestamp = Self::parse_date(obj.get("date"));
                            buffer.messages.push_back(Line {
                                id: line_id,
                                timestamp,
                                prefix: prefix.to_string(),
                                message: message.to_string(),
                                displayed,
                            });
                            if buffer.messages.len() > MAX_MESSAGES {
                                buffer.messages.pop_front();
                            }
                        }
                    }
                }
            }
        }
    }
}
