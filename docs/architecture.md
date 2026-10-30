# Architecture

WeeChatRS is one Rust binary: an [egui](https://github.com/emilk/egui)/eframe window on top of a [tokio](https://tokio.rs/) runtime. Each connection runs as tokio tasks that turn their protocol into `BackendEvent`s; the UI drains those events once per frame and talks back through the `BackendClient` trait, so it doesn't know which protocol is on the other side.

```
src/
  main.rs                    — tokio runtime, eframe window setup, app icon loading
  desktop_integration.rs     — Linux: desktop entry and icons when run without installing
  relay/
    backend.rs               — BackendClient trait + BackendEvent enum (protocol-agnostic)
    models.rs                — Buffer, Line, Nick, BufferActivity, WeeChatResponse
    weechat/
      mod.rs                 — WeeChat Relay API v2 client over WebSocket: auth,
                               reconnect, idle keepalive
      event_handler.rs       — BackendEvent dispatch into the UI state, WeeChat responses,
                               hotlist, buffer grouping and read-state tracking
    irc/
      mod.rs                 — IrcClient: BackendClient impl, command routing,
                               a connection per soju or ZNC network
      connection.rs          — TCP/TLS connection loop, IRCv3 CAP negotiation, SASL,
                               PING/PONG keepalive, message translation → BackendEvent,
                               mIRC formatting → ANSI, ZNC module queries
      parser.rs              — IRCv3 message parser (tags, prefix, command, params)
  ui/
    app.rs                   — WeeChatApp state, AppSettings, main render loop, layout
                               panels, buffer list, chat view, context menus
    settings.rs              — Settings and Connections windows
    input.rs                 — input bar: tab completion, command history, /commands
    ansi.rs                  — ANSI SGR parser (16-color, 256-color, TrueColor,
                               bold, italic, underline, URL detection)
    theme.rs                 — AppTheme, Ghostty theme parser and theme catalogue
    wallpaper.rs             — adaptive theme from the desktop wallpaper
    fonts.rs                 — bundled chat fonts and system font loading
    keybinds.rs              — rebindable shortcuts: actions, storage, dispatch
    quick_switcher.rs        — ⌘/Ctrl+P fuzzy buffer switcher
    fset.rs                  — /fset shown as a settings table
    server_search.rs         — soju server-side search results view
    znc.rs                   — ZNC networks panel and per-device read markers
    notify.rs                — desktop notifications (macOS, Windows, Linux)
    fileshare.rs             — /share uploads (Jirafeau, Litterbox)
    np.rs                    — /np now-playing (macOS Now Playing, Linux MPRIS,
                               Windows media controls)
    sysinfo.rs               — /sysinfo
    ssh_tunnel.rs            — SSH port-forward tunnel via an ssh subprocess
    secure_storage.rs        — passwords in the OS keychain
    url_safety.rs            — keeps link and image previews off private addresses
    vibrancy.rs              — macOS background blur
    emoji.rs                 — emoji shortcodes for tab completion
    icons.rs                 — Phosphor icon font
```

## Tests

`cargo test --bin weechat-rs` runs the unit tests. A few end-to-end tests need servers and are ignored by default; each says in its doc comment what it needs and how to run it (for example `znc_end_to_end`, against a local ZNC).
