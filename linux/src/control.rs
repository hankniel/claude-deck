//! Unix socket that `claude-deck launch` (run by the `claude` shell function in our tabs)
//! connects to. JSON lines both ways; see claude-deck/src/launch.rs for the protocol.

use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct Conn {
    pub id: u64,
    stream: Mutex<UnixStream>,
}

impl Conn {
    pub fn send(&self, v: Value) {
        if let Ok(mut s) = self.stream.lock() {
            let _ = writeln!(s, "{v}");
            let _ = s.flush();
        }
    }
}

pub enum Msg {
    Message(Arc<Conn>, Value),
    Closed(Arc<Conn>),
}

pub struct Server {
    pub path: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(d) => d.join("cc-deck"),
        None => claude_deck::config::data_dir().join("run"),
    }
}

pub fn start(tx: async_channel::Sender<Msg>) -> Result<Server> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    let path = dir.join(format!("ctl-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    static NEXT: AtomicU64 = AtomicU64::new(1);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let Ok(reader) = stream.try_clone() else { continue };
            let conn = Arc::new(Conn { id: NEXT.fetch_add(1, Ordering::Relaxed), stream: Mutex::new(stream) });
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(reader).lines().map_while(|l| l.ok()) {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if tx.send_blocking(Msg::Message(conn.clone(), v)).is_err() {
                            return;
                        }
                    }
                }
                let _ = tx.send_blocking(Msg::Closed(conn));
            });
        }
    });
    Ok(Server { path })
}
