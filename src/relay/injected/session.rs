use super::{
    protocol::{self, Packet},
    Command, InjectedConfig,
};
use crate::relay::irc::connection::{irc_to_ansi, nick_color_ansi};
use crate::relay::{
    backend::BackendEvent,
    models::{Buffer, BufferActivity, Line, Nick},
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

type Tx = mpsc::UnboundedSender<BackendEvent>;
fn emit(tx: &Tx, ctx: &egui::Context, event: BackendEvent) {
    let _ = tx.send(event);
    ctx.request_repaint();
}
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn casefold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            '[' => '{',
            ']' => '}',
            '\\' => '|',
            '^' => '~',
            c => c,
        })
        .collect()
}
fn buffer_id(server: &str, target: &str) -> String {
    json!([server, casefold(target)]).to_string()
}
fn split_buffer(id: &str) -> Option<(String, String)> {
    let pair: [String; 2] = serde_json::from_str(id).ok()?;
    Some((pair[0].clone(), pair[1].clone()))
}
fn timestamp(v: &Value) -> Option<DateTime<Utc>> {
    v.as_i64()
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| {
            v.as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc))
        })
}
/// A join, part, quit, kick or nick change as the IRC backend shows it: a presence
/// line, which "show filtered lines" hides. `v` is a live event or a stored history
/// entry (which also has `type` and a ready-made `text`); `kind` is its type. Other
/// history entries that aren't messages (topic, mode, …) become a `--` notice.
/// None for messages.
fn event_line(v: &Value, kind: &str, channel: &str) -> Option<Line> {
    let ts = timestamp(&v["timestamp"]).unwrap_or_else(Utc::now);
    let nick = string(v, "nick");
    let channel = Some(string(v, "channel")).filter(|c| !c.is_empty()).unwrap_or(channel);
    // Stored entries have the reason only in their text: "bob has quit (reason)".
    let reason = [string(v, "reason"), string(v, "kickReason")]
        .into_iter()
        .find(|r| !r.is_empty())
        .map(str::to_owned)
        .or_else(|| string(v, "text").split_once('(').map(|(_, r)| r.trim_end().trim_end_matches(')').to_owned()))
        .unwrap_or_default();
    let with_reason = |text: String| if reason.is_empty() { text } else { format!("{} ({})", text, irc_to_ansi(&reason)) };
    let (prefix, text, who) = match kind {
        "privmsg" | "action" | "notice" | "" => return None,
        "join" => {
            let (ident, host) = (string(v, "ident"), string(v, "hostname"));
            let from = if ident.is_empty() || host.is_empty() { String::new() } else { format!(" ({}@{})", ident, host) };
            ("-->", format!("{}{} has joined {}", nick, from, channel), nick)
        }
        "part" => ("<--", with_reason(format!("{} has left {}", nick, channel)), nick),
        "quit" => ("<--", with_reason(format!("{} has quit", nick)), nick),
        "kick" => {
            let kicked = Some(string(v, "kicked")).filter(|k| !k.is_empty()).unwrap_or(string(v, "kickedNick"));
            ("<--", with_reason(format!("{} was kicked from {} by {}", kicked, channel, nick)), kicked)
        }
        "nick" => {
            let old = Some(string(v, "oldNick")).filter(|o| !o.is_empty()).unwrap_or(nick);
            ("--", format!("{} is now known as {}", old, string(v, "newNick")), old)
        }
        _ => {
            let text = string(v, "text");
            if text.is_empty() { return None; }
            let id = json!([kind, ts.timestamp_millis(), nick, text]).to_string();
            return Some(Line::new(id, ts, "--".into(), irc_to_ansi(text), true, false));
        }
    };
    // Live events carry no message ID; an ID from the event itself lets the same
    // event from history count as already shown.
    let id = json!([kind, ts.timestamp_millis(), casefold(who)]).to_string();
    Some(Line::new(id, ts, prefix.into(), text, false, false))
}
fn nick(v: &Value) -> Nick {
    Nick {
        name: string(v, "nick").into(),
        prefix: string(v, "prefix").into(),
        color_ansi: nick_color_ansi(string(v, "nick")),
        away: v["away"].as_bool().unwrap_or(false),
    }
}

struct Pending {
    at: tokio::time::Instant,
    request: Request,
}
enum Request {
    History { id: String },
    Echo { id: String, line: Line },
    Sync,
    Action,
}

