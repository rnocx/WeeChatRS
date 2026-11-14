use crate::relay::models::{Buffer, BufferActivity, Line, Nick};

/// All events a backend can emit to the UI layer.
/// Neither WeeChat-specific nor IRC-specific — both backends translate
/// their native protocol events into these variants.
#[derive(Debug)]
#[allow(dead_code)]
pub enum BackendEvent {
    /// Successfully authenticated and ready.
    Connected,
    /// Connection lost (clean close or network drop).
    Disconnected,
    /// Unrecoverable error with a human-readable message.
    Error(String),
    /// Authentication failed (wrong password, rejected SASL, etc.).
    AuthError(String),
    /// The WeeChat relay wants a TOTP code (`relay.network.totp_secret` is set).
    /// `invalid` is true when the code we sent was refused, false when none was sent.
    TotpRequired { invalid: bool },
    /// Informational message for the connection log only (not shown as an error).
    ConnLog(String),
    /// The SSH tunnel stopped for good (bad login, changed host key…): the
    /// connection stops too, with this message.
    TunnelFailed(String),
    /// Full initial buffer list received on connect.
    BuffersLoaded(Vec<Buffer>),
    /// A single new message arrived in a buffer.
    LineAdded {
        buffer_id: String,
        line: Line,
    },
    /// A buffer's complete nicklist was (re)loaded.
    NicklistLoaded {
        buffer_id: String,
        nicks: Vec<Nick>,
    },
    /// A nick was added to a buffer's nicklist.
    NickAdded {
        buffer_id: String,
        nick: Nick,
    },
    /// A nick was removed from a buffer's nicklist.
    NickRemoved {
        buffer_id: String,
        nick_name: String,
    },
    /// A nick's away status changed.
    NickAwayChanged {
        buffer_id: String,
        nick_name: String,
        away: bool,
    },
    /// Unread activity changed for a buffer.
    ActivityChanged {
        buffer_id: String,
        activity: BufferActivity,
        unread_count: u32,
        /// When set, marks messages up to this timestamp as read (soju MARKREAD).
        markread_ts: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// A buffer's topic changed.
    TopicChanged {
        buffer_id: String,
        topic: String,
    },
    /// A channel's modes changed (`+nt`), for the topic bar.
    ModesChanged {
        buffer_id: String,
        modes: String,
    },
    /// A buffer's hidden flag changed.
    BufferHidden {
        buffer_id: String,
        hidden: bool,
    },
    /// A new buffer was opened (e.g. JOIN).
    BufferOpened(Buffer),
    /// A buffer was closed (e.g. PART/KICK).
    BufferClosed {
        buffer_id: String,
    },
    /// Lines for a buffer were loaded (response to fetch_lines).
    LinesLoaded {
        buffer_id: String,
        lines: Vec<Line>,
        is_prepend: bool,
    },

    /// Reply to `search` or `fetch_context`: the matching lines, or the lines
    /// around a message, as `(buffer_id, line)`. One per buffer searched.
    HistoryResults {
        request: String,
        lines: Vec<(String, Line)>,
        error: Option<String>,
    },

    /// Output of a ZNC module command sent with `znc_command`: `command` is what was
    /// sent (`*controlpanel AddNetwork …`), `lines` everything the bouncer answered.
    BouncerReply {
        command: String,
        lines: Vec<String>,
    },

    /// Our nick, oper status and the server software on the server whose buffer
    /// `buffer_id` is, sent whenever one of them changes.
    ServerStatus {
        buffer_id: String,
        status: ServerStatus,
    },

    /// Part of the server's channel list (`list_channels`), in the order it arrives,
    /// for the network whose server buffer is `buffer_id`. `done` comes with the last
    /// part, or with `error` when the server refused.
    ChannelList {
        buffer_id: String,
        channels: Vec<crate::relay::models::ListedChannel>,
        done: bool,
        error: Option<String>,
    },

    /// `nick` started, paused or stopped typing in the buffer.
    Typing {
        buffer_id: String,
        nick: String,
        state: TypingState,
    },

    /// A lag check on the network whose server buffer is `buffer_id`, or with `None`
    /// on the connection itself (the WeeChat relay, Injected's server).
    Lag {
        buffer_id: Option<String>,
        lag: Lag,
    },

    /// Temporary tunnel for WeeChat relay responses until the event handler
    /// is fully migrated to BackendEvent in Phase 3.
    #[doc(hidden)]
    _WeeChat(crate::relay::models::WeeChatResponse),
}

/// What someone is doing at the keyboard, as IRCv3's `+typing` tag says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypingState {
    /// Typing right now. Counts for about 6 seconds without another.
    Active,
    /// Has text in the input but stopped. Counts for about 30 seconds.
    Paused,
    /// Cleared the input without sending it.
    Done,
    /// Sent the message. The message itself ends the indicator, so IRC says
    /// nothing; only a WeeChat relay passes it on.
    Sent,
}

impl TypingState {
    pub fn as_str(self) -> &'static str {
        match self {
            TypingState::Active => "active",
            TypingState::Paused => "paused",
            TypingState::Done => "done",
            TypingState::Sent => "sent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(TypingState::Active),
            "paused" => Some(TypingState::Paused),
            "done" => Some(TypingState::Done),
            _ => None,
        }
    }
}

/// One step of a lag check, for `BackendEvent::Lag`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lag {
    /// A check went out at this moment and hasn't been answered yet.
    Waiting(std::time::Instant),
    /// The round trip of the last check.
    Measured(std::time::Duration),
    /// Under the least lag the server reports: WeeChat leaves out anything under
    /// irc.network.lag_min_show (500 ms by default).
    Low,
}

