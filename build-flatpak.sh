#!/usr/bin/env bash
set -euo pipefail

# Builds the Flatpak for this machine's architecture, with Flathub's own builder
# (org.flatpak.Builder) and its linter.
#
#   ./build-flatpak.sh            from this checkout
#                                 -> dist/flatpak/WeeChatRS-<version>-<arch>.flatpak
#   ./build-flatpak.sh --flathub  as a Flathub submission builds it: from the release's
#                                 git tag, offline, from dist/flatpak/flathub/ (the
#                                 manifest and cargo-sources.json it writes there)
#   ./build-flatpak.sh --lint     also runs Flathub's linter on the manifest and build
#
# --flathub builds the tag v<version> from FLATHUB_GIT_URL (by default the GitHub
# repository, which the app ID points to); FLATHUB_TAG picks another tag. The tag
# must exist here and at that URL.
#
# Install the bundle with:  flatpak install --user dist/flatpak/WeeChatRS-*.flatpak
#
# Runs where flatpak is installed (Linux), and otherwise in a privileged container
# from docker/Dockerfile.flatpak (Docker or Podman). Flatpak's build sandbox doesn't
# work under emulation, so other architectures need a machine of their own (Flathub
# and CI build both). The GNOME runtime and SDK come from Flathub; the first build
# downloads about 1.5 GB.

cd "$(dirname "$0")"

APP_ID="io.github.rnocx.WeeChatRS"
MANIFEST="packaging/flatpak/${APP_ID}.yml"
OUT_DIR="dist/flatpak"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
# flatpak-builder-tools' cargo generator, pinned; it turns Cargo.lock into the list
# of crates the offline build gets.
TOOLS_COMMIT="41c20aa10819cdb2a4f3ca171758a96d1955c018"
FLATHUB_GIT_URL="${FLATHUB_GIT_URL:-https://github.com/rnocx/WeeChatRS.git}"
FLATHUB_TAG="${FLATHUB_TAG:-v${VERSION}}"

LINT=0
FLATHUB=0
for arg in "$@"; do
    case "$arg" in
        --lint) LINT=1 ;;
        --flathub) FLATHUB=1 ;;
        --inside) ;;
        *) echo "Usage: $0 [--flathub] [--lint]"; exit 1 ;;
    esac
done

# The manifest a Flathub submission has: this one, building the tag from git.
flathub_manifest() {
    local dir="$1" commit
    commit="$(git rev-list -n 1 "${FLATHUB_TAG}" 2>/dev/null)" || {
        echo "ERROR: no tag ${FLATHUB_TAG} here (set FLATHUB_TAG)"
        exit 1
    }
    mkdir -p "${dir}"
    python3 - "${MANIFEST}" "${dir}/${APP_ID}.yml" "${FLATHUB_GIT_URL}" "${FLATHUB_TAG}" "${commit}" <<'PY'
import re, sys
src, dst, url, tag, commit = sys.argv[1:]
text = open(src).read()
# Drop the header comment, which describes the local build.
text = re.sub(r"\A(#.*\n)+", "", text)
local = re.search(r"      - type: dir\n(        .*\n)+", text)
git = (f"      - type: git\n        url: {url}\n        tag: {tag}\n        commit: {commit}\n")
text = text[:local.start()] + git + text[local.end():]
open(dst, "w").write(text)
PY
    cp packaging/flatpak/cargo-sources.json "${dir}/"
    echo "    ${dir}/${APP_ID}.yml: ${FLATHUB_GIT_URL} ${FLATHUB_TAG} (${commit})"
}

