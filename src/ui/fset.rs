//! The fset buffer (`/fset`) shown as a settings table.
//!
//! While the panel is open the app switches fset to its second format
//! (`fset.format.option2`) with a template of raw, separator-delimited fields, so
//! options, defaults, inherited values, ranges and descriptions can be read without
//! guessing at the terminal layout. The user's own values for those two options are
//! read first (through WeeChat's completion of `/set`) and put back when the panel
//! closes. A WeeChat that can't report them keeps its format and the panel falls back
//! to parsing fset's default one-line layout.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, Color32, FontFamily, FontId, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, TextFormat};
use regex::Regex;
use crate::relay::models::Line;
use crate::ui::app::WeeChatApp;
use crate::ui::icons;

/// Full name of fset's buffer.
const FSET_BUFFER: &str = "fset.fset";

/// Whether a buffer's full name (`{connection}/{weechat name}`) is fset's buffer.
pub fn is_fset_buffer(full_name: &str) -> bool {
    full_name.rsplit_once('/').map_or(full_name, |(_, name)| name) == FSET_BUFFER
}

/// Separates the fields of the machine-readable format (U+241E, never in option text).
const SEP: char = '\u{241E}';
/// Starts each line of the machine-readable format.
const MARKER: &str = "\u{241E}wrs\u{241E}";
/// fset variables in the machine-readable format, in order. The description goes last:
/// it is the only field that could plausibly contain anything.
const FIELDS: &[&str] = &[
    "__name", "__type_en", "__value", "__default_value", "__parent_value",
    "value_changed", "value_undef", "__min", "__max", "__string_values", "__description",
];

/// Template for `fset.format.option2`.
pub fn machine_format() -> &'static str {
    static FORMAT: OnceLock<String> = OnceLock::new();
    FORMAT.get_or_init(|| {
        let fields: Vec<String> = FIELDS.iter().map(|f| format!("${{{f}}}")).collect();
        format!("{MARKER}{}", fields.join(&SEP.to_string()))
    })
}

/// Shown for the template while it is the value of `fset.format.option2` (it contains
/// the separator, so it can't be parsed as a field).
const TEMPLATE_PLACEHOLDER: &str = "(WeeChatRS panel format)";

/// The two options the panel switches while it is open.
const FORMAT_OPTION2: &str = "fset.format.option2";
const FORMAT_NUMBER: &str = "fset.look.format_number";

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum FsetOptionType {
    Boolean,
    Integer,
    String,
    Color,
    Enum,
    Unknown(std::string::String),
}

impl FsetOptionType {
    fn from_str(s: &str) -> Self {
        match s {
            "boolean" => Self::Boolean,
            "integer" => Self::Integer,
            "string"  => Self::String,
            "color"   => Self::Color,
            "enum"    => Self::Enum,
            other     => Self::Unknown(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Boolean        => "bool",
            Self::Integer        => "int",
            Self::String         => "str",
            Self::Color          => "color",
            Self::Enum           => "enum",
            Self::Unknown(s)     => s.as_str(),
        }
    }

    pub fn type_color(&self, muted: Color32) -> Color32 {
        match self {
            Self::Boolean    => Color32::from_rgb(100, 200, 120),
            Self::Integer    => Color32::from_rgb(100, 160, 220),
            Self::Color      => Color32::from_rgb(220, 160, 80),
            Self::Enum       => Color32::from_rgb(180, 120, 220),
            Self::String     => muted,
            Self::Unknown(_) => muted,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FsetOption {
    pub name: std::string::String,
    pub option_type: FsetOptionType,
    /// Raw value; "null" when the option is unset and inherits `parent_value`.
    pub value: std::string::String,
    /// None when unknown (fset's default format doesn't show it).
    pub default_value: Option<std::string::String>,
    pub parent_value: Option<std::string::String>,
    /// Value differs from the default.
    pub is_changed: bool,
    pub is_null: bool,
    pub min: std::string::String,
    pub max: std::string::String,
    /// Allowed values of an enum.
    pub enum_values: Vec<std::string::String>,
    pub description: std::string::String,
}

impl FsetOption {
    /// Configuration file the option belongs to (`irc` for `irc.look.color_nicks`).
    pub fn file(&self) -> &str {
        self.name.split('.').next().unwrap_or("")
    }

    fn is_enum(&self) -> bool {
        self.option_type == FsetOptionType::Enum
            || (self.option_type == FsetOptionType::Integer && !self.enum_values.is_empty())
    }

    /// Value in effect: the parent's when the option is unset.
    pub fn effective_value(&self) -> &str {
        match (&self.parent_value, self.is_null) {
            (Some(parent), true) => parent,
            _ => &self.value,
        }
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a line of the machine-readable format.
pub fn parse_machine_line(plain: &str) -> Option<FsetOption> {
    let start = plain.find(MARKER)? + MARKER.len();
    let rest = plain[start..].replace(machine_format(), TEMPLATE_PLACEHOLDER);
    let f: Vec<&str> = rest.splitn(FIELDS.len(), SEP).collect();
    if f.len() < FIELDS.len() || f[0].is_empty() {
        return None;
    }
    let option_type = FsetOptionType::from_str(f[1]);
    let is_null = f[6] == "1";
    Some(FsetOption {
        name: f[0].to_string(),
        value: f[2].to_string(),
        default_value: Some(f[3].to_string()),
        parent_value: (is_null && !f[4].is_empty()).then(|| f[4].to_string()),
        is_changed: f[5] == "1",
        is_null,
        min: f[7].to_string(),
        max: f[8].to_string(),
        enum_values: if f[9].is_empty() { Vec::new() } else { f[9].split(',').map(str::to_string).collect() },
        description: f[10].trim_end().to_string(),
        option_type,
    })
}

/// Parse a line of fset's default format (`name  type  value`), used when the
/// machine-readable format couldn't be enabled. Defaults and descriptions are unknown.
pub fn parse_default_line(plain: &str) -> Option<FsetOption> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"^[*> ]?\s*([a-zA-Z][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_#-]+){2,})\s+(boolean|color|enum|integer|string)\s+(.*)$"
        ).unwrap()
    });
    let caps = re.captures(plain.trim())?;
    let option_type = FsetOptionType::from_str(&caps[2]);
    let raw = caps[3].trim();
    let (value, parent_value) = match raw.strip_prefix("null -> ") {
        Some(parent) => ("null".to_string(), Some(unquote(parent).to_string())),
        None => (unquote(raw).to_string(), None),
    };
    Some(FsetOption {
        name: caps[1].to_string(),
        is_null: parent_value.is_some() || value == "null",
        value,
        default_value: None,
        parent_value,
        is_changed: false,
        min: String::new(),
        max: String::new(),
        enum_values: Vec::new(),
        description: String::new(),
        option_type,
    })
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s)
}

/// Parse the fset buffer. Returns whether it uses the machine-readable format, and
/// the options in buffer order.
pub fn parse_fset_buffer(lines: &VecDeque<Line>) -> (bool, Vec<FsetOption>) {
    // Free-content lines are identified by their y; keep them in that order.
    let mut ordered: Vec<&Line> = lines.iter().collect();
    ordered.sort_by_key(|l| l.id.parse::<i64>().unwrap_or(i64::MAX));
    let structured = ordered.iter().any(|l| l.plain_message.contains(MARKER));
    let parse = if structured { parse_machine_line } else { parse_default_line };
    let options = ordered.iter().filter_map(|l| parse(&l.plain_message)).collect();
    (structured, options)
}

/// Argument for `/set <name> …`: strings are quoted, so that empty values and
/// leading/trailing spaces survive.
pub fn set_argument(option_type: &FsetOptionType, value: &str) -> std::string::String {
    if *option_type == FsetOptionType::String {
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value.to_string()
        } else {
            format!("\"{value}\"")
        }
    } else {
        value.trim().to_string()
    }
}

