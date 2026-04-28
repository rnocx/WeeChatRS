use crate::relay::client::{RelayClient, RelayEvent};
use crate::relay::models::*;
use crate::ui::ansi::ANSIParser;
use crate::ui::theme::AppTheme;
use egui::{FontId, ScrollArea, Label, Key, Visuals, TextStyle, FontFamily, Color32, text::LayoutJob, Margin, Frame, Rounding, Stroke, Vec2, Modifiers, Rect, Painter};
use tokio::sync::mpsc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

pub(crate) enum ImageState {
    Loading,
    Loaded(egui::TextureHandle),
    Failed,
}

pub(crate) struct LinkPreview {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub site_name: Option<String>,
}

pub(crate) enum PreviewState {
    Loading,
    Loaded(LinkPreview),
    Failed,
}

// --- HTML helpers (used in tokio::spawn, so must be free functions) ---

fn extract_attr_val(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    for q in ['"', '\''] {
        let needle = format!("{}={}", attr, q);
        if let Some(pos) = lower.find(&needle) {
            let start = pos + needle.len();
            if let Some(end) = tag[start..].find(q) {
                let val = tag[start..start + end].trim().to_string();
                if !val.is_empty() { return Some(decode_entities(&val)); }
            }
        }
    }
    None
}

fn extract_og_tag(html: &str, property: &str) -> Option<String> {
    let lower = html.to_lowercase();
    for q in ['"', '\''] {
        let needle = format!("property={}{}{}", q, property, q);
        if let Some(pos) = lower.find(&needle) {
            let tag_start = lower[..pos].rfind('<')?;
            let tag_end = tag_start + lower[tag_start..].find('>')?;
            if let Some(val) = extract_attr_val(&html[tag_start..=tag_end], "content") {
                return Some(val);
            }
        }
    }
    None
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let text = html[open_end..close].trim().to_string();
    if text.is_empty() { None } else { Some(decode_entities(&text)) }
}

fn extract_meta_description(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    for q in ['"', '\''] {
        let needle = format!("name={}description{}", q, q);
        if let Some(pos) = lower.find(&needle) {
            let tag_start = lower[..pos].rfind('<')?;
            let tag_end = tag_start + lower[tag_start..].find('>')?;
            if let Some(val) = extract_attr_val(&html[tag_start..=tag_end], "content") {
                return Some(val);
            }
        }
    }
    None
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&apos;", "'")
     .replace("&nbsp;", " ")
}

async fn fetch_link_preview(url: String) -> Result<LinkPreview, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 WeeChatRS/0.1 (link preview)")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let ct = resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !ct.contains("text/html") {
        return Err("not html".to_string());
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]);

    let title = extract_og_tag(&html, "og:title").or_else(|| extract_html_title(&html));
    let description = extract_og_tag(&html, "og:description")
        .or_else(|| extract_meta_description(&html));
    let image_url = extract_og_tag(&html, "og:image");
    let site_name = extract_og_tag(&html, "og:site_name")
        .or_else(|| url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)));

    if title.is_none() && description.is_none() {
        return Err("no preview data".to_string());
    }

    Ok(LinkPreview { title, description, image_url, site_name })
}

pub const INITIAL_LINES: usize = 1000;
pub const LOAD_MORE_LINES: usize = 1000;
pub const MAX_STORED_LINES: usize = 10_000;

/// Reorder `buffers` by moving the dragged item (and its whole server group when it is a server
/// header) to just before `drop_before_id`, or to the end when `drop_before_id` is `None`.
fn apply_drag_reorder(buffers: &mut Vec<Buffer>, drag_id: &str, drop_before_id: Option<&str>) {
    let drag_idx = match buffers.iter().position(|b| b.id == drag_id) {
        Some(i) => i,
        None => return,
    };

    let is_header = buffers[drag_idx].kind == "server" || buffers[drag_idx].kind == "core";
    let server_key = buffers[drag_idx].server.clone();

    // Indices of all buffers that will move (header + its children when moving a header).
    let group_indices: Vec<usize> = if is_header {
        buffers.iter().enumerate()
            .filter(|(_, b)| b.server == server_key)
            .map(|(i, _)| i)
            .collect()
    } else {
        vec![drag_idx]
    };

    // If the drop target is inside the group being moved, do nothing.
    if let Some(tid) = drop_before_id {
        if group_indices.iter().any(|&i| buffers[i].id == tid) {
            return;
        }
    }

    // Remove from highest index first to keep lower indices valid.
    let mut moved: Vec<Buffer> = group_indices.iter().rev()
        .map(|&i| buffers.remove(i))
        .collect();
    moved.reverse();

    let insert_at = match drop_before_id {
        Some(tid) => buffers.iter().position(|b| b.id == tid).unwrap_or(buffers.len()),
        None => buffers.len(),
    };

    for (offset, buf) in moved.into_iter().enumerate() {
        buffers.insert(insert_at + offset, buf);
    }
}

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    pub host: String,
    pub port: String,
    pub use_ssl: bool,
    pub show_filtered_lines: bool,
    pub colored_nicks: bool,
    pub theme: AppTheme,
    pub font_size: f32,
    pub use_monospace: bool,
    pub show_timestamps: bool,
    pub show_buffers: bool,
    pub show_nicklist: bool,
    pub auto_reconnect: bool,
    pub show_titlebar: bool,
    pub show_server_headers: bool,
    pub show_inline_images: bool,
    pub show_link_previews: bool,
    pub opacity: f32,
    #[serde(default)]
    pub show_hidden_buffers: bool,
    #[serde(default)]
    pub buffer_order: Vec<String>,
    /// Buffer IDs the user has explicitly read; suppresses stale hotlist entries on
    /// reconnect.  The relay API has no dedicated read-mark endpoint — the correct
    /// mechanism is POST /api/input + "/input hotlist_clear", but this local set
    /// provides a reliable fallback for the client-side indicator state.
    #[serde(default)]
    pub cleared_buffer_ids: HashSet<String>,
    #[serde(default)]
    pub save_password: bool,
    #[serde(default)]
    pub font_name: String,
    #[serde(default)]
    pub font_path: String,
    #[serde(default)]
    pub muted_buffer_names: HashSet<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: "9001".to_string(),
            use_ssl: true,
            show_filtered_lines: false,
            colored_nicks: true,
            theme: AppTheme::default(),
            font_size: 14.0,
            use_monospace: true,
            show_timestamps: true,
            show_buffers: true,
            show_nicklist: true,
            auto_reconnect: true,
            show_titlebar: true,
            show_server_headers: true,
            show_inline_images: true,
            show_link_previews: true,
            opacity: 1.0,
            show_hidden_buffers: false,
            buffer_order: Vec::new(),
            cleared_buffer_ids: HashSet::new(),
            save_password: false,
            font_name: String::new(),
            font_path: String::new(),
            muted_buffer_names: HashSet::new(),
        }
    }
}

