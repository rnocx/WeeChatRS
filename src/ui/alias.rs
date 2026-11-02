//! Command aliases for IRC, soju, ZNC and Injected (WeeChat has its own /alias):
//! `/alias ns /msg NickServ` makes `/ns identify pw` send `/msg NickServ identify pw`.

/// An alias name as stored: lowercase, without the slash.
pub fn normalize_name(name: &str) -> Option<String> {
    let name = name.trim().trim_start_matches('/').to_lowercase();
    (!name.is_empty() && !name.contains(char::is_whitespace) && !matches!(name.as_str(), "alias" | "unalias")).then_some(name)
}

/// What an alias runs, given the words after it: `$1`…`$9` are single words, `$1-`
/// the words from the first on, `$*` all of them, `$nick` our nick and `$channel`
/// the buffer. Without any `$` word the arguments go at the end. `;` separates
/// commands (`\;` is a semicolon).
pub fn expand(template: &str, args: &str, nick: &str, channel: &str) -> Vec<String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    let from = |n: usize| words.get(n.saturating_sub(1)..).map(|w| w.join(" ")).unwrap_or_default();
    let uses_args = template.contains("$*") || (1..=9).any(|n| template.contains(&format!("${n}")));
    let mut commands = Vec::new();
    for part in split_commands(template) {
        let mut out = String::new();
        let mut rest = part.as_str();
        while let Some(at) = rest.find('$') {
            out.push_str(&rest[..at]);
            rest = &rest[at + 1..];
            let digit = rest.chars().next().and_then(|c| c.to_digit(10)).filter(|&d| d > 0);
            if let Some(n) = digit {
                if rest[1..].starts_with('-') {
                    out.push_str(&from(n as usize));
                    rest = &rest[2..];
                } else {
                    out.push_str(words.get(n as usize - 1).copied().unwrap_or_default());
                    rest = &rest[1..];
                }
            } else if let Some(r) = rest.strip_prefix('*') {
                out.push_str(&from(1));
                rest = r;
            } else if let Some(r) = rest.strip_prefix("nick") {
                out.push_str(nick);
                rest = r;
            } else if let Some(r) = rest.strip_prefix("channel") {
                out.push_str(channel);
                rest = r;
            } else {
                out.push('$');
            }
        }
        out.push_str(rest);
        let mut command = out.trim().to_string();
        if !uses_args && !args.trim().is_empty() {
            command = format!("{command} {}", args.trim());
        }
        if !command.is_empty() {
            commands.push(command);
        }
    }
    commands
}

/// The commands of an alias, split at `;` but not at `\;`.
fn split_commands(template: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&';') => {
                chars.next();
                parts.last_mut().expect("never empty").push(';');
            }
            ';' => parts.push(String::new()),
            c => parts.last_mut().expect("never empty").push(c),
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_lowercase_words() {
        assert_eq!(normalize_name("/NS").as_deref(), Some("ns"));
        assert_eq!(normalize_name("two words"), None);
        assert_eq!(normalize_name("/alias"), None);
        assert_eq!(normalize_name(""), None);
    }

    #[test]
    fn arguments_go_where_the_alias_says() {
        assert_eq!(expand("/msg NickServ", "identify pw", "me", "#c"), ["/msg NickServ identify pw"]);
        assert_eq!(expand("/msg NickServ", "", "me", "#c"), ["/msg NickServ"]);
        assert_eq!(expand("/kick $channel $1 $2-", "bob go away", "me", "#rust"), ["/kick #rust bob go away"]);
        assert_eq!(expand("/me waves at $*", "you all", "me", "#c"), ["/me waves at you all"]);
        assert_eq!(expand("/msg $1 hi from $nick", "", "me", "#c"), ["/msg  hi from me"]);
        assert_eq!(expand("cost $5", "a", "me", "#c"), ["cost"]);
        assert_eq!(expand("price in $USD", "", "me", "#c"), ["price in $USD"]);
    }

    #[test]
    fn semicolons_separate_commands() {
        assert_eq!(
            expand("/join $1; /msg ChanServ op $1", "#x", "me", ""),
            ["/join #x", "/msg ChanServ op #x"],
        );
        assert_eq!(expand(r"/me thinks\; then acts", "", "me", ""), ["/me thinks; then acts"]);
    }
}
