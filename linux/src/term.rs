//! One tab = one VTE terminal running the user's shell.

use crate::control::Conn;
use crate::shell;
use claude_deck::config::home;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use vte::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeRun {
    pub account: String,
    pub session_id: Option<String>,
    pub cwd: String,
}

pub struct Tab {
    pub id: String,
    pub term: vte::Terminal,
    pub page: gtk::ScrolledWindow,
    pub title: RefCell<String>,
    pub cwd: RefCell<String>,
    pub claude: RefCell<Option<ClaudeRun>>,
    pub conn: RefCell<Option<Arc<Conn>>>,
    pub pid: Cell<Option<glib::Pid>>,
    pub last_input: Cell<Instant>,
    pub last_output: Cell<Instant>,
    pub last_switch: Cell<Option<Instant>>,
    pub warned: Cell<bool>,
    pub exited: Cell<bool>,
}

fn rgba(h: u32) -> gdk::RGBA {
    let (r, g, b) = crate::widgets::hex(h);
    gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0)
}

/// Terminal colours from the theme; the background matches the group it sits in.
pub fn apply_theme(term: &vte::Terminal, dark: bool, in_panel: bool) {
    let p = crate::theme::palette(dark);
    let bg = if in_panel { p.chrome } else { p.editor };
    let palette: Vec<gdk::RGBA> = p.term.iter().map(|h| rgba(*h)).collect();
    let refs: Vec<&gdk::RGBA> = palette.iter().collect();
    term.set_colors(Some(&rgba(p.fg)), Some(&rgba(bg)), &refs);
    term.set_color_cursor(Some(&rgba(p.accent)));
    let (r, g, b) = crate::widgets::hex(p.accent);
    term.set_color_highlight(Some(&gdk::RGBA::new(r as f32, g as f32, b as f32, if dark { 0.35 } else { 0.24 })));
    term.set_color_highlight_foreground(None);
    term.set_font(Some(&crate::theme::mono_font()));
}

const URL_RE: &str = r#"(https?|file)://[^\s<>"'`]+[^\s<>"'`.,;:!?)\]]"#;
const PCRE2_MULTILINE: u32 = 0x0000_0400;

impl Tab {
    pub fn new(cwd: &str, socket: &Path, cli: Option<&Path>) -> Rc<Tab> {
        let id = uuid::Uuid::new_v4().to_string();
        let term = vte::Terminal::new();
        term.set_scrollback_lines(10_000);
        term.set_cell_height_scale(1.08);
        term.set_mouse_autohide(true);
        term.set_bold_is_bright(false);
        term.set_allow_hyperlink(true);
        term.set_hexpand(true);
        term.set_vexpand(true);
        if let Ok(re) = vte::Regex::for_match(URL_RE, PCRE2_MULTILINE) {
            let tag = term.match_add_regex(&re, 0);
            term.match_set_cursor_name(tag, "pointer");
        }
        apply_theme(&term, crate::widgets::is_dark(), true);

        let page = gtk::ScrolledWindow::new();
        page.set_hscrollbar_policy(gtk::PolicyType::Never);
        page.set_child(Some(&term));

        let tab = Rc::new(Tab {
            id,
            term,
            page,
            title: RefCell::new(String::new()),
            cwd: RefCell::new(cwd.to_string()),
            claude: RefCell::new(None),
            conn: RefCell::new(None),
            pid: Cell::new(None),
            last_input: Cell::new(Instant::now()),
            last_output: Cell::new(Instant::now()),
            last_switch: Cell::new(None),
            warned: Cell::new(false),
            exited: Cell::new(false),
        });
        tab.install_input_handlers();
        tab.spawn(socket, cli);
        tab
    }

    fn spawn(self: &Rc<Self>, socket: &Path, cli: Option<&Path>) {
        let spec = shell::spec(socket, &self.id, cli);
        let argv: Vec<&str> = spec.argv.iter().map(|s| s.as_str()).collect();
        let env: Vec<&str> = spec.env.iter().map(|s| s.as_str()).collect();
        let cwd = self.cwd.borrow().clone();
        let me = Rc::downgrade(self);
        self.term.spawn_async(
            vte::PtyFlags::DEFAULT,
            Some(&cwd),
            &argv,
            &env,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |res| {
                let Some(tab) = me.upgrade() else { return };
                match res {
                    Ok(pid) => tab.pid.set(Some(pid)),
                    Err(e) => {
                        let msg = format!("\r\nCC Deck couldn't start the shell: {e}\r\n");
                        tab.term.feed(msg.as_bytes());
                    }
                }
            },
        );
    }

