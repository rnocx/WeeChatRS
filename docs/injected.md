## Injected backend (experimental)

Pick **Injected** in Connections and WeeChatRS opens the networks you already have on your Injected account, in its own native window. It connects to `injected-irc.com:443` over TLS by default. No Electron needed.

### Signing in

1. Add or edit an **Injected** connection.
2. Click **Sign in to Injected…**. A separate window opens Injected's official HTTPS login page.
3. Enter your username and password there, pass the CAPTCHA and any second-factor prompt, and wait for the window to close.
4. Check the account name it found, tick **Save token in keychain** if you want, then save and connect.

Your login and two-factor steps happen on Injected's own website. When they're done, the session comes back to WeeChatRS through a private pipe from the helper process, and WeeChatRS checks it with Injected before it fills in the form. The helper's web session is thrown away afterwards. It never stores your password or writes the token to a file. Close the window or click **Cancel sign-in** to stop. Sign-in only works against `injected-irc.com:443` with TLS, so a custom endpoint still needs a token.

The window comes from the `injected-web-login` Cargo feature, which is on by default. It's WKWebView on macOS, WebView2 on Windows and WebKitGTK 4.1 on Linux. The Linux release zips keep WebKitGTK out of `weechat-rs` itself. There the window is a separate program, `weechat-rs-login`, next to it, and it needs glibc 2.34 or later plus WebKitGTK 4.1 (`libwebkit2gtk-4.1-0` on Debian and Ubuntu, `webkit2gtk4.1` on Fedora, `webkit2gtk-4.1` on Arch). If they're missing, the app says so, and importing a session or pasting a token still works. See the [build requirements](../BUILDING.md).

Tested so far: the official form and a passed CAPTCHA, by eye, on macOS. The full session handoff, and WebAuthn or other second-factor prompts, still need a try with a real account.

### Without the sign-in window

Sign in to the **Injected desktop app**, then click **Import Injected session** in WeeChatRS. It reads the desktop app's local session and checks it with Injected. It doesn't look through browser profiles, and nothing is saved until you save the connection. If the app is installed somewhere unusual, **Choose data folder…** takes either its app support folder or the `Local Storage/leveldb` folder inside it. Import found nothing readable? Sign in to the desktop app again and retry. Older, compacted storage sometimes has no token the importer can read.

If you only use Injected in a browser, you can paste the token by hand. Sign in to the official web client, open the developer tools (Application or Storage, then Local Storage), and copy `injected_token` into the masked **Session token** field. That field takes a token, never your account password.

**Save token in keychain** puts the token in the system's credential store. It's never written into the profile settings. When a session expires, paste or import a new one.

### What works

- Your saved networks load. WeeChatRS attaches to the sessions already running in the cloud and brings back joined channels, topics, nick lists and channel history.
- Nick colours follow the theme, and IRC bold, italic, underline and colours are drawn with the same renderer as the IRC backend.
- Channel messages, private messages, notices and actions go both ways. `/query nick` opens an existing private conversation with its stored history.
- *Load older messages* works, history isn't shown twice, dropped connections retry with backoff, and read positions sync through Injected's `lastseen` setting.
- Commands: `/join`, `/part`, `/query`, `/msg`, `/me`, `/nick`, `/topic`, `/whois`, `/ctcp`, `/list`, `/away` and `/raw`. Hide, unhide and close are in each buffer's right-click menu.
- In a network's server buffer, `/connect` connects a saved network that's offline, `/reconnect` retries it and `/disconnect` takes it offline upstream. Disconnecting the WeeChatRS profile only closes this client's session.

Create and configure networks in the official Injected client. This backend has no account management, billing, scripting, voice, end-to-end encrypted private messages or typing indicators. Injected has no published, stable API, so the protocol comes from what the Injected 1.1.3 client does. Local tests against a mock service cover sign-in, the heartbeat, history, sending and a clean close. A check against a real account is still to do.

### Sharing text as a paste

Paste several lines into an Injected conversation and the review dialog offers **Share as Injected paste**. That uploads the original text to the connection's `/api/paste` endpoint, with its session token, and sends only the returned link to the conversation. `/paste <text>` opens the same dialog, even for one line.

Indentation, blank lines and line endings survive the upload. Nothing goes up until you pick the share button, and you can still send the lines as messages, join them into one input line, or cancel. A failed upload keeps the text, so you can retry. If the upload worked but the conversation dropped meanwhile, the link stays there to copy, or to send after you reconnect, and sending it doesn't upload the text a second time.

Only Injected connections get this. WeeChat, IRC, soju and ZNC share pastes anonymously through a PrivateBin server instead (Settings → File sharing). The Injected server decides expiry, size and rate limits, so the client can't promise how long a paste lasts.

### Development checks

`cargo test --bin weechat-rs` covers the protocol and the checks on the native bridge. `node tools/test-injected-web-login.mjs` runs the real page bridge through a simulated login, rejected sessions, foreign frames and token changes. `cargo test --no-default-features --bin weechat-rs` makes sure it builds without a web view.
