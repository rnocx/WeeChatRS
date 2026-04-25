# WeeChatRS — Development TODO

## 🛠 Core Protocol & Connectivity
- [x] Read synchronization — sync read markers with the server on buffer select
- [x] Sync markers — unread divider line in chat
- [x] Auto-reconnection — exponential backoff on dropped connections
- [x] Float ID parsing — relay sends buffer IDs as JSON floats; `parse_id()` handles i64/f64/str
- [ ] Server lag indicator — show real-time relay latency in the toolbar
- [ ] Auth error feedback — authentication failures are swallowed as plain disconnects; surface as a distinct error state

## ⌨️ UX & Productivity
- [x] Command history — Arrow Up/Down cycles sent messages in the input bar
- [x] Buffer search — `Ctrl+F` filters current buffer scrollback
- [x] Context menus — right-click nicks (query, whois); right-click buffers (leave, close)
- [x] Tab completion — nick completion with cycling
- [x] Emoji completion — `:name` + Tab inserts emoji (e.g. `:fire` → 🔥), cycles through matches
- [x] Server group headers — labeled dividers between networks in buffer list, toggleable in Settings
- [x] Merged server headers — `irc.server.*` buffer entry acts as the group header (uppercase, accent color, gap above) instead of a separate label row
- [x] Global shortcuts — `Meta+↑/↓` cycle buffers, `Meta+B/N` toggle panels
- [x] Buffer reordering — drag and drop buffers and server group headers; moving a server header moves its entire channel tree; order persists across restarts and reconnects
- [ ] Spell checking — client-side via `symspell` (pure Rust, no system deps); check words as the user types and show a suggestions popup above the input bar when the cursor is on a misspelled word; stretch goal: paint red underlines over the `TextEdit` using `ui.painter()` and galley word positions
- [ ] `Ctrl+K` quick buffer switcher — fuzzy find across all buffers
- [ ] `Alt+1-9` jump to buffer by number
- [ ] Keybinding editor — a window listing all keyboard shortcuts with the ability to rebind them; shortcuts stored in `AppSettings` and read at runtime instead of being hardcoded; accessible from the Settings window
- [ ] `/set` and `/fset` UX — currently users must type the full option path (e.g. `/set irc.look.highlight_pv " "`); add Tab completion for option names and, for `/fset`, a browsable settings panel similar to the WeeChat TUI `fset` buffer

## 🎨 Styling & Polishing
- [x] Modern UI redesign — card-style login, layered surfaces, rounded aesthetic
- [x] Top toolbar — connection status, sidebar toggles, settings button
- [ ] Unread count badge — show message count next to buffer name, not just the highlight dot
- [ ] User icons — subtle avatars next to nicks in the list
- [ ] Dynamic layout — option to move buffer list to the right or top
- [ ] Detached settings window — open Settings as a separate OS window (movable to any monitor) using eframe's multi-viewport API (`ctx.show_viewport_deferred()`); requires extracting mutable settings fields into a shared `Arc<Mutex<SettingsState>>`

## 💾 Persistence & Security
- [x] Session persistence — host, port, SSL, UI preferences saved across restarts
- [x] Theme persistence — selected `.itermcolors` theme saved
- [x] Secure storage — relay password in system keyring (macOS Keychain, libsecret, Windows Credential Manager)
- [ ] Android APK build — eframe supports Android via `android-activity`; blockers: feature-gate `notify-rust`, `plist`, `rfd` behind `#[cfg(not(target_os = "android"))]`, swap `native-tls` for `rustls`, add `android_main` entry point, wire up `cargo-apk`
- [ ] Scroll position memory — remember per-buffer scroll position when switching back

## 📷 Media & Attachments
- [x] Inline images — `🖼 preview` button on image URLs (.png/.jpg/.gif/.webp); click to load and display inline, toggleable in Settings
- [x] Link previews — `🔗 preview` button on non-image URLs; fetches OG tags (title, description, og:image), renders card with left accent bar, toggleable in Settings
- [ ] File drag & drop — upload via common paste services
- [ ] Code syntax highlighting — `syntect` for fenced code blocks in chat

---

## 🔧 Code Quality & Performance

- [ ] **Cache compiled regexes** — two regexes compiled on every call:
    - `strip_ansi()` in `event_handler.rs:430` — once per highlight notification
    - `ANSIParser::parse()` in `ansi.rs:82` — once per rendered message line
    - Fix: `std::sync::OnceLock` for both

- [ ] **Single buffer lookup per frame** — `app.rs:841-848` runs 7 separate `buffers.iter().find()` calls for the same selected buffer every frame. Extract once into a local `Option<&Buffer>` and derive all fields from it.

- [x] **Unsafe `unwrap()` in render loop** — fixed; code now uses `.and_then()` chaining with no unwrap on `current_buffer_last_read_id`.

- [ ] **VecDeque for command history** — `input.rs:159` uses `Vec::remove(0)` to trim to 100 items (O(n) shift). Replace with `VecDeque` + `pop_front`.

- [ ] **VecDeque for message buffer** — `event_handler.rs:464,529` use `buffer.messages.remove(0)` when over `MAX_MESSAGES` (O(n)). Use `VecDeque` in `Buffer::messages` for O(1) front removal.

- [ ] **Cloning entire message/nick vecs per frame** — `app.rs:841-848` clones `b.messages` (up to 400 items) and `b.nicks` every frame for the render pass. Restructure render code to borrow these directly instead of cloning.

- [ ] **Per-frame lowercase allocation in search** — `app.rs:1067` calls `self.search_text.to_lowercase()` once per rendered message line every frame. Cache the lowercased string once per frame when search is active.

- [ ] **Dead code: `debug_log` field** — `app.rs` has `#[allow(dead_code)] pub(crate) debug_log: Vec<String>` that is never read or written. Remove it.

- [ ] **Tests** — `ANSIParser::parse`, `extract_metadata`, `sort_buffers`, and `parse_id` are pure/near-pure functions; good first targets for unit tests.
