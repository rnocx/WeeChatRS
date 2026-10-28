use serde::{Deserialize, Serialize};
use egui::Color32;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ThemeColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl From<ThemeColor> for Color32 {
    fn from(c: ThemeColor) -> Self {
        Color32::from_rgb(
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
        )
    }
}

impl From<Color32> for ThemeColor {
    fn from(c: Color32) -> Self {
        let [r, g, b, _] = c.to_array();
        ThemeColor {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppTheme {
    pub name: String,
    pub ansi: Vec<ThemeColor>, // exactly 16 entries
    pub background: Option<ThemeColor>,
    pub foreground: Option<ThemeColor>,
    /// Draw 256-colour and RGB colours in the nearest of `ansi`. Set per frame from
    /// the settings, never saved.
    #[serde(skip)]
    pub fit_extended: bool,
}

impl AppTheme {
    pub fn default() -> Self {
        Self {
            name: "Default".to_string(),
            ansi: vec![
                ThemeColor { r: 0.000, g: 0.000, b: 0.000 }, // 0  Black
                ThemeColor { r: 0.694, g: 0.000, b: 0.000 }, // 1  Red
                ThemeColor { r: 0.000, g: 0.694, b: 0.000 }, // 2  Green
                ThemeColor { r: 0.694, g: 0.694, b: 0.000 }, // 3  Yellow
                ThemeColor { r: 0.000, g: 0.000, b: 0.694 }, // 4  Blue
                ThemeColor { r: 0.694, g: 0.000, b: 0.694 }, // 5  Magenta
                ThemeColor { r: 0.000, g: 0.694, b: 0.694 }, // 6  Cyan
                ThemeColor { r: 0.753, g: 0.753, b: 0.753 }, // 7  Light Gray
                ThemeColor { r: 0.502, g: 0.502, b: 0.502 }, // 8  Dark Gray
                ThemeColor { r: 1.000, g: 0.333, b: 0.333 }, // 9  Bright Red
                ThemeColor { r: 0.333, g: 1.000, b: 0.333 }, // 10 Bright Green
                ThemeColor { r: 1.000, g: 1.000, b: 0.333 }, // 11 Bright Yellow
                ThemeColor { r: 0.333, g: 0.333, b: 1.000 }, // 12 Bright Blue
                ThemeColor { r: 1.000, g: 0.333, b: 1.000 }, // 13 Bright Magenta
                ThemeColor { r: 0.333, g: 1.000, b: 1.000 }, // 14 Bright Cyan
                ThemeColor { r: 1.000, g: 1.000, b: 1.000 }, // 15 Bright White
            ],
            background: None,
            foreground: None,
            fit_extended: false,
        }
    }

    /// A Ghostty theme file: `palette = N=#rrggbb`, `background = #rrggbb`,
    /// `foreground = #rrggbb`. Other keys (cursor, selection) are ignored. None when
    /// the text sets none of these.
    pub fn parse_ghostty(text: &str, name: String) -> Option<Self> {
        let mut theme = Self { name, ..Self::default() };
        let mut found = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "palette" => {
                    let Some((index, color)) = value.split_once('=') else { continue };
                    if let (Ok(i @ 0..=15), Some(c)) = (index.trim().parse::<usize>(), parse_hex(color)) {
                        theme.ansi[i] = c;
                        found = true;
                    }
                }
                "background" => if let Some(c) = parse_hex(value) {
                    theme.background = Some(c);
                    found = true;
                },
                "foreground" => if let Some(c) = parse_hex(value) {
                    theme.foreground = Some(c);
                    found = true;
                },
                _ => {}
            }
        }
        found.then_some(theme)
    }
}

/// `#rrggbb`, `rrggbb` or `#rgb`.
fn parse_hex(s: &str) -> Option<ThemeColor> {
    let hex = s.trim().trim_start_matches('#');
    if !hex.is_ascii() {
        return None;
    }
    let channel = |h: &str| u8::from_str_radix(h, 16).ok().map(|v| v as f64 / 255.0);
    let (r, g, b) = match hex.len() {
        6 => (channel(&hex[0..2])?, channel(&hex[2..4])?, channel(&hex[4..6])?),
        3 => {
            let short = |i: usize| channel(&hex[i..i + 1].repeat(2));
            (short(0)?, short(1)?, short(2)?)
        }
        _ => return None,
    };
    Some(ThemeColor { r, g, b })
}

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/ghostty_themes.rs"));
}

