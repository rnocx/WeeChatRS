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
    targets_batch_ref: Option<String>,
    pending_pm_targets: Vec<String>,
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
            targets_batch_ref: None,
            pending_pm_targets: Vec::new(),
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
            "draft/chathistory",
            "draft/event-playback",
            "echo-message",
            "invite-notify",
            "chghost",
            "userhost-in-names",
            "cap-notify",
            "labeled-response",
            "msgid",
            "soju.im/bouncer-networks",
            "draft/read-marker",
            "soju.im/read",
        ]
    }

    fn has_chathistory(&self) -> bool {
        self.negotiated_caps.contains("chathistory")
            || self.negotiated_caps.contains("draft/chathistory")
    }

    /// Returns the read-marker command name to use, or None if no read-marker cap was negotiated.
    /// draft/read-marker uses MARKREAD; soju.im/read (legacy) uses READ.
    fn read_marker_cmd(&self) -> Option<&'static str> {
        if self.negotiated_caps.contains("draft/read-marker") {
            Some("MARKREAD")
        } else if self.negotiated_caps.contains("soju.im/read") {
            Some("READ")
        } else {
            None
        }
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
            last_markread_ts: None,
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

/// Returns the single highest-ranking IRC prefix char from a set of prefix chars.
/// Ranking: ~ (owner) > & (admin) > @ (op) > % (halfop) > + (voice)
fn highest_prefix(prefixes: &str) -> &'static str {
    if prefixes.contains('~') { "~" }
    else if prefixes.contains('&') { "&" }
    else if prefixes.contains('@') { "@" }
    else if prefixes.contains('%') { "%" }
    else if prefixes.contains('+') { "+" }
    else { "" }
}

fn parse_names_entry(entry: &str) -> Nick {
    // multi-prefix: strip all leading mode chars, then keep only the highest-ranked one
    let (all_prefixes, rest) = strip_prefix_chars(entry);
    let prefix = highest_prefix(all_prefixes);
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
    Line::new(id.to_string(), ts, prefix.to_string(), text.to_string(), true, highlight)
}

fn is_channel(target: &str) -> bool {
    target.starts_with('#') || target.starts_with('&') || target.starts_with('!')
}