/// How to put an option back to its default, if that is known. `/unset` is only safe
/// when the default is null (the option inherits again): on options without one it
/// deletes the option (an alias, a plugin variable…), so a known default is set back
/// explicitly instead.
pub fn reset_action(opt: &FsetOption) -> Option<FsetAction> {
    match opt.default_value.as_deref() {
        Some("null") => Some(FsetAction::Unset { name: opt.name.clone() }),
        Some(default) => Some(FsetAction::Set { name: opt.name.clone(), arg: set_argument(&opt.option_type, default) }),
        None => None,
    }
}

/// The filter from fset's buffer title (`… | Filter: f:irc | Sort: …`).
pub fn filter_from_title(title: &str) -> Option<std::string::String> {
    let start = title.find("| Filter: ")? + "| Filter: ".len();
    let end = title[start..].find(" | ").map_or(title.len(), |i| start + i);
    Some(title[start..end].trim().to_string())
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// fset's format settings as they were before the panel replaced them.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedFormat {
    /// None restores the default (`/unset`).
    pub option2: Option<std::string::String>,
    pub format_number: std::string::String,
}

impl SavedFormat {
    /// From the current values reported by WeeChat (first word of `/set <option> `
    /// completion). A template of ours means an earlier session never restored it.
    pub fn from_current(option2: &str, format_number: &str) -> Self {
        let option2 = unquote(option2);
        if option2.contains(MARKER) {
            Self { option2: None, format_number: "1".to_string() }
        } else {
            Self { option2: Some(option2.to_string()), format_number: format_number.to_string() }
        }
    }

    /// Take a change to one of the format options; false if `action` is about another.
    fn absorb(&mut self, action: &FsetAction) -> bool {
        match action {
            FsetAction::Set { name, arg } if name == FORMAT_NUMBER => {
                if matches!(arg.as_str(), "1" | "2") {
                    self.format_number = arg.clone();
                }
                true
            }
            FsetAction::Set { name, arg } if name == FORMAT_OPTION2 => {
                self.option2 = Some(unquote(arg).to_string());
                true
            }
            FsetAction::Unset { name } if name == FORMAT_NUMBER => {
                self.format_number = "1".to_string();
                true
            }
            FsetAction::Unset { name } if name == FORMAT_OPTION2 => {
                self.option2 = None;
                true
            }
            _ => false,
        }
    }

    /// Show the values that will be restored instead of the panel's own.
    fn present(&self, opt: &mut FsetOption) {
        let value = match opt.name.as_str() {
            FORMAT_NUMBER => self.format_number.clone(),
            FORMAT_OPTION2 => match &self.option2 {
                Some(v) => v.clone(),
                None => opt.default_value.clone().unwrap_or_default(),
            },
            _ => return,
        };
        opt.is_changed = opt.default_value.as_deref() != Some(value.as_str());
        opt.value = value;
        opt.description = format!("{} (WeeChatRS uses fset's second format while this panel is open; this value is put back when you leave it.)", opt.description);
    }

    fn restore_commands(&self) -> Vec<std::string::String> {
        let option2 = match &self.option2 {
            Some(v) => format!("/mute /set fset.format.option2 \"{v}\""),
            None => "/mute /unset fset.format.option2".to_string(),
        };
        vec![format!("/mute /set fset.look.format_number {}", self.format_number), option2]
    }
}

impl Default for SavedFormat {
    /// fset's own defaults, restored when the user's values couldn't be read.
    fn default() -> Self {
        Self { option2: None, format_number: "1".to_string() }
    }
}

#[derive(Debug, Default)]
pub enum FsetSession {
    #[default]
    Idle,
    /// The machine-readable format is in use. `saved` holds fset's defaults until the
    /// user's values are read (`known`).
    Active {
        buffer_id: std::string::String,
        conn: std::string::String,
        saved: SavedFormat,
        known: bool,
        /// When the user switched away; the format is restored after `RESTORE_AFTER`.
        left_at: Option<Instant>,
    },
}

/// How long the panel's format is kept after leaving fset, so that switching back
/// doesn't make WeeChat redraw the whole buffer (twice) again.
const RESTORE_AFTER: Duration = Duration::from_secs(120);

/// While WeeChat redraws fset (a clear, then a line at a time) a shrinking list is
/// held back until the buffer has been quiet this long, so the table doesn't empty
/// and lose its scroll position.
const REDRAW_SETTLE: Duration = Duration::from_millis(300);

