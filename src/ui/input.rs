use egui::text_edit::TextEditState;
use egui::text::{CCursorRange, CCursor};
use crate::ui::app::{WeeChatApp, CompletionState, RemoteCompletion, shown_elsewhere};
use crate::ui::emoji;

impl WeeChatApp {
    pub(crate) fn perform_completion(&mut self, ctx: &egui::Context, id: egui::Id) {
        let mut new_cursor_char = 0usize;

        if let Some(state) = &mut self.completion {
            // Cycle to next match
            if state.matches.is_empty() { return; }
            state.index = (state.index + 1) % state.matches.len();
            let mut new_text = self.input_text[..state.word_start_idx].to_string();
            new_text.push_str(&state.matches[state.index]);
            new_text.push_str(&state.suffix);
            new_cursor_char = new_text.chars().count();
            self.input_text = new_text;
        } else if self.request_remote_completion(id) {
            // The reply is applied by `apply_remote_completion`.
        } else {
            // Start new completion
            let word_start = self.input_text.rfind(' ').map(|i| i + 1).unwrap_or(0);
            let word = self.input_text[word_start..].to_string();
            if word.is_empty() { return; }

            let matches: Vec<String> = if word.starts_with(':') && word.len() >= 2 {
                emoji::find_matches(&word[1..])
            } else if let Some(typed) = word.strip_prefix('/').filter(|_| word_start == 0) {
                // On IRC, soju, ZNC and Injected: this client's commands and the aliases.
                let typed = typed.to_lowercase();
                let mut names: Vec<String> = LOCAL_COMMANDS.iter().copied().chain(["alias", "unalias"])
                    .map(str::to_string)
                    .chain(self.aliases.keys().cloned())
                    .filter(|n| n.starts_with(&typed))
                    .map(|n| format!("/{n}"))
                    .collect();
                names.sort();
                names.dedup();
                names
            } else {
                match self.selected_buffer_id.as_ref()
                    .and_then(|bid| self.buffers.iter().find(|b| &b.id == bid))
                {
                    Some(buf) => buf.nicks.iter()
                        .filter(|n| n.name.to_lowercase().starts_with(&word.to_lowercase()))
                        .map(|n| n.name.clone())
                        .collect(),
                    None => return,
                }
            };

            if !matches.is_empty() {
                // Nick at line start gets ": ", emoji and mid-line nicks get " "
                let suffix = if !word.starts_with([':', '/']) && word_start == 0 { ": " } else { " " };
                let mut new_text = self.input_text[..word_start].to_string();
                new_text.push_str(&matches[0]);
                new_text.push_str(suffix);
                new_cursor_char = new_text.chars().count();
                self.input_text = new_text;
                self.completion = Some(CompletionState {
                    matches,
                    index: 0,
                    word_start_idx: word_start,
                    suffix: suffix.to_string(),
                });
            }
        }

        if new_cursor_char > 0 {
            set_cursor(ctx, id, new_cursor_char);
        }
    }

    /// Commands (`/…`) on a WeeChat connection are completed by WeeChat itself, which
    /// knows every command, option, server and channel. Returns false when the input is
    /// not a command or the backend can't complete.
    fn request_remote_completion(&mut self, input_id: egui::Id) -> bool {
        if !self.input_text.starts_with('/') || self.input_text.starts_with("//") {
            return false;
        }
        let Some(buf_id) = self.selected_buffer_id.clone() else { return false };
        let Some(conn) = self.connections.iter().find(|c| buf_id.starts_with(&format!("{}/", c.prefix))) else { return false };
        if !conn.client.supports_completion() || !conn.client.is_connected() {
            return false;
        }
        let raw_id = &buf_id[conn.prefix.len() + 1..];
        self.remote_completion_seq += 1;
        let tag = format!("input:{}", self.remote_completion_seq);
        conn.client.complete(raw_id, &self.input_text, self.input_text.chars().count(), &tag);
        self.remote_completion = Some(RemoteCompletion {
            key: format!("{}|{}", conn.prefix, tag),
            text: self.input_text.clone(),
            input_id,
        });
        true
    }

