use crate::relay::backend::{BackendClient, BackendEvent};
use crate::relay::weechat::{WeeChatClient, WeeChatConfig};
use crate::relay::models::*;
use crate::ui::ansi::ANSIParser;
use crate::ui::theme::AppTheme;
use crate::ui::url_safety::is_safe_public_url;
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
    if !is_safe_public_url(&url) {
        return Err("blocked: non-public URL".to_string());
    }
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
const IMAGE_CACHE_MAX: usize = 200;
const PREVIEW_CACHE_MAX: usize = 200;
const PREFIX_COL_WIDTHS_MAX: usize = 500;

/// Drop entries from `map` until its size is at most `cap`. We don't track
/// insertion order, so eviction picks an arbitrary key — acceptable for caches
/// where the goal is to prevent unbounded growth, not optimal hit rate.
fn cap_map<V>(map: &mut HashMap<String, V>, cap: usize) {
    while map.len() > cap {
        let victim = match map.keys().next() {
            Some(k) => k.clone(),
            None => break,
        };
        map.remove(&victim);
    }
}
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum BackendType {
    #[default]
    WeeChat,
    Soju,
}

/// Per-connection saved profile (serialised to AppSettings).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ConnectionProfile {
    pub label: String,
    pub backend_type: BackendType,
    pub host: String,
    pub port: String,
    pub nick: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub sasl_username: String,
    pub use_ssl: bool,
    pub accept_invalid_certs: bool,
    pub auto_connect: bool,
    #[serde(default)]
    pub save_password: bool,
}

impl ConnectionProfile {
    /// Stable, filesystem-safe prefix derived from label.
    pub fn prefix(&self) -> String {
        if self.label.is_empty() { return "conn".to_string(); }
        self.label.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .collect()
    }
    /// Keyring key for this profile's password.
    pub fn keyring_host_key(&self) -> String { format!("{}@{}", self.label, self.host) }
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        Self {
            label: String::new(),
            backend_type: BackendType::WeeChat,
            host: "localhost".to_string(),
            port: "9001".to_string(),
            nick: String::new(),
            username: String::new(),
            sasl_username: String::new(),
            use_ssl: true,
            accept_invalid_certs: false,
            auto_connect: false,
            save_password: false,
        }
    }
}

/// Per-connection runtime state (NOT serialised).
pub struct ConnectionHandle {
    pub prefix: String,
    pub label: String,
    pub backend_type: BackendType,
    pub client: Box<dyn BackendClient>,
    pub status: String,
    pub is_connecting: bool,
    pub connecting_pending: bool,
    pub auth_error: Option<String>,
    pub auto_reconnect: bool,
    pub connection_log: VecDeque<String>,
}

fn spawn_event_forwarder(
    prefix: String,
    mut from_rx: mpsc::UnboundedReceiver<BackendEvent>,
    to_tx: mpsc::UnboundedSender<(String, BackendEvent)>,
) {
    tokio::spawn(async move {
        while let Some(ev) = from_rx.recv().await {
            let _ = to_tx.send((prefix.clone(), ev));
        }
    });
}

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub backend_type: BackendType,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub irc_nick: String,
    #[serde(default)]
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
    #[serde(default)]
    pub emoji_rendering: bool,
    pub opacity: f32,
    #[serde(default)]
    pub show_hidden_buffers: bool,
    #[serde(default)]
    pub buffer_order: Vec<String>,
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
    #[serde(default = "default_true")]
    pub show_toolbar: bool,
    #[serde(default = "default_nicklist_width")]
    pub nicklist_width: f32,
    #[serde(default)]
    pub buffers_width: f32,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    /// Multi-connection profiles (new).
    #[serde(default)]
    pub connections: Vec<ConnectionProfile>,
    /// Max chars in prefix column (0 = auto/dynamic, matches weechat.look.prefix_align_max).
    #[serde(default)]
    pub prefix_align_max: usize,
    /// Separator between prefix column and message (matches weechat.look.prefix_suffix).
    #[serde(default = "default_prefix_suffix")]
    pub prefix_suffix: String,
}

fn default_true() -> bool { true }
fn default_nicklist_width() -> f32 { 180.0 }
fn default_prefix_suffix() -> String { "│".to_string() }

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
            emoji_rendering: false,
            opacity: 1.0,
            show_hidden_buffers: false,
            buffer_order: Vec::new(),
            cleared_buffer_ids: HashSet::new(),
            save_password: false,
            font_name: String::new(),
            font_path: String::new(),
            muted_buffer_names: HashSet::new(),
            show_toolbar: true,
            nicklist_width: 0.0,
            buffers_width: 0.0,
            accept_invalid_certs: false,
            backend_type: BackendType::WeeChat,
            irc_nick: String::new(),
            connections: Vec::new(),
            prefix_align_max: 0,
            prefix_suffix: "│".to_string(),
        }
    }
}

pub struct WeeChatApp {
    // Multi-connection state
    pub(crate) connections: Vec<ConnectionHandle>,
    pub(crate) profiles: Vec<ConnectionProfile>,
    pub(crate) shared_event_tx: mpsc::UnboundedSender<(String, BackendEvent)>,
    pub(crate) event_rx: mpsc::UnboundedReceiver<(String, BackendEvent)>,

