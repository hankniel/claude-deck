#!/usr/bin/env bash
# Build CC Deck.app (SwiftUI) with the claude-deck CLI inside, plus a .dmg.
# Usage: ./build.sh            → build/CC Deck.app, build/CC-Deck.dmg
#        ./build.sh --install  → also copy into /Applications
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"
# Build against Xcode's SDK: macOS only turns on the Liquid Glass design for apps linked with a new SDK.
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
APP="build/CC Deck.app"

cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin claude-deck
swift build -c release --sdk "$SDK"

rm -rf build && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin"
cp .build/release/CCDeck "$APP/Contents/MacOS/CCDeck"
otool -l "$APP/Contents/MacOS/CCDeck" | grep -A4 LC_BUILD_VERSION | grep -E "minos|sdk"
cp "$ROOT/target/release/claude-deck" "$APP/Contents/Resources/bin/claude-deck"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp Resources/Info.plist "$APP/Contents/Info.plist"
codesign --force --deep --sign - "$APP"

STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "CC Deck" -srcfolder "$STAGE" -ov -format UDZO build/CC-Deck.dmg
rm -rf "$STAGE"

if [[ "${1:-}" == "--install" ]]; then
  osascript -e 'quit app "CC Deck"' 2>/dev/null || true
  rm -rf "/Applications/CC Deck.app"
  ditto "$APP" "/Applications/CC Deck.app"
  echo "installed /Applications/CC Deck.app"
fi
echo "built $APP and build/CC-Deck.dmg"
