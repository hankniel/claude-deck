//! Canvas: the tree of agent sessions and their screens (shown in the side bar) and the
//! native views of each screen (flowcharts, lineage, ERDs, reviews, row counts, documents,
//! files, git changes), which open as workbench tabs through `on_show`.

pub mod board;
pub mod draw;
pub mod erdview;
pub mod flowview;
pub mod gitview;
pub mod lineageview;
pub mod planview;
pub mod screens;
pub mod sequenceview;
pub mod view;

use crate::widgets::label;
use adw::prelude::*;
use claude_deck::canvas::store::{self, Kind, Session};
use claude_deck::canvas::{dbml, flow, lineage, sequence};
use gtk::gio;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use view::Diagram;

pub type Callback<T> = RefCell<Option<Rc<T>>>;

enum Target {
    Session(String),
    Screen(String, String),
}

/// A view the canvas asks the workbench to show.
pub struct Shown {
    pub key: String,
    pub title: String,
    pub icon: String,
    pub tooltip: String,
    pub crumb: String,
    pub widget: gtk::Widget,
    pub focus: bool,
}

struct Open {
    key: String,
    holder: gtk::Box,
    sid: String,
    name: String,
    mtime: i64,
    /// The user clicked, typed or scrolled in it: an agent update waits behind a banner
    /// instead of replacing what they were looking at or filling in.
    touched: Rc<Cell<bool>>,
}

/// Note any interaction inside `w` without consuming it.
fn track_touch(w: &gtk::Box) -> Rc<Cell<bool>> {
    let t = Rc::new(Cell::new(false));
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let t1 = t.clone();
    click.connect_pressed(move |_, _, _, _| t1.set(true));
    w.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let t2 = t.clone();
    keys.connect_key_pressed(move |_, _, _, _| {
        t2.set(true);
        gtk::glib::Propagation::Proceed
    });
    w.add_controller(keys);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let t3 = t.clone();
    scroll.connect_scroll(move |_, _, _| {
        t3.set(true);
        gtk::glib::Propagation::Proceed
    });
    w.add_controller(scroll);
    t
}

pub struct CanvasPage {
    explorer: gtk::ListBox,
    rows: RefCell<Vec<(gtk::ListBoxRow, Target)>>,
    sessions: RefCell<Vec<Session>>,
    expanded: RefCell<HashSet<String>>,
    show_ended: Cell<bool>,
    open: RefCell<Vec<Open>>,
    seen: RefCell<HashMap<String, i64>>,
    loaded: Cell<bool>,
    syncing: Cell<bool>,
    current: RefCell<Option<String>>,
    pub on_toast: Callback<dyn Fn(&str)>,
    /// Show (open or reveal) a view as a tab.
    pub on_show: Callback<dyn Fn(Shown)>,
    /// Close the tab of a view.
    pub on_hide: Callback<dyn Fn(&str)>,
    /// A view's content was reloaded.
    pub on_changed: Callback<dyn Fn(&str)>,
    me: RefCell<Weak<CanvasPage>>,
}

fn ago(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(crate::model::relative).unwrap_or_default()
}

pub fn state_color(state: &str) -> Option<draw::Rgb> {
    Some(crate::widgets::hex(match state {
        "running" => 0x30A46C,
        "waiting" => 0xF0902E,
        "idle" => 0x8B8D98,
        "ended" => 0x55575E,
        _ => return None,
    }))
}

pub fn state_text(s: &Session) -> String {
    match s.state.state.as_str() {
        "running" => "running".into(),
        "waiting" => {
            if s.state.reason.is_empty() || s.state.reason == "question" {
                "needs you".into()
            } else {
                format!("needs you: {}", s.state.reason)
            }
        }
        "idle" => "your turn".into(),
        "ended" => "ended".into(),
        _ => String::new(),
    }
}

pub fn screen_key(sid: &str, name: &str) -> String {
    format!("screen:{sid}/{name}")
}

