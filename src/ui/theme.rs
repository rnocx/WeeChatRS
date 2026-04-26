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
        }
    }

    pub fn parse_itermcolors(data: &[u8], name: String) -> Result<Self, Box<dyn std::error::Error>> {
        let plist: plist::Value = plist::from_bytes(data)?;
        let dict = plist.as_dictionary().ok_or("Invalid itermcolors file")?;

        let get_color = |key: &str| -> Option<ThemeColor> {
            let color_dict = dict.get(key)?.as_dictionary()?;
            let r = color_dict.get("Red Component")?.as_real()?;
            let g = color_dict.get("Green Component")?.as_real()?;
            let b = color_dict.get("Blue Component")?.as_real()?;
            Some(ThemeColor { r, g, b })
        };

        let mut ansi = AppTheme::default().ansi;
        for i in 0..16 {
            if let Some(c) = get_color(&format!("Ansi {} Color", i)) {
                ansi[i] = c;
            }
        }

        Ok(Self {
            name,
            ansi,
            background: get_color("Background Color"),
            foreground: get_color("Foreground Color"),
        })
    }
}
