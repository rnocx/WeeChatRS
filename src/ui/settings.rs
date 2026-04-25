use crate::ui::theme::AppTheme;
use crate::ui::app::WeeChatApp;
use egui::{Color32, Vec2, RichText};

impl WeeChatApp {
    pub(crate) fn show_settings_window(&mut self, ctx: &egui::Context, accent_color: Color32, is_light: bool) {
        let mut show_settings = self.show_settings;
        let mut show_filtered_lines = self.show_filtered_lines;
        let mut colored_nicks = self.colored_nicks;
        let mut font_size = self.font_size;
        let mut use_monospace = self.use_monospace;
        let mut show_timestamps = self.show_timestamps;
        let mut auto_reconnect = self.auto_reconnect;
        let mut show_titlebar = self.show_titlebar;
        let mut show_server_headers = self.show_server_headers;
        let mut show_inline_images = self.show_inline_images;
        let mut show_link_previews = self.show_link_previews;
        let mut show_hidden_buffers = self.show_hidden_buffers;
        let mut opacity = self.opacity;
        let mut close_clicked = false;
        let mut reset_theme = false;

        // Danger red that works on both light and dark backgrounds
        let danger_color = Color32::from_rgb(185, 55, 55);
        // Subtle variant for secondary actions
        let secondary_fill = if is_light {
            Color32::from_rgba_unmultiplied(100, 149, 237, 30)
        } else {
            Color32::from_rgba_unmultiplied(100, 149, 237, 40)
        };
        let secondary_stroke = egui::Stroke::new(1.0, accent_color.linear_multiply(0.6));

        // Build an opaque frame regardless of the application opacity setting.
        let opaque_fill = if is_light {
            Color32::from_gray(235)
        } else {
            Color32::from_rgb(38, 38, 48)
        };
        let window_frame = egui::Frame::window(&ctx.style()).fill(opaque_fill);

        egui::Window::new("Settings")
            .open(&mut show_settings)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(window_frame)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.checkbox(&mut show_filtered_lines, "Show filtered lines");
                ui.checkbox(&mut colored_nicks, "Colored nicknames in list");
                ui.checkbox(&mut show_timestamps, "Show timestamps");
                ui.checkbox(&mut auto_reconnect, "Auto-reconnect on drop");
                ui.checkbox(&mut show_titlebar, "Show Topic/Modes Titlebar");
                ui.checkbox(&mut show_server_headers, "Show server group headers in buffer list");
                ui.checkbox(&mut show_inline_images, "Show inline image previews (🖼 preview button on image URLs)");
                ui.checkbox(&mut show_link_previews, "Show link previews (🔗 preview button on URLs — fetches title, description, og:image)");
                ui.checkbox(&mut show_hidden_buffers, "Show hidden buffers in buffer list");

                ui.add_space(12.0);
                ui.label(RichText::new("Appearance").strong());
                ui.horizontal(|ui| {
                    ui.label("Font size:");
                    let sizes: &[f32] = &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 20.0, 22.0, 24.0];
                    egui::ComboBox::from_id_source("font_size_combo")
                        .selected_text(format!("{}px", font_size as u32))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for &size in sizes {
                                ui.selectable_value(&mut font_size, size, format!("{}px", size as u32));
                            }
                        });
                });
                ui.checkbox(&mut use_monospace, "Use Monospace font everywhere");
                ui.horizontal(|ui| {
                    ui.label("Opacity:");
                    ui.add(egui::Slider::new(&mut opacity, 0.1..=1.0));
                });

                ui.separator();
                ui.label(RichText::new("Theme").strong());
                ui.label(
                    RichText::new("Supports .itermcolors color schemes")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(4.0);
                ui.label(format!("Current: {}", self.theme.name));
                ui.horizontal(|ui| {
                    if ui.add(
                        egui::Button::new(RichText::new("Import").color(accent_color))
                            .fill(secondary_fill)
                            .stroke(secondary_stroke)
                            .min_size(Vec2::new(80.0, 28.0))
                    ).clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("itermcolors", &["itermcolors"]).pick_file() {
                            if let Ok(data) = std::fs::read(&path) {
                                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                                if let Ok(new_theme) = AppTheme::parse_itermcolors(&data, name) {
                                    self.theme = new_theme;
                                }
                            }
                        }
                    }
                    if ui.add(
                        egui::Button::new(RichText::new("Reset").color(Color32::WHITE))
                            .fill(danger_color)
                            .min_size(Vec2::new(80.0, 28.0))
                    ).clicked() {
                        reset_theme = true;
                    }
                });

                ui.add_space(20.0);
                ui.vertical_centered_justified(|ui| {
                    if ui.add(
                        egui::Button::new(RichText::new("Close").strong().color(Color32::WHITE))
                            .fill(accent_color)
                            .min_size(Vec2::new(0.0, 34.0))
                    ).clicked() {
                        close_clicked = true;
                    }
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
        self.show_server_headers = show_server_headers;
        self.show_inline_images = show_inline_images;
        self.show_link_previews = show_link_previews;
        self.show_hidden_buffers = show_hidden_buffers;
        self.opacity = opacity;
        if reset_theme { self.theme = AppTheme::default(); }
    }
}