struct State {
    tx: Tx,
    ctx: egui::Context,
    servers: HashMap<String, Value>,
    nicks: HashMap<String, String>,
    buffers: HashMap<String, (String, String)>,
    members: HashMap<String, Vec<Nick>>,
    seen: HashSet<(String, String)>,
    seen_order: VecDeque<(String, String)>,
    local_echoes: VecDeque<(String, String, String, i64)>,
    latest: HashMap<String, i64>,
    read_lines: HashMap<String, VecDeque<(i64, bool)>>,
    lastseen: serde_json::Map<String, Value>,
    pending: HashMap<u64, Pending>,
    history_inflight: HashSet<String>,
    out: VecDeque<String>,
    sync_ready: bool,
    pending_reads: HashSet<String>,
    next_ack: u64,
    sequence: u64,
}
impl State {
    fn new(tx: Tx, ctx: egui::Context, servers: Vec<Value>) -> Self {
        let mut state = Self {
            tx,
            ctx,
            servers: HashMap::new(),
            nicks: HashMap::new(),
            buffers: HashMap::new(),
            members: HashMap::new(),
            seen: HashSet::new(),
            seen_order: VecDeque::new(),
            local_echoes: VecDeque::new(),
            latest: HashMap::new(),
            read_lines: HashMap::new(),
            lastseen: serde_json::Map::new(),
            pending: HashMap::new(),
            history_inflight: HashSet::new(),
            out: VecDeque::new(),
            sync_ready: false,
            pending_reads: HashSet::new(),
            next_ack: 0,
            sequence: 0,
        };
        for v in servers {
            if let Some(id) = v["id"].as_str().filter(|s| !s.is_empty()) {
                state.nicks.insert(id.into(), string(&v, "nickname").into());
                state.servers.insert(id.into(), v);
            }
        }
        state
    }
    fn emit(&self, e: BackendEvent) {
        // RUST_LOG=weechat_rs::relay::injected=debug shows what reaches the UI, and when.
        if log::log_enabled!(log::Level::Debug) {
            let text = format!("{:?}", e);
            log::debug!("event: {}", text.chars().take(200).collect::<String>());
        }
        emit(&self.tx, &self.ctx, e);
    }
    fn request(&mut self, name: &str, value: Value, request: Request) {
        // Bound memory even if the remote service stops acknowledging commands.
        if self.pending.len() >= 256 {
            self.emit(BackendEvent::Error(
                "Injected has too many pending requests; try again shortly.".into(),
            ));
            return;
        }
        self.next_ack += 1;
        self.pending.insert(
            self.next_ack,
            Pending {
                at: tokio::time::Instant::now(),
                request,
            },
        );
        self.out
            .push_back(protocol::event(name, value, Some(self.next_ack)));
    }
    fn sync(&mut self) {
        self.next_ack += 1;
        self.pending.insert(
            self.next_ack,
            Pending {
                at: tokio::time::Instant::now(),
                request: Request::Sync,
            },
        );
        self.out
            .push_back(protocol::no_arg_event("sync:get", self.next_ack));
    }
    fn open(&mut self, server: &str, target: &str) -> String {
        let id = buffer_id(server, target);
        if self.buffers.contains_key(&id) {
            return id;
        }
        let kind = if target == "@status" {
            "server"
        } else if target.starts_with(['#', '&', '!', '+']) {
            "channel"
        } else {
            "private"
        };
        let label = self
            .servers
            .get(server)
            .map(|v| string(v, "name"))
            .filter(|s| !s.is_empty())
            .unwrap_or(server);
        let name = if target == "@status" { label } else { target };
        let b = Buffer {
            id: id.clone(),
            number: self.buffers.len() as i32 + 1,
            name: name.into(),
            full_name: format!("injected.{}.{}", server, target),
            plugin: "irc".into(),
            kind: kind.into(),
            server: label.into(),
            messages: Arc::new(VecDeque::new()),
            nicks: Arc::new(Vec::new()),
            activity: BufferActivity::None,
            unread_count: 0,
            last_read_id: None,
            last_markread_ts: self
                .lastseen
                .get(&format!("{}:{}", server, target.to_lowercase()))
                .and_then(timestamp),
            topic: String::new(),
            modes: String::new(),
            hidden: false,
            muted: false,
            has_nicklist: kind == "channel",
            visit_start_marker_id: None,
        };
        self.buffers
            .insert(id.clone(), (server.into(), target.into()));
        self.emit(BackendEvent::BufferOpened(b));
        id
    }
    fn ready(&mut self) {
        self.emit(BackendEvent::Connected);
        self.out.push_back(protocol::event(
            "client:state",
            json!({"active":true}),
            None,
        ));
        self.sync();
        let servers: Vec<_> = self.servers.keys().cloned().collect();
        for server in servers {
            self.open(&server, "@status");
        }
        self.emit(BackendEvent::ConnLog("Injected session authenticated. Saved networks loaded; use /connect in a network's server buffer if it is offline.".into()));
    }
    fn history(&mut self, id: &str, count: usize, before: Option<i64>) {
        // The UI calls fetch_lines immediately after fetch_lines_before. Coalesce
        // that pair rather than issuing an overlapping latest-history request.
        if self.history_inflight.contains(id) {
            return;
        }
        let Some((server, target)) = self.buffers.get(id).cloned().or_else(|| split_buffer(id))
        else {
            return;
        };
        let mut payload = json!({"serverId":server,"target":target,"limit":count.clamp(1,1000)});
        if let Some(before) = before {
            payload["before"] = json!(before);
        }
        if self.pending.len() >= 256 {
            return;
        }
        self.history_inflight.insert(id.into());
        self.request("history:get", payload, Request::History { id: id.into() });
    }
    fn line(&mut self, id: &str, v: &Value, history: bool) -> Line {
        if history {
            let channel = self.buffers.get(id).map(|b| b.1.clone()).unwrap_or_default();
            if let Some(line) = event_line(v, string(v, "type"), &channel) {
                return line;
            }
        }
        self.sequence += 1;
        let ts = timestamp(&v["timestamp"]).unwrap_or_else(Utc::now);
        let who = string(v, "nick");
        let text = if history {
            string(v, "text")
        } else {
            string(v, "message")
        };
        let action = v["isAction"].as_bool() == Some(true) || string(v, "type") == "action";
        let text = if action {
            format!(
                "{}{}\x1B[0m {}",
                nick_color_ansi(who),
                who,
                irc_to_ansi(text)
            )
        } else {
            irc_to_ansi(text)
        };
        let prefix = if action {
            "*".to_owned()
        } else {
            format!("{}{}\x1B[0m", nick_color_ansi(who), who)
        };
        let server = self.buffers.get(id).map(|p| p.0.as_str()).unwrap_or("");
        let own = self.nicks.get(server).map(String::as_str).unwrap_or("");
        let highlight = !own.is_empty()
            && casefold(who) != casefold(own)
            && text
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric() && !"_-[]{}\\`^|".contains(c))
                .any(|s| casefold(s) == casefold(own));
        let wire_id = v["tags"]["msgid"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| v["id"].as_str().map(str::to_owned))
            .or_else(|| v["id"].as_i64().map(|n| n.to_string()));
        // Live messages and history do not always share a server-generated ID.
        // A deterministic fallback keeps repeated history pages idempotent.
        let ident =
            wire_id.unwrap_or_else(|| json!([ts.timestamp_millis(), who, text]).to_string());
        Line::new(ident, ts, prefix.into(), text, true, highlight)
    }
    /// The line for a live join, part, quit, kick or nick change in buffer `id`.
    fn presence(&mut self, id: &str, kind: &str, v: &Value) {
        let channel = self.buffers.get(id).map(|b| b.1.clone()).unwrap_or_default();
        if let Some(line) = event_line(v, kind, &channel) {
            if self.remember(id, &line) {
                self.emit(BackendEvent::LineAdded { buffer_id: id.into(), line });
            }
        }
    }
    fn is_member(&self, id: &str, who: &str) -> bool {
        self.members.get(id).is_some_and(|m| m.iter().any(|n| casefold(&n.name) == casefold(who)))
    }
    fn remember(&mut self, id: &str, line: &Line) -> bool {
        let key = (id.into(), line.id.clone());
        if !self.seen.insert(key.clone()) {
            return false;
        }
        self.seen_order.push_back(key);
        while self.seen_order.len() > 20_000 {
            if let Some(key) = self.seen_order.pop_front() {
                self.seen.remove(&key);
            }
        }
        let ts = line.timestamp.timestamp_millis();
        self.latest
            .entry(id.into())
            .and_modify(|v| *v = (*v).max(ts))
            .or_insert(ts);
        if let Some(i) = self.local_echoes.iter().position(|(b, n, t, when)| {
            b == id && n == &line.prefix && t == &line.message && ts.abs_diff(*when) <= 5000
        }) {
            self.local_echoes.remove(i);
            return false;
        }
        // A server echo may beat its send acknowledgement. In that case the
        // received message is authoritative and the later ACK must not echo it again.
        let echo = self
            .pending
            .iter()
            .filter_map(|(ack, p)| match &p.request {
                Request::Echo {
                    id: buffer,
                    line: pending,
                } if buffer == id
                    && pending.prefix == line.prefix
                    && pending.message == line.message
                    && pending.timestamp.timestamp_millis().abs_diff(ts) <= 5000 =>
                {
                    Some(*ack)
                }
                _ => None,
            })
            .min();
        if let Some(ack) = echo {
            self.pending.remove(&ack);
        }
        let records = self.read_lines.entry(id.into()).or_default();
        records.push_back((ts, line.highlight));
        while records.len() > 2000 {
            records.pop_front();
        }
        true
    }
    fn ack(&mut self, ack: u64, value: Value) {
        let Some(pending) = self.pending.remove(&ack) else {
            return;
        };
        let failed = value["ok"].as_bool() == Some(false)
            || value["success"].as_bool() == Some(false)
            || value["error"].as_str().is_some();
        if failed {
            // Do not log arbitrary server text: errors can contain credentials
            // from an IRC command such as IDENTIFY or PASS.
            self.emit(BackendEvent::Error("Injected rejected the request. Check the network connection and account permissions.".into()));
            if let Request::History { id } = pending.request {
                self.history_inflight.remove(&id);
                self.emit(BackendEvent::LinesLoaded {
                    buffer_id: id,
                    lines: vec![],
                    is_prepend: true,
                });
            }
            return;
        }
        match pending.request {
            Request::History { id } => {
                self.history_inflight.remove(&id);
                let mut lines = Vec::new();
                if let Some(messages) = value["messages"].as_array() {
                    for message in messages {
                        let line = self.line(&id, message, true);
                        if self.remember(&id, &line) {
                            lines.push(line);
                        }
                    }
                }
                lines.sort_by_key(|l| l.timestamp);
                self.emit(BackendEvent::LinesLoaded {
                    buffer_id: id,
                    lines,
                    is_prepend: true,
                });
            }
            Request::Echo { id, line } => {
                // The shipped client echoes successful sends locally.
                if self.remember(&id, &line) {
                    self.local_echoes.push_back((
                        id.clone(),
                        line.prefix.clone(),
                        line.message.clone(),
                        line.timestamp.timestamp_millis(),
                    ));
                    while self.local_echoes.len() > 256 {
                        self.local_echoes.pop_front();
                    }
                    self.emit(BackendEvent::LineAdded {
                        buffer_id: id,
                        line,
                    });
                }
            }
            Request::Sync => {
                self.merge_seen(&value["lastseen"]);
                self.sync_ready = true;
                let reads: Vec<_> = self.pending_reads.drain().collect();
                for id in reads {
                    self.command(Command::Read(id));
                }
            }
            Request::Action => {}
        }
    }
    fn merge_seen(&mut self, value: &Value) {
        let Some(map) = value.as_object() else {
            return;
        };
        for (key, value) in map {
            let Some(ts) = value.as_i64() else {
                continue;
            };
            let old = self.lastseen.get(key).and_then(Value::as_i64).unwrap_or(0);
            self.lastseen.insert(key.clone(), json!(ts.max(old)));
        }
        for (id, (server, target)) in &self.buffers {
            if let Some(ts) = self
                .lastseen
                .get(&format!("{}:{}", server, target.to_lowercase()))
                .and_then(timestamp)
            {
                let (unread, highlight) = self
                    .read_lines
                    .get(id)
                    .map(|lines| {
                        lines
                            .iter()
                            .filter(|(when, _)| *when > ts.timestamp_millis())
                            .fold((0u32, false), |(count, hl), (_, is_hl)| {
                                (count + 1, hl || *is_hl)
                            })
                    })
                    .unwrap_or((0, false));
                self.emit(BackendEvent::ActivityChanged {
                    buffer_id: id.clone(),
                    activity: if highlight {
                        BufferActivity::Highlight
                    } else if unread > 0 {
                        BufferActivity::Message
                    } else {
                        BufferActivity::None
                    },
                    unread_count: unread,
                    markread_ts: Some(ts),
                });
            }
        }
    }
    fn timeout_requests(&mut self) {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.at.elapsed() > Duration::from_secs(15))
            .map(|(id, _)| *id)
            .collect();
        for ack in expired {
            if let Some(p) = self.pending.remove(&ack) {
                if let Request::History { id } = p.request {
                    self.history_inflight.remove(&id);
                    self.emit(BackendEvent::LinesLoaded {
                        buffer_id: id,
                        lines: vec![],
                        is_prepend: true,
                    });
                }
                self.emit(BackendEvent::Error(
                    "Injected request timed out; it was not retried automatically.".into(),
                ));
            }
        }
    }
    fn send_chat(&mut self, server: &str, target: &str, text: &str, action: bool) {
        let id = self.open(server, target);
        let who = self.nicks.get(server).cloned().unwrap_or_default();
        let line=self.line(&id,&json!({"nick":who,"message":text,"isAction":action,"timestamp":Utc::now().timestamp_millis()}),false);
        self.request(
            if action {
                "irc:send-action"
            } else {
                "irc:send-message"
            },
            json!({"serverId":server,"target":target,"message":text}),
            Request::Echo { id, line },
        );
    }
    fn send(&mut self, id: &str, text: &str) {
        let Some((server, target)) = self.buffers.get(id).cloned() else {
            return;
        };
        if text.contains(['\r', '\n', '\0']) {
            self.emit(BackendEvent::Error("Send one IRC line at a time.".into()));
            return;
        }
        if !text.starts_with('/') || text.starts_with("//") {
            if target == "@status" {
                self.emit(BackendEvent::Error(
                    "Select a channel or private conversation to send a message.".into(),
                ));
                return;
            }
            self.send_chat(
                &server,
                &target,
                if text.starts_with("//") {
                    &text[1..]
                } else {
                    text
                },
                false,
            );
            return;
        }
        let (command, args) = text[1..].split_once(' ').unwrap_or((&text[1..], ""));
        let args = args.trim();
        let (first, rest) = args.split_once(' ').unwrap_or((args, ""));
        match command.to_ascii_lowercase().as_str() {
            "me" | "action" if !args.is_empty() && target!="@status" => self.send_chat(&server,&target,args,true),
            "msg" | "privmsg" if !first.is_empty() && !rest.is_empty() => self.send_chat(&server,first,rest,false),
            "query" if !first.is_empty() => { self.open(&server,first); if !rest.is_empty() { self.send_chat(&server,first,rest,false); } },
            "join" | "j" if !first.is_empty() => self.request("irc:join-channel",json!({"serverId":server,"channel":first,"key":rest}),Request::Action),
            "part" | "leave" => {
                let (channel,reason)=if first.starts_with(['#','&','!','+']) {(first,rest)} else {(target.as_str(),args)};
                if channel!="@status" { self.request("irc:part-channel",json!({"serverId":server,"channel":channel,"reason":reason}),Request::Action); }
            }
            "close" if !target.starts_with(['#','&','!','+']) && target!="@status" => { self.buffers.remove(id); self.emit(BackendEvent::BufferClosed { buffer_id:id.into() }); }
            "buffer" if args == "hide" || args == "unhide" => {
                self.emit(BackendEvent::BufferHidden { buffer_id: id.into(), hidden: args == "hide" });
            }
            "close" if target.starts_with(['#', '&', '!', '+']) => {
                self.request("irc:part-channel", json!({"serverId":server,"channel":target}),Request::Action);
            }
            "connect" => { if let Some(config)=self.servers.get(&server).cloned() { self.request("irc:connect",config,Request::Action); } }
            "reconnect" => self.request("irc:reconnect",json!(server),Request::Action),
            "disconnect" | "quit" => self.request("irc:disconnect",json!(server),Request::Action),
            "nick" if !args.is_empty() => self.request("irc:change-nick",json!({"serverId":server,"newNick":first}),Request::Action),
            "topic" if !args.is_empty() && target!="@status" => self.request("irc:set-topic",json!({"serverId":server,"channel":target,"topic":args}),Request::Action),
            "whois" if !first.is_empty() => self.request("irc:whois",json!({"serverId":server,"nick":first}),Request::Action),
            "away" => self.request("irc:set-away",json!({"serverId":server,"message":args}),Request::Action),
            "raw" | "quote" if !args.is_empty() => self.request("irc:send-raw",json!({"serverId":server,"raw":args}),Request::Action),
            _ => self.emit(BackendEvent::Error("Unsupported Injected command. Use /join, /part, /query, /msg, /me, /nick, /topic, /whois, /away, /connect, /reconnect, /disconnect, or /raw.".into())),
        }
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Send(id, text) => self.send(&id, &text),
            Command::History(id, count, before) => self.history(&id, count, before),
            Command::Nicks(id) => {
                if let Some((server, target)) = self.buffers.get(&id).cloned() {
                    if target.starts_with(['#', '&', '!', '+']) {
                        self.request(
                            "irc:send-raw",
                            json!({"serverId":server,"raw":format!("NAMES {}",target)}),
                            Request::Action,
                        );
                    }
                }
            }
            Command::Read(id) => {
                if !self.sync_ready {
                    self.pending_reads.insert(id);
                    return;
                }
                if let (Some((server, target)), Some(ts)) =
                    (self.buffers.get(&id), self.latest.get(&id))
                {
                    let key = format!("{}:{}", server, target.to_lowercase());
                    let ts =
                        (*ts).max(self.lastseen.get(&key).and_then(Value::as_i64).unwrap_or(0));
                    self.lastseen.insert(key, json!(ts));
                    self.request(
                        "sync:set",
                        json!({"key":"lastseen","value":self.lastseen}),
                        Request::Action,
                    );
                }
            }
            Command::Disconnect => {}
        }
    }
    fn event(&mut self, name: &str, v: Value) {
        if name == "sync:update" {
            if string(&v, "key") == "lastseen" {
                self.merge_seen(&v["value"]);
            }
            return;
        }
        let server = string(&v, "serverId").to_owned();
        if server.is_empty() {
            return;
        }
        match name {
            "irc:event:connection-state"=> {
                let who=string(&v,"nickname");
                if !who.is_empty() { self.nicks.insert(server.clone(),who.into()); }
                let id=self.open(&server,"@status");
                let status=string(&v,"status");
                let line=Line::new(format!("state-{}",self.sequence),Utc::now(),"--".into(),format!("Network {}{}",status,if v["error"].is_string() {" (server reported an error)"} else {""}),true,false);
                self.sequence+=1;
                self.emit(BackendEvent::LineAdded {buffer_id:id,line});
            }
            "irc:event:channels-sync"=> {
                if let Some(channels)=v["channels"].as_array() {
                    let mut current=HashSet::new();
                    for channel in channels.iter().filter_map(Value::as_str) {
                        let id=self.open(&server,channel); current.insert(id.clone());
                        let lower=channel.to_lowercase();
                        if let Some(users)=v["users"][&lower].as_array() { self.users(&id,users); }
                        if let Some(topic)=v["topics"][&lower]["topic"].as_str() { self.emit(BackendEvent::TopicChanged {buffer_id:id.clone(),topic:irc_to_ansi(topic)}); }
                        self.history(&id,100,None);
                    }
                    let stale:Vec<_>=self.buffers.iter().filter(|(id,(s,t))| s==&server && t.starts_with(['#','&','!','+']) && !current.contains(*id)).map(|(id,_)|id.clone()).collect();
                    for id in stale { self.close(&id); }
                }
            }
            "irc:event:message" | "irc:event:notice"=> {
                let notice=name.ends_with(":notice");
                let target=string(&v,if notice {"target"} else {"channel"});
                let who=string(&v,"nick");
                let own=self.nicks.get(&server).cloned().unwrap_or_default();
                let target=if !own.is_empty() && casefold(target)==casefold(&own) {who} else {target};
                let target=if target.is_empty() {"@status"} else {target};
                let id=self.open(&server,target);
                let mut line=self.line(&id,&v,false);
                if notice { line = Line::new(line.id, line.timestamp, format!("-{}-", line.prefix), line.message, line.displayed, line.highlight); }
                if self.remember(&id,&line) { self.emit(BackendEvent::LineAdded {buffer_id:id,line}); }
            }
            "irc:event:user-list"=> {
                let channel=string(&v,"channel");
                if !channel.is_empty() {
                    let id=self.open(&server,channel);
                    if let Some(users)=v["users"].as_array() { self.users(&id,users); }
                }
            }
            "irc:event:channel-info" | "irc:event:topic"=> {
                let channel=string(&v,"channel");
                if !channel.is_empty() { let id=self.open(&server,channel); if let Some(topic)=v["topic"].as_str() { self.emit(BackendEvent::TopicChanged {buffer_id:id,topic:irc_to_ansi(topic)}); } }
            }
            "irc:event:join"=> {
                let channel=string(&v,"channel");
                if channel.is_empty() { return; }
                let id=self.open(&server,channel);
                let new=nick(&v);
                let members=self.members.entry(id.clone()).or_default();
                if !members.iter().any(|n| casefold(&n.name)==casefold(&new.name)) {members.push(new.clone());}
                self.emit(BackendEvent::NickAdded {buffer_id:id.clone(),nick:new});
                let own=self.nicks.get(&server).cloned().unwrap_or_default();
                if !own.is_empty() && casefold(string(&v,"nick"))==casefold(&own) { self.history(&id,100,None); self.command(Command::Nicks(id)); }
                else { self.presence(&id,"join",&v); }
            }
            "irc:event:part" | "irc:event:kick"=> {
                let kind=if name.ends_with(":kick") {"kick"} else {"part"};
                let who=string(&v,if kind=="kick" {"kicked"} else {"nick"});
                let id=buffer_id(&server,string(&v,"channel"));
                if self.nicks.get(&server).map(|n|casefold(n)==casefold(who)).unwrap_or(false) {self.close(&id);}
                else { self.presence(&id,kind,&v); self.remove_nick(&id,who); }
            }
            "irc:event:quit"=> {
                let who=string(&v,"nick");
                let ids:Vec<_>=self.buffers.iter().filter(|(_, (s,_))| s==&server).map(|(id,_)|id.clone()).collect();
                for id in ids {
                    // In the channels they were in, as the IRC backend does.
                    if self.is_member(&id,who) { self.presence(&id,"quit",&v); }
                    self.remove_nick(&id,who);
                }
            }
            "irc:event:nick"=> {
                let old=string(&v,"oldNick"); let new=string(&v,"newNick");
                if self.nicks.get(&server).map(|n|casefold(n)==casefold(old)).unwrap_or(false) {self.nicks.insert(server.clone(),new.into());}
                let ids:Vec<_>=self.buffers.iter().filter(|(_, (s,_))| s==&server).map(|(id,_)|id.clone()).collect();
                for id in ids {
                    let mut renamed=false;
                    if let Some(users)=self.members.get_mut(&id) {
                        if let Some(user)=users.iter_mut().find(|n|casefold(&n.name)==casefold(old)) {user.name=new.into(); let users=users.clone(); renamed=true; self.emit(BackendEvent::NicklistLoaded {buffer_id:id.clone(),nicks:users});}
                    }
                    if renamed { self.presence(&id,"nick",&v); }
                }
            }
            "irc:event:mode"=> {
                let channel=string(&v,"channel");
                if !channel.is_empty() { self.command(Command::Nicks(buffer_id(&server,channel))); }
            }
            "irc:event:whois" => {
                let id = self.open(&server, "@status");
                let mut fields = vec![format!("{} [{}@{}]",string(&v,"nick"),string(&v,"ident"),string(&v,"hostname"))];
                for key in ["realname", "server", "account", "away"] {
                    if let Some(value) = v[key].as_str().filter(|v| !v.is_empty()) { fields.push(format!("{}: {}",key,value)); }
                }
                if let Some(channels) = v["channels"].as_array() {
                    fields.push(format!("channels: {}",channels.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")));
                }
                self.sequence += 1;
                self.emit(BackendEvent::LineAdded {buffer_id:id,line:Line::new(format!("whois-{}",self.sequence),Utc::now(),"--".into(),fields.join(" | "),true,false)});
            }
            "irc:event:error"=> self.emit(BackendEvent::Error("Injected reported an IRC network error. Check that network in the Injected client.".into())),
            "irc:event:motd"=> {
                let id=self.open(&server,"@status"); self.sequence+=1;
                self.emit(BackendEvent::LineAdded {buffer_id:id,line:Line::new(format!("motd-{}",self.sequence),Utc::now(),"--".into(),string(&v,"motd").into(),true,false)});
            }
            _=>{},
        }
    }
    fn users(&mut self, id: &str, users: &[Value]) {
        let nicks: Vec<_> = users
            .iter()
            .map(nick)
            .filter(|n| !n.name.is_empty())
            .collect();
        self.members.insert(id.into(), nicks.clone());
        self.emit(BackendEvent::NicklistLoaded {
            buffer_id: id.into(),
            nicks,
        });
    }
    fn remove_nick(&mut self, id: &str, who: &str) {
        if let Some(m) = self.members.get_mut(id) {
            m.retain(|n| casefold(&n.name) != casefold(who));
        }
        self.emit(BackendEvent::NickRemoved {
            buffer_id: id.into(),
            nick_name: who.into(),
        });
    }
    fn close(&mut self, id: &str) {
        self.buffers.remove(id);
        self.members.remove(id);
        self.latest.remove(id);
        self.read_lines.remove(id);
        self.history_inflight.remove(id);
        self.pending_reads.remove(id);
        self.seen.retain(|(buffer, _)| buffer != id);
        self.seen_order.retain(|(buffer, _)| buffer != id);
        self.local_echoes.retain(|(buffer, _, _, _)| buffer != id);
        self.pending.retain(|_, p| match &p.request {
            Request::History { id: buffer } | Request::Echo { id: buffer, .. } => buffer != id,
            _ => true,
        });
        self.emit(BackendEvent::BufferClosed {
            buffer_id: id.into(),
        });
    }
}

#[derive(Debug)]
enum Failure {
    Stop,
    Auth,
    Other(&'static str),
}

fn origin(config: &InjectedConfig) -> Result<url::Url, Failure> {
    // Tokens may only travel over TLS, except to a local development server.
    if !config.use_ssl && !matches!(config.host.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err(Failure::Other("Injected requires TLS for a remote server."));
    }
    let mut url = url::Url::parse("https://localhost").unwrap();
    url.set_scheme(if config.use_ssl { "https" } else { "http" })
        .unwrap();
    url.set_host(Some(&config.host))
        .map_err(|_| Failure::Other("Invalid Injected host."))?;
    url.set_port(Some(config.port))
        .map_err(|_| Failure::Other("Invalid Injected port."))?;
    Ok(url)
}
async fn api(
    client: &reqwest::Client,
    base: &url::Url,
    path: &str,
    token: &str,
) -> Result<Value, Failure> {
    let url = base
        .join(path)
        .map_err(|_| Failure::Other("Invalid API URL."))?;
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| Failure::Other("Could not reach the Injected API."))?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(Failure::Auth);
    }
    if !response.status().is_success() {
        return Err(Failure::Other(
            "Injected API returned an unsuccessful response.",
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| Failure::Other("Could not read Injected API response."))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| Failure::Other("Injected API returned invalid JSON."))
}
async fn session(
    config: &InjectedConfig,
    tx: Tx,
    ctx: egui::Context,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    connected: &AtomicBool,
) -> Result<(), Failure> {
    if config.token.trim().is_empty() {
        return Err(Failure::Auth);
    }
    let base = origin(config)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Failure::Other("Could not initialize Injected HTTP client."))?;
    let servers = tokio::select! {
        result=api(&client,&base,"/api/servers",&config.token)=>result?,
        _=rx.recv()=>return Err(Failure::Stop),
    };
    let servers = servers
        .as_array()
        .ok_or(Failure::Other("Injected returned an invalid server list."))?
        .clone();
    let mut url = base.clone();
    url.set_scheme(if config.use_ssl { "wss" } else { "ws" })
        .unwrap();
    url.set_path("/socket.io/");
    url.set_query(Some("EIO=4&transport=websocket"));
    let (mut socket, _) = tokio::select! {
        result=tokio::time::timeout(Duration::from_secs(20),tokio_tungstenite::connect_async(url.as_str()))=>result.map_err(|_|Failure::Other("Injected WebSocket handshake timed out."))?.map_err(|_|Failure::Other("Could not open Injected WebSocket connection."))?,
        _=rx.recv()=>return Err(Failure::Stop),
    };
    let mut state = State::new(tx, ctx, servers);
    let mut heartbeat = Duration::from_secs(45);
    let mut last_packet = tokio::time::Instant::now();
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    let mut authenticated = false;
    let mut engine_open = false;
    let started = tokio::time::Instant::now();
    loop {
        while let Some(packet) = state.out.pop_front() {
            tokio::time::timeout(Duration::from_secs(10), socket.send(Message::Text(packet)))
                .await
                .map_err(|_| Failure::Other("Injected WebSocket write timed out."))?
                .map_err(|_| Failure::Other("Injected WebSocket write failed."))?;
        }
        tokio::select! {
            command=rx.recv()=>match command {
                None | Some(Command::Disconnect)=> { let _=tokio::time::timeout(Duration::from_secs(2),socket.close(None)).await; return Err(Failure::Stop); }
                Some(command) if authenticated=>state.command(command),
                _=>{},
            },
            _=timer.tick()=> {
                if !authenticated && started.elapsed()>Duration::from_secs(20) {return Err(Failure::Other("Injected authentication timed out."));}
                if last_packet.elapsed()>heartbeat {return Err(Failure::Other("Injected heartbeat timed out."));}
                state.timeout_requests();
            }
            message=socket.next()=> {
                match message {
                    Some(Ok(Message::Text(text)))=> {
                        last_packet=tokio::time::Instant::now();
                        match protocol::decode(&text).map_err(|_|Failure::Other("Injected sent an invalid protocol packet."))? {
                            Packet::Open {heartbeat_ms} if !engine_open=> {engine_open=true;heartbeat=Duration::from_millis(heartbeat_ms);state.out.push_back(protocol::authenticate(config.token.trim()));}
                            Packet::Ping(data)=>state.out.push_back(format!("3{}",data)),
                            Packet::Connected if engine_open && !authenticated=> {authenticated=true;connected.store(true,Ordering::Relaxed);state.ready();}
                            Packet::Event(name,value) if authenticated=>state.event(&name,value),
                            Packet::Ack(id,value) if authenticated=>state.ack(id,value),
                            Packet::Rejected=>return Err(Failure::Auth),
                            Packet::Closed=>return Err(Failure::Other("Injected closed the session.")),
                            _=>{},
                        }
                    }
                    Some(Ok(Message::Ping(bytes)))=> {socket.send(Message::Pong(bytes)).await.map_err(|_|Failure::Other("Injected WebSocket heartbeat failed."))?;}
                    Some(Ok(Message::Close(_))) | None=>return Err(Failure::Other("Injected connection closed.")),
                    Some(Err(_))=>return Err(Failure::Other("Injected WebSocket connection failed.")),
                    _=>{},
                }
            }
        }
    }
}