    // Connection management UI state
    pub(crate) show_connection_log: bool,
    pub(crate) connection_log_unread: bool,
    pub(crate) selected_conn_log: Option<String>,
    pub(crate) editing_profile: ConnectionProfile,
    pub(crate) editing_password: String,
    pub(crate) editing_profile_idx: Option<usize>,
    pub(crate) show_connections: bool,      // connections manager window
    pub(crate) conn_show_add: bool,         // add/edit form inside connections window
    pub(crate) conn_connect_idx: Option<usize>, // index awaiting password before connect

    pub(crate) buffers: Vec<Buffer>,
    /// id → index into `buffers`. Maintained in lock-step via `rebuild_buffer_idx`
    /// called after every push/retain/extend/sort/clear of `buffers`.
    pub(crate) buffer_idx: HashMap<String, usize>,
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
    pub(crate) emoji_rendering: bool,
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
    pub(crate) show_toolbar: bool,
    pub(crate) nicklist_width: f32,
    pub(crate) buffers_width: f32,

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
    pub(crate) drag_drop_before_id: Option<String>,

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
    /// Per-buffer cooldown tracking for OS notifications. Prevents a noisy channel
    /// from spamming the OS notification center.
    pub(crate) last_notif_at: HashMap<String, std::time::Instant>,
    /// Set true when a highlight notification fires while the window isn't focused.
    /// `update()` consumes this and asks the OS to draw user attention to the app
    /// (Dock icon bounce on macOS, taskbar flash on Windows, urgency hint on Linux).
    pub(crate) request_attention: bool,
    /// Deferred notification subsystem init — must run after the OS run loop starts.
    notify_initialized: bool,

    // Set to the buffer ID while a "load more" history request is in flight.
    pub(crate) loading_more_buffer_id: Option<String>,

    // Transient search text inside the font-family dropdown.
    pub(crate) font_search: String,

    // Prefix column alignment (mirrors weechat.look.prefix_align_max / prefix_suffix).
    pub(crate) prefix_align_max: usize,
    pub(crate) prefix_suffix: String,
    // Per-buffer max prefix pixel width tracked across frames for stable column alignment.
    pub(crate) prefix_col_widths: HashMap<String, f32>,
}

pub(crate) struct CompletionState {
    pub(crate) original_word: String,
    pub(crate) matches: Vec<String>,
    pub(crate) index: usize,
    pub(crate) word_start_idx: usize,
}

