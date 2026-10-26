#!/usr/bin/env bash
set -euo pipefail

APP_NAME="WeeChatRS"
BINARY_NAME="weechat-rs"
BUNDLE="${APP_NAME}.app"
ASSETS_DIR="assets"
OUT_DIR="dist/macos"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

echo "==> Installing targets..."
rustup target add x86_64-apple-darwin aarch64-apple-darwin

echo "==> Building x86_64..."
cargo build --release --target x86_64-apple-darwin

echo "==> Building aarch64..."
cargo build --release --target aarch64-apple-darwin

echo "==> Creating universal binary..."
mkdir -p "${OUT_DIR}"
lipo -create \
    "target/x86_64-apple-darwin/release/${BINARY_NAME}" \
    "target/aarch64-apple-darwin/release/${BINARY_NAME}" \
    -output "${OUT_DIR}/${BINARY_NAME}-universal"

echo "==> Assembling .app bundle..."
rm -rf "${OUT_DIR}/${BUNDLE}"
mkdir -p "${OUT_DIR}/${BUNDLE}/Contents/MacOS"
mkdir -p "${OUT_DIR}/${BUNDLE}/Contents/Resources"

cp "${OUT_DIR}/${BINARY_NAME}-universal" "${OUT_DIR}/${BUNDLE}/Contents/MacOS/${BINARY_NAME}"
cp "${ASSETS_DIR}/icon.icns" "${OUT_DIR}/${BUNDLE}/Contents/Resources/${APP_NAME}.icns"

cat > "${OUT_DIR}/${BUNDLE}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>com.github.rnocx.weechat-gui</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>${BINARY_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
</dict>
</plist>
EOF

chmod +x "${OUT_DIR}/${BUNDLE}/Contents/MacOS/${BINARY_NAME}"

echo "==> Signing .app bundle (ad-hoc)..."
codesign --force --deep --sign - "${OUT_DIR}/${BUNDLE}"

# Re-register so Finder, the Dock and Notification Center pick up a changed icon.
touch "${OUT_DIR}/${BUNDLE}"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "${OUT_DIR}/${BUNDLE}" || true

echo ""
echo "==> Done: ${OUT_DIR}/${BUNDLE}"
echo "    Universal binary: ${OUT_DIR}/${BINARY_NAME}-universal"

# --dmg: drag-to-install disk image. "Applications" must be a *symlink*;
# copying /Applications into the stage dir would pull every installed app in.
if [ "${1:-}" = "--dmg" ]; then
    DMG="${OUT_DIR}/${APP_NAME}-${VERSION}-macos-universal.dmg"
    STAGE="$(mktemp -d)"
    trap 'rm -rf "${STAGE}"' EXIT
    cp -R "${OUT_DIR}/${BUNDLE}" "${STAGE}/"
    ln -s /Applications "${STAGE}/Applications"
    echo "==> Creating ${DMG}..."
    hdiutil create -volname "${APP_NAME}" -srcfolder "${STAGE}" -ov -format UDZO "${DMG}" >/dev/null
    echo "    -> ${DMG}"
else
    echo ""
    echo "Run with --dmg to also build ${OUT_DIR}/${APP_NAME}-${VERSION}-macos-universal.dmg"
fi
