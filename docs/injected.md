## Injected backend (experimental)

Select **Injected** in Connections to use the account's existing Injected networks
with the native WeeChatRS UI. The default endpoint is `injected-irc.com:443` with
TLS enabled. No Electron runtime is required.

### Authentication

1. Add or edit an **Injected** connection in WeeChatRS.
2. Click **Sign in to Injected…**. A separate window opens the official HTTPS
   Injected login page.
3. Enter your username and password there, complete the CAPTCHA and any
   second-factor prompt, and wait for the window to close.
4. Check the returned account name, enable **Save token in keychain** if wanted,
   then save and connect.

The website handles your login and two-factor flow. WeeChatRS receives the
completed session through a private helper-process pipe and verifies it with
Injected before filling the connection form. The helper uses an ephemeral web
session; it does not store your password or write the session token to a file.
Closing the window or clicking **Cancel sign-in** cancels the flow. Sign-in is
limited to `injected-irc.com:443` with TLS; custom endpoints still use a token.

The web sign-in window is enabled by the default `injected-web-login` Cargo
feature. It uses WKWebView on macOS, WebView2 on Windows, and WebKitGTK 4.1 on Linux.
The Linux release zips keep WebKitGTK out of `weechat-rs` itself: the window is a
separate `weechat-rs-login` beside it, which needs WebKitGTK 4.1 (package
`libwebkit2gtk-4.1-0` on Debian and Ubuntu, `webkit2gtk4.1` on Fedora,
`webkit2gtk-4.1` on Arch) and glibc 2.34 or later. Without them, the app says so, and
session import and tokens still work. See [build requirements](../BUILDING.md).
The official form and a successful CAPTCHA were visually checked on macOS.
Complete session handoff and any WebAuthn/second-factor prompt still need a
real-account check.

If web sign-in is unavailable, sign in to the **Injected desktop app** and click
**Import Injected session** in WeeChatRS instead.

WeeChatRS reads the desktop app's local session and verifies it with Injected.
It does not search browser profiles or save the token until you save the connection.
For a nonstandard installation, **Choose data folder…** accepts the Injected app
support directory or its `Local Storage/leveldb` directory. If import finds no
readable session, sign in to the desktop app again and retry. Older compacted
storage may not expose a readable token to the importer.

Browser-only users can still enter a token manually: sign in to the official web
client, open browser developer tools → Application/Storage → Local Storage, and
copy `injected_token` into the masked **Session token** field. The manual token field does not accept account passwords.

**Save token in keychain** uses the OS credential store; tokens are not serialized
into profile settings. Replace or reimport an expired session.

### Supported behavior

- Load saved network configurations; attach to existing cloud sessions and restore
  joined channels, topics, nicklists, and channel history.
- Theme-aware nick colors and IRC bold, italic, underline, and message colors,
  using the same renderer as the IRC backend.
- Send/receive channel messages, private messages, notices, and actions. Open an
  existing private conversation with `/query nick` to load its stored history.
- Load older messages, deduplicate history, reconnect with backoff, and synchronize
  read timestamps with Injected's `lastseen` setting.
- Use `/join`, `/part`, `/query`, `/msg`, `/me`, `/nick`, `/topic`, `/whois`, `/away`,
  and `/raw`. Hide/unhide and close are available through buffer context menus.
- Run `/connect` in a network's server buffer to connect an offline saved network;
  `/reconnect` retries that network and `/disconnect` disconnects it upstream.
  Disconnecting the WeeChatRS profile itself only closes this client session.

Configure/create networks in the official Injected client. This backend does not
implement native account management, billing, scripting, voice,
end-to-end encrypted private messages, or typing indicators. Protocol behavior is
based on the shipped Injected 1.1.3 client, rather than a published stable API.
Local mock-service tests cover authentication, heartbeat, history, sends, and clean
close; a real-account compatibility check is still required.


### Development checks

`cargo test --bin weechat-rs` covers the protocol and native bridge validation.
`node tools/test-injected-web-login.mjs` exercises the actual page bridge with
simulated login completion, rejected sessions, foreign frames, and token changes.
`cargo test --no-default-features --bin weechat-rs` checks builds without a webview.
