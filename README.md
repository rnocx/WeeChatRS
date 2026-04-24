# WeeChatRS

A modern, high-performance GUI client for [WeeChat](https://weechat.org/) Relay, built in Rust using `egui` and `tokio`. Real-time chat with a polished native UI, full ANSI color support, and zero runtime dependencies.

## Features

- **Full WeeChat Relay API v2** over WebSocket with SSL/TLS
- **Advanced ANSI engine** — 16-color, 256-color, and TrueColor (RGB) sequences rendered natively
- **Read synchronization** — bidirectional sync of read markers; unread divider line shows where you left off
- **Server group headers** — buffer list automatically groups channels by network with labeled dividers (toggleable)
- **Tab completion** — nicknames and emoji (`:fire` + Tab → 🔥), cycling through all matches
- **Command history** — Arrow Up/Down in the input bar
- **Inline search** — `Ctrl+F` filters current buffer scrollback
- **Context menus** — right-click nicks for `/query` and `/whois`; right-click buffers to leave or close
- **Buffer reordering** — drag and drop buffers and server groups in the buffer list; order persists across restarts
- **Auto-reconnect** — exponential backoff on dropped connections
- **Theming** — import any `.itermcolors` file; background, foreground, and all 16 ANSI colors applied live
- **Native notifications** — system alerts for highlights and private messages
- **Opacity control** — real-time transparency adjustment

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Meta + ↑ / ↓` | Cycle buffers |
| `Meta + B` | Toggle buffer list |
| `Meta + N` | Toggle nick list |
| `Ctrl + F` | Toggle message search |
| `Tab` | Complete nick or emoji (cycles matches) |
| `Arrow ↑ / ↓` | Cycle command history (input bar) |
| `Enter` | Send message |
| Right-click nick | Query / Whois |
| Right-click buffer | Leave / Close |

## Building

Requires Rust stable 1.75+.

**Linux — install system dependencies first:**
```bash
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

**Run (development):**
```bash
cargo run
```

**Run (release — recommended for smooth rendering):**
```bash
cargo run --release
```

**Build binary:**
```bash
cargo build --release
# output: ./target/release/weechat-rs
```

### Cross-compilation

Use [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild) for cross-compilation:

```bash
brew install zig
cargo install cargo-zigbuild

rustup target add x86_64-unknown-linux-gnu
rustup target add x86_64-pc-windows-msvc

cargo zigbuild --target x86_64-unknown-linux-gnu --release
cargo zigbuild --target x86_64-pc-windows-msvc --release
```

## Transparency on Windows

Requires **Transparency effects** to be enabled: Settings → Personalization → Colors → Transparency effects: On. Without it the window renders with a solid background.

## Architecture

```
src/
  main.rs                — tokio runtime, eframe window setup
  relay/
    client.rs            — WebSocket client, auth, exponential backoff reconnection
    models.rs            — Buffer, Line, Nick, BufferActivity, WeeChatResponse
  ui/
    app.rs               — WeeChatApp struct, AppSettings, main render loop
    event_handler.rs     — Relay protocol response and event processing
    input.rs             — Completion (nick + emoji), command history, buffer navigation
    ansi.rs              — ANSI SGR parser (8/256/RGB color, bold, italic, underline, URLs)
    theme.rs             — AppTheme, .itermcolors plist parser
    settings.rs          — Settings window UI
    emoji.rs             — Emoji shortcode table (~150 entries)
```

## Contributing

See `todo.md` for planned features and known issues.

## License

MIT License — Copyright (c) 2026
