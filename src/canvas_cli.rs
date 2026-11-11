//! `claude-deck canvas …`: the agent's tools and the JSON the macOS app reads.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use claude_deck::canvas::{dbml, flow, git, hooks, jira, lineage, objectives, plan, scene, screens, sequence, store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum CanvasCmd {
    /// Agent hook: reads the hook JSON on stdin (session-start, prompt-submit, state).
    Hook { event: String },
    /// Wait until the user answers a screen (.rv, .doc) on the canvas; prints the answer JSON.
    Wait {
        session_dir: PathBuf,
        screen: String,
        #[arg(default_value_t = 540)]
        timeout_sec: u64,
    },
    /// List objectives: slug, status, progress, title.
    Objectives {
        #[arg(long)]
        json: bool,
    },
    /// Work this session on an objective (created if new); prints its file.
    Attach { session_dir: PathBuf, slug: String, title: Option<String> },
    /// Check a screen file parses; prints what it contains or its errors.
    Check { file: PathBuf },
    /// List canvas sessions and their screens.
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Print the canvas home directory.
    Home,
    /// Jira through the canvas's token: `claude-deck canvas jira help`.
    #[command(disable_help_flag = true)]
    Jira {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Git changes of a project: status as JSON, or one file's diff.
    Git {
        dir: PathBuf,
        #[arg(long)]
        diff: Option<String>,
        #[arg(long)]
        untracked: bool,
    },
    /// A screen's model as JSON (review, rows, plan, sequence, doc/open path…).
    Parse { file: PathBuf },
    /// A diagram laid out as JSON (.mmd, .lineage, .dbml).
    Layout {
        file: PathBuf,
        /// Flowchart groups to show closed (comma-separated ids, `*` for all; empty for none).
        #[arg(long)]
        collapsed: Option<String>,
        /// Lineage nodes showing their columns (comma-separated indices).
        #[arg(long)]
        expanded: Option<String>,
    },
    /// Apply a board edit given as JSON: {"op":"state|note|add|status", …}.
    Edit { slug: String, op: String },
    /// Send an answer to the agent of a session, as the canvas does.
    Answer {
        session_dir: PathBuf,
        screen: String,
        #[arg(long = "choice")]
        choices: Vec<String>,
        #[arg(long, default_value = "")]
        note: String,
    },
}