struct PendingRestore {
    conn: std::string::String,
    buffer_id: std::string::String,
    saved: SavedFormat,
}

struct ParsedCache {
    buffer_id: std::string::String,
    revision: u64,
    len: usize,
    structured: bool,
    options: Vec<FsetOption>,
}

pub enum FsetAction {
    /// `/set name <arg>`; `arg` is already quoted as needed.
    Set { name: std::string::String, arg: std::string::String },
    Unset { name: std::string::String },
    Filter(std::string::String),
}

#[derive(Default)]
pub struct FsetState {
    pub session: FsetSession,
    pending_restore: Option<PendingRestore>,
    /// Bumped whenever the relay touches the fset buffer; invalidates the parse cache.
    pub revision: u64,
    cache: Option<ParsedCache>,
    /// Inline editor in the table.
    editing: Option<std::string::String>,
    edit_value: std::string::String,
    /// Selected option, shown in the details pane.
    selected: Option<std::string::String>,
    /// Details pane editor and the option it was filled from.
    detail_for: Option<(std::string::String, std::string::String)>,
    detail_value: std::string::String,
    /// Values shown right after a change, until fset reprints the line.
    pending: HashMap<std::string::String, (std::string::String, Instant)>,
    filter: std::string::String,
    /// When the filter was last typed; it is sent after a short pause.
    filter_typed_at: Option<Instant>,
    files: BTreeSet<std::string::String>,
    quick: QuickFilters,
    /// Last revision seen, and when it changed (see `REDRAW_SETTLE`).
    seen_revision: u64,
    changed_at: Option<Instant>,
}

impl FsetState {
    /// Parsed options of the buffer, re-parsed only when it changed.
    fn options(&mut self, ctx: &egui::Context, buffer_id: &str, lines: &VecDeque<Line>) -> (bool, &[FsetOption]) {
        if self.seen_revision != self.revision {
            self.seen_revision = self.revision;
            self.changed_at = Some(Instant::now());
        }
        let fresh = self.cache.as_ref().is_some_and(|c| {
            c.buffer_id == buffer_id && c.revision == self.revision && c.len == lines.len()
        });
        if !fresh {
            let (structured, options) = parse_fset_buffer(lines);
            let shrinking = self.cache.as_ref()
                .is_some_and(|c| c.buffer_id == buffer_id && options.len() < c.options.len());
            let busy = self.changed_at.is_some_and(|at| at.elapsed() < REDRAW_SETTLE);
            if shrinking && busy {
                // Mid-redraw: keep the current rows and look again once it settles.
                ctx.request_repaint_after(REDRAW_SETTLE);
                let cache = self.cache.as_ref().unwrap();
                return (cache.structured, &cache.options);
            }
            self.files.extend(options.iter().map(|o| o.file().to_string()));
            self.pending.retain(|name, (value, at)| {
                at.elapsed() < Duration::from_secs(4)
                    && options.iter().find(|o| &o.name == name).is_none_or(|o| o.effective_value() != value)
            });
            self.cache = Some(ParsedCache {
                buffer_id: buffer_id.to_string(),
                revision: self.revision,
                len: lines.len(),
                structured,
                options,
            });
        }
        let cache = self.cache.as_ref().unwrap();
        (cache.structured, &cache.options)
    }
}

// ---------------------------------------------------------------------------
// Session: switching fset's format while the panel is open
// ---------------------------------------------------------------------------

const QUERY_OPTION2: &str = "fset:option2";
const QUERY_FORMAT_NUMBER: &str = "fset:format_number";

impl WeeChatApp {
    /// Enable the machine-readable format while an fset buffer is shown on a WeeChat
    /// connection, and restore the user's format once it no longer is.
    pub(crate) fn update_fset_session(&mut self, ctx: &egui::Context) {
        let closing = ctx.input(|i| i.viewport().close_requested());
        let want = self.selected_buffer_id.clone()
            .filter(|_| !closing)
            .filter(|id| self.buffer_by_id(id).is_some_and(|b| is_fset_buffer(&b.full_name)))
            .and_then(|id| {
                let conn = self.connections.iter().find(|c| id.starts_with(&format!("{}/", c.prefix)))?;
                (conn.client.supports_completion() && conn.client.is_connected())
                    .then(|| (id.clone(), conn.prefix.clone()))
            });

        if let FsetSession::Active { buffer_id, conn, .. } = &self.fset_state.session {
            let here = want.as_ref().is_some_and(|(id, _)| id == buffer_id);
            let still_open = self.buffer_by_id(buffer_id).is_some()
                && self.connections.iter().any(|c| &c.prefix == conn && c.client.is_connected());
            // Showing another connection's fset needs the format there instead.
            let other_fset = want.is_some() && !here;
            if let FsetSession::Active { left_at, .. } = &mut self.fset_state.session {
                if here {
                    *left_at = None;
                } else {
                    let left = *left_at.get_or_insert_with(Instant::now);
                    if closing || !still_open || other_fset || left.elapsed() >= RESTORE_AFTER {
                        if let FsetSession::Active { buffer_id, conn, saved, .. } = std::mem::take(&mut self.fset_state.session) {
                            self.fset_state.pending_restore = Some(PendingRestore { conn, buffer_id, saved });
                        }
                    }
                }
            }
        }

        if let Some(restore) = self.fset_state.pending_restore.take() {
            if !self.send_fset_commands(&restore.conn, &restore.buffer_id, &restore.saved.restore_commands()) {
                // Not connected: try again once it is.
                self.fset_state.pending_restore = Some(restore);
            }
        }

        match &mut self.fset_state.session {
            FsetSession::Idle => {
                let Some((buffer_id, conn)) = want else { return };
                let queries = [(QUERY_OPTION2, "fset.format.option2"), (QUERY_FORMAT_NUMBER, "fset.look.format_number")];
                for (tag, _) in queries {
                    self.completion_replies.remove(&format!("{conn}|{tag}"));
                }
                let Some((client, raw_id)) = self.client_for_buffer(&buffer_id) else { return };
                // WeeChat handles requests in order: the completions report the values from
                // before the switch, and the lines come back in the new format. One round trip.
                for (tag, option) in queries {
                    let text = format!("/set {option} ");
                    client.complete(&raw_id, &text, text.chars().count(), tag);
                }
                client.send_message(&raw_id, &format!("/mute /set fset.format.option2 \"{}\"", machine_format()));
                client.send_message(&raw_id, "/mute /set fset.look.format_number 2");
                client.fetch_lines(&raw_id, crate::ui::app::MAX_STORED_LINES);
                self.fset_state.session = FsetSession::Active { buffer_id, conn, saved: SavedFormat::default(), known: false, left_at: None };
            }
            FsetSession::Active { conn, saved, known: known @ false, .. } => {
                let keys = [format!("{conn}|{QUERY_OPTION2}"), format!("{conn}|{QUERY_FORMAT_NUMBER}")];
                if let (Some(option2), Some(number)) = (self.completion_replies.get(&keys[0]), self.completion_replies.get(&keys[1])) {
                    // A failed read keeps fset's defaults for the restore.
                    if let (true, Some(o), true, Some(n)) = (option2.ok, option2.list.first(), number.ok, number.list.first()) {
                        *saved = SavedFormat::from_current(o, n);
                    }
                    *known = true;
                    for key in &keys {
                        self.completion_replies.remove(key);
                    }
                }
            }
            _ => {}
        }
    }

