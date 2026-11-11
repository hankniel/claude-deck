//! Jira, native: your open issues, the board (drag cards between columns), epics with progress,
//! JQL search, and an issue panel to move, assign, edit, link, sprint and comment.

use crate::bg;
use crate::prefs;
use crate::widgets::{self, label};
use adw::prelude::*;
use claude_deck::canvas::jira;
use gtk::{gdk, gio, glib};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Mine,
    Board,
    Epics,
    Search,
}

const MODES: [(Mode, &str); 4] = [(Mode::Mine, "Mine"), (Mode::Board, "Board"), (Mode::Epics, "Epics"), (Mode::Search, "Search")];
const SCOPES: [&str; 3] = ["sprint", "backlog", "all"];

pub struct JiraPage {
    pub root: gtk::Stack,
    pane: gtk::Paned,
    main: gtk::Box,
    detail: gtk::Box,
    filter: gtk::SearchEntry,
    board_dd: gtk::DropDown,
    scope_dd: gtk::DropDown,
    who: gtk::Label,
    spinner: gtk::Spinner,
    mode: Cell<Mode>,
    boards: RefCell<Vec<Value>>,
    data: RefCell<Value>,
    current: RefCell<Option<String>>,
    started: Cell<bool>,
    syncing: Cell<bool>,
    pub on_toast: crate::canvas::Callback<dyn Fn(&str)>,
    /// Session folder to draw dependency graphs into, and a way to show what was drawn.
    pub pick_session: crate::canvas::Callback<dyn Fn() -> Option<(String, PathBuf)>>,
    pub show_screen: crate::canvas::Callback<dyn Fn(&str, &str)>,
    me: RefCell<Weak<JiraPage>>,
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

pub fn pill(text: &str, category: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("status-pill");
    l.add_css_class(match category {
        "done" => "cat-done",
        "indeterminate" => "cat-doing",
        _ => "cat-todo",
    });
    l.set_valign(gtk::Align::Center);
    l
}

fn type_badge(t: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(&t.chars().next().map(|c| c.to_string()).unwrap_or_default()));
    l.add_css_class("type-badge");
    l.add_css_class(&format!("type-{}", t.to_lowercase().replace(['-', ' '], "")));
    l.set_tooltip_text(Some(t));
    l.set_valign(gtk::Align::Center);
    l
}

fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

