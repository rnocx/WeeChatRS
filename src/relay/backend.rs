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
    /// Unread activity changed for a buffer.
    ActivityChanged {
        buffer_id: String,
        activity: BufferActivity,
        unread_count: u32,
    },
    /// A buffer's topic changed.
    TopicChanged {
        buffer_id: String,
        topic: String,
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

    /// Temporary tunnel for WeeChat relay responses until the event handler
    /// is fully migrated to BackendEvent in Phase 3.
    #[doc(hidden)]
    _WeeChat(crate::relay::models::WeeChatResponse),
}

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

    /// Request historical lines for a buffer (most recent `count` lines).
    fn fetch_lines(&self, buffer_id: &str, count: usize);

    /// Request the nicklist for a buffer.
    fn fetch_nicks(&self, buffer_id: &str);

    /// Signal that the user has read up to the latest message in a buffer.
    fn mark_read(&self, buffer_id: &str);

    /// True if the underlying connection is currently established.
    fn is_connected(&self) -> bool;

    /// Refresh backend-specific buffer metadata (topic, modes, type).
    /// IRC backends receive this passively via protocol events; default is a no-op.
    fn refresh_buffer(&self, _buffer_id: &str) {}

    /// Request the full buffer list. IRC backends discover buffers via JOIN/NAMES;
    /// default is a no-op.
    fn fetch_buffer_list(&self) {}

    /// Request current hotlist / unread activity data. IRC backends track this
    /// client-side; default is a no-op.
    fn fetch_hotlist(&self) {}

    /// Subscribe to server-push events. WeeChat uses POST /api/sync; IRC has
    /// no equivalent; default is a no-op.
    fn sync_subscriptions(&self) {}

    /// Request lines older than a given anchor timestamp (ISO 8601).
    /// Used by IRC backends for CHATHISTORY BEFORE; WeeChat uses fetch_lines.
    /// Default is a no-op.
    fn fetch_lines_before(&self, _buffer_id: &str, _before_ts: &str) {}
}