/// Themes with a quick button on the Theme page, after Default.
pub const QUICK_THEMES: [&str; 8] = [
    "Catppuccin Mocha", "Catppuccin Latte", "Dracula", "Gruvbox Dark",
    "Nord", "TokyoNight", "iTerm2 Solarized Dark", "One Half Dark",
];

pub struct CatalogTheme {
    pub theme: AppTheme,
    /// From the user's Ghostty themes folder rather than the bundled collection.
    pub user: bool,
}

/// The user's Ghostty themes (sorted by name) followed by the bundled collection.
/// Loaded once, on first use.
pub fn catalog() -> &'static [CatalogTheme] {
    static CATALOG: std::sync::OnceLock<Vec<CatalogTheme>> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut user: Vec<CatalogTheme> = user_theme_dirs().iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let text = std::fs::read_to_string(e.path()).ok()?;
                Some(CatalogTheme { theme: AppTheme::parse_ghostty(&text, name)?, user: true })
            })
            .collect();
        user.sort_by_key(|t| t.theme.name.to_lowercase());
        user.dedup_by(|a, b| a.theme.name == b.theme.name);
        let bundled = bundled::BUNDLED_THEMES.iter().filter_map(|(name, text)| {
            Some(CatalogTheme { theme: AppTheme::parse_ghostty(text, name.to_string())?, user: false })
        });
        user.into_iter().chain(bundled).collect()
    })
}

pub fn find_theme(name: &str) -> Option<&'static AppTheme> {
    catalog().iter().find(|t| t.theme.name == name).map(|t| &t.theme)
}

/// Where Ghostty looks for user themes.
fn user_theme_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(xdg) => dirs.push(std::path::PathBuf::from(xdg).join("ghostty/themes")),
        None => dirs.extend(home.as_ref().map(|h| h.join(".config/ghostty/themes"))),
    }
    #[cfg(target_os = "macos")]
    dirs.extend(home.map(|h| h.join("Library/Application Support/com.mitchellh.ghostty/themes")));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ghostty_theme() {
        let text = "# comment\npalette = 0=#45475a\npalette = 15=#bac2de\npalette = 16=#ffffff\n\
                    background = #1e1e2e\nforeground = cdd6f4\ncursor-color = #f5e0dc\n";
        let theme = AppTheme::parse_ghostty(text, "Mocha".into()).unwrap();
        assert_eq!(Color32::from(theme.ansi[0]), Color32::from_rgb(0x45, 0x47, 0x5a));
        assert_eq!(Color32::from(theme.ansi[15]), Color32::from_rgb(0xba, 0xc2, 0xde));
        assert_eq!(theme.ansi[1], AppTheme::default().ansi[1]);
        assert_eq!(theme.background.map(Color32::from), Some(Color32::from_rgb(0x1e, 0x1e, 0x2e)));
        assert_eq!(theme.foreground.map(Color32::from), Some(Color32::from_rgb(0xcd, 0xd6, 0xf4)));
        assert!(AppTheme::parse_ghostty("font-size = 12\n", "x".into()).is_none());
    }

    #[test]
    fn bundled_themes_parse_and_quick_themes_exist() {
        assert!(bundled::BUNDLED_THEMES.len() > 400);
        for (name, text) in bundled::BUNDLED_THEMES {
            assert!(AppTheme::parse_ghostty(text, name.to_string()).is_some(), "{name}");
        }
        for name in QUICK_THEMES {
            assert!(bundled::BUNDLED_THEMES.iter().any(|(n, _)| *n == name), "{name}");
        }
    }
}
