pub fn scan_system_fonts() -> Vec<(String, String)> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut seen = std::collections::HashSet::new();
    let mut fonts: Vec<(String, String)> = Vec::new();

    for face in db.faces() {
        if let Some((family, _)) = face.families.first() {
            if seen.insert(family.clone()) {
                if let fontdb::Source::File(path) = &face.source {
                    fonts.push((family.clone(), path.to_string_lossy().into_owned()));
                }
            }
        }
    }

    fonts.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    fonts
}

pub fn family_from_file(path: &str) -> Option<String> {
    let mut db = fontdb::Database::new();
    db.load_font_file(path).ok()?;
    let name = db.faces().next()?.families.first()?.0.clone();
    Some(name)
}

/// Font family for chat content: messages, buffer names, nick list, input and topic.
/// The rest of the UI uses the proportional family.
const CHAT_FAMILY: &str = "chat";

/// Bundled chat fonts, in fallback order (all SIL Open Font License, see assets/fonts/).
/// Like a CSS font stack, each glyph comes from the first font that has it; egui's
/// built-in monospace (Hack) and the emoji/icon fonts follow as the generic fallback.
const CHAT_FONTS: &[(&str, &[u8])] = &[
    ("JetBrainsMono", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf")),
    ("CascadiaCode", include_bytes!("../../assets/fonts/CascadiaCode-Regular.ttf")),
    ("FiraCode", include_bytes!("../../assets/fonts/FiraCode-Regular.ttf")),
];

pub fn chat_family() -> egui::FontFamily {
    egui::FontFamily::Name(CHAT_FAMILY.into())
}

/// Install fonts: the chat family (an optional user-chosen font first, then the bundled
/// stack), plus the Phosphor icon font everywhere.
pub fn apply(ctx: &egui::Context, font_path: &str) {
    ctx.set_fonts(definitions(font_path));
}

fn definitions(font_path: &str) -> egui::FontDefinitions {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    crate::ui::icons::install(&mut fonts);

    let mut chat: Vec<String> = Vec::new();
    if !font_path.is_empty() {
        match std::fs::read(font_path) {
            Ok(data) => {
                fonts.font_data.insert("user_font".to_owned(), Arc::new(egui::FontData::from_owned(data)));
                chat.push("user_font".to_owned());
            }
            Err(e) => log::warn!("chat font {font_path}: {e}"),
        }
    }
    for (name, bytes) in CHAT_FONTS {
        fonts.font_data.insert((*name).to_owned(), Arc::new(egui::FontData::from_static(bytes)));
        chat.push((*name).to_owned());
    }
    chat.extend(fonts.families.get(&egui::FontFamily::Monospace).cloned().unwrap_or_default());
    fonts.families.insert(chat_family(), chat);
    fonts
}

#[cfg(test)]
mod tests {
    #[test]
    fn chat_stack_order_and_fallbacks() {
        let fonts = super::definitions("");
        let chat = &fonts.families[&super::chat_family()];
        assert_eq!(&chat[..3], ["JetBrainsMono", "CascadiaCode", "FiraCode"]);
        // Generic monospace + icon fallbacks follow the bundled fonts.
        assert!(chat.iter().any(|f| f == "Hack"));
        assert!(chat.iter().any(|f| f == "phosphor"));
        // The UI's proportional family is untouched by the chat stack.
        assert!(!fonts.families[&egui::FontFamily::Proportional].iter().any(|f| f == "JetBrainsMono"));
    }

    #[test]
    fn user_font_goes_first_in_chat_only() {
        // Any readable font file works; reuse a bundled one.
        let tmp = std::env::temp_dir().join(format!("weechatrs-userfont-{}.ttf", std::process::id()));
        std::fs::write(&tmp, super::CHAT_FONTS[2].1).unwrap();
        let fonts = super::definitions(tmp.to_str().unwrap());
        assert_eq!(fonts.families[&super::chat_family()][0], "user_font");
        assert!(!fonts.families[&egui::FontFamily::Proportional].iter().any(|f| f == "user_font"));
        let _ = std::fs::remove_file(tmp);
    }
}
