//! Ignored nicks and highlight words, for the connections other than WeeChat (which
//! has /ignore and weechat.look.highlight of its own).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IgnoreRule {
    /// A nick, or a pattern with `*` and `?`; case doesn't matter.
    pub nick: String,
    /// Only on this network (`<connection prefix>|<server>`), or on all of them.
    #[serde(default)]
    pub network: Option<String>,
    /// The network's name as the buffer list shows it, for Settings.
    #[serde(default)]
    pub network_label: String,
}

impl IgnoreRule {
    pub fn matches(&self, network: &str, nick: &str) -> bool {
        self.network.as_deref().is_none_or(|n| n == network) && glob_matches(&self.nick.to_lowercase(), &nick.to_lowercase())
    }
}

/// Whether any rule hides `nick` on `network`.
pub fn is_ignored(rules: &[IgnoreRule], network: &str, nick: &str) -> bool {
    rules.iter().any(|r| r.matches(network, nick))
}

/// `*` matches any run of characters and `?` any one.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Whether `text` (lowercase) has one of `words` as a whole word, not inside a longer
/// one: "rust" in "I like rust!" but not in "trusty".
pub fn mentions_any(text: &str, words: &[String]) -> bool {
    words.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).any(|word| {
        text.match_indices(&word).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// The words of Settings' comma-separated field.
pub fn parse_words(text: &str) -> Vec<String> {
    text.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_match_nicks_and_patterns_on_their_network() {
        let everywhere = IgnoreRule { nick: "Spam*".into(), network: None, network_label: String::new() };
        assert!(everywhere.matches("libera|libera", "spambot"));
        assert!(everywhere.matches("x|y", "SPAM"));
        assert!(!everywhere.matches("x|y", "nospam"));
        let one = IgnoreRule { nick: "b?b".into(), network: Some("hackint|hackint".into()), network_label: "hackint".into() };
        assert!(one.matches("hackint|hackint", "Bob"));
        assert!(!one.matches("libera|libera", "bob"));
        assert!(!one.matches("hackint|hackint", "bobby"));
    }

    #[test]
    fn highlight_words_match_whole_words() {
        let words = parse_words(" Rust , weechatrs,,");
        assert_eq!(words, ["Rust", "weechatrs"]);
        assert!(mentions_any("i like rust!", &words));
        assert!(mentions_any("weechatrs 0.9.5 is out", &words));
        assert!(!mentions_any("a trusty tool", &words));
        assert!(!mentions_any("nothing here", &[]));
    }
}