    /// Send commands on `conn`, through `buffer_id` if it still exists or any other
    /// buffer of the connection. False when the connection isn't up.
    fn send_fset_commands(&self, conn: &str, buffer_id: &str, commands: &[std::string::String]) -> bool {
        let Some(handle) = self.connections.iter().find(|c| c.prefix == conn) else { return true };
        if !handle.client.is_connected() {
            return false;
        }
        let prefix = format!("{conn}/");
        let target = self.buffer_by_id(buffer_id).map(|b| b.id.clone())
            .or_else(|| self.buffers.iter().find(|b| b.id.starts_with(&prefix) && b.full_name.ends_with("/core.weechat")).map(|b| b.id.clone()));
        let Some(raw_id) = target.as_deref().and_then(|id| id.strip_prefix(&prefix)) else { return true };
        for command in commands {
            handle.client.send_message(raw_id, command);
        }
        true
    }

    /// Carry out what the panel asked for.
    pub(crate) fn apply_fset_actions(&mut self, buffer_id: &str, actions: Vec<FsetAction>) {
        // While the panel owns fset's format, changes to it go to the values restored
        // on leaving; sending them would switch the format away under the panel.
        let actions: Vec<FsetAction> = match &mut self.fset_state.session {
            FsetSession::Active { saved, known, .. } => {
                let before = actions.len();
                let actions: Vec<FsetAction> = actions.into_iter().filter(|a| !saved.absorb(a)).collect();
                // A change made before the user's values arrived wins over them.
                if actions.len() != before {
                    *known = true;
                }
                actions
            }
            _ => actions,
        };
        let commands: Vec<std::string::String> = actions.into_iter().map(|action| match action {
            FsetAction::Set { name, arg } => format!("/set {name} {arg}"),
            FsetAction::Unset { name } => format!("/unset {name}"),
            FsetAction::Filter(filter) => format!("/fset {}", if filter.trim().is_empty() { "*" } else { filter.trim() }),
        }).collect();
        if let Some((client, raw_id)) = self.client_for_buffer(buffer_id) {
            for command in &commands {
                client.send_message(&raw_id, command);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Panel renderer
// ---------------------------------------------------------------------------

pub struct FsetColors {
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub border: Color32,
}

/// Type quick filters: label, type, tooltip.
const TYPE_FILTERS: &[(&str, FsetOptionType, &str)] = &[
    ("bool", FsetOptionType::Boolean, "Booleans"),
    ("int", FsetOptionType::Integer, "Integers"),
    ("str", FsetOptionType::String, "Strings"),
    ("color", FsetOptionType::Color, "Colors"),
    ("enum", FsetOptionType::Enum, "Enums"),
];

/// Quick filters, applied here on top of fset's filter so they combine with the
/// text in the filter field (fset itself takes a single filter).
#[derive(Default)]
struct QuickFilters {
    modified: bool,
    option_type: Option<FsetOptionType>,
    file: Option<std::string::String>,
}

impl QuickFilters {
    fn is_empty(&self) -> bool {
        !self.modified && self.option_type.is_none() && self.file.is_none()
    }

    fn matches(&self, opt: &FsetOption) -> bool {
        (!self.modified || opt.is_changed)
            && self.option_type.as_ref().is_none_or(|t| *t == opt.option_type)
            && self.file.as_deref().is_none_or(|f| f == opt.file())
    }
}

/// Render the fset panel. Returns the commands the caller should send.
pub fn show_fset_panel(
    ui: &mut egui::Ui,
    buffer_id: &str,
    lines: &VecDeque<Line>,
    title: &str,
    state: &mut FsetState,
    font_size: f32,
    colors: &FsetColors,
) -> Vec<FsetAction> {
    let mut actions = Vec::new();

    // Follow the filter fset reports, unless the user is typing one.
    if state.filter_typed_at.is_none() {
        let plain_title: std::string::String = crate::ui::ansi::ANSIParser::parse(title).iter().map(|s| s.text.as_str()).collect();
        if let Some(filter) = filter_from_title(&plain_title) {
            state.filter = if filter == "*" { std::string::String::new() } else { filter };
        }
    }
    if let Some(at) = state.filter_typed_at {
        if at.elapsed() >= Duration::from_millis(400) {
            state.filter_typed_at = None;
            actions.push(FsetAction::Filter(state.filter.clone()));
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    let font_id = FontId::new(font_size, crate::ui::fonts::chat_family());
    let small_id = FontId::new((font_size - 2.0).max(10.0), FontFamily::Proportional);
    let row_height = font_size + 10.0;

    // The table borrows the options; actions on them are collected and applied after.
    let (structured, _) = state.options(ui.ctx(), buffer_id, lines);
    let mut options: Vec<FsetOption> = state.cache.as_ref().map(|c| c.options.clone()).unwrap_or_default();
    if let FsetSession::Active { saved, .. } = &state.session {
        for opt in options.iter_mut().filter(|o| o.name == FORMAT_NUMBER || o.name == FORMAT_OPTION2) {
            saved.present(opt);
        }
    }

    // -----------------------------------------------------------------------
    // Toolbar: filter, quick filters, file, counts
    // -----------------------------------------------------------------------
    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 8, bottom: 4 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(colors.muted));
            let edit = TextEdit::singleline(&mut state.filter)
                .hint_text("Filter: name, f:irc, t:bool, d (changed), =value, c:condition")
                .desired_width((ui.available_width() * 0.45).clamp(180.0, 420.0));
            let res = ui.add(edit);
            if res.changed() {
                state.filter_typed_at = Some(Instant::now());
            }
            if res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                state.filter_typed_at = None;
                actions.push(FsetAction::Filter(state.filter.clone()));
            }
            res.on_hover_text(
                "fset filter syntax:\n\
                 xxx  name contains xxx\n\
                 f:xxx  configuration file (f:irc)\n\
                 t:xxx  type (bool, int, str, col, enum)\n\
                 d  changed options; d:xxx changed and name contains xxx\n\
                 d=xxx / d==xxx  changed, value contains / is xxx\n\
                 =xxx / ==xxx  value contains / is xxx\n\
                 h=xxx  description contains xxx\n\
                 c:condition  evaluated condition",
            );

            ui.add_space(6.0);
            let quick = &mut state.quick;
            if ui.selectable_label(quick.is_empty(), RichText::new("All").small()).on_hover_text("Clear the quick filters").clicked() {
                *quick = QuickFilters::default();
            }
            if ui.selectable_label(quick.modified, RichText::new("Modified").small()).on_hover_text("Only options changed from their default").clicked() {
                quick.modified = !quick.modified;
            }
            for (label, option_type, hint) in TYPE_FILTERS {
                let selected = quick.option_type.as_ref() == Some(option_type);
                if ui.selectable_label(selected, RichText::new(*label).small()).on_hover_text(*hint).clicked() {
                    quick.option_type = if selected { None } else { Some(option_type.clone()) };
                }
            }

            egui::ComboBox::from_id_salt("fset_file_filter")
                .selected_text(RichText::new(quick.file.as_deref().unwrap_or("All files")).small())
                .height(320.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(quick.file.is_none(), "All files").clicked() {
                        quick.file = None;
                    }
                    for file in &state.files {
                        if ui.selectable_label(quick.file.as_deref() == Some(file.as_str()), file).clicked() {
                            quick.file = Some(file.clone());
                        }
                    }
                });

        });
    });

    let total = options.len();
    options.retain(|o| state.quick.matches(o));
    let changed = options.iter().filter(|o| o.is_changed).count();

    // -----------------------------------------------------------------------
    // Layout: table above, details of the selected option below
    // -----------------------------------------------------------------------
    let selected = state.selected.as_ref().and_then(|n| options.iter().find(|o| &o.name == n)).cloned();
    let details_height = if selected.is_some() { font_size * 6.0 + 72.0 } else { 0.0 };
    let table_height = (ui.available_height() - details_height).max(row_height * 3.0);

    let side = 16.0;
    let avail = (ui.available_width() - side * 2.0).max(420.0);
    let col_reset = 28.0;
    let col_type = 44.0;
    let col_default = if structured { (avail * 0.22).clamp(90.0, 260.0) } else { 0.0 };
    let col_value = (avail * 0.34).clamp(140.0, 420.0);
    let col_name = (avail - col_reset - col_type - col_default - col_value - 24.0).max(140.0);

    let row_width = col_name + col_type + col_value + col_default + col_reset + 24.0;

    // Header
    {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_height), Sense::hover());
        let mut x = rect.left() + side + 6.0;
        for (label, width) in [("Option", col_name), ("Type", col_type), ("Value", col_value), ("Default", col_default)] {
            if width > 0.0 {
                ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, small_id.clone(), colors.muted);
            }
            x += width + 6.0;
        }
        ui.painter().hline(rect.x_range().shrink(side), rect.bottom(), Stroke::new(1.0, colors.border));

        // Counts at the right end of the header.
        let mut counts = LayoutJob::default();
        let fmt = |color| TextFormat::simple(small_id.clone(), color);
        if !structured && !options.is_empty() {
            counts.append(&format!("{} ", icons::WARNING), 0.0, fmt(colors.muted));
        }
        let shown = if options.len() == total { format!("{total} options") } else { format!("{} of {total} options", options.len()) };
        counts.append(&shown, 0.0, fmt(colors.muted));
        if changed > 0 {
            counts.append(" · ", 0.0, fmt(colors.muted));
            counts.append(&format!("{changed} modified"), 0.0, fmt(colors.accent));
        }
        let galley = ui.painter().layout_job(counts);
        let pos = egui::pos2(rect.right() - side - galley.size().x, rect.center().y - galley.size().y / 2.0);
        let counts_rect = Rect::from_min_size(pos, galley.size());
        ui.painter().galley(pos, galley, colors.muted);
        if !structured && !options.is_empty() {
            ui.interact(counts_rect, ui.scope_id().with("fset_counts"), Sense::hover())
                .on_hover_text("Defaults and descriptions are unavailable: this WeeChat can't report fset's settings, so its own layout is shown.");
        }
    }

    if options.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let text = if lines.is_empty() { "Loading options…" } else { "No options match the filter" };
            ui.label(RichText::new(text).color(colors.muted));
        });
        return actions;
    }

    ScrollArea::vertical()
        .id_salt("fset_table")
        .auto_shrink([false, false])
        .max_height(table_height)
        .show_rows(ui, row_height, options.len(), |ui, range| {
            for opt in &options[range] {
                let (row_rect, row_resp) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(row_width + side * 2.0), row_height), Sense::click());
                let is_selected = state.selected.as_deref() == Some(opt.name.as_str());
                let inner = row_rect.shrink2(egui::vec2(side, 0.0));
                if is_selected {
                    ui.painter().rect_filled(inner, 4.0, colors.accent.gamma_multiply(0.16));
                } else if row_resp.hovered() {
                    ui.painter().rect_filled(inner, 4.0, colors.text.gamma_multiply(0.05));
                }
                if opt.is_changed {
                    let bar = Rect::from_min_size(inner.left_top() + egui::vec2(0.0, 4.0), egui::vec2(2.0, row_height - 8.0));
                    ui.painter().rect_filled(bar, 1.0, colors.accent);
                }
                if row_resp.clicked() {
                    state.selected = Some(opt.name.clone());
                }

                let mut x = inner.left() + 6.0;
                let mut cell = |width: f32| {
                    let r = Rect::from_min_size(egui::pos2(x, row_rect.top()), egui::vec2(width, row_height));
                    x += width + 6.0;
                    r
                };
                let name_rect = cell(col_name);
                let type_rect = cell(col_type);
                let value_rect = cell(col_value);
                let default_rect = cell(col_default);
                let reset_rect = cell(col_reset);

                paint_text(ui, name_rect, &opt.name, font_id.clone(), if opt.is_changed { colors.accent } else { colors.text });
                paint_text(ui, type_rect, opt.option_type.label(), small_id.clone(), opt.option_type.type_color(colors.muted));
                if col_default > 0.0 {
                    if let Some(default) = &opt.default_value {
                        paint_text(ui, default_rect, &display_value(&opt.option_type, default), small_id.clone(), colors.muted);
                    }
                }

                value_cell(ui, value_rect, opt, state, &font_id, colors, &mut actions);

                if opt.is_changed {
                    if let Some(reset) = reset_action(opt) {
                        let hint = if matches!(reset, FsetAction::Unset { .. }) { "Reset (inherit again)" } else { "Reset to default" };
                        let btn = egui::Button::new(RichText::new(icons::ARROW_U_UP_LEFT).color(colors.muted)).frame(false);
                        if ui.put(reset_rect, btn).on_hover_text(hint).clicked() {
                            actions.push(reset);
                        }
                    }
                }

                if !opt.description.is_empty() {
                    row_resp.on_hover_ui_at_pointer(|ui| {
                        ui.set_max_width(420.0);
                        ui.label(RichText::new(&opt.name).strong());
                        ui.label(&opt.description);
                    });
                }
            }
        });

    if let Some(opt) = selected {
        details_pane(ui, &opt, state, &font_id, &small_id, colors, &mut actions);
    }

    // Show new values immediately; they're confirmed when fset reprints the line.
    for action in &actions {
        if let FsetAction::Set { name, arg } = action {
            if arg != "toggle" {
                state.pending.insert(name.clone(), (unquote(arg).to_string(), Instant::now()));
            }
        }
    }

    actions
}

