# Building WeeChatRS from Source

Requires **Rust stable 1.95+** (egui 0.37). Install via [rustup](https://rustup.rs) or update with `rustup update stable`.

## macOS

```bash
# Install Xcode command-line tools if not already present
xcode-select --install

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
# output: ./target/release/weechat-rs
```

To build the universal `.app` bundle (and optionally a drag-to-install DMG):

```bash
./build-macos.sh          # dist/macos/WeeChatRS.app
./build-macos.sh --dmg    # + dist/macos/WeeChatRS-<version>-macos-universal.dmg
```

Notifications only work from the `.app` bundle, not from `target/release/weechat-rs`.

## Linux

Besides Rust, a build needs a C compiler, perl and make (OpenSSL is built from
source), and WebKitGTK 4.1 for the [Injected](docs/injected.md) sign-in window,
which the default build includes:

```bash
sudo dnf install gcc perl make webkit2gtk4.1-devel                # Fedora, RHEL
sudo apt-get install build-essential perl libwebkit2gtk-4.1-dev   # Debian, Ubuntu

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
# output: ./target/release/weechat-rs
```

The binary then needs WebKitGTK 4.1 at run time too (`webkit2gtk4.1`,
`libwebkit2gtk-4.1-0`). To build without the sign-in window, and without WebKitGTK,
use `cargo build --release --no-default-features`; Injected connections then sign in
by importing the desktop app's session or with a token. Window system, D-Bus and
keyring support need no development packages either way: they are loaded at run
time or written in Rust.

That binary is for your own machine: it needs your distro's glibc or newer. For
zips that run on any distro from 2018 on, as released, use `./build-linux.sh`:

```bash
./build-linux.sh              # x86_64 and aarch64
./build-linux.sh --x86_64     # or --aarch64
# output: dist/linux/WeeChatRS-<version>-linux-<arch>.zip
```

It builds with [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) against
glibc 2.28, the same way the release workflow does, on any Linux distro or on macOS,
without Docker. Like the releases, these zips leave out the Injected sign-in window
(`--no-default-features`), so they don't depend on WebKitGTK. It also needs `zip`,
and fetches cargo-zigbuild and zig into `~/.cache` with python3 unless both are
installed; it says what is missing and how to install it. Each zip holds the
binary, a `weechatrs.desktop` entry, icons and `install.sh`. To install from the
zip:

```bash
unzip WeeChatRS-*-linux-x86_64.zip && cd WeeChatRS-*-linux-x86_64
./install.sh              # per-user, into ~/.local (no root)
./install.sh --uninstall
```

The desktop entry gives the app its launcher, dock and notification icon on
GNOME/KDE (the window's Wayland app_id is `weechatrs`). If you run the binary
directly instead, it installs the same per-user entry and icons on first launch
(pointing at the binary you ran) unless a system-wide `weechatrs.desktop` exists;
set `WEECHATRS_NO_DESKTOP_INTEGRATION=1` to turn that off.

## Windows — native build

Install Rust via [rustup](https://rustup.rs) with the MSVC toolchain (requires [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload):

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup default stable-x86_64-pc-windows-msvc

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
# output: target\release\weechat-rs.exe
```

The [Injected](docs/injected.md) sign-in window uses the Microsoft Edge WebView2
Runtime, part of Windows 11 and most Windows 10 installs; install it if the window
doesn't open. `--no-default-features` builds without it.

## Windows — WSL 2 (Linux build, runs on Windows desktop via WSLg)

WSL 2 on Windows 11 (or Windows 10 21H2+) with WSLg supports native GUI apps — no X server needed:

```bash
sudo apt-get install build-essential perl libwebkit2gtk-4.1-dev

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
./target/release/weechat-rs
```

## Cross-compile for Windows from macOS or Linux

`./build-windows.sh` builds the release zip,
`dist/windows/WeeChatRS-<version>-windows-x86_64.zip`, with `weechat-rs.exe` and
`WebView2Loader.dll`. It uses the MinGW
cross-compiler when it's installed:

```bash
brew install mingw-w64                            # macOS
sudo dnf install mingw64-gcc                      # Fedora
sudo apt-get install gcc-mingw-w64-x86-64         # Debian, Ubuntu

./build-windows.sh
```

Without it, the script builds in a container from `docker/Dockerfile.windows-x86_64`,
with Docker or Podman. `./build-windows.sh --aarch64` builds for Windows on ARM in a
container with llvm-mingw (not part of the releases). By hand, the x86_64 build is:

```bash
rustup target add x86_64-pc-windows-gnu
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo build --release --target x86_64-pc-windows-gnu
# output: target/x86_64-pc-windows-gnu/release/weechat-rs.exe
```

The .exe needs `WebView2Loader.dll` in the same folder, or Windows won't start it: the
Injected sign-in window links to it. The build leaves it in
`target/x86_64-pc-windows-gnu/release/build/webview2-com-sys-*/out/x64/`. A build with
`--no-default-features` has no sign-in window and doesn't need it.

## Flatpak and Microsoft Store (MSIX)

```bash
./build-flatpak.sh      # dist/flatpak/WeeChatRS-<version>-<arch>.flatpak
./build-msix.sh         # dist/msix/WeeChatRS-<version>-x64.msix
```

Both work on Linux and macOS (in a container where the tools aren't installed). See
[docs/packaging.md](docs/packaging.md) for the details, testing, and publishing on
Flathub and in the Microsoft Store.

## Run in development mode

```bash
cargo run
```

## Run (release — recommended for smooth rendering)

```bash
cargo run --release
```

## Tests

```bash
cargo test --bin weechat-rs
```

A few end-to-end tests need servers and are skipped by default; each one's doc
comment says what it needs. Run one with `-- --ignored <name>`, for example
`znc_end_to_end` against a local ZNC.

## Releases (GitHub Actions)

Pushing a version tag to GitHub builds every platform and uploads the builds, with `SHA256SUMS`, to a **draft** release:

```bash
# Cargo.toml says version = "0.9.0"
git tag -s v0.9.0 -m "v0.9.0"
git push github v0.9.0
```

- The tag must match the `Cargo.toml` version (`v0.9.0-rc1` for `0.9.0-rc1`), or the run stops before building. Tags with a `-` suffix become pre-releases.
- Builds: macOS universal DMG, Linux x86_64 and ARM64 zips, Windows x86_64 zip.
- The macOS DMG is only ad-hoc signed. Sign and notarize it locally, replace it on the draft and update its line in `SHA256SUMS`, then add the release notes and publish.
- Re-running the workflow for a tag replaces the files on the existing draft and keeps its notes.
- *Actions → Release → Run workflow* builds everything without a release; download the builds from the run's artifacts.

## Releases built locally

The same files can be built on a Mac, for a release somewhere other than GitHub
(such as a Gitea instance) or to check a release before tagging it. Build from a
clean checkout of the tag, so uncommitted work stays out:

```bash
git worktree add ../weechatrs-release v0.9.0 && cd ../weechatrs-release
./build-macos.sh --dmg      # macOS universal app and DMG (ad-hoc signed)
./build-linux.sh            # Linux x86_64 and aarch64 zips
./build-windows.sh          # Windows x86_64 zip
```

To sign the app with a Developer ID and notarize it, so Gatekeeper opens it on
other Macs without a warning:

```bash
IDENTITY="Developer ID Application: Your Name (TEAMID)"
codesign --force --options runtime --timestamp --sign "$IDENTITY" dist/macos/WeeChatRS.app
# notarytool takes a zip or DMG; store credentials once with
#   xcrun notarytool store-credentials <profile>
ditto -c -k --keepParent dist/macos/WeeChatRS.app /tmp/WeeChatRS.zip
xcrun notarytool submit /tmp/WeeChatRS.zip --keychain-profile <profile> --wait
xcrun stapler staple dist/macos/WeeChatRS.app
```

Then build the DMG from the signed app, and sign, notarize and staple the DMG the
same way. Finish with `shasum -a 256` over the files for `SHA256SUMS`.
