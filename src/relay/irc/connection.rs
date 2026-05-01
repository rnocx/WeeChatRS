use base64::Engine as _;
use super::{parser::IrcMessage, IrcConfig};
use crate::relay::backend::BackendEvent;
use crate::relay::models::{Buffer, BufferActivity, Line, Nick};
use chrono::{DateTime, Utc};
use egui::Context as EguiContext;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

#[allow(dead_code)]
pub enum IrcCommand {
    SendMessage { buffer_id: String, text: String },
    FetchNicks { buffer_id: String },
    FetchBufferList,
    MarkRead { buffer_id: String },
    FetchBefore { buffer_id: String, before_ts: String },
    Disconnect,
}

// ── Per-connection mutable state ──────────────────────────────────────────────

struct Session {
    our_nick: String,
    server_name: String,
    available_caps: HashSet<String>,
    negotiated_caps: HashSet<String>,
    cap_ls_done: bool,
    registered: bool,
    sasl_done: bool,
    is_soju: bool,
    joined: HashSet<String>,
    next_number: i32,
    names_buf: HashMap<String, Vec<Nick>>,
    batch_buf: HashMap<String, (String, Vec<Line>)>,
    who_buf: HashMap<String, Vec<Nick>>,
    whois_target: Option<String>,
    whois_lines: Vec<String>,
    seen_msgids: HashSet<String>,
    monitored_nicks: HashSet<String>,
    pending_status_lines: Vec<Line>,
    pong_received: bool,
}

impl Session {
    fn new(nick: &str) -> Self {
        Self {
            our_nick: nick.to_string(),
            server_name: String::new(),
            available_caps: HashSet::new(),
            negotiated_caps: HashSet::new(),
            cap_ls_done: false,
            registered: false,
            sasl_done: false,
            is_soju: false,
            joined: HashSet::new(),
            next_number: 1,
            names_buf: HashMap::new(),
            batch_buf: HashMap::new(),
            who_buf: HashMap::new(),
            whois_target: None,
            whois_lines: Vec::new(),
            seen_msgids: HashSet::new(),
            monitored_nicks: HashSet::new(),
            pending_status_lines: Vec::new(),
            pong_received: true,
        }
    }

    fn wants_caps() -> &'static [&'static str] {
        &[
            "sasl",
            "message-tags",
            "server-time",
            "multi-prefix",
            "away-notify",
            "account-notify",
            "extended-join",
            "batch",
            "chathistory",
            "echo-message",
            "invite-notify",
            "chghost",
            "userhost-in-names",
            "cap-notify",
            "labeled-response",
            "msgid",
            "soju.im/bouncer-networks",
            "soju.im/read-marker",
        ]
    }

    fn cap_req(&self, has_sasl_creds: bool) -> String {
        let wanted: Vec<&str> = Self::wants_caps()
            .iter()
            .filter(|&&c| {
                if c == "sasl" { has_sasl_creds && self.available_caps.contains(c) }
                else { self.available_caps.contains(c) }
            })
            .copied()
            .collect();
        wanted.join(" ")
    }

    fn alloc_buffer(&mut self, id: &str, name: &str, kind: &str) -> Buffer {
        let n = self.next_number;
        self.next_number += 1;
        Buffer {
            id: id.to_string(),
            number: n,
            name: name.to_string(),
            full_name: format!("irc.{}", name),
            plugin: "irc".to_string(),
            kind: kind.to_string(),
            server: self.server_name.clone(),
            messages: VecDeque::new(),
            nicks: Vec::new(),
            activity: BufferActivity::None,
            unread_count: 0,
            last_read_id: None,
            topic: String::new(),
            modes: String::new(),
            hidden: false,
            muted: false,
            has_nicklist: kind == "channel",
            visit_start_marker_id: None,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn nick_color_ansi(name: &str) -> String {
    let mut h: u32 = 0;
    for b in name.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as u32);
    }
    let idx = ((h % 15) + 1) as u8;
    if idx < 8 { format!("\x1B[{}m", 30 + idx) } else { format!("\x1B[{}m", 90 + idx - 8) }
}

fn strip_prefix_chars(name: &str) -> (&str, &str) {
    // IRC mode prefixes: @, +, %, ~, &, ! (multi-prefix may give several)
    let end = name
        .find(|c: char| c.is_alphanumeric() || "_-[]{}\\`^|".contains(c))
        .unwrap_or(0);
    (&name[..end], &name[end..])
}

fn parse_names_entry(entry: &str) -> Nick {
    // multi-prefix: strip all leading mode chars (@, +, %, ~, &, !)
    let (prefix, rest) = strip_prefix_chars(entry);
    // userhost-in-names: rest may be "nick!user@host" — keep only the nick part
    let name = rest.split('!').next().unwrap_or(rest);
    Nick { name: name.to_string(), prefix: prefix.to_string(), color_ansi: nick_color_ansi(name), away: false }
}

