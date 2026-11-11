mod keys;
mod launch;
mod session;
mod tui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use claude_deck::config::Config;
use claude_deck::{accounts, usage};
use std::io::{BufRead, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use claude_deck::usage::Usage;

/// Run Claude Code sessions side by side, each on its own account.
#[derive(Parser)]
#[command(name = "claude-deck", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Account for the first session (skips the picker).
    #[arg(short, long)]
    account: Option<String>,
    /// Arguments passed to claude for the first session (after `--`).
    #[arg(last = true)]
    claude_args: Vec<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create an account profile and log in to it.
    Add {
        name: String,
        /// Create the profile without launching claude to log in.
        #[arg(long)]
        no_login: bool,
    },
    /// Remove an account profile and its stored login.
    Rm { name: String },
    /// Show every account with its usage.
    #[command(alias = "ls")]
    List,
    /// Run claude in this terminal under an account, without the TUI.
    Run {
        /// Account name or email; picks the mapped account or asks when omitted.
        name: Option<String>,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Map a directory (default: current) to an account.
    Map { account: String, dir: Option<PathBuf> },
    /// Remove a directory mapping (default: current directory).
    Unmap { dir: Option<PathBuf> },
    /// List directory mappings.
    Maps,
    /// Print `export CLAUDE_CONFIG_DIR=...` for an account (eval it in a shell).
    Env { name: String },
    /// Exclude an account from automatic choices.
    Disable { name: String },
    /// Put a disabled account back into automatic choices.
    Enable { name: String },
    /// Recreate the shared ~/.claude symlinks in every profile.
    Link,
    /// Show settings, or set one: `config threshold 80`.
    Config { key: Option<String>, value: Option<String> },
    /// Run claude on a chosen account and keep it movable (used by the shell `claude` function).
    Launch {
        #[arg(long)]
        account: Option<String>,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Accounts, usage and settings as JSON (for the desktop app).
    State {
        /// Also append a usage sample to the history used by the dashboard.
        #[arg(long)]
        record: bool,
    },
    /// Token, cost and activity statistics as JSON.
    Stats {
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
    /// Append an entry to the event log.
    Event {
        kind: String,
        message: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// Canvas: screens, objectives and the agent hooks.
    #[command(subcommand)]
    Canvas(CanvasCmd),
}

#[derive(Subcommand)]
enum CanvasCmd {
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
    Objectives,
    /// Work this session on an objective (created if new); prints its file.
    Attach { session_dir: PathBuf, slug: String, title: Option<String> },
    /// Check a screen file parses; prints what it contains or its errors.
    Check { file: PathBuf },
    /// List canvas sessions and their screens.
    Ls,
    /// Print the canvas home directory.
    Home,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load()?;
    match cli.cmd {
        None => tui::run(cfg, tui::Options { account: cli.account, claude_args: cli.claude_args }),
        Some(Cmd::Add { name, no_login }) => {
            let acc = accounts::create(&name, &cfg)?;
            println!("created {} at {}", acc.name, acc.dir().display());
            if no_login {
                return Ok(());
            }
            println!("launching claude for {name}: log in, then exit with /exit");
            let mut c = Command::new(&cfg.claude_bin);
            acc.apply_env(&mut c);
            c.status().context("running claude")?;
            match acc.email() {
                Some(e) => println!("{name} is logged in as {e}"),
                None => println!("{name} has no login yet; run `claude-deck run {name}` and use /login"),
            }
            Ok(())
        }
        Some(Cmd::Rm { name }) => {
            accounts::remove(&name)?;
            cfg.maps.retain(|_, a| a != &name);
            cfg.disabled.retain(|a| a != &name);
            cfg.save()?;
            println!("removed {name}");
            Ok(())
        }
        Some(Cmd::List) => list(&cfg),
        Some(Cmd::Launch { account, args }) => {
            let code = launch::run(&cfg, account, args)?;
            std::process::exit(code);
        }
        Some(Cmd::State { record }) => state_json(&cfg, record),
        Some(Cmd::Stats { days }) => {
            println!("{}", serde_json::to_string(&claude_deck::stats::compute(days)?)?);
            Ok(())
        }
        Some(Cmd::Canvas(c)) => canvas_cmd(c),
        Some(Cmd::Event { kind, message, account }) => {
            claude_deck::stats::log_event(&kind, account.as_deref(), &message, serde_json::json!({}));
            Ok(())
        }
        Some(Cmd::Run { name, args }) => {
            let cwd = std::env::current_dir()?;
            let name = match name.or_else(|| cfg.account_for(&cwd)) {
                Some(n) => n,
                None => prompt_account()?,
            };
            let acc = accounts::find(&name)?;
            accounts::link_shared(&acc, &cfg)?;
            let mut c = Command::new(&cfg.claude_bin);
            c.args(&args);
            acc.apply_env(&mut c);
            Err(c.exec()).context("starting claude")
        }
        Some(Cmd::Map { account, dir }) => {
            let acc = accounts::find(&account)?;
            let dir = abs_dir(dir)?;
            cfg.maps.insert(dir.to_string_lossy().into(), acc.name.clone());
            cfg.save()?;
            println!("{} → {}", dir.display(), acc.name);
            Ok(())
        }
        Some(Cmd::Unmap { dir }) => {
            let dir = abs_dir(dir)?;
            if cfg.maps.remove(dir.to_string_lossy().as_ref()).is_none() {
                bail!("{} is not mapped", dir.display());
            }
            cfg.save()?;
            println!("unmapped {}", dir.display());
            Ok(())
        }
        Some(Cmd::Maps) => {
            if cfg.maps.is_empty() {
                println!("no mappings (claude-deck map <account> [dir])");
            }
            for (d, a) in &cfg.maps {
                println!("{d} → {a}");
            }
            Ok(())
        }
        Some(Cmd::Env { name }) => {
            let acc = accounts::find(&name)?;
            match &acc.config_dir {
                Some(d) => println!("export CLAUDE_CONFIG_DIR={}", shell_quote(&d.to_string_lossy())),
                None => println!("unset CLAUDE_CONFIG_DIR"),
            }
            Ok(())
        }
        Some(Cmd::Disable { name }) => {
            let acc = accounts::find(&name)?;
            if !cfg.disabled.contains(&acc.name) {
                cfg.disabled.push(acc.name.clone());
            }
            cfg.save()?;
            println!("{} disabled for automatic choices", acc.name);
            Ok(())
        }
        Some(Cmd::Enable { name }) => {
            cfg.disabled.retain(|a| a != &name);
            cfg.save()?;
            println!("{name} enabled");
            Ok(())
        }
        Some(Cmd::Link) => {
            for acc in accounts::list() {
                for c in accounts::link_shared(&acc, &cfg)? {
                    println!("{}: {c} exists in the profile, left as is", acc.name);
                }
            }
            println!("links up to date");
            Ok(())
        }
        Some(Cmd::Config { key, value }) => {
            match (key, value) {
                (Some(k), Some(v)) => {
                    cfg.set(&k, &v)?;
                    cfg.save()?;
                    println!("{k} = {v}");
                }
                (Some(k), None) => bail!("missing value for {k}"),
                _ => print!("# {}\n{}", Config::path().display(), toml::to_string_pretty(&cfg)?),
            }
            Ok(())
        }
    }
}

fn list(cfg: &Config) -> Result<()> {
    let accs = accounts::list();
    let handles: Vec<_> = accs
        .iter()
        .cloned()
        .map(|a| std::thread::spawn(move || usage::fetch(&a)))
        .collect();
    println!("{:<14} {:<32} {:<6} {:>16} {:>16}", "ACCOUNT", "EMAIL", "PLAN", "5H", "7D");
    for (acc, h) in accs.iter().zip(handles) {
        let u = h.join().unwrap_or(Usage::Error("panic".into()));
        let email = acc.email().unwrap_or_else(|| "-".into());
        let fmt = |w: &Option<usage::Window>| match w {
            Some(w) => format!(
                "{:>3.0}% {}",
                w.current_pct(),
                w.resets_at.map(|t| format!("↻{}", usage::until(t))).unwrap_or_default()
            ),
            None => "-".into(),
        };
        let mut name = acc.name.clone();
        if cfg.disabled.contains(&acc.name) {
            name.push_str(" (off)");
        }
        match &u {
            Usage::Ok { five, week, others, plan } => {
                println!(
                    "{:<14} {:<32} {:<6} {:>16} {:>16}",
                    name,
                    email,
                    plan.clone().unwrap_or_default(),
                    fmt(five),
                    fmt(week)
                );
                for w in others {
                    println!("{:<14}   {} {:.0}%", "", w.name, w.current_pct());
                }
            }
            Usage::Error(e) => println!("{name:<14} {email:<32} error: {e}"),
            other => println!("{name:<14} {email:<32} {}", other.label()),
        }
    }
    Ok(())
}

fn window_json(w: &Option<usage::Window>) -> serde_json::Value {
    match w {
        Some(w) => serde_json::json!({
            "name": w.name,
            "pct": w.current_pct(),
            "resets_at": w.resets_at.map(|t| t.to_rfc3339()),
            "resets_in": w.resets_at.map(usage::until).unwrap_or_default(),
        }),
        None => serde_json::Value::Null,
    }
}

fn state_json(cfg: &Config, record: bool) -> Result<()> {
    use serde_json::json;
    let accs = accounts::list();
    let handles: Vec<_> = accs.iter().cloned().map(|a| std::thread::spawn(move || usage::fetch(&a))).collect();
    let now = chrono::Utc::now().to_rfc3339();
    let mut out = vec![];
    for (acc, h) in accs.iter().zip(handles) {
        let u = h.join().unwrap_or(Usage::Error("panic".into()));
        let (state, error, five, week, others, plan) = match &u {
            Usage::Ok { five, week, others, plan } => (
                "ok",
                None,
                window_json(five),
                window_json(week),
                others.iter().map(|w| window_json(&Some(w.clone()))).collect::<Vec<_>>(),
                plan.clone(),
            ),
            Usage::Unknown => ("loading", None, json!(null), json!(null), vec![], None),
            Usage::NotLoggedIn => ("logged_out", None, json!(null), json!(null), vec![], None),
            Usage::Expired => ("expired", None, json!(null), json!(null), vec![], None),
            Usage::Error(e) => ("error", Some(e.clone()), json!(null), json!(null), vec![], None),
        };
        if record {
            if let Usage::Ok { five, week, .. } = &u {
                let _ = claude_deck::stats::append_jsonl(
                    &claude_deck::stats::history_path(),
                    &json!({"ts": now, "account": acc.name,
                            "five": five.as_ref().map(|w| w.current_pct()),
                            "week": week.as_ref().map(|w| w.current_pct())}),
                );
            }
        }
        out.push(json!({
            "name": acc.name,
            "email": acc.email(),
            "account_uuid": acc.account_uuid(),
            "plan": plan,
            "disabled": cfg.disabled.contains(&acc.name),
            "state": state,
            "error": error,
            "five": five,
            "week": week,
            "others": others,
            "over": u.over(cfg),
            "pressure": u.pressure(cfg),
        }));
    }
    let v = json!({
        "accounts": out,
        "config": {
            "threshold": cfg.threshold,
            "weekly_threshold": cfg.weekly_threshold,
            "auto_switch": cfg.auto_switch,
            "idle_secs": cfg.idle_secs,
            "cooldown_secs": cfg.cooldown_secs,
            "poll_secs": cfg.poll_secs,
            "maps": cfg.maps,
        }
    });
    println!("{v}");
    Ok(())
}

fn prompt_account() -> Result<String> {
    let accs = accounts::list();
    if accs.len() == 1 {
        return Ok(accs[0].name.clone());
    }
    for (i, a) in accs.iter().enumerate() {
        println!("  {}) {:<14} {}", i + 1, a.name, a.email().unwrap_or_default());
    }
    print!("account [1]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim();
    if line.is_empty() {
        return Ok(accs[0].name.clone());
    }
    if let Ok(n) = line.parse::<usize>() {
        if (1..=accs.len()).contains(&n) {
            return Ok(accs[n - 1].name.clone());
        }
    }
    Ok(line.to_string())
}

fn abs_dir(dir: Option<PathBuf>) -> Result<PathBuf> {
    let d = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    d.canonicalize().with_context(|| format!("{} does not exist", d.display()))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn canvas_cmd(c: CanvasCmd) -> Result<()> {
    use claude_deck::canvas::{dbml, flow, hooks, lineage, objectives, screens, store};
    match c {
        CanvasCmd::Hook { event } => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
            let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
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
            let v = hooks::wait(&session_dir, &screen, std::time::Duration::from_secs(timeout_sec))?;
            println!("{v}");
            Ok(())
        }
        CanvasCmd::Objectives => {
            for o in objectives::list() {
                println!("{}\t{}\t{}/{}\t{}", o.slug, o.status, o.done(), o.tasks.len(), o.title);
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
        CanvasCmd::Check { file } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match store::Kind::of(&name) {
                Some(store::Kind::Flow) => match flow::parse(&text) {
                    Ok(f) => println!("flowchart: {} nodes ({} groups), {} edges", f.nodes.len(), f.nodes.iter().filter(|n| n.group).count(), f.edges.len()),
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
                Some(store::Kind::Review) => println!("review: {} findings", screens::parse_review(&text).findings.len()),
                Some(store::Kind::Rows) => println!("rows: {} steps", screens::parse_rows(&text).steps.len()),
                Some(k) => println!("{k:?}: ok"),
                None => bail!("not a canvas screen: {name}"),
            }
            Ok(())
        }
        CanvasCmd::Ls => {
            for s in store::list_sessions() {
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
    }
}
