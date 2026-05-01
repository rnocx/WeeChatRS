#!/usr/bin/env bash
set -euo pipefail

# Builds a Windows x86_64 .exe using Docker (docker/Dockerfile.windows-x86_64).

BINARY_NAME="weechat-rs"
OUT_DIR="dist/windows"
IMAGE="weechat-rs-windows-builder"

check_deps() {
    if ! command -v docker &>/dev/null; then
        echo "ERROR: docker not found"
        exit 1
    fi
}

mkdir -p "${OUT_DIR}"
check_deps

echo "==> Building Docker image..."
docker build -f docker/Dockerfile.windows-x86_64 -t "${IMAGE}" .

echo "==> Extracting binary..."
CID=$(docker create "${IMAGE}" --entrypoint /bin/true)
docker cp "${CID}:/weechat-rs-windows-x86_64.exe" "${OUT_DIR}/${BINARY_NAME}-x86_64-windows.exe"
docker rm "${CID}" > /dev/null

echo "    -> ${OUT_DIR}/${BINARY_NAME}-x86_64-windows.exe"
echo ""
echo "==> Done. Artifacts in ${OUT_DIR}/"
ls -lh "${OUT_DIR}/"
