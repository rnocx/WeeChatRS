#!/usr/bin/env sh
# Install WeeChatRS for the current user (no root needed):
#   ~/.local/bin/weechat-rs
#   ~/.local/bin/weechat-rs-login (the Injected sign-in window, if in the zip)
#   ~/.local/share/applications/weechatrs.desktop
#   ~/.local/share/icons/hicolor/<size>/apps/weechatrs.png
# Run with --uninstall to remove those files again.
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
SIZES="256x256 512x512"

# Portable stand-in for GNU `install -Dm<mode> src dest` (BusyBox lacks -D).
put() {
    mkdir -p "$(dirname "$3")"
    cp "$2" "$3"
    chmod "$1" "$3"
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN_DIR/weechat-rs" "$BIN_DIR/weechat-rs-login" "$DATA_DIR/applications/weechatrs.desktop"
    for s in $SIZES; do rm -f "$DATA_DIR/icons/hicolor/$s/apps/weechatrs.png"; done
    echo "WeeChatRS removed."
    exit 0
fi

put 755 "$HERE/weechat-rs" "$BIN_DIR/weechat-rs"
# weechat-rs starts it from its own folder.
if [ -f "$HERE/weechat-rs-login" ]; then
    put 755 "$HERE/weechat-rs-login" "$BIN_DIR/weechat-rs-login"
fi
# Absolute Exec path, so the menu entry works even if BIN_DIR is not on PATH.
mkdir -p "$DATA_DIR/applications"
sed "s|^Exec=.*|Exec=$BIN_DIR/weechat-rs|" "$HERE/weechatrs.desktop" > "$DATA_DIR/applications/weechatrs.desktop"
chmod 644 "$DATA_DIR/applications/weechatrs.desktop"
for s in $SIZES; do
    put 644 "$HERE/icons/$s/weechatrs.png" "$DATA_DIR/icons/hicolor/$s/apps/weechatrs.png"
done

# Refresh caches where the tools exist; harmless if they don't.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA_DIR/applications" >/dev/null 2>&1 || true
# As xdg-icon-resource does: rebuild the icon cache only where the theme has an
# index. A per-user hicolor folder usually has neither, and is read directly; a cache
# would cover every app's icons there, and gtk-update-icon-cache refuses ("the
# generated cache was invalid") when any of them is off. Bumping the folder's time
# makes GTK treat an existing cache as stale.
if [ -f "$DATA_DIR/icons/hicolor/index.theme" ]; then
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$DATA_DIR/icons/hicolor" >/dev/null 2>&1 || true
else
    touch "$DATA_DIR/icons/hicolor"
fi

echo "WeeChatRS installed to $BIN_DIR/weechat-rs"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH, so run it from the app menu or add it to PATH." ;;
esac
