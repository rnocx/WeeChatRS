use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use egui::{Color32, FontFamily, FontId, Frame, Margin, RichText, ScrollArea, TextEdit};
use regex::Regex;
use crate::relay::models::Line;
use crate::ui::icons;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum FsetOptionType {
    Boolean,
    Integer,
    String,
    Color,
    Enum,
    Unknown(std::string::String),
}

impl FsetOptionType {
    fn from_str(s: &str) -> Self {
        match s {
            "boolean" => Self::Boolean,
            "integer" => Self::Integer,
            "string"  => Self::String,
            "color"   => Self::Color,
            "enum"    => Self::Enum,
            other     => Self::Unknown(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Boolean        => "bool",
            Self::Integer        => "int",
            Self::String         => "str",
            Self::Color          => "color",
            Self::Enum           => "enum",
            Self::Unknown(s)     => s.as_str(),
        }
    }

    pub fn type_color(&self, muted: Color32) -> Color32 {
        match self {
            Self::Boolean    => Color32::from_rgb(100, 200, 120),
            Self::Integer    => Color32::from_rgb(100, 160, 220),
            Self::Color      => Color32::from_rgb(220, 160, 80),
            Self::Enum       => Color32::from_rgb(180, 120, 220),
            Self::String     => muted,
            Self::Unknown(_) => muted,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FsetOption {
    pub name: std::string::String,
    pub option_type: FsetOptionType,
    pub default_value: std::string::String,
    pub current_value: std::string::String,
    pub is_changed: bool,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn strip_ansi(s: &str) -> std::string::String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[mKHF]").unwrap());
    re.replace_all(s, "").to_string()
}

/// Parse a single fset buffer line into an FsetOption.
/// Returns None if the line doesn't look like an option entry.
pub fn parse_fset_line(prefix: &str, message: &str) -> Option<FsetOption> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"[*> ]?\s*([a-zA-Z][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_]+){2,})\s+(boolean|color|enum|integer|string)\s+(.*)"
        ).unwrap()
    });

    let plain = strip_ansi(message);
    let plain = plain.trim();
    let is_changed = prefix.contains('*');

    let caps = re.captures(plain)?;
    let name          = caps[1].trim().to_string();
    let option_type   = FsetOptionType::from_str(caps[2].trim());
    let rest          = caps[3].trim();

    let (default_value, current_value) = split_values(rest);

    Some(FsetOption { name, option_type, default_value, current_value, is_changed })
}

/// Split "default  current" — handles quoted strings and bare words.
fn split_values(s: &str) -> (std::string::String, std::string::String) {
    if let Some(pos) = s.find("  ") {
        let default = s[..pos].trim().to_string();
        let current = s[pos..].trim().to_string();
        return (default, current);
    }
    (s.to_string(), s.to_string())
}

/// Parse all lines in an fset buffer into a list of options.
pub fn parse_fset_buffer(lines: &VecDeque<Line>) -> Vec<FsetOption> {
    lines.iter()
        .filter_map(|l| parse_fset_line(&l.prefix, &l.message))
        .collect()
}

// ---------------------------------------------------------------------------
// Actions returned from the UI
// ---------------------------------------------------------------------------

pub enum FsetAction {
    SetValue { name: std::string::String, value: std::string::String },
    ResetToDefault { name: std::string::String },
    BeginEdit { name: std::string::String, current: std::string::String },
    CancelEdit,
}

// ---------------------------------------------------------------------------
// Panel renderer
// ---------------------------------------------------------------------------

pub struct FsetState {
    pub editing_option: Option<std::string::String>,
    pub edit_value: std::string::String,
    /// Optimistic values shown immediately after a change, before WeeChat echoes back.
    pub pending_values: HashMap<std::string::String, std::string::String>,
    pub search: std::string::String,
}

impl Default for FsetState {
    fn default() -> Self {
        Self {
            editing_option: None,
            edit_value: std::string::String::new(),
            pending_values: HashMap::new(),
            search: std::string::String::new(),
        }
    }
}

