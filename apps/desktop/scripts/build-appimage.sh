#!/usr/bin/env bash
# Build a self-contained AppImage of writee for Linux.
#
# Requirements (one-time):
#   * Rust toolchain (`cargo`)
#   * `appimagetool` on PATH:
#       https://github.com/AppImage/AppImageKit/releases
#
# Usage:
#   ./apps/desktop/scripts/build-appimage.sh
#
# Output:
#   ./dist/writee-x86_64.AppImage

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"

DIST="$ROOT/dist"
APPDIR="$DIST/writee.AppDir"

echo "==> Building writee in release mode"
cargo build --release -p writee-desktop

echo "==> Composing AppDir at $APPDIR"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"

cp "$ROOT/target/release/writee" "$APPDIR/usr/bin/writee"
cp "$ROOT/apps/desktop/writee.desktop" "$APPDIR/writee.desktop"
cp "$ROOT/apps/desktop/writee.desktop" "$APPDIR/usr/share/applications/writee.desktop"

# Procedural icon — write a 256×256 placeholder PNG using ImageMagick if
# present, otherwise generate a minimal PNG via Python (both are common on
# Linux build hosts). The window icon shipped *inside* the binary is what the
# user actually sees once the app launches; this file is just for the launcher.
if command -v convert >/dev/null 2>&1; then
    convert -size 256x256 xc:'#fbfbfb' \
        -fill '#121212' -gravity center -pointsize 160 -annotate +0+0 'w' \
        "$APPDIR/writee.png"
else
    cat <<'PY' | python3 - "$APPDIR/writee.png"
import struct, sys, zlib
W = H = 256
def chunk(t, d):
    crc = zlib.crc32(t + d) & 0xffffffff
    return struct.pack('>I', len(d)) + t + d + struct.pack('>I', crc)
hdr = b'\x89PNG\r\n\x1a\n'
ihdr = struct.pack('>IIBBBBB', W, H, 8, 6, 0, 0, 0)
raw = bytearray()
for y in range(H):
    raw.append(0)  # filter: none
    for x in range(W):
        # off-white card with a black border
        on_border = x < 4 or y < 4 or x >= W - 4 or y >= H - 4
        c = (40, 40, 40, 255) if on_border else (251, 251, 251, 255)
        raw.extend(c)
idat = zlib.compress(bytes(raw), 9)
with open(sys.argv[1], 'wb') as f:
    f.write(hdr)
    f.write(chunk(b'IHDR', ihdr))
    f.write(chunk(b'IDAT', idat))
    f.write(chunk(b'IEND', b''))
PY
fi
cp "$APPDIR/writee.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/writee.png"

cat > "$APPDIR/AppRun" <<'SH'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/writee" "$@"
SH
chmod +x "$APPDIR/AppRun"

echo "==> Running appimagetool"
if ! command -v appimagetool >/dev/null 2>&1; then
    echo "Error: 'appimagetool' not on PATH." >&2
    echo "  Download from https://github.com/AppImage/AppImageKit/releases" >&2
    exit 1
fi
appimagetool "$APPDIR" "$DIST/writee-x86_64.AppImage"

echo
echo "Done. Built $DIST/writee-x86_64.AppImage"
