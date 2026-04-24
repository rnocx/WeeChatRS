# WeeChatRS

A high-performance, modern, and cross-platform GUI client for [WeeChat](https://weechat.org/) Relay. Built from the ground up in Rust using `egui` and `tokio`, this client provides a polished experience with real-time synchronization, advanced ANSI parsing, and low-latency networking.

![Screenshot Placeholder](https://via.placeholder.com/800x450?text=WeeChatRS+Rust)

## 🚀 Key Features

*   **Modern Redesign:** A clean, spacious UI with rounded corners, depth-layered surfaces, and a professional color palette.
*   **Real-time Relay API v2:** Full support for WeeChat Relay protocol over WebSocket (SSL/TLS supported).
*   **Advanced ANSI Engine:** Support for 16-color, 256-color, and TrueColor (RGB) escape sequences.
*   **Read Synchronization:** bidirectional synchronization of read markers and channel activity levels (Highlights, Messages, Metadata).
*   **Productivity First:**
    *   **Tab Completion:** Intelligent nickname completion with cycling.
    *   **Command History:** Cycle through sent commands/messages with Arrow Up/Down.
    *   **Quick Search:** Instant buffer filtering with `Ctrl+F`.
    *   **Keyboard Driven:** Switch buffers using `Meta + Arrow Up/Down`.
*   **Theming Engine:** Import any `.itermcolors` file to instantly restyle your terminal environment.
*   **Zero-Config Persistence:** Automatically saves and loads your connection profiles, theme, and UI preferences.
*   **Native Notifications:** System-level alerts for highlights and private messages.

## 🛠 Prerequisites

Ensure you have the following installed:

*   **Rust Toolchain:** [Install Rust](https://rustup.rs/) (Stable 1.75+ recommended).
*   **System Dependencies (Linux only):**
    ```bash
    sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
    ```

## 🏃 Running from Source

To start the application in development mode:

```bash
cargo run
```

For the best performance and smooth scrolling, use release mode:

```bash
cargo run --release
```

## 📦 Building a Release Binary

To generate a standalone executable for your current platform:

```bash
cargo build --release
```
The binary will be located at `./target/release/weechat-rs`.

### 🌍 Cross-Compilation

WeeChatRS supports cross-compilation to Linux, macOS, and Windows. We recommend using [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild) for a seamless experience.

1.  **Install Zig and zigbuild:**
    ```bash
    brew install zig
    cargo install cargo-zigbuild
    ```

2.  **Add Targets:**
    ```bash
    rustup target add x86_64-pc-windows-msvc
    rustup target add x86_64-unknown-linux-gnu
    ```

3.  **Build:**
    ```bash
    # For Windows
    cargo zigbuild --target x86_64-pc-windows-msvc --release

    # For Linux
    cargo zigbuild --target x86_64-unknown-linux-gnu --release
    ```

## ⌨️ Global Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Meta + ↑ / ↓` | Cycle through Buffers |
| `Meta + B` | Toggle Buffer List |
| `Meta + N` | Toggle Nick List |
| `Ctrl + F` | Toggle Message Search/Filter |
| `Tab` | Nickname Completion (Cycle matches) |
| `Arrow ↑ / ↓` | Cycle Command History (in input bar) |
| `Enter` | Send Message |
| `Right Click` | Open Context Menu (on Nicks) |

## 🎨 Transparency & UI

The application supports real-time opacity adjustment via the **Settings** menu. 

### 🪟 Note for Windows Users
To enable transparency on Windows, ensure that **Transparency effects** are enabled in your system settings:
1. Open **Settings**.
2. Go to **Personalization > Colors**.
3. Toggle **Transparency effects** to **On**.

If disabled, or if your hardware/driver does not support desktop composition transparency, the application will render with a solid background instead of a translucent one.

## 🏗 Architecture Overview

*   **`relay/`**: Core networking logic. Uses `tokio-tungstenite` for asynchronous WebSocket communication and `serde_json` for protocol handling.
*   **`ui/ansi.rs`**: A custom parser that transforms raw WeeChat ANSI streams into `egui` compatible text layouts, including URL detection.
*   **`ui/app.rs`**: The main application loop, state management, and layout definition.
*   **`ui/theme.rs`**: PList parser for `.itermcolors` and global styling constants.

## 🤝 Contributing

Contributions are welcome! Please check the `todo.md` for a list of planned features and known issues.

## 📄 License

MIT License - Copyright (c) 2026
