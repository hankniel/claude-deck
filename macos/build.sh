#!/usr/bin/env bash
# Build CC Deck.app (SwiftUI) with the claude-deck CLI inside, plus a .dmg.
# Usage: ./build.sh            → build/CC Deck.app, build/CC-Deck.dmg
#        ./build.sh --install  → also copy into /Applications and link the CLI into ~/.local/bin
#        UNIVERSAL=1 ./build.sh → arm64 + x86_64 (release builds)
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"
# Build against Xcode's SDK: macOS only turns on the Liquid Glass design for apps linked with a new SDK.
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
APP="build/CC Deck.app"

if [[ "${UNIVERSAL:-}" == "1" ]]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin claude-deck --target "$t"
  done
  CLI="$ROOT/target/claude-deck-universal"
  lipo -create -output "$CLI" "$ROOT/target/aarch64-apple-darwin/release/claude-deck" "$ROOT/target/x86_64-apple-darwin/release/claude-deck"
  swift build -c release --sdk "$SDK" --arch arm64 --arch x86_64
  EXE="$(swift build -c release --sdk "$SDK" --arch arm64 --arch x86_64 --show-bin-path)/CCDeck"
else
  cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin claude-deck
  CLI="$ROOT/target/release/claude-deck"
  swift build -c release --sdk "$SDK"
  EXE=".build/release/CCDeck"
fi

rm -rf build && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin"
cp "$EXE" "$APP/Contents/MacOS/CCDeck"
otool -l "$APP/Contents/MacOS/CCDeck" | grep -A4 LC_BUILD_VERSION | grep -E "minos|sdk" || true
cp "$CLI" "$APP/Contents/Resources/bin/claude-deck"
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
  mkdir -p "$HOME/.local/bin"
  ln -sf "/Applications/CC Deck.app/Contents/Resources/bin/claude-deck" "$HOME/.local/bin/claude-deck"
  echo "installed /Applications/CC Deck.app (CLI: ~/.local/bin/claude-deck)"
fi
echo "built $APP and build/CC-Deck.dmg"
