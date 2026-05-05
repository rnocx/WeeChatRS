use crate::relay::backend::BackendEvent;
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
    pub(crate) fn handle_event(&mut self, conn_prefix: &str, event: BackendEvent) {
        match event {
            BackendEvent::Connected => {
                if let Some(conn) = self.connections.iter_mut().find(|c| c.prefix == conn_prefix) {
                    conn.is_connecting = false;
                    conn.connecting_pending = false;
                    conn.auth_error = None;
                    conn.status = "Connected".to_string();
                    match conn.backend_type {
                        crate::ui::app::BackendType::WeeChat => {
                            let ts = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  WebSocket handshake complete", ts));
                            let ts2 = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Sending credentials via Sec-WebSocket-Protocol bearer token", ts2));
                            let ts3 = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Authentication accepted by relay", ts3));
                            let ts4 = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  → GET /api/buffers", ts4));
                            let ts5 = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  → POST /api/sync  {{colors: ansi, input: false}}", ts5));
                        }
                        crate::ui::app::BackendType::Soju => {
                            let ts = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  IRC registration complete (CAP/NICK/USER)", ts));
                        }
                    }
                    let ts = chrono::Local::now().format("%H:%M").to_string();
                    conn.connection_log.push_back(format!("[{}]  Connected", ts));
                    if conn.connection_log.len() > 500 { conn.connection_log.pop_front(); }
                }
                // Remove stale buffers for this connection then re-fetch
                let pfx = format!("{}/", conn_prefix);
                self.buffers.retain(|b| !b.id.starts_with(&pfx));
                self.rebuild_buffer_idx();
                // Clear suppression set so the fresh hotlist can apply unread counts correctly
                self.cleared_buffer_ids.retain(|id| !id.starts_with(&pfx));
                // Fetch buffer list and sync subscriptions on connected connection
                let conn_prefix_owned = conn_prefix.to_string();
                if let Some(conn) = self.connections.iter().find(|c| c.prefix == conn_prefix_owned) {
                    conn.client.fetch_buffer_list();
                    conn.client.sync_subscriptions();
                }
                if !self.show_connection_log {
                    self.connection_log_unread = true;
                }
            }
            BackendEvent::Disconnected => {
                if let Some(conn) = self.connections.iter_mut().find(|c| c.prefix == conn_prefix) {
                    conn.is_connecting = false;
                    if conn.connecting_pending {
                        conn.connecting_pending = false;
                        conn.auth_error = Some("Connection closed before auth completed — check your password and relay settings.".to_string());
                        let ts = chrono::Local::now().format("%H:%M").to_string();
                        conn.connection_log.push_back(format!("[{}]  WebSocket closed by server before authentication completed", ts));
                        let ts2 = chrono::Local::now().format("%H:%M").to_string();
                        conn.connection_log.push_back(format!("[{}]  Check: relay password, relay plugin loaded, port correct", ts2));
                        conn.status = String::new();
                        if conn.connection_log.len() > 500 { conn.connection_log.pop_front(); }
                    } else {
                        conn.status = "Disconnected".to_string();
                        if conn.auto_reconnect {
                            let ts = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Disconnected — auto-reconnect is ON, will retry", ts));
                        } else {
                            let ts = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Disconnected", ts));
                        }
                        if conn.connection_log.len() > 500 { conn.connection_log.pop_front(); }
                    }
                }
                // Remove buffers for this connection
                let pfx = format!("{}/", conn_prefix);
                self.buffers.retain(|b| !b.id.starts_with(&pfx));
                self.rebuild_buffer_idx();
                if let Some(sel) = &self.selected_buffer_id {
                    if sel.starts_with(&pfx) {
                        self.selected_buffer_id = self.buffers.first().map(|b| b.id.clone());
                    }
                }
                // Remove non-auto-reconnect connections
                let should_remove = self.connections.iter()
                    .find(|c| c.prefix == conn_prefix)
                    .map(|c| !c.auto_reconnect && c.auth_error.is_none())
                    .unwrap_or(false);
                if should_remove {
                    // Keep the handle but mark disconnected so UI shows status
                }
                if !self.show_connection_log {
                    self.connection_log_unread = true;
                }
            }
            BackendEvent::AuthError(e) | BackendEvent::Error(e) => {
                if let Some(conn) = self.connections.iter_mut().find(|c| c.prefix == conn_prefix) {
                    if conn.connecting_pending {
                        let is_auth = e.contains("401") || e.contains("403")
                            || e.to_lowercase().contains("unauthorized")
                            || e.to_lowercase().contains("forbidden");
                        conn.connecting_pending = false;
                        if is_auth {
                            let ts = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Auth error: {}", ts, e));
                            let ts2 = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Wrong password or relay not configured to accept this connection", ts2));
                        } else {
                            let ts = chrono::Local::now().format("%H:%M").to_string();
                            conn.connection_log.push_back(format!("[{}]  Connection error: {}", ts, e));
                        }
                        conn.auth_error = Some(if is_auth {
                            "Wrong password or relay not configured to accept this connection.".to_string()
                        } else {
                            format!("Connection failed: {}", e)
                        });
                        conn.status = String::new();
                        if conn.connection_log.len() > 500 { conn.connection_log.pop_front(); }
                    } else {
                        let ts = chrono::Local::now().format("%H:%M").to_string();
                        conn.connection_log.push_back(format!("[{}]  Error: {}", ts, e));
                        conn.status = format!("Error: {}", e);
                        if conn.connection_log.len() > 500 { conn.connection_log.pop_front(); }
                    }
                }
                if !self.show_connection_log {
                    self.connection_log_unread = true;
                }
            }
            BackendEvent::ConnLog(msg) => {
                self.log_conn_for(conn_prefix, msg);
            }
            BackendEvent::_WeeChat(resp) => {
                self.process_response(conn_prefix, resp);
            }
            BackendEvent::BufferOpened(mut buf) => {
                let full_id = format!("{}/{}", conn_prefix, buf.id);
                let full_full_name = format!("{}/{}", conn_prefix, buf.full_name);
                buf.id = full_id.clone();
                buf.full_name = full_full_name;
                if !self.buffer_idx_of(&full_id).is_some() {
                    self.buffers.push(buf);
                    self.rebuild_buffer_idx();
                }
                // Auto-select when nothing is currently selected (e.g. first buffer on connect).
                if self.selected_buffer_id.is_none() {
                    self.select_buffer(full_id.clone());
                }
                // Resolve a pending /join or /query switch
                if let Some(target) = self.pending_buffer_switch.take() {
                    if let Some(found) = self.buffers.iter().find(|b|
                        b.name == target
                        || b.id == target
                        || b.full_name.ends_with(&target)
                    ) {
                        let id = found.id.clone();
                        self.select_buffer(id);
                    } else {
                        self.pending_buffer_switch = Some(target);
                    }
                }
            }
            BackendEvent::BufferClosed { buffer_id } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                self.buffers.retain(|b| b.id != full_id);
                self.rebuild_buffer_idx();
                if self.selected_buffer_id.as_deref() == Some(&full_id) {
                    self.selected_buffer_id = self.buffers.first().map(|b| b.id.clone());
                }
            }
            BackendEvent::BuffersLoaded(bufs) => {
                // Prefix all buffer ids from this connection, then replace those buffers
                let pfx = format!("{}/", conn_prefix);
                self.buffers.retain(|b| !b.id.starts_with(&pfx));
                for mut buf in bufs {
                    buf.id = format!("{}/{}", conn_prefix, buf.id);
                    buf.full_name = format!("{}/{}", conn_prefix, buf.full_name);
                    self.buffers.push(buf);
                }
                self.rebuild_buffer_idx();
            }
            BackendEvent::LineAdded { buffer_id, line } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                let is_selected = self.selected_buffer_id.as_deref() == Some(full_id.as_str());
                if line.highlight {
                    log::info!(
                        "LineAdded highlight=true buffer={} selected={} displayed={}",
                        full_id, is_selected, line.displayed
                    );
                }
                let mut should_notify = false;
                let mut buf_name = String::new();
                let mut notify_prefix = String::new();
                let mut notify_message = String::new();
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    if !is_selected && !buf.muted && line.displayed {
                        if line.highlight {
                            buf.activity = crate::relay::models::BufferActivity::Highlight;
                            should_notify = true;
                            buf_name = buf.name.clone();
                            notify_prefix = line.prefix.clone();
                            notify_message = line.message.clone();
                        } else if buf.activity != crate::relay::models::BufferActivity::Highlight {
                            buf.activity = crate::relay::models::BufferActivity::Message;
                        }
                        buf.unread_count = buf.unread_count.saturating_add(1);
                    }
                    buf.messages.push_back(line);
                    if buf.messages.len() > MAX_STORED_LINES {
                        buf.messages.pop_front();
                    }
                }
                if should_notify {
                    self.notify_highlight(&full_id, &buf_name, &notify_prefix, &notify_message);
                }
            }
            BackendEvent::NicklistLoaded { buffer_id, mut nicks } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                nicks.sort_by(|a, b| {
                    fn rank(p: &str) -> u8 {
                        if p.contains('~') { 0 }
                        else if p.contains('&') { 1 }
                        else if p.contains('@') { 2 }
                        else if p.contains('%') { 3 }
                        else if p.contains('+') { 4 }
                        else { 5 }
                    }
                    rank(&a.prefix).cmp(&rank(&b.prefix))
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    buf.nicks = nicks;
                }
            }
            BackendEvent::NickAdded { buffer_id, nick } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    if !buf.nicks.iter().any(|n| n.name == nick.name) {
                        buf.nicks.push(nick);
                    }
                }
            }
            BackendEvent::NickRemoved { buffer_id, nick_name } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    buf.nicks.retain(|n| !n.name.eq_ignore_ascii_case(&nick_name));
                }
            }
            BackendEvent::NickAwayChanged { buffer_id, nick_name, away } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    if let Some(nick) = buf.nicks.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&nick_name)) {
                        nick.away = away;
                    }
                }
            }
            BackendEvent::TopicChanged { buffer_id, topic } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    buf.topic = topic;
                }
            }
            BackendEvent::ActivityChanged { buffer_id, activity, unread_count } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    buf.activity = activity;
                    buf.unread_count = unread_count;
                }
            }
            BackendEvent::LinesLoaded { buffer_id, lines, is_prepend } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                let is_selected = self.selected_buffer_id.as_deref() == Some(full_id.as_str());
                let is_load_more = self.loading_more_buffer_id.as_deref() == Some(full_id.as_str());
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    // Update activity for on-connect chathistory replay, but not for
                    // user-triggered "load more" requests (those are explicitly old history).
                    if !is_selected && !buf.muted && !is_load_more {
                        for line in &lines {
                            if !line.displayed { continue; }
                            if line.highlight {
                                buf.activity = crate::relay::models::BufferActivity::Highlight;
                                buf.unread_count = buf.unread_count.saturating_add(1);
                            } else if buf.activity != crate::relay::models::BufferActivity::Highlight {
                                buf.activity = crate::relay::models::BufferActivity::Message;
                                buf.unread_count = buf.unread_count.saturating_add(1);
                            }
                        }
                    }
                    if is_prepend {
                        for line in lines.into_iter().rev() {
                            buf.messages.push_front(line);
                        }
                        while buf.messages.len() > MAX_STORED_LINES {
                            buf.messages.pop_back();
                        }
                    } else {
                        for line in lines {
                            buf.messages.push_back(line);
                        }
                        while buf.messages.len() > MAX_STORED_LINES {
                            buf.messages.pop_front();
                        }
                    }
                }
                if is_load_more {
                    self.loading_more_buffer_id = None;
                }
            }
            BackendEvent::BufferHidden { buffer_id, hidden } => {
                let full_id = format!("{}/{}", conn_prefix, buffer_id);
                if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                    buf.hidden = hidden;
                }
            }
        }
    }

    pub(crate) fn process_response(&mut self, conn_prefix: &str, resp: WeeChatResponse) {
        if let Some(id) = &resp.request_id {
            if id == "_list_buffers" {
                self.handle_buffer_list(conn_prefix, resp);
                return;
            } else if id == "_hotlist" {
                self.handle_hotlist(conn_prefix, resp);
                return;
            } else if id.starts_with("_buffer_lines:") {
                let buffer_id = id[14..].to_string();
                self.handle_buffer_lines(conn_prefix, &buffer_id, resp);
                return;
            } else if id.starts_with("_nicks:") {
                let buffer_id = id[7..].to_string();
                self.handle_nick_list(conn_prefix, &buffer_id, resp);
                return;
            } else if id.starts_with("_buffer_info:") {
                let buffer_id = id[13..].to_string();
                self.handle_buffer_info(conn_prefix, &buffer_id, resp);
                return;
            }
        }

        if let Some(event) = &resp.event_name {
            match event.as_str() {
                "buffer_line_added" => self.handle_line_added(conn_prefix, resp),
                "buffer_line_data_changed" => self.handle_line_changed(conn_prefix, resp),
                "buffer_hidden" => {
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                            buf.hidden = true;
                        }
                    }
                }
                "buffer_unhidden" => {
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                            buf.hidden = false;
                        }
                    }
                }
                "buffer_title_changed" => {
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        if let Some(conn) = self.connections.iter().find(|c| c.prefix == conn_prefix) {
                            conn.client.refresh_buffer(&raw_id);
                        }
                    }
                }
                "nicklist_nick_added" => {
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        if let Some(nick) = resp.body.as_ref().and_then(|b| b.as_object()).and_then(|o| Self::parse_nick_obj(o)) {
                            if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                                if !buf.nicks.iter().any(|n| n.name == nick.name) {
                                    buf.nicks.push(nick);
                                    buf.nicks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                }
                            }
                        }
                    }
                }
                "nicklist_nick_removing" => {
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        if let Some(name) = resp.body.as_ref().and_then(|b| b.get("name")).and_then(|v| v.as_str()) {
                            if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                                buf.nicks.retain(|n| n.name != name);
                            }
                        }
                    }
                }
                "nicklist_nick_changed" => {
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        if let Some(updated) = resp.body.as_ref().and_then(|b| b.as_object()).and_then(|o| Self::parse_nick_obj(o)) {
                            if let Some(buf) = self.buffer_by_id_mut(&full_id) {
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
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        if let Some(buf) = self.buffer_by_id_mut(&full_id) {
                            buf.messages.clear();
                            buf.last_read_id = None;
                            buf.visit_start_marker_id = None;
                        }
                    }
                }
                "upgrade" => {
                    self.log_conn_for(conn_prefix, "WeeChat is upgrading — waiting for reload…");
                }
                "upgrade_ended" => {
                    if let Some(conn) = self.connections.iter().find(|c| c.prefix == conn_prefix) {
                        conn.client.fetch_buffer_list();
                    }
                    self.log_conn_for(conn_prefix, "WeeChat upgrade complete — re-synced");
                }
                "buffer_opened" | "buffer_closed" | "buffer_renamed"
                | "buffer_localvar_added" | "buffer_localvar_changed"
                | "buffer_localvar_removed" => {
                    if let Some(conn) = self.connections.iter().find(|c| c.prefix == conn_prefix) {
                        conn.client.fetch_buffer_list();
                    }
                }
                "buffer_hotlist_added" | "buffer_hotlist_updated" => {
                    // New unread activity pushed while connected — remove from cleared set
                    // so the real unread state is applied rather than being suppressed.
                    if let Some(raw_id) = resp.buffer_id.map(|i| i.to_string()) {
                        let full_id = format!("{}/{}", conn_prefix, raw_id);
                        self.cleared_buffer_ids.remove(&full_id);
                    }
                    self.handle_hotlist(conn_prefix, resp);
                }
                "buffer_hotlist_removed" => {
                    if let Some(conn) = self.connections.iter().find(|c| c.prefix == conn_prefix) {
                        conn.client.fetch_hotlist();
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
            if let Ok(secs) = s.parse::<i64>() {
                if let Some(dt) = DateTime::from_timestamp(secs, 0) {
                    return dt;
                }
            }
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
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
            *server = "!00_core".to_string();
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
            if a.server != b.server {
                return a.server.cmp(&b.server);
            }

            let a_is_root = a.kind == "core" || a.kind == "server";
            let b_is_root = b.kind == "core" || b.kind == "server";

            if a_is_root && !b_is_root { return std::cmp::Ordering::Less; }
            if b_is_root && !a_is_root { return std::cmp::Ordering::Greater; }

            a.number.cmp(&b.number)
        });
    }

    fn handle_buffer_list(&mut self, conn_prefix: &str, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        let mut new_conn_buffers = Vec::new();

        for val in body {
            if let Some(obj) = val.as_object() {
                let raw_id = obj.get("id").and_then(|v| Self::parse_id(v));

                let number = obj.get("number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let name = obj.get("short_name").and_then(|v| v.as_str())
                    .or_else(|| obj.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("unknown").to_string();
                let raw_full_name = obj.get("name").and_then(|v| v.as_str()).unwrap_or(&name).to_string();
                let plugin = obj.get("plugin").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let hidden = obj.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false);
                let has_nicklist = obj.get("nicklist").and_then(|v| v.as_bool()).unwrap_or(true);
                let relay_last_read_id = obj.get("last_read_line_id").and_then(|v| Self::parse_id(v))
                    .or_else(|| obj.get("last_read_line").and_then(|v| v.as_object()).and_then(|o| o.get("id")).and_then(|v| Self::parse_id(v)));

                if let Some(raw_id) = raw_id {
                    let full_id = format!("{}/{}", conn_prefix, raw_id);
                    let full_full_name = format!("{}/{}", conn_prefix, raw_full_name);

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

                    if let Some(existing) = self.buffer_by_id(&full_id) {
                        messages = existing.messages.clone();
                        nicks = existing.nicks.clone();
                        activity = existing.activity;
                        unread_count = existing.unread_count;
                        last_read_id = existing.last_read_id.clone();
                        visit_start_marker_id = existing.visit_start_marker_id.clone();
                    }
                    let muted = self.muted_buffer_names.contains(&full_full_name);
                    if relay_last_read_id.is_some() {
                        last_read_id = relay_last_read_id.clone();
                    }

                    // Use raw_full_name (without prefix) for metadata extraction
                    Self::extract_metadata(obj, &mut topic, &mut modes, &mut kind, &mut server, &raw_full_name, &plugin);

                    new_conn_buffers.push(Buffer {
                        id: full_id,
                        number,
                        name,
                        full_name: full_full_name,
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

        if !new_conn_buffers.is_empty() {
            let network_count = {
                let mut seen = std::collections::HashSet::new();
                new_conn_buffers.iter().filter(|b| b.kind != "core").for_each(|b| { seen.insert(&b.server); });
                seen.len()
            };
            self.log_conn_for(conn_prefix, format!(
                "← GET /api/buffers  {} buffers, {} network(s)",
                new_conn_buffers.len(), network_count
            ));
            Self::sort_buffers(&mut new_conn_buffers);

            // Remove old buffers for this connection, then add new ones
            let pfx = format!("{}/", conn_prefix);
            self.buffers.retain(|b| !b.id.starts_with(&pfx));
            self.buffers.extend(new_conn_buffers);

            // Re-apply user's custom ordering
            if !self.buffer_order.is_empty() {
                let order = &self.buffer_order;
                let max_order = order.len();

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
                        let base = server_header_pos.get(&b.server)
                            .map(|&p| p * 10_000 + 1)
                            .unwrap_or(max_order * 10_000 + 1);
                        base + b.number as usize
                    }
                });
            }

            // When multiple connections are present, group all buffers by connection prefix
            // so each connection's buffers appear together in the sidebar.
            let conn_count = {
                let mut seen = std::collections::HashSet::new();
                for b in &self.buffers {
                    if let Some(p) = b.id.split('/').next() { seen.insert(p.to_string()); }
                }
                seen.len()
            };
            if conn_count > 1 {
                self.buffers.sort_by(|a, b| {
                    let pa = a.id.split('/').next().unwrap_or("");
                    let pb = b.id.split('/').next().unwrap_or("");
                    pa.cmp(pb)
                });
            }
            self.rebuild_buffer_idx();

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

            self.log_conn_for(conn_prefix, "→ GET /api/hotlist");
            if let Some(conn) = self.connections.iter().find(|c| c.prefix == conn_prefix) {
                conn.client.fetch_hotlist();
            }
        }
    }

    fn handle_hotlist(&mut self, conn_prefix: &str, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        let is_initial = resp.request_id.as_deref() == Some("_hotlist");
        let entry_count = body.len();
        if is_initial {
            self.log_conn_for(conn_prefix, format!("← GET /api/hotlist  {} active entr{}", entry_count, if entry_count == 1 { "y" } else { "ies" }));
        }
        for val in body {
            if let Some(obj) = val.as_object() {
                let raw_buffer_id = obj.get("buffer_id").and_then(|v| Self::parse_id(v));
                let priority = obj.get("priority")
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                    .unwrap_or(0);

                if let Some(raw_buffer_id) = raw_buffer_id {
                    let buffer_id = format!("{}/{}", conn_prefix, raw_buffer_id);
                    if self.selected_buffer_id.as_deref() == Some(&buffer_id) {
                        continue;
                    }
                    if self.buffers.iter().any(|b| b.id == buffer_id && b.muted) {
                        continue;
                    }
                    if self.cleared_buffer_ids.contains(&buffer_id) {
                        continue;
                    }
                    if let Some(buffer) = self.buffer_by_id_mut(&buffer_id) {
                        buffer.activity = match priority {
                            3 => BufferActivity::Highlight,
                            2 | 1 => BufferActivity::Message,
                            _ => BufferActivity::Metadata,
                        };
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

    fn handle_buffer_lines(&mut self, conn_prefix: &str, raw_buffer_id: &str, resp: WeeChatResponse) {
        let full_buffer_id = format!("{}/{}", conn_prefix, raw_buffer_id);
        let body = Self::body_as_vec(&resp);
        let lines: Vec<Line> = body.iter().filter_map(|val| {
            let obj = val.as_object()?;
            let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);
            let id = obj.get("id").and_then(|v| Self::parse_id(v))
                .unwrap_or_else(|| "unknown".to_string());
            let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
            let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let timestamp = Self::parse_date(obj.get("date"));
            let highlight = obj.get("highlight").and_then(|v| v.as_bool()).unwrap_or(false);
            Some(Line::new(id, timestamp, prefix.to_string(), message.to_string(), displayed, highlight))
        }).collect();

        let mut log_entry: Option<String> = None;
        let is_load_more = self.loading_more_buffer_id.as_deref() == Some(&full_buffer_id);
        let is_selected = self.selected_buffer_id.as_deref() == Some(&full_buffer_id);
        if let Some(idx) = self.buffer_idx_of(&full_buffer_id) {
            let buffer = &mut self.buffers[idx];
            let mut deque: std::collections::VecDeque<Line> = lines.into();
            if deque.len() > MAX_STORED_LINES {
                let excess = deque.len() - MAX_STORED_LINES;
                deque.drain(0..excess);
            }
            let line_count = deque.len();
            log_entry = Some(format!(
                "← GET /api/buffers/{}/lines  {} lines{}  [#{}]",
                raw_buffer_id, line_count,
                if is_load_more { " (load more)" } else { "" },
                buffer.name
            ));
            buffer.messages = deque;
            if is_selected {
                if let Some(last) = buffer.messages.back() {
                    buffer.last_read_id = Some(last.id.clone());
                }
            } else if buffer.last_read_id.is_none() {
                if let Some(last) = buffer.messages.back() {
                    buffer.last_read_id = Some(last.id.clone());
                }
            }
        }
        if is_load_more {
            self.loading_more_buffer_id = None;
        }
        if let Some(entry) = log_entry {
            self.log_conn_for(conn_prefix, entry);
        }
    }

    fn handle_buffer_info(&mut self, conn_prefix: &str, raw_buffer_id: &str, resp: WeeChatResponse) {
        let full_buffer_id = format!("{}/{}", conn_prefix, raw_buffer_id);
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                if let Some(buffer) = self.buffer_by_id_mut(&full_buffer_id) {
                    // Strip prefix from full_name for metadata extraction
                    let pfx = format!("{}/", conn_prefix);
                    let raw_full_name = buffer.full_name.strip_prefix(&pfx).unwrap_or(&buffer.full_name).to_string();
                    let plugin = buffer.plugin.clone();
                    Self::extract_metadata(obj, &mut buffer.topic, &mut buffer.modes, &mut buffer.kind, &mut buffer.server, &raw_full_name, &plugin);
                }
            }
        }
    }

    fn handle_nick_list(&mut self, conn_prefix: &str, raw_buffer_id: &str, resp: WeeChatResponse) {
        let full_buffer_id = format!("{}/{}", conn_prefix, raw_buffer_id);
        if let Some(body) = &resp.body {
            let mut nicks = Vec::new();
            self.extract_nicks(body, &mut nicks);
            if let Some(buffer) = self.buffer_by_id_mut(&full_buffer_id) {
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
        Some(Nick { name: name.to_string(), prefix: prefix.to_string(), color_ansi, away: false })
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

    /// Fire a highlight notification for `buffer_id` (full prefixed id) with a
    /// per-buffer 3-second cooldown. Also requests user-attention so the dock
    /// bounces / taskbar flashes. Used by both the WeeChat and IRC backends.
    /// Title format:
    ///   - channel highlight: `<channel> — <sender>`
    ///   - private message:  `<sender>` (no channel prefix)
    pub(crate) fn notify_highlight(
        &mut self,
        buffer_id: &str,
        buffer_name: &str,
        prefix: &str,
        message: &str,
    ) {
        let now = std::time::Instant::now();
        let cooldown = std::time::Duration::from_secs(3);
        let suppress = self
            .last_notif_at
            .get(buffer_id)
            .map(|last| now.duration_since(*last) < cooldown)
            .unwrap_or(false);
        if suppress {
            return;
        }
        self.last_notif_at.insert(buffer_id.to_string(), now);
        self.request_attention = true;

        let sender = Self::strip_ansi(prefix);
        let body = Self::strip_ansi(message);

        // Channel buffers in IRC start with #/&/!, WeeChat passes the same. If
        // the buffer name looks like a channel, prefix the title with it; for
        // PMs (or core/server buffers) just use the sender.
        let is_channel_like = buffer_name.starts_with('#')
            || buffer_name.starts_with('&')
            || buffer_name.starts_with('!');
        let title = if is_channel_like && !sender.is_empty() {
            format!("{} — {}", buffer_name, sender)
        } else if sender.is_empty() {
            buffer_name.to_string()
        } else {
            sender
        };

        crate::ui::notify::show(crate::ui::notify::Notification {
            app_name: "WeeChatRS".to_string(),
            title,
            body,
        });
    }

    pub(crate) fn strip_ansi(text: &str) -> String {
        ansi_re().replace_all(text, "").to_string()
    }

    fn handle_line_added(&mut self, conn_prefix: &str, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);
                let raw_buffer_id = resp.buffer_id.map(|i| i.to_string())
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

                if let Some(raw_buffer_id) = raw_buffer_id {
                    let buffer_id = format!("{}/{}", conn_prefix, raw_buffer_id);
                    let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                    let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let id = obj.get("id").and_then(|v| Self::parse_id(v))
                        .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string());
                    let timestamp = Self::parse_date(obj.get("date"));

                    let line = Line::new(id, timestamp, prefix.to_string(), message.to_string(), displayed, is_highlight);

                    let is_selected = self.selected_buffer_id.as_deref() == Some(&buffer_id);
                    let mut notify_data: Option<(String, String, String)> = None;
                    if let Some(idx) = self.buffer_idx_of(&buffer_id) {
                        let buffer = &mut self.buffers[idx];
                        if !buffer.messages.iter().any(|m| m.id == line.id) {
                            buffer.messages.push_back(line.clone());
                            if buffer.messages.len() > MAX_STORED_LINES {
                                buffer.messages.pop_front();
                            }

                            if is_selected {
                                buffer.last_read_id = Some(line.id.clone());
                            } else if displayed && !buffer.muted && !is_notify_none && !is_self_msg {
                                let activity = if is_highlight || notify_level == 3 {
                                    BufferActivity::Highlight
                                } else if notify_level == 2 {
                                    BufferActivity::Message
                                } else if is_join_part {
                                    BufferActivity::Metadata
                                } else {
                                    BufferActivity::Metadata
                                };

                                if !is_join_part {
                                    buffer.unread_count = buffer.unread_count.saturating_add(1);
                                }

                                if activity > buffer.activity {
                                    buffer.activity = activity;
                                    self.cleared_buffer_ids.remove(&buffer_id);
                                }

                                if (is_highlight || notify_level == 3) && !buffer.muted {
                                    notify_data = Some((
                                        buffer.name.clone(),
                                        prefix.to_string(),
                                        message.to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    if let Some((name, p, m)) = notify_data {
                        self.notify_highlight(&buffer_id, &name, &p, &m);
                    }
                }
            }
        }
    }

    fn handle_line_changed(&mut self, conn_prefix: &str, resp: WeeChatResponse) {
        let body = Self::body_as_vec(&resp);
        for val in body {
            if let Some(obj) = val.as_object() {
                let raw_buffer_id = resp.buffer_id.map(|i| i.to_string())
                    .or_else(|| obj.get("buffer_id").and_then(|v| Self::parse_id(v)));
                let line_id = obj.get("id").and_then(|v| Self::parse_id(v));
                let displayed = obj.get("displayed").and_then(|v| v.as_bool()).unwrap_or(true);

                if let (Some(raw_buffer_id), Some(line_id)) = (raw_buffer_id, line_id) {
                    let buffer_id = format!("{}/{}", conn_prefix, raw_buffer_id);
                    if let Some(buffer) = self.buffer_by_id_mut(&buffer_id) {
                        if let Some(line) = buffer.messages.iter_mut().find(|m| m.id == line_id) {
                            line.displayed = displayed;
                        } else if displayed {
                            let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                            let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
                            let timestamp = Self::parse_date(obj.get("date"));
                            buffer.messages.push_back(Line::new(line_id, timestamp, prefix.to_string(), message.to_string(), displayed, false));
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
