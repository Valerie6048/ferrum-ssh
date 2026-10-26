#!/bin/sh
# Builds dist/Ferrum.app and dist/Ferrum-<version>-macos-<arch>.dmg from a release build.
# Usage: packaging/build-macos.sh [target-triple]   (default: host)
set -eu

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TARGET=${1:-}
if [ -n "$TARGET" ]; then
    cargo build --release --locked --target "$TARGET"
    BIN=target/$TARGET/release/ferrum
    case "$TARGET" in aarch64*) ARCH=arm64 ;; *) ARCH=x86_64 ;; esac
else
    cargo build --release --locked
    BIN=target/release/ferrum
    ARCH=$(uname -m); [ "$ARCH" = "arm64" ] || ARCH=x86_64
fi

APP=dist/Ferrum.app
rm -rf dist/Ferrum.app dist/iconset.iconset
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" dist/iconset.iconset

cp "$BIN" "$APP/Contents/MacOS/ferrum"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# .icns from the pre-rendered PNGs
I=packaging/icons
cp $I/icon_16.png   dist/iconset.iconset/icon_16x16.png
cp $I/icon_32.png   dist/iconset.iconset/icon_16x16@2x.png
cp $I/icon_32.png   dist/iconset.iconset/icon_32x32.png
cp $I/icon_64.png   dist/iconset.iconset/icon_32x32@2x.png
cp $I/icon_128.png  dist/iconset.iconset/icon_128x128.png
cp $I/icon_256.png  dist/iconset.iconset/icon_128x128@2x.png
cp $I/icon_256.png  dist/iconset.iconset/icon_256x256.png
cp $I/icon_512.png  dist/iconset.iconset/icon_256x256@2x.png
cp $I/icon_512.png  dist/iconset.iconset/icon_512x512.png
cp $I/icon_1024.png dist/iconset.iconset/icon_512x512@2x.png
iconutil -c icns dist/iconset.iconset -o "$APP/Contents/Resources/Ferrum.icns"
rm -rf dist/iconset.iconset

# Ad-hoc signature so the bundle is internally consistent (not notarised).
codesign --force --deep --sign - "$APP"

DMG=dist/Ferrum-$VERSION-macos-$ARCH.dmg
rm -f "$DMG"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Ferrum" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
echo "$DMG"