pub fn run(c: CanvasCmd) -> Result<()> {
    match c {
        CanvasCmd::Hook { event } => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
            let v: Value = serde_json::from_str(&input).unwrap_or_default();
            // A hook must never break the agent: report and exit 0.
            let res = match event.as_str() {
                "session-start" => hooks::session_start(&v),
                "prompt-submit" => hooks::prompt_submit(&v),
                "state" => hooks::state(&v),
                other => Err(anyhow::anyhow!("unknown hook {other}")),
            };
            match res {
                Ok(Some(out)) => println!("{out}"),
                Ok(None) => {}
                Err(e) => eprintln!("canvas {event}: {e:#}"),
            }
            Ok(())
        }
        CanvasCmd::Wait { session_dir, screen, timeout_sec } => {
            println!("{}", hooks::wait(&session_dir, &screen, std::time::Duration::from_secs(timeout_sec))?);
            Ok(())
        }
        CanvasCmd::Objectives { json } => {
            let list = objectives::list();
            if json {
                println!("{}", serde_json::to_string(&list)?);
            } else {
                for o in list {
                    println!("{}\t{}\t{}/{}\t{}", o.slug, o.status, o.done(), o.tasks.len(), o.title);
                }
            }
            Ok(())
        }
        CanvasCmd::Attach { session_dir, slug, title } => {
            if !session_dir.join("meta.json").exists() {
                bail!("not a canvas session dir: {}", session_dir.display());
            }
            let f = objectives::create(&slug, title.as_deref().unwrap_or(""))?;
            store::write_meta(&session_dir, |m| m.objective = slug.clone())?;
            println!("{}", f.display());
            Ok(())
        }
        CanvasCmd::Check { file } => check(&file),
        CanvasCmd::Ls { json } => {
            let list = store::list_sessions();
            if json {
                println!("{}", serde_json::to_string(&list)?);
                return Ok(());
            }
            for s in list {
                let state = if s.state.state.is_empty() { "-" } else { &s.state.state };
                println!("{}  {}  [{}]  {}", s.meta.id, s.meta.project, state, s.label());
                for i in s.items.iter().take(10) {
                    println!("    {}", i.name);
                }
            }
            Ok(())
        }
        CanvasCmd::Home => {
            println!("{}", store::home().display());
            Ok(())
        }
        CanvasCmd::Jira { args } => {
            println!("{}", jira_cli(args)?);
            Ok(())
        }
        CanvasCmd::Git { dir, diff, untracked } => {
            match diff {
                Some(f) => print!("{}", git::diff(&dir, &f, untracked)),
                None => match git::status(&dir) {
                    Some(s) => println!("{}", serde_json::to_string(&s)?),
                    None => println!("{}", json!({"error": "not a git repository"})),
                },
            }
            Ok(())
        }
        CanvasCmd::Parse { file } => {
            let text = read(&file)?;
            let v = match ext(&file).as_str() {
                "rv" => {
                    let r = screens::parse_review(&text);
                    json!({"kind": "review", "title": r.title, "findings": r.findings.iter().map(|f| json!({"key": f.key, "file": f.file, "line": f.line, "severity": f.severity, "message": f.message, "fix": f.fix})).collect::<Vec<_>>()})
                }
                "rows" => {
                    let r = screens::parse_rows(&text);
                    json!({"kind": "rows", "title": r.title, "steps": r.steps.iter().map(|s| json!({"name": s.name, "in": s.rows_in, "out": s.rows_out, "cols": s.cols})).collect::<Vec<_>>()})
                }
                "plan" => {
                    let p = plan::parse(&text);
                    let keys = p.keys();
                    let live = if !keys.is_empty() && jira::configured() { jira::statuses(&keys).unwrap_or_default() } else { Default::default() };
                    json!({"kind": "plan", "plan": p, "live": live, "base": jira::base()})
                }
                "doc" | "open" => {
                    let (p, line) = screens::parse_path(&text);
                    json!({"kind": ext(&file), "path": p, "line": line})
                }
                "mmd" => match flow::parse(&text) {
                    Err(flow::Error::Unsupported(k)) if k == "sequenceDiagram" => scene::sequence_scene(&sequence::parse(&text)),
                    Err(e) => json!({"kind": "source", "error": e.to_string()}),
                    Ok(_) => json!({"kind": "flow"}),
                },
                "md" => json!({"kind": "markdown", "blocks": screens::blocks(&text)}),
                e => json!({"kind": e}),
            };
            println!("{v}");
            Ok(())
        }
        CanvasCmd::Layout { file, collapsed, expanded } => {
            let text = read(&file)?;
            let v = match ext(&file).as_str() {
                "mmd" => match flow::parse(&text) {
                    Ok(f) => {
                        let c: Option<Vec<String>> = collapsed.map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
                        scene::flow_scene(&f, c.as_deref())
                    }
                    Err(flow::Error::Unsupported(k)) if k == "sequenceDiagram" => scene::sequence_scene(&sequence::parse(&text)),
                    Err(e) => json!({"kind": "source", "error": e.to_string()}),
                },
                "lineage" => {
                    let e: Option<Vec<usize>> = expanded.map(|c| c.split(',').filter_map(|s| s.trim().parse().ok()).collect());
                    scene::lineage_scene(&lineage::parse(&text), e.as_deref())
                }
                "dbml" => scene::erd_scene(&dbml::parse(&text)),
                e => bail!("no layout for .{e}"),
            };
            println!("{v}");
            Ok(())
        }
        CanvasCmd::Edit { slug, op } => {
            let v: Value = serde_json::from_str(&op).context("op must be JSON")?;
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            let line = v["line"].as_u64().unwrap_or(0) as usize;
            let o = match s("op").as_str() {
                "state" => objectives::Op::State { line, text: s("text"), state: s("state") },
                "note" => objectives::Op::Note { line, text: s("text"), note: s("note") },
                "add" => objectives::Op::Add { text: s("text") },
                "status" => objectives::Op::Status { status: s("status") },
                o => bail!("unknown op {o}"),
            };
            println!("{}", objectives::apply(&slug, &o)?);
            Ok(())
        }
        CanvasCmd::Answer { session_dir, screen, choices, note } => {
            store::append_event(&session_dir, &screen, choices, &note)?;
            Ok(())
        }
    }
}

fn read(f: &Path) -> Result<String> {
    std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))
}

fn ext(f: &Path) -> String {
    f.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
}