    fn install_input_handlers(self: &Rc<Self>) {
        // Typing counts as activity; Shift+Enter inserts a newline in Claude Code's prompt.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let me = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(tab) = me.upgrade() else { return glib::Propagation::Proceed };
            tab.last_input.set(Instant::now());
            let mods = state & (gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK);
            if (key == gdk::Key::Return || key == gdk::Key::KP_Enter) && mods == gdk::ModifierType::SHIFT_MASK {
                tab.term.feed_child(b"\x1b\r");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.term.add_controller(keys);

        let me = Rc::downgrade(self);
        self.term.connect_contents_changed(move |_| {
            if let Some(t) = me.upgrade() {
                t.last_output.set(Instant::now());
            }
        });

        // Ctrl+click opens links (OSC 8 hyperlinks or plain URLs).
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let me = Rc::downgrade(self);
        click.connect_released(move |g, _, x, y| {
            let Some(tab) = me.upgrade() else { return };
            if !g.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return;
            }
            let uri = tab
                .term
                .check_hyperlink_at(x, y)
                .map(|s| s.to_string())
                .or_else(|| tab.term.check_match_at(x, y).0.map(|s| s.to_string()));
            if let Some(uri) = uri {
                let root = tab.term.root();
                let win = root.and_then(|r| r.downcast::<gtk::Window>().ok());
                gtk::UriLauncher::new(&uri).launch(win.as_ref(), None::<&gio::Cancellable>, |_| {});
            }
        });
        self.term.add_controller(click);
    }

    pub fn display_title(&self) -> String {
        let title = self.title.borrow();
        if let Some(c) = self.claude.borrow().as_ref() {
            return if title.is_empty() { format!("Claude · {}", c.account) } else { title.clone() };
        }
        if !title.is_empty() {
            return title.clone();
        }
        let cwd = self.cwd.borrow();
        if *cwd == home().to_string_lossy() {
            "Home".into()
        } else {
            crate::model::last_component(&cwd)
        }
    }

    pub fn subtitle(&self) -> String {
        if let Some(c) = self.claude.borrow().as_ref() {
            return format!("{} · {}", c.account, crate::model::last_component(&c.cwd));
        }
        let cwd = self.cwd.borrow();
        if *cwd == home().to_string_lossy() {
            "Terminal".into()
        } else {
            crate::model::short_path(&cwd)
        }
    }

    pub fn current_dir(&self) -> String {
        self.claude.borrow().as_ref().map(|c| c.cwd.clone()).unwrap_or_else(|| self.cwd.borrow().clone())
    }

    /// Seconds since the last output or keystroke, whichever is more recent.
    pub fn idle_secs(&self) -> f64 {
        self.last_output.get().elapsed().min(self.last_input.get().elapsed()).as_secs_f64()
    }

    pub fn type_text(&self, text: &str) {
        self.term.feed_child(text.as_bytes());
    }

    pub fn terminate(&self) {
        if let Some(pid) = self.pid.get() {
            if !self.exited.get() {
                unsafe {
                    libc::kill(pid.0, libc::SIGHUP);
                }
            }
        }
    }

    /// Update title/cwd from the terminal; returns true when something changed.
    pub fn sync_title(&self) -> bool {
        let raw = self.term.window_title().map(|s| s.to_string()).unwrap_or_default();
        let t: String = raw.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).to_string();
        if *self.title.borrow() != t {
            *self.title.borrow_mut() = t;
            return true;
        }
        false
    }

    pub fn sync_cwd(&self) -> bool {
        let Some(uri) = self.term.current_directory_uri() else { return false };
        let Some(path) = gio::File::for_uri(&uri).path() else { return false };
        let p = path.to_string_lossy().to_string();
        if p.is_empty() || *self.cwd.borrow() == p {
            return false;
        }
        *self.cwd.borrow_mut() = p;
        true
    }
}
