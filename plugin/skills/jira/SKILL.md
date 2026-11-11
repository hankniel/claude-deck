---
name: jira
description: TRIGGER only when the user explicitly says Jira, names a Jira issue key (e.g. RAID-123), or asks to create, update, push or sync issues, epics, sprints or a .plan to Jira. DO NOT TRIGGER for ordinary planning, todo lists or progress tracking that never mentions Jira (use the canvas objectives board), nor for diagrams or code review.
---

# Jira

CC Deck holds the user's Jira Cloud token and its Jira page shows the same data. Talk to Jira only
through `claude-deck canvas jira <command>` (add `--json` for machine-readable output); it answers
in a few lines. Not an MCP connector.

## Commands

Read freely:

| | |
|---|---|
| `jira mine` · `jira search "<jql>"` | open issues of the user · any JQL (50 max) |
| `jira show KEY` | details, points, links, comments, statuses it can move to |
| `jira boards [PROJ]` · `jira board ID [--scope sprint\|backlog\|all]` | boards · a board's columns (active sprint by default) |
| `jira sprints BOARD` · `jira epics PROJ` | active and future sprints · epics with progress |
| `jira plan <file.plan>` | check a plan parses: items, points, keys, errors |

**Every write needs the user's yes first**: show the exact command and text, ask, then run it.

| | |
|---|---|
| `jira move KEY "<status>"` | status by name, from those `show` lists |
| `jira comment KEY "<text>"` · `jira assign KEY me\|none` | |
| `jira edit KEY [--summary] [--description] [--points] [--labels a,b] [--due YYYY-MM-DD] [--assign me]` | only the fields that change; `--description` replaces it as plain text and drops rich formatting — say so |
| `jira link A blocks\|blocked-by\|relates B` · `jira unlink A B` | |
| `jira sprint KEY[,KEY…] <sprint id\|backlog>` | subtasks follow their parent and cannot move alone |
| `jira create PROJ "<summary>" [--type T] [--parent KEY]` | prefer a `.plan` for more than one issue |
| `jira push <file.plan>` | creates the plan's new issues and dependency links |

`jira graph EPIC <session dir>` and `jira plan-graph <file.plan>` draw dependencies as canvas screens
(no write). "not configured" means no token: the user fills `~/.local/share/claude-deck/canvas/jira.env`
(CC Deck's Jira page shows how).

## How the work is shaped

- Hierarchy (parent, one per issue, rolls progress up): **Epic → Story / Task / Bug → Subtask**.
  Story = a user- or business-visible outcome, Task = technical work, Bug = a defect.
  Work done *inside* a story is its Subtask; an independent prerequisite is a Task that blocks it.
- Dependencies are **Blocks** links (A blocks B = B waits for A); Relates is not a dependency.
  Add only real ordering constraints read from the work.
- Write in the language the existing issues use; technical terms stay English.
- Read before writing: the code, the request, and what exists. Never invent scope.

## `.plan`: breaking work down

Draft `<session dir>/content/<name>.plan`; the user reviews it on the canvas, comments per item
(arriving as `<name>.plan: #n <summary> — note: …`), and pushes it.

```
# [DPL] Lakehouse MVP                 the epic, or `epic: RAID-1` for an existing one
project: RAID
labels: lakehouse                     optional, on every issue

## Story: Ingest order events | 5 | @me | ingestion
What and why, in a few lines.
AC:
- lands in bronze within 10 minutes
- Subtask: Write the Auto Loader job | 2
## Task: Alert when ingestion lags > 1h | 1
after: #1                             blocked by item 1 (or by RAID-12)
```

- After `|`: a number = story points, `@me` = assignee (only `@me`), `KEY-1` = the issue it became
  (push writes it), anything else = labels.
- Every Story/Task gets checkable acceptance criteria.
- **Push only on the user's Push button, or `jira push` after their yes.** It creates what has no
  key (epic → items → subtasks), then the `after:` links, and never edits existing issues.