/// Render the fset panel. Returns a list of actions the caller should process.
pub fn show_fset_panel(
    ui: &mut egui::Ui,
    lines: &VecDeque<Line>,
    state: &mut FsetState,
    font_size: f32,
    _surface_color: Color32,
    text_color: Color32,
    text_muted: Color32,
    accent_color: Color32,
    border_color: Color32,
) -> Vec<FsetAction> {
    let mut actions: Vec<FsetAction> = Vec::new();
    let options = parse_fset_buffer(lines);

    // Expire pending values once the buffer line reflects the new value.
    state.pending_values.retain(|name, pending| {
        if let Some(opt) = options.iter().find(|o| &o.name == name) {
            opt.current_value != *pending
        } else {
            true
        }
    });

    let font_id = FontId::new(font_size, crate::ui::fonts::chat_family());
    let small_id = FontId::new((font_size - 2.0).max(10.0), FontFamily::Proportional);
    let row_height = font_size + 8.0;

    // -----------------------------------------------------------------------
    // Filter options
    // -----------------------------------------------------------------------
    let query = state.search.to_lowercase();
    let filtered: Vec<&FsetOption> = options.iter()
        .filter(|o| query.is_empty() || o.name.to_lowercase().contains(&query))
        .collect();

    // Summary bar
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let changed_count = filtered.iter().filter(|o| o.is_changed).count();
        ui.label(RichText::new(format!("{} options", filtered.len())).color(text_muted).small());
        if changed_count > 0 {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("• {} modified", changed_count)).color(accent_color).small());
        }
        if options.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Type /fset [pattern] to load options").color(text_muted).small());
        }
    });

    ui.add_space(4.0);

    // -----------------------------------------------------------------------
    // Column widths — computed once from available space.
    // Name column gets the bulk; the rest share what remains.
    // -----------------------------------------------------------------------
    let avail = (ui.available_width() - 32.0).max(400.0); // subtract left/right margin
    let col_type    = 46.0_f32;
    let col_default = 160.0_f32;
    let col_current = 200.0_f32;
    let col_actions = 28.0_f32;
    let col_name    = (avail - col_type - col_default - col_current - col_actions - 48.0).clamp(180.0, 400.0);

    // -----------------------------------------------------------------------
    // Options table
    // -----------------------------------------------------------------------
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::NONE
                .inner_margin(Margin::symmetric(16, 4))
                .show(ui, |ui| {
                    if filtered.is_empty() && !options.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new("No options match the filter").color(text_muted));
                        });
                        return;
                    }

                    // Paint text into an exact-size rect — the only way to guarantee column widths.
                    let paint_cell = |ui: &mut egui::Ui, width: f32, text: &str, font: FontId, color: Color32| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::hover());
                        ui.painter_at(rect).text(rect.left_center(), egui::Align2::LEFT_CENTER, text, font, color);
                    };

                    // Header row
                    ui.horizontal(|ui| {
                        paint_cell(ui, col_name,    "Option",        small_id.clone(), text_muted);
                        paint_cell(ui, col_type,    "Type",          small_id.clone(), text_muted);
                        paint_cell(ui, col_default, "Default",       small_id.clone(), text_muted);
                        paint_cell(ui, col_current, "Current value", small_id.clone(), text_muted);
                    });

                    // Header separator
                    ui.scope(|ui| {
                        ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border_color);
                        ui.separator();
                    });

                    for opt in &filtered {
                        let is_editing = state.editing_option.as_deref() == Some(&opt.name);
                        let name_color = if opt.is_changed { accent_color } else { text_color };

                        // Resolved display value — pending takes priority over parsed.
                        let display_value: std::string::String = state.pending_values
                            .get(&opt.name)
                            .cloned()
                            .unwrap_or_else(|| opt.current_value.clone());

                        ui.horizontal(|ui| {
                            // --- Option name ---
                            paint_cell(ui, col_name, &opt.name, font_id.clone(), name_color);

                            // --- Type badge ---
                            let type_color = opt.option_type.type_color(text_muted);
                            paint_cell(ui, col_type, opt.option_type.label(), small_id.clone(), type_color);

                            // --- Default value ---
                            paint_cell(ui, col_default, &opt.default_value, small_id.clone(), text_muted);

                            // --- Current value / inline editor ---
                            if is_editing {
                                match opt.option_type {
                                    FsetOptionType::Boolean => {
                                        let (cell_rect, _) = ui.allocate_exact_size(egui::vec2(col_current, row_height), egui::Sense::hover());
                                        let is_true = state.edit_value == "on" || state.edit_value == "true";
                                        let label = if is_true { "on" } else { "off" };
                                        let btn_color = if is_true { Color32::from_rgb(80, 180, 100) } else { Color32::from_rgb(180, 80, 80) };
                                        let btn = egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(btn_color);
                                        if ui.put(cell_rect, btn).clicked() {
                                            let new_val = if is_true { "off".to_string() } else { "on".to_string() };
                                            state.pending_values.insert(opt.name.clone(), new_val.clone());
                                            actions.push(FsetAction::SetValue { name: opt.name.clone(), value: new_val });
                                            actions.push(FsetAction::CancelEdit);
                                        }
                                    }
                                    _ => {
                                        let (cell_rect, _) = ui.allocate_exact_size(egui::vec2(col_current, row_height), egui::Sense::hover());
                                        let edit = TextEdit::singleline(&mut state.edit_value).font(font_id.clone());
                                        let res = ui.put(cell_rect, edit);
                                        res.request_focus();
                                        if res.lost_focus() {
                                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                                let new_val = state.edit_value.clone();
                                                state.pending_values.insert(opt.name.clone(), new_val.clone());
                                                actions.push(FsetAction::SetValue { name: opt.name.clone(), value: new_val });
                                            }
                                            actions.push(FsetAction::CancelEdit);
                                        }
                                    }
                                }
                            } else {
                                let value_color = if opt.is_changed || state.pending_values.contains_key(&opt.name) {
                                    accent_color
                                } else {
                                    text_color
                                };
                                let (cell_rect, resp) = ui.allocate_exact_size(egui::vec2(col_current, row_height), egui::Sense::click());
                                ui.painter_at(cell_rect).text(cell_rect.left_center(), egui::Align2::LEFT_CENTER, &display_value, font_id.clone(), value_color);
                                if resp.clicked() {
                                    actions.push(FsetAction::BeginEdit {
                                        name: opt.name.clone(),
                                        current: display_value.clone(),
                                    });
                                }
                                resp.on_hover_text("Click to edit");
                            }

                            // --- Reset button ---
                            if opt.is_changed {
                                let (cell_rect, _) = ui.allocate_exact_size(egui::vec2(col_actions, row_height), egui::Sense::hover());
                                let btn = egui::Button::new(RichText::new(icons::ARROW_U_UP_LEFT).color(text_muted)).frame(false);
                                if ui.put(cell_rect, btn).on_hover_text("Reset to default").clicked() {
                                    actions.push(FsetAction::ResetToDefault { name: opt.name.clone() });
                                }
                            }
                        });

                        // Row separator
                        ui.scope(|ui| {
                            ui.visuals_mut().widgets.noninteractive.bg_stroke =
                                egui::Stroke::new(1.0, border_color.linear_multiply(0.4));
                            ui.separator();
                        });
                    }
                });
        });

    actions
}