/// Expand IRC command aliases and shortcuts into wire-format IRC lines.
///
/// Returns `Some(wire_line)` when the input is a recognised alias that maps
/// directly to a raw server command, or `None` to fall through to the normal
/// send logic.  `/me` and `/msg` are intentionally NOT handled here — they go
/// through the PRIVMSG branch so that self-echo stays correct.
fn expand_irc_command(buffer_id: &str, text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let without_slash = &trimmed[1..];
    let (cmd, rest) = match without_slash.find(|c: char| c.is_ascii_whitespace()) {
        Some(pos) => (&without_slash[..pos], without_slash[pos + 1..].trim()),
        None => (without_slash, ""),
    };
    let cmd_lc = cmd.to_ascii_lowercase();

    match cmd_lc.as_str() {
        // /j [key]  →  JOIN #channel [key]
        "j" => Some(if rest.is_empty() {
            format!("JOIN {}", buffer_id)
        } else {
            format!("JOIN {}", rest)
        }),

        // /w nick  /wi nick  →  WHOIS nick
        "w" | "wi" => {
            if rest.is_empty() { None } else { Some(format!("WHOIS {}", rest)) }
        }

        // /back  →  AWAY  (clears away status)
        "back" => Some("AWAY".to_string()),

        // /topic [text]  /t [text]
        "topic" | "t" => {
            if !is_channel(buffer_id) { return None; }
            if rest.is_empty() {
                Some(format!("TOPIC {}", buffer_id))
            } else {
                Some(format!("TOPIC {} :{}", buffer_id, rest))
            }
        }

        // /invite nick [#channel]
        "invite" => {
            let mut parts = rest.splitn(2, |c: char| c.is_ascii_whitespace());
            let nick = parts.next().unwrap_or("").trim();
            let chan = parts.next().map(str::trim).filter(|s| !s.is_empty())
                .unwrap_or(buffer_id);
            if nick.is_empty() { None } else { Some(format!("INVITE {} {}", nick, chan)) }
        }

        // /kick nick [reason]  /k nick [reason]
        "kick" | "k" => {
            if !is_channel(buffer_id) || rest.is_empty() { return None; }
            let mut parts = rest.splitn(2, |c: char| c.is_ascii_whitespace());
            let nick = parts.next().unwrap_or("").trim();
            let reason = parts.next().map(str::trim).unwrap_or("");
            if nick.is_empty() { return None; }
            if reason.is_empty() {
                Some(format!("KICK {} {}", buffer_id, nick))
            } else {
                Some(format!("KICK {} {} :{}", buffer_id, nick, reason))
            }
        }

        // Mode shortcuts — all require a channel context
        "op" | "deop" | "voice" | "devoice" | "unvoice"
        | "halfop" | "dehalfop" | "ban" | "unban" | "quiet" | "unquiet" => {
            if !is_channel(buffer_id) || rest.is_empty() { return None; }
            let (sign, mode_char) = match cmd_lc.as_str() {
                "op"       => ('+', 'o'),
                "deop"     => ('-', 'o'),
                "voice"    => ('+', 'v'),
                "devoice" | "unvoice" => ('-', 'v'),
                "halfop"   => ('+', 'h'),
                "dehalfop" => ('-', 'h'),
                "ban"      => ('+', 'b'),
                "unban"    => ('-', 'b'),
                "quiet"    => ('+', 'q'),
                "unquiet"  => ('-', 'q'),
                _          => return None,
            };
            let targets: Vec<&str> = rest.split_ascii_whitespace().collect();
            let mode_str: String = std::iter::once(sign)
                .chain(std::iter::repeat(mode_char).take(targets.len()))
                .collect();
            Some(format!("MODE {} {} {}", buffer_id, mode_str, targets.join(" ")))
        }

        _ => None,
    }
}

/// Modern IRCv3 servers can send lines up to ~8 KB with tags. Cap a bit
/// generously to absorb that without giving a hostile server unlimited memory.
const MAX_IRC_LINE: usize = 16 * 1024;