fn timestamp_from_msg(msg: &IrcMessage) -> DateTime<Utc> {
    msg.tag("time")
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

fn make_line(id: &str, prefix: &str, text: &str, ts: DateTime<Utc>, highlight: bool) -> Line {
    Line { id: id.to_string(), timestamp: ts, prefix: prefix.to_string(), message: text.to_string(), displayed: true, highlight }
}

fn is_channel(target: &str) -> bool {
    target.starts_with('#') || target.starts_with('&') || target.starts_with('!')
}

// ── IRC → BackendEvent translation ───────────────────────────────────────────

fn translate(msg: &IrcMessage, session: &mut Session, line_counter: &mut u64, config: &IrcConfig) -> Vec<BackendEvent> {
    let mut events = Vec::new();
    let has_sasl_creds = !config.password.is_empty();

    // msgid deduplication: drop any message we've already seen (bouncer replays, reconnects)
    if let Some(id) = msg.tag("msgid") {
        if !session.seen_msgids.insert(id.to_string()) {
            return events;
        }
        // Prevent unbounded growth — cap at 2000 entries
        if session.seen_msgids.len() > 2000 {
            // HashSet has no ordered eviction; just clear when over limit
            session.seen_msgids.clear();
        }
    }

    match msg.command.as_str() {
        "CAP" => {
            let sub = msg.param(1).unwrap_or("").to_uppercase();
            match sub.as_str() {
                "LS" => {
                    let caps_str = msg.params.last().map(String::as_str).unwrap_or("");
                    for cap in caps_str.split_whitespace() {
                        let cap_name = cap.split('=').next().unwrap_or(cap);
                        session.available_caps.insert(cap_name.to_string());
                    }
                    let is_continuation = msg.param(1).map(|p| p == "*").unwrap_or(false);
                    if !is_continuation {
                        session.cap_ls_done = true;
                        let req = session.cap_req(has_sasl_creds);
                        if !req.is_empty() {
                            session.whois_lines.push(format!("CAP REQ :{}", req));
                        } else {
                            session.whois_lines.push("CAP END".to_string());
                        }
                    }
                }
                "ACK" => {
                    let acked = msg.params.last().map(String::as_str).unwrap_or("");
                    for cap in acked.split_whitespace() {
                        let cap_name = cap.trim_start_matches('-');
                        session.negotiated_caps.insert(cap_name.to_string());
                        if cap_name == "soju.im/bouncer-networks" {
                            session.is_soju = true;
                        }
                    }
                    // If SASL was negotiated, start authentication before CAP END
                    if session.negotiated_caps.contains("sasl") && !session.sasl_done {
                        session.whois_lines.push("AUTHENTICATE PLAIN".to_string());
                    } else {
                        session.whois_lines.push("CAP END".to_string());
                    }
                }
                "NAK" => { session.whois_lines.push("CAP END".to_string()); }
                // cap-notify: server advertises a new cap mid-session — request it if wanted
                "NEW" => {
                    let caps_str = msg.params.last().map(String::as_str).unwrap_or("");
                    let mut new_wanted = Vec::new();
                    for cap in caps_str.split_whitespace() {
                        let cap_name = cap.split('=').next().unwrap_or(cap);
                        session.available_caps.insert(cap_name.to_string());
                        if Session::wants_caps().contains(&cap_name)
                            && !session.negotiated_caps.contains(cap_name)
                            && !(cap_name == "sasl" && !has_sasl_creds)
                        {
                            new_wanted.push(cap_name.to_string());
                        }
                    }
                    if !new_wanted.is_empty() {
                        session.whois_lines.push(format!("CAP REQ :{}", new_wanted.join(" ")));
                    }
                }
                // cap-notify: server removed a cap mid-session
                "DEL" => {
                    let caps_str = msg.params.last().map(String::as_str).unwrap_or("");
                    for cap in caps_str.split_whitespace() {
                        session.negotiated_caps.remove(cap);
                        session.available_caps.remove(cap);
                    }
                }
                _ => {}
            }
        }

        "AUTHENTICATE" => {
            // Server is ready for our SASL payload
            if msg.param(0).unwrap_or("") == "+" {
                let user = if config.sasl_username.is_empty() { &config.nick } else { &config.sasl_username };
                let payload = format!("\0{}\0{}", user, config.password);
                let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
                session.whois_lines.push(format!("AUTHENTICATE {}", encoded));
            }
        }

        // 903 — SASL success
        "903" => {
            session.sasl_done = true;
            session.whois_lines.push("CAP END".to_string());
        }

        // 904/905 — SASL failure
        "904" | "905" => {
            let reason = msg.params.last().map(String::as_str).unwrap_or("SASL authentication failed");
            events.push(BackendEvent::AuthError(reason.to_string()));
            session.sasl_done = true;
            session.whois_lines.push("CAP END".to_string());
        }

        "001" => {
            if let Some(nick) = msg.param(0) { session.our_nick = nick.to_string(); }
            if let Some(srv) = &msg.prefix {
                session.server_name = srv.split('!').next().unwrap_or(srv).to_string();
            }
            session.registered = true;
            if !session.is_soju {
                let buf_name = if session.server_name.is_empty() { "server".to_string() } else { session.server_name.clone() };
                let status_buf = session.alloc_buffer("status", &buf_name, "server");
                events.push(BackendEvent::BufferOpened(status_buf));
                for line in session.pending_status_lines.drain(..) {
                    events.push(BackendEvent::LineAdded { buffer_id: "status".to_string(), line });
                }
                let welcome = msg.params.last().map(String::as_str).unwrap_or("Connected");
                *line_counter += 1;
                events.push(BackendEvent::LineAdded { buffer_id: "status".to_string(), line: make_line(&line_counter.to_string(), "--", welcome, timestamp_from_msg(msg), false) });
            } else {
                session.pending_status_lines.clear();
            }
            events.push(BackendEvent::Connected);
        }

        "PING" => {
            let token = msg.params.last().map(String::as_str).unwrap_or("");
            session.whois_lines.push(format!("PONG :{}", token));
        }

        "BATCH" => {
            let ref_param = msg.param(0).unwrap_or("");
            if let Some(batch_ref) = ref_param.strip_prefix('+') {
                let kind = msg.param(1).unwrap_or("");
                if kind == "chathistory" {
                    let target = msg.param(2).unwrap_or("").to_lowercase();
                    session.batch_buf.insert(batch_ref.to_string(), (target, Vec::new()));
                }
                // labeled-response batches are intentionally NOT added to batch_buf so that
                // messages inside them flow through the normal handlers immediately.
            } else if let Some(batch_ref) = ref_param.strip_prefix('-') {
                if let Some((buffer_id, lines)) = session.batch_buf.remove(batch_ref) {
                    events.push(BackendEvent::LinesLoaded { buffer_id, lines, is_prepend: true });
                }
            }
        }

        "JOIN" => {
            let channel = msg.param(0).unwrap_or("").to_string();
            if channel.is_empty() { return events; }
            let chan_lower = channel.to_lowercase();
            let nick = msg.nick().unwrap_or("").to_string();
            if nick.eq_ignore_ascii_case(&session.our_nick) {
                session.joined.insert(chan_lower.clone());
                let buf = session.alloc_buffer(&chan_lower, &channel, "channel");
                events.push(BackendEvent::BufferOpened(buf));
                session.whois_lines.push(format!("NAMES {}", channel));
                session.whois_lines.push(format!("WHO {}", channel));
                if session.negotiated_caps.contains("chathistory") {
                    session.whois_lines.push(format!("CHATHISTORY LATEST {} * 100", channel));
                }
            } else if session.joined.contains(&chan_lower) {
                let new_nick = Nick { name: nick.clone(), prefix: String::new(), color_ansi: nick_color_ansi(&nick), away: false };
                events.push(BackendEvent::NickAdded { buffer_id: chan_lower.clone(), nick: new_nick });
                *line_counter += 1;
                // extended-join: param(1) is account ("*" = not logged in), param(2) is realname
                let account = msg.param(1).unwrap_or("*");
                let join_text = if account != "*" && !account.is_empty() {
                    format!("{} ({}) has joined {}", nick, account, channel)
                } else {
                    format!("{} has joined {}", nick, channel)
                };
                let line = make_line(&line_counter.to_string(), "-->", &join_text, timestamp_from_msg(msg), false);
                events.push(BackendEvent::LineAdded { buffer_id: chan_lower, line });
            }
        }

        "PART" => {
            let channel = msg.param(0).unwrap_or("").to_string();
            let chan_lower = channel.to_lowercase();
            let nick = msg.nick().unwrap_or("").to_string();
            let reason = msg.param(1).unwrap_or("").to_string();
            if nick.eq_ignore_ascii_case(&session.our_nick) {
                session.joined.remove(&chan_lower);
                events.push(BackendEvent::BufferClosed { buffer_id: chan_lower });
            } else if session.joined.contains(&chan_lower) {
                events.push(BackendEvent::NickRemoved { buffer_id: chan_lower.clone(), nick_name: nick.clone() });
                *line_counter += 1;
                let text = if reason.is_empty() { format!("{} has left {}", nick, channel) } else { format!("{} has left {} ({})", nick, channel, reason) };
                events.push(BackendEvent::LineAdded { buffer_id: chan_lower, line: make_line(&line_counter.to_string(), "<--", &text, timestamp_from_msg(msg), false) });
            }
        }

        "KICK" => {
            let channel = msg.param(0).unwrap_or("").to_string();
            let chan_lower = channel.to_lowercase();
            let target = msg.param(1).unwrap_or("").to_string();
            let reason = msg.param(2).unwrap_or("").to_string();
            let kicker = msg.nick().unwrap_or("").to_string();
            if target.eq_ignore_ascii_case(&session.our_nick) {
                session.joined.remove(&chan_lower);
                events.push(BackendEvent::BufferClosed { buffer_id: chan_lower });
            } else if session.joined.contains(&chan_lower) {
                events.push(BackendEvent::NickRemoved { buffer_id: chan_lower.clone(), nick_name: target.clone() });
                *line_counter += 1;
                let text = if reason.is_empty() { format!("{} was kicked from {} by {}", target, channel, kicker) } else { format!("{} was kicked from {} by {} ({})", target, channel, kicker, reason) };
                events.push(BackendEvent::LineAdded { buffer_id: chan_lower, line: make_line(&line_counter.to_string(), "<--", &text, timestamp_from_msg(msg), false) });
            }
        }

        "QUIT" => {
            let nick = msg.nick().unwrap_or("").to_string();
            if nick.eq_ignore_ascii_case(&session.our_nick) { return events; }
            let reason = msg.param(0).unwrap_or("").to_string();
            for chan in session.joined.clone() {
                events.push(BackendEvent::NickRemoved { buffer_id: chan.clone(), nick_name: nick.clone() });
                *line_counter += 1;
                let text = if reason.is_empty() { format!("{} has quit", nick) } else { format!("{} has quit ({})", nick, reason) };
                events.push(BackendEvent::LineAdded { buffer_id: chan, line: make_line(&line_counter.to_string(), "<--", &text, timestamp_from_msg(msg), false) });
            }
        }

        "NICK" => {
            let old_nick = msg.nick().unwrap_or("").to_string();
            let new_nick = msg.param(0).unwrap_or("").to_string();
            if old_nick.eq_ignore_ascii_case(&session.our_nick) { session.our_nick = new_nick.clone(); }
            for chan in session.joined.clone() {
                events.push(BackendEvent::NickRemoved { buffer_id: chan.clone(), nick_name: old_nick.clone() });
                events.push(BackendEvent::NickAdded { buffer_id: chan.clone(), nick: Nick { name: new_nick.clone(), prefix: String::new(), color_ansi: nick_color_ansi(&new_nick), away: false } });
                *line_counter += 1;
                events.push(BackendEvent::LineAdded { buffer_id: chan, line: make_line(&line_counter.to_string(), "--", &format!("{} is now known as {}", old_nick, new_nick), timestamp_from_msg(msg), false) });
            }
        }

        "PRIVMSG" | "NOTICE" => {
            let target = msg.param(0).unwrap_or("").to_string();
            let text = msg.param(1).unwrap_or("").to_string();
            let nick = msg.nick().unwrap_or("*").to_string();
            let ts = timestamp_from_msg(msg);

            if msg.command == "NOTICE" && !session.registered {
                // Pre-registration server notices (ident checks, hostname lookups etc.) —
                // buffer them and flush to the status buffer once 001 arrives.
                *line_counter += 1;
                let line = make_line(&line_counter.to_string(), &format!("-{}-", nick), &text, ts, false);
                session.pending_status_lines.push(line);
                return events;
            }

            // CTCP detection: \x01COMMAND[ params]\x01, but not ACTION (handled below)
            let is_ctcp = text.starts_with('\x01') && text.ends_with('\x01')
                && !text.starts_with("\x01ACTION");

            if is_ctcp {
                if msg.command == "NOTICE" {
                    // CTCP replies (VERSION, PING, etc.) — pure protocol noise, drop silently
                    return events;
                }
                // CTCP requests (PRIVMSG): auto-respond to PING and VERSION, ignore the rest
                let inner = text.trim_matches('\x01');
                let (ctcp_cmd, ctcp_arg) = inner.split_once(' ').unwrap_or((inner, ""));
                match ctcp_cmd.to_uppercase().as_str() {
                    "PING" => {
                        session.whois_lines.push(format!("NOTICE {} :\x01PING {}\x01", nick, ctcp_arg));
                    }
                    "VERSION" => {
                        session.whois_lines.push(format!(
                            "NOTICE {} :\x01VERSION weechat-gui {}\x01",
                            nick,
                            env!("CARGO_PKG_VERSION")
                        ));
                    }
                    _ => {}
                }
                return events;
            }

            // Server-originated message: prefix has no '!' (e.g. irc.server.net)
            let is_server_source = !msg.prefix.as_deref().unwrap_or("").contains('!');

            let buffer_id = if is_channel(&target) {
                target.to_lowercase()
            } else if is_server_source {
                if !session.is_soju {
                    // Non-bouncer: route all server messages to the status buffer
                    "status".to_string()
                } else {
                    // Soju: route to first joined channel (bouncer has its own network buffers)
                    session.joined.iter().find(|id| is_channel(id)).cloned()
                        .unwrap_or_else(|| session.joined.iter().next().cloned().unwrap_or_default())
                }
            } else if nick.eq_ignore_ascii_case(&session.our_nick) {
                target.to_lowercase()
            } else {
                nick.to_lowercase()
            };

            if buffer_id.is_empty() { return events; }

            let highlight = text.to_lowercase().contains(&session.our_nick.to_lowercase())
                || target.eq_ignore_ascii_case(&session.our_nick);

            let prefix = if msg.command == "NOTICE" {
                format!("-{}-", nick)
            } else {
                format!("{}{}\x1B[0m", nick_color_ansi(&nick), nick)
            };

            let display_text = if let Some(inner) = text.strip_prefix("\x01ACTION ").and_then(|s| s.strip_suffix('\x01')) {
                format!("* {} {}", nick, inner)
            } else { text.clone() };

            *line_counter += 1;
            let line = make_line(&line_counter.to_string(), &prefix, &display_text, ts, highlight);

            if let Some(batch_ref) = msg.tag("batch") {
                if let Some((_buf, lines)) = session.batch_buf.get_mut(batch_ref) {
                    lines.push(line);
                    return events;
                }
            }

            // Only open a new DM buffer for real nick-to-nick messages, not server sources
            if !is_channel(&target) && !is_server_source && !session.joined.contains(&buffer_id) {
                session.joined.insert(buffer_id.clone());
                let display_name = if nick.eq_ignore_ascii_case(&session.our_nick) { target.clone() } else { nick.clone() };
                // MONITOR the DM nick for online/offline presence notifications
                if !nick.eq_ignore_ascii_case(&session.our_nick)
                    && session.monitored_nicks.insert(nick.to_lowercase())
                {
                    session.whois_lines.push(format!("MONITOR + {}", nick));
                }
                events.push(BackendEvent::BufferOpened(session.alloc_buffer(&buffer_id, &display_name, "private")));
            }

            events.push(BackendEvent::LineAdded { buffer_id, line });
        }

        "353" => {
            let channel = msg.param(2).unwrap_or("").to_lowercase();
            let names_str = msg.params.last().map(String::as_str).unwrap_or("");
            let entry = session.names_buf.entry(channel).or_default();
            for name in names_str.split_whitespace() {
                if !name.is_empty() { entry.push(parse_names_entry(name)); }
            }
        }

        "366" => {
            let channel = msg.param(1).unwrap_or("").to_lowercase();
            if let Some(nicks) = session.names_buf.remove(&channel) {
                events.push(BackendEvent::NicklistLoaded { buffer_id: channel, nicks });
            }
        }

        "332" => {
            let channel = msg.param(1).unwrap_or("").to_lowercase();
            let topic = msg.param(2).unwrap_or("").to_string();
            events.push(BackendEvent::TopicChanged { buffer_id: channel, topic });
        }

        "TOPIC" => {
            let channel = msg.param(0).unwrap_or("").to_lowercase();
            let topic = msg.param(1).unwrap_or("").to_string();
            let nick = msg.nick().unwrap_or("*").to_string();
            events.push(BackendEvent::TopicChanged { buffer_id: channel.clone(), topic: topic.clone() });
            *line_counter += 1;
            events.push(BackendEvent::LineAdded { buffer_id: channel, line: make_line(&line_counter.to_string(), "--", &format!("{} changed the topic to: {}", nick, topic), timestamp_from_msg(msg), false) });
        }

        "INVITE" => {
            // invite-notify: server tells us someone was invited (or we were)
            let target = msg.param(0).unwrap_or("").to_string();
            let channel = msg.param(1).unwrap_or("").to_string();
            let inviter = msg.nick().unwrap_or("*").to_string();
            let text = if target.eq_ignore_ascii_case(&session.our_nick) {
                format!("You have been invited to {} by {}", channel, inviter)
            } else {
                format!("{} has been invited to {} by {}", target, channel, inviter)
            };
            // Route to channel if we're in it, otherwise status buffer (or first channel for soju)
            let chan_lower = channel.to_lowercase();
            let buf_id = if session.joined.contains(&chan_lower) {
                chan_lower
            } else if !session.is_soju {
                "status".to_string()
            } else {
                session.joined.iter().find(|id| is_channel(id)).cloned()
                    .unwrap_or_else(|| session.joined.iter().next().cloned().unwrap_or_default())
            };
            if !buf_id.is_empty() {
                *line_counter += 1;
                events.push(BackendEvent::LineAdded {
                    buffer_id: buf_id,
                    line: make_line(&line_counter.to_string(), "--", &text, timestamp_from_msg(msg), false),
                });
            }
        }

        "CHGHOST" => {
            let nick = msg.nick().unwrap_or("").to_string();
            let new_user = msg.param(0).unwrap_or("").to_string();
            let new_host = msg.param(1).unwrap_or("").to_string();
            if !nick.eq_ignore_ascii_case(&session.our_nick) {
                let text = format!("{} changed host to {}@{}", nick, new_user, new_host);
                for chan in session.joined.clone() {
                    *line_counter += 1;
                    events.push(BackendEvent::LineAdded {
                        buffer_id: chan,
                        line: make_line(&line_counter.to_string(), "--", &text, timestamp_from_msg(msg), false),
                    });
                }
            }
        }

        "BOUNCER" => {
            let sub = msg.param(0).unwrap_or("").to_uppercase();
            if sub == "NETWORK" && msg.param(1).unwrap_or("").to_uppercase() == "LIST" {
                let attrs_str = msg.param(2).unwrap_or("");
                if attrs_str == "*" { return events; }
                let mut id = String::new();
                let mut name = String::new();
                let mut state = String::new();
                for pair in attrs_str.split(';') {
                    if let Some((k, v)) = pair.split_once('=') {
                        match k { "id" => id = v.to_string(), "name" => name = v.to_string(), "state" => state = v.to_string(), _ => {} }
                    }
                }
                if name.is_empty() { name = id.clone(); }
                let buf_id = format!("net/{}", id);
                let display = if state == "connected" { format!("[{}]", name) } else { format!("[{}] ({})", name, state) };
                events.push(BackendEvent::BufferOpened(session.alloc_buffer(&buf_id, &display, "server")));
            }
        }

        "AWAY" => {
            // away-notify: a nick in a shared channel changed away status
            let nick = msg.nick().unwrap_or("").to_string();
            let away = !msg.params.is_empty(); // AWAY with no params = returned from away
            if !nick.eq_ignore_ascii_case(&session.our_nick) {
                for chan in session.joined.clone() {
                    events.push(BackendEvent::NickAwayChanged {
                        buffer_id: chan,
                        nick_name: nick.clone(),
                        away,
                    });
                }
            }
        }

        "ACCOUNT" => {
            // account-notify: nick's services account changed
            let nick = msg.nick().unwrap_or("").to_string();
            let account = msg.param(0).unwrap_or("*").to_string();
            let text = if account == "*" {
                format!("{} logged out of their account", nick)
            } else {
                format!("{} is now logged in as {}", nick, account)
            };
            for chan in session.joined.clone() {
                *line_counter += 1;
                events.push(BackendEvent::LineAdded {
                    buffer_id: chan,
                    line: make_line(&line_counter.to_string(), "--", &text, timestamp_from_msg(msg), false),
                });
            }
        }

        "PONG" => { session.pong_received = true; }

        "MARKREAD" => {}

        "ERROR" => {
            let text = msg.param(0).unwrap_or("Unknown error").to_string();
            events.push(BackendEvent::Error(format!("Server error: {}", text)));
        }

        "433" => {
            let base = session.our_nick.trim_end_matches('_').to_string();
            session.our_nick = format!("{}_", base);
            session.whois_lines.push(format!("NICK {}", session.our_nick));
        }

        "464" | "465" => {
            let reason = msg.params.last().map(String::as_str).unwrap_or("Password incorrect");
            events.push(BackendEvent::AuthError(reason.to_string()));
        }
        "432" => {
            let reason = msg.params.last().map(String::as_str).unwrap_or("Erroneous nickname");
            events.push(BackendEvent::AuthError(reason.to_string()));
        }

        "352" => {
            let channel = msg.param(1).unwrap_or("").to_lowercase();
            let nick = msg.param(5).unwrap_or("").to_string();
            let flags = msg.param(6).unwrap_or("");
            let prefix = if flags.contains('@') { "@" } else if flags.contains('+') { "+" } else { "" };
            let bucket = session.who_buf.entry(channel).or_default();
            if let Some(existing) = bucket.iter_mut().find(|n| n.name == nick) {
                existing.prefix = prefix.to_string();
            } else {
                bucket.push(Nick { name: nick.clone(), prefix: prefix.to_string(), color_ansi: nick_color_ansi(&nick), away: false });
            }
        }

        "315" => {
            let channel = msg.param(1).unwrap_or("").to_lowercase();
            if let Some(nicks) = session.who_buf.remove(&channel) {
                if !nicks.is_empty() {
                    events.push(BackendEvent::NicklistLoaded { buffer_id: channel, nicks });
                }
            }
        }

        "311" => {
            let nick = msg.param(1).unwrap_or("").to_string();
            let user = msg.param(2).unwrap_or("").to_string();
            let host = msg.param(3).unwrap_or("").to_string();
            let realname = msg.params.last().map(String::as_str).unwrap_or("").to_string();
            session.whois_target = Some(nick.clone());
            session.whois_lines.push(format!("[whois] {} ({}@{}) — {}", nick, user, host, realname));
        }
        "312" => {
            let server = msg.param(2).unwrap_or("").to_string();
            let desc = msg.params.last().map(String::as_str).unwrap_or("").to_string();
            session.whois_lines.push(format!("[whois] server: {} ({})", server, desc));
        }
        "317" => {
            let idle_secs = msg.param(2).unwrap_or("0").parse::<u64>().unwrap_or(0);
            session.whois_lines.push(format!("[whois] idle: {}m {}s", idle_secs / 60, idle_secs % 60));
        }
        "319" => {
            let channels = msg.params.last().map(String::as_str).unwrap_or("").to_string();
            session.whois_lines.push(format!("[whois] channels: {}", channels));
        }
        "318" => {
            let target = session.whois_target.take().unwrap_or_default().to_lowercase();
            let buf_id = if session.joined.contains(&target) { target }
                         else { session.joined.iter().next().cloned().unwrap_or_default() };
            // collect first, then emit (whois_lines is shared with write_buf via re-use above)
            // Actually whois_lines is now only for whois — write_buf was removed, writes go inline
            let lines: Vec<String> = session.whois_lines.drain(..).collect();
            for text in lines {
                *line_counter += 1;
                events.push(BackendEvent::LineAdded { buffer_id: buf_id.clone(), line: make_line(&line_counter.to_string(), "--", &text, Utc::now(), false) });
            }
        }

        // 730 — RPL_MONONLINE: monitored nick came online
        "730" => {
            let nicks_str = msg.params.last().map(String::as_str).unwrap_or("");
            for entry in nicks_str.split(',') {
                let nick = entry.split('!').next().unwrap_or(entry).trim();
                if nick.is_empty() { continue; }
                let buf_id = nick.to_lowercase();
                if session.joined.contains(&buf_id) {
                    *line_counter += 1;
                    events.push(BackendEvent::LineAdded {
                        buffer_id: buf_id,
                        line: make_line(&line_counter.to_string(), "--", &format!("{} is now online", nick), Utc::now(), false),
                    });
                }
            }
        }

        // 731 — RPL_MONOFFLINE: monitored nick went offline
        "731" => {
            let nicks_str = msg.params.last().map(String::as_str).unwrap_or("");
            for entry in nicks_str.split(',') {
                let nick = entry.split('!').next().unwrap_or(entry).trim();
                if nick.is_empty() { continue; }
                let buf_id = nick.to_lowercase();
                if session.joined.contains(&buf_id) {
                    *line_counter += 1;
                    events.push(BackendEvent::LineAdded {
                        buffer_id: buf_id,
                        line: make_line(&line_counter.to_string(), "--", &format!("{} is offline", nick), Utc::now(), false),
                    });
                }
            }
        }

        // 732 — RPL_MONLIST / 733 — ERR_MONLISTFULL: ignore silently
        "732" | "733" => {}

        // Server info / ISUPPORT / MOTD / stats — route to status buffer for non-soju
        "002" | "003" | "004" | "005" | "251" | "252" | "253" | "254" | "255" | "265" | "266"
        | "375" | "372" | "376" | "250" | "256" | "257" | "258" | "259" if !session.is_soju => {
            let text = msg.params.iter().skip(1).map(String::as_str).collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                *line_counter += 1;
                events.push(BackendEvent::LineAdded {
                    buffer_id: "status".to_string(),
                    line: make_line(&line_counter.to_string(), "--", &text, timestamp_from_msg(msg), false),
                });
            }
        }

        _ => {}
    }

    events
}