impl CanvasPage {
    pub fn new() -> Rc<CanvasPage> {
        let explorer = gtk::ListBox::new();
        explorer.add_css_class("wb-tree");
        explorer.set_selection_mode(gtk::SelectionMode::Single);
        let page = Rc::new(CanvasPage {
            explorer,
            rows: RefCell::new(vec![]),
            sessions: RefCell::new(vec![]),
            expanded: RefCell::new(HashSet::new()),
            show_ended: Cell::new(false),
            open: RefCell::new(vec![]),
            seen: RefCell::new(HashMap::new()),
            loaded: Cell::new(false),
            syncing: Cell::new(false),
            current: RefCell::new(None),
            on_toast: RefCell::new(None),
            on_show: RefCell::new(None),
            on_hide: RefCell::new(None),
            on_changed: RefCell::new(None),
            me: RefCell::new(Weak::new()),
        });
        *page.me.borrow_mut() = Rc::downgrade(&page);

        let me = Rc::downgrade(&page);
        page.explorer.connect_row_activated(move |_, row| {
            let Some(p) = me.upgrade() else { return };
            if p.syncing.get() {
                return;
            }
            let target = p.rows.borrow().iter().find(|(r, _)| r == row).map(|(_, t)| match t {
                Target::Session(s) => Target::Session(s.clone()),
                Target::Screen(s, n) => Target::Screen(s.clone(), n.clone()),
            });
            match target {
                Some(Target::Session(sid)) => {
                    let mut e = p.expanded.borrow_mut();
                    if !e.remove(&sid) {
                        e.insert(sid);
                    }
                    drop(e);
                    p.rebuild_explorer();
                }
                Some(Target::Screen(sid, name)) => p.open_screen(&sid, &name, true),
                None => {}
            }
        });
        page
    }

    fn toast(&self, s: &str) {
        if let Some(f) = self.on_toast.borrow().clone() {
            f(s);
        }
    }

    fn show(&self, s: Shown) {
        if let Some(f) = self.on_show.borrow().clone() {
            f(s);
        }
    }

    fn hide(&self, key: &str) {
        self.forget(key);
        if let Some(f) = self.on_hide.borrow().clone() {
            f(key);
        }
    }