pub struct WeeChatApp {
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) password: String,
    pub(crate) save_password: bool,
    pub(crate) password_from_keyring: bool,
    pub(crate) use_ssl: bool,
    
    pub(crate) client: Option<RelayClient>,
    pub(crate) event_rx: mpsc::UnboundedReceiver<RelayEvent>,
    pub(crate) event_tx: mpsc::UnboundedSender<RelayEvent>,
    
    pub(crate) connection_status: String,
    pub(crate) is_connecting: bool,
    pub(crate) connecting_pending: bool,
    pub(crate) auth_error: Option<String>,
    pub(crate) buffers: Vec<Buffer>,
    pub(crate) selected_buffer_id: Option<String>,
    pub(crate) input_text: String,
    // Settings
    pub(crate) show_settings: bool,
    pub(crate) show_filtered_lines: bool,
    pub(crate) colored_nicks: bool,
    pub(crate) theme: AppTheme,
    pub(crate) font_size: f32,
    pub(crate) use_monospace: bool,
    pub(crate) show_timestamps: bool,
    pub(crate) auto_reconnect: bool,
    pub(crate) show_titlebar: bool,
    pub(crate) show_server_headers: bool,
    pub(crate) show_inline_images: bool,
    pub(crate) show_link_previews: bool,
    pub(crate) opacity: f32,
    pub(crate) show_hidden_buffers: bool,

    // Image preview state
    pub(crate) image_cache: HashMap<String, ImageState>,
    pub(crate) image_expanded: HashSet<String>,
    pub(crate) image_tx: mpsc::UnboundedSender<(String, Result<Vec<u8>, String>)>,
    pub(crate) image_rx: mpsc::UnboundedReceiver<(String, Result<Vec<u8>, String>)>,

    // Link preview state
    pub(crate) preview_cache: HashMap<String, PreviewState>,
    pub(crate) preview_expanded: HashSet<String>,
    pub(crate) preview_tx: mpsc::UnboundedSender<(String, Result<LinkPreview, String>)>,
    pub(crate) preview_rx: mpsc::UnboundedReceiver<(String, Result<LinkPreview, String>)>,

    // UI visibility
    pub(crate) show_buffers: bool,
    pub(crate) show_nicklist: bool,

    // Completion state
    pub(crate) completion: Option<CompletionState>,

    // Command History
    pub(crate) command_history: VecDeque<String>,
    pub(crate) history_index: Option<usize>,

    // Search state
    pub(crate) show_search: bool,
    pub(crate) search_text: String,

    // Navigation
    pub(crate) pending_buffer_switch: Option<String>,

    // Buffer drag-and-drop reordering
    pub(crate) buffer_order: Vec<String>,
    pub(crate) dragging_buffer_id: Option<String>,
    pub(crate) drag_drop_before_id: Option<String>, // None = drop at end

    // Buffers the user has explicitly read this session; suppresses stale hotlist entries.
    pub(crate) cleared_buffer_ids: HashSet<String>,

    // Font selection
    pub(crate) font_name: String,
    pub(crate) font_path: String,
    pub(crate) applied_font_path: String,
    pub(crate) available_fonts: Vec<(String, String)>,

    // Tracks when the current buffer was selected; drives the unread divider transition.
    pub(crate) selected_view_since: Option<std::time::Instant>,

    // Muted buffers (stored by full_name, stable across WeeChat restarts).
    pub(crate) muted_buffer_names: HashSet<String>,

    // Set to the buffer ID while a "load more" history request is in flight.
    pub(crate) loading_more_buffer_id: Option<String>,

    // Transient search text inside the font-family dropdown.
    pub(crate) font_search: String,

    // Connection status log shown while connecting / after disconnect.
    pub(crate) connection_log: VecDeque<String>,
    pub(crate) show_connection_log: bool,
    // Set when a new log entry arrives while the log panel is closed, cleared when opened.
    pub(crate) connection_log_unread: bool,
    // How many connection attempts have been made since the last explicit Connect click.
    pub(crate) connection_attempts: u32,
}

pub(crate) struct CompletionState {
    pub(crate) original_word: String,
    pub(crate) matches: Vec<String>,
    pub(crate) index: usize,
    pub(crate) word_start_idx: usize,
}

impl WeeChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (image_tx, image_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();

        let settings: AppSettings = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            AppSettings::default()
        };

        let saved_password = crate::ui::secure_storage::load(&settings.host, &settings.port);
        let password_from_keyring = saved_password.is_some();

        if !settings.font_path.is_empty() {
            crate::ui::fonts::apply(&cc.egui_ctx, &settings.font_path);
        }
        let available_fonts = crate::ui::fonts::scan_system_fonts();

        Self {
            host: settings.host,
            port: settings.port,
            password: saved_password.unwrap_or_default(),
            save_password: settings.save_password,
            password_from_keyring,
            use_ssl: settings.use_ssl,
            client: None,
            event_rx,
            event_tx,
            connection_status: "".to_string(),
            is_connecting: false,
            connecting_pending: false,
            auth_error: None,
            buffers: Vec::new(),
            selected_buffer_id: None,
            input_text: String::new(),
            show_settings: false,
            show_filtered_lines: settings.show_filtered_lines,
            colored_nicks: settings.colored_nicks,
            theme: settings.theme,
            font_size: settings.font_size,
            use_monospace: settings.use_monospace,
            show_timestamps: settings.show_timestamps,
            show_buffers: settings.show_buffers,
            show_nicklist: settings.show_nicklist,
            auto_reconnect: settings.auto_reconnect,
            show_titlebar: settings.show_titlebar,
            show_server_headers: settings.show_server_headers,
            show_inline_images: settings.show_inline_images,
            show_link_previews: settings.show_link_previews,
            opacity: settings.opacity,
            show_hidden_buffers: settings.show_hidden_buffers,
            image_cache: HashMap::new(),
            image_expanded: HashSet::new(),
            image_tx,
            image_rx,
            preview_cache: HashMap::new(),
            preview_expanded: HashSet::new(),
            preview_tx,
            preview_rx,
            completion: None,
            command_history: VecDeque::new(),
            history_index: None,
            show_search: false,
            search_text: String::new(),
            pending_buffer_switch: None,
            buffer_order: settings.buffer_order,
            dragging_buffer_id: None,
            drag_drop_before_id: None,
            cleared_buffer_ids: settings.cleared_buffer_ids,
            font_name: settings.font_name,
            font_path: settings.font_path.clone(),
            applied_font_path: settings.font_path,
            available_fonts,
            selected_view_since: None,
            muted_buffer_names: settings.muted_buffer_names,
            loading_more_buffer_id: None,
            font_search: String::new(),
            connection_log: VecDeque::new(),
            show_connection_log: false,
            connection_log_unread: false,
            connection_attempts: 0,
        }
    }

    pub(crate) fn hash_nick(name: &str) -> u8 {
        let mut h: u32 = 0;
        for b in name.as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(*b as u32);
        }
        ((h % 15) + 1) as u8
    }

    pub(crate) fn draw_sidebar_icon(painter: &Painter, rect: Rect, color: Color32, is_right: bool) {
        let stroke = Stroke::new(1.5, color);
        let rounding = Rounding::same(2.0);
        painter.rect_stroke(rect.shrink(4.0), rounding, stroke);
        
        let split_x = if is_right { rect.right() - 8.0 } else { rect.left() + 8.0 };
        painter.line_segment(
            [egui::pos2(split_x, rect.top() + 4.0), egui::pos2(split_x, rect.bottom() - 4.0)],
            stroke
        );
    }

    pub(crate) fn is_image_url(url: &str) -> bool {
        let path = url.split('?').next().unwrap_or(url).to_lowercase();
        matches!(
            std::path::Path::new(&path).extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
        )
    }

    pub(crate) fn select_buffer(&mut self, id: String) {
        // Clear the hotlist entry for the buffer we're LEAVING so WeeChat's own
        // TUI reflects it as read.  POST /api/buffers/{id}/read does not exist in
        // the relay API — the correct approach is POST /api/input with the
        // WeeChat command "/input hotlist_clear" targeted at the specific buffer.
        if let Some(prev_id) = self.selected_buffer_id.clone() {
            if prev_id != id {
                if let Some(client) = &self.client {
                    if let Ok(numeric_id) = prev_id.parse::<i64>() {
                        client.send_api("POST /api/input", None, Some(serde_json::json!({
                            "buffer_id": numeric_id,
                            "command": "/input hotlist_clear"
                        })));
                    }
                }
            }
        }

        self.selected_buffer_id = Some(id.clone());
        self.selected_view_since = Some(std::time::Instant::now());
        // Mark as cleared so stale hotlist entries are suppressed on reconnect.
        self.cleared_buffer_ids.insert(id.clone());
        if let Some(buffer) = self.buffers.iter_mut().find(|b| b.id == id) {
            buffer.activity = BufferActivity::None;
            buffer.unread_count = 0;
            // Snapshot the current read marker so the unread divider stays anchored
            // at the right position while the user views this buffer.
            buffer.visit_start_marker_id = buffer.last_read_id.clone();
            let fetch_nicks = buffer.has_nicklist;
            if let Some(client) = &self.client {
                client.send_api(&format!("GET /api/buffers/{}", id), Some(&format!("_buffer_info:{}", id)), None);
                client.send_api(&format!("GET /api/buffers/{}/lines?lines=-{}", id, INITIAL_LINES), Some(&format!("_buffer_lines:{}", id)), None);
                if fetch_nicks {
                    client.send_api(&format!("GET /api/buffers/{}/nicks", id), Some(&format!("_nicks:{}", id)), None);
                }
                // Also clear hotlist on entry so it's covered if we entered directly.
                if let Ok(numeric_id) = id.parse::<i64>() {
                    client.send_api("POST /api/input", None, Some(serde_json::json!({
                        "buffer_id": numeric_id,
                        "command": "/input hotlist_clear"
                    })));
                }
            }
        }
    }

    pub(crate) fn log_conn(&mut self, msg: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        self.connection_log.push_back(format!("[{}]  {}", ts, msg.into()));
        if self.connection_log.len() > 200 {
            self.connection_log.pop_front();
        }
        if !self.show_connection_log {
            self.connection_log_unread = true;
        }
    }
}