/// Strings are shown quoted, like fset does, so empty values and spaces are visible.
fn display_value(option_type: &FsetOptionType, value: &str) -> std::string::String {
    if *option_type == FsetOptionType::String && value != "null" {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

/// Paint one line of text, cut with an ellipsis to fit `rect`.
fn paint_text(ui: &egui::Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat::simple(font, color));
    job.wrap = TextWrapping { max_width: rect.width(), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
}

/// The value column: a switch for booleans, a menu for enums, click-to-edit otherwise.
fn value_cell(
    ui: &mut egui::Ui,
    rect: Rect,
    opt: &FsetOption,
    state: &mut FsetState,
    font_id: &FontId,
    colors: &FsetColors,
    actions: &mut Vec<FsetAction>,
) {
    let pending = state.pending.get(&opt.name).map(|(v, _)| v.clone());
    let value = pending.clone().unwrap_or_else(|| opt.effective_value().to_string());
    let inherited = opt.is_null && pending.is_none();
    let color = if inherited { colors.muted } else if opt.is_changed || pending.is_some() { colors.accent } else { colors.text };

    if state.editing.as_deref() == Some(opt.name.as_str()) {
        let res = ui.put(rect.shrink2(egui::vec2(0.0, 2.0)), TextEdit::singleline(&mut state.edit_value).font(font_id.clone()));
        if !res.has_focus() && !res.lost_focus() {
            res.request_focus();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.editing = None;
        } else if res.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(FsetAction::Set { name: opt.name.clone(), arg: set_argument(&opt.option_type, &state.edit_value) });
            }
            state.editing = None;
        }
        return;
    }

    match opt.option_type {
        FsetOptionType::Boolean => {
            let mut on = value == "on";
            let label = RichText::new(if inherited { format!("{value} (inherited)") } else { value.clone() }).color(color);
            let res = ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                ui.add(egui::Checkbox::new(&mut on, label))
            }).inner;
            if res.changed() {
                actions.push(FsetAction::Set { name: opt.name.clone(), arg: if on { "on" } else { "off" }.to_string() });
            }
        }
        _ if opt.is_enum() => {
            let mut chosen: Option<std::string::String> = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                egui::ComboBox::from_id_salt(("fset_enum", &opt.name))
                    .selected_text(RichText::new(&value).color(color))
                    .width(rect.width().min(220.0))
                    .show_ui(ui, |ui| {
                        for v in &opt.enum_values {
                            if ui.selectable_label(*v == value, v).clicked() {
                                chosen = Some(v.clone());
                            }
                        }
                    });
            });
            if let Some(v) = chosen.filter(|v| *v != value) {
                actions.push(FsetAction::Set { name: opt.name.clone(), arg: v });
            }
        }
        _ => {
            let res = ui.interact(rect, ui.scope_id().with(("fset_value", &opt.name)), Sense::click());
            let shown = if inherited { format!("{} (inherited)", display_value(&opt.option_type, &value)) } else { display_value(&opt.option_type, &value) };
            paint_text(ui, rect, &shown, font_id.clone(), color);
            if res.clicked() {
                state.editing = Some(opt.name.clone());
                state.edit_value = if inherited { std::string::String::new() } else { value };
                state.selected = Some(opt.name.clone());
            }
            res.on_hover_text("Click to edit");
        }
    }
}