impl WeeChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (shared_event_tx, event_rx) = mpsc::unbounded_channel::<(String, BackendEvent)>();
        let (image_tx, image_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();

        let settings: AppSettings = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            AppSettings::default()
        };

        // Build profiles list: use saved connections if present, else migrate legacy fields
        let mut profiles: Vec<ConnectionProfile> = settings.connections.clone();
        if profiles.is_empty() && !settings.host.is_empty() && settings.host != "localhost" {
            // Migrate from old single-connection settings
            profiles.push(ConnectionProfile {
                label: settings.host.clone(),
                backend_type: settings.backend_type.clone(),
                host: settings.host.clone(),
                port: settings.port.clone(),
                nick: settings.irc_nick.clone(),
                username: String::new(),
                sasl_username: String::new(),
                use_ssl: settings.use_ssl,
                accept_invalid_certs: settings.accept_invalid_certs,
                auto_connect: false,
                save_password: settings.save_password,
            });
        }

        if !settings.font_path.is_empty() {
            crate::ui::fonts::apply(&cc.egui_ctx, &settings.font_path);
        }
        let available_fonts = crate::ui::fonts::scan_system_fonts();

        Self {
            connections: Vec::new(),
            profiles,
            shared_event_tx,
            event_rx,
            show_connection_log: false,
            connection_log_unread: false,
            selected_conn_log: None,
            editing_profile: ConnectionProfile::default(),
            editing_password: String::new(),
            editing_profile_idx: None,
            show_connections: false,
            conn_show_add: false,
            conn_connect_idx: None,
            buffers: Vec::new(),
            buffer_idx: HashMap::new(),
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
            show_toolbar: settings.show_toolbar,
            nicklist_width: settings.nicklist_width,
            buffers_width: settings.buffers_width,
            auto_reconnect: settings.auto_reconnect,
            show_titlebar: settings.show_titlebar,
            show_server_headers: settings.show_server_headers,
            show_inline_images: settings.show_inline_images,
            show_link_previews: settings.show_link_previews,
            emoji_rendering: settings.emoji_rendering,
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
            last_notif_at: HashMap::new(),
            request_attention: false,
            notify_initialized: false,
            loading_more_buffer_id: None,
            font_search: String::new(),
            prefix_align_max: settings.prefix_align_max,
            prefix_suffix: settings.prefix_suffix,
            prefix_col_widths: HashMap::new(),
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

    pub(crate) fn is_twemoji_url(url: &str) -> bool {
        url.contains("cdnjs.cloudflare.com/ajax/libs/twemoji")
    }

    pub(crate) fn is_image_url(url: &str) -> bool {
        let path = url.split('?').next().unwrap_or(url).to_lowercase();
        matches!(
            std::path::Path::new(&path).extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
        )
    }

    pub(crate) fn is_any_connected(&self) -> bool {
        self.connections.iter().any(|c| c.client.is_connected())
    }

    /// Rebuild the `buffer_idx` map from `self.buffers`. Call after any
    /// push/retain/extend/sort/clear of `self.buffers`.
    pub(crate) fn rebuild_buffer_idx(&mut self) {
        self.buffer_idx.clear();
        self.buffer_idx.reserve(self.buffers.len());
        for (i, b) in self.buffers.iter().enumerate() {
            self.buffer_idx.insert(b.id.clone(), i);
        }
    }

    /// O(1) lookup of a buffer's index by id.
    pub(crate) fn buffer_idx_of(&self, id: &str) -> Option<usize> {
        self.buffer_idx.get(id).copied()
    }

    pub(crate) fn buffer_by_id(&self, id: &str) -> Option<&Buffer> {
        self.buffer_idx_of(id).map(|i| &self.buffers[i])
    }

    pub(crate) fn buffer_by_id_mut(&mut self, id: &str) -> Option<&mut Buffer> {
        self.buffer_idx_of(id).map(move |i| &mut self.buffers[i])
    }

    /// Returns (client_ref, raw_buffer_id) for the connection that owns the given full prefixed buffer_id.
    pub(crate) fn client_for_buffer<'a>(&'a self, buffer_id: &str) -> Option<(&'a dyn BackendClient, String)> {
        for conn in &self.connections {
            let p = format!("{}/", conn.prefix);
            if let Some(raw) = buffer_id.strip_prefix(&p) {
                return Some((&*conn.client, raw.to_string()));
            }
        }
        None
    }

    pub(crate) fn select_buffer(&mut self, id: String) {
        if let Some(prev_id) = self.selected_buffer_id.clone() {
            if prev_id != id {
                if let Some((client, raw_id)) = self.client_for_buffer(&prev_id) {
                    client.mark_read(&raw_id);
                }
            }
        }

        self.selected_buffer_id = Some(id.clone());
        self.selected_view_since = Some(std::time::Instant::now());
        self.cleared_buffer_ids.insert(id.clone());
        if let Some(buffer) = self.buffer_by_id_mut(&id) {
            buffer.activity = BufferActivity::None;
            buffer.unread_count = 0;
            buffer.visit_start_marker_id = buffer.last_read_id.clone();
            let fetch_nicks = buffer.has_nicklist;
            if let Some((client, raw_id)) = self.client_for_buffer(&id) {
                client.refresh_buffer(&raw_id);
                client.fetch_lines(&raw_id, INITIAL_LINES);
                if fetch_nicks {
                    client.fetch_nicks(&raw_id);
                }
                client.mark_read(&raw_id);
            }
        }
    }

    pub(crate) fn log_conn_for(&mut self, conn_prefix: &str, msg: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        let text = format!("[{}]  {}", ts, msg.into());
        if let Some(conn) = self.connections.iter_mut().find(|c| c.prefix == conn_prefix) {
            conn.connection_log.push_back(text.clone());
            if conn.connection_log.len() > 500 { conn.connection_log.pop_front(); }
        }
        if !self.show_connection_log {
            self.connection_log_unread = true;
        }
    }

    /// Start a connection for the given profile with the given password.
    pub(crate) fn do_connect(&mut self, profile: &ConnectionProfile, password: String, ctx: &egui::Context) {
        let prefix = profile.prefix();
        // Remove any stale handle with same prefix
        self.connections.retain(|c| c.prefix != prefix);

        let (per_conn_tx, per_conn_rx) = mpsc::unbounded_channel::<BackendEvent>();
        let port = profile.port.parse::<u16>().unwrap_or(9001);
        let (proto, path) = match profile.backend_type {
            BackendType::Soju    => (if profile.use_ssl { "ircs" } else { "irc" }, ""),
            BackendType::WeeChat => (if profile.use_ssl { "wss"  } else { "ws"  }, "/api"),
        };

        let mut client: Box<dyn BackendClient> = match profile.backend_type {
            BackendType::Soju => {
                let config = crate::relay::irc::IrcConfig {
                    host: profile.host.clone(),
                    port,
                    nick: if profile.nick.is_empty() { "user".to_string() } else { profile.nick.clone() },
                    username: profile.username.clone(),
                    sasl_username: profile.sasl_username.clone(),
                    password: password.clone(),
                    use_ssl: profile.use_ssl,
                    accept_invalid_certs: profile.accept_invalid_certs,
                };
                Box::new(crate::relay::irc::IrcClient::new(config, per_conn_tx.clone(), ctx.clone()))
            }
            BackendType::WeeChat => {
                let config = WeeChatConfig {
                    host: profile.host.clone(),
                    port,
                    password: password.clone(),
                    use_ssl: profile.use_ssl,
                    accept_invalid_certs: profile.accept_invalid_certs,
                };
                Box::new(WeeChatClient::new(config, per_conn_tx.clone(), ctx.clone()))
            }
        };
        client.connect();

        spawn_event_forwarder(prefix.clone(), per_conn_rx, self.shared_event_tx.clone());

        let mut conn = ConnectionHandle {
            prefix: prefix.clone(),
            label: profile.label.clone(),
            backend_type: profile.backend_type.clone(),
            client,
            status: "Connecting...".to_string(),
            is_connecting: true,
            connecting_pending: true,
            auth_error: None,
            auto_reconnect: self.auto_reconnect,
            connection_log: VecDeque::new(),
        };

        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        conn.connection_log.push_back(format!("[{}]  Connecting to {}://{}:{}{}", ts, proto, profile.host, port, path));
        let ts2 = chrono::Local::now().format("%H:%M:%S").to_string();
        conn.connection_log.push_back(format!("[{}]  SSL/TLS: {}", ts2, if profile.use_ssl { "enabled" } else { "disabled" }));

        if self.selected_conn_log.is_none() {
            self.selected_conn_log = Some(prefix.clone());
        }

        self.connections.push(conn);
        if !self.show_connection_log {
            self.connection_log_unread = true;
        }
    }
}

