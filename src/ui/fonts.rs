/// An installed font family and its upright faces, by width and then weight.
pub struct FontFamilyEntry {
    pub name: String,
    pub faces: Vec<FontFace>,
}

/// One face of a family: a file, and which face in it (a `.ttc` holds several).
#[derive(Clone, PartialEq)]
pub struct FontFace {
    /// "Regular", "Medium", "SemiBold Extended"…
    pub style: String,
    pub path: String,
    pub index: u32,
    weight: u16,
    stretch: fontdb::Stretch,
}

impl FontFamilyEntry {
    /// The face to start with: normal width, the weight nearest Regular.
    pub fn regular(&self) -> &FontFace {
        self.faces.iter()
            .min_by_key(|f| (f.stretch != fontdb::Stretch::Normal, f.weight.abs_diff(400)))
            .expect("a family has at least one face")
    }
}

/// Installed fonts, one entry per family, every weight kept. Italics are left out
/// unless a family has nothing else.
pub fn scan_system_fonts() -> Vec<FontFamilyEntry> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut families: std::collections::BTreeMap<String, (Vec<FontFace>, Vec<FontFace>)> = Default::default();
    for face in db.faces() {
        let (Some((family, _)), fontdb::Source::File(path)) = (face.families.first(), &face.source) else { continue };
        let entry = FontFace {
            style: style_name(face.weight.0, face.stretch),
            path: path.to_string_lossy().into_owned(),
            index: face.index,
            weight: face.weight.0,
            stretch: face.stretch,
        };
        let (upright, italic) = families.entry(family.clone()).or_default();
        if face.style == fontdb::Style::Normal { upright.push(entry) } else { italic.push(entry) }
    }

    let mut fonts: Vec<FontFamilyEntry> = families.into_iter().map(|(name, (upright, italic))| {
        let mut faces = if upright.is_empty() { italic } else { upright };
        faces.sort_by_key(|f| (f.stretch, f.weight));
        // The same weight from two files (a .ttc and a loose .ttf): keep the first.
        faces.dedup_by(|a, b| a.style == b.style);
        FontFamilyEntry { name, faces }
    }).collect();
    fonts.sort_by_key(|a| a.name.to_lowercase());
    fonts
}

fn style_name(weight: u16, stretch: fontdb::Stretch) -> String {
    let weight = match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    };
    use fontdb::Stretch::*;
    let width = match stretch {
        UltraCondensed => " UltraCondensed",
        ExtraCondensed => " ExtraCondensed",
        Condensed => " Condensed",
        SemiCondensed => " SemiCondensed",
        Normal => "",
        SemiExpanded => " SemiExtended",
        Expanded => " Extended",
        ExtraExpanded => " ExtraExtended",
        UltraExpanded => " UltraExtended",
    };
    format!("{weight}{width}")
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
pub(crate) const CHAT_FONTS: &[(&str, &[u8])] = &[
    ("JetBrainsMono", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf")),
    ("CascadiaCode", include_bytes!("../../assets/fonts/CascadiaCode-Regular.ttf")),
    ("FiraCode", include_bytes!("../../assets/fonts/FiraCode-Regular.ttf")),
];

pub fn chat_family() -> egui::FontFamily {
    egui::FontFamily::Name(CHAT_FAMILY.into())
}

/// Install fonts: the chat family (an optional user-chosen font first, then the bundled
/// stack), plus the Phosphor icon font everywhere. `font_index` picks the face in a
/// collection (`.ttc`).
pub fn apply(ctx: &egui::Context, font_path: &str, font_index: u32) {
    ctx.set_fonts(definitions(font_path, font_index));
}

fn definitions(font_path: &str, font_index: u32) -> egui::FontDefinitions {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    crate::ui::icons::install(&mut fonts);

    let mut chat: Vec<String> = Vec::new();
    if !font_path.is_empty() {
        match std::fs::read(font_path) {
            Ok(data) => {
                let mut font = egui::FontData::from_owned(data);
                font.index = font_index;
                fonts.font_data.insert("user_font".to_owned(), Arc::new(font));
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
        let fonts = super::definitions("", 0);
        let chat = &fonts.families[&super::chat_family()];
        assert_eq!(&chat[..3], ["JetBrainsMono", "CascadiaCode", "FiraCode"]);
        // Generic monospace + icon fallbacks follow the bundled fonts.
        assert!(chat.iter().any(|f| f == "Hack"));
        assert!(chat.iter().any(|f| f == "phosphor"));
        // The UI's proportional family is untouched by the chat stack.
        assert!(!fonts.families[&egui::FontFamily::Proportional].iter().any(|f| f == "JetBrainsMono"));
    }

    #[test]
    #[ignore = "lists this machine's fonts"]
    fn print_installed_iosevka() {
        for f in super::scan_system_fonts().iter().filter(|f| f.name.to_lowercase().contains("iosevka")) {
            println!("{}: {}", f.name, f.faces.iter().map(|x| format!("{} [{}#{}]", x.style, x.path.rsplit('/').next().unwrap(), x.index)).collect::<Vec<_>>().join(", "));
        }
    }

    #[test]
    fn style_names() {
        assert_eq!(super::style_name(400, fontdb::Stretch::Normal), "Regular");
        assert_eq!(super::style_name(500, fontdb::Stretch::Normal), "Medium");
        assert_eq!(super::style_name(600, fontdb::Stretch::Expanded), "SemiBold Extended");
    }

    #[test]
    fn user_font_goes_first_in_chat_only() {
        // Any readable font file works; reuse a bundled one.
        let tmp = std::env::temp_dir().join(format!("weechatrs-userfont-{}.ttf", std::process::id()));
        std::fs::write(&tmp, super::CHAT_FONTS[2].1).unwrap();
        let fonts = super::definitions(tmp.to_str().unwrap(), 0);
        assert_eq!(fonts.families[&super::chat_family()][0], "user_font");
        assert!(!fonts.families[&egui::FontFamily::Proportional].iter().any(|f| f == "user_font"));
        let _ = std::fs::remove_file(tmp);
    }
}
