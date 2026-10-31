#!/usr/bin/env bash
set -euo pipefail

# Builds the Flatpak from this checkout, for this machine's architecture.
#
#   ./build-flatpak.sh            -> dist/flatpak/WeeChatRS-<version>-<arch>.flatpak
#   ./build-flatpak.sh --lint     also runs Flathub's linter
#   ./build-flatpak.sh --x86_64   in the container, for another architecture (or
#                                 --aarch64), emulated where it isn't this machine's on the manifest and build
#
# Install the bundle with:  flatpak install --user dist/flatpak/WeeChatRS-*.flatpak
#
# Uses flatpak-builder when it's installed (Linux), and otherwise builds in a
# privileged container from docker/Dockerfile.flatpak (Docker or Podman). The GNOME
# runtime and SDK come from Flathub; the first build downloads about 1.5 GB.

cd "$(dirname "$0")"

APP_ID="io.github.rnocx.WeeChatRS"
MANIFEST="packaging/flatpak/${APP_ID}.yml"
OUT_DIR="dist/flatpak"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
# flatpak-builder-tools' cargo generator, pinned; it turns Cargo.lock into the list
# of crates the offline build gets.
TOOLS_COMMIT="41c20aa10819cdb2a4f3ca171758a96d1955c018"

LINT=0
PLATFORM=()
for arg in "$@"; do
    case "$arg" in
        --lint) LINT=1 ;;
        --inside) ;;
        --x86|--x86_64) PLATFORM=(--platform linux/amd64) ;;
        --arm64|--aarch64) PLATFORM=(--platform linux/arm64) ;;
        *) echo "Usage: $0 [--lint] [--x86_64|--aarch64]"; exit 1 ;;
    esac
done

# Everything below `build` runs where flatpak-builder is: here, or in the container.
build() {
    local arch state="${FLATPAK_STATE_DIR:-.flatpak-builder}"
    arch="$(flatpak --default-arch)"
    local tools="${state}/flatpak-builder-tools"
    if [ ! -d "${tools}/.git" ] || [ "$(git -C "${tools}" rev-parse HEAD)" != "${TOOLS_COMMIT}" ]; then
        rm -rf "${tools}"
        git clone -q https://github.com/flatpak/flatpak-builder-tools.git "${tools}"
        git -C "${tools}" checkout -q "${TOOLS_COMMIT}"
    fi
    echo "==> Listing crates from Cargo.lock..."
    python3 "${tools}/cargo/flatpak-cargo-generator.py" Cargo.lock -o packaging/flatpak/cargo-sources.json

    echo "==> Building ${APP_ID} ${VERSION} for ${arch}..."
    flatpak-builder --force-clean --disable-rofiles-fuse --install-deps-from=flathub \
        --state-dir="${state}" --repo="${state}/repo" "${state}/build" "${MANIFEST}"
    mkdir -p "${OUT_DIR}"
    local bundle="${OUT_DIR}/WeeChatRS-${VERSION}-${arch}.flatpak"
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        "${state}/repo" "${bundle}" "${APP_ID}"

    if [ "${LINT}" = 1 ]; then
        echo "==> Linting..."
        flatpak install -y --noninteractive flathub org.flatpak.Builder >/dev/null
        flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest "${MANIFEST}" || true
        flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo "${state}/repo" || true
    fi
    echo ""
    echo "==> Done: ${bundle}"
    ls -lh "${bundle}"
}

if [ "${1:-}" = "--inside" ] || { command -v flatpak-builder >/dev/null && [ ${#PLATFORM[@]} -eq 0 ]; }; then
    if ! flatpak remotes --columns=name | grep -qx flathub; then
        flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    fi
    build
    exit 0
fi

engine=""
for e in docker podman; do
    if command -v "$e" >/dev/null && "$e" info >/dev/null 2>&1; then engine="$e"; break; fi
done
if [ -z "${engine}" ]; then
    echo "ERROR: needs flatpak-builder, or a running Docker or Podman"
    echo "    Fedora: sudo dnf install flatpak-builder    Debian/Ubuntu: sudo apt-get install flatpak-builder"
    exit 1
fi
echo "==> Building in a container with ${engine}..."
"${engine}" build -q ${PLATFORM[@]+"${PLATFORM[@]}"} -f docker/Dockerfile.flatpak -t weechat-rs-flatpak-builder docker >/dev/null
# Flatpak's runtimes and the build state live in volumes (one set per architecture),
# so later builds reuse them.
suffix=""
[ ${#PLATFORM[@]} -gt 0 ] && suffix="-${PLATFORM[1]#linux/}"
"${engine}" run --rm --privileged ${PLATFORM[@]+"${PLATFORM[@]}"} \
    -v "$PWD:/src" \
    -v "weechat-rs-flatpak-system${suffix}:/var/lib/flatpak" \
    -v "weechat-rs-flatpak-state${suffix}:/state" \
    -e FLATPAK_STATE_DIR=/state \
    weechat-rs-flatpak-builder ./build-flatpak.sh --inside "$@"