    pub fn session(&self, sid: &str) -> Option<Session> {
        self.sessions.borrow().iter().find(|s| s.meta.id == sid).cloned()
    }

    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.borrow().clone()
    }

    /// Does this tab key belong to the canvas?
    pub fn owns(&self, key: &str) -> bool {
        self.open.borrow().iter().any(|o| o.key == key)
    }

    /// The workbench closed a canvas tab.
    pub fn forget(&self, key: &str) {
        self.open.borrow_mut().retain(|o| o.key != key);
    }

    /// The workbench made a tab current: highlight its screen in the tree.
    pub fn set_current(&self, key: Option<&str>) {
        *self.current.borrow_mut() = key.map(String::from);
        self.sync_selection();
    }

    pub fn set_show_ended(&self, on: bool) {
        self.show_ended.set(on);
        self.rebuild_explorer();
    }

    fn visible_session(&self, s: &Session) -> bool {
        if self.show_ended.get() {
            return true;
        }
        if s.state.state == "ended" {
            return !s.items.is_empty() && store::now_ms() - s.updated < 24 * 3600 * 1000;
        }
        !s.items.is_empty() || !s.state.state.is_empty()
    }

    pub fn has_visible(&self) -> bool {
        self.sessions.borrow().iter().any(|s| self.visible_session(s))
    }

    fn rebuild_explorer(&self) {
        self.syncing.set(true);
        for (r, _) in self.rows.borrow_mut().drain(..) {
            self.explorer.remove(&r);
        }
        let sessions = self.sessions.borrow().clone();
        let expanded = self.expanded.borrow().clone();
        let mut rows = vec![];
        for s in sessions.iter().filter(|s| self.visible_session(s)) {
            let row = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let open = expanded.contains(&s.meta.id);
            let chev = gtk::Image::from_icon_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" });
            chev.add_css_class("dim");
            chev.set_margin_start(8);
            h.append(&chev);
            let color = state_color(&s.state.state).unwrap_or(crate::widgets::hex(0x8B8D98));
            let dot = crate::widgets::dot(color, 7);
            dot.set_opacity(if s.state.state.is_empty() { 0.35 } else { 1.0 });
            dot.set_tooltip_text(Some(&state_text(s)));
            h.append(&dot);
            let t = label(&s.label(), &[]);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            h.append(&t);
            let mut sub = vec![];
            if !s.meta.project.is_empty() {
                sub.push(s.meta.project.clone());
            }
            let st = state_text(s);
            if !st.is_empty() && s.needs_you() {
                sub.push(st.clone());
            }
            let sl = label(&sub.join(" · "), &[if s.needs_you() { "warn" } else { "meta" }]);
            sl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            sl.set_hexpand(true);
            h.append(&sl);
            if !s.items.is_empty() {
                h.append(&label(&s.items.len().to_string(), &["meta", "numeric"]));
            }
            row.set_child(Some(&h));
            let mut tip = vec![s.label()];
            if !s.meta.account.is_empty() {
                tip.push(format!("account: {}", s.meta.account));
            }
            if !st.is_empty() {
                tip.push(st.clone());
            }
            tip.push(format!("updated {}", ago(s.updated)));
            tip.push(s.meta.cwd.clone());
            row.set_tooltip_text(Some(&tip.join("\n")));
            self.explorer.append(&row);
            rows.push((row, Target::Session(s.meta.id.clone())));
            if open {
                for it in &s.items {
                    let r = gtk::ListBoxRow::new();
                    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    b.set_margin_start(36);
                    let i = gtk::Image::from_icon_name(it.kind.icon());
                    i.add_css_class("dim");
                    b.append(&i);
                    let nl = label(&it.name, &[]);
                    nl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    nl.set_hexpand(true);
                    b.append(&nl);
                    b.append(&label(&ago(it.mtime), &["meta"]));
                    r.set_child(Some(&b));
                    self.explorer.append(&r);
                    rows.push((r, Target::Screen(s.meta.id.clone(), it.name.clone())));
                }
            }
        }
        *self.rows.borrow_mut() = rows;
        self.syncing.set(false);
        self.sync_selection();
    }

    fn sync_selection(&self) {
        self.syncing.set(true);
        self.explorer.unselect_all();
        let cur = self.current.borrow().clone();
        if let Some(o) = cur.and_then(|k| self.open.borrow().iter().find(|o| o.key == k).map(|o| (o.sid.clone(), o.name.clone()))) {
            for (r, t) in self.rows.borrow().iter() {
                if matches!(t, Target::Screen(s, n) if *s == o.0 && *n == o.1) {
                    self.explorer.select_row(Some(r));
                }
            }
        }
        self.syncing.set(false);
    }

    /// Row under the pointer, for context menus.
    pub fn target_at(&self, y: f64) -> Option<(String, Option<String>)> {
        let row = self.explorer.row_at_y(y as i32)?;
        self.rows.borrow().iter().find(|(r, _)| *r == row).map(|(_, t)| match t {
            Target::Session(s) => (s.clone(), None),
            Target::Screen(s, n) => (s.clone(), Some(n.clone())),
        })
    }

    pub fn explorer(&self) -> &gtk::ListBox {
        &self.explorer
    }

    /// New data from disk. Returns screens that appeared or changed (session id, name).
    pub fn update(&self, sessions: Vec<Session>) -> Vec<(String, String)> {
        let first = !self.loaded.get();
        if first {
            let mut e = self.expanded.borrow_mut();
            for s in sessions.iter().filter(|s| !s.items.is_empty()).take(4) {
                e.insert(s.meta.id.clone());
            }
        }
        *self.sessions.borrow_mut() = sessions.clone();
        let mut changed = vec![];
        {
            let mut seen = self.seen.borrow_mut();
            for s in &sessions {
                for it in &s.items {
                    let key = format!("{}/{}", s.meta.id, it.name);
                    let prev = seen.insert(key, it.mtime);
                    if !first && prev.is_none_or(|p| p < it.mtime) {
                        changed.push((s.meta.id.clone(), it.name.clone()));
                    }
                }
            }
        }
        self.loaded.set(true);

        // open tabs: reload changed, close deleted
        let open: Vec<(String, String, String, i64)> =
            self.open.borrow().iter().map(|o| (o.key.clone(), o.sid.clone(), o.name.clone(), o.mtime)).collect();
        for (key, sid, name, mtime) in open {
            if sid.is_empty() {
                continue; // a file or changes tab
            }
            match sessions.iter().find(|s| s.meta.id == sid).and_then(|s| s.items.iter().find(|i| i.name == name)) {
                Some(it) if it.mtime > mtime => self.reload(&sid, &name),
                Some(_) => {}
                None => self.hide(&key),
            }
        }
        for (sid, name) in &changed {
            let live = sessions.iter().find(|s| &s.meta.id == sid).is_some_and(|s| s.state.state != "ended");
            let already = self.owns(&screen_key(sid, name));
            if live && !already {
                self.expanded.borrow_mut().insert(sid.clone());
                self.open_screen(sid, name, false);
            }
        }
        self.rebuild_explorer();
        changed
    }

    fn on_open_fn(&self, cwd: String) -> screens::OpenFn {
        let me = self.me.borrow().clone();
        Rc::new(move |p: String, line: Option<u32>| {
            let Some(page) = me.upgrade() else { return };
            if p.starts_with("http://") || p.starts_with("https://") {
                let win = page.explorer.root().and_then(|r| r.downcast::<gtk::Window>().ok());
                gtk::UriLauncher::new(&p).launch(win.as_ref(), None::<&gio::Cancellable>, |_| {});
                return;
            }
            page.open_file(&screens::resolve(&cwd, &p), line);
        })
    }

    fn toast_fn(&self) -> screens::ToastFn {
        let me = self.me.borrow().clone();
        Rc::new(move |s: &str| {
            if let Some(p) = me.upgrade() {
                p.toast(s);
            }
        })
    }

    fn build(&self, s: &Session, name: &str) -> (gtk::Widget, i64) {
        let Some(path) = store::screen_path(&s.dir, name) else {
            return (screens::error_page(&format!("{name} is gone")), 0);
        };
        let mtime = s.items.iter().find(|i| i.name == name).map(|i| i.mtime).unwrap_or(0);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return (screens::error_page(&format!("{}: {e}", path.display())), mtime),
        };
        let on_open = self.on_open_fn(s.meta.cwd.clone());
        let w = match Kind::of(name) {
            Some(Kind::Flow) => match flow::parse(&text) {
                Ok(f) => flow_widget(f, on_open),
                Err(flow::Error::Unsupported(k)) if k == "sequenceDiagram" => {
                    let sq = sequence::parse(&text);
                    let d = Diagram::new(sequenceview::SeqModel::new(sq), on_open);
                    d.set_fit(0.6, None);
                    keep(&d.root, d.clone());
                    d.widget()
                }
                Err(flow::Error::Unsupported(k)) => screens::source(
                    &text,
                    &format!("“{k}” Mermaid diagrams are shown as source; CC Deck draws flowcharts natively. Ask for a flowchart, a .lineage or a .dbml screen for an interactive view."),
                    None,
                ),
                Err(e) => screens::source(&text, &format!("This flowchart has a syntax error: {e}"), None),
            },
            Some(Kind::Lineage) => {
                let l = lineage::parse(&text);
                if l.nodes.is_empty() {
                    screens::source(&text, &format!("No lineage found. {}", l.errors.join("; ")), None)
                } else {
                    lineage_widget(l, on_open)
                }
            }
            Some(Kind::Erd) => {
                let e = dbml::parse(&text);
                if e.tables.is_empty() {
                    screens::source(&text, &format!("No tables found. {}", e.errors.join("; ")), None)
                } else {
                    erd_widget(e, store::sidecar(&s.dir, name, "pos"), on_open)
                }
            }
            Some(Kind::Review) => screens::review(&text, &s.dir, name, on_open, self.toast_fn()),
            Some(Kind::Rows) => screens::rows(&text, name),
            Some(Kind::Doc) => screens::doc(&text, &s.dir, name, self.toast_fn()),
            Some(Kind::Open) => {
                let (p, line) = claude_deck::canvas::screens::parse_path(&text);
                screens::file(&screens::resolve(&s.meta.cwd, &p), line, on_open)
            }
            Some(Kind::Markdown) => screens::markdown(&text),
            Some(Kind::Html) => screens::source(&text, "HTML screens open in your browser.", Some(path.clone())),
            Some(Kind::Plan) => planview::plan_view(&text, path.clone(), &s.dir, self.toast_fn()),
            None => screens::source(&text, "Unknown screen type.", None),
        };
        (w, mtime)
    }

    fn holder_with(w: &gtk::Widget) -> gtk::Box {
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        w.set_vexpand(true);
        holder.append(w);
        holder
    }

    fn reveal(&self, key: &str, focus: bool) -> bool {
        if !self.owns(key) {
            return false;
        }
        self.show(Shown {
            key: key.to_string(),
            title: String::new(),
            icon: String::new(),
            tooltip: String::new(),
            crumb: String::new(),
            widget: gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
            focus,
        });
        true
    }

    pub fn open_screen(&self, sid: &str, name: &str, focus: bool) {
        let key = screen_key(sid, name);
        if self.reveal(&key, focus) {
            return;
        }
        let Some(s) = self.session(sid) else { return };
        let (w, mtime) = self.build(&s, name);
        let holder = Self::holder_with(&w);
        let touched = track_touch(&holder);
        self.open.borrow_mut().push(Open { key: key.clone(), holder: holder.clone(), sid: sid.into(), name: name.into(), mtime, touched });
        let project = if s.meta.project.is_empty() { crate::model::last_component(&s.meta.cwd) } else { s.meta.project.clone() };
        self.show(Shown {
            key,
            title: name.to_string(),
            icon: Kind::of(name).map(|k| k.icon()).unwrap_or("text-x-generic-symbolic").to_string(),
            tooltip: format!("{} — {}", s.label(), s.meta.cwd),
            crumb: format!("canvas  ›  {project}  ›  {name}"),
            widget: holder.upcast(),
            focus,
        });
    }

    fn reload(&self, sid: &str, name: &str) {
        self.reload_with(sid, name, false);
    }

    fn reload_with(&self, sid: &str, name: &str, force: bool) {
        let Some(s) = self.session(sid) else { return };
        if !force {
            let mut open = self.open.borrow_mut();
            let Some(o) = open.iter_mut().find(|o| o.sid == sid && o.name == name) else { return };
            if o.touched.get() {
                let mtime = s.items.iter().find(|i| i.name == name).map(|i| i.mtime).unwrap_or(o.mtime);
                o.mtime = mtime; // don't ask again until the next update
                if !o.holder.first_child().is_some_and(|c| c.is::<adw::Banner>()) {
                    let banner = adw::Banner::builder()
                        .title("The agent updated this screen")
                        .button_label("Reload")
                        .revealed(true)
                        .build();
                    let (me, sid2, name2) = (self.me.borrow().clone(), sid.to_string(), name.to_string());
                    banner.connect_button_clicked(move |_| {
                        if let Some(me) = me.upgrade() {
                            me.reload_with(&sid2, &name2, true);
                        }
                    });
                    o.holder.prepend(&banner);
                }
                let key = o.key.clone();
                drop(open);
                if let Some(f) = self.on_changed.borrow().clone() {
                    f(&key);
                }
                return;
            }
        }
        let (w, mtime) = self.build(&s, name);
        let key = {
            let mut open = self.open.borrow_mut();
            let Some(o) = open.iter_mut().find(|o| o.sid == sid && o.name == name) else { return };
            o.touched.set(false);
            while let Some(c) = o.holder.first_child() {
                o.holder.remove(&c);
            }
            w.set_vexpand(true);
            o.holder.append(&w);
            o.mtime = mtime;
            o.key.clone()
        };
        if let Some(f) = self.on_changed.borrow().clone() {
            f(&key);
        }
    }

    pub fn open_file(&self, path: &Path, line: Option<u32>) {
        let key = format!("file:{}", path.display());
        let on_open = self.on_open_fn(path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
        let w = screens::file(path, line, on_open);
        w.set_vexpand(true);
        let existing = self.open.borrow().iter().find(|o| o.key == key).map(|o| o.holder.clone());
        if let Some(holder) = existing {
            while let Some(c) = holder.first_child() {
                holder.remove(&c);
            }
            holder.append(&w);
            self.reveal(&key, true);
            return;
        }
        let holder = Self::holder_with(&w);
        let title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
        self.open.borrow_mut().push(Open { key: key.clone(), holder: holder.clone(), sid: String::new(), name: title.clone(), mtime: 0, touched: Rc::new(Cell::new(false)) });
        let crumb = path
            .to_string_lossy()
            .replace(&claude_deck::config::home().to_string_lossy().to_string(), "~")
            .trim_start_matches('/')
            .split('/')
            .collect::<Vec<_>>()
            .join("  ›  ");
        self.show(Shown {
            key,
            title,
            icon: crate::files::icon_for(path).to_string(),
            tooltip: path.to_string_lossy().to_string(),
            crumb,
            widget: holder.upcast(),
            focus: true,
        });
    }

    /// Git changes of a folder, in a tab.
    pub fn open_changes(&self, dir: &Path, title: &str) {
        let key = format!("changes:{}", dir.display());
        if self.reveal(&key, true) {
            return;
        }
        let w = gitview::changes(dir.to_path_buf());
        let holder = Self::holder_with(&w);
        self.open.borrow_mut().push(Open { key: key.clone(), holder: holder.clone(), sid: String::new(), name: String::new(), mtime: 0, touched: Rc::new(Cell::new(false)) });
        self.show(Shown {
            key,
            title: format!("Changes · {title}"),
            icon: "emblem-documents-symbolic".into(),
            tooltip: dir.to_string_lossy().to_string(),
            crumb: format!("source control  ›  {title}"),
            widget: holder.upcast(),
            focus: true,
        });
    }

    pub fn delete_session(&self, sid: &str) {
        let Some(s) = self.session(sid) else { return };
        let keys: Vec<String> = self.open.borrow().iter().filter(|o| o.sid == sid).map(|o| o.key.clone()).collect();
        for k in keys {
            self.hide(&k);
        }
        match store::delete_session(&s.dir) {
            Ok(()) => self.toast("Session removed from the canvas"),
            Err(e) => self.toast(&format!("{e:#}")),
        }
    }

    pub fn delete_screen(&self, sid: &str, name: &str) {
        let Some(s) = self.session(sid) else { return };
        self.hide(&screen_key(sid, name));
        if let Err(e) = store::delete_screen(&s.dir, name) {
            self.toast(&format!("{e:#}"));
        }
    }

    /// Open screens of a session, newest first (for "go to session").
    pub fn first_screen(&self, sid: &str) -> Option<String> {
        self.session(sid).and_then(|s| s.items.iter().max_by_key(|i| i.mtime).map(|i| i.name.clone()))
    }
}

