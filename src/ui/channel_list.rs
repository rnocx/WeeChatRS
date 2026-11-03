//! The channel list (`/list`) of a network: an entry of its own in the network's group,
//! filled as the channels arrive, with search, a minimum of users, sorting, and Join.
//! IRC, soju, ZNC and Injected send the server's LIST replies. On WeeChat the rows come
//! from WeeChat's own list buffer (`irc.list_<server>`, WeeChat 4.1 and later), which
//! the buffer list then leaves out.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use egui::{Color32, FontId, Label, RichText, Sense, Vec2};

use crate::relay::models::{Buffer, ListedChannel};
use crate::ui::app::WeeChatApp;
use crate::ui::icons;

/// Channels smaller than this aren't asked for unless the user lowers it.
pub(crate) const DEFAULT_MIN_USERS: u32 = 10;

/// The kind of the list's entry in the buffer list; nothing is sent for it. Not
/// "list": WeeChat's own /list buffer has that type, and must stay a WeeChat buffer.
pub(crate) const KIND: &str = "weechatrs-channel-list";

/// WeeChat's own /list buffer, whose rows feed the app's channel list.
pub(crate) fn is_weechat_list_buffer(buffer: &Buffer) -> bool {
    let name = buffer.full_name.rsplit_once('/').map_or(buffer.full_name.as_str(), |(_, n)| n);
    buffer.kind == "list" && name.starts_with("irc.list_")
}

/// A row of WeeChat's list buffer, colours stripped: the channel padded to a column,
/// its users, its modes in brackets when it has any, then the topic.
/// `#AngelicFM                1  [+lmnrstLP] Channel has moved`
pub(crate) fn parse_weechat_list_row(text: &str) -> Option<ListedChannel> {
    let text = text.trim_start();
    let (name, rest) = text.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (users, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let users = users.parse().ok()?;
    let rest = rest.trim_start();
    let topic = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((_, topic)) => topic,
        None => rest,
    };
    Some(ListedChannel { name: name.to_string(), users, topic: topic.trim().to_string() })
}

/// How many channels WeeChat's list buffer shows, from its title:
/// "86 channels (total: 86) | Filter: * | …" → 86.
fn weechat_list_shown(title: &str) -> Option<usize> {
    title.split_once(" channel")?.0.trim().parse().ok()
}

/// The list's buffer id, from its network's server buffer id.
pub(crate) fn list_id(server_buffer_id: &str) -> String {
    format!("{server_buffer_id}@list")
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Users,
    Name,
}

struct Entry {
    name: String,
    name_lower: String,
    users: u32,
    topic: String,
    topic_lower: String,
}

pub(crate) struct ChannelList {
    /// The network's server buffer, which the list is asked through and joins go to.
    server_buffer_id: String,
    network: String,
    entries: Vec<Entry>,
    /// Where each channel is in `entries`, by lowercase name.
    index: HashMap<String, usize>,
    /// Asked for and not finished.
    loading: bool,
    /// Cancelled: what still arrives of this list is left out.
    cancelled: bool,
    /// The server is still sending a list, cancelled or not; a new one waits for it.
    busy: bool,
    fetched_at: Option<DateTime<Local>>,
    fetched_min: u32,
    error: Option<String>,
    query: String,
    min_users: u32,
    hide_joined: bool,
    sort: Sort,
    selected: Option<String>,
    /// The entries shown, filtered and sorted, and what they were made from.
    view: Vec<usize>,
    view_key: Option<(String, u32, bool, bool, usize, usize)>,
}

impl ChannelList {
    fn new(server_buffer_id: String, network: String, query: String) -> Self {
        Self {
            server_buffer_id,
            network,
            entries: Vec::new(),
            index: HashMap::new(),
            loading: false,
            cancelled: false,
            busy: false,
            fetched_at: None,
            fetched_min: DEFAULT_MIN_USERS,
            error: None,
            query,
            min_users: DEFAULT_MIN_USERS,
            hide_joined: false,
            sort: Sort::Users,
            selected: None,
            view: Vec::new(),
            view_key: None,
        }
    }

