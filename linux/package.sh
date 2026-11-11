#!/usr/bin/env bash
# Package a release build for Linux: dist/cc-deck_<version>_amd64.deb and
# dist/cc-deck-linux-x86_64.tar.gz (run ./build.sh first, or pass --build).
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"
APP_ID=dev.claudedeck.CCDeck
VERSION="${VERSION:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)}"
[[ "${1:-}" == "--build" ]] && ./build.sh
BIN="$ROOT/target/release"
DIST="$ROOT/dist"
mkdir -p "$DIST"

# .deb (Ubuntu 24.04+ / Debian 13+)
STAGE="$(mktemp -d)"
install -Dm755 "$BIN/cc-deck" "$STAGE/usr/bin/cc-deck"
install -Dm755 "$BIN/claude-deck" "$STAGE/usr/bin/claude-deck"
install -Dm644 "$ROOT/macos/Resources/AppIcon.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "data/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: cc-deck
Version: ${VERSION}
Section: devel
Priority: optional
Architecture: amd64
Depends: libgtk-4-1 (>= 4.14), libadwaita-1-0 (>= 1.5), libvte-2.91-gtk4-0 (>= 0.76), git
Maintainer: hankniel <noreply@github.com>
Homepage: https://github.com/hankniel/claude-deck
Description: Terminal app for Claude Code with per-session accounts and a native canvas
 CC Deck runs each Claude Code session on the account you pick, shows usage limits,
 moves sessions between accounts, and renders what agents draw (flowcharts, lineage,
 database diagrams, reviews) natively. Includes the claude-deck CLI.
CONTROL
dpkg-deb --build --root-owner-group "$STAGE" "$DIST/cc-deck_${VERSION}_amd64.deb" >/dev/null
cp "$DIST/cc-deck_${VERSION}_amd64.deb" "$DIST/cc-deck_amd64.deb"
rm -rf "$STAGE"

# tarball for other distributions
T="$(mktemp -d)/cc-deck"
install -Dm755 "$BIN/cc-deck" "$T/bin/cc-deck"
install -Dm755 "$BIN/claude-deck" "$T/bin/claude-deck"
install -Dm644 "$ROOT/macos/Resources/AppIcon.svg" "$T/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "data/$APP_ID.desktop" "$T/share/applications/$APP_ID.desktop"
tar -C "$(dirname "$T")" -czf "$DIST/cc-deck-linux-x86_64.tar.gz" cc-deck
rm -rf "$(dirname "$T")"
echo "packaged $DIST/cc-deck_${VERSION}_amd64.deb and $DIST/cc-deck-linux-x86_64.tar.gz"
