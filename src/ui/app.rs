use crate::relay::backend::{BackendClient, BackendEvent};
use crate::relay::weechat::{WeeChatClient, WeeChatConfig};
use crate::relay::models::*;
use crate::ui::ansi::ANSIParser;
use crate::ui::theme::AppTheme;
use crate::ui::keybinds::KeybindsMap;
use crate::ui::icons;
use crate::ui::url_safety::is_safe_public_url;
use egui::{FontId, ScrollArea, Label, Key, Visuals, TextStyle, FontFamily, Color32, text::LayoutJob, Margin, Frame, CornerRadius, Stroke, Vec2, Modifiers, Rect, Painter};
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

/// Largest inline image we download.
const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;

async fn fetch_image_bytes(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        // Many hosts (e.g. Wikimedia) answer 403 to requests without a User-Agent.
        .user_agent(concat!("Mozilla/5.0 (compatible; WeeChatRS/", env!("CARGO_PKG_VERSION"), ")"))
        .build()
        .map_err(|e| e.to_string())?;
    // Fail on 4xx/5xx instead of trying to decode the error page as an image.
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    if resp.content_length().is_some_and(|n| n as usize > MAX_IMAGE_BYTES) {
        return Err("image too large".to_string());
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image too large".to_string());
    }
    Ok(bytes.to_vec())
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

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    let ct = resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let host = || url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from));
    // A link with no filename hint that turns out to be an image (e.g. a file-share
    // URL): preview it as the image itself instead of failing.
    if ct.starts_with("image/") {
        return Ok(LinkPreview { title: None, description: None, image_url: Some(url.clone()), site_name: host() });
    }
    if !ct.contains("text/html") {
        return Err("not html".to_string());
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]);

    let title = extract_og_tag(&html, "og:title").or_else(|| extract_html_title(&html));
    let description = extract_og_tag(&html, "og:description")
        .or_else(|| extract_meta_description(&html));
    let image_url = extract_og_tag(&html, "og:image");
    let site_name = extract_og_tag(&html, "og:site_name").or_else(host);

    if title.is_none() && description.is_none() {
        return Err("no preview data".to_string());
    }

    Ok(LinkPreview { title, description, image_url, site_name })
}

/// Longest gap between two messages from the same nick that still groups them.
const GROUP_MAX_GAP_SECS: i64 = 5 * 60;

/// The highmon script's buffer (`{connection}/perl.highmon`, `…/python.highmon`).
fn is_highmon_buffer(full_name: &str) -> bool {
    let name = full_name.rsplit_once('/').map_or(full_name, |(_, name)| name);
    matches!(name.split_once('.'), Some((_, "highmon")))
}

/// Whether a line's prefix is a nick (a chat message) rather than a WeeChat system
/// prefix such as `-->`, `<--`, `--`, `=!=`, or the ` *` of a `/me` action.
fn is_chat_prefix(plain_prefix: &str) -> bool {
    plain_prefix.chars().any(char::is_alphanumeric)
}

/// The style each nick's name has in the chat, from the last line it sent, so the nick
/// list can match it: WeeChat colours nicks in line prefixes with its own scheme.
fn nick_styles_in_chat<'a>(lines: impl Iterator<Item = &'a Line>) -> HashMap<String, crate::ui::ansi::AnsiStyle> {
    let mut styles = HashMap::new();
    for line in lines {
        if !is_chat_prefix(&line.plain_prefix) {
            continue;
        }
        // The nick is the last part with a letter or digit; mode characters (@, +) and
        // weechat.look.nick_prefix/suffix usually come in parts of their own colour.
        let Some(part) = line.parsed_prefix.iter().rev().find(|s| s.text.chars().any(char::is_alphanumeric)) else {
            continue;
        };
        let name = part.text.trim()
            .trim_start_matches(|c: char| "~&@%+!<(".contains(c))
            .trim_end_matches(|c: char| ">):".contains(c));
        if !name.is_empty() {
            styles.insert(name.to_string(), part.style);
        }
    }
    styles
}

/// [`nick_styles_in_chat`] for the selected buffer, rebuilt when its lines change.
pub(crate) struct NickStyleCache {
    buffer_id: String,
    len: usize,
    last_id: Option<String>,
    styles: std::sync::Arc<HashMap<String, crate::ui::ansi::AnsiStyle>>,
}

/// Whether `line` continues a group started by `prev`, the previous line shown: both are
/// chat messages from the same nick, on the same local day, at most
/// [`GROUP_MAX_GAP_SECS`] apart.
fn continues_group(prev: Option<&Line>, line: &Line) -> bool {
    let Some(prev) = prev else { return false };
    let gap = (line.timestamp - prev.timestamp).num_seconds();
    is_chat_prefix(&line.plain_prefix)
        && line.plain_prefix == prev.plain_prefix
        && (0..=GROUP_MAX_GAP_SECS).contains(&gap)
        && line.timestamp.with_timezone(&chrono::Local).date_naive()
            == prev.timestamp.with_timezone(&chrono::Local).date_naive()
}

pub const INITIAL_LINES: usize = 300;
const IMAGE_CACHE_MAX: usize = 200;
const PREVIEW_CACHE_MAX: usize = 200;
const PREFIX_COL_WIDTHS_MAX: usize = 500;

/// Drop entries from `map` until its size is at most `cap`. We don't track
/// insertion order, so eviction picks an arbitrary key — acceptable for caches
/// where the goal is to prevent unbounded growth, not optimal hit rate.
/// Height of the custom macOS titlebar strip (matches the native traffic-light area).
#[cfg(target_os = "macos")]
pub(crate) const MACOS_TITLEBAR_HEIGHT: f32 = 28.0;

/// Opaque titlebar strip for a macOS window with a full-size content view and hidden
/// native titlebar: draws `title` centred, drags the window, double-click zooms. The native
/// traffic lights sit on top of it. Applies to whichever viewport `ui` belongs to.
#[cfg(target_os = "macos")]
pub(crate) fn macos_titlebar(ui: &mut egui::Ui, title: &str, fill: Color32, border: Color32, text_color: Color32) {
    egui::Panel::top("window_titlebar")
        .exact_size(MACOS_TITLEBAR_HEIGHT)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(fill).stroke(Stroke::new(1.0, border)))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let res = ui.interact(rect, ui.scope_id().with("titlebar_drag"), egui::Sense::click_and_drag());
            if res.double_clicked() {
                let ctx = ui.ctx();
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            } else if res.drag_started_by(egui::PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, title, FontId::proportional(13.0), text_color);
        });
}

fn cap_map<V>(map: &mut HashMap<String, V>, cap: usize) {
    while map.len() > cap {
        let victim = match map.keys().next() {
            Some(k) => k.clone(),
            None => break,
        };
        map.remove(&victim);
    }
}
pub const LOAD_MORE_LINES: usize = 300;
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
    #[serde(default)]
    pub channel: String,
    // SSH tunnel
    #[serde(default)]
    pub ssh_enabled: bool,
    #[serde(default)]
    pub ssh_host: String,
    #[serde(default)]
    pub ssh_port: Option<u16>,
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub ssh_save_password: bool,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
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
    /// Keyring key for this profile's relay password.
    pub fn keyring_host_key(&self) -> String { format!("{}@{}", self.label, self.host) }
    /// Keyring key for this profile's SSH tunnel password.
    pub fn ssh_keyring_key(&self) -> String { format!("ssh:{}@{}", self.label, self.ssh_host) }
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
            channel: String::new(),
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: None,
            ssh_user: String::new(),
            ssh_save_password: false,
            auto_reconnect: true,
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
    #[allow(dead_code)] // held for Drop — kills the ssh process on disconnect
    pub ssh_tunnel: Option<crate::ui::ssh_tunnel::SshTunnel>,
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
    /// Use the chat font for the whole UI, not just chat content. Renamed from
    /// `use_monospace` (whose default was on) so older settings start with it off.
    #[serde(default)]
    pub monospace_ui: bool,
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
    /// macOS: native blur (NSVisualEffectView) behind the translucent window.
    #[serde(default = "default_true")]
    pub blur_background: bool,
    #[serde(default)]
    pub show_hidden_buffers: bool,
    /// Show the nick only once for consecutive messages from the same nick.
    #[serde(default = "default_true")]
    pub group_messages: bool,
    /// Draw 256-colour and RGB colours (WeeChat nick colours, mostly) in the nearest of
    /// the theme's 16 colours. Not applied to the Default theme.
    #[serde(default = "default_true")]
    pub fit_colors_to_theme: bool,
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
    #[serde(default)]
    pub file_share_service: crate::ui::fileshare::FileShareService,
    /// Duration passed to Jirafeau upload API (default "day"). Ignored for 0x0.st.
    #[serde(default = "default_file_share_duration")]
    pub file_share_duration: String,
    /// Jirafeau server ("selfhost"). Upload/download passwords live in the Keychain.
    #[serde(default = "default_jirafeau_url")]
    pub jirafeau_url: String,
    #[serde(default)]
    pub keybinds: KeybindsMap,
    /// Server group keys (buffer.server) whose child buffers are collapsed in the list.
    #[serde(default)]
    pub collapsed_servers: HashSet<String>,
    /// Derive theme colours from the current desktop wallpaper.
    #[serde(default)]
    pub adaptive_theme: bool,
}

