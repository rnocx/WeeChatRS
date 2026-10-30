//! ZNC: read markers kept by the app (ZNC has none), and the networks panel of the
//! Connections window, which manages the user's networks through `*controlpanel`.

use crate::relay::backend::ZncNetwork;
use crate::relay::models::Buffer;
use crate::ui::app::WeeChatApp;
use crate::ui::icons;
use egui::{Color32, RichText};

/// The Connections window's ZNC panel: the add-network form and a pending delete.
pub(crate) struct ZncUiState {
    adding: Option<String>,
    name: String,
    host: String,
    port: String,
    tls: bool,
    password: String,
    confirm_delete: Option<(String, String)>,
}

impl Default for ZncUiState {
    fn default() -> Self {
        Self {
            adding: None,
            name: String::new(),
            host: String::new(),
            port: "6697".to_string(),
            tls: true,
            password: String::new(),
            confirm_delete: None,
        }
    }
}

/// The ZNC user a profile logs in as: its username without `@client` or `/network`.
pub(crate) fn znc_user(username: &str, nick: &str) -> String {
    let login = if username.is_empty() { nick } else { username };
    login.split(['@', '/']).next().unwrap_or(login).to_string()
}

/// A buffer's read-marker key: connection, network group and buffer name, which stay
/// the same from one session to the next (buffer ids of ZNC networks may not).
fn read_marker_key(conn_prefix: &str, buffer: &Buffer) -> String {
    format!("{}|{}|{}", conn_prefix, buffer.server, buffer.name.to_lowercase())
}

impl WeeChatApp {
    fn is_znc_connection(&self, conn_prefix: &str) -> bool {
        self.connections.iter().any(|c| c.prefix == conn_prefix && c.client.is_znc())
    }

    /// A ZNC buffer was opened: lines older than when it was last read don't count as
    /// unread when ZNC replays them (as soju's read marker does for soju).
    pub(crate) fn apply_local_read_marker(&self, conn_prefix: &str, buffer: &mut Buffer) {
        if self.is_znc_connection(conn_prefix) {
            if let Some(ts) = self.bouncer_read_markers.get(&read_marker_key(conn_prefix, buffer)) {
                buffer.last_markread_ts = Some(*ts);
            }
        }
    }

    /// Remember that a ZNC buffer was read up to its newest line.
    pub(crate) fn record_local_read(&mut self, buffer_id: &str) {
        let Some(conn_prefix) = buffer_id.split('/').next() else { return };
        if !self.is_znc_connection(conn_prefix) { return; }
        let Some(buffer) = self.buffer_by_id(buffer_id) else { return };
        let Some(ts) = buffer.messages.back().map(|l| l.timestamp) else { return };
        let key = read_marker_key(conn_prefix, buffer);
        if let Some(buffer) = self.buffer_by_id_mut(buffer_id) {
            buffer.last_markread_ts = Some(ts);
        }
        self.bouncer_read_markers.insert(key, ts);
    }
}