    /// Apply a completion reply from the server once it arrives.
    pub(crate) fn apply_remote_completion(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.remote_completion else { return };
        let Some(reply) = self.completion_replies.remove(&pending.key) else { return };
        let pending = self.remote_completion.take().unwrap();
        if pending.text != self.input_text || !reply.ok {
            return;
        }
        let mut matches = reply.list.clone();
        if reply.context == "command" {
            // Commands handled by this client, which WeeChat doesn't know.
            for cmd in LOCAL_COMMANDS {
                if cmd.starts_with(reply.base_word.as_str()) && !matches.iter().any(|m| m == cmd) {
                    matches.push(cmd.to_string());
                }
            }
            matches.sort();
        }
        let Some(first) = matches.first().cloned() else { return };
        let word_start = self.input_text.char_indices().nth(reply.position_replace)
            .map(|(i, _)| i)
            .unwrap_or(self.input_text.len());
        let suffix = if reply.add_space { " " } else { "" };
        let mut new_text = self.input_text[..word_start].to_string();
        new_text.push_str(&first);
        new_text.push_str(suffix);
        let cursor = new_text.chars().count();
        self.input_text = new_text;
        self.completion = Some(CompletionState {
            matches,
            index: 0,
            word_start_idx: word_start,
            suffix: suffix.to_string(),
        });
        set_cursor(ctx, pending.input_id, cursor);
        ctx.request_repaint();
    }

