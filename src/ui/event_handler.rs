use crate::relay::client::RelayEvent;
use crate::relay::models::*;
use crate::ui::app::{WeeChatApp, MAX_MESSAGES};
use chrono::{Utc, DateTime};
use serde_json::Value;

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

    fn extract_metadata(
        obj: &serde_json::Map<String, Value>,
        topic: &mut String,
        modes: &mut String,
        kind: &mut String,
        server: &mut String,
        full_name: &str,
        plugin: &str,
    ) {
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

        if topic.is_empty() {
            if let Some(t) = obj.get("title").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            else if let Some(t) = obj.get("topic").and_then(|v| v.as_str()) { *topic = t.to_string(); }
            else if let Some(t) = obj.get("topic_string").and_then(|v| v.as_str()) { *topic = t.to_string(); }
        }

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
            if a.server != b.server { return a.server.cmp(&b.server); }
            if a.kind == "server" && b.kind != "server" { return std::cmp::Ordering::Less; }
            if b.kind == "server" && a.kind != "server" { return std::cmp::Ordering::Greater; }
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
            let id = obj.get("id").and_then(|v| v.as_i64()).map(|i| i.to_string())
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
            buffer.messages = lines;
            if buffer.messages.len() > MAX_MESSAGES {
                let start = buffer.messages.len() - MAX_MESSAGES;
                buffer.messages.drain(0..start);
            }
            if self.selected_buffer_id.as_deref() == Some(buffer_id) {
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

    pub(crate) fn strip_ansi(text: &str) -> String {
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
                    let id = obj.get("id").and_then(|v| v.as_i64()).map(|i| i.to_string())
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
                            buffer.messages.push(line.clone());
                            if buffer.messages.len() > MAX_MESSAGES {
                                buffer.messages.remove(0);
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
                            buffer.messages.push(Line {
                                id: line_id,
                                timestamp,
                                prefix: prefix.to_string(),
                                message: message.to_string(),
                                displayed,
                            });
                            if buffer.messages.len() > MAX_MESSAGES {
                                buffer.messages.remove(0);
                            }
                        }
                    }
                }
            }
        }
    }
}
