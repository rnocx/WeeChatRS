#!/usr/bin/env sh
# Zip a Linux build with its desktop entry, icons and a per-user install.sh, so the
# app gets a proper launcher/dock icon (Wayland matches app_id "weechatrs").
# Used by build-linux.sh and the GitHub release workflow; run from the repo root.
#
#   packaging/linux/package.sh <binary> <arch> <version> <out dir>
#     -> <out dir>/WeeChatRS-<version>-linux-<arch>.zip
set -eu

bin="$1"
arch="$2"
version="$3"
out_dir="$4"

name="WeeChatRS-${version}-linux-${arch}"
stage="${out_dir}/${name}"
rm -rf "${stage}" "${stage}.zip"
mkdir -p "${stage}/icons/256x256" "${stage}/icons/512x512"
install -m755 "${bin}" "${stage}/weechat-rs"
install -m644 packaging/linux/weechatrs.desktop "${stage}/"
install -m755 packaging/linux/install.sh "${stage}/"
install -m644 assets/icon.iconset/icon_256x256.png "${stage}/icons/256x256/weechatrs.png"
install -m644 assets/icon.iconset/icon_512x512.png "${stage}/icons/512x512/weechatrs.png"
(cd "${out_dir}" && zip -qr "${name}.zip" "${name}")
rm -rf "${stage}"
echo "    Packaged: ${stage}.zip"
