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
                        SettingsSection::FileSharing => self.file_sharing_section(ui),
                        SettingsSection::Keybinds => self.keybinds_section(ui, p),
                    }
                    ui.add_space(12.0);
                });
            });
    }

    fn general_section(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Connections");
        ui.checkbox(&mut self.auto_reconnect, "Auto-reconnect when a connection drops");

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
        ui.checkbox(&mut self.show_filtered_lines, "Show filtered lines");
        ui.checkbox(&mut self.colored_nicks, "Coloured nicknames in the nick list");
        ui.checkbox(&mut self.emoji_rendering, "Colour emoji rendering");
        hint(ui, "Requires the emojinize script on the WeeChat server.");

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
        let mut new_font: Option<(String, String)> = None;
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
                    new_font = Some((String::new(), String::new()));
                }
                ui.separator();
                let query = self.font_search.to_lowercase();
                for (name, path) in &self.available_fonts {
                    if query.is_empty() || name.to_lowercase().contains(&query) {
                        if ui.selectable_label(name == &self.font_name, name.as_str()).clicked() {
                            new_font = Some((name.clone(), path.clone()));
                        }
                    }
                }
            });
        if combo_resp.inner.is_none() {
            self.font_search.clear();
        }
        if ui.add(
            egui::Button::new(RichText::new("Browse…").color(p.accent))
                .fill(p.secondary_fill)
                .stroke(p.secondary_stroke)
        ).clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Font files", &["ttf", "otf", "ttc"]).pick_file() {
                let path_str = path.to_string_lossy().into_owned();
                let name = fonts::family_from_file(&path_str)
                    .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                new_font = Some((name, path_str));
            }
        }
        if let Some((name, path)) = new_font {
            self.font_name = name;
            self.font_path = path;
            self.font_search.clear();
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

    fn theme_section(&mut self, ui: &mut egui::Ui, p: &Palette) {
        section_heading(ui, "Colour scheme");
        ui.add_enabled_ui(!self.adaptive_theme, |ui| {
            ui.horizontal(|ui| {
                self.theme_picker(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(
                        egui::Button::new(RichText::new("Reset").color(Color32::WHITE))
                            .fill(p.danger)
                            .min_size(Vec2::new(70.0, 24.0))
                    ).clicked() {
                        self.theme = crate::ui::theme::default_theme();
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
        ui.checkbox(&mut adaptive_theme, "Adaptive theme (from wallpaper)");
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
                                    hint(ui, "Leave blank to disable SASL");
                                });
                                ui.end_row();
                                ui.label("Channel:");
                                ui.add(egui::TextEdit::singleline(&mut self.editing_profile.channel).desired_width(240.0).hint_text("#channel (optional)"));
                                ui.end_row();
                            }
                            ui.label(if self.editing_profile.backend_type == BackendType::Injected { "Session token:" } else { "Password:" });
                            ui.add(egui::TextEdit::singleline(&mut self.editing_password).password(true).desired_width(240.0));
                            ui.end_row();
                        });
                        if self.editing_profile.backend_type == BackendType::Injected {
                            hint(ui, "Sign in through Injected’s official page. You can also import an existing desktop session or enter a token manually.");
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
                            ui.hyperlink_to("Open Injected to sign in", "https://injected-irc.com");
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
                        if self.editing_profile.backend_type != BackendType::Injected && self.editing_profile.ssh_enabled {
                            ui.add_space(4.0);
                            egui::Grid::new("ssh_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                                ui.label("SSH Host:");
                                ui.add(egui::TextEdit::singleline(&mut self.editing_profile.ssh_host)
                                    .desired_width(240.0)
                                    .hint_text("user@host or host"));
                                ui.end_row();
                                ui.label("SSH Port:");
                                let mut ssh_port_str = self.editing_profile.ssh_port.map(|p| p.to_string()).unwrap_or_default();
                                if ui.add(egui::TextEdit::singleline(&mut ssh_port_str).desired_width(80.0).hint_text("22")).changed() {
                                    self.editing_profile.ssh_port = if ssh_port_str.is_empty() { None } else { ssh_port_str.parse::<u16>().ok() };
                                }
                                ui.end_row();
                                ui.label("SSH User:");
                                ui.add(egui::TextEdit::singleline(&mut self.editing_profile.ssh_user)
                                    .desired_width(240.0)
                                    .hint_text("leave blank to use SSH config / system user"));
                                ui.end_row();
                                ui.label("SSH Password:");
                                ui.add(egui::TextEdit::singleline(&mut self.editing_ssh_password)
                                    .password(true)
                                    .desired_width(240.0)
                                    .hint_text("leave blank to use SSH agent / keys"));
                                ui.end_row();
                            });
                            ui.checkbox(&mut self.editing_profile.ssh_save_password, "Save SSH password");
                            ui.add_space(2.0);
                            hint(ui, "Connects via an ssh -L tunnel using your SSH agent / ~/.ssh keys.\nThe relay host/port above is where the tunnel forwards to.");
                            ui.add_space(4.0);
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
                                if self.editing_profile.save_password && !self.editing_password.is_empty() {
                                    let key = self.editing_profile.keyring_host_key();
                                    let _ = crate::ui::secure_storage::save_by_key(&key, &self.editing_password);
                                }
                                if self.editing_profile.ssh_save_password && !self.editing_ssh_password.is_empty() {
                                    let key = self.editing_profile.ssh_keyring_key();
                                    let _ = crate::ui::secure_storage::save_by_key(&key, &self.editing_ssh_password);
                                }
                                // Cache passwords for the session so connect skips prompts.
                                let pfx = self.editing_profile.prefix();
                                if !self.editing_password.is_empty() {
                                    self.session_passwords.insert(pfx.clone(), self.editing_password.clone());
                                }
                                if !self.editing_ssh_password.is_empty() {
                                    self.session_ssh_passwords.insert(pfx, self.editing_ssh_password.clone());
                                }
                                let profile = self.editing_profile.clone();
                                if let Some(idx) = self.editing_profile_idx {
                                    if idx < self.profiles.len() { self.profiles[idx] = profile; }
                                } else {
                                    self.profiles.push(profile);
                                }
                                self.injected_import = Default::default();
                                self.conn_show_add = false;
                                self.editing_profile = ConnectionProfile::default();
                                self.editing_password.clear();
                                self.editing_ssh_password.clear();
                                self.editing_profile_idx = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.injected_import = Default::default();
                                self.conn_show_add = false;
                                self.editing_profile = ConnectionProfile::default();
                                self.editing_password.clear();
                                self.editing_ssh_password.clear();
                                self.editing_profile_idx = None;
                            }
                        });
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
                                        if profile.ssh_enabled {
                                            ui.add(egui::TextEdit::singleline(&mut self.editing_ssh_password).password(true).hint_text("SSH password…").desired_width(110.0));
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
                                        let ssh_pw_known = !profile.ssh_enabled
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
                self.profiles.remove(idx);
                if self.conn_connect_idx == Some(idx) { self.conn_connect_idx = None; }
            }
        }
        if let Some(idx) = do_edit {
            if idx < self.profiles.len() {
                let profile = &self.profiles[idx];
                self.editing_password = crate::ui::secure_storage::load_by_key(&profile.keyring_host_key()).unwrap_or_default();
                self.editing_ssh_password = crate::ui::secure_storage::load_by_key(&profile.ssh_keyring_key()).unwrap_or_default();
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
