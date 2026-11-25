mod canvas_cli;
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
use std::path::{Path, PathBuf};
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
    /// Add an account (or renew its token): runs `claude setup-token`, then asks for the token.
    Add {
        name: String,
        /// Use this token instead of running `claude setup-token` (`-` reads it from stdin).
        #[arg(long, conflicts_with = "config_dir")]
        token: Option<String>,
        /// Run this account on its own Claude Code config dir (CLAUDE_CONFIG_DIR, e.g.
        /// ~/.claude-bravestars) with the login, settings, plugins and history kept there.
        #[arg(long)]
        config_dir: Option<PathBuf>,
    },
    /// Remove an account and its stored token.
    Rm { name: String },
    /// Show every account with its usage.
    #[command(alias = "ls")]
    List,
    /// Run claude in this terminal under an account, without the TUI.
    Run {
        /// Account name or email; picks the mapped account or asks when omitted.
        name: Option<String>,
        /// Profile (Claude Code config) a token account runs on; default: the folder's, else `default`.
        #[arg(long)]
        profile: Option<String>,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// List profiles: the Claude Code configs sessions run on (`default` and config-dir accounts).
    Profiles,
    /// Map a directory (default: current) to an account, and optionally a profile.
    Map {
        account: String,
        dir: Option<PathBuf>,
        /// Profile token accounts run on in this directory.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Remove a directory mapping (default: current directory).
    Unmap { dir: Option<PathBuf> },
    /// List directory mappings.
    Maps,
    /// Print the shell exports that put claude on an account (eval it in a shell).
    Env {
        name: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Exclude an account from automatic choices.
    Disable { name: String },
    /// Put a disabled account back into automatic choices.
    Enable { name: String },
    /// Status line command of CC Deck sessions: records the account's usage, then runs yours.
    #[command(hide = true)]
    Statusline,
    /// Show settings, or set one: `config threshold 80`.
    Config { key: Option<String>, value: Option<String> },
    /// Run claude on a chosen account and keep it movable (used by the shell `claude` function).
    Launch {
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        profile: Option<String>,
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
    Canvas(canvas_cli::CanvasCmd),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load()?;
    match cli.cmd {
        None => tui::run(cfg, tui::Options { account: cli.account, claude_args: cli.claude_args }),
        Some(Cmd::Add { name, config_dir: Some(dir), .. }) => add_dir(&name, &dir),
        Some(Cmd::Add { name, token, .. }) => add(&cfg, &name, token),
        Some(Cmd::Rm { name }) => {
            accounts::remove(&name)?;
            cfg.maps.retain(|_, a| a != &name);
            cfg.disabled.retain(|a| a != &name);
            cfg.save()?;
            println!("removed {name}");
            Ok(())
        }
        Some(Cmd::List) => list(&cfg),
        Some(Cmd::Launch { account, profile, args }) => {
            let code = launch::run(&cfg, account, profile, args)?;
            std::process::exit(code);
        }
        Some(Cmd::State { record }) => state_json(&cfg, record),
        Some(Cmd::Stats { days }) => {
            println!("{}", serde_json::to_string(&claude_deck::stats::compute(days)?)?);
            Ok(())
        }
        Some(Cmd::Canvas(c)) => canvas_cli::run(c),
        Some(Cmd::Event { kind, message, account }) => {
            claude_deck::stats::log_event(&kind, account.as_deref(), &message, serde_json::json!({}));
            Ok(())
        }
        Some(Cmd::Run { name, profile, args }) => {
            let cwd = std::env::current_dir()?;
            let name = match name.or_else(|| cfg.account_for(&cwd)) {
                Some(n) => n,
                None => prompt_account()?,
            };
            let acc = accounts::for_session(&name, profile.as_deref(), &cfg, &cwd)?;
            let mut c = Command::new(&cfg.claude_bin);
            c.args(acc.claude_args(&cwd, &args));
            acc.apply_env(&mut c);
            Err(c.exec()).context("starting claude")
        }
        Some(Cmd::Map { account, dir, profile }) => {
            let acc = accounts::find_on(&account, profile.as_deref())?;
            let dir = abs_dir(dir)?;
            let key: String = dir.to_string_lossy().into();
            cfg.maps.insert(key.clone(), acc.name.clone());
            match &profile {
                Some(p) => {
                    cfg.profile_maps.insert(key, p.clone());
                }
                None => {
                    cfg.profile_maps.remove(&key);
                }
            }
            cfg.save()?;
            let on = profile.map(|p| format!(" on profile {p}")).unwrap_or_default();
            println!("{} → {}{on}", dir.display(), acc.name);
            Ok(())
        }
        Some(Cmd::Profiles) => {
            for p in accounts::profiles() {
                println!("{:<14} {}", p.name, p.dir().display());
            }
            Ok(())
        }
        Some(Cmd::Unmap { dir }) => {
            let dir = abs_dir(dir)?;
            let key = dir.to_string_lossy().to_string();
            let had_profile = cfg.profile_maps.remove(&key).is_some();
            if cfg.maps.remove(&key).is_none() && !had_profile {
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
                match cfg.profile_maps.get(d) {
                    Some(p) => println!("{d} → {a} on profile {p}"),
                    None => println!("{d} → {a}"),
                }
            }
            for (d, p) in cfg.profile_maps.iter().filter(|(d, _)| !cfg.maps.contains_key(*d)) {
                println!("{d} → profile {p}");
            }
            Ok(())
        }
        Some(Cmd::Env { name, profile }) => {
            let acc = accounts::find_on(&name, profile.as_deref())?;
            for (k, v) in acc.env() {
                match v {
                    Some(v) => println!("export {k}={}", accounts::shell_quote(&v)),
                    None => println!("unset {k}"),
                }
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
        Some(Cmd::Statusline) => statusline(),
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
            Usage::NoData => ("no_data", None, json!(null), json!(null), vec![], None),
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
            "has_login": acc.has_login(),
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
        "profiles": accounts::profiles().iter().map(|p| json!({"name": p.name, "dir": p.dir()})).collect::<Vec<_>>(),
        "config": {
            "threshold": cfg.threshold,
            "weekly_threshold": cfg.weekly_threshold,
            "auto_switch": cfg.auto_switch,
            "idle_secs": cfg.idle_secs,
            "cooldown_secs": cfg.cooldown_secs,
            "poll_secs": cfg.poll_secs,
            "maps": cfg.maps,
            "profile_maps": cfg.profile_maps,
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

fn add(cfg: &Config, name: &str, token: Option<String>) -> Result<()> {
    accounts::valid_name(name)?;
    if name == accounts::DEFAULT {
        bail!("`default` is your ~/.claude login; log in there with `claude` and /login");
    }
    let raw = match token.as_deref() {
        Some("-") => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
            s
        }
        Some(t) => t.to_string(),
        None => {
            println!("Sign in to the account for {name:?} in the browser; claude prints a token.");
            println!("Already signed in to another account there? Switch accounts in the browser first.\n");
            let st = Command::new(&cfg.claude_bin)
                .arg("setup-token")
                .env_remove("CLAUDE_CONFIG_DIR")
                .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
                .status()
                .context("running claude setup-token")?;
            if !st.success() {
                bail!("claude setup-token did not finish");
            }
            read_hidden("\nPaste the token (hidden): ")?
        }
    };
    let tok = accounts::clean_token(&raw)?;
    match accounts::check_token(&tok) {
        accounts::TokenCheck::Valid => {}
        accounts::TokenCheck::Rejected => bail!("Claude rejected that token; nothing was saved"),
        accounts::TokenCheck::Unknown(e) => eprintln!("warning: couldn't check the token ({e}); saving it anyway"),
    }
    let renewed = accounts::find(name).is_ok();
    accounts::save(name, &tok)?;
    claude_deck::stats::log_event("account", Some(name), if renewed { "token renewed" } else { "account added" }, serde_json::json!({}));
    println!("{name} is ready: choose it when you start claude. Usage shows once a session runs on it.");
    Ok(())
}

fn add_dir(name: &str, dir: &Path) -> Result<()> {
    let renewed = accounts::find(name).is_ok();
    let acc = accounts::save_dir(name, dir)?;
    claude_deck::stats::log_event("account", Some(name), if renewed { "account changed" } else { "account added" }, serde_json::json!({}));
    let who = acc.email().map(|e| format!(" ({e})")).unwrap_or_else(|| ", not logged in yet: start claude on it and /login".into());
    println!("{name} runs on {}{who}.", acc.dir().display());
    Ok(())
}

fn read_hidden(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if tty {
        let _ = Command::new("stty").arg("-echo").status();
    }
    let mut line = String::new();
    let r = std::io::stdin().lock().read_line(&mut line);
    if tty {
        let _ = Command::new("stty").arg("echo").status();
        eprintln!();
    }
    r?;
    Ok(line)
}

/// Save the session's rate limits for its account, then hand the same input to the user's own
/// status line and pass its output through.
fn statusline() -> Result<()> {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    if let Ok(acc) = std::env::var("CLAUDE_DECK_ACCOUNT") {
        let _ = usage::record_live(&acc, &v);
        if let Some(s) = v.get("session_id").and_then(|s| s.as_str()) {
            let _ = usage::record_session_account(s, &acc);
        }
    }
    let cwd = v
        .pointer("/workspace/project_dir")
        .or_else(|| v.get("cwd"))
        .and_then(|c| c.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(claude_deck::config::default_claude_dir);
    let Some(cmd) = accounts::user_statusline(&cwd, &config_dir).and_then(|s| s.get("command")?.as_str().map(String::from)) else {
        return Ok(());
    };
    let mut child = Command::new("sh")
        .args(["-c", &cmd])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .context("running your status line")?;
    if let Some(mut sin) = child.stdin.take() {
        let _ = sin.write_all(input.as_bytes());
    }
    let st = child.wait()?;
    std::process::exit(st.code().unwrap_or(1));
}

