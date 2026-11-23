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
shared Rust core and laid out like VS Code:

- **Activity bar**: Explorer, Accounts, Board, Jira, Dashboard; the gear holds settings and new
  sessions. The Board icon counts agent sessions waiting for you.
- **Explorer**: Terminals (shells and Claude sessions), Canvas (agent sessions and their screens)
  and Files (the active terminal's folder; files open as read-only tabs, the eye shows hidden ones).
- **Editor area**: canvas screens, files, Git changes, Board, Jira and the Dashboard open as tabs,
  with a breadcrumb. Split it into two groups from a tab's menu or by dragging a tab.
- **Panel**: terminals and Claude sessions, docked bottom, right, left or top (layout button in
  the title bar), split in two, maximized or hidden. Any tab moves between panel and editor area
  by drag and drop or its menu, so a Claude chat can sit beside its diagram.
- **Status bar**: Git branch (opens its changes), sessions waiting for you, the Claude account of
  the active terminal with its 5-hour / 7-day usage (click to move the conversation), auto-switch.
- **Title bar**: what is open, plus Ctrl+Shift+P to jump to any open tab, terminal or page.

Canvas flowcharts are drawn as Databricks-style cards: an icon tile showing the node's Mermaid
shape, the first label line as the title and the rest as a subtitle, edges as curves between
port dots with their labels as badges. Clicking a node traces its lineage and fades the rest;
folding a group animates the layout. Diagrams open at a readable zoom (85–100%) and re-fit when
the view resizes until you pan or zoom.

Settings → General → **Theme** picks Claude (warm), VS Code Modern, Nord, Solarized, GitHub or
Catppuccin, each in light and dark (Appearance: System, Light, Dark); terminals and diagrams
follow it. **Background** puts an image behind the editor area and diagrams with an adjustable
strength; **Fonts** sets the interface, terminal and diagram fonts. The layout (side bar, panel
position and size) is remembered.

Needs GTK ≥ 4.14, libadwaita ≥ 1.5, VTE ≥ 0.76 (Ubuntu 24.04 or newer).

```bash
./linux/build.sh --deps --install   # apt build deps (sudo), then ~/.local/bin + app menu entry
```

Shortcuts: Ctrl+Shift+T new terminal, Ctrl+Shift+K start Claude, Ctrl+Shift+W close tab,
Ctrl+` panel, Ctrl+Shift+B or F9 side bar, Ctrl+Shift+E explorer, Ctrl+Shift+P go to tab,
Ctrl+Shift+O board, Ctrl+Shift+J Jira, Ctrl+Shift+D dashboard, Ctrl+Shift+C/V copy/paste,
Alt+1–9 terminals, Ctrl+, settings, Ctrl+? all.

Development aid: `CC_DECK_SNAPSHOT=out.png cc-deck` renders the window to a PNG and quits, in a
separate instance. `CC_DECK_SNAPSHOT_MODE` picks what to show: `dashboard`, `settings`, `picker`,
`board`, `jira`, `canvas:<session>/<screen>`, `changes:<dir>`, or `layout:<bottom|right|left|top>`
(add `+split` for split groups). On Wayland, set `GDK_BACKEND=x11` for snapshots.

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

Every account runs on the one `~/.claude` config, so settings, CLAUDE.md, skills, agents,
plugins, hooks, MCP servers, folder trust and conversation history are the same everywhere.
Accounts differ only in who is signed in:

- `default` is the login Claude Code keeps in `~/.claude` itself (`/login`).
- Every other account is a long-lived token from `claude setup-token`, stored in
  `~/.local/share/claude-deck/tokens/<name>` (mode 600) and given to its sessions as
  `CLAUDE_CODE_OAUTH_TOKEN`.

These tokens can't read the usage endpoint, so CC Deck gives the sessions it starts a status
line that records the 5-hour / 7-day limits Claude Code gets with every reply, then runs your
own `statusLine` command unchanged. A token account's usage therefore shows once a session has
run on it, and updates while one does. Token sessions may lack claude.ai features that need a
full login (claude.ai MCP connectors, plugin sync); the email and plan of a token account are
unknown, so its name is its label.

```bash
claude-deck add personal      # runs claude setup-token, then asks for the token
claude-deck add personal --token -   # or pipe one in (also renews an expired token)
claude-deck list              # usage of every account
claude-deck rm personal
```

Older versions made a config dir per account under `~/.local/share/claude-deck/profiles/`.
Those are no longer used (they are left on disk); add the accounts again with `claude-deck add`.

## Use

```bash
claude-deck                   # TUI; first tab uses the mapped account or asks
claude-deck -a work -- --resume   # first tab on work, extra args for claude
claude-deck run personal      # plain claude on one account, no TUI
eval "$(claude-deck env work)"  # put claude in this shell on work
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