/// The networks of a connected ZNC user, with buttons to add, remove, connect and
/// disconnect them. Returns the `(module, command)`s to send.
pub(crate) fn networks_panel(
    ui: &mut egui::Ui,
    state: &mut ZncUiState,
    prefix: &str,
    user: &str,
    networks: &[ZncNetwork],
    reply: Option<&(String, Vec<String>)>,
    accent: Color32,
) -> Vec<(String, String)> {
    let mut commands = Vec::new();
    let mut send = |module: &str, text: String| commands.push((module.to_string(), text));
    let weak = ui.visuals().weak_text_color();

    egui::CollapsingHeader::new(RichText::new(format!("ZNC networks ({})", networks.len())).small())
        .id_salt(("znc_networks", prefix))
        .default_open(true)
        .show(ui, |ui| {
            for network in networks {
                ui.horizontal(|ui| {
                    let dot = if network.on_irc { Color32::from_rgb(50, 205, 50) } else { Color32::from_gray(120) };
                    ui.label(RichText::new("●").color(dot).size(9.0));
                    ui.label(RichText::new(&network.name).strong());
                    let detail = if network.on_irc {
                        let plural = if network.channels == "1" { "" } else { "s" };
                        format!("{} · {} channel{}", network.server, network.channels, plural)
                    } else {
                        "not connected".to_string()
                    };
                    ui.label(RichText::new(detail).small().color(weak));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let deleting = state.confirm_delete.as_ref()
                            .is_some_and(|(p, n)| p == prefix && *n == network.name);
                        if deleting {
                            if ui.small_button("Cancel").clicked() {
                                state.confirm_delete = None;
                            }
                            let delete = egui::Button::new(RichText::new("Delete").small().color(Color32::WHITE))
                                .fill(Color32::from_rgb(185, 55, 55));
                            if ui.add(delete).clicked() {
                                send("*controlpanel", format!("DelNetwork {} {}", user, network.name));
                                state.confirm_delete = None;
                            }
                            ui.label(RichText::new("Delete the network from ZNC?").small());
                        } else {
                            if ui.small_button(icons::X).on_hover_text("Delete from ZNC").clicked() {
                                state.confirm_delete = Some((prefix.to_string(), network.name.clone()));
                            }
                            if network.on_irc {
                                if ui.small_button("Disconnect").on_hover_text("ZNC leaves the IRC server").clicked() {
                                    send("*controlpanel", format!("Disconnect {} {}", user, network.name));
                                }
                            } else if ui.small_button("Connect").clicked() {
                                send("*controlpanel", format!("Reconnect {} {}", user, network.name));
                            }
                        }
                    });
                });
            }

            if state.adding.as_deref() == Some(prefix) {
                ui.add_space(4.0);
                egui::Grid::new(("znc_add", prefix)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Name:");
                    ui.add(egui::TextEdit::singleline(&mut state.name).desired_width(180.0).hint_text("libera"));
                    ui.end_row();
                    ui.label("Server:");
                    ui.add(egui::TextEdit::singleline(&mut state.host).desired_width(180.0).hint_text("irc.libera.chat"));
                    ui.end_row();
                    ui.label("Port:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut state.port).desired_width(60.0));
                        ui.checkbox(&mut state.tls, "TLS");
                    });
                    ui.end_row();
                    ui.label("Server password:");
                    ui.add(egui::TextEdit::singleline(&mut state.password).password(true).desired_width(180.0).hint_text("optional"));
                    ui.end_row();
                });
                let name = state.name.trim().to_string();
                let host = state.host.trim().to_string();
                let valid = !name.is_empty() && !name.contains(char::is_whitespace)
                    && !host.is_empty() && !host.contains(char::is_whitespace)
                    && state.port.trim().parse::<u16>().is_ok()
                    && !state.password.contains(char::is_whitespace);
                ui.horizontal(|ui| {
                    let add = egui::Button::new(RichText::new("Add").color(Color32::WHITE)).fill(accent);
                    if ui.add_enabled(valid, add).clicked() {
                        send("*controlpanel", format!("AddNetwork {} {}", user, name));
                        let port = format!("{}{}", if state.tls { "+" } else { "" }, state.port.trim());
                        let server = format!("AddServer {} {} {} {} {}", user, name, host, port, state.password);
                        send("*controlpanel", server.trim_end().to_string());
                        *state = ZncUiState::default();
                    }
                    if ui.button("Cancel").clicked() {
                        *state = ZncUiState::default();
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    if ui.small_button("+ Add network").clicked() {
                        *state = ZncUiState { adding: Some(prefix.to_string()), ..ZncUiState::default() };
                    }
                    if ui.small_button("Refresh").clicked() {
                        send("*status", "ListNetworks".to_string());
                    }
                });
            }

            if let Some((command, lines)) = reply {
                ui.add_space(2.0);
                // AddServer's last word may be the server's password.
                let shown: String = if command.to_ascii_lowercase().starts_with("*controlpanel addserver") {
                    command.split_whitespace().take(6).collect::<Vec<_>>().join(" ")
                } else {
                    command.clone()
                };
                ui.label(RichText::new(shown).small().color(weak));
                for line in lines.iter().take(8) {
                    ui.label(RichText::new(line).small().monospace());
                }
            }
        });
    commands
}