    pub(crate) fn cycle_history(&mut self, delta: i32, ctx: &egui::Context, id: egui::Id) {
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
                if delta < 0 { Some(self.command_history.len() - 1) } else { None }
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
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(pos))));
            state.store(ctx, id);
        }
    }

    /// `/filtered [toggle|show|hide|default]`: whether this buffer shows filtered lines.
    pub(crate) fn filtered_command(&mut self, arg: &str) {
        use crate::ui::app::FilterMode;
        let Some(id) = self.selected_buffer_id.clone() else { return };
        let showing = self.shows_filtered(&id);
        match arg {
            "" => {}
            "toggle" => self.set_buffer_filter(&id, Some(if showing { FilterMode::Hide } else { FilterMode::Show })),
            "show" => self.set_buffer_filter(&id, Some(FilterMode::Show)),
            "hide" => self.set_buffer_filter(&id, Some(FilterMode::Hide)),
            "default" => self.set_buffer_filter(&id, None),
            _ => {
                self.local_info(&id, "Usage: /filtered [toggle|show|hide|default]");
                return;
            }
        }
        let state = match self.buffer_filter_mode(&id) {
            Some(FilterMode::Show) => "always shown here",
            Some(FilterMode::Hide) => "always hidden here",
            None if self.show_filtered_lines => "shown (follows the setting)",
            None => "hidden (follows the setting)",
        };
        self.local_info(&id, &format!("Filtered lines are {state}"));
    }

    pub(crate) fn cycle_buffer(&mut self, delta: i32) {
        if self.buffers.is_empty() { return; }
        // Mentions comes first, as in the buffer list, which leaves out highmon.
        let order: Vec<String> = Some(crate::ui::mentions::MENTIONS_ID.to_string()).filter(|_| self.show_mentions).into_iter()
            .chain(self.buffers.iter().filter(|b| !shown_elsewhere(b)).map(|b| b.id.clone()))
            .collect();
        let new_id = match &self.selected_buffer_id {
            None => self.buffers[0].id.clone(),
            Some(current) => {
                let Some(idx) = order.iter().position(|id| id == current) else { return };
                order[(idx as i32 + delta).rem_euclid(order.len() as i32) as usize].clone()
            }
        };
        self.select_buffer(new_id);
    }

    pub(crate) fn jump_buffer_by_number(&mut self, n: usize) {
        if n == 0 { return; }
        let visible: Vec<String> = self.buffers.iter()
            .filter(|b| (!b.hidden || self.show_hidden_buffers) && !shown_elsewhere(b))
            .map(|b| b.id.clone())
            .collect();
        if let Some(id) = visible.get(n - 1) {
            let id = id.clone();
            self.select_buffer(id);
        }
    }

    /// Jump to the next buffer that has unread messages or highlights, wrapping around.
    pub(crate) fn jump_next_unread(&mut self) {
        use crate::relay::models::BufferActivity;
        if self.buffers.is_empty() { return; }
        let start = self.selected_buffer_id.as_ref()
            .and_then(|id| self.buffers.iter().position(|b| &b.id == id))
            .unwrap_or(0);
        let len = self.buffers.len();
        for i in 1..=len {
            let idx = (start + i) % len;
            let buf = &self.buffers[idx];
            if buf.hidden && !self.show_hidden_buffers { continue; }
            if buf.activity != BufferActivity::None || buf.unread_count > 0 {
                let id = buf.id.clone();
                self.select_buffer(id);
                return;
            }
        }
    }

    /// `/ignore` lists the ignored nicks, `/ignore <nick>` ignores one on this
    /// buffer's network, `/unignore <nick>` stops ignoring it.
    fn ignore_command(&mut self, buffer_id: &str, network: (String, String), command: &str) {
        let mut words = command.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let nick = words.next().map(str::to_string);
        let text = match (verb, nick) {
            ("/ignore", Some(nick)) => {
                let text = format!("Ignoring {} on {}", nick, network.1);
                self.add_ignore(nick, Some(network));
                text
            }
            ("/unignore", Some(nick)) => {
                let before = self.ignores.len();
                self.ignores.retain(|r| !(r.nick.eq_ignore_ascii_case(&nick) && r.network.as_ref().is_none_or(|n| *n == network.0)));
                self.reapply_ignores();
                if self.ignores.len() < before { format!("No longer ignoring {}", nick) } else { format!("{} isn't ignored here", nick) }
            }
            ("/unignore", None) => "Usage: /unignore <nick>".to_string(),
            _ if self.ignores.is_empty() => "No ignored nicks. /ignore <nick> ignores one on this network.".to_string(),
            _ => {
                let list: Vec<String> = self.ignores.iter()
                    .map(|r| if r.network.is_some() { format!("{} (on {})", r.nick, r.network_label) } else { r.nick.clone() })
                    .collect();
                format!("Ignored: {}", list.join(", "))
            }
        };
        self.local_info(buffer_id, &text);
    }

    /// `/alias`, `/unalias` and the aliases themselves, on IRC, soju, ZNC and Injected:
    /// the commands to send instead of `msg`, none when it was answered here, or
    /// `None` when `msg` isn't one of them.
    fn alias_commands(&mut self, msg: &str) -> Option<Vec<String>> {
        let buffer_id = self.selected_buffer_id.clone()?;
        self.ignore_network(&buffer_id)?;
        let (verb, rest) = msg.trim().split_once(' ').unwrap_or((msg.trim(), ""));
        let rest = rest.trim();
        let (first, args) = rest.split_once(' ').map_or((rest, ""), |(f, a)| (f, a.trim()));
        let text = match verb.to_lowercase().as_str() {
            "/alias" if rest.is_empty() && self.aliases.is_empty() => {
                "No aliases. /alias <name> <command> adds one, such as /alias ns /msg NickServ".to_string()
            }
            "/alias" if rest.is_empty() => {
                let list: Vec<String> = self.aliases.iter().map(|(name, command)| format!("/{name} → {command}")).collect();
                format!("Aliases: {}", list.join(", "))
            }
            "/alias" => match crate::ui::alias::normalize_name(first) {
                None => format!("Can't name an alias {first}"),
                Some(name) if args.is_empty() => match self.aliases.get(&name) {
                    Some(command) => format!("/{name} → {command}"),
                    None => format!("No alias /{name}"),
                },
                Some(name) => {
                    let command = if args.starts_with('/') { args.to_string() } else { format!("/{args}") };
                    let text = format!("Alias /{name} → {command}");
                    self.aliases.insert(name, command);
                    text
                }
            },
            "/unalias" => match crate::ui::alias::normalize_name(first).and_then(|name| self.aliases.remove(&name).map(|_| name)) {
                Some(name) => format!("Removed the alias /{name}"),
                None if first.is_empty() => "Usage: /unalias <name>".to_string(),
                None => format!("No alias {first}"),
            },
            _ => {
                let name = verb.strip_prefix('/').filter(|n| !n.starts_with('/'))?.to_lowercase();
                let template = self.aliases.get(&name)?.clone();
                let nick = self.server_standing(&buffer_id).map(|s| s.nick).unwrap_or_default();
                let channel = self.buffer_by_id(&buffer_id).map(|b| b.name.clone()).unwrap_or_default();
                return Some(crate::ui::alias::expand(&template, rest, &nick, &channel));
            }
        };
        self.local_info(&buffer_id, &text);
        Some(Vec::new())
    }

    pub(crate) fn send_current_message(&mut self) {
        if self.input_text.is_empty() { return; }
        let msg = self.input_text.clone();
        if let Some(commands) = self.alias_commands(&msg) {
            for command in commands {
                self.send_input(command, false);
            }
        } else {
            self.send_input(msg, true);
            return;
        }
        self.record_history(msg);
        self.input_text.clear();
        self.completion = None;
        self.history_index = None;
    }

    fn record_history(&mut self, msg: String) {
        if self.command_history.back().map(|s| s.as_str()) != Some(&msg) {
            self.command_history.push_back(msg);
            if self.command_history.len() > 100 {
                self.command_history.pop_front();
            }
        }
    }

    /// Sends `msg` from the input line, or runs it when this client handles it.
    /// `record` puts it in the input history.
    fn send_input(&mut self, msg: String, record: bool) {
        // `/paste <text>` asks like a paste of several lines, on every connection.
        if msg == "/paste" || msg.starts_with("/paste ") {
            let text = msg.strip_prefix("/paste").unwrap().strip_prefix(' ').unwrap_or("");
            if text.trim().is_empty() { self.paste_error = Some("Usage: /paste <text> (or paste several lines into the input).".into()); }
            else if let Some(id) = self.selected_buffer_id.clone() { self.ask_about_paste(id,text.to_owned()); }
            if record { self.record_history(msg); }
            self.input_text.clear(); self.completion = None; self.history_index = None;
            return;
        }
        let is_command = msg.starts_with('/');

        // Answering means you've caught up: drop the unread divider.
        if !is_command {
            self.typing_submitted = true;
            if let Some(id) = self.selected_buffer_id.clone() {
                if let Some(b) = self.buffer_by_id_mut(&id) { b.visit_start_marker_id = None; }
            }
        }

        // Determine pending_buffer_switch before the borrow via client_for_buffer
        if is_command {
            if msg.starts_with("/query ") {
                self.pending_buffer_switch = msg[7..].split_whitespace().next().map(|s| s.to_string());
            } else if msg.starts_with("/join ") || msg.starts_with("/j ") {
                let after = if msg.starts_with("/j ") { &msg[3..] } else { &msg[6..] };
                self.pending_buffer_switch = after.split_whitespace().next().map(|s| s.to_string());
            } else if let Some((id, network)) = (msg.trim() == "/ignore" || msg.starts_with("/ignore ") || msg.starts_with("/unignore "))
                .then(|| self.selected_buffer_id.clone()).flatten()
                .and_then(|id| Some((id.clone(), self.ignore_network(&id)?)))
            {
                // IRC, soju, ZNC and Injected; WeeChat's own /ignore gets the others.
                self.ignore_command(&id, network, msg.trim());
                self.input_text.clear();
                self.completion = None;
                self.history_index = None;
                return;
            } else if let Some(query) = (msg.trim() == "/list").then_some("").or_else(|| msg.strip_prefix("/list ")) {
                // IRC, soju, ZNC and Injected: the channel list browser. WeeChat has its own.
                let opened = self.selected_buffer_id.clone().is_some_and(|id| self.open_channel_list(&id, query));
                if opened {
                    if record { self.record_history(msg); }
                    self.input_text.clear();
                    self.completion = None;
                    self.history_index = None;
                    return;
                }
            } else if msg.trim() == "/filtered" || msg.starts_with("/filtered ") {
                // Not /filter, which WeeChat has for its own filters.
                self.filtered_command(msg.trim().strip_prefix("/filtered").unwrap_or("").trim());
                self.input_text.clear();
                self.completion = None;
                self.history_index = None;
                return;
            } else if msg.trim() == "/mentions" {
                self.toggle_mentions();
                self.input_text.clear();
                self.completion = None;
                self.history_index = None;
                return;
            } else if msg.trim() == "/sysinfo" || msg.trim() == "/systeminfo" {
                if let Some(buffer_id) = self.selected_buffer_id.clone() {
                    let tx = self.sysinfo_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let info = crate::ui::sysinfo::gather();
                        let text = crate::ui::sysinfo::format_line(&info);
                        let _ = tx.send((buffer_id, text));
                    });
                }
                self.input_text.clear();
                self.completion = None;
                self.history_index = None;
                return;
            } else if msg.trim() == "/share" || msg.starts_with("/share ") {
                // `/share` picks a file; `/share <path>` uploads that file. The link is
                // posted to the current buffer when the upload finishes.
                if let Some(buffer_id) = self.selected_buffer_id.clone() {
                    let arg = msg.trim().strip_prefix("/share").unwrap_or("").trim();
                    match share_path(arg) {
                        None => self.start_upload(buffer_id, None),
                        Some(path) if path.is_file() => self.start_upload(buffer_id, Some(path)),
                        Some(path) => self.file_share_error = Some(format!("No such file: {}", path.display())),
                    }
                }
                self.input_text.clear();
                self.completion = None;
                self.history_index = None;
                return;
            } else if msg.trim() == "/np" || msg.starts_with("/np ") {
                if let Some(buffer_id) = self.selected_buffer_id.clone() {
                    self.np_error = None;
                    let custom = msg.trim().strip_prefix("/np").map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.to_string());
                    let tx = self.np_tx.clone();
                    tokio::spawn(async move {
                        let text = if let Some(c) = custom {
                            format!("{} 🎵", c)
                        } else {
                            let np = match crate::ui::np::get_now_playing().await {
                                Ok(np) => np,
                                Err(e) => {
                                    let _ = tx.send(Err(e));
                                    return;
                                }
                            };
                            let mut s = np.track;
                            if !np.source.is_empty() {
                                s.push_str(&format!(" @ {}", np.source));
                            }
                            s.push_str(" 🎵");
                            if !np.url.is_empty() {
                                s.push_str(&format!(" {}", np.url));
                            }
                            s
                        };
                        let _ = tx.send(Ok((buffer_id, text)));
                    });
                }
                self.input_text.clear();
                self.completion = None;
                self.history_index = None;
                return;
            }
        }

        if let Some(buffer_id) = self.selected_buffer_id.clone() {
            if let Some((client, raw_id)) = self.client_for_buffer(&buffer_id) {
                client.send_message(&raw_id, &msg);
                if is_command {
                    client.fetch_buffer_list();
                }
            }
            if opens_fset(&msg) {
                // Show fset like /join shows the channel: now if it's open, else once it is.
                let prefix = buffer_id.split('/').next().unwrap_or("").to_string();
                let fset = self.buffers.iter()
                    .find(|b| b.id.starts_with(&format!("{prefix}/")) && crate::ui::fset::is_fset_buffer(&b.full_name))
                    .map(|b| b.id.clone());
                match fset {
                    Some(id) => self.select_buffer(id),
                    None => self.pending_buffer_switch = Some("fset.fset".to_string()),
                }
            }
        }

        if record {
            self.record_history(msg);
        }

        self.input_text.clear();
        self.completion = None;
        self.history_index = None;
    }

    pub(crate) fn send_command(&mut self, command: &str) {
        if command.starts_with("/query ") {
            self.pending_buffer_switch = Some(command[7..].trim().to_string());
        } else if command.starts_with("/join ") {
            self.pending_buffer_switch = Some(command[6..].trim().to_string());
        }

        if let Some(buffer_id) = self.selected_buffer_id.clone() {
            if let Some((client, raw_id)) = self.client_for_buffer(&buffer_id) {
                client.send_message(&raw_id, command);
                client.fetch_buffer_list();
            }
        }
    }

    pub(crate) fn send_command_to_buffer(&mut self, buffer_id: &str, command: &str) {
        if let Some((client, raw_id)) = self.client_for_buffer(buffer_id) {
            client.send_message(&raw_id, command);
            client.fetch_buffer_list();
        }
    }
}