// ---------- diagram widgets ----------

fn tool_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b
}

fn osd_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("linked");
    b.add_css_class("osd-tools");
    b
}

fn flow_widget(f: flow::Flow, on_open: screens::OpenFn) -> gtk::Widget {
    let has_groups = f.nodes.iter().any(|n| n.group);
    let d = Diagram::new(flowview::FlowModel::new(f), on_open);
    d.set_fit(0.85, None);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Find a node"));
    search.set_width_chars(16);
    search.add_css_class("osd-tools");
    let dd = Rc::downgrade(&d);
    search.connect_search_changed(move |e| {
        let Some(d) = dd.upgrade() else { return };
        let c = d.model.borrow_mut().search(&e.text());
        if let Some((x, y)) = c {
            d.center_on(x, y);
        }
        d.redraw();
    });
    d.tools.append(&search);
    if has_groups {
        let b = osd_box();
        let ex = tool_button("view-fullscreen-symbolic", "Expand all groups");
        let co = tool_button("view-restore-symbolic", "Collapse all groups");
        b.append(&ex);
        b.append(&co);
        for (btn, c) in [(ex, false), (co, true)] {
            let dd = Rc::downgrade(&d);
            btn.connect_clicked(move |_| {
                if let Some(d) = dd.upgrade() {
                    d.model.borrow_mut().set_all_collapsed(c);
                    d.refit();
                }
            });
        }
        d.tools.append(&b);
    }
    let hint = label("Click a node to trace it · click a group title to fold · drag to pan · Ctrl+scroll to zoom", &["caption", "dim-label"]);
    hint.set_valign(gtk::Align::End);
    hint.set_halign(gtk::Align::Start);
    hint.set_margin_start(14);
    hint.set_margin_bottom(18);
    d.root.add_overlay(&hint);
    keep(&d.root, d.clone());
    d.widget()
}

