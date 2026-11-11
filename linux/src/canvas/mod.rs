//! Canvas page: an explorer of agent sessions and their screens, and tabs that render each
//! screen natively (flowcharts, lineage, ERDs, reviews, row counts, documents, files).

pub mod board;
pub mod draw;
pub mod erdview;
pub mod flowview;
pub mod lineageview;
pub mod screens;
pub mod view;

use crate::widgets::label;
use adw::prelude::*;
use claude_deck::canvas::store::{self, Kind, Session};
use claude_deck::canvas::{dbml, flow, lineage};
use gtk::{gio, glib};
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

struct Open {
    key: String,
    page: adw::TabPage,
    holder: gtk::Box,
    sid: String,
    name: String,
    mtime: i64,
}

pub struct CanvasPage {
    pub root: gtk::Paned,
    explorer: gtk::ListBox,
    rows: RefCell<Vec<(gtk::ListBoxRow, Target)>>,
    tabs: adw::TabView,
    stack: gtk::Stack,
    sessions: RefCell<Vec<Session>>,
    expanded: RefCell<HashSet<String>>,
    show_ended: Cell<bool>,
    open: RefCell<Vec<Open>>,
    seen: RefCell<HashMap<String, i64>>,
    loaded: Cell<bool>,
    syncing: Cell<bool>,
    pub on_toast: Callback<dyn Fn(&str)>,
    pub on_title: Callback<dyn Fn(&str)>,
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

impl CanvasPage {
    pub fn new() -> Rc<CanvasPage> {
        let explorer = gtk::ListBox::new();
        explorer.add_css_class("navigation-sidebar");
        explorer.set_selection_mode(gtk::SelectionMode::Single);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.set_margin_start(14);
        head.set_margin_end(8);
        head.set_margin_top(10);
        head.set_margin_bottom(4);
        let t = label("Agent sessions", &["heading"]);
        t.set_hexpand(true);
        head.append(&t);
        let ended = gtk::ToggleButton::new();
        ended.set_icon_name("document-open-recent-symbolic");
        ended.add_css_class("flat");
        ended.set_tooltip_text(Some("Show ended sessions"));
        head.append(&ended);
        let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side.append(&head);
        let sc = gtk::ScrolledWindow::new();
        sc.set_hscrollbar_policy(gtk::PolicyType::Never);
        sc.set_vexpand(true);
        sc.set_child(Some(&explorer));
        side.append(&sc);
        side.set_size_request(240, -1);

        let tabs = adw::TabView::new();
        let bar = adw::TabBar::new();
        bar.set_view(Some(&tabs));
        bar.set_autohide(false);
        let tabbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tabbox.append(&bar);
        tabbox.append(&tabs);
        tabs.set_vexpand(true);
        let empty = adw::StatusPage::builder()
            .icon_name("view-paged-symbolic")
            .title("Canvas")
            .description(
                "Screens your agents draw show up here: flowcharts (.mmd), data lineage (.lineage), database diagrams (.dbml), review findings (.rv), row counts (.rows) and documents to comment on (.doc).\n\nAgents learn the formats from the canvas skill; ask one to draw something.",
            )
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&tabbox, Some("tabs"));
        stack.set_visible_child_name("empty");

        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_start_child(Some(&side));
        root.set_end_child(Some(&stack));
        root.set_position(280);
        root.set_shrink_start_child(false);
        root.set_resize_start_child(false);

        let page = Rc::new(CanvasPage {
            root,
            explorer,
            rows: RefCell::new(vec![]),
            tabs,
            stack,
            sessions: RefCell::new(vec![]),
            expanded: RefCell::new(HashSet::new()),
            show_ended: Cell::new(false),
            open: RefCell::new(vec![]),
            seen: RefCell::new(HashMap::new()),
            loaded: Cell::new(false),
            syncing: Cell::new(false),
            on_toast: RefCell::new(None),
            on_title: RefCell::new(None),
            me: RefCell::new(Weak::new()),
        });
        *page.me.borrow_mut() = Rc::downgrade(&page);

        let me = Rc::downgrade(&page);
        ended.connect_toggled(move |b| {
            if let Some(p) = me.upgrade() {
                p.show_ended.set(b.is_active());
                p.rebuild_explorer();
            }
        });
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
        let me = Rc::downgrade(&page);
        page.tabs.connect_close_page(move |view, tp| {
            if let Some(p) = me.upgrade() {
                p.open.borrow_mut().retain(|o| &o.page != tp);
                if view.n_pages() <= 1 {
                    p.stack.set_visible_child_name("empty");
                }
                p.emit_title();
            }
            glib::Propagation::Proceed
        });
        let me = Rc::downgrade(&page);
        page.tabs.connect_selected_page_notify(move |_| {
            if let Some(p) = me.upgrade() {
                p.emit_title();
                p.sync_selection();
            }
        });
        page
    }