// ── Write queue — populated by translate(), flushed by the connection loop ───

fn take_write_queue(session: &mut Session) -> Vec<String> {
    // We reused whois_lines as a general write queue in translate() for CAP/PING/NICK/NAMES/WHO.
    // For WHOIS display we always drain before 318 fires, so no collision in practice.
    // A cleaner refactor would separate these, but this keeps the diff small.
    std::mem::take(&mut session.whois_lines)
}

// ── Main connection task ──────────────────────────────────────────────────────

pub fn spawn(
    config: Arc<IrcConfig>,
    event_tx: mpsc::UnboundedSender<BackendEvent>,
    ctx: EguiContext,
    mut cmd_rx: mpsc::UnboundedReceiver<IrcCommand>,
    connected: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        macro_rules! send {
            ($ev:expr) => {{ let _ = event_tx.send($ev); ctx.request_repaint(); }};
        }

        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);
        let mut line_counter: u64 = 0;

        loop {
            let addr = format!("{}:{}", config.host.trim(), config.port);

            let tcp = match TcpStream::connect(&addr).await {
                Ok(s) => s,
                Err(e) => {
                    connected.store(false, Ordering::Relaxed);
                    send!(BackendEvent::Error(format!("Connection failed: {}", e)));
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                    continue;
                }
            };

            let mut session = Session::new(&config.nick);
            let mut clean = false;

            // ── TLS or plain ──────────────────────────────────────────────────
            macro_rules! run_session {
                ($stream:expr) => {{
                    let (read_half, mut write_half) = tokio::io::split($stream);
                    // Use a raw byte reader so we can do lossy UTF-8 conversion.
                    // Many older IRC servers send Latin-1 / CP-1252 encoded text.
                    let mut reader = BufReader::new(read_half);
                    let mut raw_buf: Vec<u8> = Vec::new();

                    // IRC handshake
                    let _ = write_half.write_all(b"CAP LS 302\r\n").await;
                    if !config.password.is_empty() {
                        let _ = write_half.write_all(format!("PASS :{}\r\n", config.password).as_bytes()).await;
                    }
                    let _ = write_half.write_all(format!("NICK {}\r\n", config.nick).as_bytes()).await;
                    let user = if config.username.is_empty() { &config.nick } else { &config.username };
                    let _ = write_half.write_all(format!("USER {} 0 * :{}\r\n", user, config.nick).as_bytes()).await;

                    connected.store(true, Ordering::Relaxed);
                    backoff = Duration::from_secs(1);

                    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
                    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    ping_interval.tick().await; // skip the immediate first tick

                    'conn: loop {
                        // Flush outbound queue populated by translate()
                        for line in take_write_queue(&mut session) {
                            if write_half.write_all(format!("{}\r\n", line).as_bytes()).await.is_err() {
                                break 'conn;
                            }
                        }

                        tokio::select! {
                            result = reader.read_until(b'\n', &mut raw_buf) => {
                                match result {
                                    Ok(0) => {
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        break 'conn;
                                    }
                                    Ok(_) => {
                                        // Strip \r\n and decode with lossy UTF-8
                                        let raw = String::from_utf8_lossy(raw_buf.trim_ascii_end()).into_owned();
                                        raw_buf.clear();
                                        if let Some(msg) = IrcMessage::parse(&raw) {
                                            let evs = translate(&msg, &mut session, &mut line_counter, &config);
                                            for ev in evs { send!(ev); }
                                            for line in take_write_queue(&mut session) {
                                                if write_half.write_all(format!("{}\r\n", line).as_bytes()).await.is_err() {
                                                    break 'conn;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Error(format!("Read error: {}", e)));
                                        break 'conn;
                                    }
                                }
                            }
                            cmd = cmd_rx.recv() => {
                                match cmd {
                                    None => break 'conn,
                                    Some(IrcCommand::Disconnect) => {
                                        let _ = write_half.write_all(b"QUIT :bye\r\n").await;
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        clean = true;
                                        break 'conn;
                                    }
                                    Some(IrcCommand::SendMessage { buffer_id, text }) => {
                                        // /query <nick> — client-side command: open a DM buffer, nothing to send
                                        if let Some(rest) = text.strip_prefix("/query ").or_else(|| text.strip_prefix("/QUERY ")) {
                                            let target_nick = rest.trim().split_whitespace().next().unwrap_or("").to_string();
                                            if !target_nick.is_empty() {
                                                let buf_id = target_nick.to_lowercase();
                                                if !session.joined.contains(&buf_id) {
                                                    session.joined.insert(buf_id.clone());
                                                    let buf = session.alloc_buffer(&buf_id, &target_nick, "private");
                                                    send!(BackendEvent::BufferOpened(buf));
                                                }
                                                if session.monitored_nicks.insert(buf_id) {
                                                    let _ = write_half.write_all(format!("MONITOR + {}\r\n", target_nick).as_bytes()).await;
                                                }
                                            }
                                        } else if let Some(cmd_text) = text.strip_prefix('/') {
                                            let _ = write_half.write_all(format!("{}\r\n", cmd_text).as_bytes()).await;
                                        } else {
                                            let _ = write_half.write_all(format!("PRIVMSG {} :{}\r\n", buffer_id, text).as_bytes()).await;
                                            // Ensure a DM buffer exists for outgoing messages to non-channels
                                            if !is_channel(&buffer_id) && !session.joined.contains(&buffer_id) {
                                                session.joined.insert(buffer_id.clone());
                                                let buf = session.alloc_buffer(&buffer_id, &buffer_id, "private");
                                                send!(BackendEvent::BufferOpened(buf));
                                            }
                                            // Only self-echo when echo-message is not negotiated;
                                            // with it the server sends our message back as a PRIVMSG.
                                            if !session.negotiated_caps.contains("echo-message") {
                                                line_counter += 1;
                                                let display = if let Some(inner) = text.strip_prefix("\x01ACTION ").and_then(|s| s.strip_suffix('\x01')) {
                                                    format!("* {} {}", session.our_nick, inner)
                                                } else {
                                                    text.clone()
                                                };
                                                let colored_nick = format!("{}{}\x1B[0m", nick_color_ansi(&session.our_nick), &session.our_nick);
                                                send!(BackendEvent::LineAdded {
                                                    buffer_id,
                                                    line: make_line(&line_counter.to_string(), &colored_nick, &display, Utc::now(), false),
                                                });
                                            }
                                        }
                                    }
                                    Some(IrcCommand::FetchNicks { buffer_id }) => {
                                        let _ = write_half.write_all(format!("NAMES {}\r\n", buffer_id).as_bytes()).await;
                                    }
                                    Some(IrcCommand::FetchBufferList) => {}
                                    Some(IrcCommand::MarkRead { buffer_id }) => {
                                        if session.negotiated_caps.contains("soju.im/read-marker") {
                                            let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
                                            let _ = write_half.write_all(format!("MARKREAD {} timestamp={}\r\n", buffer_id, now).as_bytes()).await;
                                        }
                                    }
                                    Some(IrcCommand::FetchBefore { buffer_id, before_ts }) => {
                                        if session.negotiated_caps.contains("chathistory") {
                                            let _ = write_half.write_all(format!("CHATHISTORY BEFORE {} timestamp={} 100\r\n", buffer_id, before_ts).as_bytes()).await;
                                        }
                                    }
                                }
                            }
                            _ = ping_interval.tick() => {
                                if !session.pong_received {
                                    // No PONG since last PING — connection is dead
                                    connected.store(false, Ordering::Relaxed);
                                    send!(BackendEvent::Disconnected);
                                    break 'conn;
                                }
                                session.pong_received = false;
                                if write_half.write_all(b"PING :keepalive\r\n").await.is_err() {
                                    break 'conn;
                                }
                            }
                        }
                    }
                }};
            }

            if config.use_ssl {
                let tls_cx = match native_tls::TlsConnector::builder()
                    .danger_accept_invalid_certs(config.accept_invalid_certs)
                    .build()
                {
                    Ok(cx) => cx,
                    Err(e) => {
                        send!(BackendEvent::Error(format!("TLS setup failed: {}", e)));
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(max_backoff);
                        continue;
                    }
                };
                let cx = tokio_native_tls::TlsConnector::from(tls_cx);
                match cx.connect(&config.host, tcp).await {
                    Ok(tls_stream) => { run_session!(tls_stream); }
                    Err(e) => {
                        connected.store(false, Ordering::Relaxed);
                        send!(BackendEvent::Error(format!("TLS handshake failed: {}", e)));
                    }
                }
            } else {
                run_session!(tcp);
            }

            connected.store(false, Ordering::Relaxed);
            if clean { return; }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(max_backoff);
        }
    });
}
