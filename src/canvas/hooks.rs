//! Agent hooks (Claude Code / Codex): `claude-deck canvas hook <event>` reads the hook JSON on
//! stdin and prints the hook's JSON answer, if any.

use super::objectives;
use super::store::{self, now_ms, read_meta, write_meta, State};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

fn context(event: &str, text: &str) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": event, "additionalContext": text } }).to_string()
}

fn objective_line(dir: &Path) -> String {
    let slug = read_meta(dir).objective;
    if slug.is_empty() {
        return String::new();
    }
    match (objectives::read(&slug), objectives::file(&slug)) {
        (Ok(o), Ok(f)) => format!("\nobjective: {} — {}", o.title, f.display()),
        _ => String::new(),
    }
}

/// SessionStart: register the session folder and tell the agent where screens go.
pub fn session_start(input: &Value) -> Result<Option<String>> {
    let id = store::json_str(input, "session_id");
    let cwd = store::json_str(input, "cwd");
    let dir = store::ensure_session(&id, &cwd)?;
    let agent = std::env::var("CANVAS_AGENT").unwrap_or_else(|_| "Claude".into());
    let pane = std::env::var("CC_DECK_TAB").unwrap_or_default();
    let account = std::env::var("CLAUDE_DECK_ACCOUNT").unwrap_or_default();
    write_meta(&dir, |m| {
        m.agent = agent;
        if !pane.is_empty() {
            m.pane = pane;
        }
        if !account.is_empty() {
            m.account = account;
        }
        if m.cwd.is_empty() {
            m.cwd = cwd;
        }
    })?;
    let where_ = if std::env::var_os("CC_DECK_TAB").is_some() { "shown in CC Deck's Canvas" } else { "open CC Deck → Canvas to see them" };
    Ok(Some(context(
        "SessionStart",
        &format!(
            "canvas ready ({where_}) — session dir {}; write screens to {}/content/{}",
            dir.display(),
            dir.display(),
            objective_line(&dir)
        ),
    )))
}

fn is_visual(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    let visual = ["architecture", "kiến trúc", "diagram", "sơ đồ", "lineage", "data flow", "dataflow", "luồng dữ liệu", "brainstorm", "erd", "schema"];
    let meta = ["skill", "hook", "config", "plugin", "prompt", "readme", "commit", "rename", "đổi tên", "sửa", "fix"];
    let word = |w: &str| {
        p.match_indices(w).any(|(i, _)| {
            let before = p[..i].chars().last();
            let after = p[i + w.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
        })
    };
    visual.iter().any(|w| word(w)) && !meta.iter().any(|w| word(w))
}

/// UserPromptSubmit: deliver canvas answers given since the last prompt, record the title.
pub fn prompt_submit(input: &Value) -> Result<Option<String>> {
    let id = store::json_str(input, "session_id");
    let dir = store::session_dir(&id)?;
    if !dir.exists() {
        return Ok(None);
    }
    let prompt = store::json_str(input, "prompt");
    if read_meta(&dir).title.is_empty() && !prompt.is_empty() {
        let t: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
        write_meta(&dir, |m| m.title = t)?;
    }
    let nudge = if is_visual(&prompt) {
        format!(
            "canvas: if this answer has a shape, draw ONE screen in {}/content/ (.mmd flow, .lineage, .dbml — small, short labels), then 2–4 sentences of prose. Ask the user with your input tool, not a screen.",
            dir.display()
        )
    } else {
        String::new()
    };

    let events = dir.join("events");
    let cursor_file = dir.join(".events-cursor");
    let size = std::fs::metadata(&events).map(|m| m.len()).unwrap_or(0);
    let cursor: u64 = std::fs::read_to_string(&cursor_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let mut lines = vec![];
    if size > cursor {
        let bytes = std::fs::read(&events)?;
        let tail = String::from_utf8_lossy(&bytes[cursor as usize..]).to_string();
        std::fs::write(&cursor_file, size.to_string())?;
        for l in tail.lines() {
            if let Ok(e) = serde_json::from_str::<store::Event>(l) {
                let choices = if e.choices.is_empty() { "(none)".to_string() } else { e.choices.join(", ") };
                let note = if e.note.is_empty() { String::new() } else { format!(" — note: {}", e.note) };
                lines.push(format!("{}: {choices}{note}", e.screen));
            }
        }
    }
    let mut text = String::new();
    if !lines.is_empty() {
        text = format!("canvas answers since last prompt:\n{}", lines.join("\n"));
    }
    if !nudge.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&nudge);
    }
    Ok((!text.is_empty()).then(|| context("UserPromptSubmit", &text)))
}

