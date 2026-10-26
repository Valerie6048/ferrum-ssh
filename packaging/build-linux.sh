#!/bin/sh
# Builds dist/Ferrum-<version>-linux-x86_64.tar.gz and (if appimagetool is on PATH) the AppImage.
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release --locked
mkdir -p dist
NAME=Ferrum-$VERSION-linux-x86_64

# tar.gz
ROOT=dist/$NAME
rm -rf "$ROOT"
mkdir -p "$ROOT"
cp target/release/ferrum "$ROOT/"
cp packaging/linux/ferrum.desktop "$ROOT/"
cp packaging/icons/icon_256.png "$ROOT/ferrum.png"
cp README.md LICENSE* "$ROOT/" 2>/dev/null || cp README.md "$ROOT/"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
rm -rf "$ROOT"

# AppImage
if command -v appimagetool >/dev/null 2>&1; then
    APPDIR=dist/Ferrum.AppDir
    rm -rf "$APPDIR"
    mkdir -p "$APPDIR/usr/bin"
    cp target/release/ferrum "$APPDIR/usr/bin/"
    cp packaging/linux/ferrum.desktop "$APPDIR/"
    cp packaging/icons/icon_256.png "$APPDIR/ferrum.png"
    printf '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/ferrum" "$@"\n' > "$APPDIR/AppRun"
    chmod +x "$APPDIR/AppRun"
    ARCH=x86_64 appimagetool --appimage-extract-and-run "$APPDIR" "dist/$NAME.AppImage"
    rm -rf "$APPDIR"
fi
ls dist