fn default_true() -> bool { true }
fn default_nicklist_width() -> f32 { 180.0 }
fn default_prefix_suffix() -> String { "│".to_string() }
fn default_file_share_duration() -> String { "day".to_string() }
fn default_jirafeau_url() -> String { crate::ui::fileshare::DEFAULT_JIRAFEAU_URL.to_string() }

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
            monospace_ui: false,
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
            blur_background: true,
            show_hidden_buffers: false,
            group_messages: true,
            fit_colors_to_theme: true,
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
            file_share_service: crate::ui::fileshare::FileShareService::default(),
            file_share_duration: "day".to_string(),
            jirafeau_url: default_jirafeau_url(),
            keybinds: KeybindsMap::default(),
            collapsed_servers: HashSet::new(),
            adaptive_theme: false,
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
    pub(crate) editing_ssh_password: String,
    pub(crate) editing_profile_idx: Option<usize>,
    pub(crate) show_connections: bool,      // connections manager window
    pub(crate) conn_show_add: bool,         // add/edit form inside connections window
    pub(crate) conn_connect_idx: Option<usize>, // index awaiting password before connect
    /// Per-session relay password cache — avoids re-prompting within a session.
    pub(crate) session_passwords: std::collections::HashMap<String, String>,
    /// Per-session SSH password cache.
    pub(crate) session_ssh_passwords: std::collections::HashMap<String, String>,

    pub(crate) buffers: Vec<Buffer>,
    /// id → index into `buffers`. Maintained in lock-step via `rebuild_buffer_idx`
    /// called after every push/retain/extend/sort/clear of `buffers`.
    pub(crate) buffer_idx: HashMap<String, usize>,
    pub(crate) selected_buffer_id: Option<String>,
    pub(crate) input_text: String,
    // Settings
    pub(crate) show_settings: bool,
    pub(crate) settings_section: crate::ui::settings::SettingsSection,
    /// Where the Settings / Connections windows opened; kept while open so they stay put.
    pub(crate) settings_window_pos: Option<egui::Pos2>,
    pub(crate) connections_window_pos: Option<egui::Pos2>,
    pub(crate) show_filtered_lines: bool,
    pub(crate) colored_nicks: bool,
    pub(crate) theme: AppTheme,
    pub(crate) font_size: f32,
    pub(crate) monospace_ui: bool,
    pub(crate) show_timestamps: bool,
    pub(crate) auto_reconnect: bool,
    pub(crate) show_titlebar: bool,
    pub(crate) show_server_headers: bool,
    pub(crate) show_inline_images: bool,
    pub(crate) show_link_previews: bool,
    pub(crate) emoji_rendering: bool,
    pub(crate) opacity: f32,
    pub(crate) blur_background: bool,
    /// Blur state last applied to the native window; `None` until the first frame.
    #[cfg(target_os = "macos")]
    applied_blur: Option<bool>,
    pub(crate) show_hidden_buffers: bool,
    pub(crate) group_messages: bool,
    pub(crate) fit_colors_to_theme: bool,

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
    /// Tab completion waiting on the server (WeeChat `/api/completion`).
    pub(crate) remote_completion: Option<RemoteCompletion>,
    pub(crate) remote_completion_seq: u64,
    /// Completion replies by "{connection prefix}|{tag}", taken by whoever asked.
    pub(crate) completion_replies: HashMap<String, crate::relay::models::CompletionReply>,

    // Command History
    pub(crate) command_history: VecDeque<String>,
    pub(crate) history_index: Option<usize>,
    pub(crate) focus_input: bool,

    // Search state
    pub(crate) show_search: bool,
    pub(crate) search_text: String,
    /// Results of a search on the server, shown in place of the chat.
    pub(crate) server_search: Option<crate::ui::server_search::ServerSearch>,
    /// Tags `search` and `fetch_context` replies.
    pub(crate) next_request_id: u64,

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
    // Transient search text inside the theme dropdown, and the last import failure.
    pub(crate) theme_search: String,
    pub(crate) theme_import_error: Option<String>,

    // Prefix column alignment (mirrors weechat.look.prefix_align_max / prefix_suffix).
    pub(crate) prefix_align_max: usize,
    pub(crate) prefix_suffix: String,
    // Per-buffer max prefix pixel width tracked across frames for stable column alignment.
    pub(crate) prefix_col_widths: HashMap<String, f32>,
    /// Id of the first line the chat view draws for the selected buffer (None: all).
    /// The view draws every line it shows each frame, and a buffer left alone for a
    /// long time collects up to MAX_STORED_LINES, so it starts at the newest ones.
    pub(crate) chat_view_start: Option<String>,
    pub(crate) nick_style_cache: Option<NickStyleCache>,

    // Context menu opened by secondary click on a message row.
    // Tuple: (click_pos, plain_message, plain_prefix, hovered_url)
    ctx_menu_state: Option<(egui::Pos2, String, String, Option<String>)>,
    ctx_menu_just_opened: bool,

    // Keybinds
    pub(crate) keybinds: KeybindsMap,
    /// Action currently being rebound (capture mode).
    pub(crate) editing_keybind: Option<crate::ui::keybinds::KeybindAction>,
    pub(crate) quick_switcher: crate::ui::quick_switcher::QuickSwitcherState,

    // Collapsed server groups in the buffer list (stored by buffer.server key).
    pub(crate) collapsed_servers: HashSet<String>,

    // Adaptive wallpaper theme
    pub(crate) adaptive_theme: bool,
    /// Wallpaper-derived theme override; None when adaptive is off or not yet loaded.
    pub(crate) adaptive_theme_result: Option<crate::ui::theme::AppTheme>,
    /// Channel from the wallpaper-watcher background thread.
    pub(crate) wallpaper_rx: Option<std::sync::mpsc::Receiver<crate::ui::theme::AppTheme>>,

    // /np (now-playing) channel: tokio task → main loop; Ok = message to send, Err = why not
    pub(crate) np_tx: mpsc::UnboundedSender<Result<(String, String), String>>,
    pub(crate) np_rx: mpsc::UnboundedReceiver<Result<(String, String), String>>,
    pub(crate) np_error: Option<String>,

    // /sysinfo channel: background thread → main loop → send to active buffer
    pub(crate) sysinfo_tx: mpsc::UnboundedSender<(String, String)>,
    pub(crate) sysinfo_rx: mpsc::UnboundedReceiver<(String, String)>,

    // File share: tokio upload task → main loop; Ok = URL to post, Err = error message
    pub(crate) file_share_tx: mpsc::UnboundedSender<Result<(String, String), String>>,
    pub(crate) file_share_rx: mpsc::UnboundedReceiver<Result<(String, String), String>>,
    pub(crate) file_share_uploading: bool,
    pub(crate) file_share_error: Option<String>,
    pub(crate) file_share_service: crate::ui::fileshare::FileShareService,
    pub(crate) file_share_duration: String,
    pub(crate) jirafeau_url: String,
    /// Progress of the running upload (None when idle).
    pub(crate) file_share_progress: Option<std::sync::Arc<crate::ui::fileshare::UploadProgress>>,
    /// Jirafeau passwords as edited in Settings, loaded from the Keychain for `jirafeau_pw_loaded_for`.
    pub(crate) jirafeau_upload_pw: String,
    pub(crate) jirafeau_download_pw: String,
    pub(crate) jirafeau_pw_loaded_for: Option<String>,
    /// Result of Settings → File sharing → Test server (written by the async check).
    pub(crate) jirafeau_test: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,

    // fset buffer panel state (search filter + inline edit)
    pub(crate) fset_state: crate::ui::fset::FsetState,

    // How many connection attempts have been made since the last explicit Connect click.
    pub(crate) connection_attempts: u32,
}

pub(crate) struct CompletionState {
    pub(crate) matches: Vec<String>,
    pub(crate) index: usize,
    pub(crate) word_start_idx: usize,
    /// Appended after each match (": " for a nick at line start).
    pub(crate) suffix: String,
}

pub(crate) struct RemoteCompletion {
    /// Key into `completion_replies`.
    pub(crate) key: String,
    /// Input at the time of the request; the reply is dropped if the user kept typing.
    pub(crate) text: String,
    pub(crate) input_id: egui::Id,
}

