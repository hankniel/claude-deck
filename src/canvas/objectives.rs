//! Objectives are plain Markdown so agents edit them with their normal file tools:
//!
//! ```text
//! # Title
//! status: active|paused|done
//! - [ ] todo   - [~] doing   - [x] done   - [!] blocked
//!   > a note on the task above
//! ```
//!
//! The board edits them by line number, checked against the task text it last saw.

use super::store::{self, objectives_dir};
use anyhow::{bail, Result};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Task {
    pub line: usize,
    pub state: String,
    pub text: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Objective {
    pub slug: String,
    pub title: String,
    pub status: String,
    pub tasks: Vec<Task>,
    pub mtime: i64,
}

impl Objective {
    pub fn done(&self) -> usize {
        self.tasks.iter().filter(|t| t.state == "done").count()
    }
}

pub fn valid_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub fn file(slug: &str) -> Result<PathBuf> {
    if !valid_slug(slug) {
        bail!("invalid objective slug (a-z, 0-9, -): {slug}");
    }
    Ok(objectives_dir().join(format!("{slug}.md")))
}

fn mark_state(c: char) -> Option<&'static str> {
    Some(match c {
        ' ' => "todo",
        '~' => "doing",
        'x' | 'X' => "done",
        '!' => "blocked",
        _ => return None,
    })
}

fn state_mark(s: &str) -> Option<char> {
    Some(match s {
        "todo" => ' ',
        "doing" => '~',
        "done" => 'x',
        "blocked" => '!',
        _ => return None,
    })
}

/// `- [x] text` → (mark, text)
fn task_line(line: &str) -> Option<(char, &str)> {
    let rest = line.strip_prefix("- [")?;
    let mut ch = rest.chars();
    let c = ch.next()?;
    let rest = ch.as_str().strip_prefix("] ")?;
    mark_state(c)?;
    Some((c, rest))
}

fn note_line(line: &str) -> Option<&str> {
    if !line.starts_with(char::is_whitespace) {
        return None;
    }
    let t = line.trim_start().strip_prefix('>')?;
    Some(t.strip_prefix(' ').unwrap_or(t))
}

pub fn parse(slug: &str, text: &str) -> Objective {
    let mut o = Objective { slug: slug.into(), title: String::new(), status: "active".into(), tasks: vec![], mtime: 0 };
    for (i, line) in text.lines().enumerate() {
        if o.title.is_empty() {
            if let Some(t) = line.strip_prefix("# ") {
                o.title = t.trim().to_string();
                continue;
            }
        }
        if let Some(s) = line.strip_prefix("status:") {
            o.status = s.trim().split_whitespace().next().unwrap_or("active").to_string();
        } else if let Some((c, t)) = task_line(line) {
            o.tasks.push(Task { line: i, state: mark_state(c).unwrap().into(), text: t.into(), notes: vec![] });
        } else if let Some(n) = note_line(line) {
            if let Some(last) = o.tasks.last_mut() {
                if last.line + 1 + last.notes.len() == i {
                    last.notes.push(n.to_string());
                }
            }
        }
    }
    if o.title.is_empty() {
        o.title = slug.to_string();
    }
    o
}

pub fn read(slug: &str) -> Result<Objective> {
    let f = file(slug)?;
    let mut o = parse(slug, &std::fs::read_to_string(&f)?);
    o.mtime = std::fs::metadata(&f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(o)
}

/// Every objective, most recently edited first.
pub fn list() -> Vec<Objective> {
    let mut out: Vec<Objective> = std::fs::read_dir(objectives_dir())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    let slug = n.strip_suffix(".md")?;
                    read(slug).ok()
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

pub fn create(slug: &str, title: &str) -> Result<PathBuf> {
    let f = file(slug)?;
    std::fs::create_dir_all(objectives_dir())?;
    if !f.exists() {
        let t = if title.is_empty() { slug } else { title };
        std::fs::write(&f, format!("# {t}\nstatus: active\n\n"))?;
    }
    Ok(f)
}

/// One board edit.
#[derive(Clone, Debug)]
pub enum Op {
    State { line: usize, text: String, state: String },
    Note { line: usize, text: String, note: String },
    Add { text: String },
    Status { status: String },
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Apply a board edit; returns a one-line description for the agents working on it.
pub fn apply(slug: &str, op: &Op) -> Result<String> {
    let f = file(slug)?;
    let mut lines: Vec<String> = std::fs::read_to_string(&f)?.split('\n').map(String::from).collect();
    let check = |lines: &[String], line: usize, text: &str| -> Result<()> {
        match lines.get(line).and_then(|l| task_line(l)) {
            Some((_, t)) if t == text => Ok(()),
            _ => bail!("objective changed since you loaded it"),
        }
    };
    let said = match op {
        Op::State { line, text, state } => {
            let m = state_mark(state).ok_or_else(|| anyhow::anyhow!("unknown state: {state}"))?;
            check(&lines, *line, text)?;
            lines[*line] = format!("- [{m}] {text}");
            format!("\"{text}\" → {state}")
        }
        Op::Note { line, text, note } => {
            check(&lines, *line, text)?;
            let note = clean(note);
            if note.is_empty() {
                bail!("empty note");
            }
            let mut at = line + 1;
            while lines.get(at).is_some_and(|l| note_line(l).is_some()) {
                at += 1;
            }
            lines.insert(at, format!("  > {note}"));
            format!("note on \"{text}\": {note}")
        }
        Op::Add { text } => {
            let text = clean(text);
            if text.is_empty() {
                bail!("empty task");
            }
            let mut end = lines.len();
            while end > 0 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            lines.insert(end, format!("- [ ] {text}"));
            format!("added task \"{text}\"")
        }
        Op::Status { status } => {
            if !["active", "paused", "done"].contains(&status.as_str()) {
                bail!("unknown status: {status}");
            }
            match lines.iter().position(|l| l.starts_with("status:")) {
                Some(i) => lines[i] = format!("status: {status}"),
                None => lines.insert(1.min(lines.len()), format!("status: {status}")),
            }
            format!("objective → {status}")
        }
    };
    std::fs::write(&f, lines.join("\n"))?;
    tell_agents(slug, &said);
    Ok(said)
}

/// A board edit reaches every live session on that objective with its next prompt.
pub fn tell_agents(slug: &str, said: &str) {
    for s in store::list_sessions() {
        if s.meta.objective == slug && s.state.state != "ended" {
            let _ = store::append_event(&s.dir, &format!("board:{slug}"), vec![said.to_string()], "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tasks_and_notes() {
        let o = parse("x", "# Ship it\nstatus: paused\n\n- [x] a\n  > why\n- [~] b\n- [!] c\n");
        assert_eq!(o.title, "Ship it");
        assert_eq!(o.status, "paused");
        assert_eq!(o.tasks.len(), 3);
        assert_eq!(o.tasks[0].notes, vec!["why"]);
        assert_eq!(o.tasks[1].state, "doing");
        assert_eq!(o.tasks[2].line, 6);
        assert_eq!(o.done(), 1);
    }
}
