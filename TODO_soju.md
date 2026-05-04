# soju bouncer support — actual state

Re-audited 2026-05-02. Earlier checkmarks claimed "complete" but several items
were stubs or had wrong CAP names. This file reflects what the code actually does.

## Completed

- [x] Phase 1 — `BackendClient` trait + `BackendEvent` enum (`src/relay/backend.rs`)
- [x] Phase 2 — WeeChat split into `src/relay/weechat/`
- [x] Phase 3 — `app.rs` wired to `BackendEvent`; `BackendType` selector in UI
- [x] Phase 4 — `IrcClient` stub
- [x] Phase 5 — Core IRC: CAP LS/REQ/ACK/END, NEW/DEL cap-notify, PASS/NICK/USER,
                  PING/PONG, JOIN/PART/KICK/QUIT/NICK, PRIVMSG/NOTICE,
                  353/366 NAMES, 332 TOPIC
- [x] Phase 6 (partial) — `soju.im/bouncer-networks` requested; `BOUNCER NETWORK LIST`
                  parsed → server buffers; `soju.im/read-marker` requested;
                  outgoing MARKREAD on `mark_read()`. **Now also**: incoming MARKREAD
                  handler clears unread/activity on the buffer
                  (sync read-state across clients).
- [x] Phase 7 — CHATHISTORY BEFORE scrollback, BATCH start/end deferral,
                  `fetch_lines_before()` on `BackendClient`, spinner clears on `LinesLoaded`
- [x] Phase 8 — DM auto-open on PRIVMSG; `pending_buffer_switch` resolved on
                  `BufferOpened`; backend-aware connection log; nicklist sorted
                  ops → voiced → alpha
- [x] Phase 9 (partial) — WHO #channel after JOIN; auth errors 432/464/465 → AuthError;
                  pre-001 NOTICEs routed to connection log; WHOIS 311/312/317/318/319
- [x] **Chathistory CAP name fix** — soju advertises `draft/chathistory`, not
                  `chathistory`. Both names now requested; `Session::has_chathistory()`
                  accepts either. Without this, `CHATHISTORY LATEST` was never sent
                  on JOIN against soju and history failed to load.
- [x] **Incoming MARKREAD handler** — was a no-op; now emits `ActivityChanged`
                  to clear unread state when another client (or this one earlier)
                  marks a target read.
- [x] IRCv3 baseline: SASL PLAIN (903/904/905), `server-time`, `account-tag`,
                  `extended-join`, `multi-prefix`, `away-notify`, `chghost`,
                  `echo-message`, `message-tags`, `userhost-in-names`,
                  `cap-notify`, `labeled-response`, `msgid` dedup
- [x] TLS via native-tls

## Known gaps (intentional / deferred)

- **`BIND` command** — not sent. Most soju users authenticate as `<user>/<network>`
  in the username field, which routes the connection without needing BIND. BIND
  is only needed for users who want to discover networks via `BOUNCER NETWORK LIST`
  and pick one mid-registration; that requires a persistent network-id selector
  in the connection profile + edit UI. Until then, server buffers shown for each
  network are decorative. **Workaround**: set Username to `<user>/<network>`.
- **WHO realname** — RPL_WHO 352 is parsed but realname (param 7) isn't stored
  on `Nick`. Low impact.
- **WHOIS numerics 313/330/671** — operator status, account login, secure
  connection lines aren't shown. 311/312/317/318/319 work.

## Verified working

- [x] WeeChat relay backend: live testing, message send + receive
- [x] soju: chathistory now arrives on JOIN (post-CAP-name fix)
- [x] soju: outgoing read-marker writes; incoming read-marker clears unread