    fn toast(&self, s: &str) {
        if let Some(f) = self.on_toast.borrow().clone() {
            f(s);
        }
    }

    fn emit_title(&self) {
        let t = self
            .tabs
            .selected_page()
            .map(|p| p.title().to_string())
            .unwrap_or_default();
        if let Some(f) = self.on_title.borrow().clone() {
            f(&t);
        }
    }

    pub fn current_title(&self) -> String {
        self.tabs.selected_page().map(|p| p.title().to_string()).unwrap_or_default()
    }

    pub fn session(&self, sid: &str) -> Option<Session> {
        self.sessions.borrow().iter().find(|s| s.meta.id == sid).cloned()
    }

    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.borrow().clone()
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
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            h.set_margin_top(4);
            h.set_margin_bottom(4);
            let color = state_color(&s.state.state).unwrap_or(crate::widgets::hex(0x8B8D98));
            let dot = crate::widgets::dot(color, 8);
            dot.set_opacity(if s.state.state.is_empty() { 0.35 } else { 1.0 });
            dot.set_tooltip_text(Some(&state_text(s)));
            h.append(&dot);
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            v.set_hexpand(true);
            let t = label(&s.label(), &[]);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&t);
            let mut sub = vec![];
            if !s.meta.project.is_empty() {
                sub.push(s.meta.project.clone());
            }
            if !s.meta.account.is_empty() {
                sub.push(s.meta.account.clone());
            }
            let st = state_text(s);
            if !st.is_empty() {
                sub.push(st);
            }
            sub.push(ago(s.updated));
            let sl = label(&sub.join(" · "), &["caption", "dim-label"]);
            sl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if s.needs_you() {
                sl.remove_css_class("dim-label");
                sl.add_css_class("warning");
            }
            v.append(&sl);
            h.append(&v);
            let open = expanded.contains(&s.meta.id);
            let n = label(&format!("{} {}", s.items.len(), if open { "▾" } else { "▸" }), &["caption", "dim-label", "numeric"]);
            h.append(&n);
            row.set_child(Some(&h));
            row.set_tooltip_text(Some(&format!("{}\n{}", s.meta.cwd, s.meta.id)));
            self.explorer.append(&row);
            rows.push((row, Target::Session(s.meta.id.clone())));
            if open {
                for it in &s.items {
                    let r = gtk::ListBoxRow::new();
                    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    b.set_margin_start(18);
                    let i = gtk::Image::from_icon_name(it.kind.icon());
                    i.add_css_class("dim-label");
                    b.append(&i);
                    let nl = label(&it.name, &[]);
                    nl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    nl.set_hexpand(true);
                    b.append(&nl);
                    b.append(&label(&ago(it.mtime), &["caption", "dim-label"]));
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
        if let Some(sel) = self.tabs.selected_page() {
            if let Some(o) = self.open.borrow().iter().find(|o| o.page == sel) {
                for (r, t) in self.rows.borrow().iter() {
                    if matches!(t, Target::Screen(s, n) if *s == o.sid && *n == o.name) {
                        self.explorer.select_row(Some(r));
                    }
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
    pub fn update(&self, sessions: Vec<Session>, visible: bool) -> Vec<(String, String)> {
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
        let open: Vec<(String, String, i64, adw::TabPage)> =
            self.open.borrow().iter().map(|o| (o.sid.clone(), o.name.clone(), o.mtime, o.page.clone())).collect();
        for (sid, name, mtime, page) in open {
            if sid.is_empty() {
                continue; // a file tab
            }
            match sessions.iter().find(|s| s.meta.id == sid).and_then(|s| s.items.iter().find(|i| i.name == name)) {
                Some(it) if it.mtime > mtime => self.reload(&sid, &name),
                Some(_) => {}
                None => self.tabs.close_page(&page),
            }
        }
        for (sid, name) in &changed {
            let live = sessions.iter().find(|s| &s.meta.id == sid).is_some_and(|s| s.state.state != "ended");
            let already = self.open.borrow().iter().any(|o| &o.sid == sid && &o.name == name);
            if live && !already {
                self.expanded.borrow_mut().insert(sid.clone());
                self.open_screen(sid, name, visible);
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
                let win = page.root.root().and_then(|r| r.downcast::<gtk::Window>().ok());
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
            Some(Kind::Plan) => screens::source(&text, "Work breakdown (.plan). Pushing plans to Jira arrives in a later CC Deck version.", None),
            None => screens::source(&text, "Unknown screen type.", None),
        };
        (w, mtime)
    }

    fn select_page(&self, page: &adw::TabPage, focus: bool) {
        self.stack.set_visible_child_name("tabs");
        if focus || self.tabs.selected_page().is_none() {
            self.tabs.set_selected_page(page);
        }
    }

    pub fn open_screen(&self, sid: &str, name: &str, focus: bool) {
        let key = format!("{sid}/{name}");
        if let Some(p) = self.open.borrow().iter().find(|o| o.key == key).map(|o| o.page.clone()) {
            self.select_page(&p, focus);
            return;
        }
        let Some(s) = self.session(sid) else { return };
        let (w, mtime) = self.build(&s, name);
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        w.set_vexpand(true);
        holder.append(&w);
        let page = self.tabs.append(&holder);
        page.set_title(name);
        page.set_tooltip(&glib::markup_escape_text(&format!("{} — {}", s.label(), s.meta.cwd)));
        if let Some(k) = Kind::of(name) {
            page.set_icon(Some(&gio::ThemedIcon::new(k.icon())));
        }
        self.open.borrow_mut().push(Open { key, page: page.clone(), holder, sid: sid.into(), name: name.into(), mtime });
        self.select_page(&page, focus);
        self.emit_title();
    }

    fn reload(&self, sid: &str, name: &str) {
        let Some(s) = self.session(sid) else { return };
        let (w, mtime) = self.build(&s, name);
        let mut open = self.open.borrow_mut();
        let Some(o) = open.iter_mut().find(|o| o.sid == sid && o.name == name) else { return };
        while let Some(c) = o.holder.first_child() {
            o.holder.remove(&c);
        }
        w.set_vexpand(true);
        o.holder.append(&w);
        o.mtime = mtime;
        o.page.set_needs_attention(self.tabs.selected_page().as_ref() != Some(&o.page));
    }

    pub fn open_file(&self, path: &Path, line: Option<u32>) {
        let key = format!("file:{}", path.display());
        let on_open = self.on_open_fn(path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
        let w = screens::file(path, line, on_open);
        w.set_vexpand(true);
        let existing = self.open.borrow().iter().find(|o| o.key == key).map(|o| (o.page.clone(), o.holder.clone()));
        if let Some((page, holder)) = existing {
            while let Some(c) = holder.first_child() {
                holder.remove(&c);
            }
            holder.append(&w);
            self.select_page(&page, true);
            return;
        }
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.append(&w);
        let page = self.tabs.append(&holder);
        let title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
        page.set_title(&title);
        page.set_tooltip(&glib::markup_escape_text(&path.to_string_lossy()));
        page.set_icon(Some(&gio::ThemedIcon::new(if path.is_dir() { "folder-symbolic" } else { "text-x-generic-symbolic" })));
        self.open.borrow_mut().push(Open { key, page: page.clone(), holder, sid: String::new(), name: title, mtime: 0 });
        self.select_page(&page, true);
    }

    pub fn delete_session(&self, sid: &str) {
        let Some(s) = self.session(sid) else { return };
        let pages: Vec<adw::TabPage> = self.open.borrow().iter().filter(|o| o.sid == sid).map(|o| o.page.clone()).collect();
        for p in pages {
            self.tabs.close_page(&p);
        }
        match store::delete_session(&s.dir) {
            Ok(()) => self.toast("Session removed from the canvas"),
            Err(e) => self.toast(&format!("{e:#}")),
        }
    }

    pub fn delete_screen(&self, sid: &str, name: &str) {
        let Some(s) = self.session(sid) else { return };
        let pages: Vec<adw::TabPage> = self.open.borrow().iter().filter(|o| o.sid == sid && o.name == name).map(|o| o.page.clone()).collect();
        for p in pages {
            self.tabs.close_page(&p);
        }
        if let Err(e) = store::delete_screen(&s.dir, name) {
            self.toast(&format!("{e:#}"));
        }
    }

    pub fn close_current(&self) -> bool {
        match self.tabs.selected_page() {
            Some(p) => {
                self.tabs.close_page(&p);
                true
            }
            None => false,
        }
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
    d.set_fit(0.55, None);
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