/// State hooks: what the session is doing, for the board and the session list.
pub fn state(input: &Value) -> Result<Option<String>> {
    let id = store::json_str(input, "session_id");
    let dir = store::session_dir(&id)?;
    if !dir.exists() {
        return Ok(None);
    }
    let (state, reason) = match store::json_str(input, "hook_event_name").as_str() {
        "UserPromptSubmit" | "PostToolUse" => ("running", String::new()),
        "PreToolUse" => ("waiting", "question".to_string()),
        "Notification" => {
            if store::json_str(input, "notification_type") == "idle_prompt" {
                return Ok(None);
            }
            let msg = store::json_str(input, "message");
            ("waiting", if msg.is_empty() { "needs you".into() } else { msg.chars().take(200).collect() })
        }
        "Stop" => ("idle", String::new()),
        "SessionEnd" => ("ended", String::new()),
        _ => return Ok(None),
    };
    let prev = store::read_state(&dir);
    if prev.state == state && prev.reason == reason {
        return Ok(None);
    }
    let st = State { state: state.into(), reason: reason.clone(), at: now_ms() };
    store::write_atomic(&dir.join("state.json"), serde_json::to_string(&st)?.as_bytes())?;

    // Inside CC Deck the app notifies; elsewhere the hook does.
    if std::env::var_os("CC_DECK_TAB").is_none() && matches!(state, "waiting" | "idle") {
        let meta = read_meta(&dir);
        let label = if !meta.objective.is_empty() {
            objectives::read(&meta.objective).map(|o| o.title).unwrap_or(meta.title.clone())
        } else if !meta.title.is_empty() {
            meta.title.clone()
        } else {
            meta.project.clone()
        };
        let title = format!("{} · {}", if meta.agent.is_empty() { "Claude" } else { &meta.agent }, meta.project);
        let body = if state == "waiting" { format!("{label} — {reason}") } else { format!("{label} — done, your turn") };
        notify(&title, &body);
    }
    Ok(None)
}

fn notify(title: &str, body: &str) {
    let mut c = if cfg!(target_os = "macos") {
        let q = |s: &str| format!("{s:?}");
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", &format!("display notification {} with title {}", q(body), q(title))]);
        c
    } else {
        let mut c = std::process::Command::new("notify-send");
        c.args([title, body, "--icon=dialog-information"]);
        c
    };
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let _ = c.spawn();
}

/// Block until the user answers `screen` (given after this version of it was written).
pub fn wait(dir: &Path, screen: &str, timeout: Duration) -> Result<Value> {
    let file = store::screen_path(dir, screen).ok_or_else(|| anyhow::anyhow!("no such screen: {screen}"))?;
    let since = std::fs::metadata(&file)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(e) = store::read_events(dir).into_iter().find(|e| e.screen == screen && e.ts >= since) {
            let size = std::fs::metadata(dir.join("events")).map(|m| m.len()).unwrap_or(0);
            std::fs::write(dir.join(".events-cursor"), size.to_string())?;
            return Ok(serde_json::to_value(e)?);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Ok(json!({ "timeout": true, "screen": screen }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_prompts() {
        assert!(is_visual("vẽ sơ đồ kiến trúc của pipeline"));
        assert!(is_visual("show the lineage of fct_orders"));
        assert!(!is_visual("fix the hook config"));
        assert!(!is_visual("discern this"));
    }
}