/// `/fset …`, and `/set` without a value (fset takes over listing options).
fn opens_fset(command: &str) -> bool {
    let mut words = command.split_whitespace();
    match words.next() {
        Some("/fset") => true,
        Some("/set") => words.count() <= 1,
        _ => false,
    }
}

/// Commands this client handles itself (see `send_current_message`).
const LOCAL_COMMANDS: &[&str] = &["mentions", "np", "paste", "share", "sysinfo"];

/// The input line's id, so its focus and cursor can be found.
pub(crate) fn input_id() -> egui::Id {
    egui::Id::unique("chat_input")
}

pub(crate) fn set_cursor(ctx: &egui::Context, id: egui::Id, char_index: usize) {
    if let Some(mut state) = TextEditState::load(ctx, id) {
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(char_index))));
        state.store(ctx, id);
    }
}

/// Parse the argument of `/share`: empty means "pick a file". Accepts a quoted path and a
/// leading `~/` for the home directory; on macOS/Linux also shell-escaped spaces
/// (`My\\ File.png`, as pasted from a terminal), and on Windows `~\\` and backslash
/// separators (which must not be treated as escapes).
fn share_path(arg: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    share_path_for(arg, cfg!(windows), home.as_deref().map(std::path::Path::new))
}

fn share_path_for(arg: &str, windows: bool, home: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    let unquoted = arg
        .strip_prefix('"').and_then(|a| a.strip_suffix('"'))
        .or_else(|| arg.strip_prefix('\'').and_then(|a| a.strip_suffix('\'')));
    let path = match unquoted {
        Some(p) => p.to_string(),
        None if !windows => arg.replace("\\ ", " "),
        None => arg.to_string(),
    };
    let home_rest = path.strip_prefix("~/").or_else(|| if windows { path.strip_prefix("~\\") } else { None });
    if let (Some(rest), Some(home)) = (home_rest, home) {
        return Some(home.join(rest));
    }
    Some(std::path::PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    #[test]
    fn fset_opening_commands() {
        use super::opens_fset;
        assert!(opens_fset("/fset"));
        assert!(opens_fset("/fset f:irc"));
        assert!(opens_fset("/set"));
        assert!(opens_fset("/set irc.look.*"));
        assert!(!opens_fset("/set irc.look.color_nicks off"));
        assert!(!opens_fset("/settings"));
        assert!(!opens_fset("hello /fset"));
    }

    use super::{share_path, share_path_for};
    use std::path::{Path, PathBuf};

    #[test]
    fn share_path_unix() {
        let home = Some(Path::new("/home/me"));
        let p = |a| share_path_for(a, false, home);
        assert_eq!(p(""), None);
        assert_eq!(p("   "), None);
        assert_eq!(p("/tmp/a.png"), Some(PathBuf::from("/tmp/a.png")));
        assert_eq!(p("\"/tmp/my file.png\""), Some(PathBuf::from("/tmp/my file.png")));
        assert_eq!(p("'/tmp/my file.png'"), Some(PathBuf::from("/tmp/my file.png")));
        assert_eq!(p("/tmp/My\\ File.png"), Some(PathBuf::from("/tmp/My File.png")));
        assert_eq!(p("~/x.txt"), Some(PathBuf::from("/home/me").join("x.txt")));
    }

    #[test]
    fn share_path_windows() {
        let home = Some(Path::new("C:\\Users\\me"));
        let p = |a| share_path_for(a, true, home);
        // Backslashes are separators on Windows, never escapes.
        assert_eq!(p("C:\\Users\\me\\pic.png"), Some(PathBuf::from("C:\\Users\\me\\pic.png")));
        assert_eq!(p("\"C:\\My Files\\a b.png\""), Some(PathBuf::from("C:\\My Files\\a b.png")));
        assert_eq!(p("~\\Desktop\\a.png"), Some(Path::new("C:\\Users\\me").join("Desktop\\a.png")));
        assert_eq!(p("~/Desktop/a.png"), Some(Path::new("C:\\Users\\me").join("Desktop/a.png")));
    }

    #[test]
    fn share_path_uses_real_home() {
        assert!(share_path("~/x").is_some());
    }
}
