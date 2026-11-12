#!/bin/sh
# Install CC Deck (desktop app + claude-deck CLI) and the deck-canvas Claude Code plugin.
#
#   curl -fsSL https://raw.githubusercontent.com/hankniel/claude-deck/main/install.sh | sh
#
# macOS: the latest release's CC Deck.app goes to /Applications (or ~/Applications).
# Linux: the .deb on Debian/Ubuntu (apt installs GTK/libadwaita/VTE), else a tarball into ~/.local.
# Options (as env vars or flags):
#   --source        build from source instead of downloading (CC_DECK_SOURCE=1)
#   --version vX.Y  a specific release (CC_DECK_VERSION)
#   --no-plugin     skip the Claude Code plugin (CC_DECK_NO_PLUGIN=1)
#   --uninstall     remove the app and the CLI
#   CC_DECK_FORMAT=tar  on Linux, the tarball into ~/.local even where apt exists
set -eu

REPO="hankniel/claude-deck"
VERSION="${CC_DECK_VERSION:-latest}"
SOURCE="${CC_DECK_SOURCE:-0}"
PLUGIN=1
[ "${CC_DECK_NO_PLUGIN:-0}" = "1" ] && PLUGIN=0
UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --source) SOURCE=1 ;;
    --version) shift; VERSION="$1" ;;
    --no-plugin) PLUGIN=0 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) sed -n '2,15p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

OS="$(uname -s)"
ARCH="$(uname -m)"
BIN_DIR="$HOME/.local/bin"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

url() {
  if [ "$VERSION" = "latest" ]; then
    echo "https://github.com/$REPO/releases/latest/download/$1"
  else
    echo "https://github.com/$REPO/releases/download/$VERSION/$1"
  fi
}

fetch() { # url dest → 0 when downloaded
  if have curl; then curl -fsSL "$1" -o "$2" 2>/dev/null
  elif have wget; then wget -q "$1" -O "$2"
  else die "curl or wget is needed"; fi
}

sudo_() { if [ "$(id -u)" = "0" ]; then "$@"; else sudo "$@"; fi; }

# Root, a cached sudo, or a terminal sudo can ask the password on.
can_sudo() {
  [ "$(id -u)" = "0" ] && return 0
  have sudo || return 1
  sudo -n true 2>/dev/null && return 0
  (exec </dev/tty) 2>/dev/null
}

# GTK 4.14+, libadwaita 1.5+ and VTE (GTK 4) 0.76+ present at runtime?
linux_runtime_ok() {
  have ldconfig || return 1
  libs="$(ldconfig -p 2>/dev/null)"
  for l in libgtk-4.so.1 libadwaita-1.so.0 libvte-2.91-gtk4.so.0; do
    case "$libs" in *"$l"*) ;; *) return 1 ;; esac
  done
}

link_cli() { # path to the claude-deck binary
  mkdir -p "$BIN_DIR"
  ln -sf "$1" "$BIN_DIR/claude-deck"
  case ":$PATH:" in *":$BIN_DIR:"*) ;; *) warn "$BIN_DIR is not on your PATH; add it so agents' hooks find claude-deck";; esac
}

# ---------- uninstall ----------
if [ "$UNINSTALL" = "1" ]; then
  if [ "$OS" = "Darwin" ]; then
    rm -rf "/Applications/CC Deck.app" "$HOME/Applications/CC Deck.app"
  else
    if have dpkg && dpkg -s cc-deck >/dev/null 2>&1; then sudo_ apt-get remove -y cc-deck; fi
    rm -f "$HOME/.local/bin/cc-deck" "$HOME/.local/share/applications/dev.claudedeck.CCDeck.desktop" \
      "$HOME/.local/share/icons/hicolor/scalable/apps/dev.claudedeck.CCDeck.svg"
  fi
  rm -f "$BIN_DIR/claude-deck"
  say "removed CC Deck (your accounts and canvas data in ~/.local/share/claude-deck are kept)"
  exit 0
fi

