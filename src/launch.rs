//! `claude-deck launch`: what the shell's `claude` function runs.
//!
//! It asks for the account (CC Deck's popup over CC_DECK_SOCKET, or a prompt in a plain
//! terminal), runs `claude` as a child on the same terminal, and stays alive so the app can
//! move the conversation to another account: on a `switch` message it stops claude and
//! starts it again under the new account with `--resume <session>`.
//!
//! Socket protocol (JSON lines):
//!   → {"type":"hello","tab":..,"cwd":..,"args":[..],"pick":bool}
//!   ← {"account":"name"} | {"cancel":true}            (only when pick)
//!   → {"type":"started","account":..,"session_id":..,"cwd":..}
//!   ← {"type":"switch","account":..,"claimed":[session ids of other tabs]}
//!   → {"type":"exited","code":..}

use anyhow::{bail, Context, Result};
use claude_deck::accounts::{self, Account};
use claude_deck::config::Config;
use claude_deck::pty;
use claude_deck::stats;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

extern "C" fn ignore_signal(_: libc::c_int) {}

const SESSION_FLAGS: &[&str] = &["-c", "--continue", "-r", "--resume", "--session-id", "-p", "--print", "--fork-session"];
const FLAGS_WITH_VALUE: &[&str] = &["-r", "--resume", "--session-id"];

struct Link {
    out: UnixStream,
    rx: mpsc::Receiver<Value>,
}

impl Link {
    fn connect() -> Option<(Link, UnixStream)> {
        let path = std::env::var("CC_DECK_SOCKET").ok()?;
        let s = UnixStream::connect(path).ok()?;
        let reader = s.try_clone().ok()?;
        let (tx, rx) = mpsc::channel();
        let first = reader.try_clone().ok()?;
        std::thread::spawn(move || {
            for line in BufReader::new(reader).lines().map_while(|l| l.ok()) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        Some((Link { out: s, rx }, first))
    }

    fn send(&mut self, v: Value) {
        let _ = writeln!(self.out, "{v}");
    }
}

pub fn run(cfg: &Config, account: Option<String>, args: Vec<String>) -> Result<i32> {
    // Ctrl-C belongs to claude. A no-op handler (not SIG_IGN) resets to default in the child.
    unsafe {
        libc::signal(libc::SIGINT, ignore_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGQUIT, ignore_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
    let cwd = std::env::current_dir()?;
    let mut link = Link::connect().map(|(l, _)| l);
    let tab = std::env::var("CC_DECK_TAB").unwrap_or_default();

    let name = match (&account, link.as_mut()) {
        (Some(a), Some(l)) => {
            l.send(json!({"type":"hello","tab":tab,"cwd":cwd,"args":args,"pick":false}));
            a.clone()
        }
        (Some(a), None) => a.clone(),
        (None, Some(l)) => {
            l.send(json!({"type":"hello","tab":tab,"cwd":cwd,"args":args,"pick":true}));
            match l.rx.recv() {
                Ok(v) if v.get("account").and_then(|a| a.as_str()).is_some() => {
                    v["account"].as_str().unwrap().to_string()
                }
                Ok(_) => return Ok(130),
                Err(_) => prompt_terminal(cfg, &cwd)?,
            }
        }
        (None, None) => prompt_terminal(cfg, &cwd)?,
    };

    let mut acc = accounts::find(&name)?;
    let user_args = args.clone();
    let mut start_args = start_args_for_new(&user_args);
    loop {
        accounts::link_shared(&acc, cfg)?;
        let (session_id, args) = start_args;
        let started = SystemTime::now();
        let mut child = spawn(cfg, &acc, &args)?;
        if let Some(l) = link.as_mut() {
            l.send(json!({"type":"started","account":acc.name,"session_id":session_id,"cwd":cwd}));
        }
        stats::log_event("start", Some(&acc.name), &format!("claude in {}", short(&cwd)),
            json!({"session_id": session_id, "cwd": cwd}));

        let outcome = wait(&mut child, link.as_mut());
        match outcome {
            Outcome::Exited(code) => {
                if let Some(l) = link.as_mut() {
                    l.send(json!({"type":"exited","code":code}));
                }
                return Ok(code);
            }
            Outcome::Switch { to, claimed } => {
                let target = match accounts::find(&to) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("claude-deck: {e:#}");
                        start_args = (session_id.clone(), resume_args(session_id.as_deref(), &user_args));
                        continue;
                    }
                };
                let resume = pty::current_session(&acc.dir(), &cwd, started, session_id.clone(), &claimed);
                terminate(&mut child);
                reset_terminal();
                eprintln!("\x1b[2m↻ moving this conversation from {} to {}…\x1b[0m", acc.name, target.name);
                stats::log_event("switch", Some(&target.name), &format!("{} → {}", acc.name, target.name),
                    json!({"from": acc.name, "session_id": resume, "cwd": cwd}));
                start_args = (resume.clone(), resume_args(resume.as_deref(), &user_args));
                acc = target;
            }
        }
    }
}

enum Outcome {
    Exited(i32),
    Switch { to: String, claimed: Vec<String> },
}

fn wait(child: &mut Child, mut link: Option<&mut Link>) -> Outcome {
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Outcome::Exited(status.code().unwrap_or(1));
        }
        let msg = match link.as_mut() {
            Some(l) => match l.rx.recv_timeout(Duration::from_millis(150)) {
                Ok(v) => Some(v),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    link = None;
                    None
                }
            },
            None => {
                std::thread::sleep(Duration::from_millis(200));
                None
            }
        };
        if let Some(v) = msg {
            if v.get("type").and_then(|t| t.as_str()) == Some("switch") {
                if let Some(to) = v.get("account").and_then(|a| a.as_str()) {
                    let claimed = v
                        .get("claimed")
                        .and_then(|c| c.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    return Outcome::Switch { to: to.to_string(), claimed };
                }
            }
        }
    }
}