impl WeeChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (shared_event_tx, event_rx) = mpsc::unbounded_channel::<(String, BackendEvent)>();
        let (image_tx, image_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        let (np_tx, np_rx) = mpsc::unbounded_channel::<Result<(String, String), String>>();
        let (sysinfo_tx, sysinfo_rx) = mpsc::unbounded_channel::<(String, String)>();
        let (file_share_tx, file_share_rx) = mpsc::unbounded_channel::<Result<(String, String), String>>();

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
                channel: String::new(),
                ssh_enabled: false,
                ssh_host: String::new(),
                ssh_port: None,
                ssh_user: String::new(),
                ssh_save_password: false,
                auto_reconnect: true,
            });
        }

        crate::ui::fonts::apply(&cc.egui_ctx, &settings.font_path);
        let available_fonts = crate::ui::fonts::scan_system_fonts();

        let adaptive_theme = settings.adaptive_theme;
        let wallpaper_rx = if adaptive_theme {
            Some(crate::ui::wallpaper::start_wallpaper_thread(cc.egui_ctx.clone()))
        } else {
            None
        };

        let mut app = Self {
            connections: Vec::new(),
            profiles,
            shared_event_tx,
            event_rx,
            show_connection_log: false,
            connection_log_unread: false,
            selected_conn_log: None,
            editing_profile: ConnectionProfile::default(),
            editing_password: String::new(),
            editing_ssh_password: String::new(),
            editing_profile_idx: None,
            show_connections: false,
            conn_show_add: false,
            conn_connect_idx: None,
            session_passwords: std::collections::HashMap::new(),
            session_ssh_passwords: std::collections::HashMap::new(),
            buffers: Vec::new(),
            buffer_idx: HashMap::new(),
            selected_buffer_id: None,
            input_text: String::new(),
            show_settings: false,
            settings_section: Default::default(),
            settings_window_pos: None,
            connections_window_pos: None,
            show_filtered_lines: settings.show_filtered_lines,
            colored_nicks: settings.colored_nicks,
            theme: settings.theme,
            font_size: settings.font_size,
            monospace_ui: settings.monospace_ui,
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
            blur_background: settings.blur_background,
            #[cfg(target_os = "macos")]
            applied_blur: None,
            show_hidden_buffers: settings.show_hidden_buffers,
            group_messages: settings.group_messages,
            fit_colors_to_theme: settings.fit_colors_to_theme,
            image_cache: HashMap::new(),
            image_expanded: HashSet::new(),
            image_tx,
            image_rx,
            preview_cache: HashMap::new(),
            preview_expanded: HashSet::new(),
            preview_tx,
            preview_rx,
            completion: None,
            remote_completion: None,
            remote_completion_seq: 0,
            completion_replies: HashMap::new(),
            command_history: VecDeque::new(),
            history_index: None,
            focus_input: false,
            show_search: false,
            search_text: String::new(),
            server_search: None,
            next_request_id: 0,
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
            theme_search: String::new(),
            theme_import_error: None,
            prefix_align_max: settings.prefix_align_max,
            prefix_suffix: settings.prefix_suffix,
            prefix_col_widths: HashMap::new(),
            chat_view_start: None,
            nick_style_cache: None,
            ctx_menu_state: None,
            ctx_menu_just_opened: false,
            np_tx,
            np_rx,
            np_error: None,
            sysinfo_tx,
            sysinfo_rx,
            file_share_tx,
            file_share_rx,
            file_share_uploading: false,
            file_share_error: None,
            file_share_service: settings.file_share_service,
            file_share_duration: settings.file_share_duration,
            jirafeau_url: settings.jirafeau_url,
            file_share_progress: None,
            jirafeau_upload_pw: String::new(),
            jirafeau_download_pw: String::new(),
            jirafeau_pw_loaded_for: None,
            jirafeau_test: Default::default(),
            keybinds: settings.keybinds.with_missing_defaults(),
            editing_keybind: None,
            quick_switcher: Default::default(),
            collapsed_servers: settings.collapsed_servers,
            adaptive_theme,
            adaptive_theme_result: None,
            wallpaper_rx,
            fset_state: crate::ui::fset::FsetState::default(),
            connection_attempts: 0,
        };

        // Trigger autoconnect for profiles that have the flag set and a saved password.
        let auto_profiles: Vec<ConnectionProfile> = app.profiles.iter()
            .filter(|p| p.auto_connect)
            .cloned()
            .collect();
        for profile in auto_profiles {
            let password = if profile.save_password {
                crate::ui::secure_storage::load_by_key(&profile.keyring_host_key())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            app.do_connect(&profile, password, &cc.egui_ctx);
        }

        app
    }

    pub(crate) fn hash_nick(name: &str) -> u8 {
        crate::relay::models::nick_color_slot(name)
    }

    /// Opaque titlebar strip drawn under the native traffic lights (the window
    /// uses a full-size content view, see main.rs). Handles drag and double-click zoom.
    #[cfg(target_os = "macos")]
    fn draw_macos_titlebar(&self, root_ui: &mut egui::Ui, surface_color: Color32, text_color: Color32, border_color: Color32) {
        let fill = Color32::from_rgb(surface_color.r(), surface_color.g(), surface_color.b());

        let title = self.selected_buffer_id.as_ref()
            .and_then(|id| self.buffer_by_id(id))
            .map(|b| format!("{} — WeeChatRS", b.name))
            .unwrap_or_else(|| "WeeChatRS".to_string());

        macos_titlebar(root_ui, &title, fill, border_color, text_color);
    }

    /// Keychain entry for a Jirafeau password (`kind` = "upload" or "download"), per server.
    fn jirafeau_keychain_key(kind: &str, base_url: &str) -> String {
        let url = crate::ui::fileshare::normalize_base_url(base_url).unwrap_or_else(|_| base_url.trim().to_string());
        format!("fileshare:jirafeau:{kind}:{url}")
    }

    /// Load the Jirafeau passwords for the current server into the Settings fields, once
    /// per server (switching servers loads that server's passwords).
    pub(crate) fn ensure_jirafeau_passwords_loaded(&mut self) {
        if self.jirafeau_pw_loaded_for.as_deref() == Some(self.jirafeau_url.as_str()) {
            return;
        }
        let load = |kind| crate::ui::secure_storage::load_by_key(&Self::jirafeau_keychain_key(kind, &self.jirafeau_url)).unwrap_or_default();
        self.jirafeau_upload_pw = load("upload");
        self.jirafeau_download_pw = load("download");
        self.jirafeau_pw_loaded_for = Some(self.jirafeau_url.clone());
    }

    /// Store (or, when empty, remove) a Jirafeau password for the current server.
    pub(crate) fn save_jirafeau_password(&mut self, kind: &str) {
        let key = Self::jirafeau_keychain_key(kind, &self.jirafeau_url);
        let value = if kind == "upload" { &self.jirafeau_upload_pw } else { &self.jirafeau_download_pw };
        let result = if value.is_empty() {
            crate::ui::secure_storage::delete_by_key(&key)
        } else {
            crate::ui::secure_storage::save_by_key(&key, value)
        };
        if let Err(e) = result {
            log::warn!("saving Jirafeau {kind} password: {e}");
        }
    }

    /// Upload `path` (or, when None, a file picked from a dialog) and post the link to
    /// `buf_id` when done. Progress is shown above the input bar.
    pub(crate) fn start_upload(&mut self, buf_id: String, path: Option<std::path::PathBuf>) {
        if self.file_share_progress.is_some() {
            return;
        }
        let jirafeau = if self.file_share_service == crate::ui::fileshare::FileShareService::Jirafeau {
            self.ensure_jirafeau_passwords_loaded();
            crate::ui::fileshare::JirafeauOptions {
                base_url: self.jirafeau_url.clone(),
                upload_password: self.jirafeau_upload_pw.clone(),
                download_password: self.jirafeau_download_pw.clone(),
            }
        } else {
            Default::default()
        };
        let progress = crate::ui::fileshare::UploadProgress::new();
        self.file_share_progress = Some(progress.clone());
        self.file_share_uploading = true;
        self.file_share_error = None;
        let duration = self.file_share_duration.clone();
        let service = self.file_share_service.clone();
        let tx = self.file_share_tx.clone();
        tokio::spawn(async move {
            let path = match path {
                Some(p) => p,
                None => match rfd::AsyncFileDialog::new().pick_file().await {
                    Some(h) => h.path().to_path_buf(),
                    // User cancelled — signal "done" with no error.
                    None => { let _ = tx.send(Err(String::new())); return; }
                },
            };
            let result = crate::ui::fileshare::upload(path, &service, &duration, &jirafeau, progress).await;
            let _ = tx.send(result.map(|url| (buf_id, url)));
        });
    }

    pub(crate) fn draw_sidebar_icon(painter: &Painter, rect: Rect, color: Color32, is_right: bool) {
        let stroke = Stroke::new(1.5, color);
        let rounding = CornerRadius::same(2);
        painter.rect_stroke(rect.shrink(4.0), rounding, stroke, egui::StrokeKind::Middle);

        let split_x = if is_right { rect.right() - 8.0 } else { rect.left() + 8.0 };
        painter.line_segment(
            [egui::pos2(split_x, rect.top() + 4.0), egui::pos2(split_x, rect.bottom() - 4.0)],
            stroke
        );
    }

    pub(crate) fn is_twemoji_url(url: &str) -> bool {
        url.contains("cdnjs.cloudflare.com/ajax/libs/twemoji")
    }

    /// Whether a URL points at an image, judged by the file extension of its path or,
    /// failing that, of a filename in its `#fragment`. File hosts such as Jirafeau link as
    /// `f.php?h=…&p=1#Screenshot.png`, with the real filename only in the fragment.
    pub(crate) fn is_image_url(url: &str) -> bool {
        let has_image_ext = |s: &str| matches!(
            std::path::Path::new(&s.to_lowercase()).extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
        );
        let (before_fragment, fragment) = url.split_once('#').unwrap_or((url, ""));
        let path = before_fragment.split('?').next().unwrap_or(before_fragment);
        has_image_ext(path) || (!fragment.is_empty() && has_image_ext(fragment.rsplit('/').next().unwrap_or(fragment)))
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
        self.server_search = None;
        self.focus_input = true;
        self.selected_view_since = Some(std::time::Instant::now());
        self.cleared_buffer_ids.insert(id.clone());
        self.chat_view_start = self.buffer_by_id(&id).and_then(|b| {
            b.messages.len().checked_sub(INITIAL_LINES).map(|i| b.messages[i].id.clone())
        });
        if let Some(buffer) = self.buffer_by_id_mut(&id) {
            buffer.activity = BufferActivity::None;
            buffer.unread_count = 0;
            if buffer.visit_start_marker_id.is_none() {
                buffer.visit_start_marker_id = buffer.last_read_id.clone();
            }
            let fetch_nicks = buffer.has_nicklist;
            // fset has a line per option, often thousands. On WeeChat the fset session
            // fetches them once the panel's format is in place.
            let is_fset = crate::ui::fset::is_fset_buffer(&buffer.full_name);
            let lines = if is_fset { MAX_STORED_LINES } else { INITIAL_LINES };
            if let Some((client, raw_id)) = self.client_for_buffer(&id) {
                client.refresh_buffer(&raw_id);
                if !(is_fset && client.supports_completion()) {
                    client.fetch_lines(&raw_id, lines);
                }
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
        let relay_port = profile.port.parse::<u16>().unwrap_or(9001);

        // Cache the relay password for this session so future connects skip the prompt.
        if !password.is_empty() {
            self.session_passwords.insert(prefix.clone(), password.clone());
        }

        // Start SSH tunnel if configured. Capture a log message for both success and failure.
        let ssh_password = self.session_ssh_passwords.get(&prefix).cloned()
            .or_else(|| if profile.ssh_save_password {
                crate::ui::secure_storage::load_by_key(&profile.ssh_keyring_key())
            } else { None });

        let mut ssh_tunnel_log: Option<String> = None;
        let (effective_host, effective_port, ssh_tunnel) = if profile.ssh_enabled && !profile.ssh_host.is_empty() {
            match crate::ui::ssh_tunnel::SshTunnel::spawn(
                &profile.ssh_host,
                profile.ssh_port,
                &profile.ssh_user,
                ssh_password.as_deref(),
                &profile.host,
                relay_port,
            ) {
                Ok(t) => {
                    let lp = t.local_port;
                    ssh_tunnel_log = Some(format!(
                        "SSH tunnel started: 127.0.0.1:{} → {}:{}",
                        lp, profile.host, relay_port
                    ));
                    ("127.0.0.1".to_string(), lp, Some(t))
                }
                Err(e) => {
                    ssh_tunnel_log = Some(format!("SSH tunnel failed to start: {} — connecting directly", e));
                    (profile.host.clone(), relay_port, None)
                }
            }
        } else {
            (profile.host.clone(), relay_port, None)
        };

        let (proto, path) = match profile.backend_type {
            BackendType::Soju    => (if profile.use_ssl { "ircs" } else { "irc" }, ""),
            BackendType::WeeChat => (if profile.use_ssl { "wss"  } else { "ws"  }, "/api"),
        };

        let tunnel_port = ssh_tunnel.as_ref().map(|t| t.local_port);
        let effective_auto_reconnect = profile.auto_reconnect && self.auto_reconnect;

        let mut client: Box<dyn BackendClient> = match profile.backend_type {
            BackendType::Soju => {
                let config = crate::relay::irc::IrcConfig {
                    label: profile.label.clone(),
                    host: effective_host.clone(),
                    port: effective_port,
                    nick: if profile.nick.is_empty() { "user".to_string() } else { profile.nick.clone() },
                    username: profile.username.clone(),
                    sasl_username: profile.sasl_username.clone(),
                    password: password.clone(),
                    use_ssl: profile.use_ssl,
                    accept_invalid_certs: profile.accept_invalid_certs,
                    channel: profile.channel.clone(),
                    tunnel_port,
                    auto_reconnect: effective_auto_reconnect,
                    bind_netid: None,
                    list_networks: true,
                };
                Box::new(crate::relay::irc::IrcClient::new(config, per_conn_tx.clone(), ctx.clone()))
            }
            BackendType::WeeChat => {
                let config = WeeChatConfig {
                    host: effective_host.clone(),
                    port: effective_port,
                    password: password.clone(),
                    use_ssl: profile.use_ssl,
                    accept_invalid_certs: profile.accept_invalid_certs,
                    tunnel_port,
                    auto_reconnect: effective_auto_reconnect,
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
            auto_reconnect: effective_auto_reconnect,
            connection_log: VecDeque::new(),
            ssh_tunnel,
        };

        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(msg) = ssh_tunnel_log {
            conn.connection_log.push_back(format!("[{}]  {}", ts, msg));
        }
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        conn.connection_log.push_back(format!("[{}]  Connecting to {}://{}:{}{}", ts, proto, profile.host, relay_port, path));
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        conn.connection_log.push_back(format!("[{}]  SSL/TLS: {}", ts, if profile.use_ssl { "enabled" } else { "disabled" }));

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
            monospace_ui: self.monospace_ui,
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
            blur_background: self.blur_background,
            show_hidden_buffers: self.show_hidden_buffers,
            group_messages: self.group_messages,
            fit_colors_to_theme: self.fit_colors_to_theme,
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
            file_share_service: self.file_share_service.clone(),
            file_share_duration: self.file_share_duration.clone(),
            jirafeau_url: self.jirafeau_url.clone(),
            keybinds: self.keybinds.clone(),
            collapsed_servers: self.collapsed_servers.clone(),
            adaptive_theme: self.adaptive_theme,
        };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = &root_ui.ctx().clone();

        #[cfg(not(target_os = "macos"))]
        let _ = frame;
        #[cfg(target_os = "macos")]
        if self.applied_blur != Some(self.blur_background) {
            if let Err(e) = crate::ui::vibrancy::set_blur(&*frame, self.blur_background) {
                log::warn!("vibrancy: {e}");
            }
            self.applied_blur = Some(self.blur_background);
        }
        if !self.notify_initialized {
            self.notify_initialized = true;
            crate::ui::notify::init();
        }

        while let Ok((prefix, event)) = self.event_rx.try_recv() {
            self.handle_event(&prefix, event);
        }

        // Drain the wallpaper-watcher thread; update override when a new theme arrives.
        if let Some(rx) = &self.wallpaper_rx {
            while let Ok(theme) = rx.try_recv() {
                self.adaptive_theme_result = Some(theme);
            }
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

        let mut got_image = false;
        while let Ok((url, result)) = self.image_rx.try_recv() {
            match result {
                Ok(bytes) => {
                    match image::load_from_memory(&bytes) {
                        Ok(mut img) => {
                            // Inline previews render ≤500px wide; cap the texture well
                            // below GPU texture-size limits.
                            const MAX_SIDE: u32 = 2048;
                            if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
                                img = img.thumbnail(MAX_SIDE, MAX_SIDE);
                            }
                            let rgba = img.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let color_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                            let handle = ctx.load_texture(&url, color_img, egui::TextureOptions::default());
                            self.image_cache.insert(url, ImageState::Loaded(handle));
                        }
                        Err(e) => {
                            log::warn!("image decode failed for {url}: {e}");
                            self.image_cache.insert(url, ImageState::Failed);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("image fetch failed for {url}: {e}");
                    self.image_cache.insert(url, ImageState::Failed);
                }
            }
            cap_map(&mut self.image_cache, IMAGE_CACHE_MAX);
            got_image = true;
        }
        if got_image { ctx.request_repaint(); }

        let mut got_preview = false;
        while let Ok((url, result)) = self.preview_rx.try_recv() {
            match result {
                Ok(preview) => {
                    if let Some(img_url) = &preview.image_url {
                        if is_safe_public_url(img_url) && !self.image_cache.contains_key(img_url) {
                            self.image_cache.insert(img_url.clone(), ImageState::Loading);
                            let tx = self.image_tx.clone();
                            let img_url_owned = img_url.clone();
                            let ctx2 = ctx.clone();
                            tokio::spawn(async move {
                                let result = fetch_image_bytes(&img_url_owned).await;
                                let _ = tx.send((img_url_owned, result));
                                ctx2.request_repaint();
                            });
                        }
                    }
                    self.preview_cache.insert(url, PreviewState::Loaded(preview));
                }
                Err(_) => { self.preview_cache.insert(url, PreviewState::Failed); }
            }
            cap_map(&mut self.preview_cache, PREVIEW_CACHE_MAX);
            got_preview = true;
        }
        if got_preview { ctx.request_repaint(); }

        // /np drain: send the formatted now-playing message, or show why there is none.
        while let Ok(result) = self.np_rx.try_recv() {
            match result {
                Ok((buf_id, np_text)) => {
                    if let Some((client, raw_id)) = self.client_for_buffer(&buf_id) {
                        client.send_message(&raw_id, &np_text);
                    }
                }
                Err(e) => self.np_error = Some(e),
            }
        }

        // /sysinfo drain: send to the active buffer as a regular message.
        while let Ok((buf_id, text)) = self.sysinfo_rx.try_recv() {
            if let Some((client, raw_id)) = self.client_for_buffer(&buf_id) {
                client.send_message(&raw_id, &text);
            }
        }

        // File share drain: on success send URL; on error show toast (empty = silent cancel).
        while let Ok(result) = self.file_share_rx.try_recv() {
            self.file_share_uploading = false;
            self.file_share_progress = None;
            match result {
                Ok((buf_id, url)) => {
                    if let Some((client, raw_id)) = self.client_for_buffer(&buf_id) {
                        client.send_message(&raw_id, &url);
                    }
                }
                Err(e) if !e.is_empty() && e != crate::ui::fileshare::CANCELLED => {
                    self.file_share_error = Some(e);
                }
                Err(_) => {}
            }
        }

        // Drag-and-drop file upload
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped_files.is_empty() && !self.file_share_uploading {
            if let Some(path) = dropped_files.into_iter().find_map(|f| Some(f.path().to_path_buf())) {
                if let Some(buf_id) = self.selected_buffer_id.clone() {
                    self.start_upload(buf_id, Some(path));
                }
            }
        }

        let mut tab_pressed = false;
        let mut arrow_up_shortcut = false;
        let mut arrow_down_shortcut = false;
        let mut history_up = false;
        let mut history_down = false;
        let mut search_shortcut = false;
        let mut jump_next_unread = false;
        let mut jump_buffer_n: Option<usize> = None;
        let mut quick_switcher_shortcut = false;
        let mut open_settings_shortcut = false;
        let mut open_connections_shortcut = false;
        let quick_switcher_open = self.quick_switcher.open;

        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Tab) {
                tab_pressed = true;
            }

            let kb = &self.keybinds;
            if kb.consume(i, crate::ui::keybinds::KeybindAction::CycleBufferUp) {
                arrow_up_shortcut = true;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::CycleBufferDown) {
                arrow_down_shortcut = true;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::ToggleSearch) {
                search_shortcut = true;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::ToggleBufferList) {
                self.show_buffers = !self.show_buffers;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::ToggleNicklist) {
                self.show_nicklist = !self.show_nicklist;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::ToggleToolbar) {
                self.show_toolbar = !self.show_toolbar;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::JumpNextUnread) {
                jump_next_unread = true;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::QuickSwitcher) {
                quick_switcher_shortcut = true;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::OpenSettings) {
                open_settings_shortcut = true;
            }
            if kb.consume(i, crate::ui::keybinds::KeybindAction::OpenConnections) {
                open_connections_shortcut = true;
            }
            if let Some(n) = kb.consume_jump_number(i) {
                jump_buffer_n = Some(n);
            }

            let any_mod = i.modifiers.command || i.modifiers.alt || i.modifiers.mac_cmd || i.modifiers.ctrl;
            // While the quick switcher is open it owns the arrow keys.
            if !any_mod && !quick_switcher_open {
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) { history_up = true; }
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) { history_down = true; }
            }
        });

        if arrow_up_shortcut { self.cycle_buffer(-1); }
        if arrow_down_shortcut { self.cycle_buffer(1); }
        if search_shortcut {
            self.show_search = !self.show_search;
            if !self.show_search { self.server_search = None; }
        }
        if jump_next_unread { self.jump_next_unread(); }
        if quick_switcher_shortcut { self.quick_switcher.toggle(); }
        if open_settings_shortcut { self.open_settings(ctx); }
        if open_connections_shortcut { self.open_connections(ctx); }
        if let Some(n) = jump_buffer_n { self.jump_buffer_by_number(n); }

        if self.font_path != self.applied_font_path {
            crate::ui::fonts::apply(ctx, &self.font_path);
            self.applied_font_path = self.font_path.clone();
        }

        let mut style = (*ctx.global_style()).clone();
        let font_family = if self.monospace_ui { crate::ui::fonts::chat_family() } else { FontFamily::Proportional };

        style.text_styles = [
            (TextStyle::Small, FontId::new(self.font_size * 0.8, font_family.clone())),
            (TextStyle::Body, FontId::new(self.font_size, font_family.clone())),
            (TextStyle::Button, FontId::new(self.font_size, font_family.clone())),
            (TextStyle::Heading, FontId::new(self.font_size * 1.4, font_family.clone())),
            (TextStyle::Monospace, FontId::new(self.font_size, FontFamily::Monospace)),
        ].into();
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
        style.spacing.window_margin = Margin::same(12);
        style.visuals.window_corner_radius = CornerRadius::same(12);
        style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(8);
        ctx.all_styles_mut(|s| *s = style.clone());

        // Effective theme: adaptive override when enabled and loaded, else user's theme.
        let mut render_theme: crate::ui::theme::AppTheme = self.adaptive_theme_result
            .clone()
            .unwrap_or_else(|| self.theme.clone());
        render_theme.fit_extended = self.fit_colors_to_theme && render_theme.name != "Default";

        let accent_color = if render_theme.name == "Default" {
            Color32::from_rgb(100, 149, 237)
        } else {
            Color32::from(render_theme.ansi[4])
        };
        let base_bg = render_theme.background.map(Color32::from).unwrap_or(Color32::from_rgb(18, 18, 18));
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
            // A step lighter than the background, keeping the theme's tint.
            Color32::from_rgba_unmultiplied(
                base_bg.r().saturating_add(10),
                base_bg.g().saturating_add(10),
                base_bg.b().saturating_add(10),
                alpha,
            )
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

        let text_primary = render_theme.foreground
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
            // Text fields: a darker shade of the background, not a flat black.
            let shade = |c: u8| (c as f32 * 0.65) as u8;
            Color32::from_rgba_unmultiplied(shade(base_bg.r()), shade(base_bg.g()), shade(base_bg.b()), alpha)
        };
        visuals.widgets.active.bg_fill = accent_color;
        visuals.selection.bg_fill = accent_color.linear_multiply(0.5);
        if let Some(fg) = render_theme.foreground {
            visuals.override_text_color = Some(fg.into());
        } else if is_light {
            visuals.override_text_color = Some(text_primary);
        }
        ctx.all_styles_mut(|s| s.visuals = visuals.clone());

        let mut next_selected_buffer_id = None;
        let mut pending_collapse_toggle: Option<String> = None;
        let mut pending_buffer_command = None;
        let mut next_drag_buffer_id: Option<String> = None;
        let mut pending_mute: Option<(String, String, bool)> = None;
        let mut pending_load_more: Option<(String, usize)> = None;

        #[cfg(target_os = "macos")]
        self.draw_macos_titlebar(root_ui, surface_color, text_secondary, border_color);

        if self.show_toolbar { egui::Panel::top("top_panel")
            .frame(Frame::NONE.fill(surface_color).inner_margin(Margin::symmetric(12, 8)))
            .show(root_ui, |ui| {
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

                        if ui.button(egui::RichText::new(icons::GEAR).size(16.0)).on_hover_text("Settings").clicked() {
                            self.open_settings(ui.ctx());
                        }
                        if ui.button(egui::RichText::new(icons::PLUGS).size(14.0)).on_hover_text("Connections").clicked() {
                            self.open_connections(ui.ctx());
                        }
                        let log_icon = if self.connection_log_unread {
                            egui::RichText::new(icons::TERMINAL_WINDOW).size(16.0).color(Color32::from_rgb(255, 165, 0))
                        } else {
                            egui::RichText::new(icons::TERMINAL_WINDOW).size(16.0)
                        };
                        if ui.button(log_icon).on_hover_text("Connection log").clicked() {
                            self.show_connection_log = !self.show_connection_log;
                            self.connection_log_unread = false;
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
                let buf_font_id = FontId::new(self.font_size, crate::ui::fonts::chat_family());
                let char_w = ctx.fonts_mut(|f| f.glyph_width(&buf_font_id, 'W'));
                self.buffers_width = char_w * 20.0 + 20.0;
            }
            let buffers_max_w = (ctx.content_rect().width() * 0.40).max(80.0);
            let buffers_resp = egui::Panel::left("buffers_panel")
                .resizable(true)
                .default_size(self.buffers_width)
                .min_size(80.0)
                .max_size(buffers_max_w)
                .frame(Frame::NONE.fill(bg_color).inner_margin(Margin::same(10)))
                .show(root_ui, |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
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

                    // The floating bar would sit over the fold chevrons at the right edge
                    // of the server rows, so while the list scrolls the rows end short
                    // of the bar at its widest (hovered).
                    let scroll = &mut ui.spacing_mut().scroll;
                    scroll.floating_allocated_width = scroll.bar_width + scroll.bar_inner_margin;
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut last_conn_prefix: Option<String> = None;
                        for buffer in &self.buffers {
                            if buffer.hidden && !self.show_hidden_buffers {
                                continue;
                            }
                            let is_selected = self.selected_buffer_id.as_deref() == Some(&buffer.id);
                            let is_core  = buffer.kind == "core";
                            let is_root  = buffer.kind == "server" || is_core;
                            // Skip children of collapsed server groups.
                            if !is_root && self.collapsed_servers.contains(&buffer.server) {
                                continue;
                            }
                            let is_child = !is_root;
                            let in_dragged_group = dragged_group_ids.contains(&buffer.id);

                            // Connection header — shown when ≥2 connections are active, but
                            // suppressed when the first buffer in the group is a root buffer
                            // (server/core rows render themselves as section headers).
                            {
                                let buf_conn_prefix = buffer.id.split('/').next().unwrap_or("").to_string();
                                if last_conn_prefix.as_deref() != Some(&buf_conn_prefix) {
                                    if last_conn_prefix.is_some() { ui.add_space(6.0); }
                                    if multi_conn && !is_root {
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
                                    }
                                    last_conn_prefix = Some(buf_conn_prefix);
                                }
                            }

                            // Space before each root header row (server/core), except at the
                            // very top (last_conn_prefix was just set, already has add_space).
                            if is_root {
                                ui.add_space(4.0);
                            }

                            let is_muted = buffer.muted;
                            let (bg, fg) = if is_selected {
                                (accent_color.linear_multiply(0.2), text_primary)
                            } else if is_muted {
                                (Color32::TRANSPARENT, text_muted.linear_multiply(0.6))
                            } else if is_root {
                                // Server/core headers always use accent colour
                                (Color32::TRANSPARENT, accent_color)
                            } else {
                                match buffer.activity {
                                    BufferActivity::Highlight => (Color32::from_rgb(150, 50, 50).linear_multiply(0.3), Color32::from_rgb(255, 100, 100)),
                                    BufferActivity::Message   => (Color32::TRANSPARENT, text_primary),
                                    BufferActivity::Metadata  => (Color32::TRANSPARENT, text_secondary),
                                    BufferActivity::None      => (Color32::TRANSPARENT, text_muted),
                                }
                            };

                            let indent = if is_child { 12.0 } else { 0.0 };

                            // Fixed-height row: derive height from font metrics so the row
                            // never expands beyond one line regardless of text length or font size.
                            let avail_w = ui.available_width();
                            let line_h  = ui.text_style_height(&TextStyle::Body);
                            let row_h   = line_h + 8.0; // 4 px top + 4 px bottom padding

                            // Chevron width reserved on the right for collapsible server groups.
                            let chevron_w = if is_root && !is_core { 14.0_f32 } else { 0.0_f32 };

                            // Allocate exact row space — no sense here so layout is not affected.
                            let (outer_rect, _) = ui.allocate_exact_size(
                                Vec2::new(avail_w, row_h),
                                egui::Sense::hover(),
                            );

                            // Background frame rect (indented for child buffers).
                            // For root headers the frame width leaves room for the chevron.
                            let frame_rect = egui::Rect::from_min_size(
                                egui::pos2(outer_rect.min.x + indent, outer_rect.min.y),
                                egui::vec2((outer_rect.width() - indent).max(0.0), outer_rect.height()),
                            );
                            // Clip background to panel boundary so it never overdraws adjacent panels.
                            let panel_clip  = ui.clip_rect();
                            let fill = if in_dragged_group { bg.linear_multiply(0.35) } else { bg };
                            if fill != Color32::TRANSPARENT {
                                ui.painter().with_clip_rect(frame_rect.intersect(panel_clip))
                                    .rect_filled(frame_rect, CornerRadius::same(6), fill);
                            }

                            // Inner content rect — shrink by chevron on the right for root rows.
                            let inner_rect = egui::Rect::from_min_max(
                                frame_rect.shrink2(Vec2::new(8.0, 4.0)).min,
                                egui::pos2(frame_rect.max.x - chevron_w - 2.0, frame_rect.max.y - 4.0),
                            );
                            let clip_rect  = inner_rect.intersect(panel_clip);

                            if clip_rect.is_positive() {
                                let name = if !is_muted && buffer.activity == BufferActivity::Highlight {
                                    format!("• {}", buffer.name)
                                } else if is_muted {
                                    format!("{} {}", icons::BELL_SLASH, buffer.name)
                                } else {
                                    buffer.name.clone()
                                };
                                // Root buffers (server + core) use compact header styling.
                                let label = if is_root {
                                    egui::RichText::new(name.to_uppercase()).color(fg).strong().size(10.0)
                                } else if is_muted {
                                    egui::RichText::new(name).color(fg).italics()
                                } else {
                                    egui::RichText::new(name).color(fg).strong()
                                }.family(crate::ui::fonts::chat_family());
                                let unread       = if is_muted || is_root { 0 } else { buffer.unread_count };
                                let buf_activity = buffer.activity;

                                let mut row_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(inner_rect)
                                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                                );
                                row_ui.set_clip_rect(clip_rect);

                                if unread > 0 {
                                    let badge_text = if unread > 99 { "99+".to_string() } else { unread.to_string() };
                                    let badge_bg   = if buf_activity == BufferActivity::Highlight {
                                        Color32::from_rgb(200, 50, 50)
                                    } else {
                                        accent_color
                                    };
                                    Frame::NONE
                                        .fill(badge_bg)
                                        .corner_radius(CornerRadius::same(8))
                                        .inner_margin(Margin::symmetric(4, 1))
                                        .show(&mut row_ui, |ui| {
                                            ui.label(egui::RichText::new(badge_text)
                                                .color(Color32::WHITE)
                                                .strong()
                                                .size(self.font_size * 0.72));
                                        });
                                }
                                // selectable(false) prevents Label from grabbing clicks
                                // that belong to the row interact registered below.
                                row_ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(Label::new(label).truncate().selectable(false));
                                });
                            }

                            // Row interact rect excludes the chevron column so the two
                            // interactions don't overlap and both fire correctly.
                            let row_interact_rect = if chevron_w > 0.0 {
                                egui::Rect::from_min_max(
                                    outer_rect.min,
                                    egui::pos2(outer_rect.max.x - chevron_w, outer_rect.max.y),
                                )
                            } else {
                                outer_rect
                            };

                            // Register row interaction first (lower hit-test priority than chevron).
                            let row_id = egui::Id::unique("buf_row").with(&buffer.id);
                            let resp   = ui.interact(row_interact_rect, row_id, egui::Sense::click_and_drag());

                            // Chevron collapse toggle — registered after row so it wins the hit-test
                            // when the pointer is inside the (non-overlapping) chevron rect.
                            if is_root && !is_core {
                                let chevron_rect = egui::Rect::from_min_max(
                                    egui::pos2(outer_rect.max.x - chevron_w, outer_rect.min.y),
                                    outer_rect.max,
                                );
                                let is_collapsed = self.collapsed_servers.contains(&buffer.server);
                                let chevron_char = if is_collapsed { icons::CARET_RIGHT } else { icons::CARET_DOWN };
                                let chev_id   = egui::Id::unique("buf_chev").with(&buffer.id);
                                let chev_resp = ui.interact(chevron_rect, chev_id, egui::Sense::click());
                                let chev_color = if chev_resp.hovered() { text_primary } else { fg };
                                ui.painter().with_clip_rect(chevron_rect.intersect(panel_clip))
                                    .text(
                                        chevron_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        chevron_char,
                                        egui::FontId::new(8.0, FontFamily::Proportional),
                                        chev_color,
                                    );
                                if chev_resp.clicked() {
                                    pending_collapse_toggle = Some(buffer.server.clone());
                                }
                            }

                            if resp.hovered() {
                                let cursor = if is_dragging {
                                    egui::CursorIcon::Grabbing
                                } else if ctx.input(|i| i.pointer.primary_down()) {
                                    egui::CursorIcon::Grab
                                } else {
                                    egui::CursorIcon::Default
                                };
                                ctx.set_cursor_icon(cursor);
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
                                        if ui.button(format!("{} Unmute Buffer", icons::BELL)).clicked() {
                                            pending_mute = Some((buf_id.clone(), buf_full_name.clone(), false));
                                            ui.close();
                                        }
                                    } else {
                                        if ui.button(format!("{} Mute Buffer", icons::BELL_SLASH)).clicked() {
                                            pending_mute = Some((buf_id.clone(), buf_full_name.clone(), true));
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
                                    if buf_kind == "channel" {
                                        if ui.button("Leave Channel").clicked() {
                                            pending_buffer_command = Some((buf_id.clone(), "/part".to_string()));
                                            ui.close();
                                        }
                                    }
                                    if buf_hidden {
                                        if ui.button("Unhide Buffer").clicked() {
                                            pending_buffer_command = Some((buf_id.clone(), "/buffer unhide".to_string()));
                                            ui.close();
                                        }
                                    } else {
                                        if ui.button("Hide Buffer").clicked() {
                                            pending_buffer_command = Some((buf_id.clone(), "/buffer hide".to_string()));
                                            ui.close();
                                        }
                                    }
                                    if ui.button("Close Buffer").clicked() {
                                        pending_buffer_command = Some((buf_id.clone(), "/close".to_string()));
                                        ui.close();
                                    }
                                });
                            }

                            if !in_dragged_group {
                                row_rects.push((outer_rect, buffer.id.clone()));
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
        if let Some(server) = pending_collapse_toggle {
            if !self.collapsed_servers.remove(&server) {
                self.collapsed_servers.insert(server);
            }
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

        self.update_fset_session(ctx);

        let current_buffer_id = self.selected_buffer_id.clone();
        let current_buf = current_buffer_id.as_ref().and_then(|id| self.buffer_by_id(id));
        let current_buffer_nicks = current_buf.map(|b| std::sync::Arc::clone(&b.nicks));
        let current_buffer_full_name = current_buf.map(|b| b.full_name.clone());
        let is_fset_buffer = current_buffer_full_name.as_deref().is_some_and(crate::ui::fset::is_fset_buffer);
        let current_buffer_messages = current_buf.map(|b| std::sync::Arc::clone(&b.messages));
        let _current_buffer_last_read_id = current_buf.and_then(|b| b.last_read_id.clone());
        let current_buffer_visit_marker_id = current_buf.and_then(|b| b.visit_start_marker_id.clone());
        let current_buffer_topic = current_buf.map(|b| b.topic.clone()).unwrap_or_default();
        let current_buffer_modes = current_buf.map(|b| b.modes.clone()).unwrap_or_default();
        let current_buffer_kind = current_buf.map(|b| b.kind.clone()).unwrap_or_default();

        // Chat content (messages, nick list, input) uses the chat font stack.
        let font_id = FontId::new(self.font_size, crate::ui::fonts::chat_family());

        let any_connected = self.is_any_connected();

        let current_buf_has_nicklist = current_buf.map(|b| b.has_nicklist).unwrap_or(false);
        let chat_nick_styles = match (&current_buffer_id, &current_buffer_messages) {
            (Some(id), Some(messages)) if current_buffer_nicks.as_ref().is_some_and(|n| !n.is_empty()) => {
                let last_id = messages.back().map(|l| l.id.clone());
                let fresh = self.nick_style_cache.as_ref()
                    .is_some_and(|c| &c.buffer_id == id && c.len == messages.len() && c.last_id == last_id);
                if !fresh {
                    self.nick_style_cache = Some(NickStyleCache {
                        buffer_id: id.clone(),
                        len: messages.len(),
                        last_id,
                        styles: std::sync::Arc::new(nick_styles_in_chat(messages.iter())),
                    });
                }
                self.nick_style_cache.as_ref().map(|c| std::sync::Arc::clone(&c.styles))
            }
            _ => None,
        };
        if self.show_nicklist && current_buf_has_nicklist && any_connected && current_buffer_id.is_some() {
            if self.nicklist_width < 80.0 {
                self.nicklist_width = 180.0;
            }
            let nicks_max_w = (ctx.content_rect().width() * 0.30).max(80.0);
            let nicks_resp = egui::Panel::right("nicks_panel_2")
                .resizable(true)
                .default_size(self.nicklist_width)
                .min_size(80.0)
                .max_size(nicks_max_w)
                .frame(Frame::NONE.fill(bg_color).inner_margin(Margin::same(10)))
                .show(root_ui, |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
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
                                for nick in nicks.iter() {
                                    let text = format!("{}{}", nick.prefix, nick.name);
                                    let input = if nick.away {
                                        text.clone()
                                    } else if self.colored_nicks {
                                        if render_theme.name == "Default" { format!("{}{}", nick.color_ansi, text) }
                                        else {
                                            let idx = Self::hash_nick(&nick.name);
                                            let esc = if idx < 8 { format!("\x1B[{}m", 30 + idx) } else { format!("\x1B[{}m", 90 + idx - 8) };
                                            format!("{}{}", esc, text)
                                        }
                                    } else { text };
                                    // Prefer the colour the nick has in the chat; WeeChat's nick list
                                    // is uncoloured by default and the hash above is our own.
                                    let chat_style = chat_nick_styles.as_ref()
                                        .filter(|_| self.colored_nicks && !nick.away)
                                        .and_then(|s| s.get(&nick.name));
                                    let sections = match chat_style {
                                        Some(style) => vec![crate::ui::ansi::ANSISection {
                                            text: format!("{}{}", nick.prefix, nick.name),
                                            style: *style,
                                            url: None,
                                        }],
                                        None => ANSIParser::parse(&input),
                                    };
                                    let mut job = LayoutJob::default();
                                    for s in sections {
                                        let mut fmt = s.style.to_format(font_id.clone(), &render_theme);
                                        if nick.away {
                                            fmt.color = text_muted;
                                            fmt.italics = true;
                                        }
                                        job.append(&s.text, 0.0, fmt);
                                    }

                                    let label_res = ui.add(Label::new(job).truncate().sense(egui::Sense::click()));
                                    label_res.context_menu(|ui| {
                                        if ui.button(format!("Query {}", nick.name)).clicked() {
                                            self.send_command(&format!("/query {}", nick.name));
                                            ui.close();
                                        }
                                        if ui.button(format!("Whois {}", nick.name)).clicked() {
                                            self.send_command(&format!("/whois {}", nick.name));
                                            ui.close();
                                        }
                                    });
                                }
                            }
                        });
                });
            self.nicklist_width = nicks_resp.response.rect.width();
        }

        // fset has its own controls and highmon is read-only: no input bar there.
        let shows_input = !is_fset_buffer
            && !current_buffer_full_name.as_deref().is_some_and(is_highmon_buffer);
        if current_buffer_id.is_some() && shows_input {
            // Is the SELECTED buffer's connection up? Different from any_connected when
            // multiple connections are configured and only some are alive.
            let selected_buffer_connected = current_buffer_id.as_ref()
                .and_then(|id| id.split('/').next())
                .and_then(|prefix| self.connections.iter().find(|c| c.prefix == prefix))
                .map(|c| c.client.is_connected())
                .unwrap_or(false);

            egui::Panel::bottom("input_panel")
                .frame(Frame::NONE.fill(bg_color).inner_margin(Margin::symmetric(16, 10)))
                .show(root_ui, |ui| {
                    // Upload progress: name, bar with percentage/size, and Cancel.
                    if let Some(progress) = self.file_share_progress.clone() {
                        let total = progress.total();
                        let name = progress.file_name();
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{} {}", icons::UPLOAD_SIMPLE,
                                if name.is_empty() { "Choosing file…".to_string() } else { format!("Uploading {name}") }))
                                .color(text_secondary).small());
                            if total > 0 {
                                let text = format!("{:.0}%  ·  {} / {}", progress.fraction() * 100.0,
                                    crate::ui::fileshare::human_size(progress.sent()),
                                    crate::ui::fileshare::human_size(total));
                                ui.add(egui::ProgressBar::new(progress.fraction())
                                    .desired_width((ui.available_width() - 90.0).max(80.0))
                                    .fill(accent_color)
                                    .text(egui::RichText::new(text).small()));
                            }
                            if ui.small_button("Cancel").clicked() {
                                progress.cancel();
                            }
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    // File share error toast (clears on next click anywhere)
                    if let Some(err) = self.file_share_error.clone() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("Upload failed: {}", err))
                                .color(Color32::from_rgb(220, 80, 80)).small());
                            if ui.small_button(icons::X).clicked() {
                                self.file_share_error = None;
                            }
                        });
                    }
                    if let Some(err) = self.np_error.clone() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("/np: {}", err))
                                .color(Color32::from_rgb(220, 80, 80)).small());
                            if ui.small_button(icons::X).clicked() {
                                self.np_error = None;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let hint = if !selected_buffer_connected {
                            "Disconnected — reconnect before sending"
                        } else if current_buffer_kind == "server" {
                            "Type /join #channel or any IRC command..."
                        } else {
                            "Type a message..."
                        };

                        // 📎 file share button
                        let attach_enabled = selected_buffer_connected && !self.file_share_uploading;
                        let attach_label = if self.file_share_uploading {
                            egui::RichText::new(icons::HOURGLASS).size(16.0).color(Color32::GRAY)
                        } else {
                            egui::RichText::new(icons::PAPERCLIP).size(16.0)
                        };
                        if ui.add_enabled(
                            attach_enabled,
                            egui::Button::new(attach_label).frame(false),
                        ).on_hover_text("Upload a file (or drag & drop onto window)").clicked() {
                            if let Some(buf_id) = self.selected_buffer_id.clone() {
                                self.start_upload(buf_id, None);
                            }
                        }

                        ui.add_enabled_ui(selected_buffer_connected, |ui| {
                            let text_edit = egui::TextEdit::singleline(&mut self.input_text)
                                .hint_text(hint)
                                .font(font_id.clone())
                                .margin(Margin::symmetric(8, 4))
                                .lock_focus(true)
                                .desired_width(ui.available_width() - 80.0);

                            let res = ui.add(text_edit);
                            self.apply_remote_completion(ctx);

                            if self.focus_input {
                                res.request_focus();
                                self.focus_input = false;
                            }

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
                                        self.remote_completion = None;
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
            .frame(Frame::NONE.fill(bg_color).inner_margin(Margin::same(0)))
            .show(root_ui, |ui| {
            // egui panels use screen_rect as clip_rect — explicitly restrict to this
            // panel's rect so content cannot paint into adjacent side panels.
            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
            if self.show_connection_log {
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
                                if ui.button(egui::RichText::new(icons::X).size(14.0)).on_hover_text("Close").clicked() {
                                    self.show_connection_log = false;
                                }
                            });
                        });
                        ui.add_space(8.0);
                        egui::Frame::NONE
                            .fill(if is_light { Color32::from_gray(245) } else { Color32::from_rgb(14, 14, 14) })
                            .corner_radius(egui::CornerRadius::same(8))
                            .stroke(egui::Stroke::new(1.0, border_color))
                            .inner_margin(egui::Margin::same(12))
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
                    ui.add_space(ui.available_rect_before_wrap().height() * 0.2);
                    Frame::group(ui.style())
                        .fill(surface_color)
                        .corner_radius(CornerRadius::same(12))
                        .stroke(Stroke::new(1.0, border_color))
                        .inner_margin(Margin::same(40))
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
            } else if current_buffer_full_name.is_some() {
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width());
                    if self.show_search {
                        Frame::NONE
                            .fill(surface_color.linear_multiply(0.8))
                            .inner_margin(Margin::symmetric(16, 8))
                            .show(ui, |ui| {
                                let can_search_server = self.can_search_server();
                                ui.horizontal(|ui| {
                                    ui.label(icons::MAGNIFYING_GLASS);
                                    // Room for the server-search buttons when the server has search.
                                    let buttons_w = if can_search_server { 250.0 } else { 40.0 };
                                    let res = ui.add(egui::TextEdit::singleline(&mut self.search_text)
                                        .hint_text(if can_search_server { "Search scrollback — Enter searches the server" } else { "Search scrollback..." })
                                        .desired_width((ui.available_width() - buttons_w).max(80.0)));
                                    let enter = res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    if can_search_server {
                                        let has_text = !self.search_text.trim().is_empty();
                                        if ui.add_enabled(has_text, egui::Button::new("Search history"))
                                            .on_hover_text("Search this buffer's history on the server").clicked() || (enter && has_text)
                                        {
                                            self.start_server_search(false);
                                        }
                                        if ui.add_enabled(has_text, egui::Button::new("All buffers"))
                                            .on_hover_text("Search every channel and conversation on this connection").clicked()
                                        {
                                            self.start_server_search(true);
                                        }
                                    }
                                    if ui.button(icons::X).clicked() {
                                        self.show_search = false;
                                        self.search_text.clear();
                                        self.server_search = None;
                                    }
                                    if self.show_search && self.server_search.is_none() { res.request_focus(); }
                                });
                            });
                        ui.separator();
                    }

                    // fset's title is its terminal key help; the panel has its own toolbar.
                    if self.show_titlebar && !is_fset_buffer && (!current_buffer_topic.is_empty() || !current_buffer_modes.is_empty()) {
                        Frame::NONE
                            .fill(surface_color.linear_multiply(0.3))
                            .inner_margin(Margin::symmetric(16, 6))
                            .stroke(Stroke::new(1.0, Color32::from_white_alpha(10)))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    if !current_buffer_modes.is_empty() {
                                        ui.label(egui::RichText::new(format!("[{}]", current_buffer_modes)).color(accent_color).small());
                                    }
                                    if !current_buffer_topic.is_empty() {
                                        let topic_font = FontId::new(self.font_size, crate::ui::fonts::chat_family());
                                        let sections = ANSIParser::parse(&current_buffer_topic);
                                        let mut job = LayoutJob::default();
                                        for s in sections { job.append(&s.text, 0.0, s.style.to_format(topic_font.clone(), &render_theme)); }
                                        ui.add(Label::new(job).wrap());
                                    }
                                });
                            });
                        ui.add_space(-1.0);
                        ui.separator();
                    }

                    // -------------------------------------------------------
                    // fset buffer — custom table renderer
                    // -------------------------------------------------------
                    if is_fset_buffer {
                        if let (Some(messages), Some(buf_id)) = (&current_buffer_messages, &current_buffer_id) {
                            let colors = crate::ui::fset::FsetColors {
                                text: text_primary,
                                muted: text_muted,
                                accent: accent_color,
                                border: border_color,
                            };
                            let actions = crate::ui::fset::show_fset_panel(
                                ui,
                                buf_id,
                                messages,
                                &current_buffer_topic,
                                &mut self.fset_state,
                                self.font_size,
                                &colors,
                            );
                            self.apply_fset_actions(buf_id, actions);
                        }
                    } else if self.server_search.is_some() {
                        self.draw_server_search(ui, &render_theme, accent_color, text_muted);
                    } else {

                    // Where the drawn lines start; a search looks through all of them.
                    let chat_start = match (&self.chat_view_start, &current_buffer_messages) {
                        (Some(anchor), Some(messages)) if self.search_text.is_empty() => messages.iter()
                            .position(|l| &l.id == anchor)
                            .unwrap_or_else(|| messages.len().saturating_sub(INITIAL_LINES)),
                        _ => 0,
                    };
                    let mut show_older_in_memory = false;

                    let chat_scroll = ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .horizontal_scroll_offset(0.0)
                        .show(ui, |ui| {
                        // Capture width inside the scroll area so it reflects inner_size.x
                        // (outer width minus vertical scrollbar), preventing content_is_too_large.x
                        // from going true and enabling horizontal offset drift via drag-to-scroll.
                        let msg_area_width = ui.available_width();
                        ui.set_min_width(msg_area_width);
                        ui.set_max_width(msg_area_width);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        Frame::NONE.inner_margin(Margin::same(16)).show(ui, |ui| {
                            if let (Some(buf_id), Some(messages)) = (current_buffer_id.as_ref(), current_buffer_messages.as_ref()) {
                                if chat_start > 0 || messages.len() >= INITIAL_LINES {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.add_space((ui.available_width() - 180.0).max(0.0) / 2.0);
                                        if self.loading_more_buffer_id.as_deref() == Some(buf_id.as_str()) {
                                            ui.spinner();
                                            ui.label(egui::RichText::new("Loading…").color(text_muted).small());
                                        } else if ui.button(format!("{} Load older messages", icons::ARROW_UP)).clicked() {
                                            if chat_start > 0 {
                                                show_older_in_memory = true;
                                            } else {
                                                pending_load_more = Some((buf_id.clone(), messages.len() + LOAD_MORE_LINES));
                                            }
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
                                // Previous line actually shown, for grouping consecutive messages from one nick.
                                let mut prev_shown: Option<&Line> = None;
                                let search_query = if self.search_text.is_empty() {
                                    None
                                } else {
                                    Some(self.search_text.to_lowercase())
                                };
                                for line in messages.iter().skip(chat_start) {
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
                                        // Both forms take the same height, so the label fading to a
                                        // plain line doesn't shift the lines below.
                                        let divider_height = ui.spacing().interact_size.y
                                            .max(ui.fonts_mut(|f| f.row_height(&FontId::proportional(10.0))));
                                        if elapsed < std::time::Duration::from_secs(2) {
                                            let remaining = std::time::Duration::from_secs(2) - elapsed;
                                            ctx.request_repaint_after(remaining);
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(ui.available_width(), divider_height),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_height(divider_height);
                                                    ui.add_space(20.0);
                                                    ui.separator();
                                                    ui.label(egui::RichText::new(" NEW MESSAGES ").color(divider_color).size(10.0).strong());
                                                    ui.separator();
                                                },
                                            );
                                        } else {
                                            ui.scope(|ui| {
                                                ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, divider_color);
                                                ui.add_sized([ui.available_width(), divider_height], egui::Separator::default());
                                            });
                                        }
                                        ui.add_space(8.0);
                                        marker_shown = true;
                                        prev_shown = None; // a group never spans the unread divider
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
                                        let c = Color32::from(render_theme.ansi[3]);
                                        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 28)
                                    } else {
                                        Color32::TRANSPARENT
                                    };
                                    let grouped = self.group_messages && continues_group(prev_shown, line);
                                    prev_shown = Some(line);
                                    // Grouped rows reserve the timestamp and nick columns as empty space of
                                    // the same size (nothing to select or copy); the timestamp shows on hover.
                                    let mut hidden_timestamp: Option<(Rect, String)> = None;
                                    ui.push_id(&line.id, |ui| {
                                    let mut row_hovered_url: Option<String> = None;
                                    let row_resp = Frame::NONE
                                        .fill(row_bg)
                                        .corner_radius(CornerRadius::same(3))
                                        .show(ui, |ui| {
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        if self.show_timestamps {
                                            let ts = line.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string();
                                            if grouped {
                                                let size = ui.fonts_mut(|f| f.layout_no_wrap(ts.clone(), font_id.clone(), text_muted).size());
                                                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                                                hidden_timestamp = Some((rect, ts));
                                            } else {
                                                ui.label(egui::RichText::new(&ts).font(font_id.clone()).color(text_muted));
                                            }
                                        }
                                        let prefix_sections = &line.parsed_prefix;

                                        // Measure plain-text width for stable column tracking.
                                        let measured_w = ui.fonts_mut(|f| {
                                            f.layout_no_wrap(line.plain_prefix.clone(), font_id.clone(), Color32::WHITE).size().x
                                        });
                                        let cap_px = if self.prefix_align_max > 0 {
                                            ui.fonts_mut(|f| {
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
                                                for s in prefix_sections {
                                                    prefix_job.append(&s.text, 0.0, s.style.to_format(font_id.clone(), &render_theme));
                                                }
                                                if grouped {
                                                    ui.allocate_exact_size(egui::vec2(measured_w, ui.text_style_height(&TextStyle::Body)), egui::Sense::hover());
                                                } else {
                                                    ui.add(Label::new(prefix_job).extend());
                                                }
                                            }
                                        );
                                        if !self.prefix_suffix.is_empty() {
                                            ui.add(Label::new(egui::RichText::new(&self.prefix_suffix).font(font_id.clone()).color(text_muted)).selectable(!grouped));
                                        }

                                        let msg_col_width = ui.available_width();
                                        ui.vertical(|ui| {
                                            ui.set_min_width(msg_col_width);
                                            // Like `horizontal_wrapped`, but top-aligned and starting one text line
                                            // tall: `horizontal_wrapped` centers items in an interact_size.y-tall
                                            // row, which drops the text below the timestamp and prefix columns.
                                            let row_size = egui::vec2(ui.available_width(), ui.text_style_height(&TextStyle::Body));
                                            ui.allocate_ui_with_layout(row_size, egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true), |ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                for s in msg_sections {
                                                    if let Some(url) = &s.url {
                                                        // Use hover-only sense so the row's context_menu (which needs
                                                        // Sense::click on the frame) doesn't compete with link clicks.
                                                        // URL left-click is detected via raw input instead.
                                                        let link_style = s.style.to_format(font_id.clone(), &render_theme);
                                                        let mut link_format = link_style;
                                                        link_format.color = ui.visuals().hyperlink_color;
                                                        link_format.underline = egui::Stroke::new(1.0, ui.visuals().hyperlink_color);
                                                        let mut job = egui::text::LayoutJob::default();
                                                        job.append(&s.text, 0.0, link_format);
                                                        let link_resp = ui.add(egui::Label::new(job).sense(egui::Sense::hover()));
                                                        if link_resp.hovered() {
                                                            ui.ctx().output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                                                            row_hovered_url = Some(url.clone());
                                                        }
                                                        let url_for_click = url.clone();
                                                        let link_rect = link_resp.rect;
                                                        if ui.input(|i| {
                                                            i.pointer.primary_clicked()
                                                            && i.pointer.interact_pos().map_or(false, |p| link_rect.contains(p))
                                                        }) {
                                                            ui.ctx().open_url(egui::OpenUrl::new_tab(url_for_click));
                                                        }
                                                        if self.show_inline_images && Self::is_image_url(url) && is_safe_public_url(url) {
                                                            let is_expanded = self.image_expanded.contains(url);
                                                            let btn = if is_expanded { icons::IMAGE.to_string() } else { format!("{} preview", icons::IMAGE) };
                                                            if ui.small_button(btn).clicked() {
                                                                if is_expanded {
                                                                    self.image_expanded.remove(url);
                                                                } else {
                                                                    self.image_expanded.insert(url.clone());
                                                                    if !self.image_cache.contains_key(url) {
                                                                        self.image_cache.insert(url.clone(), ImageState::Loading);
                                                                        let tx = self.image_tx.clone();
                                                                        let url_owned = url.clone();
                                                                        let ctx2 = ctx.clone();
                                                                        tokio::spawn(async move {
                                                                            let result = fetch_image_bytes(&url_owned).await;
                                                                            let _ = tx.send((url_owned, result));
                                                                            ctx2.request_repaint();
                                                                        });
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        if self.show_link_previews && !Self::is_image_url(url) && is_safe_public_url(url) {
                                                            let is_expanded = self.preview_expanded.contains(url);
                                                            let btn = if is_expanded { icons::LINK.to_string() } else { format!("{} preview", icons::LINK) };
                                                            if ui.small_button(btn).clicked() {
                                                                if is_expanded {
                                                                    self.preview_expanded.remove(url);
                                                                } else {
                                                                    self.preview_expanded.insert(url.clone());
                                                                    if !self.preview_cache.contains_key(url) {
                                                                        self.preview_cache.insert(url.clone(), PreviewState::Loading);
                                                                        let tx = self.preview_tx.clone();
                                                                        let url_owned = url.clone();
                                                                        let ctx2 = ctx.clone();
                                                                        tokio::spawn(async move {
                                                                            let result = fetch_link_preview(url_owned.clone()).await;
                                                                            let _ = tx.send((url_owned, result));
                                                                            ctx2.request_repaint();
                                                                        });
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    } else if !self.emoji_rendering {
                                                        let mut job = LayoutJob::default();
                                                        job.append(&s.text, 0.0, s.style.to_format(font_id.clone(), &render_theme));
                                                        ui.add(Label::new(job).wrap());
                                                    } else {
                                                        let emoji_size = font_id.size + 2.0;
                                                        for span in crate::ui::emoji::split_emoji(&s.text) {
                                                            match span {
                                                                crate::ui::emoji::TextSpan::Text(t) => {
                                                                    let mut job = LayoutJob::default();
                                                                    job.append(&t, 0.0, s.style.to_format(font_id.clone(), &render_theme));
                                                                    ui.add(Label::new(job).wrap());
                                                                }
                                                                crate::ui::emoji::TextSpan::Emoji(e) => {
                                                                    let eurl = crate::ui::emoji::emoji_to_twemoji_url(&e);
                                                                    if !self.image_cache.contains_key(&eurl) {
                                                                        self.image_cache.insert(eurl.clone(), ImageState::Loading);
                                                                        let tx = self.image_tx.clone();
                                                                        let url_owned = eurl.clone();
                                                                        tokio::spawn(async move {
                                                                            let result = fetch_image_bytes(&url_owned).await;
                                                                            let _ = tx.send((url_owned, result));
                                                                        });
                                                                    }
                                                                    match self.image_cache.get(&eurl) {
                                                                        Some(ImageState::Loaded(texture)) => {
                                                                            ui.add(egui::Image::new((texture.id(), egui::Vec2::splat(emoji_size))));
                                                                        }
                                                                        _ => {
                                                                            let mut job = LayoutJob::default();
                                                                            job.append(&e, 0.0, s.style.to_format(font_id.clone(), &render_theme));
                                                                            ui.add(Label::new(job).wrap());
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
                                                                ui.add(egui::Image::new((texture.id(), orig * scale)).corner_radius(4.0));
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

                                                            let card = Frame::NONE
                                                                .fill(card_bg)
                                                                .corner_radius(CornerRadius::same(6))
                                                                .stroke(Stroke::new(1.0, border_color))
                                                                .inner_margin(Margin { left: 14, right: 12, top: 8, bottom: 8 })
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
                                                                            ui.add(egui::Image::new((texture.id(), orig * scale)).corner_radius(4.0));
                                                                        }
                                                                    }
                                                                });
                                                            let bar = egui::Rect::from_min_max(
                                                                card.response.rect.min,
                                                                egui::pos2(card.response.rect.min.x + 3.0, card.response.rect.max.y),
                                                            );
                                                            ui.painter().rect_filled(bar, CornerRadius::same(3), accent_color);
                                                        }
                                                    }
                                                    ui.add_space(4.0);
                                                }
                                            }
                                        }); // end vertical (message column)
                                    }); // end horizontal (full message row)
                                    }); // end highlight frame
                                    let plain_message = line.plain_message.clone();
                                    let plain_prefix = line.plain_prefix.clone();
                                    // Detect secondary (right) click via raw input — bypasses the
                                    // egui hit test so image/expand buttons still win primary clicks.
                                    let row_rect = row_resp.response.rect;
                                    if let Some((ts_rect, ts)) = hidden_timestamp {
                                        if ui.rect_contains_pointer(row_rect) {
                                            ui.painter().text(ts_rect.left_top(), egui::Align2::LEFT_TOP, ts, font_id.clone(), text_muted);
                                        }
                                    }
                                    if ui.input(|i| {
                                        i.pointer.button_clicked(egui::PointerButton::Secondary)
                                        && i.pointer.interact_pos().map_or(false, |p| row_rect.contains(p))
                                    }) {
                                        let click_pos = ui.input(|i| i.pointer.interact_pos().unwrap_or(row_rect.left_bottom()));
                                        self.ctx_menu_state = Some((click_pos, plain_message, plain_prefix, row_hovered_url.clone()));
                                        self.ctx_menu_just_opened = true;
                                    }
                                    }); // end push_id
                                }
                            }
                        });
                    }); // ScrollArea

                    if let Some(messages) = &current_buffer_messages {
                        if show_older_in_memory {
                            self.chat_view_start = chat_start.checked_sub(LOAD_MORE_LINES)
                                .map(|i| messages[i].id.clone());
                        } else if self.search_text.is_empty() && messages.len() - chat_start > 2 * INITIAL_LINES {
                            // Lines keep arriving while the view is open. At the bottom the
                            // older ones can go without moving anything on screen.
                            let at_bottom = chat_scroll.state.clamped_offset().y + chat_scroll.inner_rect.height()
                                >= chat_scroll.content_size.y - 2.0;
                            if at_bottom {
                                self.chat_view_start = Some(messages[messages.len() - INITIAL_LINES].id.clone());
                            }
                        }
                    }
                    } // else (non-fset)
                });
            } else {
                ui.centered_and_justified(|ui| { ui.label(egui::RichText::new("Select a buffer to start chatting").color(text_muted).size(16.0)); });
            }
        });

        // -----------------------------------------------------------------------
        // Row context menu (rendered as a Foreground Area to avoid egui hit-test
        // conflicts with the image/expand buttons in the Middle layer).
        // -----------------------------------------------------------------------
        let mut close_ctx_menu = false;
        if let Some((pos, ref msg, ref prefix, ref url)) = self.ctx_menu_state {
            let msg = msg.clone();
            let prefix = prefix.clone();
            let url = url.clone();
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                close_ctx_menu = true;
            }
            let area_resp = egui::Area::new(egui::Id::unique("row_context_menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(ctx, |ui| {
                    egui::Frame::menu(ui.style())
                        .fill(egui::Color32::from_rgba_unmultiplied(
                            ui.visuals().window_fill.r(),
                            ui.visuals().window_fill.g(),
                            ui.visuals().window_fill.b(),
                            242,
                        ))
                        .show(ui, |ui| {
                            ui.set_max_width(200.0);
                            if let Some(ref u) = url {
                                if ui.button("Open URL").clicked() {
                                    ctx.open_url(egui::OpenUrl::new_tab(u.clone()));
                                    close_ctx_menu = true;
                                }
                                if ui.button("Copy URL").clicked() {
                                    ctx.copy_text(u.clone());
                                    close_ctx_menu = true;
                                }
                                ui.separator();
                            }
                            if ui.button("Copy message").clicked() {
                                ctx.copy_text(msg.clone());
                                close_ctx_menu = true;
                            }
                            if !prefix.is_empty() {
                                if ui.button("Copy with sender").clicked() {
                                    ctx.copy_text(format!("<{}> {}", prefix, msg));
                                    close_ctx_menu = true;
                                }
                            }
                        });
                });
            // Don't close on the same frame we opened (avoids immediate dismiss
            // when a secondary click also registers as clicked_elsewhere).
            if !self.ctx_menu_just_opened && area_resp.response.clicked_elsewhere() {
                close_ctx_menu = true;
            }
        }
        self.ctx_menu_just_opened = false;
        if close_ctx_menu {
            self.ctx_menu_state = None;
        }

        self.show_quick_switcher(ctx, accent_color, text_muted);
        self.show_settings_window(ctx, accent_color, is_light);
        self.show_connections_window(ctx, accent_color, is_light);

        if let Some((buf_id, count)) = pending_load_more {
            // Show all of what arrives.
            self.chat_view_start = None;
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

#[cfg(test)]
mod tests {
    use super::WeeChatApp;

    fn line(prefix: &str, secs: i64) -> crate::relay::models::Line {
        let ts = chrono::DateTime::from_timestamp(1_790_000_000 + secs, 0).unwrap();
        crate::relay::models::Line::new(secs.to_string(), ts, prefix.to_string(), "hi".to_string(), true, false)
    }

    #[test]
    fn nick_styles_come_from_the_last_chat_line() {
        use super::nick_styles_in_chat;
        use crate::ui::ansi::AnsiColor;
        let ts = chrono::Utc::now();
        let line = |prefix: &str| crate::relay::models::Line::new("1".into(), ts, prefix.into(), "hi".into(), true, false);
        let lines = [
            line("\x1b[32m@\x1b[36malice"),
            line("\x1b[92m-->"),
            line("\x1b[33mbob"),
            line("\x1b[95malice"),
            line("\x1b[37m<\x1b[34mcarol\x1b[37m>"),
        ];
        let styles = nick_styles_in_chat(lines.iter());
        assert_eq!(styles.len(), 3);
        assert_eq!(styles["alice"].fg, AnsiColor::Indexed(13));
        assert_eq!(styles["bob"].fg, AnsiColor::Indexed(3));
        assert_eq!(styles["carol"].fg, AnsiColor::Indexed(4));
    }

    #[test]
    fn detects_highmon_buffer() {
        use super::is_highmon_buffer;
        assert!(is_highmon_buffer("relay/perl.highmon"));
        assert!(is_highmon_buffer("relay/python.highmon"));
        assert!(!is_highmon_buffer("libera/irc.libera.#highmon"));
        assert!(!is_highmon_buffer("relay/fset.fset"));
    }

    #[test]
    fn groups_consecutive_messages_from_same_nick() {
        use super::continues_group;
        let a = line("roniez", 0);
        assert!(!continues_group(None, &a), "first line starts a group");
        assert!(continues_group(Some(&a), &line("roniez", 60)));
        assert!(continues_group(Some(&a), &line("roniez", 300)), "exactly 5 minutes still groups");
        assert!(!continues_group(Some(&a), &line("roniez", 301)), "more than 5 minutes starts a new group");
        assert!(!continues_group(Some(&a), &line("user", 10)), "different nick");
        assert!(!continues_group(Some(&line("roniez", 60)), &a), "time going backwards");
    }

    #[test]
    fn system_lines_and_actions_never_group() {
        use super::continues_group;
        for prefix in ["-->", "<--", "--", "=!=", " *"] {
            assert!(!continues_group(Some(&line(prefix, 0)), &line(prefix, 10)), "{prefix:?}");
        }
    }

    #[test]
    fn image_urls_by_path_or_fragment_filename() {
        assert!(WeeChatApp::is_image_url("https://example.com/a/pic.PNG?size=2"));
        assert!(WeeChatApp::is_image_url("https://files.nixsec.se/f.php?h=18TGUOq5&p=1#Screenshot_2026-10-26_at_09.38.06.png"));
        assert!(WeeChatApp::is_image_url("https://host/f.php?h=x#dir/photo.jpeg"));
        assert!(!WeeChatApp::is_image_url("https://files.nixsec.se/f.php?h=18TGUOq5&p=1"));
        assert!(!WeeChatApp::is_image_url("https://example.com/page.html#section.png-notes"));
        assert!(!WeeChatApp::is_image_url("https://example.com/docs#install"));
    }

    // Network tests: run with `cargo test -- --ignored`.

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_and_decodes_image_from_host_requiring_user_agent() {
        // Wikimedia answers 403 to requests without a User-Agent.
        let url = "https://upload.wikimedia.org/wikipedia/commons/4/47/PNG_transparency_demonstration_1.png";
        let bytes = super::fetch_image_bytes(url).await.expect("fetch");
        let img = image::load_from_memory(&bytes).expect("decode");
        assert!(img.width() > 0 && img.height() > 0);
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn http_errors_are_reported_not_decoded() {
        let url = "https://upload.wikimedia.org/wikipedia/commons/does-not-exist-weechatrs.png";
        assert!(super::fetch_image_bytes(url).await.is_err());
    }
}
