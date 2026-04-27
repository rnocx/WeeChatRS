use crate::relay::client::RelayEvent;
use crate::relay::models::*;
use crate::ui::app::{WeeChatApp, MAX_STORED_LINES};
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
                self.connection_attempts += 1;
                if self.connection_attempts == 1 {
                    self.log_conn("TCP connection + WebSocket handshake in progress…");
                } else {
                    let backoff = {
                        let secs = 1u64 << (self.connection_attempts - 2).min(4);
                        secs.min(30)
                    };
                    self.log_conn(format!(
                        "Reconnect attempt {} (backoff was {}s)…",
                        self.connection_attempts - 1,
                        backoff
                    ));
                }
            }
            RelayEvent::Connected => {
                self.is_connecting = false;
                self.connecting_pending = false;
                self.auth_error = None;
                self.connection_status = "Connected".to_string();
                self.log_conn("WebSocket handshake complete");
                self.log_conn("Sending credentials via Sec-WebSocket-Protocol bearer token");
                self.log_conn("Authentication accepted by relay");
                self.log_conn("→ GET /api/buffers");
                self.log_conn("→ POST /api/sync  {colors: ansi, input: false}");
                self.log_conn("→ GET /api/hotlist");
                self.log_conn("Connected");
                self.buffers.clear();
                // Keep cleared_buffer_ids across reconnects. The local record of which
                // buffers the user has read is more reliable than the server's hotlist
                // when POST /api/buffers/{id}/read hasn't been fully acknowledged
                // (e.g. the connection dropped before WeeChat processed it).
                // New live messages still update activity via buffer_line_added events.
                if let Some(client) = &self.client {
                    client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                    client.send_api("POST /api/sync", None, Some(serde_json::json!({"colors": "ansi", "input": false})));
                    client.send_api("GET /api/hotlist", Some("_hotlist"), None);
                }
            }
            RelayEvent::Disconnected => {
                self.is_connecting = false;
                if self.connecting_pending {
                    self.connecting_pending = false;
                    self.auth_error = Some("Connection closed before auth completed — check your password and relay settings.".to_string());
                    self.log_conn("WebSocket closed by server before authentication completed");
                    self.log_conn("Check: relay password, relay plugin loaded, port correct");
                    self.client = None;
                    self.connection_status = String::new();
                } else {
                    if !self.auto_reconnect {
                        self.client = None;
                    }
                    self.buffers.clear();
                    self.selected_buffer_id = None;
                    self.connection_status = "Disconnected".to_string();
                    if self.auto_reconnect {
                        self.log_conn("Disconnected — auto-reconnect is ON, will retry");
                    } else {
                        self.log_conn("Disconnected");
                    }
                }
            }
            RelayEvent::Error(e) => {
                if self.connecting_pending {
                    let is_auth = e.contains("401") || e.contains("403")
                        || e.to_lowercase().contains("unauthorized")
                        || e.to_lowercase().contains("forbidden");
                    self.connecting_pending = false;
                    if is_auth {
                        self.log_conn(format!("Auth error: {}", e));
                        self.log_conn("Wrong password or relay not configured to accept this connection");
                    } else {
                        self.log_conn(format!("Connection error: {}", e));
                    }
                    self.auth_error = Some(if is_auth {
                        "Wrong password or relay not configured to accept this connection.".to_string()
                    } else {
                        format!("Connection failed: {}", e)
                    });
                    self.client = None;
                    self.connection_status = String::new();
                } else {
                    self.log_conn(format!("Error: {}", e));
                    self.connection_status = format!("Error: {}", e);
                    if !self.auto_reconnect {
                        self.client = None;
                        self.buffers.clear();
                    }
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
                "buffer_line_data_changed" => self.handle_line_changed(resp),
                "buffer_hidden" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(buf) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                            buf.hidden = true;
                        }
                    }
                }
                "buffer_unhidden" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(buf) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                            buf.hidden = false;
                        }
                    }
                }
                "buffer_title_changed" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(client) = &self.client {
                            client.send_api(
                                &format!("GET /api/buffers/{}", buffer_id),
                                Some(&format!("_buffer_info:{}", buffer_id)),
                                None,
                            );
                        }
                    }
                }
                "nicklist_nick_added" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(nick) = resp.body.as_ref().and_then(|b| b.as_object()).and_then(|o| Self::parse_nick_obj(o)) {
                            if let Some(buf) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                                if !buf.nicks.iter().any(|n| n.name == nick.name) {
                                    buf.nicks.push(nick);
                                    buf.nicks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                }
                            }
                        }
                    }
                }
                "nicklist_nick_removing" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(name) = resp.body.as_ref().and_then(|b| b.get("name")).and_then(|v| v.as_str()) {
                            if let Some(buf) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                                buf.nicks.retain(|n| n.name != name);
                            }
                        }
                    }
                }
                "nicklist_nick_changed" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(updated) = resp.body.as_ref().and_then(|b| b.as_object()).and_then(|o| Self::parse_nick_obj(o)) {
                            if let Some(buf) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                                if let Some(existing) = buf.nicks.iter_mut().find(|n| n.name == updated.name) {
                                    *existing = updated;
                                } else {
                                    buf.nicks.push(updated);
                                    buf.nicks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                }
                            }
                        }
                    }
                }
                "buffer_cleared" => {
                    if let Some(buffer_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(buf) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                            buf.messages.clear();
                            buf.last_read_id = None;
                            buf.visit_start_marker_id = None;
                        }
                    }
                }
                "upgrade" => {
                    self.connection_status = "WeeChat upgrading…".to_string();
                    self.log_conn("WeeChat is upgrading — waiting for reload…");
                }
                "upgrade_ended" => {
                    // WeeChat finished reloading — re-fetch everything to get a clean state.
                    if let Some(client) = &self.client {
                        client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                        client.send_api("GET /api/hotlist", Some("_hotlist"), None);
                    }
                    self.connection_status = "Connected".to_string();
                    self.log_conn("WeeChat upgrade complete — re-synced");
                }
                "buffer_opened" | "buffer_closed" | "buffer_renamed"
                | "buffer_localvar_added" | "buffer_localvar_changed"
                | "buffer_localvar_removed" => {
                    if let Some(client) = &self.client {
                        client.send_api("GET /api/buffers", Some("_list_buffers"), None);
                    }
                }
                "buffer_hotlist_added" | "buffer_hotlist_updated" => {
                    self.handle_hotlist(resp);
                }
                "buffer_hotlist_removed" => {
                    // WeeChat cleared a hotlist entry (user read it elsewhere).
                    // Re-fetch the full hotlist so our state stays in sync.
                    if let Some(client) = &self.client {
                        client.send_api("GET /api/hotlist", Some("_hotlist"), None);
                    }
                }
                _ => {}
            }
        }
    }

    fn has_tag(obj: &serde_json::Map<String, Value>, tag: &str) -> bool {
        match obj.get("tags") {
            Some(Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some(tag)),
            Some(Value::String(s)) => s.split(',').any(|t| t.trim() == tag),
            _ => false,
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
                let has_nicklist = obj.get("nicklist").and_then(|v| v.as_bool()).unwrap_or(true);
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
                    let mut unread_count = 0u32;
                    let mut last_read_id = None;
                    let mut visit_start_marker_id = None;

                    if let Some(existing) = self.buffers.iter().find(|b| b.id == id) {
                        messages = existing.messages.clone();
                        nicks = existing.nicks.clone();
                        activity = existing.activity;
                        unread_count = existing.unread_count;
                        last_read_id = existing.last_read_id.clone();
                        visit_start_marker_id = existing.visit_start_marker_id.clone();
                    }
                    let muted = self.muted_buffer_names.contains(&full_name);
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
                        unread_count,
                        last_read_id,
                        topic,
                        modes,
                        hidden,
                        muted,
                        has_nicklist,
                        visit_start_marker_id,
                    });
                }
            }
        }

        if !new_buffers.is_empty() {
            let network_count = {
                let mut seen = std::collections::HashSet::new();
                new_buffers.iter().filter(|b| b.kind != "core").for_each(|b| { seen.insert(&b.server); });
                seen.len()
            };
            self.log_conn(format!(
                "← GET /api/buffers  {} buffers, {} network(s)",
                new_buffers.len(), network_count
            ));
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
        let is_initial = resp.request_id.as_deref() == Some("_hotlist");
        let entry_count = body.len();
        if is_initial {
            self.log_conn(format!("← GET /api/hotlist  {} active entr{}", entry_count, if entry_count == 1 { "y" } else { "ies" }));
        }
        for val in body {
            if let Some(obj) = val.as_object() {
                let buffer_id = obj.get("buffer_id").and_then(|v| Self::parse_id(v));
                let priority = obj.get("priority")
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                    .unwrap_or(0);

                if let Some(buffer_id) = buffer_id {
                    // Skip the buffer the user is currently viewing.
                    if self.selected_buffer_id.as_deref() == Some(&buffer_id) {
                        continue;
                    }
                    // Skip muted buffers — they are intentionally silenced.
                    if self.buffers.iter().any(|b| b.id == buffer_id && b.muted) {
                        continue;
                    }
                    // Skip buffers the user has explicitly read in this or a previous session.
                    // This suppresses stale hotlist entries when the server-side read call is
                    // unavailable (older WeeChat versions).
                    if self.cleared_buffer_ids.contains(&buffer_id) {
                        continue;
                    }
                    if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
                        // WeeChat hotlist priorities:
                        // 0 = low (join/part/system), 1 = message, 2 = private, 3 = highlight
                        buffer.activity = match priority {
                            3 => BufferActivity::Highlight,
                            2 | 1 => BufferActivity::Message,
                            _ => BufferActivity::Metadata,
                        };
                        // count is an array: [low_priority, message, private, highlight]
                        if let Some(count_arr) = obj.get("count").and_then(|v| v.as_array()) {
                            let msg  = count_arr.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
                            let priv_msg = count_arr.get(2).and_then(|v| v.as_i64()).unwrap_or(0);
                            let hl   = count_arr.get(3).and_then(|v| v.as_i64()).unwrap_or(0);
                            buffer.unread_count = (msg + priv_msg + hl) as u32;
                        }
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

        let mut log_entry: Option<String> = None;
        if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == buffer_id) {
            let mut deque: std::collections::VecDeque<Line> = lines.into();
            if deque.len() > MAX_STORED_LINES {
                let excess = deque.len() - MAX_STORED_LINES;
                deque.drain(0..excess);
            }
            let line_count = deque.len();
            let is_load_more = self.loading_more_buffer_id.as_deref() == Some(buffer_id);
            log_entry = Some(format!(
                "← GET /api/buffers/{}/lines  {} lines{}  [#{}]",
                buffer_id, line_count,
                if is_load_more { " (load more)" } else { "" },
                buffer.name
            ));
            buffer.messages = deque;
            // Clear the in-flight load-more marker now that the response arrived.
            if is_load_more {
                self.loading_more_buffer_id = None;
            }
            let is_selected = self.selected_buffer_id.as_deref() == Some(buffer_id);
            if is_selected {
                // Advance the persistent marker to the latest message so the next
                // visit to this buffer starts with no divider.  The visual divider
                // for the *current* visit is anchored on visit_start_marker_id which
                // was snapshotted in select_buffer before this update.
                if let Some(last) = buffer.messages.back() {
                    buffer.last_read_id = Some(last.id.clone());
                }
            } else if buffer.last_read_id.is_none() {
                if let Some(last) = buffer.messages.back() {
                    buffer.last_read_id = Some(last.id.clone());
                }
            }
        }
        if let Some(entry) = log_entry {
            self.log_conn(entry);
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

    fn parse_nick_obj(obj: &serde_json::Map<String, Value>) -> Option<Nick> {
        let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() { return None; }
        let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
        let color = obj.get("color").and_then(|v| v.as_str()).unwrap_or("");
        let color_ansi = if color.is_empty() {
            obj.get("color_name").and_then(|v| v.as_str()).unwrap_or("").to_string()
        } else {
            color.to_string()
        };
        Some(Nick { name: name.to_string(), prefix: prefix.to_string(), color_ansi })
    }

    fn extract_nicks(&self, val: &Value, nicks: &mut Vec<Nick>) {
        if let Some(obj) = val.as_object() {
            if let Some(Value::Array(nick_arr)) = obj.get("nicks") {
                for n in nick_arr {
                    if let Some(no) = n.as_object() {
                        if let Some(nick) = Self::parse_nick_obj(no) {
                            nicks.push(nick);
                        }
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

    #[cfg(target_os = "macos")]
    fn osascript_quote(s: &str) -> String {
        // AppleScript strings use double-quotes; escape embedded quotes with \"
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
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
                let notify_level = obj.get("notify_level")
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                    .unwrap_or(0);
                let is_self_msg = Self::has_tag(obj, "self_msg");
                let is_notify_none = Self::has_tag(obj, "notify_none");
                let is_join_part = Self::has_tag(obj, "irc_join")
                    || Self::has_tag(obj, "irc_part")
                    || Self::has_tag(obj, "irc_quit");

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
                            if buffer.messages.len() > MAX_STORED_LINES {
                                buffer.messages.pop_front();
                            }

                            if self.selected_buffer_id.as_deref() == Some(&buffer_id) {
                                buffer.last_read_id = Some(line.id.clone());
                            } else if displayed && !buffer.muted && !is_notify_none && !is_self_msg {
                                let activity = if is_highlight || notify_level == 3 {
                                    BufferActivity::Highlight
                                } else if notify_level == 2 {
                                    BufferActivity::Message
                                } else if is_join_part {
                                    // Join/part lines get a metadata marker but no badge count.
                                    BufferActivity::Metadata
                                } else {
                                    BufferActivity::Metadata
                                };

                                // Only increment the badge for real messages, not join/part noise.
                                if !is_join_part {
                                    buffer.unread_count = buffer.unread_count.saturating_add(1);
                                }

                                if activity > buffer.activity {
                                    buffer.activity = activity;
                                    // A real new message arrived while the user isn't watching —
                                    // evict from cleared set so it shows as unread on next reconnect.
                                    self.cleared_buffer_ids.remove(&buffer_id);
                                }

                                if (is_highlight || notify_level == 3) && !buffer.muted {
                                    let sender = Self::strip_ansi(prefix);
                                    let text = Self::strip_ansi(message);
                                    let body = if sender.is_empty() { text } else { format!("{}: {}", sender, text) };
                                    #[cfg(target_os = "macos")]
                                    {
                                        // notify-rust uses a mac-notification-sys helper app which
                                        // macOS tries to activate on notification click instead of
                                        // WeeChatRS. Use osascript directly to avoid that.
                                        let script = format!(
                                            "display notification {} with title {}",
                                            Self::osascript_quote(&body),
                                            Self::osascript_quote(&buffer.name),
                                        );
                                        let _ = std::process::Command::new("osascript")
                                            .args(["-e", &script])
                                            .spawn();
                                    }
                                    #[cfg(not(target_os = "macos"))]
                                    {
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
                            if buffer.messages.len() > MAX_STORED_LINES {
                                buffer.messages.pop_front();
                            }
                        }
                    }
                }
            }
        }
    }
}