pub(super) fn spawn(
    config: Arc<InjectedConfig>,
    tx: Tx,
    ctx: egui::Context,
    mut rx: mpsc::UnboundedReceiver<Command>,
    connected: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut delay = 1;
        loop {
            let result = session(&config, tx.clone(), ctx.clone(), &mut rx, &connected).await;
            let was_connected = connected.swap(false, Ordering::Relaxed);
            if was_connected {
                emit(&tx, &ctx, BackendEvent::Disconnected);
                delay = 1;
            }
            match result {
                Err(Failure::Stop) => break,
                Err(Failure::Auth) => {
                    emit(&tx,&ctx,BackendEvent::AuthError("Injected session token was rejected or is missing. Sign in to Injected and replace the token in this connection profile.".into()));
                    break;
                }
                Err(Failure::Other(message)) => {
                    emit(&tx, &ctx, BackendEvent::Error(message.into()))
                }
                Ok(()) => {}
            }
            if !config.auto_reconnect {
                break;
            }
            emit(
                &tx,
                &ctx,
                BackendEvent::ConnLog(format!("Reconnecting to Injected in {} seconds…", delay)),
            );
            let sleep = tokio::time::sleep(Duration::from_secs(delay));
            tokio::pin!(sleep);
            loop {
                tokio::select! {_=&mut sleep=>break,command=rx.recv()=>match command {None | Some(Command::Disconnect)=>return,_=>{}}}
            }
            delay = (delay * 2).min(60);
        }
        connected.store(false, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> (State, mpsc::UnboundedReceiver<BackendEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            State::new(
                tx,
                egui::Context::default(),
                vec![json!({"id":"net","name":"Libera","nickname":"alice"})],
            ),
            rx,
        )
    }
    fn drain(rx: &mut mpsc::UnboundedReceiver<BackendEvent>) -> Vec<BackendEvent> {
        let mut out = vec![];
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }
    #[test]
    fn messages_history_and_nicklist_use_the_active_theme_palette() {
        use crate::ui::{
            ansi::{ANSIParser, AnsiColor},
            theme::{AppTheme, ThemeColor},
        };
        let (mut s, _rx) = state();
        let id = s.open("net", "#rust");
        let value = json!({"nick":"bob","message":"\u{0002}bold\u{0002} \u{0003}04red","timestamp":1700000000000i64});
        let line = s.line(&id, &value, false);
        assert_eq!(line.plain_prefix, "bob");
        assert_eq!(line.plain_message, "bold red");
        assert!(line.parsed_message.iter().any(|part| part.style.bold));
        let style = line
            .parsed_prefix
            .iter()
            .find(|part| part.text == "bob")
            .unwrap()
            .style;
        let AnsiColor::Indexed(slot) = style.fg else {
            panic!("Nick should use a theme palette slot");
        };
        let mut first = AppTheme::default();
        first.ansi[slot as usize] = ThemeColor {
            r: 0.1,
            g: 0.2,
            b: 0.3,
        };
        let mut second = first.clone();
        second.ansi[slot as usize] = ThemeColor {
            r: 0.9,
            g: 0.8,
            b: 0.7,
        };
        assert_ne!(
            style.to_format(egui::FontId::default(), &first).color,
            style.to_format(egui::FontId::default(), &second).color
        );
        let member = nick(&json!({"nick":"bob","prefix":"@"}));
        let member_style = ANSIParser::parse(&format!("{}bob", member.color_ansi))[0].style;
        assert_eq!(member_style.fg, style.fg);
        let history = s.line(
            &id,
            &json!({"nick":"bob","text":"hi","timestamp":1700000000000i64}),
            true,
        );
        assert_eq!(history.parsed_prefix[0].style.fg, style.fg);
    }

    #[test]
    fn restores_channels_topics_members_and_deduplicates_history() {
        let (mut s, mut rx) = state();
        s.ready();
        drain(&mut rx);
        s.event("irc:event:channels-sync",json!({"serverId":"net","channels":["#Rust"],"topics":{"#rust":{"topic":"Welcome"}},"users":{"#rust":[{"nick":"bob","prefix":"@"}]}}));
        let events = drain(&mut rx);
        assert!(events.iter().any(
            |e| matches!(e,BackendEvent::BufferOpened(b) if b.name=="#Rust" && b.server=="Libera")
        ));
        assert!(events
            .iter()
            .any(|e| matches!(e,BackendEvent::TopicChanged{topic,..} if topic=="Welcome")));
        assert!(events.iter().any(|e|matches!(e,BackendEvent::NicklistLoaded{nicks,..} if nicks[0].name=="bob" && nicks[0].prefix=="@")));
        let id = buffer_id("net", "#rust");
        let ack = s.next_ack;
        let payload = json!({"messages":[{"id":"m1","nick":"bob","text":"hi alice","timestamp":1700000000000i64}]});
        s.ack(ack, payload.clone());
        let events = drain(&mut rx);
        assert!(events.iter().any(|e|matches!(e,BackendEvent::LinesLoaded{lines,..} if lines.len()==1 && lines[0].highlight)));
        s.history(&id, 100, None);
        s.ack(s.next_ack, payload);
        assert!(drain(&mut rx)
            .iter()
            .any(|e| matches!(e,BackendEvent::LinesLoaded{lines,..} if lines.is_empty())));
        s.event("irc:event:message",json!({"serverId":"net","id":"m1","channel":"#RUST","nick":"bob","message":"hi alice","timestamp":1700000000000i64}));
        assert!(!drain(&mut rx)
            .iter()
            .any(|e| matches!(e, BackendEvent::LineAdded { .. })));
    }
    #[test]
    fn dms_are_routed_by_our_nick_and_membership_updates() {
        let (mut s, mut rx) = state();
        s.event(
            "irc:event:connection-state",
            json!({"serverId":"net","status":"connected","nickname":"alice_"}),
        );
        s.event("irc:event:message",json!({"serverId":"net","channel":"ALICE_","nick":"bob","message":"private","timestamp":1700000000000i64}));
        assert!(drain(&mut rx).iter().any(
            |e| matches!(e,BackendEvent::BufferOpened(b) if b.kind=="private" && b.name=="bob")
        ));
        s.event(
            "irc:event:user-list",
            json!({"serverId":"net","channel":"#rust","users":[{"nick":"bob"},{"nick":"alice_"}]}),
        );
        s.event(
            "irc:event:nick",
            json!({"serverId":"net","oldNick":"bob","newNick":"bob_"}),
        );
        assert_eq!(s.members[&buffer_id("net", "#rust")][0].name, "bob_");
        s.event("irc:event:quit", json!({"serverId":"net","nick":"bob_"}));
        assert_eq!(s.members[&buffer_id("net", "#rust")].len(), 1);
        s.event(
            "irc:event:kick",
            json!({"serverId":"net","channel":"#rust","kicked":"alice_"}),
        );
        assert!(!s.buffers.contains_key(&buffer_id("net", "#rust")));
    }
    #[test]
    fn joins_parts_quits_are_filtered_lines_live_and_from_history() {
        let (mut s, mut rx) = state();
        s.event("irc:event:connection-state", json!({"serverId":"net","status":"connected","nickname":"alice"}));
        s.event("irc:event:user-list", json!({"serverId":"net","channel":"#rust","users":[{"nick":"alice"},{"nick":"bob"}]}));
        s.event("irc:event:user-list", json!({"serverId":"net","channel":"#go","users":[{"nick":"alice"}]}));
        drain(&mut rx);
        // Live events, with the fields the official web client reads.
        let ts = 1700000000000i64;
        for (name, v) in [
            ("join", json!({"channel":"#rust","nick":"carol","ident":"c","hostname":"host","timestamp":ts})),
            ("part", json!({"channel":"#rust","nick":"carol","reason":"bye","timestamp":ts + 1})),
            ("nick", json!({"oldNick":"bob","newNick":"bobby","timestamp":ts + 2})),
            ("kick", json!({"channel":"#rust","nick":"alice","kicked":"bobby","reason":"spam","timestamp":ts + 3})),
            ("join", json!({"channel":"#rust","nick":"dave","timestamp":ts + 4})),
            ("quit", json!({"nick":"dave","reason":"Ping timeout","timestamp":ts + 5})),
        ] {
            let mut v = v;
            v["serverId"] = json!("net");
            s.event(&format!("irc:event:{name}"), v);
        }
        let rust = buffer_id("net", "#rust");
        let lines: Vec<(String, String, String, bool)> = drain(&mut rx).into_iter().filter_map(|e| match e {
            BackendEvent::LineAdded { buffer_id, line } => Some((buffer_id, line.plain_prefix, line.plain_message, line.displayed)),
            _ => None,
        }).collect();
        let presence = |prefix: &str, text: &str| (rust.clone(), prefix.to_string(), text.to_string(), false);
        // Presence lines (displayed = false), so "show filtered lines" hides them; the
        // quit only in #rust, where dave was.
        assert_eq!(lines, vec![
            presence("-->", "carol (c@host) has joined #rust"),
            presence("<--", "carol has left #rust (bye)"),
            presence("--", "bob is now known as bobby"),
            presence("<--", "bobby was kicked from #rust by alice (spam)"),
            presence("-->", "dave has joined #rust"),
            presence("<--", "dave has quit (Ping timeout)"),
        ]);

        // History as Injected stores it: a type and a ready-made text. An event already
        // seen live isn't repeated; the rest become the same lines, not messages.
        s.history(&rust, 100, None);
        s.ack(s.next_ack, json!({"messages":[
            {"id":"h1","type":"part","nick":"carol","text":"carol has left #rust (bye)","timestamp":ts + 1},
            {"id":"h2","type":"quit","nick":"erin","ident":"e","hostname":"h","text":"erin has quit (Quit: later)","timestamp":ts - 5},
            {"id":"h3","type":"topic","nick":"alice","text":"alice changed the topic to: hi","timestamp":ts - 4},
            {"id":"h4","type":"privmsg","nick":"bob","text":"hello","timestamp":ts - 3},
        ]}));
        let loaded: Vec<(String, String, bool)> = drain(&mut rx).into_iter().find_map(|e| match e {
            BackendEvent::LinesLoaded { lines, .. } => Some(lines.into_iter().map(|l| (l.plain_prefix, l.plain_message, l.displayed)).collect()),
            _ => None,
        }).unwrap();
        assert_eq!(loaded, vec![
            ("<--".to_string(), "erin has quit (Quit: later)".to_string(), false),
            ("--".to_string(), "alice changed the topic to: hi".to_string(), true),
            ("bob".to_string(), "hello".to_string(), true),
        ]);
    }
    #[test]
    fn pagination_is_coalesced_and_read_sync_is_monotonic() {
        let (mut s, mut rx) = state();
        let id = s.open("net", "#rust");
        s.history(&id, 100, Some(1700000000000));
        let ack = s.next_ack;
        s.history(&id, 500, None);
        assert_eq!(s.next_ack, ack);
        assert!(s.out.back().unwrap().contains("\"before\":1700000000000"));
        s.ack(ack, json!({"messages":[]}));
        s.merge_seen(&json!({"net:#rust":1700000000000i64,"other:#chan":1700000000500i64}));
        s.merge_seen(&json!({"net:#rust":1}));
        assert_eq!(s.lastseen["net:#rust"], json!(1700000000000i64));
        s.sync_ready = true;
        s.latest.insert(id.clone(), 1700000001000i64);
        s.command(Command::Read(id));
        let packet = s.out.back().unwrap();
        assert!(packet.contains("other:#chan"));
        assert!(packet.contains("1700000001000"));
        drain(&mut rx);
    }
    #[test]
    fn failed_send_is_not_echoed_and_error_text_is_redacted() {
        let (mut s, mut rx) = state();
        let id = s.open("net", "#rust");
        drain(&mut rx);
        s.send(&id, "hello");
        let ack = s.next_ack;
        assert!(!drain(&mut rx)
            .iter()
            .any(|e| matches!(e, BackendEvent::LineAdded { .. })));
        s.ack(ack, json!({"error":"secret-token"}));
        let events = drain(&mut rx);
        assert!(!events
            .iter()
            .any(|e| matches!(e, BackendEvent::LineAdded { .. })));
        assert!(!format!("{:?}", events).contains("secret-token"));
        s.send(&id, "hello again");
        s.ack(s.next_ack, json!({"ok":true}));
        assert!(drain(&mut rx).iter().any(|e|matches!(e,BackendEvent::LineAdded{line,..} if line.plain_prefix=="alice" && line.message=="hello again")));
        s.send(&id, "bad\r\nPRIVMSG #else :oops");
        assert!(drain(&mut rx)
            .iter()
            .any(|e| matches!(e, BackendEvent::Error(_))));
    }
    #[test]
    fn network_disconnect_is_explicit_and_buffer_ids_do_not_collide() {
        assert_ne!(buffer_id("a:b", "c"), buffer_id("a", "b:c"));
        let (mut s, _rx) = state();
        let id = s.open("net", "@status");
        s.send(&id, "/connect");
        assert!(s.out.back().unwrap().contains("irc:connect"));
        s.send(&id, "/disconnect");
        assert!(s
            .out
            .back()
            .unwrap()
            .contains(r#"["irc:disconnect","net"]"#));
        let config = InjectedConfig {
            host: "example.com".into(),
            port: 80,
            token: "secret".into(),
            use_ssl: false,
            auto_reconnect: false,
        };
        assert!(origin(&config).is_err());
    }

    #[test]
    fn closing_then_rejoining_restores_history_and_echo_races_do_not_duplicate() {
        let (mut s, mut rx) = state();
        let id = s.open("net", "#rust");
        let message =
            json!({"id":"m1","nick":"bob","message":"hello","timestamp":1700000000000i64});
        let line = s.line(&id, &message, false);
        assert!(s.remember(&id, &line));
        s.close(&id);
        s.open("net", "#rust");
        assert!(s.remember(&id, &line));
        drain(&mut rx);
        s.send(&id, "racing echo");
        let ack = s.next_ack;
        let ts = Utc::now().timestamp_millis();
        s.event("irc:event:message",json!({"serverId":"net","channel":"#rust","id":"echo1","nick":"alice","message":"racing echo","timestamp":ts}));
        s.ack(ack, json!({"ok":true}));
        let lines = drain(&mut rx)
            .into_iter()
            .filter(|e| matches!(e, BackendEvent::LineAdded { .. }))
            .count();
        assert_eq!(lines, 1);
    }
    #[test]
    fn read_updates_wait_for_initial_sync_and_preserve_highlights() {
        let (mut s, mut rx) = state();
        let id = s.open("net", "#rust");
        s.sync();
        let ack = s.next_ack;
        s.event("irc:event:message",json!({"serverId":"net","channel":"#rust","nick":"bob","message":"hi alice","timestamp":1700000001000i64}));
        s.command(Command::Read(id.clone()));
        assert!(s.pending_reads.contains(&id));
        assert!(!s.out.iter().any(|p| p.contains("sync:set")));
        s.ack(
            ack,
            json!({"lastseen":{"net:#rust":1700000000000i64,"other:#chan":1000}}),
        );
        assert!(s.out.back().unwrap().contains("other:#chan"));
        let activity = drain(&mut rx)
            .into_iter()
            .find(|e| matches!(e, BackendEvent::ActivityChanged { .. }))
            .unwrap();
        assert!(matches!(
            activity,
            BackendEvent::ActivityChanged {
                activity: BufferActivity::Highlight,
                unread_count: 1,
                ..
            }
        ));
    }

    async fn receive_event(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        wanted: &str,
    ) -> (u64, Value) {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Message::Text(text) = message {
                if let Ok(Packet::Event(name, value)) = protocol::decode(&text) {
                    if name == wanted {
                        let body = &text[2..];
                        let begin = body.find('[').unwrap();
                        return (body[..begin].parse().unwrap_or(0), value);
                    }
                }
            }
        }
    }
    async fn mock_api(listener: &tokio::net::TcpListener) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![];
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /api/servers HTTP/1.1"));
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer test-token"));
        let body = r#"[{"id":"net","name":"Test","nickname":"alice"}]"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
    }
    #[tokio::test]
    async fn mock_service_authenticates_pongs_loads_history_sends_and_closes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            mock_api(&listener).await;
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(req.uri().path(), "/socket.io/");
                    assert_eq!(req.uri().query(), Some("EIO=4&transport=websocket"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            socket
                .send(Message::Text(
                    r#"0{"sid":"mock","pingInterval":25000,"pingTimeout":20000}"#.into(),
                ))
                .await
                .unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::Text(r#"40{"token":"test-token"}"#.into())
            );
            socket
                .send(Message::Text(r#"40{"sid":"namespace"}"#.into()))
                .await
                .unwrap();
            let (ack, _) = receive_event(&mut socket, "sync:get").await;
            socket
                .send(Message::Text(format!("43{}[{{\"lastseen\":{{}}}}]", ack)))
                .await
                .unwrap();
            socket.send(Message::Text("2".into())).await.unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::Text("3".into())
            );
            socket
                .send(Message::Text(
                    r##"42["irc:event:channels-sync",{"serverId":"net","channels":["#rust"]}]"##
                        .into(),
                ))
                .await
                .unwrap();
            let (ack, payload) = receive_event(&mut socket, "history:get").await;
            assert_eq!(payload["target"], "#rust");
            socket.send(Message::Text(format!("43{}[{{\"messages\":[{{\"id\":\"m1\",\"nick\":\"bob\",\"text\":\"hello\",\"timestamp\":1700000000000}}]}}]",ack))).await.unwrap();
            let (ack, payload) = receive_event(&mut socket, "irc:send-message").await;
            assert_eq!(payload["message"], "world");
            socket
                .send(Message::Text(format!("43{}[{{\"ok\":true}}]", ack)))
                .await
                .unwrap();
            // Closing the UI connection must not disconnect the cloud IRC network.
            assert!(matches!(
                socket.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
        });
        let config = InjectedConfig {
            host: "127.0.0.1".into(),
            port,
            token: "test-token".into(),
            use_ssl: false,
            auto_reconnect: false,
        };
        let (tx, mut events) = mpsc::unbounded_channel();
        let (commands, mut rx) = mpsc::unbounded_channel();
        let connected = Arc::new(AtomicBool::new(false));
        let c = connected.clone();
        let task = tokio::spawn(async move {
            session(&config, tx, egui::Context::default(), &mut rx, &c).await
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut saw_connected = false;
            loop {
                match events.recv().await.unwrap() {
                    BackendEvent::Connected => saw_connected = true,
                    BackendEvent::LinesLoaded { lines, .. } if !lines.is_empty() => {
                        assert!(saw_connected);
                        assert_eq!(lines[0].message, "hello");
                        break;
                    }
                    _ => {}
                }
            }
            commands
                .send(Command::Send(buffer_id("net", "#rust"), "world".into()))
                .unwrap();
            loop {
                if let BackendEvent::LineAdded { line, .. } = events.recv().await.unwrap() {
                    assert_eq!(line.message, "world");
                    break;
                }
            }
            commands.send(Command::Disconnect).unwrap();
            assert!(matches!(task.await.unwrap(), Err(Failure::Stop)));
            server.await.unwrap();
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn rejected_token_stops_without_reconnect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            mock_api(&listener).await;
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    r#"0{"pingInterval":25000,"pingTimeout":20000}"#.into(),
                ))
                .await
                .unwrap();
            socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(
                    r#"44{"message":"Authentication error"}"#.into(),
                ))
                .await
                .unwrap();
        });
        let config = Arc::new(InjectedConfig {
            host: "127.0.0.1".into(),
            port,
            token: "test-token".into(),
            use_ssl: false,
            auto_reconnect: true,
        });
        let (tx, mut events) = mpsc::unbounded_channel();
        let (_commands, rx) = mpsc::unbounded_channel();
        let connected = Arc::new(AtomicBool::new(false));
        spawn(config, tx, egui::Context::default(), rx, connected.clone());
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, BackendEvent::AuthError(_)));
        assert!(!connected.load(Ordering::Relaxed));
        assert!(tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .is_none());
        server.await.unwrap();
    }
}
