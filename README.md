# claude-deck

Terminal multiplexer for Claude Code. Each tab runs `claude` on the account you pick, with live
5-hour / 7-day usage per account and optional auto-switch before a limit. macOS and Linux.

```
┌ accounts ──────────────────┐ 1 work · claude-config   2 personal · dbt
│● work team                 │ ╭─ Claude Code ───────────────────────────╮
│  you@work.com              │ │ > ...                                   │
│  5h ██████░░░░  62% 2h10m  │
│  7d ██░░░░░░░░  21% 3d4h   │
│  personal pro              │
│  5h ░░░░░░░░░░   3% 4h55m  │
└────────────────────────────┘ C-a ? help · c new · s switch account · a auto
```

## Desktop app (CC Deck, macOS)

`macos/` is a native SwiftUI app (macOS 26+, Liquid Glass) with SwiftTerm terminals. On Linux
use the GTK app in `linux/` (below) or the `claude-deck` CLI/TUI; all share the same Rust core.

- Opens as a normal terminal (your shell, ⌘T for tabs). Typing `claude` shows a popup to pick
  the account, with each account's usage; the folder's default account is preselected.
- The toolbar account menu moves a running conversation to another account (`--resume`).
- Sidebar usage bars, menu bar gauge, notifications near limits, optional auto-switch.
- Dashboard: usage limits over time, tokens per day by model/account, estimated cost,
  per-account and per-project totals, switch/limit log.

Build (needs Xcode, only at build time; the app runs without it):

```bash
./macos/build.sh --install   # → /Applications/CC Deck.app and macos/build/CC-Deck.dmg
```

In plain terminals, `claude-deck launch` gives the same account choice as a text prompt.

## Desktop app (CC Deck, Linux)

`linux/` is the same app for GNOME/GTK desktops (GTK 4 + libadwaita + VTE), built on the
shared Rust core: terminal tabs with the account picker on `claude`, the account menu that
moves a running conversation, sidebar usage rings, limit banner and auto-switch, desktop
notifications, settings (accounts, folder defaults, limit lines) and the dashboard.
Needs GTK ≥ 4.14, libadwaita ≥ 1.5, VTE ≥ 0.76 (Ubuntu 24.04 or newer).

```bash
./linux/build.sh --deps --install   # apt build deps (sudo), then ~/.local/bin + app menu entry
```

Shortcuts follow Linux terminals: Ctrl+Shift+T/W new/close tab, Ctrl+Shift+K start Claude,
Ctrl+Shift+C/V copy/paste, Alt+1–9 tabs, Ctrl+Shift+D dashboard, F9 sidebar, Ctrl+? all.
`CC_DECK_SNAPSHOT=out.png CC_DECK_SNAPSHOT_MODE=dashboard|settings|picker cc-deck` renders the
window to a PNG and quits (development aid).

## CLI / TUI install

```bash
cargo install --path .
```

## Accounts

`default` is your normal `~/.claude` login. Every other account is a profile dir passed to
Claude Code as `CLAUDE_CONFIG_DIR` (`~/.local/share/claude-deck/profiles/<name>`). Claude Code
keeps each profile's own login (macOS Keychain or `.credentials.json`); claude-deck only reads
the token to show usage and never refreshes or copies it.

Profiles symlink the shared parts of `~/.claude` (settings, CLAUDE.md, skills, agents, plugins,
projects/history...), so every account sees the same setup and conversations. Edit `shared` in
the config to change that. User-scope MCP servers live in each profile's own `.claude.json`.

```bash
claude-deck add personal      # creates the profile and opens claude: /login, then /exit
claude-deck list              # usage of every account
claude-deck rm personal
```

## Use

```bash
claude-deck                   # TUI; first tab uses the mapped account or asks
claude-deck -a work -- --resume   # first tab on work, extra args for claude
claude-deck run personal      # plain claude on one account, no TUI
eval "$(claude-deck env work)"  # export CLAUDE_CONFIG_DIR in this shell
```

Keys (prefix `Ctrl-a`): `c` new tab · `C` new tab, always pick · `s` move this tab to another
account (resumes the chat) · `R` restart · `x` close · `n`/`p`/`1-9` tabs · `a` auto-switch ·
`r` refresh usage · `b` sidebar · `q` quit · `Ctrl-a` literal Ctrl-a. `Shift-PgUp/PgDn` scrolls.

## Directory mapping

```bash
claude-deck map work ~/work/acme   # subdirectories inherit
claude-deck maps
claude-deck unmap ~/work/acme
```

New tabs in a mapped directory start on that account without asking.

## Auto-switch

When a tab's account passes `threshold` (5h, default 90%) or `weekly_threshold` (7d, default
95%), claude-deck waits until the tab has been idle for `idle_secs`, then restarts it on the
account with the most headroom with `claude --resume <session>`. With auto-switch off it only
warns. Disabled accounts (`claude-deck disable <name>`) are never chosen automatically.

```bash
claude-deck config                    # show settings (~/.config/claude-deck/config.toml)
claude-deck config auto_switch true
claude-deck config threshold 80
```

Rotating accounts to get around usage limits may conflict with Anthropic's terms; the feature
is meant for people who legitimately hold several accounts (e.g. work and personal).
