//! Phosphor icon font (regular weight, MIT — https://phosphoricons.com).
//!
//! The font is registered as a fallback in every family, so these constants can
//! be dropped into any `RichText` / button label. Codepoints are taken from
//! `egui-phosphor`, which can't be used directly as it trails egui releases.
//!
//! The bundled font has its icons moved from U+E000.. to U+100000.. (same offset).
//! Text fonts such as Inter put their own symbols in the BMP private use area, and
//! being earlier in the fallback chain they'd win; nothing ships glyphs up there.

const PHOSPHOR_TTF: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
const FONT_NAME: &str = "phosphor";

pub const ARROW_COUNTER_CLOCKWISE: &str = "\u{100038}";
pub const ARROW_U_UP_LEFT: &str = "\u{10008A}";
pub const ARROW_UP: &str = "\u{10008E}";
pub const AT: &str = "\u{1000AC}";
pub const BELL: &str = "\u{1000CE}";
pub const BELL_SLASH: &str = "\u{1000D4}";
pub const CARET_DOWN: &str = "\u{100136}";
pub const CARET_RIGHT: &str = "\u{10013A}";
pub const CHAT_CIRCLE_TEXT: &str = "\u{10016E}";
pub const GEAR: &str = "\u{100270}";
pub const HOURGLASS: &str = "\u{1002B2}";
pub const IMAGE: &str = "\u{1002CA}";
pub const KEYBOARD: &str = "\u{1002D8}";
pub const LINK: &str = "\u{1002E2}";
pub const PALETTE: &str = "\u{1006C8}";
pub const MAGNIFYING_GLASS: &str = "\u{10030C}";
pub const PAPERCLIP: &str = "\u{10039A}";
pub const PENCIL_SIMPLE: &str = "\u{1003B4}";
pub const PLUGS: &str = "\u{100B56}";
pub const SIDEBAR_SIMPLE: &str = "\u{100C24}";
pub const SWATCHES: &str = "\u{1005B8}";
pub const TERMINAL_WINDOW: &str = "\u{100AE8}";
pub const UPLOAD_SIMPLE: &str = "\u{1004C0}";
pub const USERS: &str = "\u{1004D6}";
pub const WARNING: &str = "\u{1004E0}";
pub const X: &str = "\u{1004F6}";

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

#[cfg(test)]
mod tests {
    #[test]
    fn icons_only_resolve_to_phosphor() {
        let icons = [
            super::ARROW_COUNTER_CLOCKWISE, super::ARROW_U_UP_LEFT, super::ARROW_UP, super::AT, super::BELL,
            super::BELL_SLASH, super::CARET_DOWN, super::CARET_RIGHT, super::CHAT_CIRCLE_TEXT, super::GEAR,
            super::HOURGLASS, super::IMAGE, super::KEYBOARD, super::LINK, super::PALETTE, super::MAGNIFYING_GLASS,
            super::PAPERCLIP, super::PENCIL_SIMPLE, super::PLUGS, super::SIDEBAR_SIMPLE, super::SWATCHES,
            super::TERMINAL_WINDOW, super::UPLOAD_SIMPLE, super::USERS, super::WARNING, super::X,
        ];
        let phosphor = ttf_parser::Face::parse(super::PHOSPHOR_TTF, 0).unwrap();
        for icon in icons {
            let c = icon.chars().next().unwrap();
            assert!(phosphor.glyph_index(c).is_some(), "U+{:X} missing from Phosphor", c as u32);
            // Fonts ahead of Phosphor in the chain must not have it, or theirs is drawn.
            for (name, bytes) in crate::ui::fonts::CHAT_FONTS {
                let face = ttf_parser::Face::parse(bytes, 0).unwrap();
                assert!(face.glyph_index(c).is_none(), "{name} shadows U+{:X}", c as u32);
            }
        }
    }
}
