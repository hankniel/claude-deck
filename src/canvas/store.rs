//! On-disk canvas data, shared by the app, the CLI and the agent hooks.
//!
//! ```text
//! <canvas home>/
//!   sessions/<session id>/
//!     meta.json        id, cwd, project, title, started, agent, pane (CC Deck tab), account, objective
//!     state.json       running | waiting | idle | ended, reason, at
//!     events           answers from the canvas, JSON lines, read by the prompt hook
//!     .events-cursor   byte offset the prompt hook already delivered
//!     content/         screens the agent writes (.mmd .lineage .dbml .rv .rows .doc .open …)
//!   objectives/<slug>.md
//! ```

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `$CANVAS_HOME`, else `<claude-deck data dir>/canvas` (one home for every account).
pub fn home() -> PathBuf {
    match std::env::var_os("CANVAS_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => p,
        None => crate::config::data_dir().join("canvas"),
    }
}

pub fn sessions_dir() -> PathBuf {
    home().join("sessions")
}

pub fn objectives_dir() -> PathBuf {
    home().join("objectives")
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn session_dir(id: &str) -> Result<PathBuf> {
    if !valid_session_id(id) {
        bail!("invalid session id: {id}");
    }
    Ok(sessions_dir().join(id))
}

/// What kind of screen a file is, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Flow,
    Lineage,
    Erd,
    Review,
    Rows,
    Doc,
    Open,
    Plan,
    Html,
    Markdown,
}

