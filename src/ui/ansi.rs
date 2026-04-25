use egui::{Color32, TextFormat, FontId, Stroke};
use crate::ui::theme::AppTheme;
use regex::Regex;
use std::sync::OnceLock;

static URL_RE: OnceLock<Regex> = OnceLock::new();

fn url_re() -> &'static Regex {
    URL_RE.get_or_init(|| Regex::new(r"https?://[^\s<>]+").unwrap())
}

#[derive(Clone)]
pub struct ANSISection {
    pub text: String,
    pub format: TextFormat,
    pub url: Option<String>,
}

pub struct ANSIParser;

#[derive(Clone, Copy)]
struct SGRState {
    fg: Option<Color32>,
    bg: Option<Color32>,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl SGRState {
    fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

impl ANSIParser {
    pub fn parse(input: &str, font_id: FontId, theme: &AppTheme) -> Vec<ANSISection> {
        let mut raw_sections = Vec::new();
        let mut state = SGRState::new();
        
        let mut current_pos = 0;
        let bytes = input.as_bytes();
        
        while current_pos < bytes.len() {
            if bytes[current_pos] == 0x1B { // ESC
                if current_pos + 1 < bytes.len() && bytes[current_pos + 1] == b'[' {
                    // CSI
                    let mut end_pos = current_pos + 2;
                    while end_pos < bytes.len() && !bytes[end_pos].is_ascii_alphabetic() {
                        end_pos += 1;
                    }
                    
                    if end_pos < bytes.len() {
                        let cmd = bytes[end_pos] as char;
                        let params = &input[current_pos + 2..end_pos];
                        
                        if cmd == 'm' {
                            Self::apply_sgr(params, &mut state, theme);
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
                raw_sections.push((text.to_string(), Self::make_format(&state, font_id.clone())));
                current_pos = end_pos;
            }
        }
        
        // Linkify pass
        let url_re = url_re();
        let mut final_sections = Vec::new();
        
        for (text, format) in raw_sections {
            let mut last_match_end = 0;
            for mat in url_re.find_iter(&text) {
                if mat.start() > last_match_end {
                    final_sections.push(ANSISection {
                        text: text[last_match_end..mat.start()].to_string(),
                        format: format.clone(),
                        url: None,
                    });
                }
                final_sections.push(ANSISection {
                    text: mat.as_str().to_string(),
                    format: format.clone(),
                    url: Some(mat.as_str().to_string()),
                });
                last_match_end = mat.end();
            }
            if last_match_end < text.len() {
                final_sections.push(ANSISection {
                    text: text[last_match_end..].to_string(),
                    format: format.clone(),
                    url: None,
                });
            }
        }
        
        if final_sections.is_empty() && !input.is_empty() {
             // Fallback for empty input cases if needed
        }
        
        final_sections
    }

    fn make_format(state: &SGRState, font_id: FontId) -> TextFormat {
        TextFormat {
            font_id,
            color: state.fg.unwrap_or(Color32::PLACEHOLDER),
            background: state.bg.unwrap_or(Color32::TRANSPARENT),
            italics: state.italic,
            underline: if state.underline { Stroke::new(1.0, state.fg.unwrap_or(Color32::PLACEHOLDER)) } else { Stroke::NONE },
            ..Default::default()
        }
    }

    fn apply_sgr(params: &str, state: &mut SGRState, theme: &AppTheme) {
        let codes: Vec<u8> = if params.is_empty() {
            vec![0]
        } else {
            params.split(';').filter_map(|s| s.parse().ok()).collect()
        };
        
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0 => state.reset(),
                1 => state.bold = true,
                3 => state.italic = true,
                4 => state.underline = true,
                22 => state.bold = false,
                23 => state.italic = false,
                24 => state.underline = false,
                30..=37 => state.fg = Some(Self::ansi_color(codes[i] - 30, theme)),
                38 => {
                    if i + 2 < codes.len() && codes[i + 1] == 5 {
                        state.fg = Some(Self::color_256(codes[i + 2], theme));
                        i += 2;
                    } else if i + 4 < codes.len() && codes[i + 1] == 2 {
                        state.fg = Some(Color32::from_rgb(codes[i+2], codes[i+3], codes[i+4]));
                        i += 4;
                    }
                }
                39 => state.fg = None,
                40..=47 => state.bg = Some(Self::ansi_color(codes[i] - 40, theme)),
                48 => {
                    if i + 2 < codes.len() && codes[i + 1] == 5 {
                        state.bg = Some(Self::color_256(codes[i + 2], theme));
                        i += 2;
                    } else if i + 4 < codes.len() && codes[i + 1] == 2 {
                        state.bg = Some(Color32::from_rgb(codes[i+2], codes[i+3], codes[i+4]));
                        i += 4;
                    }
                }
                49 => state.bg = None,
                90..=97 => state.fg = Some(Self::ansi_color(codes[i] - 90 + 8, theme)),
                100..=107 => state.bg = Some(Self::ansi_color(codes[i] - 100 + 8, theme)),
                _ => {}
            }
            i += 1;
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
            return Self::ansi_color(n, theme);
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
}