/// An optional backend-owned paste upload; only Injected implements it.
pub type PasteUpload = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>;

/// Contract every backend must satisfy.
/// The UI instantiates one of these and interacts only through this trait.
#[allow(dead_code)]
pub trait BackendClient: Send {
    /// Open the connection. Events flow back via the channel passed at construction.
    fn connect(&mut self);

    /// Close the connection cleanly.
    fn disconnect(&mut self);

    /// Send a chat message to a buffer.
    fn send_message(&self, buffer_id: &str, text: &str);

    /// Whether this backend offers its own authenticated paste-link service.
    fn supports_paste_upload(&self) -> bool { false }

    /// Create a paste without posting it to chat. The UI asks first and sends the
    /// returned link to the original buffer only after a successful upload.
    fn create_paste(&self, _content: String) -> PasteUpload {
        Box::pin(async { Err("This backend does not offer a paste service.".into()) })
    }

    /// Request historical lines for a buffer (most recent `count` lines).
    fn fetch_lines(&self, buffer_id: &str, count: usize);

    /// Request the nicklist for a buffer.
    fn fetch_nicks(&self, buffer_id: &str);

    /// Signal that the user has read up to the latest message in a buffer.
    fn mark_read(&self, buffer_id: &str);

    /// True if the underlying connection is currently established.
    fn is_connected(&self) -> bool;

    /// Tell the others in `buffer_id` that we are typing, paused or done. Backends
    /// that can't carry it ignore the call.
    fn send_typing(&self, _buffer_id: &str, _state: TypingState) {}

    /// Refresh backend-specific buffer metadata (topic, modes, type).
    /// IRC backends receive this passively via protocol events; default is a no-op.
    fn refresh_buffer(&self, _buffer_id: &str) {}

    /// Request the full buffer list. IRC backends discover buffers via JOIN/NAMES;
    /// default is a no-op.
    fn fetch_buffer_list(&self) {}

    /// Request current hotlist / unread activity data. IRC backends track this
    /// client-side; default is a no-op.
    fn fetch_hotlist(&self) {}

    /// The same request, asked again while connected to see what was read on the
    /// server's side. Only WeeChat has one; default is a no-op.
    fn poll_hotlist(&self) {}

    /// Subscribe to server-push events. WeeChat uses POST /api/sync; IRC has
    /// no equivalent; default is a no-op.
    fn sync_subscriptions(&self) {}

    /// Request lines older than a given anchor timestamp (ISO 8601).
    /// Used by IRC backends for CHATHISTORY BEFORE; WeeChat uses fetch_lines.
    /// Default is a no-op.
    fn fetch_lines_before(&self, _buffer_id: &str, _before_ts: &str) {}

    /// True if `search` runs on the server (soju.im/search).
    fn supports_search(&self) -> bool { false }

    /// Search the server's history of `buffer_id` for `text`; one `HistoryResults`
    /// tagged `request` comes back. Default is a no-op.
    fn search(&self, _buffer_id: &str, _text: &str, _request: &str) {}

    /// Fetch the lines around `timestamp` (ISO 8601) in `buffer_id`, as a
    /// `HistoryResults` tagged `request`. Default is a no-op.
    fn fetch_context(&self, _buffer_id: &str, _timestamp: &str, _request: &str) {}

    /// True if `complete` asks the server to complete input.
    fn supports_completion(&self) -> bool { false }

    /// Ask the server to complete `text` at character `position`; the reply comes back
    /// tagged with `tag`. Only WeeChat implements this; default is a no-op.
    fn complete(&self, _buffer_id: &str, _text: &str, _position: usize, _tag: &str) {}

    /// True once the server turned out to be a ZNC bouncer (it offers `znc.in/*` caps).
    fn is_znc(&self) -> bool { false }

    /// The ZNC user's networks, as of the last `*status ListNetworks`.
    fn znc_networks(&self) -> Vec<ZncNetwork> { Vec::new() }

    /// Send `text` to the ZNC module `module` (`*controlpanel`, `*status`) without it
    /// showing in the chat; the reply comes back as `BouncerReply`. Changes to the
    /// networks are followed by a fresh `znc_networks` list. Default is a no-op.
    fn znc_command(&self, _module: &str, _text: &str) {}

    /// Whether the app shows this backend's channel list; WeeChat has its own /list
    /// buffer. Default is false.
    fn supports_channel_list(&self) -> bool { false }

    /// Asks the server of `buffer_id`'s network for its channels, those with at least
    /// `min_users` where the server can filter; they come back as `ChannelList`.
    fn list_channels(&self, _buffer_id: &str, _min_users: u32) {}

    /// True if the backend reports whether we're an IRC operator (`ServerStatus`).
    /// Where it doesn't, the user says so per server. Default is false.
    fn detects_oper(&self) -> bool { false }
}

/// What we are on a server, for the menus.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerStatus {
    /// Our nick there; empty if unknown.
    pub nick: String,
    /// We're an IRC operator (user mode +o or +O).
    pub oper: bool,
    /// The server software from `004`, e.g. `UnrealIRCd-6.1.8`; empty if unknown.
    pub software: String,
}

/// A network of a ZNC user, from a row of `*status ListNetworks`.
#[derive(Debug, Clone, PartialEq)]
pub struct ZncNetwork {
    pub name: String,
    /// ZNC is connected to the network's IRC server.
    pub on_irc: bool,
    pub server: String,
    pub channels: String,
}
