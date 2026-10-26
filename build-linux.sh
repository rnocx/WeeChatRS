#!/usr/bin/env bash
set -euo pipefail

# Builds x86_64 and aarch64 Linux binaries using Docker.
# Requires Docker with multi-platform support (docker buildx / QEMU for cross-arch).

BINARY_NAME="weechat-rs"
OUT_DIR="dist/linux"

check_deps() {
    if ! command -v docker &>/dev/null; then
        echo "ERROR: docker not found"
        exit 1
    fi
}

mkdir -p "${OUT_DIR}"
check_deps

extract_binary() {
    local image="$1"
    local src="$2"
    local dest="$3"
    local cid
    cid=$(docker create "${image}" --entrypoint /bin/true)
    docker cp "${cid}:${src}" "${dest}"
    docker rm "${cid}" > /dev/null
}

build_x86() {
    local image="weechat-rs-linux-x86_64-builder"
    echo "==> Building Docker image (x86_64)..."
    docker build --pull -f docker/Dockerfile.linux-x86_64 -t "${image}" .
    echo "==> Extracting x86_64 binary..."
    extract_binary "${image}" "/weechat-rs-linux-x86_64" "${OUT_DIR}/${BINARY_NAME}-x86_64-linux"
    echo "    -> ${OUT_DIR}/${BINARY_NAME}-x86_64-linux"
}

build_arm64() {
    local image="weechat-rs-linux-aarch64-builder"
    echo "==> Building Docker image (aarch64)..."
    docker build --pull -f docker/Dockerfile.linux-aarch64 -t "${image}" .
    echo "==> Extracting aarch64 binary..."
    extract_binary "${image}" "/weechat-rs-linux-aarch64" "${OUT_DIR}/${BINARY_NAME}-aarch64-linux"
    echo "    -> ${OUT_DIR}/${BINARY_NAME}-aarch64-linux"
}

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

# Zip with the binary, desktop entry, icons and a per-user install.sh, so the
# app gets a proper launcher/dock icon (Wayland matches app_id "weechatrs").
package() {
    local arch="$1"
    local bin="${OUT_DIR}/${BINARY_NAME}-${arch}-linux"
    local name="WeeChatRS-${VERSION}-linux-${arch}"
    local stage="${OUT_DIR}/${name}"
    if [ -f "${bin}" ]; then
        rm -rf "${stage}" "${stage}.zip"
        mkdir -p "${stage}/icons/256x256" "${stage}/icons/512x512"
        install -m755 "${bin}" "${stage}/${BINARY_NAME}"
        install -m644 packaging/linux/weechatrs.desktop "${stage}/"
        install -m755 packaging/linux/install.sh "${stage}/"
        install -m644 assets/icon.iconset/icon_256x256.png "${stage}/icons/256x256/weechatrs.png"
        install -m644 assets/icon.iconset/icon_512x512.png "${stage}/icons/512x512/weechatrs.png"
        (cd "${OUT_DIR}" && zip -qr "${name}.zip" "${name}")
        rm -rf "${stage}"
        echo "    Packaged: ${stage}.zip"
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