    /// Adds what the server sent; `done` with the last part. A channel that's already
    /// there is updated: WeeChat prints a row again when it changes.
    pub(crate) fn receive(&mut self, channels: Vec<ListedChannel>, done: bool, error: Option<String>) {
        if !self.cancelled {
            for c in channels {
                let topic = WeeChatApp::strip_ansi(&c.topic);
                let entry = Entry { name_lower: c.name.to_lowercase(), topic_lower: topic.to_lowercase(), name: c.name, users: c.users, topic };
                match self.index.get(&entry.name_lower) {
                    Some(&i) => self.entries[i] = entry,
                    None => {
                        self.index.insert(entry.name_lower.clone(), self.entries.len());
                        self.entries.push(entry);
                    }
                }
            }
            self.view_key = None;
        }
        if done {
            self.busy = false;
            if !self.cancelled {
                self.loading = false;
                self.fetched_at = Some(Local::now());
                self.error = error;
            }
        }
    }

    /// A new list is coming: what was there goes.
    fn restart(&mut self) {
        self.entries.clear();
        self.index.clear();
        self.view_key = None;
        self.loading = true;
        self.cancelled = false;
        self.busy = true;
        self.error = None;
    }

    /// Recomputes the shown entries when what they depend on changed.
    fn refresh_view(&mut self, joined: &HashSet<String>) {
        let key = (self.query.trim().to_lowercase(), self.min_users, self.hide_joined, self.sort == Sort::Name, self.entries.len(), joined.len());
        if self.view_key.as_ref() == Some(&key) {
            return;
        }
        let q = &key.0;
        let mut view: Vec<usize> = self.entries.iter().enumerate()
            .filter(|(_, e)| e.users >= self.min_users)
            .filter(|(_, e)| !self.hide_joined || !joined.contains(&e.name_lower))
            .filter(|(_, e)| q.is_empty() || e.name_lower.contains(q.as_str()) || e.topic_lower.contains(q.as_str()))
            .map(|(i, _)| i)
            .collect();
        match self.sort {
            Sort::Users => view.sort_by(|&a, &b| self.entries[b].users.cmp(&self.entries[a].users).then_with(|| self.entries[a].name_lower.cmp(&self.entries[b].name_lower))),
            Sort::Name => view.sort_by(|&a, &b| self.entries[a].name_lower.trim_start_matches(['#', '&']).cmp(self.entries[b].name_lower.trim_start_matches(['#', '&']))),
        }
        self.view = view;
        self.view_key = Some(key);
    }
}

/// What the view asks the app to do once it's drawn.
enum Action {
    Fetch,
    Join(String),
    Open(String),
}

impl WeeChatApp {
    /// `/list [search]` in a buffer of an IRC, soju, ZNC or Injected network: shows the
    /// network's channel list, fetching it the first time. False where the backend
    /// has its own (WeeChat).
    pub(crate) fn open_channel_list(&mut self, buffer_id: &str, query: &str) -> bool {
        let Some(server) = self.network_server_buffer(buffer_id) else { return false };
        let supported = self.client_for_buffer(&server.id).is_some_and(|(c, _)| c.supports_channel_list());
        if !supported {
            return false;
        }
        let id = list_id(&server.id);
        if self.buffer_by_id(&id).is_none() {
            let entry = Buffer {
                id: id.clone(),
                name: "Channel list".into(),
                full_name: format!("{}/channel list", server.full_name),
                kind: KIND.into(),
                server: server.server.clone(),
                number: server.number,
                ..Default::default()
            };
            // Last in its network's group.
            let at = self.buffers.iter().rposition(|b| crate::ui::app::same_group(b, &server)).map_or(self.buffers.len(), |i| i + 1);
            self.buffers.insert(at, entry);
            self.rebuild_buffer_idx();
        }
        let fresh = !self.channel_lists.contains_key(&id);
        let list = self.channel_lists.entry(id.clone())
            .or_insert_with(|| ChannelList::new(server.id.clone(), server.name.clone(), String::new()));
        if !query.trim().is_empty() {
            list.query = query.trim().to_string();
        }
        if fresh {
            self.fetch_channel_list(&id);
        }
        self.select_buffer(id);
        true
    }

