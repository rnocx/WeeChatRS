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

pub fn apply(ctx: &egui::Context, font_path: &str) {
    let mut fonts = egui::FontDefinitions::default();
    if !font_path.is_empty() {
        if let Ok(data) = std::fs::read(font_path) {
            fonts.font_data.insert(
                "user_font".to_owned(),
                egui::FontData::from_owned(data),
            );
            fonts.families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "user_font".to_owned());
            fonts.families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "user_font".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}