# Everything below `build` runs where flatpak is: here, or in the container.
build() {
    local arch state="${FLATPAK_STATE_DIR:-.flatpak-builder}" manifest="${MANIFEST}"
    arch="$(flatpak --default-arch)"
    git config --global --add safe.directory "$PWD" 2>/dev/null || true
    # The builder runs sandboxed and can't install what the manifest needs, so that's
    # done here, in the user installation it uses: the runtime and SDK it names, and the Rust extension of that SDK's
    # Freedesktop base (GNOME 51 is on 26.08).
    local runtime
    runtime="$(sed -n "s/^runtime-version: '\(.*\)'/\1/p" "${MANIFEST}")"
    flatpak install --user -y --noninteractive flathub org.flatpak.Builder \
        "org.gnome.Platform//${runtime}" "org.gnome.Sdk//${runtime}" \
        org.freedesktop.Sdk.Extension.rust-stable//26.08 >/dev/null

    local tools="${state}/flatpak-builder-tools"
    if [ ! -d "${tools}/.git" ] || [ "$(git -C "${tools}" rev-parse HEAD)" != "${TOOLS_COMMIT}" ]; then
        rm -rf "${tools}"
        git clone -q https://github.com/flatpak/flatpak-builder-tools.git "${tools}"
        git -C "${tools}" checkout -q "${TOOLS_COMMIT}"
    fi
    echo "==> Listing crates from Cargo.lock..."
    python3 "${tools}/cargo/flatpak-cargo-generator.py" Cargo.lock -o packaging/flatpak/cargo-sources.json
    if [ "${FLATHUB}" = 1 ]; then
        echo "==> Writing the Flathub manifest..."
        flathub_manifest "${OUT_DIR}/flathub"
        manifest="${OUT_DIR}/flathub/${APP_ID}.yml"
    fi

    echo "==> Building ${APP_ID} ${VERSION} for ${arch}..."
    # As Flathub's builders do: screenshots and icons mirrored with full URLs (and
    # committed to the repo's screenshots/<arch> branch), as the linter requires.
    flatpak run org.flatpak.Builder --user --force-clean --disable-rofiles-fuse \
        --compose-url-policy=full --mirror-screenshots-url=https://dl.flathub.org/media \
        --state-dir="${state}" --repo="${state}/repo" "${state}/build" "${manifest}"
    mkdir -p "${OUT_DIR}"
    local bundle="${OUT_DIR}/WeeChatRS-${VERSION}-${arch}.flatpak"
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        "${state}/repo" "${bundle}" "${APP_ID}"

    echo ""
    echo "==> Done: ${bundle}"
    ls -lh "${bundle}"

    if [ "${LINT}" = 1 ]; then
        echo "==> Linting..."
        local failed=0
        # From the state dir: the check makes a .flatpak-builder folder where it runs,
        # and the sandboxed linter can't write into a checkout it doesn't own (CI).
        (cd "${state}" && flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest "${OLDPWD}/${manifest}") || failed=1
        flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo "${state}/repo" || failed=1
        if [ "${failed}" = 1 ]; then
            echo "ERROR: Flathub's linter found problems (above)"
            exit 1
        fi
        echo "    No problems found."
    fi
}

if [ "${1:-}" = "--inside" ] || command -v flatpak >/dev/null; then
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    build
    exit 0
fi

engine=""
for e in docker podman; do
    if command -v "$e" >/dev/null && "$e" info >/dev/null 2>&1; then engine="$e"; break; fi
done
if [ -z "${engine}" ]; then
    echo "ERROR: needs flatpak, or a running Docker or Podman"
    echo "    Fedora: sudo dnf install flatpak    Debian/Ubuntu: sudo apt-get install flatpak"
    exit 1
fi
echo "==> Building in a container with ${engine}..."
"${engine}" build -q -f docker/Dockerfile.flatpak -t weechat-rs-flatpak-builder docker >/dev/null
# Flatpak's runtimes and the build state live in volumes, so later builds reuse them.
# The sandboxed builder asks flatpak outside it for the runtime and to run each build
# step, over D-Bus and with a runtime dir, which a desktop session has and a container
# doesn't.
"${engine}" run --rm --privileged \
    -v "$PWD:/src" \
    -v weechat-rs-flatpak-user:/root/.local/share/flatpak \
    -v weechat-rs-flatpak-state:/state \
    -e FLATPAK_STATE_DIR=/state \
    -e XDG_RUNTIME_DIR=/run/user/0 \
    -e FLATHUB_GIT_URL="${FLATHUB_GIT_URL}" -e FLATHUB_TAG="${FLATHUB_TAG}" \
    weechat-rs-flatpak-builder sh -c 'mkdir -p -m 700 /run/user/0 && exec dbus-run-session -- ./build-flatpak.sh --inside "$@"' sh "$@"
