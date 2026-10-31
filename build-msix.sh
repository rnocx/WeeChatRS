#!/usr/bin/env bash
set -euo pipefail

# Packs the Windows build as an MSIX for the Microsoft Store.
#
#   ./build-msix.sh           -> dist/msix/WeeChatRS-<version>-x64.msix (unsigned; the Store signs it)
#   ./build-msix.sh --sign    signed with a self-signed test certificate, to install on a
#                             test PC: dist/msix/WeeChatRS-test.cer goes into that PC's
#                             "Trusted People" store first (see docs/packaging.md)
#
# The package identity comes from Partner Center (the app's Product identity page):
#   MSIX_IDENTITY_NAME   Package/Identity/Name, e.g. 12345rnocx.WeeChatRS
#   MSIX_PUBLISHER       Package/Identity/Publisher, e.g. CN=1A2B3C4D-…
#   MSIX_PUBLISHER_NAME  Package/Properties/PublisherDisplayName
# Without them, test values are used, which the Store won't accept.
#
# Takes weechat-rs.exe and WebView2Loader.dll from build-windows.sh's zip (and runs it
# if the zip isn't there), and packs with makemsix in a container from
# docker/Dockerfile.msix (Docker or Podman).

cd "$(dirname "$0")"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
OUT_DIR="dist/msix"
ZIP="dist/windows/WeeChatRS-${VERSION}-windows-x86_64.zip"

SIGN=0
for arg in "$@"; do
    case "$arg" in
        --sign) SIGN=1 ;;
        *) echo "Usage: $0 [--sign]"; exit 1 ;;
    esac
done

IDENTITY_NAME="${MSIX_IDENTITY_NAME:-WeeChatRS.Test}"
PUBLISHER="${MSIX_PUBLISHER:-CN=WeeChatRS Test}"
PUBLISHER_NAME="${MSIX_PUBLISHER_NAME:-rnocx}"
if [ -z "${MSIX_IDENTITY_NAME:-}" ] || [ -z "${MSIX_PUBLISHER:-}" ]; then
    echo "NOTE: MSIX_IDENTITY_NAME or MSIX_PUBLISHER not set; using test values the Store won't accept"
fi

engine=""
for e in docker podman; do
    if command -v "$e" >/dev/null && "$e" info >/dev/null 2>&1; then engine="$e"; break; fi
done
if [ -z "${engine}" ]; then
    echo "ERROR: needs a running Docker or Podman"
    exit 1
fi

if [ ! -f "${ZIP}" ]; then
    ./build-windows.sh
fi

STAGE="${OUT_DIR}/stage"
rm -rf "${STAGE}"
mkdir -p "${STAGE}/Assets"
unzip -q -o "${ZIP}" -d "${STAGE}"
for f in weechat-rs.exe WebView2Loader.dll; do
    [ -f "${STAGE}/${f}" ] || { echo "ERROR: ${f} not in ${ZIP}"; exit 1; }
done
# The Store wants a four-part version whose last part is 0, without a pre-release
# suffix: 0.9.4-rc1 packs as 0.9.4.0, like 0.9.4 itself.
MSIX_VERSION="${VERSION%%-*}.0"
sed -e "s|@IDENTITY_NAME@|${IDENTITY_NAME}|" \
    -e "s|@PUBLISHER@|${PUBLISHER}|" \
    -e "s|@PUBLISHER_DISPLAY_NAME@|${PUBLISHER_NAME}|" \
    -e "s|@VERSION@|${MSIX_VERSION}|" \
    packaging/msix/AppxManifest.xml > "${STAGE}/AppxManifest.xml"
cp assets/icon.iconset/icon_512x512.png "${OUT_DIR}/icon.png"

PACKAGE="${OUT_DIR}/WeeChatRS-${VERSION}-x64.msix"
echo "==> Packing ${PACKAGE} (${IDENTITY_NAME})..."
"${engine}" build -q -f docker/Dockerfile.msix -t weechat-rs-msix docker >/dev/null
"${engine}" run --rm -v "$PWD:/src" -e SIGN="${SIGN}" -e PUBLISHER="${PUBLISHER}" \
    -e STAGE="${STAGE}" -e PACKAGE="${PACKAGE}" -e OUT_DIR="${OUT_DIR}" weechat-rs-msix sh -euc '
    a="${STAGE}/Assets"
    # The tile and list icons, from the app icon.
    convert "${OUT_DIR}/icon.png" -resize 44x44 "$a/Square44x44Logo.png"
    convert "${OUT_DIR}/icon.png" -resize 50x50 "$a/StoreLogo.png"
    convert "${OUT_DIR}/icon.png" -resize 150x150 "$a/Square150x150Logo.png"
    convert "${OUT_DIR}/icon.png" -resize 150x150 -background none -gravity center -extent 310x150 "$a/Wide310x150Logo.png"
    rm -f "${PACKAGE}"
    makemsix pack -d "${STAGE}" -p "${PACKAGE}" >/dev/null
    if [ "${SIGN}" = 1 ]; then
        # A self-signed code-signing certificate whose subject is the publisher, as
        # Windows requires for installing the package.
        key="${OUT_DIR}/test.key"; cert="${OUT_DIR}/WeeChatRS-test.cer"
        if [ ! -f "$key" ]; then
            openssl req -x509 -newkey rsa:3072 -nodes -days 365 -keyout "$key" -out "${OUT_DIR}/test.pem" \
                -subj "/$(echo "${PUBLISHER}" | sed "s/^CN=/CN=/; s/, */\//g")" \
                -addext "extendedKeyUsage=codeSigning" -addext "basicConstraints=CA:FALSE" 2>/dev/null
            openssl x509 -in "${OUT_DIR}/test.pem" -outform DER -out "$cert"
        fi
        osslsigncode sign -certs "${OUT_DIR}/test.pem" -key "$key" -h sha256 \
            -in "${PACKAGE}" -out "${PACKAGE}.signed" >/dev/null
        mv "${PACKAGE}.signed" "${PACKAGE}"
        echo "    Signed with ${cert}"
    fi
'
rm -rf "${STAGE}" "${OUT_DIR}/icon.png"
echo ""
echo "==> Done: ${PACKAGE}"
ls -lh "${PACKAGE}"
