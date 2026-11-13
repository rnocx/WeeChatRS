# IRC protocol support

What the IRC backend speaks when WeeChatRS connects straight to an IRC server or a bouncer. For soju and ZNC specifics, see [Bouncers](bouncers.md).

## IRCv3 capabilities

All capabilities are negotiated via `CAP LS 302` on connect and requested only when the server advertises them. `cap-notify` allows new caps to be requested mid-session.

| Capability | Behaviour |
| :--- | :--- |
| `sasl` | SASL login, requested only when a password or a client certificate is set: PLAIN with the password, or EXTERNAL with the certificate (certfp) |
| `message-tags` | Needed for `server-time`, `msgid` and the other tag-based features |
| `server-time` | Accurate message timestamps from the `@time=` tag |
| `multi-prefix` | All mode prefixes shown per nick (e.g. `@+`) in NAMES and WHO |
| `away-notify` | The server sends `AWAY` when a nick goes away or comes back, and the nick list dims away nicks |
| `account-notify` | The server sends `ACCOUNT` on a services login or logout, shown as a channel notice |
| `extended-join` | JOIN carries the account name, which shows in the join message |
| `batch` | Used to deliver `chathistory` replies as a single atomic batch |
| `chathistory` | `CHATHISTORY LATEST` on join, and `CHATHISTORY BEFORE` for older lines |
| `echo-message` | Your message shows as soon as you press Enter, and the server's copy of it is dropped when it arrives. Messages over 400 bytes wait for that copy, since a server may cut them |
| `invite-notify` | Receive `INVITE` events for other users in shared channels |
| `chghost` | Host changes for nicks in shared channels shown as a channel notice |
| `userhost-in-names` | `nick!user@host` entries in NAMES are read down to the nick |
| `cap-notify` | Newly advertised caps are requested dynamically mid-session |
| `labeled-response` | Matches replies to requests, such as server-side search and history around a message |
| `msgid` | Deduplicates messages by ID to prevent bouncer replay duplicates (ring-buffer capped at 2000) |
| `MONITOR` | Opening a private buffer sends `MONITOR + nick`, and the buffer shows when the nick comes online or leaves |

## Formatting

IRC formatting codes in messages and topics are drawn: bold, italic, underline, colours 0–15 (in the theme's colours), colours 16–98 (their fixed RGB values), `\x04` hex colours, and reset. Reverse, strikethrough and monospace are dropped.

Messages that look like ASCII art stay on one row, spaces and all. Block, box-drawing and braille characters count, and so do runs of three spaces. Wide art scrolls sideways and never wraps.

## Typing indicators

With **Show who is typing** on, a line above the input says who is typing in the open conversation. It comes from IRCv3 `+typing` tags, and an indicator lapses after about 6 seconds without a refresh, or 30 seconds once paused.

**Tell others when I'm typing** is off by default, because it shows when you start and stop. When it's on, WeeChatRS sends a `+typing` TAGMSG while you type, one every 3 seconds at most, and says paused after 5 seconds without a keystroke. Lines that start with `/` aren't announced. It only sends when the server agreed to `message-tags`, and stops for the session if the server answers a TAGMSG with an error. Injected has its own events for the same thing. Through a WeeChat relay both directions work when WeeChat has the `POST /api/typing` endpoint and its own typing options switched on (`typing.look.*` and `irc.look.typing_status_*`). A relay without the endpoint just ignores the requests.

## CTCP

`/ctcp <nick> <command>` sends a CTCP, and the nick list's right-click menu has Version, Ping and Time. The query and the reply both show in the network's server buffer, a PING reply with its round trip in seconds. Requests from others show there too, and the app answers `PING`, `VERSION`, `TIME` and `CLIENTINFO`.
