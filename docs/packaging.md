# Flatpak and Microsoft Store packages

Besides the release zips and the DMG ([BUILDING.md](../BUILDING.md)), WeeChatRS can be
packaged as a Flatpak, for Flathub, and as an MSIX, for the Microsoft Store. Neither
is published yet; this page says how to build them and what publishing takes.

## Flatpak

```bash
./build-flatpak.sh            # -> dist/flatpak/WeeChatRS-<version>-<arch>.flatpak
./build-flatpak.sh --lint     # also runs Flathub's linter
flatpak install --user dist/flatpak/WeeChatRS-*.flatpak
flatpak run io.github.rnocx.WeeChatRS
```

The script uses `flatpak-builder` where it's installed, and otherwise a privileged
container from `docker/Dockerfile.flatpak` (Docker or Podman, so it also works on
macOS). It builds for the machine's own architecture; the first build downloads the
GNOME runtime and SDK, about 1.5 GB. The crates come from `Cargo.lock` through
Flatpak's `flatpak-cargo-generator` (pinned in the script), since Flathub builds
offline.

The files are in `packaging/flatpak/`: the manifest, the desktop entry and the
AppStream metainfo that Flathub and software centres show.

- **App ID:** `io.github.rnocx.WeeChatRS`. In a Flatpak the window's app_id and the
  notifications use it, and the app doesn't install its own desktop entry.
- **Runtime:** GNOME 51. It includes WebKitGTK 4.1, so the Injected sign-in window is
  built in and needs nothing installed on the host.
- **Permissions** (`finish-args`): network; Wayland, with X11 as a fallback; the GPU;
  desktop notifications; the keyring (Secret Service); read access to the Injected
  desktop app's data, for session import; and the SSH agent and `~/.ssh` (read-only),
  for SSH tunnels. File uploads and opening links go through the desktop portals.
- **Not in the sandbox:** `/np` runs `playerctl`, which isn't in the runtime, so it
  finds no player. The system info for `/sysinfo` is partial (no `lspci`). SSH can't
  add new hosts to `~/.ssh/known_hosts`, which is read-only there.

Checked in a container on ARM64: the app starts, and the sign-in window shows
Injected's login page. Not yet checked on a real desktop, or on x86_64.

### Publishing on Flathub

1. The license files (`LICENSE-MIT`, `LICENSE-APACHE`) must be in the release that
   the manifest builds; Flathub wants the license with the source.
2. Fork [flathub/flathub](https://github.com/flathub/flathub), branch from `new-pr`,
   and add `io.github.rnocx.WeeChatRS.yml` and `cargo-sources.json`. The manifest
   is this repository's, with the `dir` source replaced by the release:

   ```yaml
   - type: git
     url: https://github.com/rnocx/WeeChatRS.git
     tag: v0.9.3
     commit: <the tag's commit>
   ```
3. Open a pull request against `new-pr`. Reviewers check the permissions; Flathub
   builds both architectures and mirrors the screenshots (the linter's
   `appstream-external-screenshot-url` error goes away there).
4. Once merged, Flathub creates `flathub/io.github.rnocx.WeeChatRS`. Each release is
   a pull request there with the new tag, commit, `cargo-sources.json` and a
   `<release>` entry in the metainfo. Verifying the app (the checkmark) goes through
   the GitHub account `rnocx`, which owns the ID.

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
