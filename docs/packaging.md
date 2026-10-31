# Flatpak and Microsoft Store packages

Besides the release zips and the DMG ([BUILDING.md](../BUILDING.md)), WeeChatRS can be
packaged as a Flatpak, for Flathub, and as an MSIX, for the Microsoft Store. Neither
is published yet; this page says how to build them and what publishing takes.

## Flatpak

```bash
./build-flatpak.sh --lint     # -> dist/flatpak/WeeChatRS-<version>-<arch>.flatpak, and Flathub's linter
./build-flatpak.sh --flathub  # the same from the release's git tag, as Flathub builds it
flatpak install --user dist/flatpak/WeeChatRS-*.flatpak
flatpak run io.github.rnocx.WeeChatRS
```

The script builds with Flathub's own builder (`org.flatpak.Builder`), as Flathub
does: offline, with the screenshots and icon mirrored. With `--lint` it fails on
anything Flathub's linter reports. It runs where `flatpak` is installed, and
otherwise in a privileged container from `docker/Dockerfile.flatpak` (Docker or
Podman, so it also works on macOS). The first build downloads the GNOME runtime and
SDK, about 1.5 GB. The crates come from `Cargo.lock` through Flatpak's
`flatpak-cargo-generator` (pinned in the script).

It builds for the machine's own architecture: Flatpak's build sandbox doesn't work
under emulation. The release workflow builds both, x86_64 and ARM64, each on its own
runner.

The files are in `packaging/flatpak/`: the manifest, the desktop entry and the
AppStream metainfo that Flathub and software centres show.

- **App ID:** `io.github.rnocx.WeeChatRS`. In a Flatpak the window's app_id uses it,
  and the app doesn't install its own desktop entry.
- **Runtime:** GNOME 51. It includes WebKitGTK 4.1, so the Injected sign-in window is
  built in and needs nothing installed on the host.
- **Permissions** (`finish-args`), kept to what portals don't cover: network;
  Wayland, with X11 as a fallback; the GPU; the keyring (Secret Service), for
  connection passwords; and for SSH tunnels the SSH agent, plus a `~/.ssh` of the
  sandbox's own for `known_hosts`. Notifications, choosing files and folders, and
  opening links go through portals.
- **In the sandbox:**
  - SSH tunnels authenticate with keys from the SSH agent; keys in the real `~/.ssh`
    aren't visible.
  - *Import Injected session* can't find the Injected app's data by itself; use
    *Choose data folder…* (`~/.config/injected`) or sign in with the window.
  - `/np` runs `playerctl`, which isn't in the runtime, so it finds no player, and
    `/sysinfo` shows less (no `lspci`).
- **Version:** software centres show the newest `<release>` in the metainfo, so each
  release adds one (`type="development"` for a release candidate).

Checked in a container on ARM64: it builds with no linter findings, both from the
checkout and from a git tag; the app starts, and the sign-in window shows Injected's
login page. Not yet checked on a real desktop, or on x86_64.

### Publishing on Flathub

1. Release a stable version (Flathub takes no release candidates for a new app) with
   a `<release>` entry in the metainfo, and push its tag to GitHub: the app ID
   points to `github.com/rnocx/WeeChatRS`, and Flathub builds from there.
2. `./build-flatpak.sh --flathub --lint` builds that tag as Flathub will, and writes
   `dist/flatpak/flathub/`: `io.github.rnocx.WeeChatRS.yml` (building the tag and
   its commit from GitHub) and `cargo-sources.json`.
3. Fork [flathub/flathub](https://github.com/flathub/flathub) with *Copy the master
   branch only* unticked, branch from `new-pr`, add those two files, and open a pull
   request against `new-pr` titled "Add io.github.rnocx.WeeChatRS". Explain the
   permissions in it: the keyring for passwords, and the SSH agent and `.ssh` for
   tunnels. Comment `bot, build` for a test build on Flathub's builders.
4. Once accepted, Flathub creates `flathub/io.github.rnocx.WeeChatRS`; accept the
   invitation within a week (with 2FA on). Each release is then a pull request there
   with the new tag, commit, `cargo-sources.json` and metainfo `<release>`. Verify the
   app in Flathub's Developer Portal by logging in as `rnocx`, the owner of the GitHub
   repository.

## MSIX (Microsoft Store)

```bash
./build-msix.sh           # -> dist/msix/WeeChatRS-<version>-x64.msix, unsigned, for the Store
./build-msix.sh --sign    # signed with a test certificate, to install on a test PC
```

It packs `weechat-rs.exe` and `WebView2Loader.dll` from `build-windows.sh`'s zip
(building it first if needed) with `makemsix`, Microsoft's packing tool built from
source in `docker/Dockerfile.msix`, so it runs on macOS and Linux. The manifest is
`packaging/msix/AppxManifest.xml`; the tile icons are made from the app icon.

In the package, the app is a full-trust desktop app. Notifications use the package's
own identity (`<package family>!WeeChatRS`) instead of registering one, so they show
the Store app's name and icon. Settings and the WebView2 data live in the package's
own folders, apart from a zip install's.

### Testing on Windows

1. Build with `--sign`, and copy `dist/msix/WeeChatRS-<version>-x64.msix` and
   `dist/msix/WeeChatRS-test.cer` to the PC.
2. Install the certificate, once: double-click it, **Install Certificate…** →
   **Local Machine** → **Place all certificates in the following store** →
   **Trusted People**.
3. Double-click the `.msix` and choose **Install**.

The test key stays in `dist/msix/` (not committed); keep it to sign later test
builds, or a test PC needs the new certificate.

### Publishing in the Store

1. Sign in to [Partner Center](https://partner.microsoft.com/dashboard) with a
   developer account (free for individuals) and reserve the name **WeeChatRS**.
2. On the app's **Product identity** page, note *Package/Identity/Name*,
   *Package/Identity/Publisher* and *Package/Properties/PublisherDisplayName*, and
   build with them:

   ```bash
   MSIX_IDENTITY_NAME='…' MSIX_PUBLISHER='CN=…' MSIX_PUBLISHER_NAME='…' ./build-msix.sh
   ```
3. Create a submission: upload the unsigned `.msix` (the Store signs it), fill in
   the listing (description, screenshots, a privacy policy URL), the age rating
   questionnaire and pricing. The `runFullTrust` capability needs a short
   explanation: it's a desktop app.
4. Certification takes from hours to a few days. Store installs then update from the
   Store, with SmartScreen no longer warning.

Each release needs a higher version; the package's is `Cargo.toml`'s with `.0`
appended. Not yet checked on a real Windows PC.
