//! Phosphor icon font (regular weight, MIT — https://phosphoricons.com).
//!
//! The font is registered as a fallback in every family, so these constants can
//! be dropped into any `RichText` / button label. Codepoints are taken from
//! `egui-phosphor`, which can't be used directly as it trails egui releases.

const PHOSPHOR_TTF: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
const FONT_NAME: &str = "phosphor";

pub const ARROW_COUNTER_CLOCKWISE: &str = "\u{E038}";
pub const ARROW_U_UP_LEFT: &str = "\u{E08A}";
pub const ARROW_UP: &str = "\u{E08E}";
pub const BELL: &str = "\u{E0CE}";
pub const BELL_SLASH: &str = "\u{E0D4}";
pub const CARET_DOWN: &str = "\u{E136}";
pub const CARET_RIGHT: &str = "\u{E13A}";
pub const CHAT_CIRCLE_TEXT: &str = "\u{E16E}";
pub const GEAR: &str = "\u{E270}";
pub const HOURGLASS: &str = "\u{E2B2}";
pub const IMAGE: &str = "\u{E2CA}";
pub const KEYBOARD: &str = "\u{E2D8}";
pub const LINK: &str = "\u{E2E2}";
pub const PALETTE: &str = "\u{E6C8}";
pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
pub const PAPERCLIP: &str = "\u{E39A}";
pub const PENCIL_SIMPLE: &str = "\u{E3B4}";
pub const PLUGS: &str = "\u{EB56}";
pub const SWATCHES: &str = "\u{E5B8}";
pub const TERMINAL_WINDOW: &str = "\u{EAE8}";
pub const UPLOAD_SIMPLE: &str = "\u{E4C0}";
pub const WARNING: &str = "\u{E4E0}";
pub const X: &str = "\u{E4F6}";

/// Add the icon font as a fallback to both font families.
pub fn install(fonts: &mut egui::FontDefinitions) {
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(PHOSPHOR_TTF).tweak(egui::FontTweak {
            // Phosphor glyphs sit slightly high next to Latin text.
            y_offset_factor: 0.1,
            ..Default::default()
        })),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(FONT_NAME.to_owned());
    }
}