/// New conversation: pin a session id unless the user already chose one.
fn start_args_for_new(user: &[String]) -> (Option<String>, Vec<String>) {
    if user.iter().any(|a| SESSION_FLAGS.contains(&a.as_str()) || a.starts_with("--resume=") || a.starts_with("--session-id=")) {
        return (None, user.to_vec());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut args = vec!["--session-id".to_string(), id.clone()];
    args.extend(user.iter().cloned());
    (Some(id), args)
}

/// Keep the user's other flags (model, permissions…) but replace how the session starts.
fn resume_args(session: Option<&str>, user: &[String]) -> Vec<String> {
    let mut out = match session {
        Some(id) => vec!["--resume".to_string(), id.to_string()],
        None => vec!["--continue".to_string()],
    };
    let mut skip = false;
    for a in user {
        if skip {
            skip = false;
            continue;
        }
        if FLAGS_WITH_VALUE.contains(&a.as_str()) {
            skip = true;
            continue;
        }
        if SESSION_FLAGS.contains(&a.as_str()) || a.starts_with("--resume=") || a.starts_with("--session-id=") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

fn spawn(cfg: &Config, acc: &Account, args: &[String]) -> Result<Child> {
    let mut c = Command::new(&cfg.claude_bin);
    c.args(args);
    acc.apply_env(&mut c);
    c.spawn().with_context(|| format!("starting {}", cfg.claude_bin))
}

fn terminate(child: &mut Child) {
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Undo terminal modes a killed claude may leave behind.
fn reset_terminal() {
    let _ = Command::new("stty").arg("sane").status();
    eprint!("\x1b[0m\x1b[?25h\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?1000l\x1b[?1006l\r\n");
}

fn short(p: &Path) -> String {
    let home = claude_deck::config::home();
    match p.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn prompt_terminal(cfg: &Config, cwd: &Path) -> Result<String> {
    let accs = accounts::list();
    if accs.len() == 1 {
        return Ok(accs[0].name.clone());
    }
    let mapped = cfg.account_for(cwd);
    let default = mapped
        .as_ref()
        .and_then(|m| accs.iter().position(|a| &a.name == m))
        .unwrap_or(0);
    eprintln!("Run claude on which account?");
    for (i, a) in accs.iter().enumerate() {
        let mark = if i == default { "›" } else { " " };
        eprintln!(" {mark} {}) {:<14} {}", i + 1, a.name, a.email().unwrap_or_default());
    }
    eprint!("account [{}]: ", default + 1);
    std::io::stderr().flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        bail!("cancelled");
    }
    let line = line.trim();
    if line.is_empty() {
        return Ok(accs[default].name.clone());
    }
    if let Ok(n) = line.parse::<usize>() {
        if (1..=accs.len()).contains(&n) {
            return Ok(accs[n - 1].name.clone());
        }
    }
    Ok(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn new_session_pins_id_unless_user_chose() {
        let (id, args) = start_args_for_new(&s(&["--model", "opus"]));
        assert!(id.is_some());
        assert_eq!(args[0], "--session-id");
        let (id, args) = start_args_for_new(&s(&["--resume", "abc"]));
        assert!(id.is_none());
        assert_eq!(args, s(&["--resume", "abc"]));
    }

    #[test]
    fn resume_keeps_other_flags() {
        let out = resume_args(Some("x"), &s(&["--resume", "old", "--model", "opus", "-c"]));
        assert_eq!(out, s(&["--resume", "x", "--model", "opus"]));
    }
}
