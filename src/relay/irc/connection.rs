use super::{parser::IrcMessage, IrcConfig};
use crate::relay::backend::BackendEvent;
use crate::relay::models::{Buffer, BufferActivity, Line, Nick};
use chrono::{DateTime, Utc};
use egui::Context as EguiContext;
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_tls_with_config, tungstenite::client::IntoClientRequest,
    tungstenite::protocol::Message, Connector,
};
use url::Url;

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
    /// Capabilities advertised by the server (from CAP LS).
    available_caps: HashSet<String>,
    /// Capabilities we successfully negotiated.
    negotiated_caps: HashSet<String>,
    cap_ls_done: bool,
    registered: bool,
    /// Channels we have joined (lowercase).
    joined: HashSet<String>,
    /// Running counter for Buffer.number assignment.
    next_number: i32,
    /// Accumulating NAMES lists (lowercased channel → nicks).
    names_buf: HashMap<String, Vec<Nick>>,
    /// Open BATCH blocks: ref → (buffer_id, accumulated lines).
    batch_buf: HashMap<String, (String, Vec<Line>)>,
    /// Outbound raw IRC lines queued during handshake, drained after WS ready.
    write_buf: Vec<String>,
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
            joined: HashSet::new(),
            next_number: 1,
            names_buf: HashMap::new(),
            batch_buf: HashMap::new(),
            write_buf: Vec::new(),
        }
    }

    fn wants_caps() -> &'static [&'static str] {
        &[
            "message-tags",
            "server-time",
            "multi-prefix",
            "away-notify",
            "account-notify",
            "extended-join",
            "batch",
            "chathistory",
            "soju.im/bouncer-networks",
            "soju.im/read-marker",
        ]
    }

    /// Produce the CAP REQ string filtered to what the server actually offers.
    fn cap_req(&self) -> String {
        let wanted: Vec<&str> = Self::wants_caps()
            .iter()
            .filter(|&&c| self.available_caps.contains(c))
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

// ── IRC nick coloring ─────────────────────────────────────────────────────────

fn nick_color_ansi(name: &str) -> String {
    let mut h: u32 = 0;
    for b in name.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as u32);
    }
    let idx = ((h % 15) + 1) as u8;
    if idx < 8 {
        format!("\x1B[{}m", 30 + idx)
    } else {
        format!("\x1B[{}m", 90 + idx - 8)
    }
}

fn strip_prefix_chars(name: &str) -> (&str, &str) {
    let prefix_end = name
        .find(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '[' || c == ']' || c == '{' || c == '}' || c == '\\' || c == '`' || c == '^' || c == '|')
        .unwrap_or(0);
    (&name[..prefix_end], &name[prefix_end..])
}

fn parse_names_entry(entry: &str) -> Nick {
    let (prefix, name) = strip_prefix_chars(entry);
    Nick {
        name: name.to_string(),
        prefix: prefix.to_string(),
        color_ansi: nick_color_ansi(name),
    }
}

// ── IRC → BackendEvent translation ───────────────────────────────────────────