impl Kind {
    pub fn of(name: &str) -> Option<Kind> {
        let ext = Path::new(name).extension()?.to_str()?;
        Some(match ext {
            "mmd" => Kind::Flow,
            "lineage" => Kind::Lineage,
            "dbml" => Kind::Erd,
            "rv" => Kind::Review,
            "rows" => Kind::Rows,
            "doc" => Kind::Doc,
            "open" => Kind::Open,
            "plan" => Kind::Plan,
            "html" => Kind::Html,
            "md" => Kind::Markdown,
            _ => return None,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Flow => "network-workgroup-symbolic",
            Kind::Lineage => "view-continuous-symbolic",
            Kind::Erd => "x-office-spreadsheet-symbolic",
            Kind::Review => "emblem-ok-symbolic",
            Kind::Rows => "view-list-ordered-symbolic",
            Kind::Doc | Kind::Markdown => "x-office-document-symbolic",
            Kind::Open => "folder-open-symbolic",
            Kind::Plan => "view-list-bullet-symbolic",
            Kind::Html => "web-browser-symbolic",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Meta {
    pub id: String,
    pub cwd: String,
    pub project: String,
    pub title: String,
    pub started: i64,
    pub agent: String,
    /// CC Deck tab the session runs in.
    pub pane: String,
    /// claude-deck account it runs on.
    pub account: String,
    pub objective: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct State {
    pub state: String,
    pub reason: String,
    pub at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub name: String,
    pub kind: Kind,
    pub mtime: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Session {
    pub dir: PathBuf,
    pub meta: Meta,
    pub state: State,
    pub items: Vec<Item>,
    /// Latest of the newest screen, the state change and the start.
    pub updated: i64,
}

impl Session {
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    pub fn label(&self) -> String {
        if !self.meta.title.is_empty() {
            self.meta.title.clone()
        } else if !self.meta.project.is_empty() {
            self.meta.project.clone()
        } else {
            self.meta.id.chars().take(8).collect()
        }
    }

    pub fn needs_you(&self) -> bool {
        self.state.state == "waiting"
    }
}

fn mtime_ms(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn read_meta(dir: &Path) -> Meta {
    std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_meta(dir: &Path, f: impl FnOnce(&mut Meta)) -> Result<()> {
    let mut m = read_meta(dir);
    f(&mut m);
    write_atomic(&dir.join("meta.json"), serde_json::to_string(&m)?.as_bytes())
}

pub fn read_state(dir: &Path) -> State {
    std::fs::read_to_string(dir.join("state.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Create the session folder (idempotent).
pub fn ensure_session(id: &str, cwd: &str) -> Result<PathBuf> {
    let dir = session_dir(id)?;
    std::fs::create_dir_all(dir.join("content")).with_context(|| format!("creating {}", dir.display()))?;
    if !dir.join("meta.json").exists() {
        let project = Path::new(cwd).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        write_meta(&dir, |m| {
            m.id = id.to_string();
            m.cwd = cwd.to_string();
            m.project = project;
            m.started = now_ms();
        })?;
    }
    Ok(dir)
}

/// Screens in a session, newest first.
pub fn list_items(dir: &Path) -> Vec<Item> {
    let mut out: Vec<Item> = std::fs::read_dir(dir.join("content"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') || !e.file_type().ok()?.is_file() {
                        return None;
                    }
                    let kind = Kind::of(&name)?;
                    Some(Item { mtime: mtime_ms(&e.path()), name, kind })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

pub fn load_session(dir: &Path) -> Option<Session> {
    if !dir.is_dir() {
        return None;
    }
    let mut meta = read_meta(dir);
    if meta.id.is_empty() {
        meta.id = dir.file_name()?.to_string_lossy().to_string();
    }
    let state = read_state(dir);
    let items = list_items(dir);
    let updated = items.first().map(|i| i.mtime).unwrap_or(0).max(state.at).max(meta.started);
    Some(Session { dir: dir.to_path_buf(), meta, state, items, updated })
}

/// Every session, most recently active first.
pub fn list_sessions() -> Vec<Session> {
    let mut out: Vec<Session> = std::fs::read_dir(sessions_dir())
        .map(|rd| rd.flatten().filter_map(|e| load_session(&e.path())).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

/// A screen file inside a session's content folder: plain file, no traversal.
pub fn screen_path(dir: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.starts_with('.') || Path::new(name).file_name()?.to_str()? != name {
        return None;
    }
    let p = dir.join("content").join(name);
    let md = std::fs::symlink_metadata(&p).ok()?;
    (md.is_file() && !md.file_type().is_symlink()).then_some(p)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Event {
    pub screen: String,
    pub choices: Vec<String>,
    #[serde(default)]
    pub note: String,
    pub ts: i64,
}

/// An answer from the canvas, delivered to the agent with its next prompt (or to `canvas wait`).
pub fn append_event(dir: &Path, screen: &str, choices: Vec<String>, note: &str) -> Result<()> {
    let e = Event { screen: screen.into(), choices, note: note.chars().take(4000).collect(), ts: now_ms() };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("events"))?;
    writeln!(f, "{}", serde_json::to_string(&e)?)?;
    Ok(())
}

pub fn read_events(dir: &Path) -> Vec<Event> {
    std::fs::read_to_string(dir.join("events"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Remove a session folder (screens, events, state).
pub fn delete_session(dir: &Path) -> Result<()> {
    if dir.parent() != Some(sessions_dir().as_path()) {
        bail!("not a canvas session: {}", dir.display());
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

pub fn delete_screen(dir: &Path, name: &str) -> Result<()> {
    let p = screen_path(dir, name).with_context(|| format!("no screen {name}"))?;
    std::fs::remove_file(p)?;
    Ok(())
}

/// Sidecar file next to a screen (layout positions etc.), hidden from the screen list.
pub fn sidecar(dir: &Path, name: &str, suffix: &str) -> PathBuf {
    dir.join("content").join(format!(".{name}.{suffix}"))
}

/// Cheap fingerprint of everything the canvas UI shows, to poll for changes.
pub fn fingerprint() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let walk = |dir: &Path, h: &mut std::collections::hash_map::DefaultHasher| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut v: Vec<(String, i64)> = rd
                .flatten()
                .map(|e| (e.file_name().to_string_lossy().to_string(), mtime_ms(&e.path())))
                .filter(|(n, _)| !n.starts_with('.'))
                .collect();
            v.sort();
            v.hash(h);
        }
    };
    walk(&objectives_dir(), &mut h);
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            d.hash(&mut h);
            mtime_ms(&d.join("meta.json")).hash(&mut h);
            mtime_ms(&d.join("state.json")).hash(&mut h);
            walk(&d.join("content"), &mut h);
        }
    }
    h.finish()
}

pub fn json_str(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}
