# soju_feature branch — progress tracker

## Completed
- [x] Phase 1 — BackendClient trait + BackendEvent enum (src/relay/backend.rs)
- [x] Phase 2 — WeeChat reorganised into src/relay/weechat/
- [x] Phase 3 — app.rs wired to BackendEvent; BackendType selector in UI
- [x] Phase 4 — IrcClient stub in src/relay/irc/
- [x] Phase 5 — Full IRC connection loop: CAP negotiation, PASS/NICK/USER,
                  PING/PONG, JOIN/PART/KICK/QUIT/NICK, PRIVMSG/NOTICE,
                  353/366 NAMES, 332/TOPIC, chathistory LATEST on JOIN
- [x] Phase 6 — soju.im/bouncer-networks (BOUNCER NETWORK LIST → server buffers)
                  soju.im/read-marker (MARKREAD on mark_read())
- [x] Phase 7 — CHATHISTORY BEFORE scroll-back; BATCH open/close tracking;
                  fetch_lines_before() on BackendClient; spinner clears on LinesLoaded
- [x] Phase 8 — IRC UI fixes:
                  - Auto-open DM buffers on incoming PRIVMSG
                  - pending_buffer_switch resolved on BufferOpened (/join auto-nav)
                  - Connection log is backend-aware (WeeChat vs Soju messages)
                  - Nicklist sorted: ops → voiced → alphabetical

## Remaining
- [x] Phase 9 — Polish:
                  - WHO #channel after join (richer nick info: away, realname)
                  - /whois response display (show as system lines in active buffer)
                  - Handle 464/465/432 auth error codes → AuthError event
                  - Server NOTICE messages (pre-001) shown in connection log
- [ ] Phase 10 — Live testing against soju + WeeChat relay regression check
