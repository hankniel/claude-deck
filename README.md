# claude-deck

Terminal app for Claude Code. Each tab runs `claude` on the account you pick, with live
5-hour / 7-day usage per account and optional auto-switch before a limit; agents draw on a
native **canvas** (flowcharts, Databricks-style lineage, dbdiagram-style ERDs, review findings,
row counts) and track work on an objectives **board**. macOS and Linux.

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

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/hankniel/claude-deck/main/install.sh | sh
```

- **macOS 26+** (Apple silicon and Intel): installs `CC Deck.app` into /Applications and links the
  `claude-deck` CLI into `~/.local/bin`.
- **Linux x86_64**: on Ubuntu 24.04+ / Debian 13+ installs the `.deb` with apt (GTK 4, libadwaita
  and VTE come along); elsewhere unpacks the release into `~/.local` (needs GTK ≥ 4.14,
  libadwaita ≥ 1.5, VTE GTK 4 ≥ 0.76 from your distribution).
- Also adds the `deck-canvas` plugin to Claude Code when `claude` is on PATH.
- `… | sh -s -- --source` builds from source (Rust + Xcode on macOS, GTK dev packages on Linux),
  `--version v0.2.0` picks a release, `--no-plugin` skips the plugin, `--uninstall` removes it.

Release downloads are also on the [releases page](https://github.com/hankniel/claude-deck/releases)
(`CC-Deck-macos.zip` / `.dmg`, `cc-deck_amd64.deb`, `cc-deck-linux-x86_64.tar.gz`). The macOS app
is not notarized; the installer clears the quarantine flag, or right-click → Open the first time.

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

## Canvas and board

Agents write small text files ("screens") into their session folder; CC Deck's Canvas page renders
them natively and reloads them as they change. Answers (kept review findings, document comments,
board edits) go back to the agent with its next prompt.

| Screen | Shows | Format |
|---|---|---|
| `.mmd` | interactive flowchart: collapsible groups, click-to-trace, search, links to files | Mermaid flowchart |
| `.lineage` | data lineage like Databricks Unity Catalog: entity cards, columns, column-level tracing | `a, b > c > d`, `t.col < s.col` |
| `.dbml` | database diagram like dbdiagram.io: draggable tables, crow's-foot relations, groups, enums | DBML |
| `.rv` | review findings to keep or skip | `key: file:line \| sev \| what \| fix` |
| `.rows` | row counts through a pipeline | `step \| rows_in \| rows_out \| cols` |
| `.doc` / `.open` | a Markdown file to comment on / a file or folder | a path |
| `.plan` | a work breakdown with live Jira status, pushed to Jira on your click | Markdown-like |

Mermaid `sequenceDiagram` screens are drawn natively as well.

**Jira**: the Jira page lists your issues, the board (drag cards between columns), epics with
progress and dependency graphs, and JQL search; an issue panel moves, assigns, edits, sets the
sprint, links and comments. Credentials go in `~/.local/share/claude-deck/canvas/jira.env`
(`JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`); agents use the same token through
`claude-deck canvas jira …` (the plugin's `jira` skill). **Git changes** of any session or terminal
tab open from their right-click menu.

The Board lists objectives (`<canvas home>/objectives/<slug>.md`, Markdown task lists) with the
sessions working on them and what each is doing (running / needs you / your turn); terminal tabs
show the same state, and CC Deck notifies you when an agent needs you.

Install the agent side (hooks + the `canvas` skill that teaches the formats) as a Claude Code plugin:

```text
/plugin marketplace add hankniel/claude-deck
/plugin install deck-canvas@claude-deck
```

Codex: merge `plugin/codex_hooks.json` into `~/.codex/hooks.json` and link
`plugin/skills/canvas` into `~/.agents/skills`. The hooks call `claude-deck canvas hook …`, so the
CLI must be on PATH. `claude-deck canvas check <file>` validates a screen; `claude-deck canvas
wait|objectives|attach|ls|home` are the agent's tools. Canvas data lives in
`~/.local/share/claude-deck/canvas` (`CANVAS_HOME` overrides).

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
