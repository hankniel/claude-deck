#!/usr/bin/env bash
# Build CC Deck for Linux (GTK 4 + libadwaita + VTE) together with the claude-deck CLI.
# Usage: ./build.sh            → ../target/release/{cc-deck,claude-deck}
#        ./build.sh --deps     → first install build dependencies with apt (needs sudo)
#        ./build.sh --install  → also install into ~/.local (binaries, menu entry, icon)
# Flags can be combined. PREFIX overrides ~/.local.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
APP_ID=dev.claudedeck.CCDeck

for arg in "$@"; do
  if [[ "$arg" == "--deps" ]]; then
    sudo apt-get install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev
  fi
done

cargo build --release --manifest-path "$ROOT/Cargo.toml" -p claude-deck -p cc-deck

for arg in "$@"; do
  if [[ "$arg" == "--install" ]]; then
    install -Dm755 "$ROOT/target/release/cc-deck" "$PREFIX/bin/cc-deck"
    install -Dm755 "$ROOT/target/release/claude-deck" "$PREFIX/bin/claude-deck"
    install -Dm644 "$ROOT/macos/Resources/AppIcon.svg" "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    install -d "$PREFIX/share/applications"
    sed "s|^Exec=.*|Exec=$PREFIX/bin/cc-deck|" "data/$APP_ID.desktop" > "$PREFIX/share/applications/$APP_ID.desktop"
    gtk4-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true
    update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
    echo "installed $PREFIX/bin/cc-deck (open \"CC Deck\" from the app menu)"
  fi
done
echo "built $ROOT/target/release/cc-deck and claude-deck"
