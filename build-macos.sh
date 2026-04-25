#!/usr/bin/env bash
set -euo pipefail

APP_NAME="WeeChatRS"
BINARY_NAME="weechat-rs"
BUNDLE="${APP_NAME}.app"
ASSETS_DIR="assets"
OUT_DIR="dist/macos"

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
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
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

echo ""
echo "==> Done: ${OUT_DIR}/${BUNDLE}"
echo "    Universal binary: ${OUT_DIR}/${BINARY_NAME}-universal"
echo ""
echo "To create a DMG (optional):"
echo "    hdiutil create -volname '${APP_NAME}' -srcfolder '${OUT_DIR}/${BUNDLE}' -ov -format UDZO '${OUT_DIR}/${APP_NAME}.dmg'"
