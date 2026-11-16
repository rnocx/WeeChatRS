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
    let mut entries = vec![run("Full whois", format!("/quote WHOIS {nick} {nick}"))];
    // Read-only lookups.
    match ircd {
        Ircd::Unreal => entries.push(run("User IP", format!("/quote USERIP {nick}"))),
        Ircd::InspIRCd => entries.push(run("Check user", format!("/quote CHECK {nick}"))),
        Ircd::Charybdis => entries.push(run("Trace connection", format!("/quote ETRACE {nick}"))),
        Ircd::Generic => {}
    }
    entries.push(None);
    entries.push(fill("Kill…", format!("/quote KILL {nick} :")));
    entries.extend(match ircd {
        Ircd::Unreal => vec![
            fill("G-line…", format!("/quote GLINE {nick} 1d :")),
            fill("GZ-line…", format!("/quote GZLINE {nick} 1d :")),
            fill("Shun…", format!("/quote SHUN {nick} 1d :")),
            fill("Temporary shun…", format!("/quote TEMPSHUN {nick} :")),
        ],
        Ircd::InspIRCd => vec![
            fill("G-line…", format!("/quote GLINE {nick} 1d :")),
            fill("Z-line…", format!("/quote ZLINE {nick} 1d :")),
            fill("Shun…", format!("/quote SHUN {nick} 1d :")),
        ],
        Ircd::Charybdis => vec![
            fill("K-line…", format!("/quote KLINE 1440 {nick} :")),
            fill("D-line…", format!("/quote DLINE 1440 {nick} :")),
        ],
        Ircd::Generic => vec![fill("K-line…", format!("/quote KLINE {nick} :"))],
    });
    if ircd.has_sa_commands() {
        entries.push(None);
        entries.push(fill("Force join…", format!("/quote SAJOIN {nick} #")));
        if let Some(channel) = channel {
            entries.push(fill("Force part…", format!("/quote SAPART {nick} {channel}")));
        }
        entries.push(fill("Force nick…", format!("/quote SANICK {nick} ")));
        if ircd == Ircd::InspIRCd {
            entries.push(fill("Force quit…", format!("/quote SAQUIT {nick} :")));
            if let Some(channel) = channel {
                entries.push(fill("Force kick…", format!("/quote SAKICK {channel} {nick} :")));
            }
        }
        entries.push(None);
        entries.push(fill("Change host…", format!("/quote CHGHOST {nick} ")));
        entries.push(fill("Change ident…", format!("/quote CHGIDENT {nick} ")));
        entries.push(fill("Change real name…", format!("/quote CHGNAME {nick} :")));
    }
    entries
}

/// For a channel in the buffer list; `nick` is ours.
pub(crate) fn channel_actions(ircd: Ircd, channel: &str, nick: &str) -> Entries {
    let mut entries = Vec::new();
    match ircd {
        Ircd::InspIRCd => entries.push(run("Check channel", format!("/quote CHECK {channel}"))),
        Ircd::Charybdis => entries.push(run("Trace channel", format!("/quote CHANTRACE {channel}"))),
        _ => {}
    }
    if ircd.has_sa_commands() {
        entries.push(run("Op me", format!("/quote SAMODE {channel} +o {nick}")));
        entries.push(fill("Set mode…", format!("/quote SAMODE {channel} ")));
        entries.push(fill("Set topic…", format!("/quote SATOPIC {channel} :")));
    }
    // Closing a channel to everyone.
    match ircd {
        Ircd::InspIRCd => {
            entries.push(None);
            entries.push(fill("Close channel…", format!("/quote CBAN {channel} 1d :")));
        }
        Ircd::Charybdis => {
            entries.push(None);
            entries.push(fill("Close channel…", format!("/quote RESV {channel} :")));
            entries.push(fill("Reopen channel…", format!("/quote UNRESV {channel}")));
        }
        _ => {}
    }
    entries
}