impl eframe::App for WeeChatApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = AppSettings {
            backend_type: BackendType::default(),
            host: String::new(),
            port: String::new(),
            use_ssl: false,
            show_filtered_lines: self.show_filtered_lines,
            colored_nicks: self.colored_nicks,
            theme: self.theme.clone(),
            font_size: self.font_size,
            use_monospace: self.use_monospace,
            show_timestamps: self.show_timestamps,
            show_buffers: self.show_buffers,
            show_nicklist: self.show_nicklist,
            show_toolbar: self.show_toolbar,
            nicklist_width: self.nicklist_width,
            buffers_width: self.buffers_width,
            auto_reconnect: self.auto_reconnect,
            show_titlebar: self.show_titlebar,
            show_server_headers: self.show_server_headers,
            show_inline_images: self.show_inline_images,
            show_link_previews: self.show_link_previews,
            emoji_rendering: self.emoji_rendering,
            opacity: self.opacity,
            show_hidden_buffers: self.show_hidden_buffers,
            buffer_order: self.buffer_order.clone(),
            cleared_buffer_ids: self.cleared_buffer_ids.clone(),
            save_password: false,
            font_name: self.font_name.clone(),
            font_path: self.font_path.clone(),
            muted_buffer_names: self.muted_buffer_names.clone(),
            accept_invalid_certs: false,
            irc_nick: String::new(),
            connections: self.profiles.clone(),
            prefix_align_max: self.prefix_align_max,
            prefix_suffix: self.prefix_suffix.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.notify_initialized {
            self.notify_initialized = true;
            crate::ui::notify::init();
        }

        while let Ok((prefix, event)) = self.event_rx.try_recv() {
            self.handle_event(&prefix, event);
        }

        if self.request_attention {
            self.request_attention = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }

        // Periodically prune prefix_col_widths so it can't grow indefinitely as
        // transient buffers come and go. Cheap when below cap.
        if self.prefix_col_widths.len() > PREFIX_COL_WIDTHS_MAX {
            self.prefix_col_widths.retain(|id, _| self.buffer_idx.contains_key(id));
            // If still over cap (huge connected session), drop arbitrary entries.
            cap_map(&mut self.prefix_col_widths, PREFIX_COL_WIDTHS_MAX);
        }

        // Same for the notification cooldown map — drop entries older than 5 minutes.
        if self.last_notif_at.len() > 200 {
            let now = std::time::Instant::now();
            self.last_notif_at.retain(|_, t| now.duration_since(*t) < std::time::Duration::from_secs(300));
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
            cap_map(&mut self.image_cache, IMAGE_CACHE_MAX);
        }