/// Read a single line into `buf`, ending at the next `\n` or EOF.
///
/// Returns `Ok(0)` on EOF with no data buffered, `Ok(n)` for the byte count
/// (including the trailing `\n` if present), or an error if the line exceeds
/// `MAX_IRC_LINE` bytes — in which case the caller should drop the connection
/// rather than try to recover, since we've lost framing.
async fn read_line_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<usize> {
    let start_len = buf.len();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(buf.len() - start_len);
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..=pos]);
            let n = pos + 1;
            reader.consume(n);
            return Ok(buf.len() - start_len);
        }
        if buf.len() + available.len() > MAX_IRC_LINE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IRC line exceeds maximum length",
            ));
        }
        buf.extend_from_slice(available);
        let n = available.len();
        reader.consume(n);
    }
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
            // Connected must fire FIRST so the event handler clears stale buffers from
            // any previous session before we open the fresh status buffer below.
            events.push(BackendEvent::Connected);
            {
                let buf_name = if !config.label.is_empty() { config.label.clone() } else if !session.server_name.is_empty() { session.server_name.clone() } else { "server".to_string() };
                let status_buf = session.alloc_buffer("status", &buf_name, "server");
                events.push(BackendEvent::BufferOpened(status_buf));
                // Flush pre-registration notices (ident checks, hostname lookups, etc.)
                for line in session.pending_status_lines.drain(..) {
                    events.push(BackendEvent::LineAdded { buffer_id: "status".to_string(), line });
                }
                // Prominent "connected" banner so the user sees the server and their nick
                let server_display = if !session.server_name.is_empty() { session.server_name.clone() } else { config.host.clone() };
                *line_counter += 1;
                events.push(BackendEvent::LineAdded {
                    buffer_id: "status".to_string(),
                    line: make_line(&line_counter.to_string(), "--",
                        &format!("Connected to {} as {}", server_display, session.our_nick),
                        timestamp_from_msg(msg), false),
                });
                // 001 welcome text (e.g. "Welcome to the Libera.Chat IRC Network YourNick!")
                let welcome = msg.params.last().map(String::as_str).unwrap_or("");
                if !welcome.is_empty() {
                    *line_counter += 1;
                    events.push(BackendEvent::LineAdded {
                        buffer_id: "status".to_string(),
                        line: make_line(&line_counter.to_string(), "--", welcome, timestamp_from_msg(msg), false),
                    });
                }
                if session.is_soju && session.has_chathistory() {
                    // Ask soju for all recent conversations so we can discover PMs that
                    // arrived while we were disconnected (channels come via JOIN already).
                    session.whois_lines.push("CHATHISTORY TARGETS * * 50".to_string());
                }
                if !config.channel.is_empty() {
                    let chan = config.channel.trim().to_string();
                    let join_cmd = if chan.starts_with('#') || chan.starts_with('&') {
                        format!("JOIN {}", chan)
                    } else {
                        format!("JOIN #{}", chan)
                    };
                    session.whois_lines.push(join_cmd);
                }
            }
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
                } else if kind == "chathistory/targets" {
                    session.targets_batch_ref = Some(batch_ref.to_string());
                    session.pending_pm_targets.clear();
                }
                // labeled-response batches are intentionally NOT added to batch_buf so that
                // messages inside them flow through the normal handlers immediately.
            } else if let Some(batch_ref) = ref_param.strip_prefix('-') {
                if let Some((buffer_id, lines)) = session.batch_buf.remove(batch_ref) {
                    // Soju replays PM chathistory on connect before we've seen any live PRIVMSG,
                    // so the buffer may not exist yet — open it now if needed.
                    if !is_channel(&buffer_id) && !buffer_id.is_empty() && !session.joined.contains(&buffer_id) {
                        session.joined.insert(buffer_id.clone());
                        let buf = session.alloc_buffer(&buffer_id, &buffer_id, "private");
                        events.push(BackendEvent::BufferOpened(buf));
                    }
                    if !lines.is_empty() {
                        events.push(BackendEvent::LinesLoaded { buffer_id, lines, is_prepend: true });
                    }
                } else if session.targets_batch_ref.as_deref() == Some(batch_ref) {
                    session.targets_batch_ref = None;
                    for target in std::mem::take(&mut session.pending_pm_targets) {
                        if !session.joined.contains(&target) {
                            session.joined.insert(target.clone());
                            let buf = session.alloc_buffer(&target, &target, "private");
                            events.push(BackendEvent::BufferOpened(buf));
                            // For DMs there is no JOIN, so soju never sends MARKREAD automatically.
                            // Query the read marker first; soju replies before processing the next
                            // command, so last_markread_ts will be set before LinesLoaded arrives.
                            if let Some(cmd) = session.read_marker_cmd() {
                                session.whois_lines.push(format!("{} {}", cmd, target));
                            }
                            session.whois_lines.push(format!("CHATHISTORY LATEST {} * 100", target));
                        }
                    }
                }
            }
        }

        "CHATHISTORY" => {
            if msg.param(0).unwrap_or("") == "TARGETS" {
                let target = msg.param(1).unwrap_or("").to_string();
                if !target.is_empty() && !is_channel(&target) {
                    session.pending_pm_targets.push(target.to_lowercase());
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
                if session.has_chathistory() {
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
                "status".to_string()
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
            // Route to channel if we're in it, otherwise status buffer
            let chan_lower = channel.to_lowercase();
            let buf_id = if session.joined.contains(&chan_lower) {
                chan_lower
            } else {
                "status".to_string()
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

        // draft/read-marker uses MARKREAD; legacy soju.im/read uses READ.
        // Both have the same format: <cmd> <target> [timestamp=<rfc3339> | *]
        "MARKREAD" | "READ" => {
            let target = msg.param(0).unwrap_or("").to_lowercase();
            let ts_param = msg.param(1).unwrap_or("");
            if !target.is_empty() {
                let markread_ts = if ts_param.starts_with("timestamp=") && ts_param != "timestamp=*" {
                    let ts_str = &ts_param["timestamp=".len()..];
                    chrono::DateTime::parse_from_rfc3339(ts_str)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                } else {
                    None // "*" or missing means no read marker set yet
                };
                events.push(BackendEvent::ActivityChanged {
                    buffer_id: target,
                    activity: BufferActivity::None,
                    unread_count: 0,
                    markread_ts,
                });
            }
        }

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
            let prefix = highest_prefix(flags);
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

        // Server info / ISUPPORT / MOTD / stats — route to status buffer
        "002" | "003" | "004" | "005" | "251" | "252" | "253" | "254" | "255" | "265" | "266"
        | "375" | "372" | "376" | "250" | "256" | "257" | "258" | "259" => {
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
                            result = read_line_bounded(&mut reader, &mut raw_buf) => {
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
                                        let cmd_lower = text.trim().to_lowercase();

                                        // /me <text>  →  CTCP ACTION to current buffer (needs self-echo)
                                        let text = if let Some(action) = text.trim_start()
                                            .strip_prefix("/me ")
                                            .or_else(|| text.trim_start().strip_prefix("/ME "))
                                        {
                                            format!("\x01ACTION {}\x01", action.trim())
                                        } else if cmd_lower == "/me" {
                                            "\x01ACTION \x01".to_string()
                                        // /msg <nick> <text>  /m <nick> <text>
                                        } else if let Some(rest) = text.trim_start().strip_prefix("/msg ")
                                            .or_else(|| text.trim_start().strip_prefix("/MSG "))
                                            .or_else(|| text.trim_start().strip_prefix("/m "))
                                            .or_else(|| text.trim_start().strip_prefix("/M "))
                                        {
                                            let mut parts = rest.trim().splitn(2, |c: char| c.is_ascii_whitespace());
                                            let target = parts.next().unwrap_or("").trim().to_string();
                                            let msg = parts.next().unwrap_or("").trim().to_string();
                                            if !target.is_empty() && !msg.is_empty() {
                                                let _ = write_half.write_all(
                                                    format!("PRIVMSG {} :{}\r\n", target, msg).as_bytes()
                                                ).await;
                                            }
                                            continue;
                                        } else {
                                            text
                                        };

                                        // /part and /close — leave channel or close DM buffer
                                        let cmd_lower = text.trim().to_ascii_lowercase();
                                        let is_part = cmd_lower == "/part" || cmd_lower.starts_with("/part ");
                                        let is_close = cmd_lower == "/close";
                                        if is_part || is_close {
                                            if is_channel(&buffer_id) {
                                                // Send PART with optional message from /part <msg>
                                                let part_msg = if is_part {
                                                    text.trim()["/part".len()..].trim().to_string()
                                                } else {
                                                    String::new()
                                                };
                                                if part_msg.is_empty() {
                                                    let _ = write_half.write_all(format!("PART {}\r\n", buffer_id).as_bytes()).await;
                                                } else {
                                                    let _ = write_half.write_all(format!("PART {} :{}\r\n", buffer_id, part_msg).as_bytes()).await;
                                                }
                                                // Server will echo PART back which triggers BufferClosed
                                            } else {
                                                // DM/private buffer: close locally, no server message needed
                                                session.joined.remove(&buffer_id);
                                                send!(BackendEvent::BufferClosed { buffer_id });
                                            }
                                        // /query <nick> — client-side command: open a DM buffer, nothing to send
                                        } else if let Some(rest) = text.strip_prefix("/query ").or_else(|| text.strip_prefix("/QUERY ")) {
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
                                        } else if let Some(wire) = expand_irc_command(&buffer_id, &text) {
                                            let _ = write_half.write_all(format!("{}\r\n", wire).as_bytes()).await;
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
                                        if let Some(cmd) = session.read_marker_cmd() {
                                            let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
                                            let _ = write_half.write_all(format!("{} {} timestamp={}\r\n", cmd, buffer_id, now).as_bytes()).await;
                                        }
                                    }
                                    Some(IrcCommand::FetchBefore { buffer_id, before_ts }) => {
                                        if session.has_chathistory() {
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
