use crate::relay::client::RelayClient;
use crate::relay::models::Buffer;
use crate::ui::ansi::ANSIParser;
use crate::ui::theme::AppTheme;
use crate::ui::settings::AppSettings;
use crate::ui::input::CompletionState;
use egui::{
    FontId, ScrollArea, Label, Key, Visuals, TextStyle, FontFamily, Color32,
    text::LayoutJob, Margin, Frame, Rounding, Stroke, Vec2, Modifiers, Rect, Painter,
};
use tokio::sync::mpsc;
use crate::relay::client::RelayEvent;

pub(crate) const MAX_MESSAGES: usize = 400;

pub struct WeeChatApp {
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) password: String,
    pub(crate) use_ssl: bool,

    pub(crate) client: Option<RelayClient>,
    pub(crate) event_rx: mpsc::UnboundedReceiver<RelayEvent>,
    pub(crate) event_tx: mpsc::UnboundedSender<RelayEvent>,

    pub(crate) connection_status: String,
    pub(crate) is_connecting: bool,
    pub(crate) buffers: Vec<Buffer>,
    pub(crate) selected_buffer_id: Option<String>,
    pub(crate) input_text: String,
    #[allow(dead_code)]
    pub(crate) debug_log: Vec<String>,

    // Settings
    pub(crate) show_settings: bool,
    pub(crate) show_filtered_lines: bool,
    pub(crate) colored_nicks: bool,
    pub(crate) theme: AppTheme,
    pub(crate) font_size: f32,
    pub(crate) use_monospace: bool,
    pub(crate) show_timestamps: bool,
    pub(crate) auto_reconnect: bool,
    pub(crate) show_titlebar: bool,
    pub(crate) opacity: f32,

    // UI visibility
    pub(crate) show_buffers: bool,
    pub(crate) show_nicklist: bool,

    // Completion state
    pub(crate) completion: Option<CompletionState>,

    // Command history
    pub(crate) command_history: Vec<String>,
    pub(crate) history_index: Option<usize>,

    // Search state
    pub(crate) show_search: bool,
    pub(crate) search_text: String,

    // Navigation
    pub(crate) pending_buffer_switch: Option<String>,
}

impl WeeChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let settings: AppSettings = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            AppSettings::default()
        };

        Self {
            host: settings.host,
            port: settings.port,
            password: String::new(),
            use_ssl: settings.use_ssl,
            client: None,
            event_rx,
            event_tx,
            connection_status: String::new(),
            is_connecting: false,
            buffers: Vec::new(),
            selected_buffer_id: None,
            input_text: String::new(),
            debug_log: Vec::new(),
            show_settings: false,
            show_filtered_lines: settings.show_filtered_lines,
            colored_nicks: settings.colored_nicks,
            theme: settings.theme,
            font_size: settings.font_size,
            use_monospace: settings.use_monospace,
            show_timestamps: settings.show_timestamps,
            show_buffers: settings.show_buffers,
            show_nicklist: settings.show_nicklist,
            auto_reconnect: settings.auto_reconnect,
            show_titlebar: settings.show_titlebar,
            opacity: settings.opacity,
            completion: None,
            command_history: Vec::new(),
            history_index: None,
            show_search: false,
            search_text: String::new(),
            pending_buffer_switch: None,
        }
    }

    pub(crate) fn select_buffer(&mut self, id: String) {
        self.selected_buffer_id = Some(id.clone());
        if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == id) {
            buffer.activity = crate::relay::models::BufferActivity::None;
            if let Some(client) = &self.client {
                client.send_api(&format!("GET /api/buffers/{}", id), Some(&format!("_buffer_info:{}", id)), None);
                client.send_api(&format!("GET /api/buffers/{}/lines?lines=-{}", id, MAX_MESSAGES), Some(&format!("_buffer_lines:{}", id)), None);
                client.send_api(&format!("GET /api/buffers/{}/nicks", id), Some(&format!("_nicks:{}", id)), None);
                client.send_api("POST /api/input", None, Some(serde_json::json!({
                    "buffer_id": id.parse::<i64>().unwrap_or(0),
                    "command": "/buffer set localvar_set_read_marker 1"
                })));
            }
        }
    }

    fn hash_nick(name: &str) -> u8 {
        let mut h: u32 = 0;
        for b in name.as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(*b as u32);
        }
        ((h % 15) + 1) as u8
    }

    fn draw_sidebar_icon(painter: &Painter, rect: Rect, color: Color32, is_right: bool) {
        let stroke = Stroke::new(1.5, color);
        let rounding = Rounding::same(2.0);
        painter.rect_stroke(rect.shrink(4.0), rounding, stroke);
        let split_x = if is_right { rect.right() - 8.0 } else { rect.left() + 8.0 };
        painter.line_segment(
            [egui::pos2(split_x, rect.top() + 4.0), egui::pos2(split_x, rect.bottom() - 4.0)],
            stroke,
        );
    }
}

