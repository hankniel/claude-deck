//! TUI tab = one `claude` process in a PTY, rendered through a vt100 parser.

use claude_deck::accounts::Account;
use claude_deck::config::Config;
use claude_deck::pty::{self, Start};
use anyhow::Result;
use portable_pty::{Child, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

const SCROLLBACK: usize = 5000;

pub struct Tab {
    pub account: String,
    pub account_dir: PathBuf,
    pub cwd: PathBuf,
    pub session_id: Option<String>,
    pub started: SystemTime,
    pub parser: Arc<Mutex<vt100::Parser>>,
    pub last_output: Arc<Mutex<Instant>>,
    pub last_input: Instant,
    pub exited: Arc<AtomicBool>,
    pub last_switch: Option<Instant>,
    pub warned: bool,
    pub scroll: usize,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Tab {
    pub fn spawn(
        acc: &Account,
        cfg: &Config,
        cwd: &Path,
        size: (u16, u16),
        start: Start,
        dirty: Arc<AtomicBool>,
    ) -> Result<Tab> {
        let (rows, cols) = (size.0.max(2), size.1.max(10));
        let sp = pty::spawn_claude(acc, cfg, cwd, rows, cols, start)?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let exited = Arc::new(AtomicBool::new(false));
        let mut reader = sp.reader;
        {
            let (parser, last_output, exited) = (parser.clone(), last_output.clone(), exited.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 16384];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            parser.lock().unwrap().process(&buf[..n]);
                            *last_output.lock().unwrap() = Instant::now();
                            dirty.store(true, Ordering::Relaxed);
                        }
                    }
                }
                exited.store(true, Ordering::Relaxed);
                dirty.store(true, Ordering::Relaxed);
            });
        }
        Ok(Tab {
            account: acc.name.clone(),
            account_dir: acc.dir(),
            cwd: cwd.to_path_buf(),
            session_id: sp.session_id,
            started: sp.started,
            parser,
            last_output,
            last_input: Instant::now(),
            exited,
            last_switch: None,
            warned: false,
            scroll: 0,
            writer: sp.writer,
            master: sp.master,
            child: sp.child,
        })
    }

    pub fn is_exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.is_exited() {
            return;
        }
        self.last_input = Instant::now();
        self.scroll = 0;
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self.parser.lock().unwrap().screen().bracketed_paste();
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        self.write(&bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        let mut p = self.parser.lock().unwrap();
        if p.screen().size() == (rows, cols) {
            return;
        }
        p.set_size(rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }

    pub fn idle_for(&self) -> std::time::Duration {
        let out = self.last_output.lock().unwrap().elapsed();
        out.min(self.last_input.elapsed())
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn current_session(&self, claimed: &[String]) -> Option<String> {
        pty::current_session(&self.account_dir, &self.cwd, self.started, self.session_id.clone(), claimed)
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        if !self.is_exited() {
            self.kill();
        }
    }
}
