#!/usr/bin/env bash
set -euo pipefail

# Builds the Windows release zip from macOS or Linux.
#
#   ./build-windows.sh            x86_64, as released
#   ./build-windows.sh --aarch64  ARM64 instead (in a container, with llvm-mingw)
#   -> dist/windows/WeeChatRS-<version>-windows-<arch>.zip
#
# x86_64 uses the MinGW cross-compiler when it's installed, and otherwise builds in
# a container (Docker or Podman):
#   macOS          brew install mingw-w64
#   Fedora         sudo dnf install mingw64-gcc
#   Debian/Ubuntu  sudo apt-get install gcc-mingw-w64-x86-64

cd "$(dirname "$0")"

OUT_DIR="dist/windows"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

ARCH="x86_64"
for arg in "$@"; do
    case "$arg" in
        --x86|--x86_64) ARCH="x86_64" ;;
        --arm64|--aarch64) ARCH="aarch64" ;;
        *) echo "Usage: $0 [--x86_64|--aarch64]"; exit 1 ;;
    esac
done

if ! command -v zip >/dev/null; then
    echo "ERROR: zip not found; install it with your package manager"
    exit 1
fi
mkdir -p "${OUT_DIR}"
EXE="${OUT_DIR}/weechat-rs.exe"
# The Injected sign-in window (WebView2) needs Microsoft's loader DLL next to the
# .exe: a MinGW build links to it, so without it Windows won't start the program.
DLL="${OUT_DIR}/WebView2Loader.dll"
rm -f "${EXE}" "${DLL}"

# Build in a container from docker/Dockerfile.windows-<arch> and copy the .exe out.
build_in_container() {
    local engine
    if command -v docker >/dev/null; then engine=docker
    elif command -v podman >/dev/null; then engine=podman
    else
        echo "ERROR: needs the MinGW cross-compiler, or Docker or Podman (see the top of $0)"
        exit 1
    fi
    local image="weechat-rs-windows-${ARCH}-builder"
    echo "==> Building ${ARCH} with ${engine}..."
    "${engine}" build --pull -f "docker/Dockerfile.windows-${ARCH}" -t "${image}" .
    local cid
    cid=$("${engine}" create "${image}" /bin/true)
    "${engine}" cp "${cid}:/weechat-rs-windows-${ARCH}.exe" "${EXE}"
    "${engine}" cp "${cid}:/WebView2Loader-${ARCH}.dll" "${DLL}"
    "${engine}" rm "${cid}" > /dev/null
}

if [ "${ARCH}" = "x86_64" ] && command -v x86_64-w64-mingw32-gcc >/dev/null; then
    echo "==> Building x86_64 with the MinGW cross-compiler..."
    rustup target add x86_64-pc-windows-gnu >/dev/null
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
        cargo build --release --target x86_64-pc-windows-gnu
    cp "${TARGET_DIR}/x86_64-pc-windows-gnu/release/weechat-rs.exe" "${EXE}"
    # webview2-com-sys's build script leaves it in its out dir; the newest is this build's.
    LOADER=$(ls -t "${TARGET_DIR}"/x86_64-pc-windows-gnu/release/build/webview2-com-sys-*/out/x64/WebView2Loader.dll 2>/dev/null | head -1)
    if [ -z "${LOADER}" ]; then
        echo "ERROR: WebView2Loader.dll not found in the build output"
        exit 1
    fi
    cp "${LOADER}" "${DLL}"
else
    build_in_container
fi

# The same zip the release workflow makes: weechat-rs.exe and WebView2Loader.dll.
ZIP="${OUT_DIR}/WeeChatRS-${VERSION}-windows-${ARCH}.zip"
rm -f "${ZIP}"
zip -qj "${ZIP}" "${EXE}" "${DLL}"
rm -f "${EXE}" "${DLL}"

echo ""
echo "==> Done: ${ZIP}"
ls -lh "${ZIP}"
