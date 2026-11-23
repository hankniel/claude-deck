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
    pub started: std::time::SystemTime,
}

pub struct Tab {
    pub id: String,
    pub term: vte::Terminal,
    pub page: gtk::Box,
    search: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
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
    term.set_font_scale(crate::theme::term_scale());
}

const URL_RE: &str = r#"(https?|file)://[^\s<>"'`]+[^\s<>"'`.,;:!?)\]]"#;
const PCRE2_MULTILINE: u32 = 0x0000_0400;
const PCRE2_CASELESS: u32 = 0x0000_0008;
const PCRE2_LITERAL: u32 = 0x0200_0000;

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

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_child(Some(&term));
        let search_entry = gtk::SearchEntry::builder().placeholder_text("Find in terminal").hexpand(true).build();
        let search = gtk::SearchBar::builder().show_close_button(true).child(&search_entry).build();
        search.connect_entry(&search_entry);
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&search);
        page.append(&scroller);

        let tab = Rc::new(Tab {
            id,
            term,
            page,
            search,
            search_entry,
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
        tab.install_search();
        tab.install_drop();
        tab.spawn(socket, cli);
        tab
    }

    /// Ctrl+Shift+F: find in the scrollback. Enter / Shift+Enter go to the next / previous match.
    pub fn find(&self) {
        self.search.set_search_mode(true);
        self.search_entry.grab_focus();
    }

    fn install_search(self: &Rc<Self>) {
        self.term.search_set_wrap_around(true);
        let me = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |e| {
            let Some(t) = me.upgrade() else { return };
            let q = e.text();
            let re = (!q.is_empty())
                .then(|| vte::Regex::for_search(&q, PCRE2_LITERAL | PCRE2_CASELESS | PCRE2_MULTILINE).ok())
                .flatten();
            t.term.search_set_regex(re.as_ref(), 0);
            if re.is_some() {
                t.term.unselect_all();
                t.term.search_find_previous();
            }
        });
        let me = Rc::downgrade(self);
        self.search_entry.connect_activate(move |_| {
            if let Some(t) = me.upgrade() {
                t.term.search_find_previous();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let me = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(t) = me.upgrade() else { return glib::Propagation::Proceed };
            if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
                if state.contains(gdk::ModifierType::SHIFT_MASK) {
                    t.term.search_find_next();
                } else {
                    t.term.search_find_previous();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search_entry.add_controller(keys);
        // Closing the bar hands the keyboard back to the terminal.
        let me = Rc::downgrade(self);
        self.search.connect_search_mode_enabled_notify(move |b| {
            if let Some(t) = me.upgrade() {
                if !b.is_search_mode() {
                    t.term.search_set_regex(None, 0);
                    t.term.grab_focus();
                }
            }
        });
    }

    /// Dropping files or folders types their shell-quoted paths, like GNOME Terminal.
    fn install_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let me = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let Some(t) = me.upgrade() else { return false };
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            let paths: Vec<String> = list
                .files()
                .iter()
                .filter_map(|f| f.path())
                .map(|p| claude_deck::accounts::shell_quote(&p.to_string_lossy()))
                .collect();
            if paths.is_empty() {
                return false;
            }
            t.last_input.set(Instant::now());
            t.term.feed_child(format!("{} ", paths.join(" ")).as_bytes());
            t.term.grab_focus();
            true
        });
        self.term.add_controller(target);
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
            if tab.exited.get() && (key == gdk::Key::Return || key == gdk::Key::KP_Enter) {
                let _ = tab.term.activate_action("win.tab-close", Some(&tab.id.to_variant()));
                return glib::Propagation::Stop;
            }
            if (key == gdk::Key::Return || key == gdk::Key::KP_Enter) && mods == gdk::ModifierType::SHIFT_MASK {
                tab.term.feed_child(b"\x1b\r");
                return glib::Propagation::Stop;
            }
            // Zoom only here, so Ctrl +/- elsewhere (canvas, files) is left to that view.
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && !mods.contains(gdk::ModifierType::ALT_MASK) {
                let action = match key {
                    gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => Some("win.zoom-in"),
                    gdk::Key::minus | gdk::Key::KP_Subtract => Some("win.zoom-out"),
                    gdk::Key::_0 | gdk::Key::KP_0 => Some("win.zoom-reset"),
                    _ => None,
                };
                if let Some(a) = action {
                    let _ = tab.term.activate_action(a, None);
                    return glib::Propagation::Stop;
                }
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

        // A click on a link opens it (OSC 8 hyperlinks or plain URLs); a drag doesn't.
        // Capture phase: with mouse tracking on (Claude Code), VTE takes the click for the app
        // and a bubble-phase gesture never sees the release. The event still reaches VTE.
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let down: Rc<RefCell<Option<(f64, f64, String)>>> = Rc::new(RefCell::new(None));
        let (d1, me1) = (down.clone(), Rc::downgrade(self));
        click.connect_pressed(move |_, n, x, y| {
            let Some(tab) = me1.upgrade() else { return };
            let uri = (n == 1)
                .then(|| {
                    tab.term
                        .check_hyperlink_at(x, y)
                        .map(|s| s.to_string())
                        .or_else(|| tab.term.check_match_at(x, y).0.map(|s| s.to_string()))
                })
                .flatten();
            *d1.borrow_mut() = uri.map(|u| (x, y, u));
        });
        let me = Rc::downgrade(self);
        click.connect_released(move |_, _, x, y| {
            let Some(tab) = me.upgrade() else { return };
            let Some((x0, y0, uri)) = down.borrow_mut().take() else { return };
            if (x - x0).abs() > 4.0 || (y - y0).abs() > 4.0 {
                return;
            }
            let win = tab.term.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            let fallback = uri.clone();
            gtk::UriLauncher::new(&uri).launch(win.as_ref(), None::<&gio::Cancellable>, move |res| {
                if res.is_err() {
                    let _ = gio::AppInfo::launch_default_for_uri(&fallback, None::<&gio::AppLaunchContext>);
                }
            });
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