impl JiraPage {
    pub fn new() -> Rc<JiraPage> {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.set_margin_start(14);
        bar.set_margin_end(14);
        bar.set_margin_top(10);
        bar.set_margin_bottom(8);
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        modes.add_css_class("linked");
        let mut first: Option<gtk::ToggleButton> = None;
        let mut btns = vec![];
        for (m, name) in MODES {
            let b = gtk::ToggleButton::with_label(name);
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            modes.append(&b);
            btns.push((m, b));
        }
        bar.append(&modes);
        let board_dd = gtk::DropDown::from_strings(&[]);
        board_dd.set_tooltip_text(Some("Board"));
        let scope_dd = gtk::DropDown::from_strings(&["Active sprint", "Backlog", "All open"]);
        bar.append(&board_dd);
        bar.append(&scope_dd);
        let filter = gtk::SearchEntry::new();
        filter.set_hexpand(true);
        filter.set_width_chars(12);
        filter.set_placeholder_text(Some("Filter by key, summary, person, epic"));
        bar.append(&filter);
        let new_btn = gtk::MenuButton::new();
        new_btn.set_label("New");
        new_btn.set_icon_name("list-add-symbolic");
        new_btn.set_tooltip_text(Some("Create an issue"));
        let new_pop = gtk::Popover::new();
        new_btn.set_popover(Some(&new_pop));
        bar.append(&new_btn);
        let spinner = gtk::Spinner::new();
        bar.append(&spinner);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Reload from Jira"));
        bar.append(&refresh);
        let who = label("", &["dim-label", "caption"]);
        who.set_ellipsize(gtk::pango::EllipsizeMode::End);
        who.set_max_width_chars(18);
        bar.append(&who);

        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.set_vexpand(true);
        main.set_hexpand(true);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.set_size_request(420, -1);
        let pane = gtk::Paned::new(gtk::Orientation::Horizontal);
        pane.set_start_child(Some(&main));
        pane.set_end_child(Some(&detail));
        pane.set_resize_end_child(false);
        pane.set_shrink_end_child(false);
        detail.set_visible(false);
        let ui = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // the bar scrolls sideways rather than forcing a minimum window width
        let bar_scroll = gtk::ScrolledWindow::new();
        bar_scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
        bar_scroll.set_hscrollbar_policy(gtk::PolicyType::External);
        bar_scroll.set_propagate_natural_height(true);
        bar_scroll.set_child(Some(&bar));
        ui.append(&bar_scroll);
        ui.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        ui.append(&pane);
        pane.set_vexpand(true);

        let setup = adw::StatusPage::builder()
            .icon_name("network-server-symbolic")
            .title("Connect Jira")
            .description(format!(
                "Create an API token at id.atlassian.com → Security → API tokens, then put these lines in\n<tt>{}</tt>:\n\n<tt>JIRA_BASE_URL=https://your-site.atlassian.net\nJIRA_EMAIL=you@company.com\nJIRA_API_TOKEN=…</tt>\n\nThe agent's <tt>claude-deck canvas jira</tt> commands use the same token.",
                glib::markup_escape_text(&jira::env_file().to_string_lossy())
            ))
            .build();
        let sb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        sb.set_halign(gtk::Align::Center);
        let open_env = gtk::Button::with_label("Open jira.env");
        open_env.add_css_class("pill");
        open_env.add_css_class("suggested-action");
        let retry = gtk::Button::with_label("Check again");
        retry.add_css_class("pill");
        sb.append(&open_env);
        sb.append(&retry);
        setup.set_child(Some(&sb));

        let root = gtk::Stack::new();
        root.add_named(&setup, Some("setup"));
        root.add_named(&ui, Some("ui"));

        let page = Rc::new(JiraPage {
            root,
            pane,
            main,
            detail,
            filter,
            board_dd,
            scope_dd,
            who,
            spinner,
            mode: Cell::new(match prefs::get("jira_mode").as_deref() {
                Some("board") => Mode::Board,
                Some("epics") => Mode::Epics,
                Some("search") => Mode::Search,
                _ => Mode::Mine,
            }),
            boards: RefCell::new(vec![]),
            data: RefCell::new(Value::Null),
            current: RefCell::new(None),
            started: Cell::new(false),
            syncing: Cell::new(false),
            on_toast: RefCell::new(None),
            pick_session: RefCell::new(None),
            show_screen: RefCell::new(None),
            me: RefCell::new(Weak::new()),
        });
        *page.me.borrow_mut() = Rc::downgrade(&page);
        let scope = prefs::get("jira_scope").and_then(|s| SCOPES.iter().position(|x| *x == s)).unwrap_or(0);
        page.scope_dd.set_selected(scope as u32);

        for (m, b) in &btns {
            b.set_active(*m == page.mode.get());
            let (me, m) = (Rc::downgrade(&page), *m);
            b.connect_toggled(move |b| {
                if let (true, Some(p)) = (b.is_active(), me.upgrade()) {
                    p.mode.set(m);
                    prefs::set("jira_mode", &format!("{m:?}").to_lowercase());
                    p.sync_bar();
                    p.load(false);
                }
            });
        }
        let me = Rc::downgrade(&page);
        page.board_dd.connect_selected_notify(move |d| {
            let Some(p) = me.upgrade() else { return };
            if p.syncing.get() {
                return;
            }
            if let Some(b) = p.boards.borrow().get(d.selected() as usize) {
                prefs::set("jira_board", &b["id"].to_string());
            }
            p.load(false);
        });
        let me = Rc::downgrade(&page);
        page.scope_dd.connect_selected_notify(move |d| {
            if let Some(p) = me.upgrade() {
                prefs::set("jira_scope", SCOPES[d.selected() as usize % 3]);
                p.load(false);
            }
        });
        let me = Rc::downgrade(&page);
        page.filter.connect_search_changed(move |_| {
            if let Some(p) = me.upgrade() {
                if p.mode.get() != Mode::Search {
                    p.render();
                }
            }
        });
        let me = Rc::downgrade(&page);
        page.filter.connect_activate(move |_| {
            if let Some(p) = me.upgrade() {
                if p.mode.get() == Mode::Search {
                    p.load(true);
                }
            }
        });
        let me = Rc::downgrade(&page);
        refresh.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.load(true);
            }
        });
        let me = Rc::downgrade(&page);
        retry.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.started.set(false);
                p.activate();
            }
        });
        open_env.connect_clicked(move |b| {
            let f = jira::env_file();
            if !f.exists() {
                let _ = std::fs::create_dir_all(f.parent().unwrap());
                let _ = std::fs::write(&f, "JIRA_BASE_URL=https://your-site.atlassian.net\nJIRA_EMAIL=you@company.com\nJIRA_API_TOKEN=\n");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600));
                }
            }
            let win = b.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            gtk::FileLauncher::new(Some(&gio::File::for_path(&f))).launch(win.as_ref(), None::<&gio::Cancellable>, |_| {});
        });
        let me = Rc::downgrade(&page);
        new_pop.connect_show(move |pop| {
            if let Some(p) = me.upgrade() {
                pop.set_child(Some(&p.new_issue_form(pop)));
            }
        });
        page.sync_bar();
        page
    }

    /// Project, type, summary → create; types come from the selected board.
    fn new_issue_form(self: &Rc<Self>, pop: &gtk::Popover) -> gtk::Widget {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.set_margin_top(10);
        b.set_margin_bottom(10);
        b.set_margin_start(10);
        b.set_margin_end(10);
        b.set_size_request(340, -1);
        b.append(&label("New issue", &["heading"]));
        let project = gtk::Entry::new();
        project.set_placeholder_text(Some("Project key"));
        project.set_text(&self.board().map(|x| s(&x["project"])).unwrap_or_default());
        b.append(&project);
        let mut types: Vec<String> = self.data.borrow()["types"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default();
        if types.is_empty() {
            types = vec!["Task".into(), "Story".into(), "Bug".into()];
        }
        let refs: Vec<&str> = types.iter().map(|x| x.as_str()).collect();
        let ty = gtk::DropDown::from_strings(&refs);
        b.append(&ty);
        let summary = gtk::Entry::new();
        summary.set_placeholder_text(Some("Summary"));
        b.append(&summary);
        let create = gtk::Button::with_label("Create");
        create.add_css_class("suggested-action");
        b.append(&create);
        let (me, pop) = (Rc::downgrade(self), pop.clone());
        let go = move || {
            let (proj, sum) = (project.text().trim().to_uppercase(), summary.text().trim().to_string());
            let kind = types[ty.selected() as usize % types.len()].clone();
            if proj.is_empty() || sum.is_empty() {
                return;
            }
            pop.popdown();
            let Some(p) = me.upgrade() else { return };
            let me2 = Rc::downgrade(&p);
            bg::run(
                move || {
                    jira::create(&jira::NewIssue { project: &proj, summary: &sum, kind: &kind, parent: None, description: None, points: None, labels: vec![], assignee: None })
                },
                move |r| {
                    let Some(p) = me2.upgrade() else { return };
                    match r {
                        Ok(key) => {
                            p.toast(&format!("Created {key}"));
                            p.load(true);
                            p.open_issue(&key);
                        }
                        Err(e) => p.toast(&e),
                    }
                },
            );
        };
        let go = Rc::new(go);
        let g2 = go.clone();
        create.connect_clicked(move |_| g2());
        b.upcast()
    }

    fn toast(&self, t: &str) {
        if let Some(f) = self.on_toast.borrow().clone() {
            f(t);
        }
    }

    fn sync_bar(&self) {
        let m = self.mode.get();
        self.board_dd.set_visible(m == Mode::Board || m == Mode::Epics);
        self.scope_dd.set_visible(m == Mode::Board);
        self.filter.set_placeholder_text(Some(if m == Mode::Search { "JQL, e.g. project = RAID AND text ~ \"ingest\" — Enter" } else { "Filter by key, summary, person, epic" }));
    }

    /// Called when the page is shown.
    pub fn activate(self: &Rc<Self>) {
        if self.started.get() {
            return;
        }
        if !jira::configured() {
            self.root.set_visible_child_name("setup");
            return;
        }
        self.started.set(true);
        self.root.set_visible_child_name("ui");
        let me = Rc::downgrade(self);
        bg::run(
            || Ok((jira::myself()?, jira::boards("")?)),
            move |r| {
                let Some(p) = me.upgrade() else { return };
                match r {
                    Ok((who, boards)) => {
                        p.who.set_text(&s(&who["name"]));
                        p.syncing.set(true);
                        let names: Vec<String> = boards.iter().map(|b| format!("{} ({})", s(&b["name"]), s(&b["project"]))).collect();
                        let refs: Vec<&str> = names.iter().map(|x| x.as_str()).collect();
                        p.board_dd.set_model(Some(&gtk::StringList::new(&refs)));
                        let want = prefs::get("jira_board");
                        let i = boards.iter().position(|b| Some(b["id"].to_string()) == want).unwrap_or(0);
                        p.board_dd.set_selected(i as u32);
                        *p.boards.borrow_mut() = boards;
                        p.syncing.set(false);
                        p.load(false);
                    }
                    Err(e) => {
                        p.started.set(false);
                        p.toast(&e);
                        p.show_message(&e);
                    }
                }
            },
        );
    }

    fn board(&self) -> Option<Value> {
        self.boards.borrow().get(self.board_dd.selected() as usize).cloned()
    }

    fn show_message(&self, text: &str) {
        while let Some(c) = self.main.first_child() {
            self.main.remove(&c);
        }
        let l = label(text, &["dim-label"]);
        l.set_wrap(true);
        l.set_margin_top(40);
        l.set_halign(gtk::Align::Center);
        self.main.append(&l);
    }

    fn load(self: &Rc<Self>, fresh: bool) {
        if !self.started.get() {
            return;
        }
        let mode = self.mode.get();
        let board = self.board();
        let scope = SCOPES[self.scope_dd.selected() as usize % 3].to_string();
        let jql = self.filter.text().to_string();
        if fresh {
            // drop cached reads by touching a write-free path: the cache is short anyway
        }
        if mode == Mode::Search && jql.trim().is_empty() {
            self.show_message("Type JQL above and press Enter.");
            return;
        }
        self.spinner.start();
        let me = Rc::downgrade(self);
        bg::run(
            move || {
                Ok(match mode {
                    Mode::Mine => json!(jira::mine()?),
                    Mode::Search => json!(jira::search(&jql, 100)?),
                    Mode::Board => match board {
                        Some(b) => jira::board(b["id"].as_u64().unwrap_or(0), &scope)?,
                        None => json!(null),
                    },
                    Mode::Epics => match board.and_then(|b| b["project"].as_str().map(String::from)).filter(|p| !p.is_empty()) {
                        Some(p) => json!(jira::epics(&p)?),
                        None => json!([]),
                    },
                })
            },
            move |r| {
                let Some(p) = me.upgrade() else { return };
                p.spinner.stop();
                match r {
                    Ok(v) => {
                        *p.data.borrow_mut() = v;
                        p.render();
                    }
                    Err(e) => p.show_message(&e),
                }
            },
        );
    }

    fn matches(&self, i: &Value) -> bool {
        let q = self.filter.text().to_lowercase();
        if q.is_empty() || self.mode.get() == Mode::Search {
            return true;
        }
        [&i["key"], &i["summary"], &i["assignee"], &i["status"], &i["epic"]["key"], &i["epic"]["summary"], &i["type"]]
            .iter()
            .any(|v| v.as_str().is_some_and(|x| x.to_lowercase().contains(&q)))
    }

    fn render(self: &Rc<Self>) {
        while let Some(c) = self.main.first_child() {
            self.main.remove(&c);
        }
        let data = self.data.borrow().clone();
        match self.mode.get() {
            Mode::Mine | Mode::Search => {
                let list = gtk::ListBox::new();
                list.add_css_class("boxed-list");
                list.set_selection_mode(gtk::SelectionMode::None);
                let items: Vec<Value> = data.as_array().cloned().unwrap_or_default().into_iter().filter(|i| self.matches(i)).collect();
                for i in &items {
                    list.append(&self.issue_row(i));
                }
                let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
                b.set_margin_top(14);
                b.set_margin_start(14);
                b.set_margin_end(14);
                b.set_margin_bottom(14);
                b.append(&label(
                    &format!("{} issue{}", items.len(), if items.len() == 1 { "" } else { "s" }),
                    &["dim-label", "caption"],
                ));
                if !items.is_empty() {
                    b.append(&list);
                }
                self.main.append(&scrolled(&b, true));
            }
            Mode::Board => self.render_board(&data),
            Mode::Epics => self.render_epics(&data),
        }
    }

    fn issue_row(self: &Rc<Self>, i: &Value) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.append(&type_badge(&s(&i["type"])));
        row.append(&label(&s(&i["key"]), &["dim-label", "numeric", "caption"]));
        let sum = label(&s(&i["summary"]), &[]);
        sum.set_hexpand(true);
        sum.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&sum);
        row.append(&pill(&s(&i["status"]), &s(&i["category"])));
        if !s(&i["assignee"]).is_empty() {
            let a = label(&initials(&s(&i["assignee"])), &["avatar-initials"]);
            a.set_tooltip_text(Some(&s(&i["assignee"])));
            row.append(&a);
        }
        let key = s(&i["key"]);
        let me = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            if let Some(p) = me.upgrade() {
                p.open_issue(&key);
            }
        });
        row.add_controller(click);
        row.set_cursor_from_name(Some("pointer"));
        row.upcast()
    }

    fn card(self: &Rc<Self>, i: &Value) -> gtk::Widget {
        let c = gtk::Box::new(gtk::Orientation::Vertical, 6);
        c.add_css_class("card");
        c.add_css_class("jira-card");
        if self.current.borrow().as_deref() == Some(&s(&i["key"])) {
            c.add_css_class("selected-card");
        }
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.append(&type_badge(&s(&i["type"])));
        let k = label(&s(&i["key"]), &["dim-label", "caption", "numeric"]);
        k.set_hexpand(true);
        top.append(&k);
        if i["blocked"] == json!(true) {
            let b = label("blocked", &["blocked-flag"]);
            b.set_tooltip_text(Some(&format!("blocked by {}", i["blockedBy"].as_array().map(|a| a.iter().map(s).collect::<Vec<_>>().join(", ")).unwrap_or_default())));
            top.append(&b);
        }
        c.append(&top);
        let sum = label(&s(&i["summary"]), &[]);
        sum.set_wrap(true);
        sum.set_lines(3);
        sum.set_ellipsize(gtk::pango::EllipsizeMode::End);
        c.append(&sum);
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if let Some(e) = i["epic"]["key"].as_str() {
            let l = label(e, &["epic-chip"]);
            l.set_tooltip_text(Some(&s(&i["epic"]["summary"])));
            foot.append(&l);
        }
        if let Some(pt) = i["points"].as_f64() {
            foot.append(&label(&format!("{pt} pt"), &["dim-label", "caption"]));
        }
        let due = s(&i["duedate"]);
        if !due.is_empty() {
            let late = chrono::NaiveDate::parse_from_str(&due, "%Y-%m-%d").is_ok_and(|d| d < chrono::Local::now().date_naive()) && i["category"] != "done";
            foot.append(&label(&format!("due {due}"), if late { &["caption", "error"] } else { &["caption", "dim-label"] }));
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        foot.append(&spacer);
        if !s(&i["assignee"]).is_empty() {
            let a = label(&initials(&s(&i["assignee"])), &["avatar-initials"]);
            a.set_tooltip_text(Some(&s(&i["assignee"])));
            foot.append(&a);
        }
        c.append(&foot);
        let key = s(&i["key"]);
        let me = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        let k2 = key.clone();
        click.connect_released(move |_, _, _, _| {
            if let Some(p) = me.upgrade() {
                p.open_issue(&k2);
            }
        });
        c.add_controller(click);
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::MOVE);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&key.to_value())));
        c.add_controller(drag);
        c.set_cursor_from_name(Some("grab"));
        c.upcast()
    }

    fn render_board(self: &Rc<Self>, b: &Value) {
        if b.is_null() {
            self.show_message("No boards visible to you.");
            return;
        }
        let wrap = gtk::Box::new(gtk::Orientation::Vertical, 8);
        wrap.set_margin_top(10);
        wrap.set_margin_start(14);
        wrap.set_margin_end(14);
        let mut head = s(&b["name"]);
        if let Some(sp) = b["sprint"]["name"].as_str() {
            head = format!("{head} · {sp}");
            let end = s(&b["sprint"]["end"]);
            if end.len() >= 10 {
                head = format!("{head} (ends {})", &end[..10]);
            }
        }
        wrap.append(&label(&head, &["heading"]));
        let cols = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        cols.set_margin_bottom(14);
        for c in b["columns"].as_array().cloned().unwrap_or_default() {
            let issues: Vec<Value> = c["issues"].as_array().cloned().unwrap_or_default().into_iter().filter(|i| self.matches(i)).collect();
            let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
            col.add_css_class("jira-column");
            col.set_size_request(270, -1);
            let pts: f64 = issues.iter().filter_map(|i| i["points"].as_f64()).sum();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let name = label(&s(&c["name"]).to_uppercase(), &["caption-heading"]);
            name.set_hexpand(true);
            h.append(&name);
            h.append(&label(&format!("{}{}", issues.len(), if pts > 0.0 { format!(" · {pts} pt") } else { String::new() }), &["dim-label", "caption", "numeric"]));
            col.append(&h);
            for i in &issues {
                col.append(&self.card(i));
            }
            let statuses: Vec<String> = c["statuses"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default();
            let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
            let (me, col2, cname) = (Rc::downgrade(self), col.clone(), s(&c["name"]));
            drop.connect_enter(move |_, _, _| {
                col2.add_css_class("drop-hover");
                gdk::DragAction::MOVE
            });
            let col3 = col.clone();
            drop.connect_leave(move |_| col3.remove_css_class("drop-hover"));
            let col4 = col.clone();
            drop.connect_drop(move |_, v, _, _| {
                col4.remove_css_class("drop-hover");
                let (Some(p), Ok(key)) = (me.upgrade(), v.get::<String>()) else { return false };
                p.move_to_column(&key, &statuses, &cname);
                true
            });
            col.add_controller(drop);
            cols.append(&col);
        }
        wrap.append(&cols);
        let sc = gtk::ScrolledWindow::new();
        sc.set_child(Some(&wrap));
        sc.set_vexpand(true);
        self.main.append(&sc);
    }

    fn move_to_column(self: &Rc<Self>, key: &str, statuses: &[String], column: &str) {
        let (k, st, col) = (key.to_string(), statuses.to_vec(), column.to_string());
        self.spinner.start();
        let me = Rc::downgrade(self);
        bg::run(
            move || {
                let tr = jira::transitions(&k)?;
                let t = tr
                    .iter()
                    .find(|t| st.contains(&s(&t["toId"])))
                    .ok_or_else(|| anyhow::anyhow!("{k} can't move to {col} from its status"))?;
                jira::move_to(&k, &s(&t["id"]))
            },
            move |r| {
                let Some(p) = me.upgrade() else { return };
                p.spinner.stop();
                match r {
                    Ok(said) => {
                        p.toast(&said);
                        p.load(true);
                    }
                    Err(e) => p.toast(&e),
                }
            },
        );
    }

    fn render_epics(self: &Rc<Self>, data: &Value) {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
        b.set_margin_top(14);
        b.set_margin_start(14);
        b.set_margin_end(14);
        b.set_margin_bottom(14);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let epics: Vec<Value> = data.as_array().cloned().unwrap_or_default().into_iter().filter(|i| self.matches(i)).collect();
        for e in &epics {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
            row.set_margin_top(10);
            row.set_margin_bottom(10);
            row.set_margin_start(12);
            row.set_margin_end(12);
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            h.append(&label(&s(&e["key"]), &["dim-label", "numeric", "caption"]));
            let t = label(&s(&e["summary"]), &["heading"]);
            t.set_hexpand(true);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            h.append(&t);
            h.append(&pill(&s(&e["status"]), &s(&e["category"])));
            let show = gtk::Button::with_label("Board");
            show.add_css_class("flat");
            show.set_tooltip_text(Some("Show this epic on the board"));
            let (me, k) = (Rc::downgrade(self), s(&e["key"]));
            show.connect_clicked(move |_| {
                if let Some(p) = me.upgrade() {
                    p.filter.set_text(&k);
                    p.set_mode(Mode::Board);
                }
            });
            h.append(&show);
            let graph = gtk::Button::with_label("Dependencies");
            graph.add_css_class("flat");
            graph.set_tooltip_text(Some("Draw the Blocks links among its issues on the canvas"));
            let (me, k) = (Rc::downgrade(self), s(&e["key"]));
            graph.connect_clicked(move |_| {
                if let Some(p) = me.upgrade() {
                    p.draw_epic(&k);
                }
            });
            h.append(&graph);
            row.append(&h);
            let total = e["total"].as_f64().unwrap_or(0.0);
            let done = e["done"].as_f64().unwrap_or(0.0);
            row.append(&widgets::thin_bar(if total > 0.0 { done / total * 100.0 } else { 0.0 }, widgets::hex(0x30A46C), 5));
            row.append(&label(
                &format!(
                    "{done}/{total} done · {} in progress · {}/{} pt{}",
                    e["doing"],
                    e["pointsDone"].as_f64().unwrap_or(0.0),
                    e["points"].as_f64().unwrap_or(0.0),
                    if e["blocked"].as_u64().unwrap_or(0) > 0 { format!(" · {} blocked", e["blocked"]) } else { String::new() }
                ),
                &["dim-label", "caption"],
            ));
            list.append(&row);
        }
        b.append(&label(&format!("{} epics in {}", epics.len(), self.board().map(|x| s(&x["project"])).unwrap_or_default()), &["dim-label", "caption"]));
        if !epics.is_empty() {
            b.append(&list);
        }
        self.main.append(&scrolled(&b, true));
    }

    fn set_mode(self: &Rc<Self>, m: Mode) {
        self.mode.set(m);
        // toggle the matching button
        let bar = self.root.child_by_name("ui").and_then(|u| u.first_child()).and_then(|sc| sc.downcast::<gtk::ScrolledWindow>().ok()).and_then(|sc| sc.child());
        let bar = bar.and_then(|b| if b.is::<gtk::Viewport>() { b.first_child() } else { Some(b) });
        let mut w = bar.and_then(|bar| bar.first_child()).and_then(|modes| modes.first_child());
        let mut i = 0;
        while let Some(b) = w {
            if let Some(t) = b.downcast_ref::<gtk::ToggleButton>() {
                if MODES[i].0 == m {
                    t.set_active(true);
                }
            }
            i += 1;
            w = b.next_sibling();
        }
        self.sync_bar();
        self.load(false);
    }

    fn draw_epic(self: &Rc<Self>, epic: &str) {
        let target = self.pick_session.borrow().clone().and_then(|f| f());
        let Some((sid, dir)) = target else {
            self.toast("Start an agent session first: graphs are drawn into its canvas");
            return;
        };
        let e = epic.to_string();
        let me = Rc::downgrade(self);
        bg::run(
            move || {
                let (m, n, edges) = jira::epic_graph(&e)?;
                let name = format!("{e}-deps.mmd");
                std::fs::write(dir.join("content").join(&name), m)?;
                Ok((name, n, edges))
            },
            move |r| {
                let Some(p) = me.upgrade() else { return };
                match r {
                    Ok((name, n, edges)) => {
                        p.toast(&format!("{name}: {n} issues, {edges} dependencies"));
                        if let Some(f) = p.show_screen.borrow().clone() {
                            f(&sid, &name);
                        }
                    }
                    Err(e) => p.toast(&e),
                }
            },
        );
    }

    // ---------- issue panel ----------

    fn open_issue(self: &Rc<Self>, key: &str) {
        *self.current.borrow_mut() = Some(key.to_string());
        self.detail.set_visible(true);
        if self.pane.position() < 200 || self.pane.position() > self.pane.width() - 300 {
            self.pane.set_position((self.pane.width() - 440).max(300));
        }
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let sp = gtk::Spinner::new();
        sp.start();
        sp.set_margin_top(40);
        self.detail.append(&sp);
        let (k, board) = (key.to_string(), self.board());
        let me = Rc::downgrade(self);
        bg::run(
            move || {
                let i = jira::issue(&k)?;
                let sprints = board.and_then(|b| b["id"].as_u64()).map(|id| jira::sprints(id).unwrap_or_default()).unwrap_or_default();
                let cur = jira::issue_sprint(&k);
                Ok((i, sprints, cur))
            },
            move |r| {
                let Some(p) = me.upgrade() else { return };
                match r {
                    Ok((i, sprints, cur)) => p.render_issue(&i, &sprints, cur.as_ref()),
                    Err(e) => {
                        while let Some(c) = p.detail.first_child() {
                            p.detail.remove(&c);
                        }
                        p.detail.append(&label(&e, &["error"]));
                    }
                }
            },
        );
    }

    /// Run a Jira write, then reload the issue and the list.
    fn write(self: &Rc<Self>, key: &str, f: impl FnOnce() -> anyhow::Result<String> + Send + 'static) {
        let me = Rc::downgrade(self);
        let k = key.to_string();
        bg::run(f, move |r| {
            let Some(p) = me.upgrade() else { return };
            match r {
                Ok(said) => {
                    p.toast(&said);
                    p.open_issue(&k);
                    p.load(true);
                }
                Err(e) => p.toast(&e),
            }
        });
    }

    fn render_issue(self: &Rc<Self>, i: &Value, sprints: &[Value], cur_sprint: Option<&Value>) {
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let key = s(&i["key"]);
        let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
        b.set_margin_top(14);
        b.set_margin_bottom(20);
        b.set_margin_start(16);
        b.set_margin_end(16);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.append(&type_badge(&s(&i["type"])));
        let link = gtk::LinkButton::with_label(&s(&i["url"]), &key);
        link.add_css_class("flat");
        top.append(&link);
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        top.append(&sp);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        let me = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.detail.set_visible(false);
                *p.current.borrow_mut() = None;
            }
        });
        top.append(&close);
        b.append(&top);

        let summary = gtk::Entry::new();
        summary.set_text(&s(&i["summary"]));
        summary.add_css_class("title-4");
        summary.set_tooltip_text(Some("Edit and press Enter"));
        let (me, k) = (Rc::downgrade(self), key.clone());
        summary.connect_activate(move |e| {
            let (t, k2) = (e.text().to_string(), k.clone());
            if let Some(p) = me.upgrade() {
                p.write(&k, move || jira::edit(&k2, &json!({"summary": t})));
            }
        });
        b.append(&summary);

        // status
        let st = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        st.append(&pill(&s(&i["status"]), &s(&i["category"])));
        let trs: Vec<Value> = i["transitions"].as_array().cloned().unwrap_or_default();
        if !trs.is_empty() {
            let names: Vec<String> = std::iter::once("Move to…".to_string()).chain(trs.iter().map(|t| s(&t["to"]))).collect();
            let refs: Vec<&str> = names.iter().map(|x| x.as_str()).collect();
            let dd = gtk::DropDown::from_strings(&refs);
            let (me, k) = (Rc::downgrade(self), key.clone());
            dd.connect_selected_notify(move |d| {
                let n = d.selected() as usize;
                if n == 0 {
                    return;
                }
                let (id, k2) = (s(&trs[n - 1]["id"]), k.clone());
                if let Some(p) = me.upgrade() {
                    p.write(&k, move || jira::move_to(&k2, &id));
                }
            });
            st.append(&dd);
        }
        b.append(&st);

        // fields
        let grid = gtk::Grid::new();
        grid.set_column_spacing(10);
        grid.set_row_spacing(8);
        let mut row = 0;
        let mut field = |name: &str, w: &gtk::Widget| {
            grid.attach(&label(name, &["dim-label"]), 0, row, 1, 1);
            w.set_hexpand(true);
            grid.attach(w, 1, row, 1, 1);
            row += 1;
        };
        let who = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let a = s(&i["assignee"]);
        who.append(&label(if a.is_empty() { "Unassigned" } else { &a }, &[]));
        let mine = gtk::Button::with_label("Assign to me");
        mine.add_css_class("flat");
        let (me, k) = (Rc::downgrade(self), key.clone());
        mine.connect_clicked(move |_| {
            let k2 = k.clone();
            if let Some(p) = me.upgrade() {
                p.write(&k, move || jira::assign(&k2, "me"));
            }
        });
        who.append(&mine);
        field("Assignee", who.upcast_ref());
        let entry_field = |page: &Rc<JiraPage>, value: String, key: &str, field_name: &'static str, placeholder: &str| -> gtk::Widget {
            let e = gtk::Entry::new();
            e.set_text(&value);
            e.set_placeholder_text(Some(placeholder));
            let (me, k) = (Rc::downgrade(page), key.to_string());
            e.connect_activate(move |e| {
                let (v, k2) = (e.text().to_string(), k.clone());
                if let Some(p) = me.upgrade() {
                    p.write(&k, move || jira::edit(&k2, &json!({ field_name: v })));
                }
            });
            e.upcast()
        };
        if i["pointsField"] == json!(true) {
            let v = i["points"].as_f64().map(|x| x.to_string()).unwrap_or_default();
            field("Points", &entry_field(self, v, &key, "points", "story points — Enter"));
        }
        let labels = i["labels"].as_array().map(|a| a.iter().map(s).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        field("Labels", &entry_field(self, labels, &key, "labels", "a, b — Enter"));
        field("Due", &entry_field(self, s(&i["duedate"]), &key, "duedate", "YYYY-MM-DD — Enter"));
        if !sprints.is_empty() {
            let mut names = vec!["Backlog".to_string()];
            names.extend(sprints.iter().map(|x| format!("{} ({})", s(&x["name"]), s(&x["state"]))));
            let refs: Vec<&str> = names.iter().map(|x| x.as_str()).collect();
            let dd = gtk::DropDown::from_strings(&refs);
            let cur = cur_sprint.and_then(|c| sprints.iter().position(|x| x["id"] == c["id"])).map(|p| p + 1).unwrap_or(0);
            dd.set_selected(cur as u32);
            let (me, k, ids) = (Rc::downgrade(self), key.clone(), sprints.iter().map(|x| x["id"].to_string()).collect::<Vec<_>>());
            dd.connect_selected_notify(move |d| {
                let n = d.selected() as usize;
                let target = if n == 0 { "backlog".to_string() } else { ids[n - 1].clone() };
                let k2 = k.clone();
                if let Some(p) = me.upgrade() {
                    p.write(&k, move || jira::set_sprint(&[k2], &target));
                }
            });
            field("Sprint", dd.upcast_ref());
        }
        let rep = s(&i["reporter"]);
        if !rep.is_empty() {
            field("Reporter", label(&rep, &[]).upcast_ref());
        }
        b.append(&grid);

        // description
        b.append(&label("Description", &["heading"]));
        let desc = gtk::TextView::new();
        desc.set_wrap_mode(gtk::WrapMode::WordChar);
        desc.buffer().set_text(&s(&i["description"]));
        desc.set_top_margin(8);
        desc.set_bottom_margin(8);
        desc.set_left_margin(8);
        desc.set_right_margin(8);
        let df = gtk::Frame::new(None);
        df.set_child(Some(&desc));
        df.set_size_request(-1, 120);
        b.append(&df);
        let save = gtk::Button::with_label("Save description");
        save.set_halign(gtk::Align::Start);
        let (me, k, d2) = (Rc::downgrade(self), key.clone(), desc.clone());
        save.connect_clicked(move |_| {
            let buf = d2.buffer();
            let (t, k2) = (buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string(), k.clone());
            if let Some(p) = me.upgrade() {
                p.write(&k, move || jira::edit(&k2, &json!({"description": t})));
            }
        });
        b.append(&save);

        // links
        b.append(&label("Links", &["heading"]));
        for l in i["links"].as_array().cloned().unwrap_or_default() {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            r.append(&label(&s(&l["label"]), &["dim-label", "caption"]));
            let other = s(&l["key"]);
            let ob = gtk::Button::with_label(&other);
            ob.add_css_class("flat");
            let (me, o) = (Rc::downgrade(self), other.clone());
            ob.connect_clicked(move |_| {
                if let Some(p) = me.upgrade() {
                    p.open_issue(&o);
                }
            });
            r.append(&ob);
            let t = label(&s(&l["summary"]), &[]);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.set_hexpand(true);
            r.append(&t);
            r.append(&pill(&s(&l["status"]), &s(&l["category"])));
            let x = gtk::Button::from_icon_name("list-remove-symbolic");
            x.add_css_class("flat");
            x.set_tooltip_text(Some("Remove link"));
            let (me, k) = (Rc::downgrade(self), key.clone());
            x.connect_clicked(move |_| {
                let (k2, o2) = (k.clone(), other.clone());
                if let Some(p) = me.upgrade() {
                    p.write(&k, move || jira::unlink(&k2, &o2));
                }
            });
            r.append(&x);
            b.append(&r);
        }
        let add = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let kinds = ["blocks", "blocked-by", "relates", "duplicates"];
        let kd = gtk::DropDown::from_strings(&kinds);
        add.append(&kd);
        let other = gtk::Entry::new();
        other.set_placeholder_text(Some("KEY-123 — Enter"));
        other.set_hexpand(true);
        let (me, k) = (Rc::downgrade(self), key.clone());
        other.connect_activate(move |e| {
            let (o, kind, k2) = (e.text().trim().to_uppercase(), kinds[kd.selected() as usize % 4], k.clone());
            if !jira::valid_key(&o) {
                return;
            }
            if let Some(p) = me.upgrade() {
                p.write(&k, move || jira::link(&k2, kind, &o));
            }
        });
        add.append(&other);
        b.append(&add);

        let subs = i["subtasks"].as_array().cloned().unwrap_or_default();
        if !subs.is_empty() {
            b.append(&label("Subtasks", &["heading"]));
            for st in subs {
                b.append(&self.issue_row(&st));
            }
        }

        // comments
        b.append(&label("Comments", &["heading"]));
        for c in i["comments"].as_array().cloned().unwrap_or_default() {
            let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
            v.add_css_class("comment");
            let created = s(&c["created"]);
            v.append(&label(&format!("{} · {}", s(&c["author"]), &created[..created.len().min(10)]), &["caption-heading"]));
            let t = label(&s(&c["text"]), &[]);
            t.set_wrap(true);
            t.set_selectable(true);
            v.append(&t);
            b.append(&v);
        }
        let ct = gtk::TextView::new();
        ct.set_wrap_mode(gtk::WrapMode::WordChar);
        ct.set_top_margin(6);
        ct.set_bottom_margin(6);
        ct.set_left_margin(8);
        let cf = gtk::Frame::new(None);
        cf.set_child(Some(&ct));
        cf.set_size_request(-1, 70);
        b.append(&cf);
        let post = gtk::Button::with_label("Comment");
        post.add_css_class("suggested-action");
        post.set_halign(gtk::Align::Start);
        let (me, k) = (Rc::downgrade(self), key.clone());
        post.connect_clicked(move |_| {
            let buf = ct.buffer();
            let (t, k2) = (buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string(), k.clone());
            if t.trim().is_empty() {
                return;
            }
            if let Some(p) = me.upgrade() {
                p.write(&k, move || jira::comment(&k2, &t));
            }
        });
        b.append(&post);

        self.detail.append(&scrolled(&b, false));
    }
}

fn scrolled(child: &impl IsA<gtk::Widget>, clamp: bool) -> gtk::ScrolledWindow {
    let sc = gtk::ScrolledWindow::new();
    sc.set_hscrollbar_policy(gtk::PolicyType::Never);
    sc.set_vexpand(true);
    if clamp {
        let c = adw::Clamp::builder().maximum_size(1100).child(child).build();
        sc.set_child(Some(&c));
    } else {
        sc.set_child(Some(child));
    }
    sc
}
