//! Join, part, quit, kick and nick-change lines for the IRC and Injected backends,
//! coloured the way WeeChat colours them by default, so they look the same on every
//! connection: a green `-->`, a red `<--`, the nick in its own colour, the host in
//! cyan between green brackets.
//!
//! Each builder returns `(prefix, message)`. A reason must already be in the chat's
//! ANSI form (`irc_to_ansi`).

use crate::relay::models::nick_color_slot;

const JOIN: &str = "\x1B[92m";
const LEAVE: &str = "\x1B[91m";
const NETWORK: &str = "\x1B[35m";
const DELIMITER: &str = "\x1B[32m";
const HOST: &str = "\x1B[36m";
const CHANNEL: &str = "\x1B[1m";
const RESET: &str = "\x1B[0m";

fn nick(name: &str) -> String {
    let slot = nick_color_slot(name);
    let code = if slot < 8 { 30 + slot } else { 90 + slot - 8 };
    format!("\x1B[{code}m{name}{RESET}")
}

fn channel(name: &str) -> String {
    format!("{CHANNEL}{name}{RESET}")
}

/// `(text)` with green brackets; empty for empty text.
fn bracketed(text: &str, color: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(" {DELIMITER}({RESET}{color}{text}{RESET}{DELIMITER}){RESET}")
}

/// `nick (user@host) has joined #chan`; `host` may be an account name, or empty.
pub fn join(who: &str, host: &str, chan: &str) -> (String, String) {
    (format!("{JOIN}-->{RESET}"), format!("{}{} has joined {}", nick(who), bracketed(host, HOST), channel(chan)))
}

pub fn part(who: &str, chan: &str, reason: &str) -> (String, String) {
    (format!("{LEAVE}<--{RESET}"), format!("{} has left {}{}", nick(who), channel(chan), bracketed(reason, "")))
}

pub fn quit(who: &str, reason: &str) -> (String, String) {
    (format!("{LEAVE}<--{RESET}"), format!("{} has quit{}", nick(who), bracketed(reason, "")))
}

pub fn kick(kicked: &str, chan: &str, by: &str, reason: &str) -> (String, String) {
    (
        format!("{LEAVE}<--{RESET}"),
        format!("{} was kicked from {} by {}{}", nick(kicked), channel(chan), nick(by), bracketed(reason, "")),
    )
}

pub fn nick_change(old: &str, new: &str) -> (String, String) {
    (format!("{NETWORK}--{RESET}"), format!("{} is now known as {}", nick(old), nick(new)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ansi::ANSIParser;

    fn plain(text: &str) -> String {
        ANSIParser::parse(text).iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn plain_text_reads_as_before() {
        let (p, m) = join("carol", "c@host", "#rust");
        assert_eq!((plain(&p), plain(&m)), ("-->".into(), "carol (c@host) has joined #rust".into()));
        assert!(m.contains("\x1B[36mc@host"), "host in cyan: {m:?}");
        assert_eq!(plain(&join("dave", "", "#rust").1), "dave has joined #rust");
        assert_eq!(plain(&part("carol", "#rust", "bye").1), "carol has left #rust (bye)");
        assert_eq!(plain(&part("carol", "#rust", "").1), "carol has left #rust");
        assert_eq!(plain(&quit("dave", "Ping timeout").1), "dave has quit (Ping timeout)");
        assert_eq!(plain(&kick("bobby", "#rust", "alice", "spam").1), "bobby was kicked from #rust by alice (spam)");
        let (p, m) = nick_change("bob", "bobby");
        assert_eq!((plain(&p), plain(&m)), ("--".into(), "bob is now known as bobby".into()));
        assert!(part("a", "#c", "").0.starts_with("\x1B[91m"));
    }
}
