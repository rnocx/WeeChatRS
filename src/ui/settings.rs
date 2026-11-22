use crate::ui::theme::{AppTheme, ThemeColor};
use crate::ui::app::{WeeChatApp, ConnectionProfile, BackendType};
use crate::ui::fonts;
use crate::ui::icons;
use egui::{Color32, Vec2, RichText, ScrollArea, Frame, CornerRadius, Margin, Stroke, ViewportBuilder, ViewportCommand, ViewportId};
use egui::color_picker::{color_edit_button_srgba, Alpha};

/// Sections in the Settings window sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum SettingsSection {
    #[default]
    General,
    Chat,
    Appearance,
    Theme,
    FileSharing,
    Keybinds,
}

impl SettingsSection {
    const ALL: [Self; 6] = [Self::General, Self::Chat, Self::Appearance, Self::Theme, Self::FileSharing, Self::Keybinds];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Chat => "Chat",
            Self::Appearance => "Appearance",
            Self::Theme => "Theme",
            Self::FileSharing => "File sharing",
            Self::Keybinds => "Keybinds",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => icons::GEAR,
            Self::Chat => icons::CHAT_CIRCLE_TEXT,
            Self::Appearance => icons::PALETTE,
            Self::Theme => icons::SWATCHES,
            Self::FileSharing => icons::UPLOAD_SIMPLE,
            Self::Keybinds => icons::KEYBOARD,
        }
    }
}

/// Colours shared by the Settings and Connections windows.
struct Palette {
    accent: Color32,
    /// Opaque window background (the main window's fills carry the opacity setting).
    window: Color32,
    sidebar: Color32,
    card: Color32,
    border: Color32,
    secondary_fill: Color32,
    secondary_stroke: Stroke,
    danger: Color32,
}

impl Palette {
    fn new(accent: Color32, is_light: bool) -> Self {
        Self {
            accent,
            window: if is_light { Color32::from_gray(240) } else { Color32::from_rgb(30, 30, 38) },
            sidebar: if is_light { Color32::from_gray(228) } else { Color32::from_rgb(24, 24, 31) },
            card: if is_light { Color32::from_gray(228) } else { Color32::from_rgb(38, 38, 48) },
            border: if is_light { Color32::from_gray(200) } else { Color32::from_gray(55) },
            secondary_fill: Color32::from_rgba_unmultiplied(100, 149, 237, if is_light { 30 } else { 40 }),
            secondary_stroke: Stroke::new(1.0, accent.linear_multiply(0.6)),
            danger: Color32::from_rgb(185, 55, 55),
        }
    }
}

/// Builder for a tool window: titled, sized, and on first show centred over the main window.
fn tool_window(title: &str, size: [f32; 2], min_size: [f32; 2], pos: egui::Pos2) -> ViewportBuilder {
    let builder = ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size(min_size)
        .with_position(pos);
    // macOS: same opaque, theme-coloured titlebar as the main window (see macos_titlebar).
    #[cfg(target_os = "macos")]
    let builder = builder
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    builder
}

/// Where to open a window of `size` so it's centred over the main window. Called from the
/// main viewport; the result is kept while the window is open so egui doesn't move it back
/// after the user drags it.
fn centered_over_main(ctx: &egui::Context, size: [f32; 2]) -> egui::Pos2 {
    let main = ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0)));
    (main.center() - egui::vec2(size[0], size[1]) / 2.0).max(main.min)
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(4.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(ui.visuals().weak_text_color()));
}

/// A path field with a Browse button, for a PEM certificate or key file.
fn pem_file_row(ui: &mut egui::Ui, path: &mut String, title: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(path).desired_width(170.0).hint_text("/path/to/file.pem"));
        if ui.button("Browse…").clicked() {
            if let Some(file) = rfd::FileDialog::new().set_title(title).add_filter("PEM", &["pem", "crt", "cer", "key"]).pick_file() {
                *path = file.to_string_lossy().into_owned();
            }
        }
        if !path.is_empty() && ui.small_button("Clear").clicked() {
            path.clear();
        }
    });
}

/// The colours the chat area uses when a theme leaves them unset.
fn theme_bg_fg(theme: &AppTheme) -> (Color32, Color32) {
    (
        theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18)),
        theme.foreground.map(Color32::from).unwrap_or(Color32::from_rgb(204, 204, 204)),
    )
}

/// A small preview of a theme: its background with six of its colours.
fn theme_swatch(ui: &mut egui::Ui, theme: &AppTheme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(52.0, 14.0), egui::Sense::hover());
    let (bg, _) = theme_bg_fg(theme);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), bg);
    painter.rect_stroke(rect, CornerRadius::same(3), ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    for (k, i) in (1..=6).enumerate() {
        let center = egui::pos2(rect.left() + 8.0 + k as f32 * 7.2, rect.center().y);
        painter.circle_filled(center, 2.6, Color32::from(theme.ansi[i]));
    }
}

