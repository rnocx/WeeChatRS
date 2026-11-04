//! The nick list's rows: nicks, or nicks under rank headings (Owners, Admins…).

use crate::relay::models::Nick;
use egui::Color32;

/// Rank headings in order, by the prefix symbol that puts a nick under them, and the
/// colour of the heading and the symbol.
const RANKS: [(char, &str, Color32); 5] = [
    ('~', "Owners", Color32::from_rgb(205, 55, 55)),
    ('&', "Admins", Color32::from_rgb(85, 130, 240)),
    ('@', "Operators", Color32::from_rgb(70, 190, 95)),
    ('%', "Half-ops", Color32::from_rgb(110, 200, 245)),
    ('+', "Voiced", Color32::from_rgb(240, 150, 50)),
];

pub enum Row<'a> {
    /// A heading, how many nicks are under it, and its colour (None for Users).
    Heading(&'static str, usize, Option<Color32>),
    Nick(&'a Nick),
}

/// The nick's highest mode symbol, shown before it: `~`, `@`, `+`… WeeChat gives
/// nicks without one a blank prefix.
pub fn mode_symbol(nick: &Nick) -> Option<char> {
    nick.prefix.chars().next().filter(|c| !c.is_whitespace())
}

/// The colour of a mode symbol, for the ranks above; None for one this list doesn't know.
pub fn mode_color(symbol: char) -> Option<Color32> {
    RANKS.iter().find(|(s, _, _)| *s == symbol).map(|(_, _, c)| *c)
}

/// The heading a nick goes under: its highest prefix's, or "Users" without one (or
/// with a symbol this list doesn't know).
fn rank(nick: &Nick) -> usize {
    nick.prefix.chars().next()
        .and_then(|c| RANKS.iter().position(|(symbol, _, _)| *symbol == c))
        .unwrap_or(RANKS.len())
}

/// The rows to draw. Grouped, each rank that has nicks gets a heading, in rank
/// order; the nicks keep their order within it.
pub fn rows(nicks: &[Nick], grouped: bool) -> Vec<Row<'_>> {
    if !grouped {
        return nicks.iter().map(Row::Nick).collect();
    }
    let mut groups: Vec<Vec<&Nick>> = vec![Vec::new(); RANKS.len() + 1];
    for nick in nicks {
        groups[rank(nick)].push(nick);
    }
    let mut rows = Vec::with_capacity(nicks.len() + groups.len());
    for (i, group) in groups.into_iter().enumerate().filter(|(_, g)| !g.is_empty()) {
        let (title, color) = RANKS.get(i).map_or(("Users", None), |(_, t, c)| (*t, Some(*c)));
        rows.push(Row::Heading(title, group.len(), color));
        rows.extend(group.into_iter().map(Row::Nick));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nick(prefix: &str, name: &str) -> Nick {
        Nick { name: name.into(), prefix: prefix.into(), color_ansi: String::new(), away: false }
    }

    fn shown(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| match r {
            Row::Heading(t, n, _) => format!("{t} {n}"),
            Row::Nick(n) => format!("{}{}", n.prefix, n.name),
        }).collect()
    }

    #[test]
    fn grouped_by_highest_prefix() {
        let nicks = [nick("@", "op"), nick("", "bob"), nick("~@", "boss"), nick("+", "v"), nick("", "amy"), nick("!", "odd")];
        assert_eq!(shown(&rows(&nicks, true)), [
            "Owners 1", "~@boss",
            "Operators 1", "@op",
            "Voiced 1", "+v",
            "Users 3", "bob", "amy", "!odd",
        ]);
        assert_eq!(shown(&rows(&nicks, false)), ["@op", "bob", "~@boss", "+v", "amy", "!odd"]);
        assert_eq!(mode_symbol(&nick("~@", "boss")), Some('~'));
        assert_eq!(mode_symbol(&nick(" ", "bob")), None);
        assert!(mode_color('@').is_some() && mode_color('!').is_none());
    }
}
