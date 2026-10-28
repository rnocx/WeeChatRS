use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct IrcMessage {
    pub tags: HashMap<String, Option<String>>,
    pub prefix: Option<String>,
    pub command: String,
    pub params: Vec<String>,
}

impl IrcMessage {
    pub fn parse(raw: &str) -> Option<Self> {
        let mut s = raw.trim_end_matches('\r').trim_end_matches('\n');

        let mut tags = HashMap::new();
        if s.starts_with('@') {
            let (tags_str, rest) = s[1..].split_once(' ')?;
            s = rest.trim_start();
            for tag in tags_str.split(';') {
                if tag.is_empty() { continue; }
                if let Some((k, v)) = tag.split_once('=') {
                    tags.insert(k.to_string(), Some(unescape_tag_value(v)));
                } else {
                    tags.insert(tag.to_string(), None);
                }
            }
        }

        let prefix = if s.starts_with(':') {
            let (pfx, rest) = s[1..].split_once(' ')?;
            s = rest.trim_start();
            Some(pfx.to_string())
        } else {
            None
        };

        let (command, mut rest) = match s.split_once(' ') {
            Some((c, r)) => (c.to_uppercase(), r.trim_start()),
            None => (s.to_uppercase(), ""),
        };

        let mut params = Vec::new();
        while !rest.is_empty() {
            if let Some(trailing) = rest.strip_prefix(':') {
                params.push(trailing.to_string());
                break;
            }
            match rest.split_once(' ') {
                Some((p, r)) => {
                    params.push(p.to_string());
                    rest = r.trim_start();
                }
                None => {
                    params.push(rest.to_string());
                    break;
                }
            }
        }

        Some(IrcMessage { tags, prefix, command, params })
    }

    /// Extract the nick from `nick!user@host` prefix.
    pub fn nick(&self) -> Option<&str> {
        self.prefix.as_deref()?.split('!').next()
    }

    /// Get an IRCv3 message tag value.
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key)?.as_deref()
    }

    /// The last (trailing) parameter, or the nth param.
    pub fn param(&self, n: usize) -> Option<&str> {
        self.params.get(n).map(String::as_str)
    }
}

/// Attributes in the message-tag format (`key=value;key2=value2`), as used by
/// soju.im/bouncer-networks. A key without a value maps to an empty string.
pub fn parse_attributes(s: &str) -> HashMap<String, String> {
    s.split(';')
        .filter(|a| !a.is_empty())
        .map(|a| match a.split_once('=') {
            Some((k, v)) => (k.to_string(), unescape_tag_value(v)),
            None => (a.to_string(), String::new()),
        })
        .collect()
}

fn unescape_tag_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(':')  => out.push(';'),
                Some('s')  => out.push(' '),
                Some('\\') => out.push('\\'),
                Some('r')  => out.push('\r'),
                Some('n')  => out.push('\n'),
                Some(c)    => out.push(c),
                None       => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_are_unescaped() {
        let attrs = parse_attributes("name=Libera\\sChat;host=irc.libera.chat;tls");
        assert_eq!(attrs["name"], "Libera Chat");
        assert_eq!(attrs["host"], "irc.libera.chat");
        assert_eq!(attrs["tls"], "");
        assert!(parse_attributes("").is_empty());
    }
}