# ---------- from source ----------
from_source() {
  say "building from source"
  have git || die "git is needed"
  if ! have cargo; then
    [ -x "$HOME/.cargo/bin/cargo" ] || { say "installing Rust (rustup)"; fetch https://sh.rustup.rs "$TMP/rustup.sh"; sh "$TMP/rustup.sh" -y --profile minimal >/dev/null; }
    PATH="$HOME/.cargo/bin:$PATH"
  fi
  SRC="${CC_DECK_SRC:-$HOME/.local/share/claude-deck/src}"
  if [ -d "$SRC/.git" ]; then git -C "$SRC" pull -q --ff-only; else git clone -q "https://github.com/$REPO.git" "$SRC"; fi
  if [ "$OS" = "Darwin" ]; then
    xcrun --find swift >/dev/null 2>&1 || die "Xcode 26 is needed to build the macOS app (or install a release without --source)"
    "$SRC/macos/build.sh" --install
  else
    if pkg-config --exists gtk4 libadwaita-1 vte-2.91-gtk4 2>/dev/null; then
      say "build dependencies already installed"
    elif ! can_sudo; then
      die "installing build dependencies needs sudo; run this in a terminal"
    elif have apt-get; then
      say "installing build dependencies (sudo)"
      sudo_ apt-get install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev
    elif have dnf; then sudo_ dnf install -y gcc pkgconf-pkg-config gtk4-devel libadwaita-devel vte291-gtk4-devel
    elif have pacman; then sudo_ pacman -S --needed --noconfirm base-devel gtk4 libadwaita vte4
    else warn "install the GTK 4, libadwaita and VTE (GTK 4) development packages yourself"; fi
    "$SRC/linux/build.sh" --install
  fi
}

# ---------- macOS ----------
install_macos() {
  say "downloading CC Deck for macOS"
  fetch "$(url CC-Deck-macos.zip)" "$TMP/app.zip" || return 1
  (cd "$TMP" && ditto -x -k app.zip .) || return 1
  DEST=/Applications
  [ -w "$DEST" ] || DEST="$HOME/Applications"
  mkdir -p "$DEST"
  osascript -e 'quit app "CC Deck"' >/dev/null 2>&1 || true
  rm -rf "$DEST/CC Deck.app"
  ditto "$TMP/CC Deck.app" "$DEST/CC Deck.app"
  # Not notarized: clear the download quarantine so Gatekeeper opens it.
  xattr -dr com.apple.quarantine "$DEST/CC Deck.app" 2>/dev/null || true
  link_cli "$DEST/CC Deck.app/Contents/Resources/bin/claude-deck"
  say "installed $DEST/CC Deck.app"
}

# ---------- Linux ----------
install_tarball() {
  say "downloading the tarball"
  fetch "$(url cc-deck-linux-x86_64.tar.gz)" "$TMP/cc.tgz" || return 2
  tar -C "$TMP" -xzf "$TMP/cc.tgz"
  mkdir -p "$HOME/.local"
  cp -R "$TMP/cc-deck/." "$HOME/.local/"
  sed -i "s|^Exec=.*|Exec=$HOME/.local/bin/cc-deck|" "$HOME/.local/share/applications/dev.claudedeck.CCDeck.desktop" 2>/dev/null || true
  have update-desktop-database && update-desktop-database -q "$HOME/.local/share/applications" 2>/dev/null || true
  linux_runtime_ok || warn "install GTK >= 4.14, libadwaita >= 1.5 and VTE (GTK 4) >= 0.76 from your distribution (Ubuntu: sudo apt install libgtk-4-1 libadwaita-1-0 libvte-2.91-gtk4-0)"
  say "installed CC Deck into ~/.local (open it from your app menu or run cc-deck)"
}

install_linux() {
  [ "$ARCH" = "x86_64" ] || return 2
  if [ "${CC_DECK_FORMAT:-}" != "tar" ] && have apt-get && have dpkg; then
    if can_sudo; then
      say "downloading the .deb"
      fetch "$(url cc-deck_amd64.deb)" "$TMP/cc-deck_amd64.deb" || return 2
      say "installing with apt (sudo): GTK 4, libadwaita and VTE come along"
      if sudo_ apt-get install -y "$TMP/cc-deck_amd64.deb"; then
        link_cli /usr/bin/claude-deck
        say "installed CC Deck (open it from your app menu or run cc-deck)"
        return 0
      fi
      warn "apt couldn't install the .deb; installing the tarball into ~/.local instead"
    else
      warn "sudo can't ask for a password here; installing into ~/.local without sudo (run in a terminal for the .deb)"
    fi
  fi
  install_tarball
}

if [ "$SOURCE" = "1" ]; then
  from_source
elif [ "$OS" = "Darwin" ]; then
  install_macos || { warn "no release download available; building from source"; from_source; }
elif [ "$OS" = "Linux" ]; then
  rc=0; install_linux || rc=$?
  [ "$rc" = "2" ] && { warn "no release download for this system; building from source"; from_source; }
  [ "$rc" = "0" ] || [ "$rc" = "2" ] || exit "$rc"
else
  die "unsupported system: $OS"
fi

# ---------- the agent side ----------
if [ "$PLUGIN" = "1" ]; then
  if have claude; then
    say "adding the deck-canvas plugin to Claude Code"
    claude plugin marketplace add "$REPO" >/dev/null 2>&1 || true
    claude plugin install deck-canvas@claude-deck >/dev/null 2>&1 \
      && say "plugin installed (restart running Claude Code sessions)" \
      || warn "couldn't install the plugin automatically; in Claude Code run: /plugin marketplace add $REPO, then /plugin install deck-canvas@claude-deck"
  else
    warn "Claude Code not found; later run in it: /plugin marketplace add $REPO, then /plugin install deck-canvas@claude-deck"
  fi
fi
say "done"