    /// The server buffer of `buffer_id`'s network.
    pub(crate) fn network_server_buffer(&self, buffer_id: &str) -> Option<Buffer> {
        let buffer = self.buffer_by_id(buffer_id)?;
        self.buffers.iter()
            .find(|b| b.kind == "server" && crate::ui::app::same_group(b, buffer))
            .cloned()
    }

    fn fetch_channel_list(&mut self, id: &str) {
        // WeeChat's /list asks for every channel, and the app filters them.
        let weechat = self.connections.iter()
            .any(|c| id.starts_with(&format!("{}/", c.prefix)) && c.backend_type == crate::ui::app::BackendType::WeeChat);
        let Some(list) = self.channel_lists.get_mut(id) else { return };
        if list.busy {
            return;
        }
        list.restart();
        list.fetched_min = if weechat { 0 } else { list.min_users };
        let (server, min) = (list.server_buffer_id.clone(), list.min_users);
        let client = self.client_for_buffer(&server);
        log::debug!("channel list fetch for {server} (raw {:?}), at least {min} users", client.as_ref().map(|(_, raw)| raw));
        if let Some((client, raw)) = client {
            client.list_channels(&raw, min);
        }
    }

    /// Part of a network's channel list arrived, for its server buffer `server_buffer_id`.
    pub(crate) fn channel_list_received(&mut self, server_buffer_id: &str, channels: Vec<ListedChannel>, done: bool, error: Option<String>) {
        log::debug!("channel list for {server_buffer_id}: {} channels, done {done}, error {error:?}, open {}",
            channels.len(), self.channel_lists.contains_key(&list_id(server_buffer_id)));
        if let Some(list) = self.channel_lists.get_mut(&list_id(server_buffer_id)) {
            list.receive(channels, done, error);
        }
    }

    /// The open channel list that WeeChat's list buffer `buffer_id` feeds, by its id.
    fn list_fed_by(&self, buffer_id: &str) -> Option<String> {
        let buffer = self.buffer_by_id(buffer_id).filter(|b| is_weechat_list_buffer(b))?;
        let server = self.network_server_buffer(&buffer.id)?;
        let id = list_id(&server.id);
        self.channel_lists.contains_key(&id).then_some(id)
    }

    /// A row of WeeChat's list buffer arrived.
    pub(crate) fn weechat_list_row(&mut self, buffer_id: &str, message: &str) {
        let Some(id) = self.list_fed_by(buffer_id) else { return };
        if let (Some(list), Some(row)) = (self.channel_lists.get_mut(&id), parse_weechat_list_row(&WeeChatApp::strip_ansi(message))) {
            list.receive(vec![row], false, None);
        }
    }

    /// WeeChat cleared its list buffer: a new list is on its way.
    pub(crate) fn weechat_list_cleared(&mut self, buffer_id: &str) {
        let Some(id) = self.list_fed_by(buffer_id) else { return };
        if let Some(list) = self.channel_lists.get_mut(&id) {
            list.restart();
        }
    }

    /// WeeChat retitled its list buffer with how many channels it shows: once they're
    /// all here, the list is complete.
    pub(crate) fn weechat_list_title(&mut self, buffer_id: &str, title: &str) {
        let Some(id) = self.list_fed_by(buffer_id) else { return };
        if let (Some(list), Some(shown)) = (self.channel_lists.get_mut(&id), weechat_list_shown(title)) {
            if list.busy && list.entries.len() >= shown {
                list.receive(Vec::new(), true, None);
            }
        }
    }

