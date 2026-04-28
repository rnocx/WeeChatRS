# Building WeeChatRS from Source

Requires **Rust stable 1.75+**. Install via [rustup](https://rustup.rs) if needed.

## macOS

```bash
# Install Xcode command-line tools if not already present
xcode-select --install

git clone https://github.com/rnocx/WeeChatRS.git
cd WeeChatRS
cargo build --release
# output: ./target/release/weechat-rs
```

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