fn lineage_widget(l: lineage::Lineage, on_open: screens::OpenFn) -> gtk::Widget {
    let title = l.title.clone();
    let has_cols = l.nodes.iter().any(|n| !n.columns.is_empty());
    let d = Diagram::new(lineageview::LineageModel::new(l), on_open);
    let anchor = d.model.borrow().focus_center();
    d.set_fit(0.62, anchor);
    if !title.is_empty() {
        let t = label(&title, &["heading", "osd-title"]);
        d.tools.append(&t);
    }
    if has_cols {
        let b = osd_box();
        let ex = tool_button("view-list-symbolic", "Show all columns");
        let co = tool_button("view-restore-symbolic", "Hide columns");
        b.append(&ex);
        b.append(&co);
        for (btn, e) in [(ex, true), (co, false)] {
            let dd = Rc::downgrade(&d);
            btn.connect_clicked(move |_| {
                if let Some(d) = dd.upgrade() {
                    d.model.borrow_mut().set_all_expanded(e);
                    d.redraw();
                }
            });
        }
        d.tools.append(&b);
    }
    let hint = label("Point at an entity to see its upstream and downstream · click a column to trace it", &["caption", "dim-label"]);
    hint.set_valign(gtk::Align::End);
    hint.set_halign(gtk::Align::Start);
    hint.set_margin_start(14);
    hint.set_margin_bottom(18);
    d.root.add_overlay(&hint);
    keep(&d.root, d.clone());
    d.widget()
}

