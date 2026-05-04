# Audit Checklist

Findings from the security & performance audit, ordered by priority.

## Security

- [x] **#1 SSRF guard for URL fetches** — block private/loopback/link-local IPs, non-http(s) schemes for image/link-preview fetches. (`src/ui/url_safety.rs`)
- [x] **#2 Unbounded IRC line buffer** — cap line length to 16 KB via custom `read_line_bounded`, errors out on overlong line. (`src/relay/irc/connection.rs`)
- [ ] **#3 TLS `accept_invalid_certs` UX** — show a visible badge in connection status when MITM-able.
- [~] **#4 Password exposure in headers** — **dropped, not actionable.** Both `Authorization: Basic` and `Sec-WebSocket-Protocol` transmit the password base64-encoded; removing either doesn't reduce wire exposure (both are TLS-encrypted). The original "send post-handshake" fix isn't possible because the WeeChat REST WebSocket API requires auth at handshake. Sending both also defends against proxies that strip one or the other on WS upgrade.
- [x] **#5 Header `parse().unwrap()` panics** — three header `.unwrap()`s replaced with `?`-propagated errors that emit `AuthError` and exit cleanly. (`src/relay/weechat/mod.rs`)
- [x] **#6 macOS notification — drop osascript** — unified all platforms on `notify_rust::Notification` (Cargo.toml already had it on macOS); `osascript_quote` helper removed. No more shell-string construction from message content.

## Performance

- [x] **#7 Cache ANSI parse on Line** — parse once at insertion; render uses cached spans + cheap `to_format(font, theme)`. Search uses cached lowercased plain text. (`src/relay/models.rs`, `src/ui/ansi.rs`, `src/ui/app.rs`)
- [ ] **#8 Search re-strips ANSI per keystroke** — covered partially by #7 (cached plain text now used); confirm no remaining hot paths.
- [x] **#9 O(n) buffer lookup per frame** — `HashMap<id, idx>` index added; `buffer_by_id`/`buffer_by_id_mut`/`buffer_idx_of` helpers; rebuilt at every push/retain/extend/sort site. (`src/ui/app.rs`, `src/relay/weechat/event_handler.rs`)
- [x] **#10 Unbounded growing collections** — `image_cache`/`preview_cache` capped at 200 each via `cap_map` helper called at insertion sites; `prefix_col_widths` pruned per-frame to current buffer set (cap 500); `last_notif_at` ages out entries >5min when over 200; `command_history` already capped at 100.
- [ ] **#11 Unbounded `tokio::spawn` per click** — track in-flight URL set to dedupe spawns.

## Stability

- [ ] **#12 Reconsider `cleared_buffer_ids` persistence** — now that mark_read works, this set should be session-local only; trust the server's `last_read_line_ufr`.
- [x] **#13 Keyring thread `.expect` panic** — `run_keyring` now returns `Result<R, String>`; callers flatten via `.and_then`/`.ok().flatten()`. (`src/ui/secure_storage.rs`)
- [x] **#14 No auth-failure detection in reconnect loop** — `tungstenite::Error::Http(401|403)` now emits `AuthError` and `return`s from the reconnect task instead of looping forever. (`src/relay/weechat/mod.rs`)
- [ ] **#15 cmd_rx not drained on disconnect** — pending sends pile up across reconnects.

## UX

- [x] **#16 Disabled input bar when disconnected** — input is gated on the *selected buffer's* connection (not just any connection); shows "Disconnected — reconnect before sending" hint when down.
- [ ] **#17 Reconnect preserves scroll position** — keep last-N lines per buffer through reconnect cycle.
- [ ] **#18 Keyboard buffer navigation** — Cmd/Alt+↑↓ for prev/next, Cmd+K jump-to-buffer.
- [ ] **#19 Atomic settings write** — write to `.tmp` and rename to avoid corruption on crash. **Deferred**: settings persisted via eframe's private `FileStorage` (non-atomic `fs::write`); fixing requires implementing a custom `eframe::Storage` wrapper or a separate persistence layer.
- [x] **#20 Notification rate-limiting** — 3s per-buffer cooldown via `last_notif_at: HashMap<buffer_id, Instant>` checked before each highlight notification.
- [x] **#21 Right-click → copy text on messages** — context menu on each message row offers "Copy message" and "Copy with sender" using cached `plain_message`/`plain_prefix`.