impl eframe::App for WeeChatApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = AppSettings {
            host: self.host.clone(),
            port: self.port.clone(),
            use_ssl: self.use_ssl,
            show_filtered_lines: self.show_filtered_lines,
            colored_nicks: self.colored_nicks,
            theme: self.theme.clone(),
            font_size: self.font_size,
            use_monospace: self.use_monospace,
            show_timestamps: self.show_timestamps,
            show_buffers: self.show_buffers,
            show_nicklist: self.show_nicklist,
            auto_reconnect: self.auto_reconnect,
            show_titlebar: self.show_titlebar,
            opacity: self.opacity,
        };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event);
        }

        let mut tab_pressed = false;
        let mut arrow_up_shortcut = false;
        let mut arrow_down_shortcut = false;
        let mut history_up = false;
        let mut history_down = false;
        let mut search_shortcut = false;

        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Tab) { tab_pressed = true; }

            let meta = i.modifiers.command || i.modifiers.alt || i.modifiers.mac_cmd || i.modifiers.ctrl;
            if meta {
                if i.consume_key(i.modifiers, Key::ArrowUp) || i.key_pressed(Key::ArrowUp) { arrow_up_shortcut = true; }
                if i.consume_key(i.modifiers, Key::ArrowDown) || i.key_pressed(Key::ArrowDown) { arrow_down_shortcut = true; }
                if i.consume_key(i.modifiers, Key::F) { search_shortcut = true; }
                if i.consume_key(i.modifiers, Key::B) { self.show_buffers = !self.show_buffers; }
                if i.consume_key(i.modifiers, Key::N) { self.show_nicklist = !self.show_nicklist; }
            } else {
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) { history_up = true; }
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) { history_down = true; }
            }
        });

        if arrow_up_shortcut { self.cycle_buffer(-1); }
        if arrow_down_shortcut { self.cycle_buffer(1); }
        if search_shortcut { self.show_search = !self.show_search; }

        // Apply font and style settings
        let mut style = (*ctx.style()).clone();
        let font_family = if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional };
        style.text_styles = [
            (TextStyle::Small,    FontId::new(self.font_size * 0.8, font_family.clone())),
            (TextStyle::Body,     FontId::new(self.font_size,       font_family.clone())),
            (TextStyle::Button,   FontId::new(self.font_size,       font_family.clone())),
            (TextStyle::Heading,  FontId::new(self.font_size * 1.4, font_family.clone())),
            (TextStyle::Monospace,FontId::new(self.font_size,       FontFamily::Monospace)),
        ].into();
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
        style.spacing.window_margin = Margin::same(12.0);
        style.visuals.window_rounding = Rounding::same(12.0);
        style.visuals.widgets.noninteractive.rounding = Rounding::same(8.0);
        style.visuals.widgets.inactive.rounding = Rounding::same(8.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(8.0);
        style.visuals.widgets.active.rounding = Rounding::same(8.0);
        ctx.set_style(style);

        let accent_color = Color32::from_rgb(100, 149, 237);
        let base_bg = self.theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18));
        let alpha = (self.opacity * 255.0) as u8;
        let bg_color = Color32::from_rgba_unmultiplied(base_bg.r(), base_bg.g(), base_bg.b(), alpha);
        let surface_color = Color32::from_rgba_unmultiplied(30, 30, 30, alpha);

        let mut visuals = Visuals::dark();
        visuals.panel_fill = bg_color;
        visuals.window_fill = bg_color;
        visuals.extreme_bg_color = Color32::from_rgba_unmultiplied(10, 10, 10, alpha);
        visuals.widgets.active.bg_fill = accent_color;
        visuals.selection.bg_fill = accent_color.linear_multiply(0.5);
        if let Some(fg) = self.theme.foreground {
            visuals.override_text_color = Some(fg.into());
        }
        ctx.set_visuals(visuals);

        // ── Top bar ──────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("top_panel")
            .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(12.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    let icon_size = Vec2::splat(24.0);

                    let (rect, res) = ui.allocate_at_least(icon_size, egui::Sense::click());
                    if res.clicked() { self.show_buffers = !self.show_buffers; }
                    Self::draw_sidebar_icon(ui.painter(), rect, if self.show_buffers { accent_color } else { Color32::GRAY }, false);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, res) = ui.allocate_at_least(icon_size, egui::Sense::click());
                        if res.clicked() { self.show_nicklist = !self.show_nicklist; }
                        Self::draw_sidebar_icon(ui.painter(), rect, if self.show_nicklist { accent_color } else { Color32::GRAY }, true);
                        ui.add_space(8.0);

                        if ui.button(egui::RichText::new("⚙").size(16.0)).on_hover_text("Settings").clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        if self.client.is_some() {
                            let (status_text, status_color) = if self.is_connecting {
                                ("● Connecting", Color32::from_rgb(255, 165, 0))
                            } else {
                                ("● Connected", Color32::from_rgb(50, 205, 50))
                            };
                            ui.label(egui::RichText::new(status_text).color(status_color).small());
                            if ui.button("Disconnect").clicked() {
                                if let Some(client) = &self.client { client.disconnect(); }
                                self.client = None;
                                self.connection_status = "Disconnected".to_string();
                            }
                        }
                    });
                });
            });

        // ── Buffer list (left sidebar) ────────────────────────────────────────
        let mut next_selected_buffer_id: Option<String> = None;

        if self.show_buffers {
            egui::SidePanel::left("buffers_panel")
                .resizable(true)
                .default_width(180.0)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("BUFFERS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for buffer in &self.buffers {
                            let is_selected = self.selected_buffer_id.as_deref() == Some(&buffer.id);
                            let (bg, fg) = if is_selected {
                                (accent_color.linear_multiply(0.2), Color32::WHITE)
                            } else {
                                match buffer.activity {
                                    crate::relay::models::BufferActivity::Highlight => (Color32::from_rgb(150, 50, 50).linear_multiply(0.3), Color32::from_rgb(255, 100, 100)),
                                    crate::relay::models::BufferActivity::Message   => (Color32::TRANSPARENT, Color32::WHITE),
                                    crate::relay::models::BufferActivity::Metadata  => (Color32::TRANSPARENT, Color32::from_rgb(130, 130, 130)),
                                    crate::relay::models::BufferActivity::None      => (Color32::TRANSPARENT, Color32::from_rgb(100, 100, 100)),
                                }
                            };

                            let is_child = buffer.kind == "channel" || buffer.kind == "private";
                            let indent = if is_child { 12.0 } else { 0.0 };

                            ui.horizontal(|ui| {
                                ui.add_space(indent);
                                let response = Frame::none()
                                    .fill(bg)
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(Margin::symmetric(8.0, 4.0))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        let text = if buffer.activity == crate::relay::models::BufferActivity::Highlight {
                                            format!("• {}", buffer.name)
                                        } else {
                                            buffer.name.clone()
                                        };
                                        ui.label(egui::RichText::new(text).color(fg).strong());
                                    }).response;
                                let response = ui.interact(response.rect, response.id, egui::Sense::click());
                                if response.clicked() {
                                    next_selected_buffer_id = Some(buffer.id.clone());
                                }
                            });
                        }
                    });
                });
        }

        if let Some(id) = next_selected_buffer_id {
            self.select_buffer(id);
        }

        // ── Settings window ───────────────────────────────────────────────────
        if self.show_settings {
            let mut show_settings = self.show_settings;
            let mut show_filtered_lines = self.show_filtered_lines;
            let mut colored_nicks = self.colored_nicks;
            let mut font_size = self.font_size;
            let mut use_monospace = self.use_monospace;
            let mut show_timestamps = self.show_timestamps;
            let mut auto_reconnect = self.auto_reconnect;
            let mut show_titlebar = self.show_titlebar;
            let mut opacity = self.opacity;
            let mut close_clicked = false;
            let mut reset_theme = false;

            egui::Window::new("Settings")
                .open(&mut show_settings)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.checkbox(&mut show_filtered_lines, "Show filtered lines");
                    ui.checkbox(&mut colored_nicks, "Colored nicknames in list");
                    ui.checkbox(&mut show_timestamps, "Show timestamps");
                    ui.checkbox(&mut auto_reconnect, "Auto-reconnect on drop");
                    ui.checkbox(&mut show_titlebar, "Show Topic/Modes Titlebar");

                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("Appearance").strong());
                    ui.horizontal(|ui| {
                        ui.label("Font size:");
                        ui.add(egui::Slider::new(&mut font_size, 8.0..=32.0));
                    });
                    ui.checkbox(&mut use_monospace, "Use Monospace font everywhere");
                    ui.horizontal(|ui| {
                        ui.label("Opacity:");
                        ui.add(egui::Slider::new(&mut opacity, 0.1..=1.0));
                    });

                    ui.separator();
                    ui.label(egui::RichText::new("Theme").strong());
                    ui.label(format!("Current: {}", self.theme.name));
                    ui.horizontal(|ui| {
                        if ui.button("Import .itermcolors").clicked() {
                            if let Some(path) = rfd::FileDialog::new().add_filter("itermcolors", &["itermcolors"]).pick_file() {
                                if let Ok(data) = std::fs::read(&path) {
                                    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                                    if let Ok(new_theme) = AppTheme::parse_itermcolors(&data, name) {
                                        self.theme = new_theme;
                                    }
                                }
                            }
                        }
                        if ui.button("Reset to Default").clicked() { reset_theme = true; }
                    });

                    ui.add_space(20.0);
                    ui.vertical_centered_justified(|ui| {
                        if ui.button("Close").clicked() { close_clicked = true; }
                    });
                });

            self.show_settings = if close_clicked { false } else { show_settings };
            self.show_filtered_lines = show_filtered_lines;
            self.colored_nicks = colored_nicks;
            self.font_size = font_size;
            self.use_monospace = use_monospace;
            self.show_timestamps = show_timestamps;
            self.auto_reconnect = auto_reconnect;
            self.show_titlebar = show_titlebar;
            self.opacity = opacity;
            if reset_theme { self.theme = AppTheme::default(); }
        }

        // Snapshot current buffer data once for rendering (avoids repeated linear searches)
        let current_buffer = self.selected_buffer_id.as_ref()
            .and_then(|id| self.buffers.iter().find(|b| &b.id == id));

        let current_buffer_nicks        = current_buffer.map(|b| b.nicks.clone());
        let current_buffer_full_name    = current_buffer.map(|b| b.full_name.clone());
        let current_buffer_messages     = current_buffer.map(|b| b.messages.clone());
        let current_buffer_last_read_id = current_buffer.and_then(|b| b.last_read_id.clone());
        let current_buffer_topic        = current_buffer.map(|b| b.topic.clone()).unwrap_or_default();
        let current_buffer_modes        = current_buffer.map(|b| b.modes.clone()).unwrap_or_default();
        let current_buffer_kind         = current_buffer.map(|b| b.kind.clone()).unwrap_or_default();

        let font_id = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });
        let is_query_or_core = current_buffer_kind == "private"
            || current_buffer_kind == "server"
            || current_buffer_full_name.as_deref() == Some("weechat");

        // ── Nick list (right sidebar) ─────────────────────────────────────────
        if self.show_nicklist && !is_query_or_core && self.client.is_some() && self.selected_buffer_id.is_some() {
            egui::SidePanel::right("nicks_panel")
                .resizable(true)
                .default_width(140.0)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("NICKS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);
                    ScrollArea::vertical().show(ui, |ui| {
                        if let Some(nicks) = &current_buffer_nicks {
                            for nick in nicks {
                                let text = format!("{}{}", nick.prefix, nick.name);
                                let input = if self.colored_nicks {
                                    if self.theme.name == "Default" {
                                        format!("{}{}", nick.color_ansi, text)
                                    } else {
                                        let idx = Self::hash_nick(&nick.name);
                                        let esc = if idx < 8 {
                                            format!("\x1B[{}m", 30 + idx)
                                        } else {
                                            format!("\x1B[{}m", 90 + idx - 8)
                                        };
                                        format!("{}{}", esc, text)
                                    }
                                } else {
                                    text
                                };

                                let sections = ANSIParser::parse(&input, font_id.clone(), &self.theme);
                                let mut job = LayoutJob::default();
                                for s in sections { job.append(&s.text, 0.0, s.format); }

                                let label_res = ui.add(Label::new(job).wrap(false).sense(egui::Sense::click()));
                                label_res.context_menu(|ui| {
                                    if ui.button(format!("Query {}", nick.name)).clicked() {
                                        self.send_command(&format!("/query {}", nick.name));
                                        ui.close_menu();
                                    }
                                    if ui.button(format!("Whois {}", nick.name)).clicked() {
                                        self.send_command(&format!("/whois {}", nick.name));
                                        ui.close_menu();
                                    }
                                });
                            }
                        }
                    });
                });
        }

        // ── Input bar (bottom) ────────────────────────────────────────────────
        if self.client.is_some() && self.selected_buffer_id.is_some() {
            egui::TopBottomPanel::bottom("input_panel")
                .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(16.0, 10.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let text_edit = egui::TextEdit::singleline(&mut self.input_text)
                            .hint_text("Type a message...")
                            .margin(Margin::symmetric(8.0, 4.0))
                            .lock_focus(true)
                            .desired_width(ui.available_width() - 80.0);

                        let res = ui.add(text_edit);

                        if res.has_focus() {
                            if tab_pressed {
                                self.perform_completion(ctx, res.id);
                                res.request_focus();
                            } else if history_up {
                                self.cycle_history(-1, ctx, res.id);
                                res.request_focus();
                            } else if history_down {
                                self.cycle_history(1, ctx, res.id);
                                res.request_focus();
                            } else {
                                let any_other_key = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
                                if any_other_key && !tab_pressed && !history_up && !history_down {
                                    self.completion = None;
                                }
                            }
                        }

                        if ui.add(egui::Button::new("Send").min_size(Vec2::new(60.0, 0.0))).clicked()
                            || (res.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            self.send_current_message();
                            res.request_focus();
                        }
                    });
                });
        }

        // ── Central panel (messages / connect screen) ─────────────────────────
        egui::CentralPanel::default()
            .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                if self.client.is_none() {
                    // Connect screen
                    ui.vertical_centered(|ui| {
                        ui.add_space(ctx.available_rect().height() * 0.2);
                        Frame::group(ui.style())
                            .fill(surface_color)
                            .rounding(Rounding::same(12.0))
                            .stroke(Stroke::new(1.0, Color32::from_gray(60)))
                            .inner_margin(Margin::same(30.0))
                            .show(ui, |ui| {
                                ui.set_max_width(400.0);
                                ui.heading(egui::RichText::new("Connect to WeeChatRS").strong().size(24.0));
                                ui.add_space(20.0);

                                egui::Grid::new("login_grid").num_columns(2).spacing([15.0, 15.0]).show(ui, |ui| {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Host:"); });
                                    ui.add(egui::TextEdit::singleline(&mut self.host).desired_width(240.0));
                                    ui.end_row();
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Port:"); });
                                    ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(240.0));
                                    ui.end_row();
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Password:"); });
                                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(240.0));
                                    ui.end_row();
                                });

                                ui.add_space(15.0);
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut self.use_ssl, "Use SSL");
                                    ui.add_space(20.0);
                                    ui.checkbox(&mut self.auto_reconnect, "Auto-reconnect");
                                });
                                ui.add_space(25.0);

                                ui.horizontal(|ui| {
                                    if ui.add(egui::Button::new(egui::RichText::new("Connect").strong()).min_size(Vec2::new(120.0, 40.0))).clicked() {
                                        let port = self.port.parse().unwrap_or(9001);
                                        self.client = Some(RelayClient::connect(
                                            self.host.clone(), port, self.password.clone(),
                                            self.use_ssl, self.event_tx.clone(),
                                        ));
                                        self.connection_status = "Connecting...".to_string();
                                    }
                                    if ui.button("Save Profile").clicked() {
                                        ctx.memory_mut(|m| m.data.insert_persisted(egui::Id::NULL, ()));
                                    }
                                });

                                ui.add_space(15.0);
                                if !self.connection_status.is_empty() {
                                    let color = if self.connection_status.starts_with("Error") {
                                        Color32::from_rgb(255, 100, 100)
                                    } else {
                                        accent_color
                                    };
                                    ui.label(egui::RichText::new(&self.connection_status).color(color));
                                }
                            });
                    });
                } else if current_buffer_full_name.is_some() {
                    // Message view
                    ui.vertical(|ui| {
                        if self.show_search {
                            Frame::none()
                                .fill(surface_color.linear_multiply(0.8))
                                .inner_margin(Margin::symmetric(16.0, 8.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("🔍");
                                        let res = ui.add(egui::TextEdit::singleline(&mut self.search_text)
                                            .hint_text("Search scrollback...")
                                            .desired_width(ui.available_width() - 40.0));
                                        if ui.button("❌").clicked() {
                                            self.show_search = false;
                                            self.search_text.clear();
                                        }
                                        if self.show_search { res.request_focus(); }
                                    });
                                });
                            ui.separator();
                        }

                        if self.show_titlebar && (!current_buffer_topic.is_empty() || !current_buffer_modes.is_empty()) {
                            Frame::none()
                                .fill(surface_color.linear_multiply(0.3))
                                .inner_margin(Margin::symmetric(16.0, 6.0))
                                .stroke(Stroke::new(1.0, Color32::from_white_alpha(10)))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_wrapped(|ui| {
                                        if !current_buffer_modes.is_empty() {
                                            ui.label(egui::RichText::new(format!("[{}]", current_buffer_modes)).color(accent_color).small());
                                        }
                                        if !current_buffer_topic.is_empty() {
                                            let topic_font = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });
                                            let sections = ANSIParser::parse(&current_buffer_topic, topic_font, &self.theme);
                                            let mut job = LayoutJob::default();
                                            for s in sections { job.append(&s.text, 0.0, s.format); }
                                            ui.add(Label::new(job).wrap(true));
                                        }
                                    });
                                });
                            ui.add_space(-1.0);
                            ui.separator();
                        }

                        ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            Frame::none().inner_margin(Margin::same(16.0)).show(ui, |ui| {
                                if let Some(messages) = &current_buffer_messages {
                                    let mut marker_shown = false;
                                    for line in messages {
                                        if !self.show_filtered_lines && !line.displayed { continue; }

                                        if !self.search_text.is_empty() {
                                            let clean_prefix = Self::strip_ansi(&line.prefix).to_lowercase();
                                            let clean_msg = Self::strip_ansi(&line.message).to_lowercase();
                                            let q = self.search_text.to_lowercase();
                                            if !clean_prefix.contains(&q) && !clean_msg.contains(&q) { continue; }
                                        }

                                        if !marker_shown {
                                            if let Some(last_read) = &current_buffer_last_read_id {
                                                if &line.id > last_read {
                                                    ui.add_space(8.0);
                                                    ui.horizontal(|ui| {
                                                        ui.add_space(20.0);
                                                        ui.separator();
                                                        ui.label(egui::RichText::new(" NEW MESSAGES ").color(Color32::from_rgb(255, 100, 100)).size(10.0).strong());
                                                        ui.separator();
                                                    });
                                                    ui.add_space(8.0);
                                                    marker_shown = true;
                                                }
                                            }
                                        }

                                        ui.horizontal_wrapped(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            if self.show_timestamps {
                                                ui.label(egui::RichText::new(line.timestamp.format("%H:%M:%S").to_string()).font(font_id.clone()).color(Color32::from_gray(100)));
                                            }
                                            let prefix_sections = ANSIParser::parse(&line.prefix, font_id.clone(), &self.theme);
                                            let mut prefix_job = LayoutJob::default();
                                            for s in prefix_sections { prefix_job.append(&s.text, 0.0, s.format); }
                                            ui.label(prefix_job);

                                            for s in ANSIParser::parse(&line.message, font_id.clone(), &self.theme) {
                                                if let Some(url) = s.url {
                                                    if ui.link(egui::RichText::new(&s.text).font(font_id.clone())).clicked() {
                                                        ui.ctx().output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url)));
                                                    }
                                                } else {
                                                    let mut job = LayoutJob::default();
                                                    job.append(&s.text, 0.0, s.format);
                                                    ui.add(Label::new(job).wrap(true));
                                                }
                                            }
                                        });
                                    }
                                }
                            });
                        });
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Select a buffer to start chatting").color(Color32::from_gray(100)).size(16.0));
                    });
                }
            });

        if self.client.is_some() { ctx.request_repaint(); }
    }
}
