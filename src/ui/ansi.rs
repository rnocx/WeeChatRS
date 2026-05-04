use egui::{Color32, TextFormat, FontId, Stroke};
use crate::ui::theme::AppTheme;
use regex::Regex;
use std::sync::OnceLock;

static URL_RE: OnceLock<Regex> = OnceLock::new();

fn url_re() -> &'static Regex {
    URL_RE.get_or_init(|| Regex::new(r"https?://[^\s<>]+").unwrap())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiColor {
    Default,
    Indexed(u8),
    Index256(u8),
    Rgb(u8, u8, u8),
}

impl Default for AnsiColor {
    fn default() -> Self { AnsiColor::Default }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnsiStyle {
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl AnsiStyle {
    pub fn to_format(&self, font_id: FontId, theme: &AppTheme) -> TextFormat {
        let fg = resolve_color(self.fg, theme).unwrap_or(Color32::PLACEHOLDER);
        TextFormat {
            font_id,
            color: fg,
            background: resolve_color(self.bg, theme).unwrap_or(Color32::TRANSPARENT),
            italics: self.italic,
            underline: if self.underline { Stroke::new(1.0, fg) } else { Stroke::NONE },
            ..Default::default()
        }
    }
}

fn resolve_color(c: AnsiColor, theme: &AppTheme) -> Option<Color32> {
    match c {
        AnsiColor::Default => None,
        AnsiColor::Indexed(n) => Some(ansi_color(n, theme)),
        AnsiColor::Index256(n) => Some(color_256(n, theme)),
        AnsiColor::Rgb(r, g, b) => Some(Color32::from_rgb(r, g, b)),
    }
}

fn ansi_color(n: u8, theme: &AppTheme) -> Color32 {
    if (n as usize) < theme.ansi.len() {
        return theme.ansi[n as usize].into();
    }
    Color32::WHITE
}

fn color_256(n: u8, theme: &AppTheme) -> Color32 {
    if n < 16 {
        return ansi_color(n, theme);
    }
    if n >= 232 {
        let v = (n - 232) as f32 / 23.0 * 255.0;
        let v = v as u8;
        return Color32::from_rgb(v, v, v);
    }
    let i = n - 16;
    let levels = [0, 95, 135, 175, 215, 255];
    Color32::from_rgb(
        levels[((i / 36) % 6) as usize],
        levels[((i / 6) % 6) as usize],
        levels[(i % 6) as usize],
    )
}

#[derive(Clone, Debug)]
pub struct ANSISection {
    pub text: String,
    pub style: AnsiStyle,
    pub url: Option<String>,
}

pub struct ANSIParser;

impl ANSIParser {
    pub fn parse(input: &str) -> Vec<ANSISection> {
        let mut raw_sections: Vec<(String, AnsiStyle)> = Vec::new();
        let mut state = AnsiStyle::default();

        let mut current_pos = 0;
        let bytes = input.as_bytes();

        while current_pos < bytes.len() {
            if bytes[current_pos] == 0x1B { // ESC
                if current_pos + 1 < bytes.len() && bytes[current_pos + 1] == b'[' {
                    let mut end_pos = current_pos + 2;
                    while end_pos < bytes.len() && !bytes[end_pos].is_ascii_alphabetic() {
                        end_pos += 1;
                    }

                    if end_pos < bytes.len() {
                        let cmd = bytes[end_pos] as char;
                        let params = &input[current_pos + 2..end_pos];

                        if cmd == 'm' {
                            apply_sgr(params, &mut state);
                        }

                        current_pos = end_pos + 1;
                        continue;
                    }
                }
                current_pos += 1;
            } else {
                let mut end_pos = current_pos;
                while end_pos < bytes.len() && bytes[end_pos] != 0x1B {
                    end_pos += 1;
                }

                let text = &input[current_pos..end_pos];
                raw_sections.push((text.to_string(), state));
                current_pos = end_pos;
            }
        }

        // Linkify pass
        let url_re = url_re();
        let mut final_sections = Vec::new();

        for (text, style) in raw_sections {
            let mut last_match_end = 0;
            for mat in url_re.find_iter(&text) {
                if mat.start() > last_match_end {
                    final_sections.push(ANSISection {
                        text: text[last_match_end..mat.start()].to_string(),
                        style,
                        url: None,
                    });
                }
                final_sections.push(ANSISection {
                    text: mat.as_str().to_string(),
                    style,
                    url: Some(mat.as_str().to_string()),
                });
                last_match_end = mat.end();
            }
            if last_match_end < text.len() {
                final_sections.push(ANSISection {
                    text: text[last_match_end..].to_string(),
                    style,
                    url: None,
                });
            }
        }

        final_sections
    }
}

fn apply_sgr(params: &str, state: &mut AnsiStyle) {
    let codes: Vec<u8> = if params.is_empty() {
        vec![0]
    } else {
        params.split(';').filter_map(|s| s.parse().ok()).collect()
    };

    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => *state = AnsiStyle::default(),
            1 => state.bold = true,
            3 => state.italic = true,
            4 => state.underline = true,
            22 => state.bold = false,
            23 => state.italic = false,
            24 => state.underline = false,
            30..=37 => state.fg = AnsiColor::Indexed(codes[i] - 30),
            38 => {
                if i + 2 < codes.len() && codes[i + 1] == 5 {
                    state.fg = AnsiColor::Index256(codes[i + 2]);
                    i += 2;
                } else if i + 4 < codes.len() && codes[i + 1] == 2 {
                    state.fg = AnsiColor::Rgb(codes[i+2], codes[i+3], codes[i+4]);
                    i += 4;
                }
            }
            39 => state.fg = AnsiColor::Default,
            40..=47 => state.bg = AnsiColor::Indexed(codes[i] - 40),
            48 => {
                if i + 2 < codes.len() && codes[i + 1] == 5 {
                    state.bg = AnsiColor::Index256(codes[i + 2]);
                    i += 2;
                } else if i + 4 < codes.len() && codes[i + 1] == 2 {
                    state.bg = AnsiColor::Rgb(codes[i+2], codes[i+3], codes[i+4]);
                    i += 4;
                }
            }
            49 => state.bg = AnsiColor::Default,
            90..=97 => state.fg = AnsiColor::Indexed(codes[i] - 90 + 8),
            100..=107 => state.bg = AnsiColor::Indexed(codes[i] - 100 + 8),
            _ => {}
        }
        i += 1;
    }
}
