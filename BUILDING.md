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

Install the required system libraries before building:

```bash
sudo apt-get install \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev libdbus-1-dev pkg-config

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
# output: ./target/release/weechat-rs
```

`./build-linux.sh` builds x86_64 and aarch64 in Docker and writes
`dist/linux/WeeChatRS-<version>-linux-<arch>.zip`, containing the binary, a
`weechatrs.desktop` entry, icons and `install.sh`. To install from the zip:

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

## Windows — WSL 2 (Linux build, runs on Windows desktop via WSLg)

WSL 2 on Windows 11 (or Windows 10 21H2+) with WSLg supports native GUI apps — no X server needed:

```bash
sudo apt-get install \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev libdbus-1-dev pkg-config

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
./target/release/weechat-rs
```

## Cross-compile for Windows from macOS

```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu

# Add to .cargo/config.toml:
# [target.x86_64-pc-windows-gnu]
# linker = "x86_64-w64-mingw32-gcc"

cargo build --release --target x86_64-pc-windows-gnu
# output: target/x86_64-pc-windows-gnu/release/weechat-rs.exe
```

## Cross-compile for Windows from Linux or WSL (Docker)

```bash
docker build --platform linux/amd64 \
  -f docker/Dockerfile.windows-x86_64 \
  -t weechat-windows-x86_64 .

CONTAINER=$(docker create weechat-windows-x86_64)
docker cp "$CONTAINER:/weechat-rs-windows-x86_64.exe" ./weechat-rs.exe
docker rm "$CONTAINER"
```

## Run in development mode

```bash
cargo run
```

## Run (release — recommended for smooth rendering)

```bash
cargo run --release
```

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
