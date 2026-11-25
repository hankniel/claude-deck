//! Start `claude` in a PTY under an account, and find which conversation it is on.

use crate::accounts::Account;
use crate::config::Config;
use crate::usage::Usage;
use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::SystemTime;

pub enum Start {
    /// Fresh conversation with a generated id.
    New,
    /// Continue a conversation by id (account switch / restart).
    Resume(String),
    /// Continue the newest conversation in the directory.
    Continue,
    /// User-supplied claude args; the conversation id is unknown.
    Raw(Vec<String>),
}

pub struct Spawned {
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub session_id: Option<String>,
    pub started: SystemTime,
}

pub fn spawn_claude(
    acc: &Account,
    cfg: &Config,
    cwd: &Path,
    rows: u16,
    cols: u16,
    start: Start,
) -> Result<Spawned> {
    let (rows, cols) = (rows.max(2), cols.max(10));
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .context("opening a pty")?;

    let mut session_id = None;
    let args: Vec<String> = match start {
        Start::New => {
            let id = uuid::Uuid::new_v4().to_string();
            session_id = Some(id.clone());
            vec!["--session-id".into(), id]
        }
        Start::Resume(id) => {
            session_id = Some(id.clone());
            vec!["--resume".into(), id]
        }
        Start::Continue => vec!["--continue".into()],
        Start::Raw(a) => a,
    };

    let mut cmd = CommandBuilder::new(&cfg.claude_bin);
    cmd.args(acc.claude_args(cwd, &args));
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "en_US.UTF-8");
    }
    cmd.env_remove("TMUX");
    for (k, v) in acc.env() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        }
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("starting {}", cfg.claude_bin))?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    Ok(Spawned { master: pair.master, child, reader, writer, session_id, started: SystemTime::now() })
}

/// Claude Code's per-project transcript dir name: every non-alphanumeric char becomes '-'.
pub fn project_slug(cwd: &Path) -> String {
    let p = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    p.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Copy conversation `session` of `cwd` from one Claude Code config dir to another, so
/// `--resume` finds it after moving to an account on a different config dir.
pub fn carry_session(from: &Path, to: &Path, cwd: &Path, session: &str) {
    fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
        if src.is_dir() {
            std::fs::create_dir_all(dst)?;
            for e in std::fs::read_dir(src)?.flatten() {
                copy_tree(&e.path(), &dst.join(e.file_name()))?;
            }
        } else if !dst.exists() {
            std::fs::copy(src, dst)?;
        }
        Ok(())
    }
    if from == to {
        return;
    }
    let slug = project_slug(cwd);
    let (src, dst) = (from.join("projects").join(&slug), to.join("projects").join(&slug));
    let _ = std::fs::create_dir_all(&dst);
    let _ = copy_tree(&src.join(format!("{session}.jsonl")), &dst.join(format!("{session}.jsonl")));
    if src.join(session).is_dir() {
        let _ = copy_tree(&src.join(session), &dst.join(session));
    }
}

/// The conversation a session is most likely on: the newest transcript for `cwd`
/// written since it started (covers /clear), not claimed by another session.
pub fn current_session(
    account_dir: &Path,
    cwd: &Path,
    started: SystemTime,
    fallback: Option<String>,
    claimed: &[String],
) -> Option<String> {
    let dir = account_dir.join("projects").join(project_slug(cwd));
    let newest = std::fs::read_dir(&dir).ok().and_then(|rd| {
        rd.flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension()? != "jsonl" {
                    return None;
                }
                let stem = p.file_stem()?.to_string_lossy().to_string();
                let mtime = e.metadata().ok()?.modified().ok()?;
                (mtime >= started && !claimed.contains(&stem)).then_some((mtime, stem))
            })
            .max()
    });
    newest.map(|(_, s)| s).or(fallback)
}

/// Lowest-pressure usable account other than `exclude`.
pub fn best_account(
    accounts: &[Account],
    usage: &HashMap<String, Usage>,
    cfg: &Config,
    exclude: Option<&str>,
) -> Option<Account> {
    accounts
        .iter()
        .filter(|a| Some(a.name.as_str()) != exclude && !cfg.disabled.contains(&a.name))
        .filter_map(|a| {
            let p = usage.get(&a.name)?.pressure(cfg)?;
            (p < 1.0).then_some((p, a))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, a)| a.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn slug() {
        assert_eq!(
            super::project_slug(std::path::Path::new("/nonexistent/a.b/c_d")),
            "-nonexistent-a-b-c-d"
        );
    }
}

#[cfg(test)]
mod carry_tests {
    use super::*;

    #[test]
    fn carries_transcript_to_other_config_dir() {
        let root = std::env::temp_dir().join(format!("cc-deck-carry-{}", std::process::id()));
        let (from, to, cwd) = (root.join("a"), root.join("b"), root.join("proj"));
        std::fs::create_dir_all(&cwd).unwrap();
        let src = from.join("projects").join(project_slug(&cwd));
        std::fs::create_dir_all(src.join("s1/subagents")).unwrap();
        std::fs::write(src.join("s1.jsonl"), "{}\n").unwrap();
        std::fs::write(src.join("s1/subagents/x.jsonl"), "{}\n").unwrap();
        carry_session(&from, &to, &cwd, "s1");
        let dst = to.join("projects").join(project_slug(&cwd));
        assert!(dst.join("s1.jsonl").is_file());
        assert!(dst.join("s1/subagents/x.jsonl").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