/// For the server buffer; `nick` is ours.
pub(crate) fn server_actions(ircd: Ircd, nick: &str) -> Entries {
    let mut entries = vec![
        run("Users", "/quote LUSERS".to_string()),
        run("Links", "/quote LINKS".to_string()),
        run("Map", "/quote MAP".to_string()),
        run("Uptime", "/quote STATS u".to_string()),
        run("Operators online", match ircd {
            Ircd::Unreal => "/quote STATS O",
            _ => "/quote STATS p",
        }.to_string()),
        fill("Stats…", "/quote STATS ".to_string()),
        fill("Trace…", "/quote TRACE ".to_string()),
        None,
    ];
    // Messages to the other opers.
    match ircd {
        Ircd::Unreal => {
            entries.push(fill("Globops…", "/quote GLOBOPS :".to_string()));
            entries.push(fill("Locops…", "/quote LOCOPS :".to_string()));
        }
        Ircd::InspIRCd => entries.push(fill("Globops…", "/quote GLOBOPS :".to_string())),
        Ircd::Charybdis => {
            entries.push(fill("Operwall…", "/quote OPERWALL :".to_string()));
            entries.push(fill("Locops…", "/quote LOCOPS :".to_string()));
        }
        Ircd::Generic => {}
    }
    entries.push(fill("Wallops…", "/quote WALLOPS :".to_string()));
    entries.push(fill("Server notices…", format!("/quote MODE {nick} +s +")));
    entries.push(None);
    // Taking bans back, and testing what a mask matches.
    match ircd {
        Ircd::Unreal => {
            entries.push(fill("Remove G-line…", "/quote GLINE -".to_string()));
            entries.push(fill("Remove shun…", "/quote SHUN -".to_string()));
        }
        Ircd::InspIRCd => {
            // No duration removes a line.
            entries.push(fill("Remove G-line…", "/quote GLINE ".to_string()));
            entries.push(fill("Remove Z-line…", "/quote ZLINE ".to_string()));
            entries.push(fill("Remove shun…", "/quote SHUN ".to_string()));
        }
        Ircd::Charybdis => {
            entries.push(fill("Test mask…", "/quote TESTLINE ".to_string()));
            entries.push(fill("Remove K-line…", "/quote UNKLINE ".to_string()));
            entries.push(fill("Remove D-line…", "/quote UNDLINE ".to_string()));
        }
        Ircd::Generic => entries.push(fill("Remove K-line…", "/quote UNKLINE ".to_string())),
    }
    entries.push(None);
    entries.push(fill("Connect server…", "/quote CONNECT ".to_string()));
    entries.push(fill("Disconnect server…", "/quote SQUIT ".to_string()));
    entries.push(fill("Rehash…", "/quote REHASH".to_string()));
    entries.push(run("Deoper", format!("/quote MODE {nick} -o")));
    entries
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

    const ALL: [Ircd; 4] = [Ircd::Unreal, Ircd::InspIRCd, Ircd::Charybdis, Ircd::Generic];

    /// What the menus may send at a click: lookups and our own state, never a command
    /// that does something to another user, a channel or the network.
    const SAFE_TO_RUN: &[&str] = &[
        "Full whois", "User IP", "Check user", "Trace connection", "Check channel", "Trace channel", "Op me",
        "Users", "Links", "Map", "Uptime", "Operators online", "Deoper",
    ];

    #[test]
    fn harmful_commands_only_fill_the_input() {
        for ircd in ALL {
            let all = nick_actions(ircd, "bob", Some("#rust")).into_iter()
                .chain(channel_actions(ircd, "#rust", "me"))
                .chain(server_actions(ircd, "me"))
                .flatten();
            for action in all {
                if action.run {
                    assert!(SAFE_TO_RUN.contains(&action.label), "{action:?} runs at a click");
                }
                assert!(action.command.starts_with("/quote "), "{action:?}");
                assert!(!action.command.contains('\n'), "{action:?}");
            }
        }
        let kline = nick_actions(Ircd::Charybdis, "bob", None).into_iter().flatten().find(|a| a.label == "K-line…").unwrap();
        assert_eq!(kline.command, "/quote KLINE 1440 bob :");
    }

    #[test]
    fn each_server_family_gets_its_own_commands() {
        let labels = |entries: Entries| -> Vec<&'static str> { entries.into_iter().flatten().map(|a| a.label).collect() };
        let inspircd = labels(channel_actions(Ircd::InspIRCd, "#rust", "me"));
        assert!(inspircd.contains(&"Check channel") && inspircd.contains(&"Close channel…") && inspircd.contains(&"Op me"));
        let solanum = labels(channel_actions(Ircd::Charybdis, "#rust", "me"));
        assert!(solanum.contains(&"Trace channel") && solanum.contains(&"Close channel…") && !solanum.contains(&"Op me"));
        assert!(labels(channel_actions(Ircd::Generic, "#rust", "me")).is_empty());
        assert!(labels(nick_actions(Ircd::InspIRCd, "bob", Some("#rust"))).contains(&"Force kick…"));
        assert!(!labels(nick_actions(Ircd::Unreal, "bob", Some("#rust"))).contains(&"Force kick…"));
        assert!(labels(server_actions(Ircd::Unreal, "me")).contains(&"Globops…"));
        assert!(labels(server_actions(Ircd::Charybdis, "me")).contains(&"Operwall…"));
        assert!(labels(server_actions(Ircd::Generic, "me")).contains(&"Remove K-line…"));
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