fn check(file: &Path) -> Result<()> {
    let text = read(file)?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match store::Kind::of(&name) {
        Some(store::Kind::Flow) => match flow::parse(&text) {
            Ok(f) => println!("flowchart: {} nodes ({} groups), {} edges", f.nodes.len(), f.nodes.iter().filter(|n| n.group).count(), f.edges.len()),
            Err(flow::Error::Unsupported(k)) if k == "sequenceDiagram" => {
                let s = sequence::parse(&text);
                println!("sequence: {} participants, {} events", s.participants.len(), s.events.len());
                if !s.errors.is_empty() {
                    bail!("{}", s.errors.join("\n"));
                }
            }
            Err(e) => bail!("{e}"),
        },
        Some(store::Kind::Lineage) => {
            let l = lineage::parse(&text);
            println!("lineage: {} nodes, {} edges, {} column links", l.nodes.len(), l.edges.len(), l.col_edges.len());
            if !l.errors.is_empty() {
                bail!("{}", l.errors.join("\n"));
            }
        }
        Some(store::Kind::Erd) => {
            let e = dbml::parse(&text);
            println!("dbml: {} tables, {} refs, {} enums, {} groups", e.tables.len(), e.refs.len(), e.enums.len(), e.groups.len());
            if !e.errors.is_empty() {
                bail!("{}", e.errors.join("\n"));
            }
        }
        Some(store::Kind::Plan) => {
            let p = plan::parse(&text);
            println!("plan: {} items, {} pt", p.items.len(), p.total);
            if !p.errors.is_empty() {
                bail!("{}", p.errors.join("\n"));
            }
        }
        Some(store::Kind::Review) => println!("review: {} findings", screens::parse_review(&text).findings.len()),
        Some(store::Kind::Rows) => println!("rows: {} steps", screens::parse_rows(&text).steps.len()),
        Some(k) => println!("{k:?}: ok"),
        None => bail!("not a canvas screen: {name}"),
    }
    Ok(())
}

const JIRA_HELP: &str = "claude-deck canvas jira <command> [--json]
  status                          who you are signed in as
  mine                            your open issues
  search \"<jql>\"                  issues matching JQL (50 max)
  show KEY                        details, comments, and the statuses it can move to
  boards [PROJECT]                boards (id, name)
  board ID [--scope sprint|backlog|all]   board columns (the active sprint by default)
  sprints BOARD                   active and future sprints (id, name)
  epics PROJECT                   epics with progress
  sprint KEY[,KEY…] <sprint id|backlog>   move issues into a sprint or back (writes to Jira)
  move KEY \"<status>\"             change status (writes to Jira)
  comment KEY \"<text>\"            add a comment (writes to Jira)
  assign KEY me|none|<accountId>  change assignee (writes to Jira)
  create PROJECT \"<summary>\" [--type Task] [--parent KEY] [--description \"<text>\"]   (writes to Jira)
  edit KEY [--summary …] [--description …] [--points N] [--labels a,b] [--due YYYY-MM-DD] [--assign me|none]   (writes to Jira)
  link A blocks|blocked-by|relates|duplicates B   (writes to Jira)
  unlink A B                      remove the links between A and B (writes to Jira)
  graph EPIC <session dir>        draw the epic's dependencies as a canvas screen
  graph-keys KEY,KEY… <session dir> [name]   dependencies among any issues
  plan <file>.plan                check a plan: items, points, keys, errors
  push <file>.plan                create the plan's new issues (writes to Jira)
  plan-graph <file>.plan          draw the plan's dependencies next to it
Credentials: JIRA_BASE_URL, JIRA_EMAIL, JIRA_API_TOKEN in the environment or in";

