use crate::ui::theme::{AppTheme, ThemeColor};
use crate::ui::app::WeeChatApp;
use crate::ui::fonts;
use egui::{Color32, Vec2, RichText};
use egui::color_picker::{color_edit_button_srgba, Alpha};

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
        let mut new_font: Option<(String, String)> = None; // (name, path)
        let mut reset_font = false;

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

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Font family:");
                    let selected_label = if self.font_name.is_empty() {
                        "Default".to_string()
                    } else {
                        self.font_name.clone()
                    };
                    egui::ComboBox::from_id_source("font_family_combo")
                        .selected_text(&selected_label)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.font_name.is_empty(), "Default").clicked() {
                                reset_font = true;
                            }
                            ui.separator();
                            for (name, path) in &self.available_fonts {
                                let selected = name.as_str() == self.font_name.as_str();
                                if ui.selectable_label(selected, name.as_str()).clicked() {
                                    new_font = Some((name.clone(), path.clone()));
                                }
                            }
                        });
                    if ui.add(
                        egui::Button::new(RichText::new("Browse…").color(accent_color))
                            .fill(secondary_fill)
                            .stroke(secondary_stroke)
                    ).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Font files", &["ttf", "otf", "ttc"])
                            .pick_file()
                        {
                            let path_str = path.to_string_lossy().into_owned();
                            let name = fonts::family_from_file(&path_str)
                                .unwrap_or_else(|| path.file_stem()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default());
                            new_font = Some((name, path_str));
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Opacity:");
                    ui.add(egui::Slider::new(&mut opacity, 0.1..=1.0));
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Theme").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(
                            egui::Button::new(RichText::new("Reset").color(Color32::WHITE))
                                .fill(danger_color)
                                .min_size(Vec2::new(70.0, 24.0))
                        ).clicked() {
                            reset_theme = true;
                        }
                        if ui.add(
                            egui::Button::new(RichText::new("Import .itermcolors").color(accent_color))
                                .fill(secondary_fill)
                                .stroke(secondary_stroke)
                                .min_size(Vec2::new(140.0, 24.0))
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
                    });
                });
                ui.label(
                    RichText::new(format!("Current: {}", self.theme.name))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );

                ui.add_space(6.0);

                // BG / FG
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Background:").small());
                    let mut bg = self.theme.background
                        .map(Color32::from)
                        .unwrap_or(Color32::from_rgb(18, 18, 18));
                    if color_edit_button_srgba(ui, &mut bg, Alpha::Opaque).changed() {
                        self.theme.background = Some(ThemeColor::from(bg));
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("Foreground:").small());
                    let mut fg = self.theme.foreground
                        .map(Color32::from)
                        .unwrap_or(Color32::from_rgb(204, 204, 204));
                    if color_edit_button_srgba(ui, &mut fg, Alpha::Opaque).changed() {
                        self.theme.foreground = Some(ThemeColor::from(fg));
                    }
                });

                ui.add_space(6.0);

                // ANSI palette — two rows of 8
                const LABELS: [&str; 16] = [
                    "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White",
                    "Br.Black", "Br.Red", "Br.Green", "Br.Yellow", "Br.Blue", "Br.Magenta", "Br.Cyan", "Br.White",
                ];
                for row in 0..2usize {
                    ui.horizontal(|ui| {
                        for col in 0..8usize {
                            let i = row * 8 + col;
                            ui.vertical(|ui| {
                                ui.set_width(46.0);
                                let mut c = Color32::from(self.theme.ansi[i]);
                                if color_edit_button_srgba(ui, &mut c, Alpha::Opaque).changed() {
                                    self.theme.ansi[i] = ThemeColor::from(c);
                                }
                                ui.label(
                                    RichText::new(LABELS[i])
                                        .size(9.0)
                                        .color(ui.visuals().weak_text_color()),
                                );
                            });
                        }
                    });
                    ui.add_space(2.0);
                }

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
        if reset_font {
            self.font_name.clear();
            self.font_path.clear();
        }
        if let Some((name, path)) = new_font {
            self.font_name = name;
            self.font_path = path;
        }
    }
}
