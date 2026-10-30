#!/usr/bin/env bash
set -euo pipefail

# Builds the Linux release zips with cargo-zigbuild, linked against glibc 2.28 so
# they run on any distro from 2018 on (Debian 10, RHEL 8, Ubuntu 20.04 and newer),
# whatever distro builds them. The release workflow builds them the same way.
# Runs on any Linux distro or on macOS; no Docker.
#
#   ./build-linux.sh              x86_64 and aarch64
#   ./build-linux.sh --x86_64     one of them (or --aarch64)
#   -> dist/linux/WeeChatRS-<version>-linux-<arch>.zip
#
# Needs rustup, a C compiler, perl and make (OpenSSL is built from source), zip,
# and python3 to fetch cargo-zigbuild and zig into ~/.cache unless both are
# installed already. It says what is missing, and how to install it.

cd "$(dirname "$0")"

GLIBC="2.28"
# Kept the same as .github/workflows/release.yml.
ZIGBUILD_VERSION="0.24.0"
ZIG_VERSION="0.16.0"
OUT_DIR="dist/linux"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

ARCHES=(x86_64 aarch64)
for arg in "$@"; do
    case "$arg" in
        --x86|--x86_64) ARCHES=(x86_64) ;;
        --arm64|--aarch64) ARCHES=(aarch64) ;;
        *) echo "Usage: $0 [--x86_64|--aarch64]"; exit 1 ;;
    esac
done

# The command that installs `packages` here, for the error message.
install_hint() {
    local packages="$1"
    if [ "$(uname -s)" = "Darwin" ]; then
        echo "xcode-select --install   (and: brew install ${packages})"
    elif command -v dnf >/dev/null; then echo "sudo dnf install ${packages}"
    elif command -v apt-get >/dev/null; then echo "sudo apt-get install ${packages}"
    elif command -v pacman >/dev/null; then echo "sudo pacman -S ${packages}"
    elif command -v zypper >/dev/null; then echo "sudo zypper install ${packages}"
    else echo "install with your package manager: ${packages}"
    fi
}

check_deps() {
    local missing=()
    for tool in perl make zip; do
        command -v "$tool" >/dev/null || missing+=("$tool")
    done
    # Rust's build scripts are compiled for this machine with its C compiler.
    command -v cc >/dev/null || missing+=(gcc)
    if [ ${#missing[@]} -gt 0 ]; then
        echo "ERROR: missing ${missing[*]}"
        echo "    $(install_hint "${missing[*]}")"
        exit 1
    fi
    # OpenSSL's build script needs these; some distros (Fedora) leave them out of
    # the base perl package.
    if ! perl -MFindBin -MIPC::Cmd -MFile::Compare -MFile::Copy -e1 2>/dev/null; then
        echo "ERROR: perl lacks modules OpenSSL's build needs (FindBin, IPC::Cmd, File::Compare, File::Copy)"
        if command -v dnf >/dev/null; then
            echo "    sudo dnf install perl-FindBin perl-IPC-Cmd perl-File-Compare perl-File-Copy"
        else
            echo "    $(install_hint perl)"
        fi
        exit 1
    fi
    if ! command -v rustup >/dev/null; then
        echo "ERROR: rustup not found; install Rust from https://rustup.rs"
        exit 1
    fi
}

# cargo-zigbuild and zig: the installed ones, or the pinned versions in a venv.
setup_zigbuild() {
    if command -v cargo-zigbuild >/dev/null && command -v zig >/dev/null; then
        return
    fi
    local venv="${XDG_CACHE_HOME:-$HOME/.cache}/weechatrs/zigbuild-${ZIGBUILD_VERSION}-zig-${ZIG_VERSION}"
    if [ ! -x "${venv}/bin/cargo-zigbuild" ]; then
        if ! command -v python3 >/dev/null; then
            echo "ERROR: cargo-zigbuild and zig not found, and no python3 to fetch them"
            echo "    $(install_hint python3)"
            exit 1
        fi
        echo "==> Fetching cargo-zigbuild ${ZIGBUILD_VERSION} and zig ${ZIG_VERSION} into ${venv}..."
        if ! python3 -m venv "${venv}"; then
            echo "ERROR: python3 can't create a venv"
            echo "    $(install_hint python3-venv)"
            exit 1
        fi
        "${venv}/bin/pip" install --quiet "cargo-zigbuild==${ZIGBUILD_VERSION}" "ziglang==${ZIG_VERSION}"
    fi
    export PATH="${venv}/bin:${PATH}"
}

check_deps
setup_zigbuild
mkdir -p "${OUT_DIR}"

for arch in "${ARCHES[@]}"; do
    target="${arch}-unknown-linux-gnu"
    echo "==> Building ${arch} (glibc ${GLIBC})..."
    rustup target add "${target}" >/dev/null
    # Keep these portable builds free of WebKitGTK and its newer system ABI.
    # Native Linux builds can enable web sign-in (the Cargo default); see BUILDING.md.
    cargo zigbuild --release --no-default-features --target "${target}.${GLIBC}"
    packaging/linux/package.sh "${TARGET_DIR}/${target}/release/weechat-rs" "${arch}" "${VERSION}" "${OUT_DIR}"
done

echo ""
echo "==> Done. Artifacts in ${OUT_DIR}/"
ls -lh "${OUT_DIR}/"