fn jira_cli(mut args: Vec<String>) -> Result<String> {
    let as_json = if let Some(i) = args.iter().position(|a| a == "--json") {
        args.remove(i);
        true
    } else {
        false
    };
    let mut flag = |name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == &format!("--{name}"))?;
        args.remove(i);
        (i < args.len()).then(|| args.remove(i))
    };
    let scope = flag("scope");
    let kind = flag("type");
    let parent = flag("parent");
    let description = flag("description");
    let summary = flag("summary");
    let points = flag("points");
    let labels = flag("labels");
    let due = flag("due");
    let assign_to = flag("assign");
    let cmd = args.first().cloned().unwrap_or_else(|| "help".into());
    let a = |i: usize| args.get(i).cloned().unwrap_or_default();
    let out = |v: Value, text: String| -> String { if as_json { v.to_string() } else { text } };
    let lines = |v: &[Value], empty: &str| -> String {
        if v.is_empty() {
            empty.to_string()
        } else {
            v.iter().map(jira::issue_line).collect::<Vec<_>>().join("\n")
        }
    };
    Ok(match cmd.as_str() {
        "help" | "-h" | "--help" => format!("{JIRA_HELP} {}", jira::env_file().display()),
        "status" => match jira::config() {
            None => out(json!({"configured": false}), format!("not configured: set JIRA_BASE_URL, JIRA_EMAIL, JIRA_API_TOKEN in {}", jira::env_file().display())),
            Some(c) => {
                let me = jira::myself()?;
                out(json!({"configured": true, "base": c.base, "user": me}), format!("signed in to {} as {} <{}>", c.base, me["name"].as_str().unwrap_or(""), me["email"].as_str().unwrap_or("")))
            }
        },
        "mine" => {
            let v = jira::mine()?;
            out(json!(v), lines(&v, "no open issues assigned to you"))
        }
        "search" => {
            let v = jira::search(&a(1), 50)?;
            out(json!(v), lines(&v, "no matches"))
        }
        "show" => {
            let i = jira::issue(&a(1))?;
            let s = |k: &str| i[k].as_str().unwrap_or("").to_string();
            let mut t = vec![format!("{}  {}", s("key"), s("summary")), format!("{} · {} · assignee {} · reporter {}", s("type"), s("status"), s("assignee"), s("reporter"))];
            if !i["points"].is_null() {
                t.push(format!("{} pt", i["points"]));
            }
            if !s("duedate").is_empty() {
                t.push(format!("due {}", s("duedate")));
            }
            for l in i["links"].as_array().cloned().unwrap_or_default() {
                t.push(format!("  {} {} [{}] {}", l["label"].as_str().unwrap_or(""), l["key"].as_str().unwrap_or(""), l["status"].as_str().unwrap_or(""), l["summary"].as_str().unwrap_or("")));
            }
            t.push(s("url"));
            if !s("description").is_empty() {
                t.push(format!("\n{}", s("description")));
            }
            for c in i["comments"].as_array().cloned().unwrap_or_default().iter().rev().take(5).rev() {
                t.push(format!("  {} ({}): {}", c["author"].as_str().unwrap_or(""), &c["created"].as_str().unwrap_or("")[..10.min(c["created"].as_str().unwrap_or("").len())], c["text"].as_str().unwrap_or("").replace('\n', " ")));
            }
            t.push(format!("can move to: {}", i["transitions"].as_array().map(|a| a.iter().filter_map(|x| x["to"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default()));
            out(i.clone(), t.join("\n"))
        }
        "boards" => {
            let v = jira::boards(&a(1))?;
            let t = v.iter().map(|b| format!("{}  {}  ({}{})", b["id"], b["name"].as_str().unwrap_or(""), b["type"].as_str().unwrap_or(""), b["project"].as_str().map(|p| if p.is_empty() { String::new() } else { format!(", {p}") }).unwrap_or_default())).collect::<Vec<_>>().join("\n");
            out(json!(v), t)
        }
        "board" => {
            let b = jira::board(a(1).parse().context("board id")?, scope.as_deref().unwrap_or("sprint"))?;
            let mut t = vec![format!("{}{}", b["name"].as_str().unwrap_or(""), b["sprint"]["name"].as_str().map(|n| format!(" · {n}")).unwrap_or_default())];
            for c in b["columns"].as_array().cloned().unwrap_or_default() {
                let iss = c["issues"].as_array().cloned().unwrap_or_default();
                t.push(format!("\n{} ({})", c["name"].as_str().unwrap_or(""), iss.len()));
                t.extend(iss.iter().map(|i| format!("  {}", jira::issue_line(i))));
            }
            out(b.clone(), t.join("\n"))
        }
        "sprints" => {
            let v = jira::sprints(a(1).parse().context("board id")?)?;
            let t = v.iter().map(|x| format!("{}  {}  ({})", x["id"], x["name"].as_str().unwrap_or(""), x["state"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n");
            out(json!(v), t)
        }
        "epics" => {
            let v = jira::epics(&a(1))?;
            let t = v.iter().map(|e| format!("{}  {}  {}/{} done · {} pt", e["key"].as_str().unwrap_or(""), e["summary"].as_str().unwrap_or(""), e["done"], e["total"], e["points"])).collect::<Vec<_>>().join("\n");
            out(json!(v), t)
        }
        "sprint" => jira::set_sprint(&a(1).split(',').filter(|k| !k.is_empty()).map(String::from).collect::<Vec<_>>(), &a(2))?,
        "move" => jira::move_to(&a(1), &a(2))?,
        "comment" => jira::comment(&a(1), &a(2))?,
        "assign" => jira::assign(&a(1), if a(2).is_empty() { "me" } else { &args[2] })?,
        "create" => {
            let key = jira::create(&jira::NewIssue {
                project: &a(1),
                summary: &a(2),
                kind: kind.as_deref().unwrap_or("Task"),
                parent: parent.as_deref(),
                description: description.as_deref(),
                points: None,
                labels: vec![],
                assignee: None,
            })?;
            out(json!({"key": key}), format!("created {key}"))
        }
        "edit" => {
            let mut ch = serde_json::Map::new();
            for (k, v) in [("summary", summary), ("description", description), ("points", points), ("labels", labels), ("duedate", due), ("assignee", assign_to)] {
                if let Some(v) = v {
                    ch.insert(k.into(), json!(v));
                }
            }
            jira::edit(&a(1), &Value::Object(ch))?
        }
        "link" => jira::link(&a(1), &a(2), &a(3))?,
        "unlink" => jira::unlink(&a(1), &a(2))?,
        "graph" | "graph-keys" => {
            let dir = PathBuf::from(a(2));
            if !dir.join("content").is_dir() {
                bail!("{} is not a canvas session dir", dir.display());
            }
            let (mermaid, count, edges, name) = if cmd == "graph" {
                let (m, c, e) = jira::epic_graph(&a(1))?;
                (m, c, e, format!("{}-deps.mmd", a(1)))
            } else {
                let keys: Vec<String> = a(1).split(',').filter(|k| !k.is_empty()).map(String::from).collect();
                let (m, c, e) = jira::keys_graph(&keys, "issue dependencies")?;
                let n = if a(3).is_empty() { "issue-deps".to_string() } else { a(3) };
                (m, c, e, format!("{}.mmd", n.trim_end_matches(".mmd")))
            };
            std::fs::write(dir.join("content").join(&name), mermaid)?;
            out(json!({"name": name, "count": count, "edges": edges}), format!("{name}: {count} issues, {edges} dependencies — open on the canvas"))
        }
        "plan" => {
            let p = plan::parse(&read(Path::new(&a(1)))?);
            let keys = p.keys();
            let live = if keys.is_empty() { Default::default() } else { jira::statuses(&keys).unwrap_or_default() };
            let tag = |k: &str| if k.is_empty() { "new".to_string() } else { format!("{k}{}", live.get(k).and_then(|l| l["status"].as_str()).map(|s| format!(" [{s}]")).unwrap_or_default()) };
            let mut t = vec![format!("{} · Epic: {} ({}) · {} pt", p.project, if p.epic.summary.is_empty() { &p.epic.key } else { &p.epic.summary }, tag(&p.epic.key), p.total)];
            for it in &p.items {
                t.push(format!("  {}: {} · {} pt · {}{}", it.kind, it.f.summary, it.f.points.map(|x| x.to_string()).unwrap_or("-".into()), tag(&it.f.key), if it.ac.is_empty() { String::new() } else { format!(" · {} AC", it.ac.len()) }));
                for st in &it.subtasks {
                    t.push(format!("    Subtask: {} · {} pt · {}", st.f.summary, st.f.points.map(|x| x.to_string()).unwrap_or("-".into()), tag(&st.f.key)));
                }
            }
            t.extend(p.errors.iter().map(|e| format!("error: {e}")));
            out(json!({"plan": p, "live": live}), t.join("\n"))
        }
        "push" => {
            let file = PathBuf::from(a(1));
            let r = plan::push(&file)?;
            if !r.created.is_empty() || !r.linked.is_empty() {
                if let Some(dir) = file.parent().and_then(|c| c.parent()) {
                    let mut all = r.created.clone();
                    all.extend(r.linked.clone());
                    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let _ = store::append_event(dir, &name, vec![format!("pushed to Jira: {}", all.join("; "))], "");
                }
            }
            let mut t = vec![if r.created.is_empty() { "nothing new to create".to_string() } else { format!("created:\n{}", r.created.iter().map(|x| format!("  {x}")).collect::<Vec<_>>().join("\n")) }];
            if !r.linked.is_empty() {
                t.push(format!("linked:\n{}", r.linked.iter().map(|x| format!("  {x}")).collect::<Vec<_>>().join("\n")));
            }
            out(serde_json::to_value(&r)?, t.join("\n"))
        }
        "plan-graph" => {
            let file = PathBuf::from(a(1));
            let p = plan::parse(&read(&file)?);
            let keys: Vec<String> = p.items.iter().map(|i| i.f.key.clone()).filter(|k| !k.is_empty()).collect();
            let live = if keys.is_empty() || !jira::configured() { Default::default() } else { jira::statuses(&keys).unwrap_or_default() };
            let out_file = file.with_file_name(format!("{}-deps.mmd", file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
            std::fs::write(&out_file, plan::graph(&p, &live, &jira::base()))?;
            out(json!({"name": out_file.file_name().map(|n| n.to_string_lossy().to_string())}), format!("wrote {}", out_file.display()))
        }
        other => bail!("unknown jira command: {other}\n{JIRA_HELP} {}", jira::env_file().display()),
    })
}