fn timestamp_from_msg(msg: &IrcMessage) -> DateTime<Utc> {
    msg.tag("time")
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

fn make_line(id: &str, prefix: &str, text: &str, ts: DateTime<Utc>, highlight: bool) -> Line {
    Line {
        id: id.to_string(),
        timestamp: ts,
        prefix: prefix.to_string(),
        message: text.to_string(),
        displayed: true,
        highlight,
    }
}

fn is_channel(target: &str) -> bool {
    target.starts_with('#') || target.starts_with('&') || target.starts_with('!')
}

/// Translate one IRC message into zero or more BackendEvents.
/// Returns `None` for messages that need an outbound reply (PING) — caller handles those.
fn translate(msg: &IrcMessage, session: &mut Session, line_counter: &mut u64) -> Vec<BackendEvent> {
    let mut events = Vec::new();

    match msg.command.as_str() {
        // ── CAP negotiation ───────────────────────────────────────────────
        "CAP" => {
            let sub = msg.param(1).unwrap_or("").to_uppercase();
            match sub.as_str() {
                "LS" => {
                    let caps_str = msg.params.last().map(String::as_str).unwrap_or("");
                    for cap in caps_str.split_whitespace() {
                        // Strip =value if present
                        let cap_name = cap.split('=').next().unwrap_or(cap);
                        session.available_caps.insert(cap_name.to_string());
                    }
                    // Multiline CAP LS ends when the * is absent (or line doesn't have *)
                    let is_continuation = msg.param(1).map(|p| p == "*").unwrap_or(false);
                    if !is_continuation {
                        session.cap_ls_done = true;
                        let req = session.cap_req();
                        if !req.is_empty() {
                            session.write_buf.push(format!("CAP REQ :{}", req));
                        } else {
                            session.write_buf.push("CAP END".to_string());
                        }
                    }
                }
                "ACK" => {
                    let acked = msg.params.last().map(String::as_str).unwrap_or("");
                    for cap in acked.split_whitespace() {
                        session.negotiated_caps.insert(cap.trim_start_matches('-').to_string());
                    }
                    session.write_buf.push("CAP END".to_string());
                }
                "NAK" => {
                    // Server rejected our request; just end CAP and proceed
                    session.write_buf.push("CAP END".to_string());
                }
                _ => {}
            }
        }

        // ── Registration replies ──────────────────────────────────────────
        "001" => {
            // Update our nick from the target (server may have adjusted it)
            if let Some(nick) = msg.param(0) {
                session.our_nick = nick.to_string();
            }
            if let Some(srv) = &msg.prefix {
                session.server_name = srv.split('!').next().unwrap_or(srv).to_string();
            }
            session.registered = true;
            events.push(BackendEvent::Connected);
        }

        // ── PING/PONG ─────────────────────────────────────────────────────
        "PING" => {
            let token = msg.params.last().map(String::as_str).unwrap_or("");
            session.write_buf.push(format!("PONG :{}", token));
        }

        // ── JOIN ──────────────────────────────────────────────────────────
        "JOIN" => {
            let channel = msg.param(0).unwrap_or("").to_string();
            if channel.is_empty() { return events; }
            let chan_lower = channel.to_lowercase();
            let nick = msg.nick().unwrap_or("").to_string();

            if nick.eq_ignore_ascii_case(&session.our_nick) {
                // We joined a channel
                session.joined.insert(chan_lower.clone());
                let buf = session.alloc_buffer(&chan_lower, &channel, "channel");
                events.push(BackendEvent::BufferOpened(buf));
                // Request NAMES (soju delivers them automatically, but ask anyway)
                session.write_buf.push(format!("NAMES {}", channel));
                // Request chathistory if negotiated
                if session.negotiated_caps.contains("chathistory") {
                    session.write_buf.push(format!("CHATHISTORY LATEST {} * 100", channel));
                }
            } else if session.joined.contains(&chan_lower) {
                // Someone else joined
                let new_nick = Nick {
                    name: nick.clone(),
                    prefix: String::new(),
                    color_ansi: nick_color_ansi(&nick),
                };
                events.push(BackendEvent::NickAdded {
                    buffer_id: chan_lower.clone(),
                    nick: new_nick,
                });
                // System line
                *line_counter += 1;
                let line = make_line(
                    &line_counter.to_string(),
                    "-->",
                    &format!("{} has joined {}", nick, channel),
                    timestamp_from_msg(msg),
                    false,
                );
                events.push(BackendEvent::LineAdded { buffer_id: chan_lower, line });
            }
        }

        // ── PART ──────────────────────────────────────────────────────────
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
                let msg_text = if reason.is_empty() {
                    format!("{} has left {}", nick, channel)
                } else {
                    format!("{} has left {} ({})", nick, channel, reason)
                };
                let line = make_line(&line_counter.to_string(), "<--", &msg_text, timestamp_from_msg(msg), false);
                events.push(BackendEvent::LineAdded { buffer_id: chan_lower, line });
            }
        }

        // ── KICK ──────────────────────────────────────────────────────────
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
                let msg_text = if reason.is_empty() {
                    format!("{} was kicked from {} by {}", target, channel, kicker)
                } else {
                    format!("{} was kicked from {} by {} ({})", target, channel, kicker, reason)
                };
                let line = make_line(&line_counter.to_string(), "<--", &msg_text, timestamp_from_msg(msg), false);
                events.push(BackendEvent::LineAdded { buffer_id: chan_lower, line });
            }
        }

        // ── QUIT ──────────────────────────────────────────────────────────
        "QUIT" => {
            let nick = msg.nick().unwrap_or("").to_string();
            if nick.eq_ignore_ascii_case(&session.our_nick) {
                return events;
            }
            let reason = msg.param(0).unwrap_or("").to_string();
            for chan in session.joined.clone() {
                events.push(BackendEvent::NickRemoved { buffer_id: chan.clone(), nick_name: nick.clone() });
                *line_counter += 1;
                let msg_text = if reason.is_empty() {
                    format!("{} has quit", nick)
                } else {
                    format!("{} has quit ({})", nick, reason)
                };
                let line = make_line(&line_counter.to_string(), "<--", &msg_text, timestamp_from_msg(msg), false);
                events.push(BackendEvent::LineAdded { buffer_id: chan, line });
            }
        }

        // ── NICK change ───────────────────────────────────────────────────
        "NICK" => {
            let old_nick = msg.nick().unwrap_or("").to_string();
            let new_nick = msg.param(0).unwrap_or("").to_string();

            if old_nick.eq_ignore_ascii_case(&session.our_nick) {
                session.our_nick = new_nick.clone();
            }

            for chan in session.joined.clone() {
                events.push(BackendEvent::NickRemoved { buffer_id: chan.clone(), nick_name: old_nick.clone() });
                let new = Nick { name: new_nick.clone(), prefix: String::new(), color_ansi: nick_color_ansi(&new_nick) };
                events.push(BackendEvent::NickAdded { buffer_id: chan.clone(), nick: new });
                *line_counter += 1;
                let line = make_line(
                    &line_counter.to_string(),
                    "--",
                    &format!("{} is now known as {}", old_nick, new_nick),
                    timestamp_from_msg(msg),
                    false,
                );
                events.push(BackendEvent::LineAdded { buffer_id: chan, line });
            }
        }

        // ── BATCH ─────────────────────────────────────────────────────────
        "BATCH" => {
            let ref_param = msg.param(0).unwrap_or("");
            if let Some(batch_ref) = ref_param.strip_prefix('+') {
                // Opening a batch: BATCH +<ref> chathistory <target>
                let kind = msg.param(1).unwrap_or("");
                if kind == "chathistory" {
                    let target = msg.param(2).unwrap_or("").to_lowercase();
                    session.batch_buf.insert(batch_ref.to_string(), (target, Vec::new()));
                }
            } else if let Some(batch_ref) = ref_param.strip_prefix('-') {
                // Closing a batch — emit collected lines as LinesLoaded (prepend)
                if let Some((buffer_id, lines)) = session.batch_buf.remove(batch_ref) {
                    events.push(BackendEvent::LinesLoaded {
                        buffer_id,
                        lines,
                        is_prepend: true,
                    });
                }
            }
        }

        // ── PRIVMSG / NOTICE ──────────────────────────────────────────────
        "PRIVMSG" | "NOTICE" => {
            let target = msg.param(0).unwrap_or("").to_string();
            let text = msg.param(1).unwrap_or("").to_string();
            let nick = msg.nick().unwrap_or("*").to_string();
            let ts = timestamp_from_msg(msg);

            let buffer_id = if is_channel(&target) {
                target.to_lowercase()
            } else if nick.eq_ignore_ascii_case(&session.our_nick) {
                // Our own message echoed back — target is the DM recipient
                target.to_lowercase()
            } else {
                nick.to_lowercase()
            };

            let highlight = text.to_lowercase().contains(&session.our_nick.to_lowercase())
                || target.eq_ignore_ascii_case(&session.our_nick);

            let prefix = if msg.command == "NOTICE" {
                format!("-{}-", nick)
            } else {
                nick.clone()
            };

            // ACTION (/me)
            let display_text = if let Some(inner) = text.strip_prefix("\x01ACTION ").and_then(|s| s.strip_suffix('\x01')) {
                format!("* {} {}", nick, inner)
            } else {
                text.clone()
            };

            *line_counter += 1;
            let id = line_counter.to_string();
            let line = make_line(&id, &prefix, &display_text, ts, highlight);

            // If this message belongs to an open batch, accumulate it there
            if let Some(batch_ref) = msg.tag("batch") {
                if let Some((_buf, lines)) = session.batch_buf.get_mut(batch_ref) {
                    lines.push(line);
                    return events;
                }
            }

            events.push(BackendEvent::LineAdded { buffer_id, line });
        }

        // ── 353 RPL_NAMREPLY ──────────────────────────────────────────────
        "353" => {
            // :server 353 our_nick = #channel :nick1 @nick2 +nick3
            let channel = msg.param(2).unwrap_or("").to_lowercase();
            let names_str = msg.params.last().map(String::as_str).unwrap_or("");
            let entry = session.names_buf.entry(channel).or_default();
            for name in names_str.split_whitespace() {
                if !name.is_empty() {
                    entry.push(parse_names_entry(name));
                }
            }
        }

        // ── 366 RPL_ENDOFNAMES ────────────────────────────────────────────
        "366" => {
            let channel = msg.param(1).unwrap_or("").to_lowercase();
            if let Some(nicks) = session.names_buf.remove(&channel) {
                events.push(BackendEvent::NicklistLoaded { buffer_id: channel, nicks });
            }
        }

        // ── 332 RPL_TOPIC ─────────────────────────────────────────────────
        "332" => {
            let channel = msg.param(1).unwrap_or("").to_lowercase();
            let topic = msg.param(2).unwrap_or("").to_string();
            events.push(BackendEvent::TopicChanged { buffer_id: channel, topic });
        }

        // ── TOPIC (live change) ───────────────────────────────────────────
        "TOPIC" => {
            let channel = msg.param(0).unwrap_or("").to_lowercase();
            let topic = msg.param(1).unwrap_or("").to_string();
            let nick = msg.nick().unwrap_or("*").to_string();
            events.push(BackendEvent::TopicChanged { buffer_id: channel.clone(), topic: topic.clone() });
            *line_counter += 1;
            let line = make_line(
                &line_counter.to_string(),
                "--",
                &format!("{} changed the topic to: {}", nick, topic),
                timestamp_from_msg(msg),
                false,
            );
            events.push(BackendEvent::LineAdded { buffer_id: channel, line });
        }

        // ── BOUNCER (soju.im/bouncer-networks) ───────────────────────────
        "BOUNCER" => {
            // :server BOUNCER NETWORK LIST id=<id>;name=<name>;state=<state>
            // :server BOUNCER NETWORK LIST * :end of network list
            let sub = msg.param(0).unwrap_or("").to_uppercase();
            if sub == "NETWORK" {
                let action = msg.param(1).unwrap_or("").to_uppercase();
                if action == "LIST" {
                    let attrs_str = msg.param(2).unwrap_or("");
                    // '*' signals end of list
                    if attrs_str == "*" { return events; }

                    // Parse semicolon-separated key=value attrs
                    let mut id = String::new();
                    let mut name = String::new();
                    let mut state = String::new();
                    for pair in attrs_str.split(';') {
                        if let Some((k, v)) = pair.split_once('=') {
                            match k {
                                "id"    => id    = v.to_string(),
                                "name"  => name  = v.to_string(),
                                "state" => state = v.to_string(),
                                _ => {}
                            }
                        }
                    }
                    if name.is_empty() { name = id.clone(); }

                    // Create a server-type buffer for each network
                    let buf_id = format!("net/{}", id);
                    let display = if state == "connected" {
                        format!("[{}]", name)
                    } else {
                        format!("[{}] ({})", name, state)
                    };
                    let buf = session.alloc_buffer(&buf_id, &display, "server");
                    events.push(BackendEvent::BufferOpened(buf));
                }
            }
        }

        // ── MARKREAD (soju.im/read-marker) ───────────────────────────────
        "MARKREAD" => {
            // :server MARKREAD #channel timestamp=<ISO8601>
            // We receive this when another client updates the read marker.
            // For now we just acknowledge it; future: sync unread counts.
            let _target = msg.param(0).unwrap_or("");
            let _stamp  = msg.param(1).unwrap_or("");
        }

        // ── ERROR ─────────────────────────────────────────────────────────
        "ERROR" => {
            let text = msg.param(0).unwrap_or("Unknown error").to_string();
            events.push(BackendEvent::Error(format!("Server error: {}", text)));
        }

        // ── 433 nick already in use ───────────────────────────────────────
        "433" => {
            let base = session.our_nick.trim_end_matches('_').to_string();
            session.our_nick = format!("{}_", base);
            session.write_buf.push(format!("NICK {}", session.our_nick));
        }

        _ => {}
    }

    events
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
            ($ev:expr) => {{
                let _ = event_tx.send($ev);
                ctx.request_repaint();
            }};
        }

        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);
        let mut line_counter: u64 = 0;

        let host_clean = config.host.trim()
            .replace("https://", "").replace("http://", "")
            .replace("wss://", "").replace("ws://", "");
        let scheme = if config.use_ssl { "wss" } else { "ws" };
        let url_str = format!("{}://{}:{}", scheme, host_clean, config.port);

        loop {
            let url = match Url::parse(&url_str) {
                Ok(u) => u,
                Err(e) => {
                    send!(BackendEvent::Error(format!("Invalid URL: {}", e)));
                    return;
                }
            };

            send!(BackendEvent::Error(format!("Connecting to {}…", url_str)));

            let mut request = match url.into_client_request() {
                Ok(r) => r,
                Err(e) => {
                    send!(BackendEvent::Error(format!("Request error: {}", e)));
                    return;
                }
            };
            // soju WebSocket endpoint expects these headers
            request.headers_mut().insert(
                "Origin",
                format!("https://{}", host_clean).parse().unwrap(),
            );
            request.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                "text.ircv3.net".parse().unwrap(),
            );

            let connector = if config.use_ssl {
                native_tls::TlsConnector::builder()
                    .danger_accept_invalid_certs(config.accept_invalid_certs)
                    .build()
                    .ok()
                    .map(Connector::NativeTls)
            } else {
                None
            };

            match connect_async_tls_with_config(request, None, false, connector).await {
                Err(e) => {
                    connected.store(false, Ordering::Relaxed);
                    send!(BackendEvent::Error(format!("Connection failed: {}", e)));
                }
                Ok((ws_stream, _)) => {
                    connected.store(true, Ordering::Relaxed);
                    backoff = Duration::from_secs(1);

                    let (mut ws_tx, mut ws_rx) = ws_stream.split();
                    let mut session = Session::new(&config.nick);

                    // IRC handshake — start CAP negotiation before auth
                    let _ = ws_tx.send(Message::Text("CAP LS 302\r\n".to_string().into())).await;
                    if !config.password.is_empty() {
                        let _ = ws_tx.send(Message::Text(format!("PASS :{}\r\n", config.password).into())).await;
                    }
                    let _ = ws_tx.send(Message::Text(format!("NICK {}\r\n", config.nick).into())).await;
                    let _ = ws_tx.send(Message::Text(format!("USER {} 0 * :{}\r\n", config.nick, config.nick).into())).await;

                    let mut clean = false;

                    'conn: loop {
                        // Flush any queued writes from message processing
                        for line in session.write_buf.drain(..) {
                            if ws_tx.send(Message::Text(format!("{}\r\n", line).into())).await.is_err() {
                                break 'conn;
                            }
                        }

                        tokio::select! {
                            ws_msg = ws_rx.next() => {
                                match ws_msg {
                                    None | Some(Err(_)) => break 'conn,
                                    Some(Ok(Message::Close(_))) => {
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        break 'conn;
                                    }
                                    Some(Ok(Message::Text(text))) => {
                                        // IRC can send multiple lines in one frame
                                        for raw_line in text.lines() {
                                            if raw_line.is_empty() { continue; }
                                            if let Some(msg) = IrcMessage::parse(raw_line) {
                                                let evs = translate(&msg, &mut session, &mut line_counter);
                                                for ev in evs {
                                                    send!(ev);
                                                }
                                                // Flush writes generated by translate()
                                                for line in session.write_buf.drain(..) {
                                                    if ws_tx.send(Message::Text(format!("{}\r\n", line).into())).await.is_err() {
                                                        break 'conn;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some(Ok(_)) => {}
                                }
                            }
                            cmd = cmd_rx.recv() => {
                                match cmd {
                                    None => break 'conn,
                                    Some(IrcCommand::Disconnect) => {
                                        let _ = ws_tx.send(Message::Text("QUIT :bye\r\n".to_string().into())).await;
                                        let _ = ws_tx.send(Message::Close(None)).await;
                                        connected.store(false, Ordering::Relaxed);
                                        send!(BackendEvent::Disconnected);
                                        clean = true;
                                        break 'conn;
                                    }
                                    Some(IrcCommand::SendMessage { buffer_id, text }) => {
                                        // Decode the buffer_id back to a proper target
                                        let target = if is_channel(&buffer_id) {
                                            buffer_id.clone()
                                        } else {
                                            buffer_id.clone()
                                        };
                                        // Handle commands prefixed with /
                                        if let Some(cmd_text) = text.strip_prefix('/') {
                                            let irc_line = cmd_text.to_string();
                                            let _ = ws_tx.send(Message::Text(format!("{}\r\n", irc_line).into())).await;
                                        } else {
                                            let _ = ws_tx.send(Message::Text(
                                                format!("PRIVMSG {} :{}\r\n", target, text).into()
                                            )).await;
                                        }
                                    }
                                    Some(IrcCommand::FetchNicks { buffer_id }) => {
                                        let _ = ws_tx.send(Message::Text(
                                            format!("NAMES {}\r\n", buffer_id).into()
                                        )).await;
                                    }
                                    Some(IrcCommand::FetchBufferList) => {
                                        // IRC discovers buffers via JOINs passively
                                    }
                                    Some(IrcCommand::MarkRead { buffer_id }) => {
                                        if session.negotiated_caps.contains("soju.im/read-marker") {
                                            let now = chrono::Utc::now()
                                                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                                                .to_string();
                                            let _ = ws_tx.send(Message::Text(
                                                format!("MARKREAD {} timestamp={}\r\n", buffer_id, now).into()
                                            )).await;
                                        }
                                    }
                                    Some(IrcCommand::FetchBefore { buffer_id, before_ts }) => {
                                        if session.negotiated_caps.contains("chathistory") {
                                            let _ = ws_tx.send(Message::Text(
                                                format!("CHATHISTORY BEFORE {} timestamp={} 100\r\n", buffer_id, before_ts).into()
                                            )).await;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    connected.store(false, Ordering::Relaxed);
                    if clean { return; }
                    send!(BackendEvent::Disconnected);
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(max_backoff);
        }
    });
}
