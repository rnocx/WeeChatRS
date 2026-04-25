#!/usr/bin/env bash
set -euo pipefail

# Builds x86_64 and aarch64 Linux binaries from macOS (or Linux).
#
# Requires:
#   brew install zig
#   cargo install cargo-zigbuild
#
# cargo-zigbuild uses Zig as a C cross-compiler, so no Linux toolchain or
# Docker is needed. It also handles vendored C deps (e.g. openssl-sys).

BINARY_NAME="weechat-rs"
OUT_DIR="dist/linux"

X86_TARGET="x86_64-unknown-linux-gnu"
ARM64_TARGET="aarch64-unknown-linux-gnu"

check_deps() {
    if ! command -v zig &>/dev/null; then
        echo "ERROR: 'zig' is not installed. Run: brew install zig"
        exit 1
    fi
    if ! command -v cargo-zigbuild &>/dev/null; then
        echo "ERROR: 'cargo-zigbuild' is not installed. Run: cargo install cargo-zigbuild"
        exit 1
    fi
}

mkdir -p "${OUT_DIR}"
check_deps

build_x86() {
    echo "==> Installing target ${X86_TARGET}..."
    rustup target add "${X86_TARGET}"
    echo "==> Building x86_64..."
    cargo zigbuild --release --target "${X86_TARGET}"
    cp "target/${X86_TARGET}/release/${BINARY_NAME}" "${OUT_DIR}/${BINARY_NAME}-x86_64-linux"
    echo "    -> ${OUT_DIR}/${BINARY_NAME}-x86_64-linux"
}

build_arm64() {
    echo "==> Installing target ${ARM64_TARGET}..."
    rustup target add "${ARM64_TARGET}"
    echo "==> Building aarch64..."
    cargo zigbuild --release --target "${ARM64_TARGET}"
    cp "target/${ARM64_TARGET}/release/${BINARY_NAME}" "${OUT_DIR}/${BINARY_NAME}-aarch64-linux"
    echo "    -> ${OUT_DIR}/${BINARY_NAME}-aarch64-linux"
}

package() {
    local arch="$1"
    local bin="${OUT_DIR}/${BINARY_NAME}-${arch}-linux"
    local tarball="${OUT_DIR}/${BINARY_NAME}-${arch}-linux.tar.gz"
    if [ -f "${bin}" ]; then
        tar -czf "${tarball}" -C "${OUT_DIR}" "$(basename "${bin}")"
        echo "    Packaged: ${tarball}"
    fi
}

# Parse args: default builds both; pass --x86 or --arm64 to build one.
BUILD_X86=true
BUILD_ARM64=true

for arg in "$@"; do
    case "$arg" in
        --x86|--x86_64) BUILD_X86=true; BUILD_ARM64=false ;;
        --arm64|--aarch64) BUILD_X86=false; BUILD_ARM64=true ;;
    esac
done

[ "$BUILD_X86"   = true ] && build_x86
[ "$BUILD_ARM64" = true ] && build_arm64

echo ""
echo "==> Packaging..."
[ "$BUILD_X86"   = true ] && package "x86_64"
[ "$BUILD_ARM64" = true ] && package "aarch64"

echo ""
echo "==> Done. Artifacts in ${OUT_DIR}/"
ls -lh "${OUT_DIR}/"