    /// Closes a channel list's entry.
    pub(crate) fn close_channel_list(&mut self, id: &str) {
        // On WeeChat its list buffer closes with it.
        if let Some(server) = self.channel_lists.get(id).and_then(|l| self.buffer_by_id(&l.server_buffer_id)).cloned() {
            let weechat_list = self.buffers.iter()
                .find(|b| is_weechat_list_buffer(b) && crate::ui::app::same_group(b, &server))
                .map(|b| b.id.clone());
            if let Some(buffer) = weechat_list {
                self.send_command_to_buffer(&buffer, "/buffer close");
            }
        }
        self.channel_lists.remove(id);
        self.buffers.retain(|b| b.id != id);
        self.rebuild_buffer_idx();
        if self.selected_buffer_id.as_deref() == Some(id) {
            let next = self.buffers.first().map(|b| b.id.clone());
            self.selected_buffer_id = None;
            if let Some(next) = next {
                self.select_buffer(next);
            }
        }
    }

    /// The channel list in the chat area.
    pub(crate) fn draw_channel_list(&mut self, ui: &mut egui::Ui, id: &str, accent: Color32, muted: Color32, bar_fill: Color32) {
        // The network's joined channels, by lowercase name, and their buffers.
        let Some(server) = self.channel_lists.get(id).map(|l| l.server_buffer_id.clone()) else { return };
        let Some(server_buffer) = self.buffer_by_id(&server).cloned() else { return };
        let joined: HashMap<String, String> = self.buffers.iter()
            .filter(|b| b.kind == "channel" && crate::ui::app::same_group(b, &server_buffer))
            .map(|b| (b.name.to_lowercase(), b.id.clone()))
            .collect();
        let joined_names: HashSet<String> = joined.keys().cloned().collect();
        let font = FontId::new(self.font_size, crate::ui::fonts::chat_family());
        let row_h = ui.text_style_height(&egui::TextStyle::Body).max(self.font_size) + 14.0;
        let mut action = None;
        let list = self.channel_lists.get_mut(id).expect("checked above");
        list.refresh_view(&joined_names);
        let red = Color32::from_rgb(220, 80, 80);

        // Search, filters and Refresh.
        egui::Frame::NONE.fill(bar_fill).inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(muted));
                ui.add(egui::TextEdit::singleline(&mut list.query).hint_text("Search names and topics…").desired_width(260.0));
                ui.add_space(8.0);
                ui.label("At least");
                ui.add(egui::DragValue::new(&mut list.min_users).range(0..=100_000).speed(1));
                ui.label("users");
                ui.add_space(8.0);
                ui.checkbox(&mut list.hide_joined, "Hide channels I'm in");
                ui.add_space(8.0);
                if list.loading {
                    if ui.button("Cancel").clicked() {
                        list.cancelled = true;
                        list.loading = false;
                        list.fetched_at = Some(Local::now());
                    }
                } else if ui.add_enabled(!list.busy, egui::Button::new("Refresh")).clicked() {
                    action = Some(Action::Fetch);
                }
                if !list.loading && list.min_users < list.fetched_min {
                    ui.label(RichText::new(format!("Refresh to include channels under {} users", list.fetched_min)).small().color(accent));
                }
            });
        });
        ui.add_space(-1.0);
        ui.separator();

        if list.loading && list.entries.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.spinner();
                ui.add_space(10.0);
                ui.label(RichText::new(format!("Fetching the channel list from {}…", list.network)).strong());
                if list.fetched_min > 1 {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!(
                        "Channels with {} or more users are asked for where the server can tell; lower the number and refresh for smaller ones.",
                        list.fetched_min,
                    )).color(muted));
                }
            });
        } else if let Some(error) = list.error.clone().filter(|_| list.entries.is_empty()) {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new(format!("The server sent no list: {error}")).color(red));
            });
        } else {
            // Column headers; a click sorts.
            let name_w = 220.0;
            let users_w = 80.0;
            let button_w = 120.0;
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let arrow = |on: bool, mark: &str| if on { format!(" {mark}") } else { String::new() };
                let name = RichText::new(format!("CHANNEL{}", arrow(list.sort == Sort::Name, "▲"))).small().strong()
                    .color(if list.sort == Sort::Name { accent } else { muted });
                if ui.add_sized([name_w, 20.0], egui::Button::new(name).frame(false)).clicked() {
                    list.sort = Sort::Name;
                }
                let users = RichText::new(format!("USERS{}", arrow(list.sort == Sort::Users, "▼"))).small().strong()
                    .color(if list.sort == Sort::Users { accent } else { muted });
                if ui.add_sized([users_w, 20.0], egui::Button::new(users).frame(false)).clicked() {
                    list.sort = Sort::Users;
                }
                ui.add_space(12.0);
                ui.label(RichText::new("TOPIC").small().strong().color(muted));
            });
            ui.separator();

            let footer_h = 30.0;
            let rows = list.view.len();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height((ui.available_height() - footer_h).max(row_h))
                .show_rows(ui, row_h, rows, |ui, range| {
                    for &i in &list.view[range] {
                        let entry = &list.entries[i];
                        let (rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
                        let selected = list.selected.as_deref() == Some(entry.name.as_str());
                        if selected || row.hovered() {
                            ui.painter().rect_filled(rect, 4.0, accent.linear_multiply(if selected { 0.18 } else { 0.08 }));
                        }
                        let mut cells = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(16.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        cells.add_sized([name_w, row_h], Label::new(RichText::new(&entry.name).font(font.clone()).strong()).truncate());
                        cells.allocate_ui_with_layout(Vec2::new(users_w, row_h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(entry.users.to_string()).font(font.clone()));
                        });
                        cells.add_space(12.0);
                        let topic_w = (cells.available_width() - button_w - 12.0).max(40.0);
                        cells.add_sized([topic_w, row_h], Label::new(RichText::new(&entry.topic).color(muted)).truncate());
                        cells.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            match joined.get(&entry.name_lower) {
                                Some(buffer) => {
                                    if ui.button(RichText::new("Joined · Open").small()).clicked() {
                                        action = Some(Action::Open(buffer.clone()));
                                    }
                                }
                                None => {
                                    let join = egui::Button::new(RichText::new("Join").small().strong().color(Color32::WHITE)).fill(accent);
                                    if ui.add(join).clicked() {
                                        action = Some(Action::Join(entry.name.clone()));
                                    }
                                }
                            }
                        });
                        if row.clicked() {
                            list.selected = Some(entry.name.clone());
                        }
                        if row.double_clicked() {
                            action = Some(match joined.get(&entry.name_lower) {
                                Some(buffer) => Action::Open(buffer.clone()),
                                None => Action::Join(entry.name.clone()),
                            });
                        }
                    }
                    if rows == 0 {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("No channels match. Try fewer letters or a lower user count.").color(muted));
                        });
                    }
                });

            // Showing, fetched, and the state of the fetch.
            ui.separator();
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                if list.loading {
                    ui.spinner();
                    ui.label(RichText::new(format!("Receiving… {} channels so far", list.entries.len())).small().color(muted));
                } else {
                    let floor = if list.fetched_min > 1 { format!(" with {}+ users", list.fetched_min) } else { String::new() };
                    ui.label(RichText::new(format!("Showing {} of {} channels{floor}", rows, list.entries.len())).small().color(muted));
                    if let Some(at) = list.fetched_at {
                        ui.label(RichText::new(format!("· fetched {} from {}", at.format("%H:%M"), list.network)).small().color(muted));
                    }
                    if let Some(error) = &list.error {
                        ui.label(RichText::new(format!("· {error}")).small().color(red));
                    }
                }
            });
        }

        // Enter joins the selected channel.
        if action.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().egui_wants_keyboard_input() {
            if let Some(name) = list.selected.clone() {
                action = Some(match joined.get(&name.to_lowercase()) {
                    Some(buffer) => Action::Open(buffer.clone()),
                    None => Action::Join(name),
                });
            }
        }
        match action {
            Some(Action::Fetch) => self.fetch_channel_list(id),
            Some(Action::Join(name)) => self.send_command_to_buffer(&server, &format!("/join {name}")),
            Some(Action::Open(buffer)) => self.select_buffer(buffer),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(name: &str, users: u32, topic: &str) -> ListedChannel {
        ListedChannel { name: name.into(), users, topic: topic.into() }
    }

    #[test]
    fn filters_and_sorts_what_arrived() {
        let mut list = ChannelList::new("c/status".into(), "libera".into(), String::new());
        list.loading = true;
        list.busy = true;
        list.receive(vec![channel("#rust", 842, "The Rust language"), channel("#go-nuts", 241, "Go")], false, None);
        list.receive(vec![channel("#linux", 1412, "Linux help \x1B[1mbold\x1B[0m"), channel("#tiny", 3, "")], true, None);
        assert!(!list.loading && !list.busy);
        let names = |l: &ChannelList| l.view.iter().map(|&i| l.entries[i].name.clone()).collect::<Vec<_>>();
        list.refresh_view(&HashSet::new());
        assert_eq!(names(&list), ["#linux", "#rust", "#go-nuts"]);
        assert_eq!(list.entries[2].topic, "Linux help bold");
        list.query = "LANG".into();
        list.refresh_view(&HashSet::new());
        assert_eq!(names(&list), ["#rust"]);
        list.query.clear();
        list.sort = Sort::Name;
        list.hide_joined = true;
        list.min_users = 0;
        list.refresh_view(&["#rust".to_string()].into());
        assert_eq!(names(&list), ["#go-nuts", "#linux", "#tiny"]);
    }

    #[test]
    fn reads_weechats_list_rows_and_title() {
        // As WeeChat 4.10 sent them over the relay; the first is the selected line.
        let rows = [
            "\u{1b}[97m\u{1b}[48;5;24m#201337                   1  [+nt] ",
            "#AngelicFM                1  [+lmnrstLP] Channel has moved: Please move to #WHR",
            "#plain                   42  No modes, a topic",
        ];
        let parsed: Vec<ListedChannel> = rows.iter().filter_map(|r| parse_weechat_list_row(&WeeChatApp::strip_ansi(r))).collect();
        assert_eq!(parsed, [
            channel("#201337", 1, ""),
            channel("#AngelicFM", 1, "Channel has moved: Please move to #WHR"),
            channel("#plain", 42, "No modes, a topic"),
        ]);
        assert_eq!(parse_weechat_list_row("not a row"), None);
        assert_eq!(weechat_list_shown("86 channels (total: 86) | Filter: * | Sort: ~name2"), Some(86));
        assert_eq!(weechat_list_shown("1 channel (total: 1) | Filter: *"), Some(1));
        assert_eq!(weechat_list_shown("Receiving…"), None);
    }

    #[test]
    fn a_row_printed_again_is_updated_not_added() {
        let mut list = ChannelList::new("c/status".into(), "nixsec".into(), String::new());
        list.restart();
        list.receive(vec![channel("#a", 5, "old")], false, None);
        list.receive(vec![channel("#A", 6, "new")], false, None);
        assert_eq!(list.entries.len(), 1);
        assert_eq!((list.entries[0].users, list.entries[0].topic.as_str()), (6, "new"));
        list.restart();
        assert!(list.entries.is_empty() && list.busy && list.loading);
    }

    #[test]
    fn a_cancelled_list_keeps_what_came_and_ignores_the_rest() {
        let mut list = ChannelList::new("c/status".into(), "libera".into(), String::new());
        list.loading = true;
        list.busy = true;
        list.receive(vec![channel("#a", 20, "")], false, None);
        list.cancelled = true;
        list.loading = false;
        list.receive(vec![channel("#b", 20, "")], true, None);
        assert_eq!(list.entries.len(), 1);
        assert!(!list.busy, "a new fetch can start once the server is done");
    }
}
