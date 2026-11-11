---
name: canvas
description: TRIGGER when an answer needs a picture the terminal can't draw — a flow / architecture / DAG, data lineage (tables, notebooks, jobs, column-level), a database schema / ERD, review findings tied to file:line, row counts through a SQL refactor or pipeline, or a document to mark up — and when starting or progressing a multi-step objective on the board. Write it as a canvas screen that CC Deck renders natively, instead of ASCII art or a long structured wall of text. DO NOT TRIGGER for questions (use your input tool), shareable deliverables, or anything that fits in a few lines of prose.
---

# Canvas

The SessionStart hook prints `canvas ready … session dir <dir>; write screens to <dir>/content/`.
CC Deck (Canvas page) shows each file you write there and reloads it when you rewrite it.
Write the **leanest file that carries the idea**; check one with `claude-deck canvas check <file>`.

## How much to draw

**One screen per answer**, only when the terminal can't show it: a shape, a quantity, or a list
the user acts on. Fits in a few lines → no screen. A decision → your input tool, not a screen.
Never draw one idea twice. After writing, say in one line what is on the canvas, then 2–4 sentences.

| Screen | For | Format |
|---|---|---|
| `.mmd` | a flow, architecture, DAG | Mermaid flowchart |
| `.lineage` | data lineage: tables, views, notebooks, jobs, dashboards, columns | lines below |
| `.dbml` | a database schema / ERD | DBML (dbdiagram.io) |
| `.rv` | review findings to keep or skip | `key: file:line \| high\|medium\|low \| what \| fix` |
| `.rows` | row counts that must survive a refactor | `step \| rows_in \| rows_out \| columns` |
| `.doc` / `.open` | a Markdown file to comment on / a file or folder to show | one path (`path[:line]`) |
| `.plan` | a work breakdown to push to Jira | see the jira skill |

## `.mmd` — flowcharts

Plain Mermaid `flowchart LR|TB`; **≤ 12 boxes on the first view, labels of a few words**. CC Deck
draws it interactively (collapsible subgraphs, click-to-trace, search, pan/zoom), so a larger graph
is fine **when grouped**: one `subgraph` per layer or system, detail closed.

```
flowchart LR
  %% collapsed: staging
  raw[(events_raw)] --> stg_events
  subgraph staging [Staging]
    stg_events --> stg_sessions
  end
  stg_sessions -->|daily| fct_retention
  click stg_events "models/staging/stg_events.sql:12" "stg_events"
```

`%% collapsed: a, b` (or `*`) starts groups closed; `click id "path[:line]|url" "tip"` opens a file
or URL. `sequenceDiagram` is drawn natively too (participants, messages, notes, alt/loop blocks);
other Mermaid types are shown as source only — prefer a flowchart, `.lineage` or `.dbml`.

## `.lineage` — Databricks-style lineage

One line each; names are three-part (`catalog.schema.table`) or paths. Any name used only in an
edge is a table. Kinds: `table view mv streaming notebook job pipeline dashboard query model volume
file external`. Columns are optional (`name` or `name:type`).

```
title: Orders
focus: main.silver.orders
main.bronze.orders   table     id, amount:decimal, ts
etl/clean_orders     notebook
kafka.orders         external
kafka.orders > main.bronze.orders
main.bronze.orders, main.bronze.fx > etl/clean_orders > main.silver.orders > revenue_dash
main.silver.orders.amount_usd < main.bronze.orders.amount, main.bronze.fx.rate
```

`a, b > c > d` are edges (`->` works too); `target.col < source.col, …` is column lineage (it also
links the tables). Only edges read from code or catalog lineage, never guessed.

## `.dbml` — database diagrams

Standard DBML: `Table name [headercolor: #hex] { col type [pk, not null, unique, ref: > t.col,
note: '…'] }`, `Ref: a.col > b.col` (`>` many-to-one, `<` one-to-many, `-` one-to-one, `<>`
many-to-many), `Enum`, `TableGroup name [color: #hex] { t1 t2 }`. Only the columns that matter.

## `.rv`, `.rows`, `.doc`

```
? Review: incremental refactor of stg_events
1: models/staging/stg_events.sql:42 | high | dedup keeps the first row | use argMax(value, ingested_at)
```

The user keeps findings and sends; **fix only the kept ones**. Wait with
`claude-deck canvas wait <session dir> <name>.rv 540` (Bash timeout 600000) or take the answer
from the next prompt. `.rows` lines come from counts you actually ran (`? title` first).
`.doc` comments arrive as `¶3: …`.

## Objectives board

Long-lived objectives with tasks, plus each session's live state. A session works on at most one.

- Several steps, or part of a named larger goal: `claude-deck canvas objectives`, reuse a matching
  slug, else ask before `claude-deck canvas attach <session dir> <slug> "Title"` (prints the file).
- Edit that file: `- [ ]` todo, `- [~]` doing, `- [x]` done (verified, not just written),
  `- [!]` blocked, `  > note` under a task. Tasks are outcomes, not tool calls.
- `board:<slug>: …` lines in a prompt are the user's edits: follow them, re-read the file first.

## Rules

- New file per screen with a semantic name; reuse a name only to update that view.
- `.html` is a last resort: it opens in a browser, not in CC Deck.