/// Details of the selected option, with a full-width editor.
fn details_pane(
    ui: &mut egui::Ui,
    opt: &FsetOption,
    state: &mut FsetState,
    font_id: &FontId,
    small_id: &FontId,
    colors: &FsetColors,
    actions: &mut Vec<FsetAction>,
) {
    // Refill the editor when the selection or the value changes underneath it.
    let key = (opt.name.clone(), opt.value.clone());
    if state.detail_for.as_ref() != Some(&key) {
        state.detail_value = if opt.is_null { std::string::String::new() } else { opt.value.clone() };
        state.detail_for = Some(key);
    }

    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range().shrink(16.0), rect.top(), Stroke::new(1.0, colors.border));
    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 8, bottom: 8 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&opt.name).font(font_id.clone()).strong().color(colors.text));
            ui.label(RichText::new(opt.option_type.label()).small().color(opt.option_type.type_color(colors.muted)));
            if opt.is_changed {
                ui.label(RichText::new("modified").small().color(colors.accent));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new(icons::X).color(colors.muted)).frame(false)).on_hover_text("Close").clicked() {
                    state.selected = None;
                }
            });
        });
        let description = if opt.description.is_empty() { "No description." } else { opt.description.as_str() };
        ui.add(egui::Label::new(RichText::new(description).color(colors.muted)).truncate()).on_hover_text(description);

        ui.horizontal(|ui| {
            let hint = match (&opt.parent_value, opt.is_null) {
                (Some(parent), true) => format!("inherits {}", display_value(&opt.option_type, parent)),
                _ => std::string::String::new(),
            };
            let res = ui.add(
                TextEdit::singleline(&mut state.detail_value)
                    .font(font_id.clone())
                    .hint_text(hint)
                    .desired_width((ui.available_width() - 150.0).max(160.0)),
            );
            let submit = res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Set").clicked() || submit {
                actions.push(FsetAction::Set { name: opt.name.clone(), arg: set_argument(&opt.option_type, &state.detail_value) });
            }
            let reset = reset_action(opt).filter(|_| opt.is_changed);
            let hint = if opt.default_value.is_none() { "Default unknown" } else { "Back to the default (or inherit again)" };
            if ui.add_enabled(reset.is_some(), egui::Button::new("Reset")).on_hover_text(hint).clicked() {
                actions.extend(reset);
            }
        });

        let mut facts: Vec<std::string::String> = Vec::new();
        if let Some(default) = &opt.default_value {
            facts.push(format!("Default: {}", display_value(&opt.option_type, default)));
        }
        if opt.is_enum() {
            facts.push(format!("Values: {}", opt.enum_values.join(", ")));
        } else if opt.option_type == FsetOptionType::Integer && !opt.min.is_empty() {
            facts.push(format!("Range: {} – {}", opt.min, opt.max));
        } else if opt.option_type == FsetOptionType::String && opt.max.parse::<i64>().is_ok_and(|m| m > 0) {
            facts.push(format!("Max length: {}", opt.max));
        }
        if let (Some(parent), true) = (&opt.parent_value, opt.is_null) {
            facts.push(format!("Inherited: {}", display_value(&opt.option_type, parent)));
        }
        ui.label(RichText::new(facts.join("   ·   ")).font(small_id.clone()).color(colors.muted));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine_line(fields: &[&str]) -> std::string::String {
        format!("{MARKER}{}", fields.join(&SEP.to_string()))
    }

    #[test]
    fn template_lists_every_field() {
        let t = machine_format();
        assert!(t.starts_with(MARKER));
        assert_eq!(t.matches(SEP).count(), 2 + FIELDS.len() - 1);
        assert!(t.contains("${__default_value}") && t.contains("${value_undef}"));
    }

    #[test]
    fn parses_machine_line() {
        let o = parse_machine_line(&machine_line(&[
            "irc.look.color_nicks_in_names", "boolean", "on", "off", "", "1", "0", "", "", "", "use nick color in /names",
        ])).unwrap();
        assert_eq!(o.name, "irc.look.color_nicks_in_names");
        assert_eq!(o.option_type, FsetOptionType::Boolean);
        assert_eq!((o.value.as_str(), o.default_value.as_deref()), ("on", Some("off")));
        assert!(o.is_changed && !o.is_null);
        assert_eq!(o.file(), "irc");
        assert_eq!(o.description, "use nick color in /names");
    }

    #[test]
    fn parses_inherited_enum_and_strings_with_spaces() {
        let o = parse_machine_line(&machine_line(&[
            "irc.server.libera.sasl_mechanism", "enum", "null", "null", "plain", "0", "1", "", "", "plain,scram-sha-1,external", "",
        ])).unwrap();
        assert!(o.is_null && !o.is_changed);
        assert_eq!(o.effective_value(), "plain");
        assert_eq!(o.enum_values, ["plain", "scram-sha-1", "external"]);
        assert!(o.is_enum());

        let s = parse_machine_line(&machine_line(&[
            "weechat.look.prefix_suffix", "string", " | ", "|", "", "1", "0", "0", "0", "", "desc ␞ odd",
        ])).unwrap();
        assert_eq!(s.value, " | ");
        assert_eq!(s.description, "desc ␞ odd", "the description may contain anything");
    }

    #[test]
    fn falls_back_to_the_default_format() {
        let o = parse_default_line("  irc.server.libera.anti_flood  integer  null -> 2000").unwrap();
        assert!(o.is_null);
        assert_eq!(o.parent_value.as_deref(), Some("2000"));
        assert_eq!(o.default_value, None);

        let s = parse_default_line("*irc.look.nick_prefix  string  \"<\"").unwrap();
        assert_eq!(s.value, "<");
        assert!(!s.is_changed, "a * marks the option for bulk actions, it doesn't mean changed");
        assert!(parse_default_line("some other text").is_none());
    }

    #[test]
    fn buffer_parse_uses_line_order_and_prefers_machine_format() {
        let ts = chrono::Utc::now();
        let mk = |id: &str, msg: std::string::String| Line::new(id.into(), ts, std::string::String::new(), msg, true, false);
        let mut lines = VecDeque::new();
        lines.push_back(mk("1", machine_line(&["b.c.d", "integer", "2", "1", "", "1", "0", "0", "9", "", ""])));
        lines.push_back(mk("0", machine_line(&["a.b.c", "string", "x", "x", "", "0", "0", "0", "0", "", ""])));
        lines.push_back(mk("2", "  a.b.c  string  \"x\"".into()));
        let (structured, opts) = parse_fset_buffer(&lines);
        assert!(structured);
        assert_eq!(opts.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["a.b.c", "b.c.d"]);
    }

    #[test]
    fn reset_never_unsets_an_option_with_a_default() {
        let mut o = parse_machine_line(&machine_line(&[
            "alias.cmd.beep", "string", "print ", "print -beep", "", "1", "0", "0", "0", "", "",
        ])).unwrap();
        assert!(matches!(reset_action(&o), Some(FsetAction::Set { ref arg, .. }) if arg == "\"print -beep\""));
        o.default_value = Some("null".into());
        assert!(matches!(reset_action(&o), Some(FsetAction::Unset { .. })));
        o.default_value = None;
        assert!(reset_action(&o).is_none(), "unknown default: no reset");
    }

    #[test]
    fn set_arguments_quote_strings_only() {
        assert_eq!(set_argument(&FsetOptionType::String, "a b"), "\"a b\"");
        assert_eq!(set_argument(&FsetOptionType::String, ""), "\"\"");
        assert_eq!(set_argument(&FsetOptionType::String, "\"kept\""), "\"kept\"");
        assert_eq!(set_argument(&FsetOptionType::Integer, " 42 "), "42");
    }

    #[test]
    fn saved_format_restores_user_values_or_defaults() {
        let user = SavedFormat::from_current("\"${name} ${value}\"", "2");
        assert_eq!(user.restore_commands(), [
            "/mute /set fset.look.format_number 2",
            "/mute /set fset.format.option2 \"${name} ${value}\"",
        ]);
        // Left behind by a session that never restored (crash): back to defaults.
        let stale = SavedFormat::from_current(&format!("\"{}\"", machine_format()), "2");
        assert_eq!(stale, SavedFormat { option2: None, format_number: "1".into() });
        assert_eq!(stale.restore_commands()[1], "/mute /unset fset.format.option2");
    }

    #[test]
    fn detects_fset_buffer_by_exact_name() {
        assert!(is_fset_buffer("myrelay/fset.fset"));
        assert!(is_fset_buffer("fset.fset"));
        assert!(!is_fset_buffer("libera/irc.libera.#fset"));
        assert!(!is_fset_buffer("myrelay/python.fsetx"));
    }

    #[test]
    fn template_as_a_value_does_not_break_the_line() {
        let line = machine_line(&[
            "fset.format.option2", "string", machine_format(), "${marked} ${name}", "", "1", "0", "0", "0", "", "second format",
        ]);
        let o = parse_machine_line(&line).unwrap();
        assert_eq!(o.value, TEMPLATE_PLACEHOLDER);
        assert_eq!(o.default_value.as_deref(), Some("${marked} ${name}"));
        assert_eq!(o.description, "second format");
    }

    #[test]
    fn format_options_change_the_restored_values_while_active() {
        let mut saved = SavedFormat { option2: None, format_number: "2".into() };
        assert!(saved.absorb(&FsetAction::Set { name: FORMAT_NUMBER.into(), arg: "1".into() }));
        assert_eq!(saved.format_number, "1");
        assert!(saved.absorb(&FsetAction::Set { name: FORMAT_OPTION2.into(), arg: "\"${name}\"".into() }));
        assert_eq!(saved.option2.as_deref(), Some("${name}"));
        assert!(saved.absorb(&FsetAction::Unset { name: FORMAT_OPTION2.into() }));
        assert_eq!(saved.option2, None);
        assert!(!saved.absorb(&FsetAction::Set { name: "irc.look.x".into(), arg: "1".into() }));

        let mut opt = parse_machine_line(&machine_line(&[
            FORMAT_NUMBER, "integer", "2", "1", "", "1", "0", "1", "2", "", "number of format",
        ])).unwrap();
        saved.present(&mut opt);
        assert_eq!(opt.value, "1");
        assert!(!opt.is_changed, "1 is the default");
    }

    #[test]
    fn quick_filters_combine() {
        let opt = |name: &str, t: &str, changed: &str| parse_machine_line(&machine_line(&[
            name, t, "1", "0", "", changed, "0", "", "", "", "",
        ])).unwrap();
        let a = opt("irc.look.a", "boolean", "1");
        let b = opt("irc.look.b", "integer", "1");
        let c = opt("weechat.look.c", "boolean", "0");
        let mut q = QuickFilters::default();
        assert!([&a, &b, &c].iter().all(|o| q.matches(o)));
        q.modified = true;
        q.option_type = Some(FsetOptionType::Boolean);
        assert!(q.matches(&a) && !q.matches(&b) && !q.matches(&c));
        q.file = Some("weechat".into());
        assert!(!q.matches(&a));
    }

    #[test]
    fn redraw_keeps_rows_until_it_settles() {
        let ctx = egui::Context::default();
        let ts = chrono::Utc::now();
        let full: VecDeque<Line> = (0..3).map(|i| Line::new(i.to_string(), ts, std::string::String::new(),
            machine_line(&[&format!("a.b.o{i}"), "integer", "1", "1", "", "0", "0", "0", "9", "", ""]), true, false)).collect();
        let mut state = FsetState::default();
        assert_eq!(state.options(&ctx, "b", &full).1.len(), 3);

        // WeeChat cleared the buffer and has sent one line back so far.
        state.revision += 1;
        let partial: VecDeque<Line> = full.iter().take(1).cloned().collect();
        assert_eq!(state.options(&ctx, "b", &partial).1.len(), 3, "rows held during the redraw");

        // Quiet for long enough: a smaller list (a narrower filter) is taken.
        state.changed_at = Some(Instant::now() - REDRAW_SETTLE);
        assert_eq!(state.options(&ctx, "b", &partial).1.len(), 1);
    }

    #[test]
    fn filter_is_read_from_title() {
        assert_eq!(filter_from_title("12/340 | Filter: f:irc | Sort: ~name | Key(input): …").as_deref(), Some("f:irc"));
        assert_eq!(filter_from_title("no filter here"), None);
    }
}