        while let Ok((url, result)) = self.preview_rx.try_recv() {
            match result {
                Ok(preview) => {
                    if let Some(img_url) = &preview.image_url {
                        if is_safe_public_url(img_url) && !self.image_cache.contains_key(img_url) {
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
            cap_map(&mut self.preview_cache, PREVIEW_CACHE_MAX);
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
                if i.consume_key(i.modifiers, Key::T) { self.show_toolbar = !self.show_toolbar; }
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

        let accent_color = if self.theme.name == "Default" {
            Color32::from_rgb(100, 149, 237)
        } else {
            Color32::from(self.theme.ansi[4])
        };
        let base_bg = self.theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18));
        let alpha = (self.opacity * 255.0) as u8;
        let bg_color = Color32::from_rgba_unmultiplied(base_bg.r(), base_bg.g(), base_bg.b(), alpha);

        let luma = 0.299 * base_bg.r() as f32 + 0.587 * base_bg.g() as f32 + 0.114 * base_bg.b() as f32;
        let is_light = luma > 140.0;

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

        let text_primary = self.theme.foreground
            .map(Color32::from)
            .unwrap_or_else(|| if is_light { Color32::from_gray(15) } else { Color32::WHITE });
        let text_secondary = if is_light { Color32::from_gray(70)  } else { Color32::from_gray(160) };
        let text_muted     = if is_light { Color32::from_gray(120) } else { Color32::from_gray(100) };

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
        let mut pending_mute: Option<(String, String, bool)> = None;
        let mut pending_load_more: Option<(String, usize)> = None;

        if self.show_toolbar { egui::TopBottomPanel::top("top_panel")
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
                        let buf_has_nicklist = self.selected_buffer_id.as_ref()
                            .and_then(|id| self.buffer_by_id(id))
                            .map(|b| b.has_nicklist)
                            .unwrap_or(false);
                        let (rect, res) = ui.allocate_at_least(icon_size, if buf_has_nicklist { egui::Sense::click() } else { egui::Sense::hover() });
                        if buf_has_nicklist && res.clicked() { self.show_nicklist = !self.show_nicklist; }
                        let color = if buf_has_nicklist && self.show_nicklist { accent_color } else { Color32::GRAY.linear_multiply(if buf_has_nicklist { 1.0 } else { 0.4 }) };
                        Self::draw_sidebar_icon(ui.painter(), rect, color, true);
                        ui.add_space(8.0);

                        if ui.button(egui::RichText::new("⚙").size(16.0)).on_hover_text("Settings").clicked() {
                            self.show_settings = !self.show_settings;
                            self.show_connections = false;
                        }
                        if ui.button(egui::RichText::new("🔌").size(14.0)).on_hover_text("Connections").clicked() {
                            self.show_connections = !self.show_connections;
                            self.show_settings = false;
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
                            self.show_connections = false;
                        }
                        // Show status indicators for each active connection
                        for conn in &self.connections {
                            if conn.client.is_connected() || conn.connecting_pending {
                                let status_text = if conn.connecting_pending {
                                    format!("● {}", conn.label)
                                } else {
                                    format!("● {}", conn.label)
                                };
                                let status_color = if conn.connecting_pending {
                                    Color32::from_rgb(255, 165, 0)
                                } else {
                                    Color32::from_rgb(50, 205, 50)
                                };
                                ui.label(egui::RichText::new(status_text).color(status_color).small());
                            }
                        }
                    });
                });
            });
        }

        if self.show_buffers {
            if self.buffers_width == 0.0 {
                let buf_font_id = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });
                let char_w = ctx.fonts(|f| f.glyph_width(&buf_font_id, 'W'));
                self.buffers_width = char_w * 20.0 + 20.0;
            }
            let buffers_max_w = (ctx.screen_rect().width() * 0.40).max(80.0);
            let buffers_resp = egui::SidePanel::left("buffers_panel")
                .resizable(true)
                .default_width(self.buffers_width)
                .min_width(80.0)
                .max_width(buffers_max_w)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("BUFFERS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);

                    let is_dragging = self.dragging_buffer_id.is_some();
                    let pointer_pos = ctx.pointer_hover_pos();

                    let dragged_group_ids: HashSet<String> = if let Some(drag_id) = &self.dragging_buffer_id {
                        if let Some(drag_buf) = self.buffer_by_id(drag_id) {
                            if drag_buf.kind == "server" || drag_buf.kind == "core" {
                                let skey = drag_buf.server.clone();
                                self.buffers.iter().filter(|b| b.server == skey).map(|b| b.id.clone()).collect()
                            } else {
                                std::iter::once(drag_id.clone()).collect()
                            }
                        } else { HashSet::new() }
                    } else { HashSet::new() };

                    let mut row_rects: Vec<(egui::Rect, String)> = Vec::new();

                    // Build a lookup from connection prefix → friendly label for headers
                    let conn_label_map: std::collections::HashMap<String, String> = self.connections.iter()
                        .map(|c| (c.prefix.clone(), c.label.clone()))
                        .collect();
                    let multi_conn = conn_label_map.len() > 1;

                    ScrollArea::vertical().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut last_conn_prefix: Option<String> = None;
                        for buffer in &self.buffers {
                            if buffer.hidden && !self.show_hidden_buffers {
                                continue;
                            }
                            let is_selected = self.selected_buffer_id.as_deref() == Some(&buffer.id);
                            let is_root = buffer.kind == "server" || buffer.kind == "core";
                            let is_child = buffer.kind == "channel" || buffer.kind == "private";
                            let in_dragged_group = dragged_group_ids.contains(&buffer.id);

                            // Connection header — only shown when ≥2 connections are active
                            if multi_conn {
                                let buf_conn_prefix = buffer.id.split('/').next().unwrap_or("").to_string();
                                if last_conn_prefix.as_deref() != Some(&buf_conn_prefix) {
                                    if last_conn_prefix.is_some() { ui.add_space(6.0); }
                                    let header_label = conn_label_map.get(&buf_conn_prefix)
                                        .cloned()
                                        .unwrap_or_else(|| buf_conn_prefix.clone());
                                    ui.label(
                                        egui::RichText::new(header_label.to_uppercase())
                                            .strong()
                                            .color(accent_color)
                                            .size(10.0)
                                    );
                                    ui.add_space(2.0);
                                    last_conn_prefix = Some(buf_conn_prefix);
                                }
                            }

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
                                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                                    ui.add(Label::new(label).truncate(true));
                                                });
                                            });
                                        } else {
                                            ui.add(Label::new(label).truncate(true));
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

                            if !in_dragged_group {
                                row_rects.push((outer_resp.rect, buffer.id.clone()));
                            }
                        }
                    });

                    if let Some(id) = next_drag_buffer_id.take() {
                        self.dragging_buffer_id = Some(id);
                    }

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
                                ui.painter().hline(
                                    first.left()..=first.right(),
                                    indicator_y,
                                    Stroke::new(2.0, accent_color),
                                );
                            }
                        }
                    }

                    if is_dragging && ctx.input(|i| !i.pointer.primary_down()) {
                        if let Some(drag_id) = self.dragging_buffer_id.take() {
                            let drop_id = self.drag_drop_before_id.take();
                            apply_drag_reorder(&mut self.buffers, &drag_id, drop_id.as_deref());
                            self.buffer_order = self.buffers.iter().map(|b| b.id.clone()).collect();
                        }
                        self.drag_drop_before_id = None;
                    }
                });
            let w = buffers_resp.response.rect.width();
            if w >= 80.0 { self.buffers_width = w; }
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
            if let Some(b) = self.buffer_by_id_mut(&buf_id) {
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
        let current_buf = current_buffer_id.as_ref().and_then(|id| self.buffer_by_id(id));
        let current_buffer_nicks = current_buf.map(|b| b.nicks.clone());
        let current_buffer_full_name = current_buf.map(|b| b.full_name.clone());
        let current_buffer_messages = current_buf.map(|b| b.messages.clone());
        let _current_buffer_last_read_id = current_buf.and_then(|b| b.last_read_id.clone());
        let current_buffer_visit_marker_id = current_buf.and_then(|b| b.visit_start_marker_id.clone());
        let current_buffer_topic = current_buf.map(|b| b.topic.clone()).unwrap_or_default();
        let current_buffer_modes = current_buf.map(|b| b.modes.clone()).unwrap_or_default();
        let current_buffer_kind = current_buf.map(|b| b.kind.clone()).unwrap_or_default();

        let font_id = FontId::new(self.font_size, if self.use_monospace { FontFamily::Monospace } else { FontFamily::Proportional });

        let any_connected = self.is_any_connected();

        let current_buf_has_nicklist = current_buf.map(|b| b.has_nicklist).unwrap_or(false);
        if self.show_nicklist && current_buf_has_nicklist && any_connected && current_buffer_id.is_some() {
            if self.nicklist_width < 80.0 {
                self.nicklist_width = 180.0;
            }
            let nicks_max_w = (ctx.screen_rect().width() * 0.30).max(80.0);
            let nicks_resp = egui::SidePanel::right("nicks_panel_2")
                .resizable(true)
                .default_width(self.nicklist_width)
                .min_width(80.0)
                .max_width(nicks_max_w)
                .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(10.0)))
                .show(ctx, |ui| {
                    // Force content to fill the full panel width so that egui's
                    // PanelState stores the actual panel width (not just content
                    // min_rect), preventing the panel from snapping back to
                    // content width after every resize.
                    ui.set_min_width(ui.available_width());
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("NICKS").strong().color(accent_color).size(11.0));
                    ui.add_space(8.0);
                    ScrollArea::vertical().show(ui, |ui| {
                            if let Some(nicks) = &current_buffer_nicks {
                                for nick in nicks {
                                    let text = format!("{}{}", nick.prefix, nick.name);
                                    let input = if nick.away {
                                        text.clone()
                                    } else if self.colored_nicks {
                                        if self.theme.name == "Default" { format!("{}{}", nick.color_ansi, text) }
                                        else {
                                            let idx = Self::hash_nick(&nick.name);
                                            let esc = if idx < 8 { format!("\x1B[{}m", 30 + idx) } else { format!("\x1B[{}m", 90 + idx - 8) };
                                            format!("{}{}", esc, text)
                                        }
                                    } else { text };
                                    let sections = ANSIParser::parse(&input);
                                    let mut job = LayoutJob::default();
                                    for s in sections {
                                        let mut fmt = s.style.to_format(font_id.clone(), &self.theme);
                                        if nick.away {
                                            fmt.color = text_muted;
                                            fmt.italics = true;
                                        }
                                        job.append(&s.text, 0.0, fmt);
                                    }

                                    let label_res = ui.add(Label::new(job).truncate(true).sense(egui::Sense::click()));
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
            self.nicklist_width = nicks_resp.response.rect.width();
        }

        if current_buffer_id.is_some() {
            // Is the SELECTED buffer's connection up? Different from any_connected when
            // multiple connections are configured and only some are alive.
            let selected_buffer_connected = current_buffer_id.as_ref()
                .and_then(|id| id.split('/').next())
                .and_then(|prefix| self.connections.iter().find(|c| c.prefix == prefix))
                .map(|c| c.client.is_connected())
                .unwrap_or(false);

            egui::TopBottomPanel::bottom("input_panel")
                .frame(Frame::none().fill(surface_color).inner_margin(Margin::symmetric(16.0, 10.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let hint = if !selected_buffer_connected {
                            "Disconnected — reconnect before sending"
                        } else if current_buffer_kind == "server" {
                            "Type /join #channel or any IRC command..."
                        } else {
                            "Type a message..."
                        };
                        ui.add_enabled_ui(selected_buffer_connected, |ui| {
                            let text_edit = egui::TextEdit::singleline(&mut self.input_text)
                                .hint_text(hint)
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
                });
        }

        // Collect pending connect/disconnect actions from the connection panel

        egui::CentralPanel::default()
            .frame(Frame::none().fill(bg_color).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
            if self.show_settings {
                self.show_settings_window(ui, accent_color, is_light);
            } else if self.show_connections {
                self.show_connections_window(ui, accent_color, is_light);
            } else if self.show_connection_log {
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.show_connection_log = false;
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    let log_w = (ui.available_width() - 80.0).min(720.0);
                    ui.allocate_ui(egui::vec2(log_w, ui.available_height() - 32.0), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Connection Log").strong().size(14.0));
                            // Connection selector tabs
                            for conn in &self.connections {
                                let selected = self.selected_conn_log.as_deref() == Some(&conn.prefix);
                                let dot_color = if conn.client.is_connected() {
                                    Color32::from_rgb(50, 205, 50)
                                } else if conn.connecting_pending {
                                    Color32::from_rgb(255, 165, 0)
                                } else {
                                    Color32::from_rgb(180, 60, 60)
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("●").color(dot_color).size(10.0));
                                    if ui.selectable_label(selected, &conn.label).clicked() {
                                        self.selected_conn_log = Some(conn.prefix.clone());
                                    }
                                });
                            }
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
                                        // Find selected connection log
                                        let selected_prefix = self.selected_conn_log.clone();
                                        let log_entries: Vec<String> = self.connections.iter()
                                            .find(|c| selected_prefix.as_deref() == Some(&c.prefix))
                                            .map(|c| c.connection_log.iter().cloned().collect())
                                            .unwrap_or_default();
                                        let is_pending = self.connections.iter()
                                            .find(|c| selected_prefix.as_deref() == Some(&c.prefix))
                                            .map(|c| c.connecting_pending)
                                            .unwrap_or(false);
                                        if log_entries.is_empty() {
                                            ui.label(egui::RichText::new("No connection activity yet.").color(text_muted).italics());
                                        }
                                        for entry in &log_entries {
                                            let color = if entry.contains("Error") || entry.contains("failed") || entry.contains("Disconnected") {
                                                Color32::from_rgb(220, 80, 80)
                                            } else if entry.contains("Connected") {
                                                Color32::from_rgb(50, 205, 50)
                                            } else {
                                                text_secondary
                                            };
                                            ui.label(egui::RichText::new(entry).font(log_font.clone()).color(color));
                                        }
                                        if is_pending {
                                            ui.spinner();
                                        }
                                    });
                            });
                    });
                });
            } else if self.profiles.is_empty() && !any_connected {
                // No profiles yet — show a minimal landing page that opens the connections window
                ui.vertical_centered(|ui| {
                    ui.add_space(ctx.available_rect().height() * 0.2);
                    Frame::group(ui.style())
                        .fill(surface_color)
                        .rounding(Rounding::same(12.0))
                        .stroke(Stroke::new(1.0, border_color))
                        .inner_margin(Margin::same(40.0))
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            ui.heading(egui::RichText::new("No connections configured").strong().size(20.0));
                            ui.add_space(12.0);
                            ui.label(egui::RichText::new("Add a connection to get started.").color(text_secondary));
                            ui.add_space(20.0);
                            let btn = egui::Button::new(egui::RichText::new("+ Add Connection").strong().color(Color32::WHITE))
                                .fill(accent_color)
                                .min_size(Vec2::new(160.0, 40.0));
                            if ui.add(btn).clicked() {
                                self.show_connections = true;
                                self.conn_show_add = true;
                                self.editing_profile = ConnectionProfile::default();
                                self.editing_password.clear();
                                self.editing_profile_idx = None;
                            }
                        });
                });
            } else if let Some(_full_name) = current_buffer_full_name {
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width());
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
                                        let sections = ANSIParser::parse(&current_buffer_topic);
                                        let mut job = LayoutJob::default();
                                        for s in sections { job.append(&s.text, 0.0, s.style.to_format(topic_font.clone(), &self.theme)); }
                                        ui.add(Label::new(job).wrap(true));
                                    }
                                });
                            });
                        ui.add_space(-1.0);
                        ui.separator();
                    }

                    let msg_area_width = ui.available_width();
                    ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_min_width(msg_area_width);
                        ui.set_max_width(msg_area_width);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        Frame::none().inner_margin(Margin::same(16.0)).show(ui, |ui| {
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
                                        if !line.plain_prefix_lower.contains(q) && !line.plain_message_lower.contains(q) { continue; }
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
                                            let remaining = std::time::Duration::from_secs(2) - elapsed;
                                            ctx.request_repaint_after(remaining);
                                            ui.horizontal(|ui| {
                                                ui.add_space(20.0);
                                                ui.separator();
                                                ui.label(egui::RichText::new(" NEW MESSAGES ").color(divider_color).size(10.0).strong());
                                                ui.separator();
                                            });
                                        } else {
                                            ui.scope(|ui| {
                                                ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, divider_color);
                                                ui.separator();
                                            });
                                        }
                                        ui.add_space(8.0);
                                        marker_shown = true;
                                    }

                                    let msg_sections = &line.parsed_message;

                                    let image_urls_in_line: Vec<String> = if self.show_inline_images {
                                        msg_sections.iter()
                                            .filter_map(|s| s.url.as_ref())
                                            .filter(|u| Self::is_image_url(u) && !Self::is_twemoji_url(u))
                                            .cloned()
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };
                                    let preview_urls_in_line: Vec<String> = if self.show_link_previews {
                                        msg_sections.iter()
                                            .filter_map(|s| s.url.as_ref())
                                            .filter(|u| !Self::is_image_url(u) && !Self::is_twemoji_url(u))
                                            .cloned()
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };

                                    let row_bg = if line.highlight {
                                        let c = Color32::from(self.theme.ansi[3]);
                                        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 28)
                                    } else {
                                        Color32::TRANSPARENT
                                    };
                                    let row_resp = Frame::none()
                                        .fill(row_bg)
                                        .rounding(Rounding::same(3.0))
                                        .show(ui, |ui| {
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        if self.show_timestamps {
                                            ui.label(egui::RichText::new(line.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).font(font_id.clone()).color(text_muted));
                                        }
                                        let prefix_sections = &line.parsed_prefix;

                                        // Measure plain-text width for stable column tracking.
                                        let measured_w = ui.fonts(|f| {
                                            f.layout_no_wrap(line.plain_prefix.clone(), font_id.clone(), Color32::WHITE).size().x
                                        });
                                        let cap_px = if self.prefix_align_max > 0 {
                                            ui.fonts(|f| {
                                                f.layout_no_wrap("M".repeat(self.prefix_align_max), font_id.clone(), Color32::WHITE).size().x
                                            })
                                        } else {
                                            f32::INFINITY
                                        };
                                        let entry = self.prefix_col_widths.entry(current_buffer_id.clone().unwrap_or_default()).or_insert(0.0);
                                        *entry = entry.max(measured_w).min(cap_px);
                                        let col_width = *entry;

                                        ui.allocate_ui_with_layout(
                                            egui::vec2(col_width, ui.text_style_height(&TextStyle::Body)),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let mut prefix_job = LayoutJob::default();
                                                prefix_job.halign = egui::Align::RIGHT;
                                                for s in prefix_sections { prefix_job.append(&s.text, 0.0, s.style.to_format(font_id.clone(), &self.theme)); }
                                                ui.add(Label::new(prefix_job).wrap(false));
                                            }
                                        );
                                        if !self.prefix_suffix.is_empty() {
                                            ui.label(egui::RichText::new(&self.prefix_suffix).font(font_id.clone()).color(text_muted));
                                        }

                                        let msg_col_width = ui.available_width();
                                        ui.vertical(|ui| {
                                            ui.set_min_width(msg_col_width);
                                            ui.horizontal_wrapped(|ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                for s in msg_sections {
                                                    if let Some(url) = &s.url {
                                                        if ui.link(egui::RichText::new(&s.text).font(font_id.clone())).clicked() {
                                                            ui.ctx().output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url.clone())));
                                                        }
                                                        if self.show_inline_images && Self::is_image_url(url) && is_safe_public_url(url) {
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
                                                        if self.show_link_previews && !Self::is_image_url(url) && is_safe_public_url(url) {
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
                                                    } else if !self.emoji_rendering {
                                                        let mut job = LayoutJob::default();
                                                        job.append(&s.text, 0.0, s.style.to_format(font_id.clone(), &self.theme));
                                                        ui.add(Label::new(job).wrap(true));
                                                    } else {
                                                        let emoji_size = font_id.size + 2.0;
                                                        for span in crate::ui::emoji::split_emoji(&s.text) {
                                                            match span {
                                                                crate::ui::emoji::TextSpan::Text(t) => {
                                                                    let mut job = LayoutJob::default();
                                                                    job.append(&t, 0.0, s.style.to_format(font_id.clone(), &self.theme));
                                                                    ui.add(Label::new(job).wrap(true));
                                                                }
                                                                crate::ui::emoji::TextSpan::Emoji(e) => {
                                                                    let eurl = crate::ui::emoji::emoji_to_twemoji_url(&e);
                                                                    if !self.image_cache.contains_key(&eurl) {
                                                                        self.image_cache.insert(eurl.clone(), ImageState::Loading);
                                                                        let tx = self.image_tx.clone();
                                                                        let url_owned = eurl.clone();
                                                                        tokio::spawn(async move {
                                                                            let result: Result<Vec<u8>, String> = async {
                                                                                let bytes = reqwest::get(&url_owned).await
                                                                                    .map_err(|e| e.to_string())?
                                                                                    .bytes().await
                                                                                    .map_err(|e| e.to_string())?;
                                                                                Ok(bytes.to_vec())
                                                                            }.await;
                                                                            let _ = tx.send((url_owned, result));
                                                                        });
                                                                    }
                                                                    match self.image_cache.get(&eurl) {
                                                                        Some(ImageState::Loaded(texture)) => {
                                                                            ui.add(egui::Image::new((texture.id(), egui::Vec2::splat(emoji_size))));
                                                                        }
                                                                        _ => {
                                                                            let mut job = LayoutJob::default();
                                                                            job.append(&e, 0.0, s.style.to_format(font_id.clone(), &self.theme));
                                                                            ui.add(Label::new(job).wrap(true));
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            });

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
                                    }); // end highlight frame
                                    let interactable = ui.interact(
                                        row_resp.response.rect,
                                        egui::Id::new("msg_row").with(&line.id),
                                        egui::Sense::click(),
                                    );
                                    let plain_message = line.plain_message.clone();
                                    let plain_prefix = line.plain_prefix.clone();
                                    interactable.context_menu(|ui| {
                                        if ui.button("Copy message").clicked() {
                                            ui.ctx().output_mut(|o| o.copied_text = plain_message.clone());
                                            ui.close_menu();
                                        }
                                        if !plain_prefix.is_empty() {
                                            if ui.button("Copy with sender").clicked() {
                                                ui.ctx().output_mut(|o| o.copied_text = format!("<{}> {}", plain_prefix, plain_message));
                                                ui.close_menu();
                                            }
                                        }
                                    });
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
            if let Some((client, raw_id)) = self.client_for_buffer(&buf_id) {
                let oldest_ts = self.buffers.iter()
                    .find(|b| b.id == buf_id)
                    .and_then(|b| b.messages.front())
                    .map(|l| l.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
                if let Some(ts) = oldest_ts {
                    client.fetch_lines_before(&raw_id, &ts);
                }
                client.fetch_lines(&raw_id, count);
            }
        }

        if any_connected {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
