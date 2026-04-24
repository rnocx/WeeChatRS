# WeeChatRS (Rust) - Development TODO

## 🛠 Core Protocol & Connectivity
- [x] **Read Synchronization:** Sync read markers with the server when buffers are selected.
- [x] **Sync Markers:** Visual indicator in chat for where you last left off.
- [x] **Auto-Reconnection:** Implement exponential backoff for dropped connections.
- [ ] **Server Lag Indicator:** Show real-time relay latency in the UI.

## ⌨️ UX & Productivity
- [x] **Command History:** Cycle through sent messages with Arrow Up/Down in the input bar.
- [x] **Buffer Search:** Local search (`Ctrl+F`) for the current buffer scrollback.
- [x] **Context Menus:** Right-click nicks for `/query` and `/whois`.
- [x] **Tab Completion:** Nickname completion in the input bar with cycling.
- [x] **Global Shortcuts:**
    - [x] `Meta + Arrow Up/Down` to jump between buffers.
    - [ ] `Ctrl+K` for quick buffer switcher (fuzzy find).
    - [ ] `Alt+1-9` to jump to specific buffers.
- [ ] **Emoji Support:** colon-completion (e.g., `:smile:`) or a dedicated picker.

## 🎨 Styling & Polishing
- [x] **Modern UI Redesign:** Card-style login, layered surfaces, and rounded aesthetic.
- [x] **Status Bar:** Added top toolbar with toggles and connection status.
- [ ] **User Icons:** Subtle icons or avatars next to nicks in the list.
- [ ] **Dynamic Layout:** Option to move the buffer list to the right or top.

## 💾 Persistence & Security
- [x] **Session Persistence:** Save Host, Port, and SSL settings locally.
- [x] **Theming Persistence:** Save the currently selected `.itermcolors` theme.
- [ ] **Secure Storage:** Store relay password in the system keyring (macOS Keychain, etc.).

## 📷 Media & Attachments
- [ ] **Inline Images:** Detect image URLs and render small previews in chat.
- [ ] **File Drag & Drop:** Support for uploading files via common services.
- [ ] **Code Syntax Highlighting:** Use a lighter version of `syntect` for code blocks in chat.
