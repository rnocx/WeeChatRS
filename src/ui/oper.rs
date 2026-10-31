//! IRC operator commands for the right-click menus. Servers don't list their oper
//! commands, so the set comes from the server software `004` names.
//!
//! Everything goes out as `/quote`, which WeeChat, the IRC backend and Injected all
//! send as it is. Commands that hurt someone (kill, bans) aren't sent: they fill the
//! input line, for a reason and Enter.

/// The family of IRC server software, for which oper commands exist.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Ircd {
    Unreal,
    InspIRCd,
    /// Solanum, Charybdis, ircd-seven, ratbox, hybrid: K-lines, no SA* commands.
    Charybdis,
    Generic,
}

impl Ircd {
    pub(crate) fn from_software(software: &str) -> Self {
        let s = software.to_ascii_lowercase();
        if s.contains("unreal") {
            Ircd::Unreal
        } else if s.contains("inspircd") {
            Ircd::InspIRCd
        } else if ["solanum", "charybdis", "ircd-seven", "ratbox", "hybrid"].iter().any(|n| s.contains(n)) {
            Ircd::Charybdis
        } else {
            Ircd::Generic
        }
    }

    fn has_sa_commands(self) -> bool {
        matches!(self, Ircd::Unreal | Ircd::InspIRCd)
    }
}

/// A menu entry: `command` is sent when `run`, otherwise it fills the input line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OperAction {
    pub label: &'static str,
    pub command: String,
    pub run: bool,
}

/// `None` stands for a separator.
type Entries = Vec<Option<OperAction>>;

fn run(label: &'static str, command: String) -> Option<OperAction> {
    Some(OperAction { label, command, run: true })
}

fn fill(label: &'static str, command: String) -> Option<OperAction> {
    Some(OperAction { label, command, run: false })
}

/// For a nick in the nick list; `channel` is the channel shown, if any.
pub(crate) fn nick_actions(ircd: Ircd, nick: &str, channel: Option<&str>) -> Entries {
    let mut entries = vec![
        run("Full whois", format!("/quote WHOIS {nick} {nick}")),
        None,
        fill("Kill…", format!("/quote KILL {nick} :")),
    ];
    entries.extend(match ircd {
        Ircd::Unreal => vec![
            fill("G-line…", format!("/quote GLINE {nick} 1d :")),
            fill("GZ-line…", format!("/quote GZLINE {nick} 1d :")),
            fill("Shun…", format!("/quote SHUN {nick} 1d :")),
        ],
        Ircd::InspIRCd => vec![
            fill("G-line…", format!("/quote GLINE {nick} 1d :")),
            fill("Z-line…", format!("/quote ZLINE {nick} 1d :")),
            fill("Shun…", format!("/quote SHUN {nick} 1d :")),
        ],
        Ircd::Charybdis => vec![fill("K-line…", format!("/quote KLINE 1440 {nick} :"))],
        Ircd::Generic => vec![fill("K-line…", format!("/quote KLINE {nick} :"))],
    });
    if ircd.has_sa_commands() {
        entries.push(None);
        entries.push(fill("Force join…", format!("/quote SAJOIN {nick} #")));
        if let Some(channel) = channel {
            entries.push(fill("Force part…", format!("/quote SAPART {nick} {channel}")));
        }
        entries.push(fill("Force nick…", format!("/quote SANICK {nick} ")));
        entries.push(fill("Change host…", format!("/quote CHGHOST {nick} ")));
    }
    entries
}

/// For a channel in the buffer list; `nick` is ours.
pub(crate) fn channel_actions(ircd: Ircd, channel: &str, nick: &str) -> Entries {
    if !ircd.has_sa_commands() {
        return Vec::new();
    }
    vec![
        run("Op me", format!("/quote SAMODE {channel} +o {nick}")),
        fill("Set mode…", format!("/quote SAMODE {channel} ")),
        fill("Set topic…", format!("/quote SATOPIC {channel} :")),
    ]
}

/// For the server buffer; `nick` is ours.
pub(crate) fn server_actions(nick: &str) -> Entries {
    vec![
        run("Users", "/quote LUSERS".to_string()),
        run("Links", "/quote LINKS".to_string()),
        fill("Stats…", "/quote STATS ".to_string()),
        None,
        fill("Rehash…", "/quote REHASH".to_string()),
        run("Deoper", format!("/quote MODE {nick} -o")),
    ]
}

/// Whether our prefix in a channel's nick list lets us moderate it (halfop and up).
pub(crate) fn can_moderate(prefix: &str) -> bool {
    prefix.contains(['~', '&', '@', '%'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_software_picks_the_command_set() {
        assert_eq!(Ircd::from_software("UnrealIRCd-6.1.8"), Ircd::Unreal);
        assert_eq!(Ircd::from_software("InspIRCd-4"), Ircd::InspIRCd);
        assert_eq!(Ircd::from_software("solanum-1.0-dev"), Ircd::Charybdis);
        assert_eq!(Ircd::from_software("ircd-hybrid-8.2.43"), Ircd::Charybdis);
        assert_eq!(Ircd::from_software(""), Ircd::Generic);
    }

    #[test]
    fn harmful_commands_only_fill_the_input() {
        for ircd in [Ircd::Unreal, Ircd::InspIRCd, Ircd::Charybdis, Ircd::Generic] {
            for action in nick_actions(ircd, "bob", Some("#rust")).into_iter().flatten() {
                if action.run {
                    assert_eq!(action.label, "Full whois");
                }
                assert!(action.command.starts_with("/quote "), "{action:?}");
            }
        }
        let kline = nick_actions(Ircd::Charybdis, "bob", None).into_iter().flatten().find(|a| a.label == "K-line…").unwrap();
        assert_eq!(kline.command, "/quote KLINE 1440 bob :");
        assert!(channel_actions(Ircd::Charybdis, "#rust", "me").is_empty());
    }

    #[test]
    fn halfops_and_up_moderate() {
        assert!(can_moderate("@"));
        assert!(can_moderate("%"));
        assert!(can_moderate("~"));
        assert!(!can_moderate("+"));
        assert!(!can_moderate(""));
    }
}
