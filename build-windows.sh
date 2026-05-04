#!/usr/bin/env bash
set -euo pipefail

# Builds Windows x86_64 and aarch64 .exe files using Docker.

BINARY_NAME="weechat-rs"
OUT_DIR="dist/windows"

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
    local image="weechat-rs-windows-x86_64-builder"
    echo "==> Building Docker image (x86_64)..."
    docker build -f docker/Dockerfile.windows-x86_64 -t "${image}" .
    echo "==> Extracting x86_64 binary..."
    extract_binary "${image}" "/weechat-rs-windows-x86_64.exe" "${OUT_DIR}/${BINARY_NAME}-x86_64-windows.exe"
    echo "    -> ${OUT_DIR}/${BINARY_NAME}-x86_64-windows.exe"
}

build_arm64() {
    local image="weechat-rs-windows-aarch64-builder"
    echo "==> Building Docker image (aarch64)..."
    docker build -f docker/Dockerfile.windows-aarch64 -t "${image}" .
    echo "==> Extracting aarch64 binary..."
    extract_binary "${image}" "/weechat-rs-windows-aarch64.exe" "${OUT_DIR}/${BINARY_NAME}-aarch64-windows.exe"
    echo "    -> ${OUT_DIR}/${BINARY_NAME}-aarch64-windows.exe"
}

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
echo "==> Done. Artifacts in ${OUT_DIR}/"
ls -lh "${OUT_DIR}/"