/// A quick-pick button drawn in the theme's own colours.
fn theme_chip(ui: &mut egui::Ui, theme: &AppTheme, selected: bool) -> egui::Response {
    let (bg, fg) = theme_bg_fg(theme);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    for i in [1, 2, 4] {
        job.append("●", 0.0, egui::TextFormat::simple(font.clone(), Color32::from(theme.ansi[i])));
    }
    job.append(&theme.name, 6.0, egui::TextFormat::simple(font, fg));
    let stroke = if selected {
        Stroke::new(2.0, ui.visuals().selection.bg_fill)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    ui.add(egui::Button::new(job).fill(bg).stroke(stroke).corner_radius(CornerRadius::same(12)))
}

const DEFAULT_CHAT_FONT: &str = "Default (JetBrains Mono)";
const SETTINGS_SIZE: [f32; 2] = [760.0, 560.0];
// Tall enough for the whole IRC connection form, Save button included.
const CONNECTIONS_SIZE: [f32; 2] = [620.0, 600.0];

impl WeeChatApp {
    /// Saves the connection being edited, with its passwords in the keychain when
    /// asked to. A password under the connection's old name, or one no longer to be
    /// kept, is removed. Fails when another connection would get the same prefix
    /// (the label in lower case, other characters as `_`), which names its buffers.
    fn save_edited_profile(&mut self) -> Result<(), String> {
        self.profile_save_error = None;
        let profile = self.editing_profile.clone();
        let prefix = profile.prefix();
        let taken = self.profiles.iter().enumerate()
            .any(|(i, other)| Some(i) != self.editing_profile_idx && other.prefix() == prefix);
        if taken {
            return Err(if profile.label.trim().is_empty() {
                "Give the connection a label: another one is without a label too.".to_string()
            } else {
                format!("Another connection is called almost the same as \"{}\". Choose a different label.", profile.label)
            });
        }
        let old = self.editing_profile_idx.and_then(|i| self.profiles.get(i)).cloned();
        let secrets = [
            (profile.keyring_host_key(), profile.save_password, &self.editing_password,
                old.as_ref().map(|o| o.keyring_host_key())),
            (profile.ssh_keyring_key(), profile.ssh_save_password, &self.editing_ssh_password,
                old.as_ref().map(|o| o.ssh_keyring_key())),
        ];
        for (key, keep, secret, old_key) in secrets {
            if keep && !secret.is_empty() {
                crate::ui::secure_storage::save_by_key(&key, secret)
                    .map_err(|e| format!("The keychain didn't take the password: {e}"))?;
            } else if !keep {
                let _ = crate::ui::secure_storage::delete_by_key(&key);
            }
            // Not when the password couldn't be read to move it: it would be lost.
            if let Some(old_key) = old_key.filter(|k| *k != key && (!keep || !secret.is_empty())) {
                let _ = crate::ui::secure_storage::delete_by_key(&old_key);
            }
        }
        // Cache passwords for the session so connect skips prompts.
        if !self.editing_password.is_empty() {
            self.session_passwords.insert(prefix.clone(), self.editing_password.clone());
        }
        if !self.editing_ssh_password.is_empty() {
            self.session_ssh_passwords.insert(prefix, self.editing_ssh_password.clone());
        }
        match self.editing_profile_idx {
            Some(idx) if idx < self.profiles.len() => self.profiles[idx] = profile,
            _ => self.profiles.push(profile),
        }
        Ok(())
    }

    fn settings_viewport_id() -> ViewportId { ViewportId::from_hash_of("weechatrs_settings") }
    fn connections_viewport_id() -> ViewportId { ViewportId::from_hash_of("weechatrs_connections") }

    /// Open the Settings window, or bring it to the front if it's already open.
    pub(crate) fn open_settings(&mut self, ctx: &egui::Context) {
        if self.show_settings {
            ctx.send_viewport_cmd_to(Self::settings_viewport_id(), ViewportCommand::Focus);
        }
        self.show_settings = true;
    }

    /// Open the Connections window, or bring it to the front if it's already open.
    pub(crate) fn open_connections(&mut self, ctx: &egui::Context) {
        if self.show_connections {
            ctx.send_viewport_cmd_to(Self::connections_viewport_id(), ViewportCommand::Focus);
        }
        self.show_connections = true;
    }

    /// Settings as its own native window (an in-app window where the platform can't open
    /// more windows). Call once per frame from the main viewport.
    pub(crate) fn show_settings_window(&mut self, ctx: &egui::Context, accent_color: Color32, is_light: bool) {
        if !self.show_settings || self.settings_section != SettingsSection::Theme {
            // Back to the theme in use the next time the Theme page opens.
            self.editing_light_theme = None;
        }
        if !self.show_settings {
            self.settings_window_pos = None;
            return;
        }
        if self.settings_window_pos.is_none() {
            // Just opened: an immediate viewport only repaints with the main window, so ask for
            // another pass or it can stay blank until the next input event.
            ctx.request_repaint();
        }
        let pos = *self.settings_window_pos.get_or_insert_with(|| centered_over_main(ctx, SETTINGS_SIZE));
        let builder = tool_window("WeeChatRS Settings", SETTINGS_SIZE, [560.0, 380.0], pos);
        let palette = Palette::new(accent_color, is_light);
        let mut close = false;
        ctx.show_viewport_immediate(Self::settings_viewport_id(), builder, |ui, class| {
            #[cfg(target_os = "macos")]
            if class == egui::ViewportClass::Immediate {
                let title_color = ui.visuals().weak_text_color();
                crate::ui::app::macos_titlebar(ui, "WeeChatRS Settings", palette.window, palette.border, title_color);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = class;
            // Esc closes, except while a keybind is being captured (Esc cancels that).
            if ui.input(|i| i.viewport().close_requested()
                || (self.editing_keybind.is_none() && i.key_pressed(egui::Key::Escape)))
            {
                close = true;
            }
            self.settings_contents(ui, &palette);
        });
        if close {
            self.show_settings = false;
            self.editing_keybind = None;
        }
    }

    fn settings_contents(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::left("settings_nav")
            .resizable(false)
            .exact_size(184.0)
            .frame(Frame::NONE.fill(p.sidebar).inner_margin(Margin::symmetric(10, 16)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Settings").strong().size(17.0));
                });
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).size(11.0).color(p.accent.linear_multiply(0.7)));
                });
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                for section in SettingsSection::ALL {
                    let selected = self.settings_section == section;
                    let text = RichText::new(format!("{}   {}", section.icon(), section.label()));
                    let text = if selected { text.color(p.accent).strong() } else { text };
                    let button = egui::Button::selectable(selected, text)
                        .min_size(Vec2::new(ui.available_width(), 30.0));
                    if ui.add(button).clicked() {
                        self.settings_section = section;
                        self.editing_keybind = None;
                    }
                }
            });

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(p.window).inner_margin(Margin::symmetric(24, 18)))
            .show(ui, |ui| {
                ui.label(RichText::new(self.settings_section.label()).strong().size(20.0));
                ui.add_space(4.0);
                ui.separator();
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    match self.settings_section {
                        SettingsSection::General => self.general_section(ui),
                        SettingsSection::Chat => self.chat_section(ui),
                        SettingsSection::Appearance => self.appearance_section(ui, p),
                        SettingsSection::Theme => self.theme_section(ui, p),
                        SettingsSection::FileSharing => {
                            self.file_sharing_section(ui);
                            self.privatebin_section(ui);
                        }
                        SettingsSection::Keybinds => self.keybinds_section(ui, p),
                    }
                    ui.add_space(12.0);
                });
            });
    }

    fn general_section(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Connections");
        ui.checkbox(&mut self.auto_reconnect, "Auto-reconnect when a connection drops");
        ui.checkbox(&mut self.show_lag, "Show lag in the toolbar");
        hint(ui, "For the network of the buffer you're reading. Orange from 1 second, red from 5.");

        section_heading(ui, "WeeChat relay");
        ui.checkbox(&mut self.sync_read_weechat, "Sync read state with WeeChat");
        hint(ui, "Reading a buffer here clears it from WeeChat's hotlist and moves its read marker, and a buffer read in WeeChat is read here.");
        ui.checkbox(&mut self.follow_weechat, "Follow in WeeChat");
        hint(ui, "WeeChat switches to the buffer you open here, so its own screen shows the same one.");

        section_heading(ui, "Buffer list");
        ui.checkbox(&mut self.show_server_headers, "Show server group headers");
        ui.checkbox(&mut self.show_hidden_buffers, "Show hidden buffers");
        ui.checkbox(&mut self.show_titlebar, "Show topic and modes bar above the chat");

        section_heading(ui, "Notifications");
        hint(ui, "Highlights and private messages show a system notification.");
        if ui.button(format!("{} Send test notification", icons::BELL)).clicked() {
            crate::ui::notify::show(crate::ui::notify::Notification {
                app_name: "WeeChatRS".to_string(),
                title: "WeeChatRS".to_string(),
                body: "Notifications are working.".to_string(),
            });
        }
    }

    fn chat_section(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Messages");
        ui.checkbox(&mut self.show_timestamps, "Show timestamps");
        ui.checkbox(&mut self.group_messages, "Group consecutive messages from the same nick");
        hint(ui, "Shows the nick once for messages sent within 5 minutes of each other; hover a line for its time.");
        ui.checkbox(&mut self.show_nick_modes, "Show nick modes (@, %, +) in chat");
        hint(ui, "On IRC, soju, ZNC and Injected, from the nick list. WeeChat shows them by its own option, irc.look.nick_mode.");
        ui.checkbox(&mut self.show_channel_modes, "Show channel modes (such as [+nt]) before the topic");
        ui.checkbox(&mut self.show_typing, "Show who is typing");
        ui.checkbox(&mut self.send_typing, "Tell others when I'm typing");
        hint(ui, "Shows or sends IRCv3 typing indicators. On IRC, soju and ZNC they need a server that takes message tags, Injected has its own. The WeeChat relay has none. Sending lets others see when you start and stop typing.");
        ui.checkbox(&mut self.show_filtered_lines, "Show filtered lines");
        hint(ui, "Joins, parts and quits the smart filter hides, lines from ignored nicks, and on WeeChat whatever its /filter rules hide. A buffer can differ: right-click it, Filtered lines.");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.smart_filter, "Smart filter for joins, parts, quits and nick changes");
        });
        ui.add_enabled_ui(self.smart_filter, |ui| {
            ui.horizontal(|ui| {
                ui.label("Show them for nicks that spoke in the last");
                ui.add(egui::DragValue::new(&mut self.smart_filter_minutes).range(1..=120).suffix(" min"));
            });
        });
        hint(ui, "On IRC, soju, ZNC and Injected, as WeeChat does. A join shows after all when the nick speaks soon after. WeeChat filters on its server. Applies to new lines.");
        ui.checkbox(&mut self.colored_nicks, "Coloured nicknames in the nick list");
        ui.checkbox(&mut self.group_nicklist, "Group the nick list by rank");
        hint(ui, "Owners, Admins, Operators, Half-ops, Voiced and Users, each under its own heading.");
        ui.checkbox(&mut self.emoji_rendering, "Colour emoji rendering");
        hint(ui, "Requires the emojinize script on the WeeChat server.");

        section_heading(ui, "Highlights");
        let mentions_before = self.show_mentions;
        ui.checkbox(&mut self.show_mentions, "Show Mentions");
        hint(ui, "An entry at the top of the buffer list with the lines that highlighted you, from every buffer. Also the toolbar button, its shortcut, /mentions and the quick switcher.");
        if self.show_mentions != mentions_before {
            self.mentions_setting_changed();
        }
        ui.horizontal(|ui| {
            ui.label("Highlight words:");
            let edit = egui::TextEdit::singleline(&mut self.highlight_words_text)
                .hint_text("your name, a project")
                .desired_width(300.0);
            if ui.add(edit).changed() {
                self.highlight_words = crate::ui::ignore::parse_words(&self.highlight_words_text);
            }
        });
        hint(ui, "Besides your nick, separated by commas; whole words, any case. On IRC, soju, ZNC and Injected; WeeChat uses its own weechat.look.highlight.");

        section_heading(ui, "Ignored nicks");
        hint(ui, "Their messages are hidden, and shown again with Show filtered lines. A nick or a pattern with * and ?; right-click a nick to ignore it on one network. On IRC, soju, ZNC and Injected; WeeChat has its own /ignore.");
        let mut remove = None;
        for (i, rule) in self.ignores.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&rule.nick).strong());
                let scope = if rule.network.is_some() { format!("on {}", rule.network_label) } else { "on all networks".to_string() };
                ui.label(RichText::new(scope).color(ui.visuals().weak_text_color()));
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        let mut add = false;
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut self.ignore_input).hint_text("nick or pattern").desired_width(200.0));
            add = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            add |= ui.add_enabled(!self.ignore_input.trim().is_empty(), egui::Button::new("Ignore on all networks")).clicked();
        });
        if let Some(i) = remove {
            self.ignores.remove(i);
            self.reapply_ignores();
        }
        let nick = self.ignore_input.trim().to_string();
        if add && !nick.is_empty() {
            self.add_ignore(nick, None);
            self.ignore_input.clear();
        }

        section_heading(ui, "Aliases");
        hint(ui, "Your own commands: with ns → /msg NickServ, typing /ns identify pw sends /msg NickServ identify pw. In the command, $1, $2… are single words, $1- the words from the first on, $* all of them, $nick your nick and $channel the buffer; ; separates commands. Also /alias and /unalias. On IRC, soju, ZNC and Injected; WeeChat has its own /alias.");
        let mut remove = None;
        egui::Grid::new("aliases_grid").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
            for (name, command) in &self.aliases {
                ui.label(RichText::new(format!("/{name}")).strong());
                ui.label(RichText::new(command).monospace());
                if ui.small_button("Remove").clicked() {
                    remove = Some(name.clone());
                }
                ui.end_row();
            }
        });
        if let Some(name) = remove {
            self.aliases.remove(&name);
        }
        let mut add = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.alias_input.0).hint_text("name").desired_width(90.0));
            let resp = ui.add(egui::TextEdit::singleline(&mut self.alias_input.1).hint_text("/msg NickServ").desired_width(260.0));
            add = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let name = crate::ui::alias::normalize_name(&self.alias_input.0);
            add |= ui.add_enabled(name.is_some() && !self.alias_input.1.trim().is_empty(), egui::Button::new("Add alias")).clicked();
        });
        let command = self.alias_input.1.trim();
        if let (true, Some(name), false) = (add, crate::ui::alias::normalize_name(&self.alias_input.0), command.is_empty()) {
            let command = if command.starts_with('/') { command.to_string() } else { format!("/{command}") };
            self.aliases.insert(name, command);
            self.alias_input = Default::default();
        }

        self.spell_check_settings(ui);

        section_heading(ui, "Nick column");
        egui::Grid::new("chat_nick_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Max width (chars):");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.prefix_align_max).range(0usize..=64usize).speed(1));
                hint(ui, "0 = auto");
            });
            ui.end_row();
            ui.label("Nick/message separator:");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.prefix_suffix).desired_width(40.0));
                hint(ui, "weechat.look.prefix_suffix");
            });
            ui.end_row();
        });

        section_heading(ui, "Previews");
        ui.checkbox(&mut self.show_inline_images, "Inline image previews");
        hint(ui, "Adds a preview button to image links.");
        ui.checkbox(&mut self.show_link_previews, "Link previews");
        hint(ui, "Adds a preview button to other links (title, description, image).");
    }

    fn appearance_section(&mut self, ui: &mut egui::Ui, p: &Palette) {
        section_heading(ui, "Font");
        hint(ui, "The chat font is used for messages, buffer names, the nick list and the input bar.");
        egui::Grid::new("appearance_font_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Size:");
            let sizes: &[f32] = &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 20.0, 22.0, 24.0];
            egui::ComboBox::from_id_salt("font_size_combo")
                .selected_text(format!("{}px", self.font_size as u32))
                .width(80.0)
                .show_ui(ui, |ui| {
                    for &size in sizes {
                        ui.selectable_value(&mut self.font_size, size, format!("{}px", size as u32));
                    }
                });
            ui.end_row();

            ui.label("Chat font:");
            ui.horizontal(|ui| self.font_family_picker(ui, p));
            ui.end_row();
        });
        ui.checkbox(&mut self.monospace_ui, "Use the chat font for the whole interface");

        section_heading(ui, "Density");
        hint(ui, "Spacing in the buffer list, the chat and the nick list. Font size is separate.");
        ui.horizontal(|ui| {
            for (name, value) in crate::ui::app::DENSITY_STOPS {
                if ui.selectable_label((self.density - value).abs() < 0.01, name).clicked() {
                    self.density = value;
                }
            }
        });
        ui.add(
            egui::Slider::new(&mut self.density, crate::ui::app::DENSITY_MIN..=crate::ui::app::DENSITY_MAX)
                .show_value(false)
                .step_by(0.05),
        );

        section_heading(ui, "Window");
        ui.horizontal(|ui| {
            ui.label("Opacity:");
            ui.add(egui::Slider::new(&mut self.opacity, 0.1..=1.0));
        });
        #[cfg(target_os = "macos")]
        {
            ui.checkbox(&mut self.blur_background, "Blur background");
            hint(ui, "Lower the opacity to see the blur.");
        }
    }

    fn font_family_picker(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let selected_label = if self.font_name.is_empty() { DEFAULT_CHAT_FONT.to_string() } else { self.font_name.clone() };
        // (family, path, face index)
        let mut new_font: Option<(String, String, u32)> = None;
        let combo_resp = egui::ComboBox::from_id_salt("font_family_combo")
            .selected_text(&selected_label)
            .width(220.0)
            .show_ui(ui, |ui| {
                let search_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.font_search)
                        .hint_text("Search fonts…")
                        .desired_width(f32::INFINITY),
                );
                if search_edit.gained_focus() || (!search_edit.has_focus() && egui::Popup::is_any_open(ui.ctx())) {
                    search_edit.request_focus();
                }
                ui.separator();
                if ui.selectable_label(self.font_name.is_empty(), DEFAULT_CHAT_FONT).clicked() {
                    new_font = Some((String::new(), String::new(), 0));
                }
                ui.separator();
                let query = self.font_search.to_lowercase();
                for family in &self.available_fonts {
                    if (query.is_empty() || family.name.to_lowercase().contains(&query))
                        && ui.selectable_label(family.name == self.font_name, family.name.as_str()).clicked() {
                            let face = family.regular();
                            new_font = Some((family.name.clone(), face.path.clone(), face.index));
                        }
                }
            });
        if combo_resp.inner.is_none() {
            self.font_search.clear();
        }
        self.font_weight_picker(ui);
        if ui.add(
            egui::Button::new(RichText::new("Browse…").color(p.accent))
                .fill(p.secondary_fill)
                .stroke(p.secondary_stroke)
        ).clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Font files", &["ttf", "otf", "ttc"]).pick_file() {
                let path_str = path.to_string_lossy().into_owned();
                let name = fonts::family_from_file(&path_str)
                    .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                new_font = Some((name, path_str, 0));
            }
        }
        if let Some((name, path, index)) = new_font {
            self.font_name = name;
            self.font_path = path;
            self.font_index = index;
            self.font_search.clear();
        }
    }

    /// The weights of the chosen family, when it has more than one: thin fonts such as
    /// Iosevka look grey at Regular, so Medium or SemiBold matter.
    fn font_weight_picker(&mut self, ui: &mut egui::Ui) {
        let Some(family) = self.available_fonts.iter().find(|f| f.name == self.font_name) else { return };
        if family.faces.len() < 2 {
            return;
        }
        let current = family.faces.iter()
            .find(|f| f.path == self.font_path && f.index == self.font_index)
            .map_or("Regular", |f| f.style.as_str());
        let mut picked = None;
        egui::ComboBox::from_id_salt("font_weight_combo")
            .selected_text(current)
            .width(150.0)
            .show_ui(ui, |ui| {
                for face in &family.faces {
                    let selected = face.path == self.font_path && face.index == self.font_index;
                    if ui.selectable_label(selected, face.style.as_str()).clicked() {
                        picked = Some((face.path.clone(), face.index));
                    }
                }
            });
        if let Some((path, index)) = picked {
            self.font_path = path;
            self.font_index = index;
        }
    }

    /// The theme dropdown. While it is open, ↑/↓ apply each theme as a preview; a
    /// click or Enter keeps the theme, and closing it any other way (Esc, clicking
    /// outside) goes back to the theme it was opened with.
    fn theme_picker(&mut self, ui: &mut egui::Ui) {
        let mut picked: Option<AppTheme> = None;
        let combo_resp = egui::ComboBox::from_id_salt("theme_combo")
            .selected_text(&self.theme.name)
            .width(240.0)
            .height(380.0)
            .show_ui(ui, |ui| {
                if self.theme_before_preview.is_none() {
                    self.theme_before_preview = Some(self.theme.clone());
                }
                // Taken before the search box sees them: it would move its cursor,
                // and Enter would only drop its focus.
                let (down, up, enter) = ui.input_mut(|i| (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                ));
                let query = self.theme_search.to_lowercase();
                let shown: Vec<&crate::ui::theme::CatalogTheme> = crate::ui::theme::catalog().iter()
                    .filter(|entry| query.is_empty() || entry.theme.name.to_lowercase().contains(&query))
                    .collect();
                // Step from the theme in use, so the highlight is always the applied one.
                let current = shown.iter().position(|entry| entry.theme.name == self.theme.name);
                let step = match (down, up, current) {
                    (true, _, Some(i)) => Some((i + 1).min(shown.len().saturating_sub(1))),
                    (_, true, Some(i)) => Some(i.saturating_sub(1)),
                    (true, _, None) | (_, true, None) => Some(0),
                    _ => None,
                };
                let mut stepped = false;
                if let Some(entry) = step.and_then(|i| shown.get(i)) {
                    stepped = entry.theme.name != self.theme.name;
                    self.theme = entry.theme.clone();
                }
                if enter && current.is_some() {
                    picked = Some(self.theme.clone());
                    ui.close();
                }

                let search_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.theme_search)
                        .hint_text("Search themes…")
                        .desired_width(f32::INFINITY),
                );
                if search_edit.gained_focus() || (!search_edit.has_focus() && egui::Popup::is_any_open(ui.ctx())) {
                    search_edit.request_focus();
                }
                let mut section: Option<bool> = None;
                for entry in shown {
                    if section != Some(entry.user) {
                        section = Some(entry.user);
                        ui.separator();
                        hint(ui, if entry.user { "Your Ghostty themes" } else { "Ghostty themes" });
                    }
                    let selected = entry.theme.name == self.theme.name;
                    let label = ui.horizontal(|ui| {
                        theme_swatch(ui, &entry.theme);
                        ui.selectable_label(selected, entry.theme.name.as_str())
                    }).inner;
                    if selected && stepped {
                        label.scroll_to_me(None);
                    }
                    if label.clicked() {
                        picked = Some(entry.theme.clone());
                    }
                }
            });
        if combo_resp.inner.is_none() {
            self.theme_search.clear();
            // Closed without a pick: undo the arrow-key preview.
            if let Some(theme) = self.theme_before_preview.take() {
                self.theme = theme;
            }
        }
        if let Some(theme) = picked {
            self.theme = theme;
            self.theme_before_preview = None;
            self.theme_search.clear();
            self.theme_import_error = None;
        }
    }

    /// Whether the app is drawn in the light theme: following the system and it asks
    /// for light, or the Theme page is open on the light theme.
    pub(crate) fn shows_light_theme(&self) -> bool {
        if !self.follow_system_theme {
            return false;
        }
        match self.editing_light_theme {
            Some(light) if self.show_settings && self.settings_section == SettingsSection::Theme => light,
            _ => self.system_light,
        }
    }

    fn theme_section(&mut self, ui: &mut egui::Ui, p: &Palette) {
        section_heading(ui, "Colour scheme");
        ui.add_enabled_ui(!self.adaptive_theme, |ui| self.follow_system_controls(ui));
        // The controls below change `theme`, so the light theme takes its place while
        // they're drawn.
        let light = self.follow_system_theme && self.editing_light_theme.unwrap_or(self.system_light);
        if light {
            std::mem::swap(&mut self.theme, &mut self.light_theme);
        }
        self.theme_controls(ui, p, light);
        if light {
            std::mem::swap(&mut self.theme, &mut self.light_theme);
        }
    }

    fn follow_system_controls(&mut self, ui: &mut egui::Ui) {
        let mut follow = self.follow_system_theme;
        ui.checkbox(&mut follow, "Follow the system's light or dark appearance");
        if follow != self.follow_system_theme {
            self.follow_system_theme = follow;
            self.editing_light_theme = None;
            if let Some(theme) = self.theme_before_preview.take() {
                self.theme = theme;
            }
            if follow {
                // A light theme picked before is the light one from now on.
                if self.theme.is_light() {
                    self.light_theme = std::mem::replace(&mut self.theme, crate::ui::theme::default_theme());
                }
            } else if self.system_light {
                // Keep the theme that was showing.
                self.theme = self.light_theme.clone();
            }
        }
        if !self.follow_system_theme {
            return;
        }
        let editing_light = self.editing_light_theme.unwrap_or(self.system_light);
        let mut light = editing_light;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut light, false, format!("Dark: {}", self.theme.name));
            ui.selectable_value(&mut light, true, format!("Light: {}", self.light_theme.name));
        });
        if light != editing_light {
            // Undo an arrow-key preview in the theme being left.
            if let Some(theme) = self.theme_before_preview.take() {
                if editing_light { self.light_theme = theme } else { self.theme = theme }
            }
            self.editing_light_theme = Some(light);
        }
        if ui.ctx().system_theme().is_none() {
            hint(ui, "The system doesn't say whether it's light or dark, so the dark theme is used.");
        } else {
            hint(ui, "The settings below change the selected theme, and the app shows it while this page is open.");
        }
        ui.add_space(4.0);
    }

    /// The theme picker, quick themes and colours, all changing `theme`; `light` when
    /// that is the light theme for now (Reset then picks the default light theme).
    fn theme_controls(&mut self, ui: &mut egui::Ui, p: &Palette, light: bool) {
        ui.add_enabled_ui(!self.adaptive_theme, |ui| {
            ui.horizontal(|ui| {
                self.theme_picker(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(
                        egui::Button::new(RichText::new("Reset").color(Color32::WHITE))
                            .fill(p.danger)
                            .min_size(Vec2::new(70.0, 24.0))
                    ).clicked() {
                        self.theme = if light { crate::ui::theme::default_light_theme() } else { crate::ui::theme::default_theme() };
                    }
                    if ui.add(
                        egui::Button::new(RichText::new("Import Ghostty theme…").color(p.accent))
                            .fill(p.secondary_fill)
                            .stroke(p.secondary_stroke)
                            .min_size(Vec2::new(140.0, 24.0))
                    ).on_hover_text("A Ghostty theme file (palette = 0=#rrggbb, background = …)").clicked() {
                        // Ghostty theme files have no extension, so no filter.
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                            let theme = std::fs::read_to_string(&path).ok()
                                .and_then(|text| AppTheme::parse_ghostty(&text, name.clone()));
                            match theme {
                                Some(theme) => {
                                    self.theme = theme;
                                    self.theme_import_error = None;
                                }
                                None => self.theme_import_error = Some(format!("{name} is not a Ghostty theme.")),
                            }
                        }
                    }
                });
            });
            if let Some(err) = &self.theme_import_error {
                ui.label(RichText::new(err).small().color(p.danger));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                let default = crate::ui::theme::default_theme();
                let quick = std::iter::once(&default)
                    .chain(crate::ui::theme::QUICK_THEMES.iter().filter_map(|n| crate::ui::theme::find_theme(n)));
                for theme in quick {
                    if theme_chip(ui, theme, theme.name == self.theme.name).clicked() {
                        self.theme = theme.clone();
                        self.theme_import_error = None;
                    }
                }
            });
        });

        ui.checkbox(&mut self.fit_colors_to_theme, "Fit 256-colour and RGB colours to the theme");
        hint(ui, "Draws colours outside the theme's 16 (such as WeeChat nick colours set by number) in the nearest theme colour. The Default theme keeps them as they are.");

        let mut adaptive_theme = self.adaptive_theme;
        if crate::ui::wallpaper::supported() {
            ui.checkbox(&mut adaptive_theme, "Adaptive theme (from wallpaper)");
        }
        if adaptive_theme {
            hint(ui, "Colours are derived from the desktop wallpaper. Manual editing is disabled while active.");
        }
        if adaptive_theme != self.adaptive_theme {
            self.adaptive_theme = adaptive_theme;
            if adaptive_theme {
                self.wallpaper_rx = Some(crate::ui::wallpaper::start_wallpaper_thread(ui.ctx().clone()));
            } else {
                self.wallpaper_rx = None;
                self.adaptive_theme_result = None;
            }
        }

        section_heading(ui, "Colours");
        ui.add_enabled_ui(!self.adaptive_theme, |ui| {
            ui.horizontal(|ui| {
                ui.label("Background:");
                let mut bg = self.theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18));
                if color_edit_button_srgba(ui, &mut bg, Alpha::Opaque).changed() {
                    self.theme.background = Some(ThemeColor::from(bg));
                }
                ui.add_space(12.0);
                ui.label("Foreground:");
                let mut fg = self.theme.foreground.map(Color32::from).unwrap_or(Color32::from_rgb(204, 204, 204));
                if color_edit_button_srgba(ui, &mut fg, Alpha::Opaque).changed() {
                    self.theme.foreground = Some(ThemeColor::from(fg));
                }
            });
            ui.add_space(6.0);
            const LABELS: [&str; 16] = [
                "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White",
                "Br.Black", "Br.Red", "Br.Green", "Br.Yellow", "Br.Blue", "Br.Magenta", "Br.Cyan", "Br.White",
            ];
            for row in 0..2usize {
                ui.horizontal(|ui| {
                    for col in 0..8usize {
                        let i = row * 8 + col;
                        ui.vertical(|ui| {
                            ui.set_width(52.0);
                            let mut c = Color32::from(self.theme.ansi[i]);
                            if color_edit_button_srgba(ui, &mut c, Alpha::Opaque).changed() {
                                self.theme.ansi[i] = ThemeColor::from(c);
                            }
                            ui.label(RichText::new(LABELS[i]).size(9.0).color(ui.visuals().weak_text_color()));
                        });
                    }
                });
                ui.add_space(2.0);
            }
        });
    }

    fn spell_check_settings(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Spell check");
        let was_on = self.spell_check;
        ui.checkbox(&mut self.spell_check, "Underline misspelled words in the input line");
        if self.spell_check && !was_on {
            for lang in self.spell_languages.clone() {
                if !self.spell_checkers.contains_key(&lang) {
                    self.load_spell_language(ui.ctx(), &lang);
                }
            }
        }
        if !self.spell_check {
            return;
        }
        hint(ui, "A word is right when any ticked language knows it. Commands, links, channels and the buffer's nicks aren't checked.");
        let installed = crate::ui::spellcheck::available_languages();
        let mut codes: Vec<&str> = installed.iter().map(String::as_str).collect();
        codes.extend(crate::ui::spellcheck::CATALOGUE.iter().map(|d| d.code).filter(|c| !installed.iter().any(|i| i == c)));
        egui::Grid::new("spell_languages").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            for code in codes {
                let name = format!("{} ({})", crate::ui::spellcheck::language_name(code), code);
                if installed.iter().any(|i| i == code) {
                    let mut on = self.spell_languages.iter().any(|l| l == code);
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            self.spell_languages.push(code.to_string());
                            self.load_spell_language(ui.ctx(), code);
                        } else {
                            self.spell_languages.retain(|l| l != code);
                            self.spell_checkers.remove(code);
                        }
                    }
                    ui.label("");
                } else {
                    ui.label(RichText::new(name).color(ui.visuals().weak_text_color()));
                    if self.spell_downloading.as_deref() == Some(code) {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Downloading…");
                        });
                    } else if ui.add_enabled(self.spell_downloading.is_none(), egui::Button::new("Download")).clicked() {
                        self.download_spell_language(ui.ctx(), code);
                    }
                }
                ui.end_row();
            }
        });
        if let Some(err) = &self.spell_error {
            ui.label(RichText::new(err).small().color(Color32::from_rgb(220, 60, 60)));
        }
    }

    fn file_sharing_section(&mut self, ui: &mut egui::Ui) {
        use crate::ui::fileshare::FileShareService;
        section_heading(ui, "Service");
        hint(ui, "Files dropped on the chat or attached with the paperclip are uploaded here, and the link is posted to the buffer.");
        egui::ComboBox::from_id_salt("file_share_service_combo")
            .selected_text(match self.file_share_service {
                FileShareService::Jirafeau => "Jirafeau",
                FileShareService::Litterbox => "Litterbox (litterbox.catbox.moe)",
            })
            .width(260.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.file_share_service, FileShareService::Jirafeau, "Jirafeau");
                ui.selectable_value(&mut self.file_share_service, FileShareService::Litterbox, "Litterbox (litterbox.catbox.moe)");
            });

        if self.file_share_service == FileShareService::Jirafeau {
            section_heading(ui, "Jirafeau server");
            let mut url_focused = false;
            egui::Grid::new("jirafeau_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Server URL:");
                ui.vertical(|ui| {
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.jirafeau_url)
                        .hint_text(crate::ui::fileshare::DEFAULT_JIRAFEAU_URL)
                        .desired_width(320.0));
                    url_focused = resp.has_focus();
                    if resp.lost_focus() {
                        if self.jirafeau_url.trim().is_empty() {
                            self.jirafeau_url = crate::ui::fileshare::DEFAULT_JIRAFEAU_URL.to_string();
                        } else if let Ok(url) = crate::ui::fileshare::normalize_base_url(&self.jirafeau_url) {
                            self.jirafeau_url = url;
                        }
                        if let Ok(mut t) = self.jirafeau_test.lock() { *t = None; }
                    }
                    hint(ui, "Your own instance, or the default public one.");
                });
                ui.end_row();

                // Passwords are per server; don't reload them on every keystroke in the URL.
                if !url_focused {
                    self.ensure_jirafeau_passwords_loaded();
                }
                for (kind, label, help) in [
                    ("upload", "Upload password:", "Only if the server restricts who may upload."),
                    ("download", "Download password:", "Protects every upload: anyone opening the link must enter it. It's never posted — share it separately."),
                ] {
                    ui.label(label);
                    ui.vertical(|ui| {
                        let value = if kind == "upload" { &mut self.jirafeau_upload_pw } else { &mut self.jirafeau_download_pw };
                        let resp = ui.add(egui::TextEdit::singleline(value).password(true).hint_text("none").desired_width(220.0));
                        // Save to the Keychain when the field is left after editing.
                        let dirty_id = resp.id.with("dirty");
                        if resp.changed() { ui.data_mut(|d| d.insert_temp(dirty_id, true)); }
                        if resp.lost_focus() && ui.data(|d| d.get_temp::<bool>(dirty_id).unwrap_or(false)) {
                            ui.data_mut(|d| d.remove::<bool>(dirty_id));
                            self.save_jirafeau_password(kind);
                        }
                        hint(ui, help);
                    });
                    ui.end_row();
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Test server").clicked() {
                    let url = self.jirafeau_url.clone();
                    let result = self.jirafeau_test.clone();
                    let ctx = ui.ctx().clone();
                    if let Ok(mut t) = result.lock() { *t = Some(Ok("Checking…".to_string())); }
                    tokio::spawn(async move {
                        let text = crate::ui::fileshare::jirafeau_server_info(&url).await.map(|info| {
                            let max = if info.max_file_mb == 0 { "no file size limit".to_string() } else { format!("files up to {} MB", info.max_file_mb) };
                            format!("Jirafeau {} · {max} · {} per request", info.version, crate::ui::fileshare::human_size(info.request_limit))
                        });
                        if let Ok(mut t) = result.lock() { *t = Some(text); }
                        ctx.request_repaint();
                    });
                }
                let test = self.jirafeau_test.lock().ok().and_then(|t| t.clone());
                match test {
                    Some(Ok(text)) => { ui.label(RichText::new(text).small().color(Color32::from_rgb(80, 190, 110))); }
                    Some(Err(err)) => { ui.label(RichText::new(err).small().color(Color32::from_rgb(220, 80, 80))); }
                    None => {}
                }
            });
        }

        section_heading(ui, "Expiry");
        match self.file_share_service {
            FileShareService::Jirafeau => {
                const DURATIONS: &[(&str, &str)] = &[
                    ("minute",    "1 minute"),
                    ("hour",      "1 hour"),
                    ("day",       "1 day"),
                    ("week",      "1 week"),
                    ("fortnight", "2 weeks"),
                    ("month",     "1 month"),
                    ("quarter",   "3 months"),
                    ("year",      "1 year"),
                    ("none",      "Never expires"),
                ];
                let selected_label = DURATIONS.iter()
                    .find(|(v, _)| *v == self.file_share_duration)
                    .map(|(_, l)| *l)
                    .unwrap_or("1 day");
                egui::ComboBox::from_id_salt("file_share_duration_combo")
                    .selected_text(selected_label)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for &(value, label) in DURATIONS {
                            ui.selectable_value(&mut self.file_share_duration, value.to_string(), label);
                        }
                    });
                hint(ui, "Servers can disable some of these; the upload then reports it.");
            }
            FileShareService::Litterbox => {
                let selected_label = crate::ui::fileshare::LITTERBOX_TIMES.iter()
                    .find(|(v, _)| *v == self.file_share_duration)
                    .map(|(_, l)| *l)
                    .unwrap_or("24 hours");
                egui::ComboBox::from_id_salt("file_share_duration_combo")
                    .selected_text(selected_label)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for &(value, label) in crate::ui::fileshare::LITTERBOX_TIMES {
                            ui.selectable_value(&mut self.file_share_duration, value.to_string(), label);
                        }
                    });
            }
        }
    }

    /// The SSH part of the connection form: the server, how to log in, and a test.
    fn ssh_tunnel_form(&mut self, ui: &mut egui::Ui) {
        use crate::ui::ssh_tunnel::SshAuth;
        let red = Color32::from_rgb(220, 80, 80);
        ui.add_space(4.0);
        egui::Grid::new("ssh_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("SSH server:");
            ui.vertical(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.editing_profile.ssh_host)
                    .desired_width(240.0)
                    .hint_text("user@host:port or an ssh alias"));
                if !self.editing_profile.ssh_host.trim().is_empty() {
                    if let Err(e) = crate::ui::ssh_tunnel::parse_server(&self.editing_profile.ssh_host) {
                        ui.label(RichText::new(e).small().color(red));
                    }
                }
            });
            ui.end_row();

            ui.label("Log in with:");
            let auth = self.editing_profile.ssh_auth.get_or_insert_default();
            ui.horizontal(|ui| {
                ui.selectable_value(auth, SshAuth::Auto, "Agent and keys").on_hover_text("The SSH agent, your ~/.ssh keys and ~/.ssh/config");
                ui.selectable_value(auth, SshAuth::KeyFile, "Key file");
                ui.selectable_value(auth, SshAuth::Password, "Password");
            });
            ui.end_row();

            let auth = *auth;
            if auth == SshAuth::KeyFile {
                ui.label("Key file:");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.editing_profile.ssh_key_file)
                            .desired_width(240.0)
                            .hint_text("~/.ssh/id_ed25519"));
                        if ui.button("Choose…").clicked() {
                            let mut dialog = rfd::FileDialog::new();
                            if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                                dialog = dialog.set_directory(std::path::PathBuf::from(home).join(".ssh"));
                            }
                            if let Some(path) = dialog.pick_file() {
                                self.editing_profile.ssh_key_file = path.display().to_string();
                            }
                        }
                    });
                    let path = crate::ui::ssh_tunnel::expand_home(&self.editing_profile.ssh_key_file);
                    if !self.editing_profile.ssh_key_file.trim().is_empty() {
                        match crate::ui::ssh_tunnel::key_has_passphrase(&path) {
                            Ok(true) => hint(ui, "This key has a passphrase."),
                            Ok(false) => hint(ui, "This key has no passphrase."),
                            Err(e) => { ui.label(RichText::new(e).small().color(red)); }
                        }
                    }
                });
                ui.end_row();
            }
            if auth != SshAuth::Auto {
                ui.label(if auth == SshAuth::Password { "Password:" } else { "Passphrase:" });
                ui.add(egui::TextEdit::singleline(&mut self.editing_ssh_password)
                    .password(true)
                    .desired_width(240.0)
                    .hint_text(if self.editing_profile.ssh_save_password { "saved; type to change" } else { "asked when connecting" }));
                ui.end_row();
            }
        });
        let auth = self.editing_profile.ssh_auth.unwrap_or_default();
        match auth {
            SshAuth::Auto => hint(ui, "A key with a passphrase must be in the SSH agent: on macOS, ssh-add --apple-use-keychain ~/.ssh/<key> once. Or choose Key file and give the passphrase."),
            SshAuth::KeyFile => { ui.checkbox(&mut self.editing_profile.ssh_save_password, "Save passphrase in keychain"); }
            SshAuth::Password => { ui.checkbox(&mut self.editing_profile.ssh_save_password, "Save SSH password in keychain"); }
        }
        hint(ui, "Host and port above are the relay as the SSH server reaches it, often localhost.");

        ui.horizontal(|ui| {
            let ready = crate::ui::ssh_tunnel::parse_server(&self.editing_profile.ssh_host).is_ok();
            let testing = matches!(self.ssh_test.lock().ok().and_then(|t| t.clone()), Some(Ok(ref s)) if s == "Testing…");
            if ui.add_enabled(ready && !testing, egui::Button::new("Test tunnel")).clicked() {
                let profile = &self.editing_profile;
                let secret = Some(self.editing_ssh_password.clone()).filter(|s| !s.is_empty())
                    .or_else(|| profile.ssh_save_password.then(|| crate::ui::secure_storage::load_by_key(&profile.ssh_keyring_key())).flatten());
                let default_port = if profile.backend_type == BackendType::Soju { 6697 } else { 9001 };
                let spec = crate::ui::ssh_tunnel::TunnelSpec {
                    server: profile.ssh_host.clone(),
                    auth,
                    key_file: profile.ssh_key_file.clone(),
                    secret,
                    relay_host: profile.host.clone(),
                    relay_port: profile.port.parse().unwrap_or(default_port),
                };
                let (result, ctx) = (self.ssh_test.clone(), ui.ctx().clone());
                if let Ok(mut t) = result.lock() { *t = Some(Ok("Testing…".to_string())); }
                std::thread::spawn(move || {
                    let outcome = crate::ui::ssh_tunnel::test(&spec);
                    if let Ok(mut t) = result.lock() { *t = Some(outcome); }
                    ctx.request_repaint();
                });
            }
            match self.ssh_test.lock().ok().and_then(|t| t.clone()) {
                Some(Ok(text)) if text == "Testing…" => { ui.spinner(); ui.label(RichText::new(text).small()); }
                Some(Ok(text)) => { ui.label(RichText::new(text).small().color(Color32::from_rgb(80, 190, 110))); }
                Some(Err(err)) => { ui.label(RichText::new(err).small().color(red)); }
                None => {}
            }
        });
        ui.add_space(4.0);
    }

    fn privatebin_section(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Text pastes (PrivateBin)");
        hint(ui, "When you paste several lines, Share as PrivateBin paste sends a link instead, on IRC, soju, ZNC and WeeChat (Injected has its own pastes). The text is encrypted on this computer and the key is only in the link, so the server can't read it. No account is used; the server sees only your IP address.");
        egui::Grid::new("privatebin_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Server URL:");
            ui.vertical(|ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.privatebin_server)
                    .hint_text(crate::ui::privatebin::DEFAULT_SERVER)
                    .desired_width(320.0));
                if resp.lost_focus() {
                    if self.privatebin_server.trim().is_empty() {
                        self.privatebin_server = crate::ui::privatebin::DEFAULT_SERVER.to_string();
                    } else if let Ok(url) = crate::ui::privatebin::normalize_server(&self.privatebin_server) {
                        self.privatebin_server = url.to_string();
                    }
                }
                match crate::ui::privatebin::normalize_server(&self.privatebin_server) {
                    Err(err) => { ui.label(RichText::new(err).small().color(Color32::from_rgb(220, 80, 80))); }
                    Ok(_) => hint(ui, "Your own PrivateBin, or a public one."),
                }
            });
            ui.end_row();

            ui.label("Expiry:");
            let selected = crate::ui::privatebin::EXPIRY.iter()
                .find(|(v, _)| *v == self.privatebin_expire)
                .map_or("1 week", |(_, l)| *l);
            egui::ComboBox::from_id_salt("privatebin_expire_combo")
                .selected_text(selected)
                .width(140.0)
                .show_ui(ui, |ui| {
                    for &(value, label) in crate::ui::privatebin::EXPIRY {
                        ui.selectable_value(&mut self.privatebin_expire, value.to_string(), label);
                    }
                });
            ui.end_row();
        });
        ui.checkbox(&mut self.privatebin_burn, "Burn after reading");
        hint(ui, "The paste is deleted once it's opened, so only the first person to open the link can read it. In a channel, that's rarely what you want.");
    }

    fn keybinds_section(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            hint(ui, "Click a shortcut, then press the new key combination.");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Reset all").clicked() {
                    self.keybinds = crate::ui::keybinds::KeybindsMap::default();
                    self.editing_keybind = None;
                }
            });
        });
        ui.add_space(6.0);

        // Capture mode: consume the next key event and store it.
        if let Some(capturing) = self.editing_keybind {
            let mut captured: Option<(crate::ui::keybinds::SerdeModifiers, crate::ui::keybinds::AppKey)> = None;
            let mut cancel = false;
            ui.input_mut(|i| {
                let events = std::mem::take(&mut i.events);
                let mut remaining = Vec::new();
                let mut found = false;
                for event in events {
                    if !found {
                        if let egui::Event::Key { key, pressed: true, modifiers, repeat: false, .. } = &event {
                            if *key == egui::Key::Escape {
                                cancel = true;
                                found = true;
                                continue;
                            } else if let Some(app_key) = crate::ui::keybinds::AppKey::from_egui(*key) {
                                captured = Some((crate::ui::keybinds::SerdeModifiers::from_egui(*modifiers), app_key));
                                found = true;
                                continue;
                            }
                        }
                    }
                    remaining.push(event);
                }
                i.events = remaining;
            });
            if cancel { self.editing_keybind = None; }
            if let Some((mods, key)) = captured {
                self.keybinds.0.insert(capturing, (mods, key));
                self.editing_keybind = None;
            }
        }

        egui::Grid::new("keybinds_grid")
            .num_columns(3)
            .spacing([16.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                for &action in crate::ui::keybinds::KeybindAction::ALL {
                    ui.label(action.label());
                    let is_capturing = self.editing_keybind == Some(action);
                    let btn_text = if is_capturing {
                        RichText::new("Press a key…").color(p.accent)
                    } else {
                        RichText::new(self.keybinds.shortcut_label(action))
                    };
                    if ui.add(egui::Button::new(btn_text).min_size(Vec2::new(110.0, 0.0))).clicked() {
                        self.editing_keybind = if is_capturing { None } else { Some(action) };
                    }
                    if ui.small_button(icons::ARROW_COUNTER_CLOCKWISE).on_hover_text("Reset to default").clicked() {
                        if let Some(default) = crate::ui::keybinds::KeybindsMap::default().0.get(&action) {
                            self.keybinds.0.insert(action, *default);
                        }
                        if self.editing_keybind == Some(action) {
                            self.editing_keybind = None;
                        }
                    }
                    ui.end_row();
                }
            });

        if self.editing_keybind.is_some() {
            ui.add_space(4.0);
            hint(ui, "Press a key combination to bind, or Esc to cancel.");
        }
    }

    /// Connections as its own native window (an in-app window where the platform can't
    /// open more windows). Call once per frame from the main viewport.
    pub(crate) fn show_connections_window(&mut self, ctx: &egui::Context, accent_color: Color32, is_light: bool) {
        if !self.show_connections {
            self.connections_window_pos = None;
            return;
        }
        if self.connections_window_pos.is_none() {
            ctx.request_repaint(); // see show_settings_window
        }
        let pos = *self.connections_window_pos.get_or_insert_with(|| centered_over_main(ctx, CONNECTIONS_SIZE));
        let builder = tool_window("WeeChatRS Connections", CONNECTIONS_SIZE, [460.0, 320.0], pos);
        let palette = Palette::new(accent_color, is_light);
        let mut close = false;
        ctx.show_viewport_immediate(Self::connections_viewport_id(), builder, |ui, class| {
            #[cfg(target_os = "macos")]
            if class == egui::ViewportClass::Immediate {
                let title_color = ui.visuals().weak_text_color();
                crate::ui::app::macos_titlebar(ui, "WeeChatRS Connections", palette.window, palette.border, title_color);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = class;
            if ui.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(Frame::NONE.fill(palette.window).inner_margin(Margin::symmetric(24, 18)))
                .show(ui, |ui| self.connections_contents(ui, &palette));
        });
        if close {
            self.show_connections = false;
        }
    }

    fn connections_contents(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if let Some(receiver) = &self.injected_import.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.injected_import.receiver = None;
                    match result {
                        Ok(session) if self.conn_show_add && self.editing_profile.backend_type == BackendType::Injected => {
                            self.editing_password = session.token;
                            self.injected_import.message = Some(format!("Signed in as {}. Save the connection to use it.", session.username));
                        }
                        Ok(_) => self.injected_import.message = None,
                        Err(error) => self.injected_import.message = Some(error),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.injected_import.receiver = None;
                    self.injected_import.message = Some("Sign-in stopped. Please retry.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }

        let accent_color = p.accent;

        // Pending deferred actions
        let mut do_connect_idx: Option<usize> = None;
        let mut do_disconnect: Option<String> = None;
        let mut do_delete: Option<usize> = None;
        let mut do_edit: Option<usize> = None;
        let mut toggle_add = false;
        let mut znc_commands: Vec<(String, String, String)> = Vec::new();

        ui.label(RichText::new("Connections").strong().size(20.0));
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(8.0);

        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.conn_show_add {
                Frame::NONE
                    .fill(p.card)
                    .corner_radius(CornerRadius::same(8))
                    .stroke(Stroke::new(1.0, accent_color.linear_multiply(0.4)))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let title = if self.editing_profile_idx.is_some() { "Edit Connection" } else { "New Connection" };
                        ui.label(RichText::new(title).strong());
                        ui.add_space(8.0);

                        egui::Grid::new("conn_add_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                            ui.label("Label:");
                            ui.add(egui::TextEdit::singleline(&mut self.editing_profile.label).desired_width(240.0));
                            ui.end_row();
                            ui.label("Backend:");
                            let previous_backend = self.editing_profile.backend_type.clone();
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.editing_profile.backend_type, BackendType::WeeChat, "WeeChat");
                                ui.selectable_value(&mut self.editing_profile.backend_type, BackendType::Soju, "IRC");
                                if ui.selectable_value(&mut self.editing_profile.backend_type, BackendType::Injected, "Injected").changed() {
                                    self.editing_profile.host = "injected-irc.com".into();
                                    self.editing_profile.port = "443".into();
                                    self.editing_profile.use_ssl = true;
                                    self.editing_profile.accept_invalid_certs = false;
                                    self.editing_profile.ssh_enabled = false;
                                    self.editing_password.clear();
                                    self.injected_import = Default::default();
                                }
                            });
                            if previous_backend != self.editing_profile.backend_type { self.editing_password.clear(); self.injected_import = Default::default(); }
                            ui.end_row();
                            ui.label("Host:");
                            ui.add(egui::TextEdit::singleline(&mut self.editing_profile.host).desired_width(240.0));
                            ui.end_row();
                            ui.label("Port:");
                            ui.add(egui::TextEdit::singleline(&mut self.editing_profile.port).desired_width(240.0));
                            ui.end_row();
                            if self.editing_profile.backend_type == BackendType::Soju {
                                ui.label("Nick:");
                                ui.add(egui::TextEdit::singleline(&mut self.editing_profile.nick).desired_width(240.0));
                                ui.end_row();
                                ui.label("Username:");
                                ui.vertical(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut self.editing_profile.username).desired_width(240.0).hint_text("leave blank to use nick"));
                                    hint(ui, "IRC USER field. ZNC: user for all networks, user/network for one");
                                });
                                ui.end_row();
                                ui.label("SASL username:");
                                ui.vertical(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut self.editing_profile.sasl_username).desired_width(240.0).hint_text("leave blank to use nick"));
                                    hint(ui, "The account SASL logs in to, with the password above");
                                });
                                ui.end_row();
                                ui.label("");
                                ui.vertical(|ui| {
                                    ui.checkbox(&mut self.editing_profile.sasl_optional, "Stay connected if the login fails");
                                    hint(ui, "Off: a refused SASL login ends the connection, so your host isn't shown without your account");
                                });
                                ui.end_row();
                                ui.label("Client cert:");
                                ui.vertical(|ui| {
                                    pem_file_row(ui, &mut self.editing_profile.client_cert, "Choose the TLS client certificate (PEM)");
                                    hint(ui, "certfp: log in with a TLS certificate (SASL EXTERNAL). Needs SSL");
                                });
                                ui.end_row();
                                if !self.editing_profile.client_cert.trim().is_empty() {
                                    ui.label("Cert key:");
                                    ui.vertical(|ui| {
                                        pem_file_row(ui, &mut self.editing_profile.client_key, "Choose the client certificate's private key (PKCS#8 PEM)");
                                        hint(ui, "Leave blank if the key is in the certificate file");
                                    });
                                    ui.end_row();
                                }
                                ui.label("Channel:");
                                ui.add(egui::TextEdit::singleline(&mut self.editing_profile.channel).desired_width(240.0).hint_text("#channel (optional)"));
                                ui.end_row();
                            }
                            ui.label(if self.editing_profile.backend_type == BackendType::Injected { "Session token:" } else { "Password:" });
                            ui.add(egui::TextEdit::singleline(&mut self.editing_password).password(true).desired_width(240.0));
                            ui.end_row();
                        });
                        if self.editing_profile.backend_type == BackendType::Injected {
                            hint(ui, "Sign in through Injected's official page. You can also import an existing desktop session or enter a token manually.");
                            let official = self.editing_profile.host == "injected-irc.com" && self.editing_profile.port == "443" && self.editing_profile.use_ssl;
                            ui.add_enabled_ui(official && self.injected_import.receiver.is_none(), |ui| {
                                ui.horizontal(|ui| {
                                    #[cfg(web_login)]
                                    if ui.button("Sign in to Injected…").clicked() {
                                        crate::relay::injected::web_login::start(&mut self.injected_import, ui.ctx().clone());
                                    }
                                    if ui.button("Import Injected session").clicked() {
                                        self.injected_import.message = None;
                                        self.injected_import.receiver = Some(crate::relay::injected::login::start(None, ui.ctx().clone()));
                                    }
                                    if ui.button("Choose data folder…").clicked() {
                                        if let Some(folder) = rfd::FileDialog::new().set_title("Choose the Injected app data folder or its Local Storage/leveldb folder").pick_folder() {
                                            self.injected_import.message = None;
                                            self.injected_import.receiver = Some(crate::relay::injected::login::start(Some(folder), ui.ctx().clone()));
                                        }
                                    }
                                });
                            });
                            if !official { hint(ui, "Session import is available for injected-irc.com with TLS on port 443. Custom endpoints use a manually entered token."); }
                            if self.injected_import.receiver.is_some() {
                                ui.label("Waiting for Injected sign-in or session verification…");
                                if ui.button("Cancel sign-in").clicked() { self.injected_import = Default::default(); }
                            }
                            if let Some(message) = &self.injected_import.message { hint(ui, message); }
                            hint(ui, "The masked token field also accepts a token manually. Save token in keychain keeps it for future connections.");
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.editing_profile.use_ssl, "SSL");
                            if self.editing_profile.use_ssl && self.editing_profile.backend_type != BackendType::Injected {
                                ui.checkbox(&mut self.editing_profile.accept_invalid_certs, "Accept self-signed");
                            }
                            ui.add_space(12.0);
                            if self.editing_profile.backend_type != BackendType::Injected { ui.checkbox(&mut self.editing_profile.ssh_enabled, "SSH Tunnel"); }
                        });
                        if self.editing_profile.sends_password_in_clear() {
                            ui.label(RichText::new(format!("{} Without SSL the password is sent unencrypted, and anyone on the network between you and the server can read it. Turn SSL on, or use an SSH tunnel.", icons::WARNING))
                                .small().color(Color32::from_rgb(230, 150, 40)));
                        }
                        if self.editing_profile.backend_type != BackendType::Injected && self.editing_profile.ssh_enabled {
                            self.ssh_tunnel_form(ui);
                        }
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.editing_profile.auto_connect, "Auto-connect");
                            ui.checkbox(&mut self.editing_profile.auto_reconnect, "Auto-reconnect");
                            ui.checkbox(&mut self.editing_profile.save_password, if self.editing_profile.backend_type == BackendType::Injected { "Save token in keychain" } else { "Save password" });
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.add(
                                egui::Button::new(RichText::new("Save").strong().color(Color32::WHITE))
                                    .fill(accent_color).min_size(Vec2::new(80.0, 28.0))
                            ).clicked() {
                                match self.save_edited_profile() {
                                    Ok(()) => {
                                        self.injected_import = Default::default();
                                        self.conn_show_add = false;
                                        self.editing_profile = ConnectionProfile::default();
                                        self.editing_password.clear();
                                        self.editing_ssh_password.clear();
                                        self.editing_profile_idx = None;
                                        if let Ok(mut t) = self.ssh_test.lock() { *t = None; }
                                    }
                                    Err(e) => self.profile_save_error = Some(e),
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.injected_import = Default::default();
                                self.conn_show_add = false;
                                self.editing_profile = ConnectionProfile::default();
                                self.editing_password.clear();
                                self.editing_ssh_password.clear();
                                self.editing_profile_idx = None;
                                self.profile_save_error = None;
                                if let Ok(mut t) = self.ssh_test.lock() { *t = None; }
                            }
                        });
                        if let Some(error) = &self.profile_save_error {
                            ui.label(RichText::new(format!("{} {error}", icons::WARNING)).small().color(Color32::from_rgb(230, 150, 40)));
                        }
                    });
            } else {
                if self.profiles.is_empty() {
                    hint(ui, "No connections yet. Add a WeeChat relay or an IRC server to get started.");
                    ui.add_space(8.0);
                }
                for (idx, profile) in self.profiles.iter().enumerate() {
                    let prefix = profile.prefix();
                    let conn_opt = self.connections.iter().find(|c| c.prefix == prefix);
                    let is_connected = conn_opt.map(|c| c.client.is_connected()).unwrap_or(false);
                    let is_pending = conn_opt.map(|c| c.connecting_pending).unwrap_or(false);
                    let has_error = conn_opt.and_then(|c| c.auth_error.as_ref()).is_some();
                    let dot_color = if is_connected && !is_pending {
                        Color32::from_rgb(50, 205, 50)
                    } else if is_pending {
                        Color32::from_rgb(255, 165, 0)
                    } else if has_error {
                        Color32::from_rgb(220, 60, 60)
                    } else {
                        Color32::from_gray(120)
                    };

                    Frame::NONE
                        .fill(p.card)
                        .corner_radius(CornerRadius::same(6))
                        .stroke(Stroke::new(1.0, p.border))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").color(dot_color).size(11.0));
                                ui.label(RichText::new(&profile.label).strong());
                                let is_znc = is_connected && conn_opt.is_some_and(|c| c.client.is_znc());
                                let badge = match profile.backend_type {
                                    BackendType::Injected => "Injected",
                                    BackendType::WeeChat => "WeeChat",
                                    BackendType::Soju if is_znc => "ZNC",
                                    BackendType::Soju => "IRC",
                                };
                                ui.label(RichText::new(badge).small().color(accent_color));
                                ui.label(RichText::new(format!("{}:{}", profile.host, profile.port)).small().color(ui.visuals().weak_text_color()));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button(icons::X).on_hover_text("Delete").clicked() { do_delete = Some(idx); }
                                    if ui.small_button(icons::PENCIL_SIMPLE).on_hover_text("Edit").clicked() { do_edit = Some(idx); }
                                    if is_connected || is_pending {
                                        if ui.button("Disconnect").clicked() { do_disconnect = Some(prefix.clone()); }
                                    } else if self.conn_connect_idx == Some(idx) {
                                        // Inline password prompt — shown only when passwords aren't cached/stored.
                                        ui.add(egui::TextEdit::singleline(&mut self.editing_password).password(true).hint_text(if profile.backend_type == BackendType::Injected { "Session token…" } else { "Password…" }).desired_width(110.0));
                                        if let Some(what) = profile.ssh_secret_needed() {
                                            ui.add(egui::TextEdit::singleline(&mut self.editing_ssh_password).password(true).hint_text(format!("{what}…")).desired_width(110.0));
                                        }
                                        if ui.button("OK").clicked() {
                                            if !self.editing_ssh_password.is_empty() {
                                                self.session_ssh_passwords.insert(prefix.clone(), std::mem::take(&mut self.editing_ssh_password));
                                            }
                                            do_connect_idx = Some(idx);
                                        }
                                        if ui.button(icons::X).clicked() {
                                            self.conn_connect_idx = None;
                                            self.editing_password.clear();
                                            self.editing_ssh_password.clear();
                                        }
                                    } else if ui.button("Connect").clicked() {
                                        // Check session cache, then keyring — skip prompt if password found.
                                        let relay_pw = self.session_passwords.get(&prefix).cloned()
                                            .or_else(|| crate::ui::secure_storage::load_by_key(&profile.keyring_host_key()));
                                        let ssh_pw_known = profile.ssh_secret_needed().is_none()
                                            || self.session_ssh_passwords.contains_key(&prefix)
                                            || profile.ssh_save_password;
                                        if relay_pw.is_some() && ssh_pw_known {
                                            self.editing_password = relay_pw.unwrap_or_default();
                                            do_connect_idx = Some(idx);
                                        } else {
                                            self.editing_password = relay_pw.unwrap_or_default();
                                            self.conn_connect_idx = Some(idx);
                                        }
                                    }
                                });
                            });
                            if let Some(err) = conn_opt.and_then(|c| c.auth_error.as_ref()) {
                                ui.label(RichText::new(format!("{} {}", icons::WARNING, err)).small().color(Color32::from_rgb(220, 80, 80)));
                            }
                            if let Some(conn) = conn_opt.filter(|c| is_connected && c.client.is_znc()) {
                                let user = crate::ui::znc::znc_user(&profile.username, &profile.nick);
                                let networks = conn.client.znc_networks();
                                for (module, text) in crate::ui::znc::networks_panel(
                                    ui, &mut self.znc_ui, &prefix, &user, &networks, conn.znc_reply.as_ref(), accent_color,
                                ) {
                                    znc_commands.push((prefix.clone(), module, text));
                                }
                            }
                        });
                    ui.add_space(4.0);
                }

                ui.add_space(4.0);
                if ui.add(
                    egui::Button::new(RichText::new("+ Add Connection").color(accent_color))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, accent_color))
                        .min_size(Vec2::new(0.0, 30.0))
                ).clicked() {
                    toggle_add = true;
                }
            }
        });

        // Process deferred actions
        for (prefix, module, text) in znc_commands {
            if let Some(conn) = self.connections.iter().find(|c| c.prefix == prefix) {
                conn.client.znc_command(&module, &text);
            }
        }
        if let Some(idx) = do_connect_idx {
            if idx < self.profiles.len() {
                let profile = self.profiles[idx].clone();
                let password = std::mem::take(&mut self.editing_password);
                self.conn_connect_idx = None;
                if !self.editing_ssh_password.is_empty() {
                    self.session_ssh_passwords.insert(profile.prefix(), std::mem::take(&mut self.editing_ssh_password));
                }
                self.do_connect(&profile, password, ui.ctx());
            }
        }
        if let Some(prefix) = do_disconnect {
            if let Some(conn) = self.connections.iter_mut().find(|c| c.prefix == prefix) {
                conn.client.disconnect();
                // Asked for: not a failed login, and nothing to retry. Connect makes a
                // new handle, so this one's flags don't outlive it.
                conn.connecting_pending = false;
                conn.auth_error = None;
                conn.auto_reconnect = false;
            }
        }
        if let Some(idx) = do_delete {
            if idx < self.profiles.len() {
                let gone = self.profiles.remove(idx);
                // Its passwords go with it.
                for key in [gone.keyring_host_key(), gone.ssh_keyring_key()] {
                    if let Err(e) = crate::ui::secure_storage::delete_by_key(&key) {
                        log::warn!("removing a deleted connection's password from the keychain: {e}");
                    }
                }
                self.session_passwords.remove(&gone.prefix());
                self.session_ssh_passwords.remove(&gone.prefix());
                if self.conn_connect_idx == Some(idx) { self.conn_connect_idx = None; }
            }
        }
        if let Some(idx) = do_edit {
            if idx < self.profiles.len() {
                let profile = &self.profiles[idx];
                let mut keychain_error = None;
                let mut load = |key: String| match crate::ui::secure_storage::load_by_key_checked(&key) {
                    Ok(secret) => secret.unwrap_or_default(),
                    Err(e) => { keychain_error = Some(e); String::new() }
                };
                self.editing_password = load(profile.keyring_host_key());
                self.editing_ssh_password = load(profile.ssh_keyring_key());
                self.profile_save_error = keychain_error.map(|e| format!("The keychain couldn't be read, so the saved password isn't shown: {e}"));
                self.injected_import = Default::default();
                self.editing_profile = profile.clone();
                self.editing_profile_idx = Some(idx);
                self.conn_show_add = true;
            }
        }
        if toggle_add {
            self.injected_import = Default::default();
            self.editing_profile = ConnectionProfile::default();
            self.editing_password.clear();
            self.editing_ssh_password.clear();
            self.editing_profile_idx = None;
            self.conn_show_add = true;
        }
    }
}