impl eframe::App for WeeChatApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = AppSettings {
            host: self.host.clone(),
            port: self.port.clone(),
            use_ssl: self.use_ssl,
            show_filtered_lines: self.show_filtered_lines,
            colored_nicks: self.colored_nicks,
            theme: self.theme.clone(),
            font_size: self.font_size,
            use_monospace: self.use_monospace,
            show_timestamps: self.show_timestamps,
            show_buffers: self.show_buffers,
            show_nicklist: self.show_nicklist,
            auto_reconnect: self.auto_reconnect,
            show_titlebar: self.show_titlebar,
            show_server_headers: self.show_server_headers,
            show_inline_images: self.show_inline_images,
            show_link_previews: self.show_link_previews,
            opacity: self.opacity,
            show_hidden_buffers: self.show_hidden_buffers,
            buffer_order: self.buffer_order.clone(),
            cleared_buffer_ids: self.cleared_buffer_ids.clone(),
            save_password: self.save_password,
            font_name: self.font_name.clone(),
            font_path: self.font_path.clone(),
            muted_buffer_names: self.muted_buffer_names.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event);
        }

        while let Ok((url, result)) = self.image_rx.try_recv() {
            match result {
                Ok(bytes) => {
                    match image::load_from_memory(&bytes) {
                        Ok(img) => {
                            let rgba = img.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let color_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                            let handle = ctx.load_texture(&url, color_img, egui::TextureOptions::default());
                            self.image_cache.insert(url, ImageState::Loaded(handle));
                        }
                        Err(_) => { self.image_cache.insert(url, ImageState::Failed); }
                    }
                }
                Err(_) => { self.image_cache.insert(url, ImageState::Failed); }
            }
        }

        while let Ok((url, result)) = self.preview_rx.try_recv() {
            match result {
                Ok(preview) => {
                    // Auto-load the og:image through the existing image channel
                    if let Some(img_url) = &preview.image_url {
                        if !self.image_cache.contains_key(img_url) {
                            self.image_cache.insert(img_url.clone(), ImageState::Loading);
                            let tx = self.image_tx.clone();
                            let img_url_owned = img_url.clone();
                            tokio::spawn(async move {
                                let result = async {
                                    let bytes = reqwest::get(&img_url_owned).await?.bytes().await?;
                                    Ok::<Vec<u8>, reqwest::Error>(bytes.to_vec())
                                }.await;
                                let _ = tx.send((img_url_owned, result.map_err(|e| e.to_string())));
                            });
                        }
                    }
                    self.preview_cache.insert(url, PreviewState::Loaded(preview));
                }
                Err(_) => { self.preview_cache.insert(url, PreviewState::Failed); }
            }
        }

        let mut tab_pressed = false;
        let mut arrow_up_shortcut = false;
        let mut arrow_down_shortcut = false;
        let mut history_up = false;
        let mut history_down = false;
        let mut search_shortcut = false;

        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Tab) {
                tab_pressed = true;
            }
            
            let meta = i.modifiers.command || i.modifiers.alt || i.modifiers.mac_cmd || i.modifiers.ctrl;
            if meta {
                if i.consume_key(i.modifiers, Key::ArrowUp) || i.key_pressed(Key::ArrowUp) { arrow_up_shortcut = true; }
                if i.consume_key(i.modifiers, Key::ArrowDown) || i.key_pressed(Key::ArrowDown) { arrow_down_shortcut = true; }
                if i.consume_key(i.modifiers, Key::F) { search_shortcut = true; }
                if i.consume_key(i.modifiers, Key::B) { self.show_buffers = !self.show_buffers; }
                if i.consume_key(i.modifiers, Key::N) { self.show_nicklist = !self.show_nicklist; }
            } else {
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) { history_up = true; }
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) { history_down = true; }
            }
        });

        if arrow_up_shortcut { self.cycle_buffer(-1); }
        if arrow_down_shortcut { self.cycle_buffer(1); }
        if search_shortcut { self.show_search = !self.show_search; }

        if self.font_path != self.applied_font_path {
            crate::ui::fonts::apply(ctx, &self.font_path);
            self.applied_font_path = self.font_path.clone();
        }

        let mut style = (*ctx.style()).clone();
        let font_family = if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional };
        
        style.text_styles = [
            (TextStyle::Small, FontId::new(self.font_size * 0.8, font_family.clone())),
            (TextStyle::Body, FontId::new(self.font_size, font_family.clone())),
            (TextStyle::Button, FontId::new(self.font_size, font_family.clone())),
            (TextStyle::Heading, FontId::new(self.font_size * 1.4, font_family.clone())),
            (TextStyle::Monospace, FontId::new(self.font_size, FontFamily::Monospace)),
        ].into();
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
        style.spacing.window_margin = Margin::same(12.0);
        style.visuals.window_rounding = Rounding::same(12.0);
        style.visuals.widgets.noninteractive.rounding = Rounding::same(8.0);
        style.visuals.widgets.inactive.rounding = Rounding::same(8.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(8.0);
        style.visuals.widgets.active.rounding = Rounding::same(8.0);
        ctx.set_style(style);

        let accent_color = Color32::from_rgb(100, 149, 237);
        let base_bg = self.theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18));
        let alpha = (self.opacity * 255.0) as u8;
        let bg_color = Color32::from_rgba_unmultiplied(base_bg.r(), base_bg.g(), base_bg.b(), alpha);

        // Detect light vs dark so every derived color adapts automatically
        let luma = 0.299 * base_bg.r() as f32 + 0.587 * base_bg.g() as f32 + 0.114 * base_bg.b() as f32;
        let is_light = luma > 140.0;

        // Surface: slightly darker than bg for light themes, slightly lighter for dark
        let surface_color = if is_light {
            Color32::from_rgba_unmultiplied(
                base_bg.r().saturating_sub(18),
                base_bg.g().saturating_sub(18),
                base_bg.b().saturating_sub(18),
                alpha,
            )
        } else {
            Color32::from_rgba_unmultiplied(30, 30, 30, alpha)
        };

        // Card background for preview panels
        let card_bg = if is_light {
            Color32::from_rgba_unmultiplied(
                base_bg.r().saturating_sub(12),
                base_bg.g().saturating_sub(12),
                base_bg.b().saturating_sub(12),
                230,
            )
        } else {
            Color32::from_rgba_unmultiplied(35, 35, 45, 220)
        };

        // Semantic text tiers — text_primary follows the theme foreground when set
        let text_primary = self.theme.foreground
            .map(Color32::from)
            .unwrap_or_else(|| if is_light { Color32::from_gray(15) } else { Color32::WHITE });
        let text_secondary = if is_light { Color32::from_gray(70)  } else { Color32::from_gray(160) };
        let text_muted     = if is_light { Color32::from_gray(120) } else { Color32::from_gray(100) };

        // Border for frames/cards
        let border_color = if is_light { Color32::from_gray(200) } else { Color32::from_gray(55) };

        let mut visuals = if is_light { Visuals::light() } else { Visuals::dark() };
        visuals.panel_fill = bg_color;
        visuals.window_fill = surface_color;
        visuals.extreme_bg_color = if is_light {
            Color32::from_rgba_unmultiplied(255, 255, 255, alpha)
        } else {
            Color32::from_rgba_unmultiplied(10, 10, 10, alpha)
        };
        visuals.widgets.active.bg_fill = accent_color;
        visuals.selection.bg_fill = accent_color.linear_multiply(0.5);
        if let Some(fg) = self.theme.foreground {
            visuals.override_text_color = Some(fg.into());
        } else if is_light {
            visuals.override_text_color = Some(text_primary);
        }
        ctx.set_visuals(visuals);

        let mut next_selected_buffer_id = None;
        let mut pending_buffer_command = None;
        let mut next_drag_buffer_id: Option<String> = None;
        let mut pending_mute: Option<(String, String, bool)> = None; // (buf_id, full_name, mute)
        let mut pending_load_more: Option<(String, usize)> = None; // (buffer_id, lines_to_request)

        egui::TopBottomPanel::top("top_panel")
            .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(12.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    
                    let icon_size = Vec2::splat(24.0);
                    
                    let (rect, res) = ui.allocate_at_least(icon_size, egui::Sense::click());
                    if res.clicked() { self.show_buffers = !self.show_buffers; }
                    let color = if self.show_buffers { accent_color } else { Color32::GRAY };
                    Self::draw_sidebar_icon(ui.painter(), rect, color, false);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, res) = ui.allocate_at_least(icon_size, egui::Sense::click());
                        if res.clicked() { self.show_nicklist = !self.show_nicklist; }
                        let color = if self.show_nicklist { accent_color } else { Color32::GRAY };
                        Self::draw_sidebar_icon(ui.painter(), rect, color, true);
                        ui.add_space(8.0);

                        if ui.button(egui::RichText::new("⚙").size(16.0)).on_hover_text("Settings").clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        let log_icon = if self.connection_log_unread {
                            egui::RichText::new("⬡").size(16.0).color(Color32::from_rgb(255, 165, 0))
                        } else {
                            egui::RichText::new("⬡").size(16.0)
                        };
                        if ui.button(log_icon).on_hover_text("Connection log").clicked() {
                            self.show_connection_log = !self.show_connection_log;
                            self.connection_log_unread = false;
                            self.show_settings = false;
                        }
                        if self.client.is_some() {
                            let status_text = if self.is_connecting { "● Connecting" } else { "● Connected" };
                            let status_color = if self.is_connecting { Color32::from_rgb(255, 165, 0) } else { Color32::from_rgb(50, 205, 50) };
                            ui.label(egui::RichText::new(status_text).color(status_color).small());
                            
                            if ui.button("Disconnect").clicked() {
                                if let Some(client) = &self.client {
                                    client.disconnect();
                                }
                                self.client = None;
                                self.connection_status = "Disconnected".to_string();
                                self.log_conn("Disconnected by user");
                            }
                        }
                    });
                });
            });

        if self.show_buffers {
            egui::SidePanel::left("buffers_panel")
                .resizable(true)
                .default_width(180.0)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("BUFFERS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);

                    let is_dragging = self.dragging_buffer_id.is_some();
                    let pointer_pos = ctx.pointer_hover_pos();

                    // Pre-compute which buffer IDs belong to the dragged group so we can fade
                    // them all out and skip them as drop targets.
                    let dragged_group_ids: HashSet<String> = if let Some(drag_id) = &self.dragging_buffer_id {
                        if let Some(drag_buf) = self.buffers.iter().find(|b| &b.id == drag_id) {
                            if drag_buf.kind == "server" || drag_buf.kind == "core" {
                                let skey = drag_buf.server.clone();
                                self.buffers.iter().filter(|b| b.server == skey).map(|b| b.id.clone()).collect()
                            } else {
                                std::iter::once(drag_id.clone()).collect()
                            }
                        } else { HashSet::new() }
                    } else { HashSet::new() };

                    let mut row_rects: Vec<(egui::Rect, String)> = Vec::new();

                    ScrollArea::vertical().show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for buffer in &self.buffers {
                            if buffer.hidden && !self.show_hidden_buffers {
                                continue;
                            }
                            let is_selected = self.selected_buffer_id.as_deref() == Some(&buffer.id);
                            let is_root = buffer.kind == "server" || buffer.kind == "core";
                            let is_child = buffer.kind == "channel" || buffer.kind == "private";
                            let in_dragged_group = dragged_group_ids.contains(&buffer.id);

                            if self.show_server_headers && is_root {
                                ui.add_space(8.0);
                            }

                            let is_muted = buffer.muted;
                            let (bg, fg) = if is_selected {
                                (accent_color.linear_multiply(0.2), text_primary)
                            } else if is_muted {
                                (Color32::TRANSPARENT, text_muted.linear_multiply(0.6))
                            } else if self.show_server_headers && is_root && buffer.activity == BufferActivity::None {
                                (Color32::TRANSPARENT, accent_color.linear_multiply(0.75))
                            } else {
                                match buffer.activity {
                                    BufferActivity::Highlight => (Color32::from_rgb(150, 50, 50).linear_multiply(0.3), Color32::from_rgb(255, 100, 100)),
                                    BufferActivity::Message   => (Color32::TRANSPARENT, text_primary),
                                    BufferActivity::Metadata  => (Color32::TRANSPARENT, text_secondary),
                                    BufferActivity::None      => (Color32::TRANSPARENT, text_muted),
                                }
                            };

                            let indent = if is_child { 12.0 } else { 0.0 };

                            let outer_resp = ui.horizontal(|ui| {
                                ui.add_space(indent);
                                Frame::none()
                                    .fill(if in_dragged_group { bg.linear_multiply(0.35) } else { bg })
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(Margin::symmetric(8.0, 4.0))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        let name = if !is_muted && buffer.activity == BufferActivity::Highlight {
                                            format!("• {}", buffer.name)
                                        } else if is_muted {
                                            format!("🔇 {}", buffer.name)
                                        } else {
                                            buffer.name.clone()
                                        };
                                        let label = if self.show_server_headers && is_root {
                                            egui::RichText::new(name.to_uppercase()).color(fg).italics()
                                        } else if is_muted {
                                            egui::RichText::new(name).color(fg).italics()
                                        } else {
                                            egui::RichText::new(name).color(fg).strong()
                                        };
                                        let unread = if is_muted || is_root { 0 } else { buffer.unread_count };
                                        let buf_activity = buffer.activity;
                                        if unread > 0 {
                                            ui.horizontal(|ui| {
                                                ui.label(label);
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    let badge_text = if unread > 99 { "99+".to_string() } else { unread.to_string() };
                                                    let badge_bg = if buf_activity == BufferActivity::Highlight {
                                                        Color32::from_rgb(200, 50, 50)
                                                    } else {
                                                        accent_color
                                                    };
                                                    Frame::none()
                                                        .fill(badge_bg)
                                                        .rounding(Rounding::same(8.0))
                                                        .inner_margin(Margin::symmetric(4.0, 1.0))
                                                        .show(ui, |ui| {
                                                            ui.label(egui::RichText::new(badge_text).color(Color32::WHITE).strong().size(self.font_size * 0.72));
                                                        });
                                                });
                                            });
                                        } else {
                                            ui.label(label);
                                        }
                                    });
                            }).response;

                            let row_id = egui::Id::new("buf_row").with(&buffer.id);
                            let resp = ui.interact(outer_resp.rect, row_id, egui::Sense::click_and_drag());

                            if resp.hovered() {
                                ctx.set_cursor_icon(if is_dragging { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
                            }
                            if resp.drag_started() {
                                next_drag_buffer_id = Some(buffer.id.clone());
                            }
                            if resp.clicked() && !is_dragging {
                                next_selected_buffer_id = Some(buffer.id.clone());
                            }
                            if !is_dragging {
                                let buf_id = buffer.id.clone();
                                let buf_full_name = buffer.full_name.clone();
                                let buf_kind = buffer.kind.clone();
                                let buf_hidden = buffer.hidden;
                                let buf_muted = buffer.muted;
                                resp.context_menu(|ui| {
                                    if buf_muted {
                                        if ui.button("🔔 Unmute Buffer").clicked() {
                                            pending_mute = Some((buf_id.clone(), buf_full_name.clone(), false));
                                            ui.close_menu();
                                        }
                                    } else {
                                        if ui.button("🔇 Mute Buffer").clicked() {
                                            pending_mute = Some((buf_id.clone(), buf_full_name.clone(), true));
                                            ui.close_menu();
                                        }
                                    }
                                    ui.separator();
                                    if buf_kind == "channel" {
                                        if ui.button("Leave Channel").clicked() {
                                            pending_buffer_command = Some((buf_id.clone(), "/part".to_string()));
                                            ui.close_menu();
                                        }
                                    }
                                    if buf_hidden {
                                        if ui.button("Unhide Buffer").clicked() {
                                            pending_buffer_command = Some((buf_id.clone(), "/buffer unhide".to_string()));
                                            ui.close_menu();
                                        }
                                    } else {
                                        if ui.button("Hide Buffer").clicked() {
                                            pending_buffer_command = Some((buf_id.clone(), "/buffer hide".to_string()));
                                            ui.close_menu();
                                        }
                                    }
                                    if ui.button("Close Buffer").clicked() {
                                        pending_buffer_command = Some((buf_id.clone(), "/close".to_string()));
                                        ui.close_menu();
                                    }
                                });
                            }

                            // Only non-dragged-group rows are valid drop targets.
                            if !in_dragged_group {
                                row_rects.push((outer_resp.rect, buffer.id.clone()));
                            }
                        }
                    });

                    // Handle drag start (must happen after the scroll area, outside of it).
                    if let Some(id) = next_drag_buffer_id.take() {
                        self.dragging_buffer_id = Some(id);
                    }

                    // Update drop target and draw the indicator line.
                    // This runs every frame while dragging, using current pointer position
                    // vs the collected row_rects (all in screen-space coordinates).
                    if self.dragging_buffer_id.is_some() {
                        if let Some(pos) = pointer_pos {
                            let mut drop_before: Option<String> = None;
                            let mut indicator_y: f32 = row_rects.last().map(|(r, _)| r.bottom()).unwrap_or(0.0);

                            for (rect, id) in &row_rects {
                                if pos.y < rect.center().y {
                                    drop_before = Some(id.clone());
                                    indicator_y = rect.top();
                                    break;
                                }
                                indicator_y = rect.bottom();
                            }

                            self.drag_drop_before_id = drop_before;

                            if let Some((first, _)) = row_rects.first() {
                                // Draw over the scroll area — ui.painter() here is the panel painter.
                                ui.painter().hline(
                                    first.left()..=first.right(),
                                    indicator_y,
                                    Stroke::new(2.0, accent_color),
                                );
                            }
                        }
                    }

                    // Handle drag release.
                    if is_dragging && ctx.input(|i| !i.pointer.primary_down()) {
                        if let Some(drag_id) = self.dragging_buffer_id.take() {
                            let drop_id = self.drag_drop_before_id.take();
                            apply_drag_reorder(&mut self.buffers, &drag_id, drop_id.as_deref());
                            self.buffer_order = self.buffers.iter().map(|b| b.id.clone()).collect();
                        }
                        self.drag_drop_before_id = None;
                    }
                });
        }

        if let Some(id) = next_selected_buffer_id {
            self.select_buffer(id);
        }

        if let Some((buf_id, full_name, mute)) = pending_mute {
            if mute {
                self.muted_buffer_names.insert(full_name);
            } else {
                self.muted_buffer_names.remove(&full_name);
            }
            if let Some(b) = self.buffers.iter_mut().find(|b| b.id == buf_id) {
                b.muted = mute;
                if mute {
                    b.activity = BufferActivity::None;
                    b.unread_count = 0;
                }
            }
        }

        if let Some((id, cmd)) = pending_buffer_command {
            self.send_command_to_buffer(&id, &cmd);
        }

        let current_buffer_id = self.selected_buffer_id.clone();
        let current_buf = current_buffer_id.as_ref().and_then(|id| self.buffers.iter().find(|b| &b.id == id));
        let current_buffer_nicks = current_buf.map(|b| b.nicks.clone());
        let current_buffer_full_name = current_buf.map(|b| b.full_name.clone());
        let current_buffer_messages = current_buf.map(|b| b.messages.clone());
        let _current_buffer_last_read_id = current_buf.and_then(|b| b.last_read_id.clone());
        let current_buffer_visit_marker_id = current_buf.and_then(|b| b.visit_start_marker_id.clone());
        let current_buffer_topic = current_buf.map(|b| b.topic.clone()).unwrap_or_default();
        let current_buffer_modes = current_buf.map(|b| b.modes.clone()).unwrap_or_default();
        let current_buffer_kind = current_buf.map(|b| b.kind.clone()).unwrap_or_default();

        let font_id = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });

        let is_query_or_core = current_buffer_kind == "private" || current_buffer_kind == "server" || current_buffer_kind == "core" || current_buffer_full_name.as_ref().map(|n| n == "weechat" || n.contains("highmon")).unwrap_or(false);

        if self.show_nicklist && !is_query_or_core && self.client.is_some() && current_buffer_id.is_some() {
            egui::SidePanel::right("nicks_panel")
                .resizable(true)
                .default_width(140.0)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("NICKS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);
                    ScrollArea::vertical().show(ui, |ui| {
                        if let Some(nicks) = &current_buffer_nicks {
                            for nick in nicks {
                                let text = format!("{}{}", nick.prefix, nick.name);
                                let input = if self.colored_nicks {
                                    if self.theme.name == "Default" { format!("{}{}", nick.color_ansi, text) }
                                    else {
                                        let idx = Self::hash_nick(&nick.name);
                                        let esc = if idx < 8 { format!("\x1B[{}m", 30 + idx) } else { format!("\x1B[{}m", 90 + idx - 8) };
                                        format!("{}{}", esc, text)
                                    }
                                } else { text };
                                let sections = ANSIParser::parse(&input, font_id.clone(), &self.theme);
                                let mut job = LayoutJob::default();
                                for s in sections { job.append(&s.text, 0.0, s.format); }
                                
                                let label_res = ui.add(Label::new(job).wrap(false).sense(egui::Sense::click()));
                                label_res.context_menu(|ui| {
                                    if ui.button(format!("Query {}", nick.name)).clicked() {
                                        self.send_command(&format!("/query {}", nick.name));
                                        ui.close_menu();
                                    }
                                    if ui.button(format!("Whois {}", nick.name)).clicked() {
                                        self.send_command(&format!("/whois {}", nick.name));
                                        ui.close_menu();
                                    }
                                });
                            }
                        }
                    });
                });
        }

        if self.client.is_some() && current_buffer_id.is_some() {
            egui::TopBottomPanel::bottom("input_panel")
                .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(16.0, 10.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let text_edit = egui::TextEdit::singleline(&mut self.input_text)
                            .hint_text("Type a message...")
                            .margin(Margin::symmetric(8.0, 4.0))
                            .lock_focus(true) 
                            .desired_width(ui.available_width() - 80.0);
                        
                        let res = ui.add(text_edit);
                        
                        if res.has_focus() {
                            if tab_pressed { 
                                self.perform_completion(ctx, res.id); 
                                res.request_focus();
                            } else if history_up {
                                self.cycle_history(-1, ctx, res.id);
                                res.request_focus();
                            } else if history_down {
                                self.cycle_history(1, ctx, res.id);
                                res.request_focus();
                            } else {
                                let any_other_key = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
                                if any_other_key && !tab_pressed && !history_up && !history_down {
                                    self.completion = None;
                                }
                            }
                        }

                        if ui.add(egui::Button::new(egui::RichText::new("Send").color(Color32::WHITE).strong()).fill(accent_color).min_size(Vec2::new(60.0, 0.0))).clicked() || (res.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter))) {
                            self.send_current_message();
                            res.request_focus();
                        }
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
            if self.show_settings {
                self.show_settings_window(ui, accent_color, is_light);
            } else if self.show_connection_log {
                // Connection log panel — toggled via toolbar button, closed with X or Escape.
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.show_connection_log = false;
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    let log_w = (ui.available_width() - 80.0).min(720.0);
                    ui.allocate_ui(egui::vec2(log_w, ui.available_height() - 32.0), |ui| {
                        ui.horizontal(|ui| {
                            let (dot_color, label) = if self.connecting_pending {
                                (Color32::from_rgb(255, 165, 0), "Connecting…")
                            } else if self.client.is_some() {
                                (Color32::from_rgb(50, 205, 50), "Connected")
                            } else {
                                (Color32::from_rgb(220, 60, 60), "Disconnected")
                            };
                            ui.label(egui::RichText::new("●").color(dot_color).size(14.0));
                            ui.label(egui::RichText::new(label).strong().size(14.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(egui::RichText::new("✕").size(14.0)).on_hover_text("Close").clicked() {
                                    self.show_connection_log = false;
                                }
                            });
                        });
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(if is_light { Color32::from_gray(245) } else { Color32::from_rgb(14, 14, 14) })
                            .rounding(egui::Rounding::same(8.0))
                            .stroke(egui::Stroke::new(1.0, border_color))
                            .inner_margin(egui::Margin::same(12.0))
                            .show(ui, |ui| {
                                let log_font = egui::FontId::new(self.font_size * 0.88, egui::FontFamily::Monospace);
                                egui::ScrollArea::vertical()
                                    .stick_to_bottom(true)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        if self.connection_log.is_empty() {
                                            ui.label(egui::RichText::new("No connection activity yet.").color(text_muted).italics());
                                        }
                                        for entry in &self.connection_log {
                                            let color = if entry.contains("Error") || entry.contains("failed") || entry.contains("Disconnected") {
                                                Color32::from_rgb(220, 80, 80)
                                            } else if entry.contains("Connected") {
                                                Color32::from_rgb(50, 205, 50)
                                            } else {
                                                text_secondary
                                            };
                                            ui.label(egui::RichText::new(entry).font(log_font.clone()).color(color));
                                        }
                                        if self.connecting_pending {
                                            ui.spinner();
                                        }
                                    });
                            });
                    });
                });
            } else if self.client.is_none() || self.connecting_pending {
                ui.vertical_centered(|ui| {
                    ui.add_space(ctx.available_rect().height() * 0.2);
                    
                    Frame::group(ui.style())
                        .fill(surface_color)
                        .rounding(Rounding::same(12.0))
                        .stroke(Stroke::new(1.0, border_color))
                        .inner_margin(Margin::same(30.0))
                        .show(ui, |ui| {
                            ui.set_max_width(400.0);
                            ui.heading(egui::RichText::new("Connect to WeeChatRS").strong().size(24.0));
                            ui.add_space(20.0);
                            
                            egui::Grid::new("login_grid").num_columns(2).spacing([15.0, 15.0]).show(ui, |ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Host:"); });
                                if ui.add(egui::TextEdit::singleline(&mut self.host).desired_width(240.0)).changed() { self.auth_error = None; }
                                ui.end_row();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Port:"); });
                                if ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(240.0)).changed() { self.auth_error = None; }
                                ui.end_row();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label("Password:"); });
                                ui.horizontal(|ui| {
                                    if ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(200.0)).changed() { self.auth_error = None; }
                                    if self.password_from_keyring {
                                        ui.label(egui::RichText::new("(keyring)").small().color(ui.visuals().weak_text_color()));
                                    }
                                });
                                ui.end_row();
                            });
                            
                            ui.add_space(15.0);
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.use_ssl, "Use SSL");
                                ui.add_space(20.0);
                                ui.checkbox(&mut self.auto_reconnect, "Auto-reconnect");
                            });
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut self.save_password, "Remember password").changed() && !self.save_password {
                                    let _ = crate::ui::secure_storage::delete(&self.host, &self.port);
                                    self.password_from_keyring = false;
                                }
                                if self.password_from_keyring {
                                    ui.add_space(12.0);
                                    if ui.small_button("Forget").clicked() {
                                        let _ = crate::ui::secure_storage::delete(&self.host, &self.port);
                                        self.password.clear();
                                        self.password_from_keyring = false;
                                        self.save_password = false;
                                    }
                                }
                            });
                            ui.add_space(25.0);

                            ui.horizontal(|ui| {
                                let btn_label = if self.connecting_pending { "Connecting…" } else { "Connect" };
                                let btn = egui::Button::new(egui::RichText::new(btn_label).strong().color(Color32::WHITE))
                                    .fill(if self.connecting_pending { accent_color.linear_multiply(0.5) } else { accent_color })
                                    .min_size(Vec2::new(120.0, 40.0));
                                if ui.add_enabled(!self.connecting_pending, btn).clicked() {
                                    if self.save_password && !self.password.is_empty() {
                                        let _ = crate::ui::secure_storage::save(&self.host, &self.port, &self.password);
                                        self.password_from_keyring = true;
                                    }
                                    let port = self.port.parse().unwrap_or(9001);
                                    self.auth_error = None;
                                    self.connecting_pending = true;
                                    self.connection_status = "Connecting...".to_string();
                                    self.connection_log.clear();
                                    self.connection_attempts = 0;
                                    let proto = if self.use_ssl { "wss" } else { "ws" };
                                    self.log_conn(format!("Connecting to {}://{}:{}/api", proto, self.host, port));
                                    self.log_conn(format!("SSL/TLS: {}", if self.use_ssl { "enabled" } else { "disabled" }));
                                    self.log_conn("Auth method: base64url bearer token over Sec-WebSocket-Protocol header");
                                    self.client = Some(RelayClient::connect(self.host.clone(), port, self.password.clone(), self.use_ssl, self.event_tx.clone(), ctx.clone()));
                                }
                            });

                            ui.add_space(15.0);
                            if let Some(err) = &self.auth_error.clone() {
                                ui.label(egui::RichText::new(format!("⚠ {}", err)).color(Color32::from_rgb(220, 80, 80)));
                            } else if self.connecting_pending {
                                ui.label(egui::RichText::new("Connecting…").color(accent_color));
                            } else if !self.connection_status.is_empty() && self.connection_status.starts_with("Error") {
                                ui.label(egui::RichText::new(&self.connection_status.clone()).color(Color32::from_rgb(220, 80, 80)));
                            }
                        });
                });
            } else if let Some(_full_name) = current_buffer_full_name {
                ui.vertical(|ui| {
                    if self.show_search {
                        Frame::none()
                            .fill(surface_color.linear_multiply(0.8))
                            .inner_margin(Margin::symmetric(16.0, 8.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("🔍");
                                    let res = ui.add(egui::TextEdit::singleline(&mut self.search_text)
                                        .hint_text("Search scrollback...")
                                        .desired_width(ui.available_width() - 40.0));
                                    if ui.button("❌").clicked() {
                                        self.show_search = false;
                                        self.search_text.clear();
                                    }
                                    if self.show_search { res.request_focus(); }
                                });
                            });
                        ui.separator();
                    }

                    if self.show_titlebar && (!current_buffer_topic.is_empty() || !current_buffer_modes.is_empty()) {
                        Frame::none()
                            .fill(surface_color.linear_multiply(0.3))
                            .inner_margin(Margin::symmetric(16.0, 6.0))
                            .stroke(Stroke::new(1.0, Color32::from_white_alpha(10)))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    if !current_buffer_modes.is_empty() {
                                        ui.label(egui::RichText::new(format!("[{}]", current_buffer_modes)).color(accent_color).small());
                                    }
                                    if !current_buffer_topic.is_empty() {
                                        let topic_font = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });
                                        let sections = ANSIParser::parse(&current_buffer_topic, topic_font, &self.theme);
                                        let mut job = LayoutJob::default();
                                        for s in sections { job.append(&s.text, 0.0, s.format); }
                                        ui.add(Label::new(job).wrap(true)); 
                                    }
                                });
                            });
                        ui.add_space(-1.0);
                        ui.separator();
                    }
                    
                    ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        Frame::none().inner_margin(Margin::same(16.0)).show(ui, |ui| {
                            // "Load more" button — shown at the top when there may be older lines.
                            if let (Some(buf_id), Some(messages)) = (current_buffer_id.as_ref(), current_buffer_messages.as_ref()) {
                                if messages.len() >= INITIAL_LINES {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.add_space((ui.available_width() - 180.0).max(0.0) / 2.0);
                                        if self.loading_more_buffer_id.as_deref() == Some(buf_id.as_str()) {
                                            ui.spinner();
                                            ui.label(egui::RichText::new("Loading…").color(text_muted).small());
                                        } else if ui.button("⬆ Load older messages").clicked() {
                                            pending_load_more = Some((buf_id.clone(), messages.len() + LOAD_MORE_LINES));
                                        }
                                    });
                                    ui.add_space(8.0);
                                    ui.scope(|ui| {
                                        ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border_color);
                                        ui.separator();
                                    });
                                    ui.add_space(4.0);
                                }
                            }

                            if let Some(messages) = &current_buffer_messages {
                                let mut marker_shown = false;
                                let search_query = if self.search_text.is_empty() {
                                    None
                                } else {
                                    Some(self.search_text.to_lowercase())
                                };
                                for line in messages {
                                    if !self.show_filtered_lines && !line.displayed { continue; }

                                    if let Some(q) = &search_query {
                                        let clean_prefix = Self::strip_ansi(&line.prefix).to_lowercase();
                                        let clean_msg = Self::strip_ansi(&line.message).to_lowercase();
                                        if !clean_prefix.contains(q) && !clean_msg.contains(q) { continue; }
                                    }

                                    let past_visit_marker = current_buffer_visit_marker_id.as_ref().map(|vid| {
                                        let lid = line.id.parse::<i64>().unwrap_or(0);
                                        let vid = vid.parse::<i64>().unwrap_or(0);
                                        lid > vid
                                    }).unwrap_or(false);
                                    if !marker_shown && past_visit_marker {
                                        let elapsed = self.selected_view_since
                                            .map(|t| t.elapsed())
                                            .unwrap_or(std::time::Duration::from_secs(99));
                                        let divider_color = Color32::from_rgb(200, 50, 50);
                                        ui.add_space(8.0);
                                        if elapsed < std::time::Duration::from_secs(2) {
                                            // Text phase — schedule repaint for the transition.
                                            let remaining = std::time::Duration::from_secs(2) - elapsed;
                                            ctx.request_repaint_after(remaining);
                                            ui.horizontal(|ui| {
                                                ui.add_space(20.0);
                                                ui.separator();
                                                ui.label(egui::RichText::new(" NEW MESSAGES ").color(divider_color).size(10.0).strong());
                                                ui.separator();
                                            });
                                        } else {
                                            // Line phase — plain red rule.
                                            ui.scope(|ui| {
                                                ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, divider_color);
                                                ui.separator();
                                            });
                                        }
                                        ui.add_space(8.0);
                                        marker_shown = true;
                                    }

                                    let msg_sections = ANSIParser::parse(&line.message, font_id.clone(), &self.theme);

                                    // Collect image and preview URLs for display below
                                    let image_urls_in_line: Vec<String> = if self.show_inline_images {
                                        msg_sections.iter()
                                            .filter_map(|s| s.url.as_ref())
                                            .filter(|u| Self::is_image_url(u))
                                            .cloned()
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };
                                    let preview_urls_in_line: Vec<String> = if self.show_link_previews {
                                        msg_sections.iter()
                                            .filter_map(|s| s.url.as_ref())
                                            .filter(|u| !Self::is_image_url(u))
                                            .cloned()
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };

                                    // Two-part row: fixed left anchor (timestamp + prefix) keeps
                                    // wrapped message lines from bleeding back into the timestamp column.
                                    // TOP alignment ensures timestamp/nick always share the first text line.
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        if self.show_timestamps {
                                            ui.label(egui::RichText::new(line.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).font(font_id.clone()).color(text_muted));
                                        }
                                        let prefix_sections = ANSIParser::parse(&line.prefix, font_id.clone(), &self.theme);
                                        let mut prefix_job = LayoutJob::default();
                                        prefix_job.halign = egui::Align::LEFT;
                                        for s in prefix_sections { prefix_job.append(&s.text, 0.0, s.format); }
                                        ui.add(Label::new(prefix_job).wrap(false));

                                        // Message column: takes all remaining width. horizontal_wrapped
                                        // inside this vertical wraps back to the column's left edge, not x=0.
                                        let msg_col_width = ui.available_width();
                                        ui.vertical(|ui| {
                                            ui.set_min_width(msg_col_width);
                                            ui.horizontal_wrapped(|ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                for s in &msg_sections {
                                                    if let Some(url) = &s.url {
                                                        if ui.link(egui::RichText::new(&s.text).font(font_id.clone())).clicked() {
                                                            ui.ctx().output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url.clone())));
                                                        }
                                                        if self.show_inline_images && Self::is_image_url(url) {
                                                            let is_expanded = self.image_expanded.contains(url);
                                                            let btn = if is_expanded { "🖼" } else { "🖼 preview" };
                                                            if ui.small_button(btn).clicked() {
                                                                if is_expanded {
                                                                    self.image_expanded.remove(url);
                                                                } else {
                                                                    self.image_expanded.insert(url.clone());
                                                                    if !self.image_cache.contains_key(url) {
                                                                        self.image_cache.insert(url.clone(), ImageState::Loading);
                                                                        let tx = self.image_tx.clone();
                                                                        let url_owned = url.clone();
                                                                        tokio::spawn(async move {
                                                                            let result = async {
                                                                                let bytes = reqwest::get(&url_owned).await?.bytes().await?;
                                                                                Ok::<Vec<u8>, reqwest::Error>(bytes.to_vec())
                                                                            }.await;
                                                                            let _ = tx.send((url_owned, result.map_err(|e| e.to_string())));
                                                                        });
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        if self.show_link_previews && !Self::is_image_url(url) {
                                                            let is_expanded = self.preview_expanded.contains(url);
                                                            let btn = if is_expanded { "🔗" } else { "🔗 preview" };
                                                            if ui.small_button(btn).clicked() {
                                                                if is_expanded {
                                                                    self.preview_expanded.remove(url);
                                                                } else {
                                                                    self.preview_expanded.insert(url.clone());
                                                                    if !self.preview_cache.contains_key(url) {
                                                                        self.preview_cache.insert(url.clone(), PreviewState::Loading);
                                                                        let tx = self.preview_tx.clone();
                                                                        let url_owned = url.clone();
                                                                        tokio::spawn(async move {
                                                                            let result = fetch_link_preview(url_owned.clone()).await;
                                                                            let _ = tx.send((url_owned, result));
                                                                        });
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    } else {
                                                        let mut job = LayoutJob::default();
                                                        job.append(&s.text, 0.0, s.format.clone());
                                                        ui.add(Label::new(job).wrap(true));
                                                    }
                                                }
                                            });

                                            // Image previews (inside message column — indented correctly)
                                            if self.show_inline_images {
                                                for url in &image_urls_in_line {
                                                    if self.image_expanded.contains(url) {
                                                        ui.add_space(4.0);
                                                        match self.image_cache.get(url) {
                                                            Some(ImageState::Loaded(texture)) => {
                                                                let orig = texture.size_vec2();
                                                                let max_w = (ui.available_width() - 32.0).min(500.0);
                                                                let scale = if orig.x > max_w { max_w / orig.x } else { 1.0 };
                                                                ui.add(egui::Image::new((texture.id(), orig * scale)).rounding(4.0));
                                                            }
                                                            Some(ImageState::Loading) | None => {
                                                                ui.label(egui::RichText::new("Loading image…").color(text_muted).italics().small());
                                                            }
                                                            Some(ImageState::Failed) => {
                                                                ui.label(egui::RichText::new("Failed to load image").color(Color32::from_rgb(220, 80, 80)).small());
                                                            }
                                                        }
                                                        ui.add_space(4.0);
                                                    }
                                                }
                                            }

                                            // Link preview cards (inside message column — indented correctly)
                                            if self.show_link_previews {
                                                for url in &preview_urls_in_line {
                                                    if !self.preview_expanded.contains(url) { continue; }
                                                    ui.add_space(4.0);
                                                    match self.preview_cache.get(url) {
                                                        Some(PreviewState::Loading) | None => {
                                                            ui.label(egui::RichText::new("Loading preview…").color(text_muted).italics().small());
                                                        }
                                                        Some(PreviewState::Failed) => {
                                                            ui.label(egui::RichText::new("No preview available").color(text_muted).small());
                                                        }
                                                        Some(PreviewState::Loaded(preview)) => {
                                                            let title = preview.title.clone();
                                                            let desc = preview.description.clone();
                                                            let site = preview.site_name.clone();
                                                            let img_url = preview.image_url.clone();

                                                            let card = Frame::none()
                                                                .fill(card_bg)
                                                                .rounding(Rounding::same(6.0))
                                                                .stroke(Stroke::new(1.0, border_color))
                                                                .inner_margin(Margin { left: 14.0, right: 12.0, top: 8.0, bottom: 8.0 })
                                                                .show(ui, |ui| {
                                                                    ui.set_max_width(ui.available_width().min(520.0));
                                                                    if let Some(s) = &site {
                                                                        ui.label(egui::RichText::new(s).small().color(text_muted));
                                                                    }
                                                                    if let Some(t) = &title {
                                                                        ui.label(egui::RichText::new(t).strong());
                                                                    }
                                                                    if let Some(d) = &desc {
                                                                        let truncated: String = {
                                                                            let mut chars = d.chars();
                                                                            let s: String = chars.by_ref().take(240).collect();
                                                                            if chars.next().is_some() { s + "…" } else { s }
                                                                        };
                                                                        ui.label(egui::RichText::new(truncated).small().color(text_secondary));
                                                                    }
                                                                    if let Some(iu) = &img_url {
                                                                        if let Some(ImageState::Loaded(texture)) = self.image_cache.get(iu) {
                                                                            let orig = texture.size_vec2();
                                                                            let max_w = ui.available_width().min(460.0);
                                                                            let scale = if orig.x > max_w { max_w / orig.x } else { 1.0 };
                                                                            ui.add_space(4.0);
                                                                            ui.add(egui::Image::new((texture.id(), orig * scale)).rounding(4.0));
                                                                        }
                                                                    }
                                                                });
                                                            let bar = egui::Rect::from_min_max(
                                                                card.response.rect.min,
                                                                egui::pos2(card.response.rect.min.x + 3.0, card.response.rect.max.y),
                                                            );
                                                            ui.painter().rect_filled(bar, Rounding::same(3.0), accent_color);
                                                        }
                                                    }
                                                    ui.add_space(4.0);
                                                }
                                            }
                                        }); // end vertical (message column)
                                    }); // end horizontal (full message row)
                                }
                            }
                        });
                    });
                });
            } else {
                ui.centered_and_justified(|ui| { ui.label(egui::RichText::new("Select a buffer to start chatting").color(text_muted).size(16.0)); });
            }
        });

        if let Some((buf_id, count)) = pending_load_more {
            self.loading_more_buffer_id = Some(buf_id.clone());
            if let Some(client) = &self.client {
                client.send_api(
                    &format!("GET /api/buffers/{}/lines?lines=-{}", buf_id, count),
                    Some(&format!("_buffer_lines:{}", buf_id)),
                    None,
                );
            }
        }

        if self.client.is_some() { ctx.request_repaint(); }
    }
}