fn erd_widget(e: dbml::Erd, pos: PathBuf, on_open: screens::OpenFn) -> gtk::Widget {
    let summary = format!(
        "{}{} tables · {} relations{}",
        e.project.as_ref().map(|p| format!("{p} · ")).unwrap_or_default(),
        e.tables.len(),
        e.refs.len(),
        if e.enums.is_empty() { String::new() } else { format!(" · {} enums", e.enums.len()) }
    );
    let d = Diagram::new(erdview::ErdModel::new(e, Some(pos)), on_open);
    d.set_fit(0.6, None);
    d.tools.append(&label(&summary, &["heading", "osd-title"]));
    let reset = tool_button("view-refresh-symbolic", "Reset layout");
    reset.add_css_class("osd-tools");
    let dd = Rc::downgrade(&d);
    reset.connect_clicked(move |_| {
        if let Some(d) = dd.upgrade() {
            d.model.borrow_mut().reset_layout();
            d.refit();
        }
    });
    d.tools.append(&reset);
    let hint = label("Drag tables (or a group's title) to arrange · point at a column or line to follow a relation", &["caption", "dim-label"]);
    hint.set_valign(gtk::Align::End);
    hint.set_halign(gtk::Align::Start);
    hint.set_margin_start(14);
    hint.set_margin_bottom(18);
    d.root.add_overlay(&hint);
    keep(&d.root, d.clone());
    d.widget()
}

/// Tie an Rc to a widget's lifetime (the widget owns its controller state).
fn keep<T: 'static>(w: &impl IsA<gtk::Widget>, v: Rc<T>) {
    unsafe {
        w.as_ref().set_data("cc-deck-keep", v);
    }
}
