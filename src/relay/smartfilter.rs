//! WeeChat's smart filter, for the connections that have no server-side filters
//! (IRC, soju, ZNC, Injected). A join, part, quit or nick change is hidden unless
//! the nick spoke lately, so a busy channel shows who just left a conversation and
//! nothing about the lurkers.
//!
//! - A join is hidden. If the nick speaks within the delay, the join is shown after
//!   all, where it happened.
//! - A part or quit is shown only if the nick spoke within the delay before it.
//! - A nick change is shown by the same rule, and the nick's record follows it.
//! - A kick is always shown, and so is anything about our own nick.
//!
//! Hidden lines are the ones with `displayed` unset, so "show filtered lines"
//! brings them back.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

use crate::relay::models::{Line, PresenceKind};

/// What one buffer's smart filter remembers.
#[derive(Default)]
pub struct SmartFilter {
    /// When each nick (lowercase) last spoke.
    spoke: HashMap<String, DateTime<Utc>>,
    /// Each nick's join line that might yet be shown: its id and time.
    joins: HashMap<String, (String, DateTime<Utc>)>,
}

/// Enough nicks that a long-running busy channel doesn't grow without bound.
const KEEP_NICKS: usize = 5000;

impl SmartFilter {
    /// Looks at a line as it arrives, in order. A presence line gets its `displayed`
    /// set. A chat line is noted, and may bring back its author's join: the id of
    /// that earlier line is returned, to be set displayed. `ours` is true when the
    /// presence line is about our own nick.
    pub fn apply(&mut self, line: &mut Line, ours: bool, delay: Duration) -> Option<String> {
        let at = line.timestamp;
        let Some(presence) = line.presence.clone() else {
            let speaker = crate::ui::app::is_chat_prefix(&line.plain_prefix)
                .then(|| crate::ui::app::prefix_nick(line).map(|(n, _)| n.to_lowercase()))
                .flatten()?;
            self.spoke.insert(speaker.clone(), at);
            if self.spoke.len() > KEEP_NICKS {
                self.spoke.retain(|_, t| at - *t <= delay);
            }
            return self.joins.remove(&speaker).filter(|(_, t)| at - *t <= delay).map(|(id, _)| id);
        };
        let nick = presence.nick.to_lowercase();
        match presence.kind {
            PresenceKind::Join => {
                line.displayed = ours;
                if !ours {
                    self.joins.retain(|_, (_, t)| at - *t <= delay);
                    self.joins.insert(nick, (line.id.clone(), at));
                }
            }
            PresenceKind::Part | PresenceKind::Quit => {
                self.joins.remove(&nick);
                line.displayed = ours || self.spoke_lately(&nick, at, delay);
            }
            PresenceKind::Kick => {
                self.joins.remove(&nick);
                line.displayed = true;
            }
            PresenceKind::Nick(new) => {
                line.displayed = ours || self.spoke_lately(&nick, at, delay);
                let new = new.to_lowercase();
                if let Some(t) = self.spoke.remove(&nick) {
                    self.spoke.insert(new.clone(), t);
                }
                if let Some(j) = self.joins.remove(&nick) {
                    self.joins.insert(new, j);
                }
            }
        }
        None
    }

    fn spoke_lately(&self, nick: &str, at: DateTime<Utc>, delay: Duration) -> bool {
        self.spoke.get(nick).is_some_and(|t| at - *t <= delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn said(id: &str, secs: i64, nick: &str) -> Line {
        Line::new(id.into(), at(secs), nick.into(), "hi".into(), true, false)
    }

    fn presence(id: &str, secs: i64, kind: PresenceKind, nick: &str) -> Line {
        Line::new(id.into(), at(secs), "-->".into(), format!("{nick} did something"), false, false).with_presence(kind, nick)
    }

    const DELAY: Duration = Duration::minutes(5);

    #[test]
    fn a_join_stays_hidden_until_the_nick_speaks() {
        let mut f = SmartFilter::default();
        let mut join = presence("1", 0, PresenceKind::Join, "Carol");
        assert_eq!(f.apply(&mut join, false, DELAY), None);
        assert!(!join.displayed);
        // Spoke within the delay, in any case: the join comes back.
        assert_eq!(f.apply(&mut said("2", 120, "carol"), false, DELAY), Some("1".into()));
        // Only once.
        assert_eq!(f.apply(&mut said("3", 130, "carol"), false, DELAY), None);
    }

    #[test]
    fn a_late_first_message_leaves_the_join_hidden() {
        let mut f = SmartFilter::default();
        f.apply(&mut presence("1", 0, PresenceKind::Join, "carol"), false, DELAY);
        assert_eq!(f.apply(&mut said("2", 6 * 60, "carol"), false, DELAY), None);
    }

    #[test]
    fn a_leave_shows_only_after_recent_speech() {
        let mut f = SmartFilter::default();
        f.apply(&mut said("1", 0, "carol"), false, DELAY);
        let mut soon = presence("2", 4 * 60, PresenceKind::Quit, "Carol");
        f.apply(&mut soon, false, DELAY);
        assert!(soon.displayed);
        let mut late = presence("3", 10 * 60, PresenceKind::Part, "carol");
        f.apply(&mut late, false, DELAY);
        assert!(!late.displayed);
        let mut lurker = presence("4", 10 * 60, PresenceKind::Quit, "bob");
        f.apply(&mut lurker, false, DELAY);
        assert!(!lurker.displayed);
    }

    #[test]
    fn a_nick_change_follows_the_speaker() {
        let mut f = SmartFilter::default();
        f.apply(&mut said("1", 0, "carol"), false, DELAY);
        let mut change = presence("2", 60, PresenceKind::Nick("carol_".into()), "carol");
        f.apply(&mut change, false, DELAY);
        assert!(change.displayed);
        // The new nick has the old one's record.
        let mut quit = presence("3", 120, PresenceKind::Quit, "carol_");
        f.apply(&mut quit, false, DELAY);
        assert!(quit.displayed);
    }

    #[test]
    fn kicks_and_our_own_lines_are_always_shown() {
        let mut f = SmartFilter::default();
        let mut kick = presence("1", 0, PresenceKind::Kick, "bob");
        f.apply(&mut kick, false, DELAY);
        assert!(kick.displayed);
        let mut join = presence("2", 1, PresenceKind::Join, "me");
        f.apply(&mut join, true, DELAY);
        assert!(join.displayed);
        let mut part = presence("3", 2, PresenceKind::Part, "me");
        f.apply(&mut part, true, DELAY);
        assert!(part.displayed);
    }

    #[test]
    fn lines_without_a_nick_do_nothing() {
        let mut f = SmartFilter::default();
        let mut info = Line::new("1".into(), at(0), "--".into(), "Topic is x".into(), true, false);
        assert_eq!(f.apply(&mut info, false, DELAY), None);
        assert!(info.displayed);
    }
}
