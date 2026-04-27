use crate::ui::theme::{AppTheme, ThemeColor};
use crate::ui::app::WeeChatApp;
use crate::ui::fonts;
use egui::{Color32, Vec2, RichText, ScrollArea, Frame, Rounding, Margin, Stroke};
use egui::color_picker::{color_edit_button_srgba, Alpha};

impl WeeChatApp {
    /// Renders settings directly into `ui` (called from inside the central panel so it works
    /// in macOS fullscreen and every other windowing mode without z-order issues).
    pub(crate) fn show_settings_window(&mut self, ui: &mut egui::Ui, accent_color: Color32, is_light: bool) {
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
        let mut close = false;
        let mut reset_theme = false;
        let mut new_font: Option<(String, String)> = None;
        let mut reset_font = false;

        let danger_color = Color32::from_rgb(185, 55, 55);
        let secondary_fill = if is_light {
            Color32::from_rgba_unmultiplied(100, 149, 237, 30)
        } else {
            Color32::from_rgba_unmultiplied(100, 149, 237, 40)
        };
        let secondary_stroke = Stroke::new(1.0, accent_color.linear_multiply(0.6));
        let opaque_fill = if is_light { Color32::from_gray(235) } else { Color32::from_rgb(38, 38, 48) };
        let border_color = if is_light { Color32::from_gray(200) } else { Color32::from_gray(55) };

        // Close on Escape.
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_settings = false;
            return;
        }

        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            Frame::none()
                .fill(opaque_fill)
                .rounding(Rounding::same(12.0))
                .stroke(Stroke::new(1.0, border_color))
                .inner_margin(Margin::same(24.0))
                .show(ui, |ui| {
                    ui.set_max_width(540.0);

                    // Header row
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Settings").strong().size(18.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("✕").size(16.0)).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    ScrollArea::vertical().show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.checkbox(&mut show_filtered_lines, "Show filtered lines");
                            ui.checkbox(&mut colored_nicks, "Colored nicknames in list");
                            ui.checkbox(&mut show_timestamps, "Show timestamps");
                            ui.checkbox(&mut auto_reconnect, "Auto-reconnect on drop");
                            ui.checkbox(&mut show_titlebar, "Show Topic/Modes Titlebar");
                            ui.checkbox(&mut show_server_headers, "Show server group headers in buffer list");
                            ui.checkbox(&mut show_inline_images, "Show inline image previews (🖼 preview button on image URLs)");
                            ui.checkbox(&mut show_link_previews, "Show link previews (🔗 preview button on URLs)");
                            ui.checkbox(&mut show_hidden_buffers, "Show hidden buffers in buffer list");
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
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
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Font family:");
                            let selected_label = if self.font_name.is_empty() {
                                "Default".to_string()
                            } else {
                                self.font_name.clone()
                            };
                            let combo_resp = egui::ComboBox::from_id_source("font_family_combo")
                                .selected_text(&selected_label)
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    // Search field at the top of the popup.
                                    let search_edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.font_search)
                                            .hint_text("Search fonts…")
                                            .desired_width(f32::INFINITY),
                                    );
                                    // Auto-focus the search field when the popup first opens.
                                    if search_edit.gained_focus() || (!search_edit.has_focus() && ui.memory(|m| m.any_popup_open()) ) {
                                        search_edit.request_focus();
                                    }
                                    ui.separator();
                                    if ui.selectable_label(self.font_name.is_empty(), "Default").clicked() {
                                        reset_font = true;
                                        self.font_search.clear();
                                    }
                                    ui.separator();
                                    let query = self.font_search.to_lowercase();
                                    for (name, path) in &self.available_fonts {
                                        if query.is_empty() || name.to_lowercase().contains(&query) {
                                            let selected = name.as_str() == self.font_name.as_str();
                                            if ui.selectable_label(selected, name.as_str()).clicked() {
                                                new_font = Some((name.clone(), path.clone()));
                                                self.font_search.clear();
                                            }
                                        }
                                    }
                                });
                            // Clear search when the dropdown closes.
                            if combo_resp.inner.is_none() {
                                self.font_search.clear();
                            }
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
                        ui.checkbox(&mut use_monospace, "Use Monospace font everywhere");

                        ui.horizontal(|ui| {
                            ui.label("Opacity:");
                            ui.add(egui::Slider::new(&mut opacity, 0.1..=1.0));
                        });

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Theme").strong());
                            ui.label(
                                RichText::new(format!("({})", self.theme.name))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
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
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("itermcolors", &["itermcolors"])
                                        .pick_file()
                                    {
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

                        ui.add_space(6.0);
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

                        ui.add_space(16.0);
                        ui.vertical_centered_justified(|ui| {
                            if ui.add(
                                egui::Button::new(RichText::new("Close").strong().color(Color32::WHITE))
                                    .fill(accent_color)
                                    .min_size(Vec2::new(0.0, 34.0))
                            ).clicked() {
                                close = true;
                            }
                        });
                    });
                });
        });

        if close { self.show_settings = false; }
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
