//! The main window: a VS Code-style workbench (see `workbench.rs`) holding terminals (shells
//! and Claude sessions), canvas screens, files, Board, Jira and the Dashboard, plus the glue
//! to `claude-deck launch` (account picker, moving sessions, auto-switch).

use crate::canvas::board::Board;
use crate::canvas::CanvasPage;
use crate::control::{self, Conn, Msg};
use crate::dashboard::Dashboard;
use crate::files::FileTree;
use crate::model::{self, AccountInfo};
use crate::picker::{PickReq, Picker};
use crate::settings::Settings;
use crate::term::{ClaudeRun, Tab};
use crate::widgets::{self, label};
use crate::workbench::{self, Spec, Workbench, EDITOR, PANEL};
use adw::prelude::*;
use claude_deck::accounts;
use claude_deck::config::{home, Config};
use claude_deck::stats;
use gtk::{gdk, gio, glib};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vte::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub enum Sel {
    Tab(String),
    Canvas,
    Board,
    Jira,
    Dashboard,
}

pub fn term_key(id: &str) -> String {
    format!("term:{id}")
}

fn term_id(key: &str) -> Option<&str> {
    key.strip_prefix("term:")
}

pub struct Deck {
    pub app: adw::Application,
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    wb: Rc<Workbench>,
    term_menu: gtk::PopoverMenu,
    banner: adw::Banner,
    account_btn: gtk::MenuButton,
    account_label: gtk::Label,
    account_menu: gtk::Box,
    start_btn: gtk::Button,
    branch_btn: gtk::Button,
    branch_label: gtk::Label,
    needs_btn: gtk::Button,
    needs_label: gtk::Label,
    auto_label: gtk::Label,
    sessions: gtk::ListBox,
    session_rows: RefCell<Vec<gtk::ListBoxRow>>,
    account_list: gtk::ListBox,
    spinner: gtk::Spinner,
    dashboard: Rc<Dashboard>,
    pub canvas: Rc<CanvasPage>,
    board: Rc<Board>,
    jira: Rc<crate::jira::JiraPage>,
    files: Rc<FileTree>,
    files_title: gtk::Label,
    canvas_note: gtk::Label,
    canvas_fp: Cell<u64>,
    board_badge: gtk::Label,
    accounts_badge: gtk::Label,
    last_states: RefCell<HashMap<String, String>>,
    socket: PathBuf,
    pub cli: Option<PathBuf>,
    _server: Option<control::Server>,
    pub tabs: RefCell<Vec<Rc<Tab>>>,
    active_term: RefCell<Option<String>>,
    pub accounts: RefCell<Vec<AccountInfo>>,
    pub config: RefCell<Config>,
    picker: RefCell<Option<Rc<Picker>>>,
    pick_queue: RefCell<VecDeque<PickReq>>,
    last_refresh: Cell<Option<Instant>>,
    refreshing: Cell<bool>,
    syncing: Cell<bool>,
    pub settings: RefCell<Option<Rc<Settings>>>,
    auto_action: gio::SimpleAction,
    banner_target: RefCell<Option<(String, String)>>,
}

/// A collapsible explorer section (VS Code style) around `content`.
fn section(title: &str, title_label: Option<&gtk::Label>, tools: &[gtk::Widget], content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let key = format!("wb.sec.{}", title.to_lowercase());
    let open = crate::prefs::get(&key).as_deref() != Some("0");
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let btn = gtk::Button::new();
    btn.add_css_class("wb-section");
    btn.set_hexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let chev = gtk::Image::from_icon_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" });
    inner.append(&chev);
    let l = title_label.cloned().unwrap_or_else(|| label(title, &[]));
    l.set_text(&title.to_uppercase());
    inner.append(&l);
    btn.set_child(Some(&inner));
    head.append(&btn);
    let tbox = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tbox.set_margin_end(6);
    for t in tools {
        t.add_css_class("wb-section-tool");
        tbox.append(t);
    }
    tbox.set_visible(open);
    head.append(&tbox);
    head.add_css_class("wb-section");
    b.append(&head);
    let rev = gtk::Revealer::new();
    rev.set_transition_duration(120);
    rev.set_reveal_child(open);
    rev.set_child(Some(content));
    b.append(&rev);
    btn.connect_clicked(move |_| {
        let on = !rev.reveals_child();
        rev.set_reveal_child(on);
        chev.set_icon_name(Some(if on { "pan-down-symbolic" } else { "pan-end-symbolic" }));
        tbox.set_visible(on);
        crate::prefs::set(&key, if on { "1" } else { "0" });
    });
    b
}

fn icon_btn(icon: &str, tip: &str, action: Option<&str>) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    if let Some(a) = action {
        b.set_action_name(Some(a));
    }
    b
}

fn status_btn(icon: Option<&str>, text: &gtk::Label) -> gtk::Button {
    let b = gtk::Button::new();
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    if let Some(i) = icon {
        h.append(&gtk::Image::from_icon_name(i));
    }
    h.append(text);
    b.set_child(Some(&h));
    b
}

fn key_hint(keys: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(keys));
    l.add_css_class("wb-key");
    l.set_valign(gtk::Align::Center);
    l
}

/// What the editor area shows when nothing is open.
fn welcome() -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_halign(gtk::Align::Center);
    col.set_valign(gtk::Align::Center);
    col.set_width_request(420);
    let logo = gtk::Image::from_icon_name(crate::APP_ID);
    logo.set_pixel_size(64);
    logo.set_halign(gtk::Align::Start);
    logo.set_opacity(0.9);
    logo.set_margin_bottom(10);
    col.append(&logo);
    col.append(&label("CC Deck", &["wb-welcome-title"]));
    let sub = label("Claude Code sessions on the account you pick, with their canvas beside them.", &["wb-welcome-sub"]);
    sub.set_wrap(true);
    col.append(&sub);
    col.append(&label("START", &["wb-welcome-head"]));
    for (text, action, keys) in [
        ("New Claude session", "win.new-claude", "Ctrl+Shift+K"),
        ("New terminal", "win.new-tab", "Ctrl+Shift+T"),
        ("Open the Board", "win.board", "Ctrl+Shift+O"),
        ("Open Jira", "win.jira", "Ctrl+Shift+J"),
        ("Usage dashboard", "win.dashboard", "Ctrl+Shift+D"),
    ] {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let b = gtk::Button::with_label(text);
        b.add_css_class("wb-link");
        b.add_css_class("flat");
        b.set_action_name(Some(action));
        b.set_hexpand(true);
        b.set_halign(gtk::Align::Start);
        h.append(&b);
        h.append(&key_hint(keys));
        col.append(&h);
    }
    col.append(&label("LAYOUT", &["wb-welcome-head"]));
    for (text, keys) in [
        ("Show or hide the panel", "Ctrl+`"),
        ("Show or hide the side bar", "Ctrl+Shift+B"),
        ("Go to an open tab", "Ctrl+Shift+P"),
        ("Drag a tab onto another group to split", "drag"),
    ] {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = label(text, &["wb-welcome-sub"]);
        l.set_hexpand(true);
        h.append(&l);
        h.append(&key_hint(keys));
        h.set_margin_top(4);
        col.append(&h);
    }
    let note = label(
        "Screens your agents draw (flowcharts, lineage, ERDs, reviews, row counts, documents) open here as tabs.",
        &["wb-welcome-sub"],
    );
    note.set_wrap(true);
    note.set_margin_top(18);
    col.append(&note);
    let sc = gtk::ScrolledWindow::new();
    sc.set_hscrollbar_policy(gtk::PolicyType::Never);
    sc.set_child(Some(&col));
    sc.upcast()
}

fn panel_empty() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Center);
    b.append(&label("No terminal open", &["wb-welcome-sub"]));
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let t = gtk::Button::with_label("New Terminal");
    t.set_action_name(Some("win.new-tab"));
    let c = gtk::Button::with_label("New Claude Session");
    c.set_action_name(Some("win.new-claude"));
    c.add_css_class("suggested-action");
    h.append(&t);
    h.append(&c);
    b.append(&h);
    b.upcast()
}

/// Current branch of the repository holding `dir`.
fn git_branch(dir: &Path) -> Option<String> {
    let mut d = Some(dir);
    while let Some(p) = d {
        let g = p.join(".git");
        let gitdir = if g.is_dir() {
            Some(g)
        } else if g.is_file() {
            std::fs::read_to_string(&g)
                .ok()
                .and_then(|s| s.trim().strip_prefix("gitdir:").map(|r| p.join(r.trim())))
        } else {
            None
        };
        if let Some(gd) = gitdir {
            let head = std::fs::read_to_string(gd.join("HEAD")).ok()?;
            let head = head.trim();
            return Some(match head.strip_prefix("ref: refs/heads/") {
                Some(b) => b.to_string(),
                None => head.chars().take(8).collect(),
            });
        }
        d = p.parent();
    }
    None
}

impl Deck {
    pub fn new(app: &adw::Application) -> Rc<Deck> {
        crate::shell::install();
        crate::theme::install();
        let cli = crate::shell::cli_binary();
        let (tx, rx) = async_channel::unbounded::<Msg>();
        let (server, server_err) = match control::start(tx) {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("Account picker unavailable: {e:#}"))),
        };
        let socket = server.as_ref().map(|s| s.path.clone()).unwrap_or_default();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("CC Deck")
            .default_width(1440)
            .default_height(900)
            .width_request(480)
            .height_request(360)
            .build();
        window.add_css_class("wb");

        let wb = Workbench::new();

        // ----- activity bar -----
        let (explorer_act, _) = wb.add_activity("explorer", "folder-symbolic", "Explorer (Ctrl+Shift+E)", false);
        let (accounts_act, accounts_badge) = wb.add_activity("accounts", "avatar-default-symbolic", "Accounts and usage", false);
        let (board_act, board_badge) = wb.add_activity("board", "checkbox-checked-symbolic", "Board (Ctrl+Shift+O)", false);
        let (jira_act, _) = wb.add_activity("jira", "network-server-symbolic", "Jira (Ctrl+Shift+J)", false);
        let (dash_act, _) = wb.add_activity("dashboard", "power-profile-performance-symbolic", "Dashboard (Ctrl+Shift+D)", false);
        let (gear, _) = wb.add_activity("menu", "emblem-system-symbolic", "Manage", true);
        explorer_act.set_action_name(Some("win.explorer"));
        accounts_act.set_action_name(Some("win.accounts-view"));
        board_act.set_action_name(Some("win.board"));
        jira_act.set_action_name(Some("win.jira"));
        dash_act.set_action_name(Some("win.dashboard"));
        {
            let menu = gio::Menu::new();
            let s1 = gio::Menu::new();
            s1.append(Some("New Claude Session"), Some("win.new-claude"));
            s1.append(Some("New Terminal"), Some("win.new-tab"));
            s1.append(Some("New Terminal in Home Folder"), Some("win.new-tab-home"));
            menu.append_section(None, &s1);
            let s2 = gio::Menu::new();
            s2.append(Some("Settings"), Some("win.settings"));
            s2.append(Some("Manage Accounts…"), Some("win.accounts"));
            s2.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
            s2.append(Some("About CC Deck"), Some("win.about"));
            menu.append_section(None, &s2);
            let s3 = gio::Menu::new();
            s3.append(Some("Quit"), Some("app.quit"));
            menu.append_section(None, &s3);
            let pop = gtk::PopoverMenu::from_model(Some(&menu));
            pop.set_parent(&gear);
            pop.set_position(gtk::PositionType::Right);
            pop.set_has_arrow(false);
            gear.connect_clicked(move |_| pop.popup());
        }

        // ----- explorer -----
        let sessions = gtk::ListBox::new();
        sessions.add_css_class("wb-tree");
        let canvas = CanvasPage::new();
        let canvas_note = label("No agent screens yet. Ask Claude to draw a flowchart, lineage or ERD.", &["wb-empty-note"]);
        canvas_note.set_wrap(true);
        let canvas_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        canvas_box.append(canvas.explorer());
        canvas_box.append(&canvas_note);
        let ended = gtk::ToggleButton::new();
        ended.set_icon_name("document-open-recent-symbolic");
        ended.set_tooltip_text(Some("Show ended sessions"));
        ended.add_css_class("flat");
        let c2 = canvas.clone();
        ended.connect_toggled(move |b| c2.set_show_ended(b.is_active()));
        let files = FileTree::new();
        let files_title = label("FILES", &[]);
        let explorer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        explorer.append(&section(
            "Terminals",
            None,
            &[
                icon_btn("starred-symbolic", "New Claude Session", Some("win.new-claude")).upcast(),
                icon_btn("list-add-symbolic", "New Terminal", Some("win.new-tab")).upcast(),
            ],
            &sessions,
        ));
        explorer.append(&section("Canvas", None, &[ended.upcast()], &canvas_box));
        let refresh_files = icon_btn("view-refresh-symbolic", "Refresh", None);
        let f2 = files.clone();
        refresh_files.connect_clicked(move |_| f2.rebuild());
        let hidden = gtk::ToggleButton::new();
        hidden.set_icon_name("view-reveal-symbolic");
        hidden.set_tooltip_text(Some("Show hidden files"));
        hidden.add_css_class("flat");
        hidden.set_active(files.show_hidden());
        let f3 = files.clone();
        hidden.connect_toggled(move |b| f3.set_show_hidden(b.is_active()));
        explorer.append(&section("Files", Some(&files_title), &[hidden.upcast(), refresh_files.upcast()], &files.list));
        let ex_scroll = gtk::ScrolledWindow::new();
        ex_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        ex_scroll.set_child(Some(&explorer));
        wb.add_side_view("explorer", "Explorer", &[], &ex_scroll);

        // ----- accounts view -----
        let account_list = gtk::ListBox::new();
        account_list.add_css_class("wb-tree");
        account_list.set_selection_mode(gtk::SelectionMode::None);
        let spinner = gtk::Spinner::new();
        let acc_menu = gio::Menu::new();
        acc_menu.append(Some("Auto-switch near limits"), Some("win.auto-switch"));
        acc_menu.append(Some("Refresh usage"), Some("win.refresh"));
        acc_menu.append(Some("Manage accounts…"), Some("win.accounts"));
        let acc_menu_btn = gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&acc_menu).tooltip_text("Accounts").build();
        acc_menu_btn.add_css_class("flat");
        acc_menu_btn.add_css_class("wb-section-tool");
        let acc_view = gtk::ScrolledWindow::new();
        acc_view.set_hscrollbar_policy(gtk::PolicyType::Never);
        acc_view.set_child(Some(&account_list));
        let refresh_btn = icon_btn("view-refresh-symbolic", "Refresh usage (Ctrl+Shift+R)", Some("win.refresh"));
        refresh_btn.add_css_class("wb-section-tool");
        wb.add_side_view("accounts", "Accounts", &[spinner.clone().upcast(), refresh_btn.upcast(), acc_menu_btn.upcast()], &acc_view);
        let side = crate::prefs::get("wb.side_view").filter(|s| s == "accounts").unwrap_or_else(|| "explorer".into());
        wb.pick_side_view(&side);

        // ----- panel tools -----
        let pt = &wb.panel_tools;
        pt.append(&workbench::strip_tool("list-add-symbolic", "New Terminal (Ctrl+Shift+T)", "win.new-tab"));
        pt.append(&workbench::strip_tool("starred-symbolic", "New Claude Session (Ctrl+Shift+K)", "win.new-claude"));
        wb.set_placeholder(EDITOR, &welcome());
        wb.set_placeholder(PANEL, &panel_empty());

        // ----- status bar -----
        let branch_label = label("", &[]);
        let branch_btn = status_btn(Some("media-playlist-shuffle-symbolic"), &branch_label);
        branch_btn.set_tooltip_text(Some("Git changes of this folder"));
        branch_btn.set_action_name(Some("win.changes"));
        wb.status_left.append(&branch_btn);
        let needs_label = label("", &["warn"]);
        let needs_btn = status_btn(Some("dialog-warning-symbolic"), &needs_label);
        needs_btn.add_css_class("warn");
        needs_btn.set_tooltip_text(Some("Agent sessions waiting for you"));
        needs_btn.set_action_name(Some("win.board"));
        needs_btn.set_visible(false);
        wb.status_left.append(&needs_btn);

        let auto_label = label("", &[]);
        let auto_btn = status_btn(Some("media-playlist-repeat-symbolic"), &auto_label);
        auto_btn.set_action_name(Some("win.auto-switch"));
        auto_btn.set_tooltip_text(Some("Move idle Claude sessions to another account near a limit"));
        let start_btn = gtk::Button::new();
        {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            b.append(&gtk::Image::from_icon_name("starred-symbolic"));
            b.append(&gtk::Label::new(Some("Start Claude")));
            start_btn.set_child(Some(&b));
            start_btn.set_tooltip_text(Some("Run claude in this terminal (Ctrl+Shift+K)"));
            start_btn.set_action_name(Some("win.start-claude"));
            start_btn.add_css_class("claude");
        }
        let account_label = gtk::Label::new(None);
        let account_btn = gtk::MenuButton::new();
        {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let i = gtk::Image::from_icon_name("starred-symbolic");
            i.add_css_class("claude");
            b.append(&i);
            b.append(&account_label);
            account_btn.set_child(Some(&b));
            account_btn.set_tooltip_text(Some("Account of this Claude session (click to move it)"));
            account_btn.set_direction(gtk::ArrowType::Up);
        }
        let account_menu = gtk::Box::new(gtk::Orientation::Vertical, 4);
        account_menu.set_margin_top(6);
        account_menu.set_margin_bottom(6);
        account_menu.set_margin_start(6);
        account_menu.set_margin_end(6);
        let pop = gtk::Popover::new();
        pop.set_child(Some(&account_menu));
        account_btn.set_popover(Some(&pop));
        wb.status_right.append(&start_btn);
        wb.status_right.append(&account_btn);
        wb.status_right.append(&auto_btn);

        // ----- pages opened as tabs -----
        let dashboard = Dashboard::new();
        let board = Board::new();
        let jira = crate::jira::JiraPage::new();

        let banner = adw::Banner::new("");
        banner.set_use_markup(true);
        wb.top_slot.append(&banner);

        let term_model = gio::Menu::new();
        term_model.append(Some("Copy"), Some("win.copy"));
        term_model.append(Some("Paste"), Some("win.paste"));
        term_model.append(Some("Select All"), Some("win.select-all"));
        term_model.append(Some("Find…"), Some("win.find"));
        let term_menu = gtk::PopoverMenu::from_model(Some(&term_model));
        term_menu.set_parent(&wb.root);
        term_menu.set_has_arrow(false);
        term_menu.set_halign(gtk::Align::Start);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&wb.root));
        window.set_content(Some(&toasts));

        let (accounts, config) = model::quick_state();
        let auto_action = gio::SimpleAction::new_stateful("auto-switch", None, &config.auto_switch.to_variant());

        let deck = Rc::new(Deck {
            app: app.clone(),
            window,
            toasts,
            wb,
            term_menu,
            banner,
            account_btn,
            account_label,
            account_menu,
            start_btn,
            branch_btn,
            branch_label,
            needs_btn,
            needs_label,
            auto_label,
            sessions,
            session_rows: RefCell::new(vec![]),
            account_list,
            spinner,
            dashboard,
            canvas,
            board,
            jira,
            files,
            files_title,
            canvas_note,
            canvas_fp: Cell::new(0),
            board_badge,
            accounts_badge,
            last_states: RefCell::new(HashMap::new()),
            socket,
            cli,
            _server: server,
            tabs: RefCell::new(vec![]),
            active_term: RefCell::new(None),
            accounts: RefCell::new(accounts),
            config: RefCell::new(config),
            picker: RefCell::new(None),
            pick_queue: RefCell::new(VecDeque::new()),
            last_refresh: Cell::new(None),
            refreshing: Cell::new(false),
            syncing: Cell::new(false),
            settings: RefCell::new(None),
            auto_action,
            banner_target: RefCell::new(None),
        });
        deck.dashboard.set_deck(Rc::downgrade(&deck));
        deck.wire_workbench();
        deck.wire_canvas();
        deck.connect_signals();
        deck.install_actions();
        deck.wb.install_actions(&deck.window);

        let d = deck.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                d.handle(msg);
            }
        });

        let d = deck.clone();
        glib::timeout_add_local(Duration::from_millis(900), move || {
            d.poll_canvas();
            glib::ControlFlow::Continue
        });
        let d = deck.clone();
        glib::timeout_add_seconds_local(2, move || {
            d.auto_switch_tick();
            glib::ControlFlow::Continue
        });
        let d = deck.clone();
        glib::timeout_add_seconds_local(30, move || {
            let poll = d.config.borrow().poll_secs.max(30);
            if d.last_refresh.get().is_none_or(|t| t.elapsed() >= Duration::from_secs(poll)) {
                d.refresh(true);
            }
            glib::ControlFlow::Continue
        });

        // `claude-deck add` runs in a terminal tab; show the account as soon as its token lands.
        let tokens = accounts::tokens_dir();
        let _ = std::fs::create_dir_all(&tokens);
        if let Ok(m) = gio::File::for_path(&tokens).monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            let d = Rc::downgrade(&deck);
            m.connect_changed(move |_, _, _, ev| {
                if matches!(ev, gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedIn | gio::FileMonitorEvent::Renamed) {
                    if let Some(d) = d.upgrade() {
                        d.refresh(false);
                    }
                }
            });
            std::mem::forget(m); // lives as long as the app
        }

        deck.rebuild_accounts();
        let panel_was_shown = deck.wb.panel_shown();
        deck.new_tab(None, None);
        if !panel_was_shown {
            // the first terminal must not overrule a panel the user had hidden
            deck.wb.set_panel_shown(false);
        }
        deck.refresh(true);
        if let Some(e) = server_err {
            deck.toast(&e);
        }
        if deck.cli.is_none() {
            deck.toast("claude-deck CLI not found: `claude` in tabs runs without the account picker");
        }
        deck
    }

    /// React to the workbench: closed / selected / moved tabs, tab menus.
    fn wire_workbench(self: &Rc<Self>) {
        let d = Rc::downgrade(self);
        *self.wb.on_closed.borrow_mut() = Some(Rc::new(move |key: &str| {
            let Some(d) = d.upgrade() else { return };
            if let Some(id) = term_id(key) {
                d.close_tab(id);
            } else if d.canvas.owns(key) {
                d.canvas.forget(key);
            }
        }));
        let d = Rc::downgrade(self);
        *self.wb.on_selected.borrow_mut() = Some(Rc::new(move |key: &str| {
            let Some(d) = d.upgrade() else { return };
            if let Some(id) = term_id(key) {
                let changed = d.active_term.borrow().as_deref() != Some(id);
                if changed {
                    *d.active_term.borrow_mut() = Some(id.to_string());
                    d.rebuild_sessions();
                }
                d.update_chrome();
                if let Some(t) = d.tab(id) {
                    if d.wb.focused_group() == d.wb.group_of(key).unwrap_or(EDITOR) {
                        let term = t.term.clone();
                        glib::idle_add_local_once(move || {
                            if !term.has_focus() {
                                term.grab_focus();
                            }
                        });
                    }
                }
            } else {
                d.canvas.set_current(Some(key));
                match key {
                    "jira" => d.jira.activate(),
                    "dashboard" => d.dashboard.load_if_stale(),
                    _ => {}
                }
                d.update_chrome();
            }
        }));
        let d = Rc::downgrade(self);
        *self.wb.on_moved.borrow_mut() = Some(Rc::new(move |key: &str, group: usize| {
            let Some(d) = d.upgrade() else { return };
            if let Some(t) = term_id(key).and_then(|id| d.tab(id)) {
                crate::term::apply_theme(&t.term, widgets::is_dark(), workbench::is_panel(group));
            }
        }));
        let d = Rc::downgrade(self);
        *self.wb.on_menu.borrow_mut() = Some(Rc::new(move |key: &str, menu: &gio::Menu| {
            let Some(d) = d.upgrade() else { return };
            if let Some(id) = term_id(key) {
                if d.tab(id).is_some_and(|t| t.claude.borrow().is_none()) {
                    menu.append(Some("Start Claude Here"), Some(&format!("win.tab-claude::{id}")));
                }
                menu.append(Some("New Terminal in This Folder"), Some(&format!("win.tab-new-here::{id}")));
                menu.append(Some("Git Changes"), Some(&format!("win.tab-changes::{id}")));
            }
        }));
        let d = Rc::downgrade(self);
        *self.wb.on_side.borrow_mut() = Some(Rc::new(move |id: &str| {
            if let Some(d) = d.upgrade() {
                if id == "explorer" {
                    d.follow_files();
                }
            }
        }));
        let d = Rc::downgrade(self);
        self.wb.command.connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.quick_open();
            }
        });
    }

    fn connect_signals(self: &Rc<Self>) {
        let d = self.clone();
        self.sessions.connect_row_activated(move |_, row| {
            if d.syncing.get() {
                return;
            }
            let id = d.tabs.borrow().get(row.index() as usize).map(|t| t.id.clone());
            if let Some(id) = id {
                d.select(Sel::Tab(id));
            }
        });
        // Right-click on a terminal row.
        let pop = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        pop.set_parent(&self.sessions);
        pop.set_has_arrow(false);
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let d = self.clone();
        click.connect_pressed(move |_, _, x, y| {
            let Some(row) = d.sessions.row_at_y(y as i32) else { return };
            let Some(id) = d.tabs.borrow().get(row.index() as usize).map(|t| t.id.clone()) else { return };
            let menu = gio::Menu::new();
            menu.append(Some("Start Claude Here"), Some(&format!("win.tab-claude::{id}")));
            menu.append(Some("New Terminal in This Folder"), Some(&format!("win.tab-new-here::{id}")));
            menu.append(Some("Git Changes"), Some(&format!("win.tab-changes::{id}")));
            let key = term_key(&id);
            let mv = gio::Menu::new();
            if d.wb.group_of(&key).is_some_and(workbench::is_panel) {
                mv.append(Some("Move to Editor Area"), Some(&format!("wb.move-editor::{key}")));
            } else {
                mv.append(Some("Move to Panel"), Some(&format!("wb.move-panel::{key}")));
            }
            menu.append_section(None, &mv);
            let m2 = gio::Menu::new();
            m2.append(Some("Close Terminal"), Some(&format!("win.tab-close::{id}")));
            menu.append_section(None, &m2);
            pop.set_menu_model(Some(&menu));
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        });
        self.sessions.add_controller(click);

        let d = self.clone();
        self.banner.connect_button_clicked(move |_| {
            let target = d.banner_target.borrow().clone();
            if let Some((tab_id, account)) = target {
                if let Some(t) = d.tab(&tab_id) {
                    d.move_tab(&t, &account, "Moved");
                }
            }
        });
        let d = self.clone();
        self.account_btn.popover().unwrap().connect_show(move |_| d.fill_account_menu());
        let d = self.clone();
        self.window.connect_close_request(move |_| {
            d.shutdown();
            glib::Propagation::Proceed
        });
        let d = self.clone();
        self.window.connect_is_active_notify(move |w| {
            if w.is_active() {
                d.files.rebuild();
            }
        });
        let d = Rc::downgrade(self);
        crate::theme::watch(move || {
            let Some(d) = d.upgrade() else { return false };
            let dark = widgets::is_dark();
            for t in d.tabs.borrow().iter() {
                let panel = d.wb.group_of(&term_key(&t.id)).is_none_or(workbench::is_panel);
                crate::term::apply_theme(&t.term, dark, panel);
            }
            d.rebuild_accounts();
            d.dashboard.redraw();
            true
        });
    }

    fn install_actions(self: &Rc<Self>) {
        let win = &self.window;
        let add = |name: &str, f: Box<dyn Fn(&Rc<Deck>)>| {
            let a = gio::SimpleAction::new(name, None);
            let d = self.clone();
            a.connect_activate(move |_, _| f(&d));
            win.add_action(&a);
        };
        add("new-tab", Box::new(|d| d.new_tab(None, None)));
        add("new-tab-home", Box::new(|d| d.new_tab(Some(home().to_string_lossy().to_string()), None)));
        add("new-claude", Box::new(|d| d.new_tab(None, Some("claude".into()))));
        add("canvas", Box::new(|d| d.select(Sel::Canvas)));
        add("explorer", Box::new(|d| d.wb.toggle_side_view("explorer")));
        add("accounts-view", Box::new(|d| d.wb.toggle_side_view("accounts")));
        add("board", Box::new(|d| d.select(Sel::Board)));
        add("jira", Box::new(|d| d.select(Sel::Jira)));
        add("dashboard", Box::new(|d| d.select(Sel::Dashboard)));
        add("quick-open", Box::new(|d| d.quick_open()));
        add("changes", Box::new(|d| {
            if let Some(t) = d.active_tab() {
                let dir = t.current_dir();
                d.canvas.open_changes(Path::new(&dir), &crate::model::last_component(&dir));
            }
        }));
        add("close-tab", Box::new(|d| {
            if let Some(k) = d.wb.current() {
                d.wb.close(&k);
            }
        }));
        add("start-claude", Box::new(|d| {
            match d.active_tab().filter(|t| t.claude.borrow().is_none()) {
                Some(t) => {
                    t.type_text("claude\r");
                    d.select(Sel::Tab(t.id.clone()));
                }
                None => d.new_tab(None, Some("claude".into())),
            }
        }));
        add("refresh", Box::new(|d| d.refresh(false)));
        add("next-tab", Box::new(|d| d.cycle(1)));
        add("prev-tab", Box::new(|d| d.cycle(-1)));
        // Copy/paste follow the focus: a terminal, else the focused text (file view, note, label).
        add("copy", Box::new(|d| match d.focused_tab() {
            Some(t) => t.term.copy_clipboard_format(vte::Format::Text),
            None => d.focus_action("clipboard.copy"),
        }));
        add("paste", Box::new(|d| match d.focused_tab() {
            Some(t) => {
                t.last_input.set(Instant::now());
                vte::prelude::TerminalExt::paste_clipboard(&t.term);
            }
            None => d.focus_action("clipboard.paste"),
        }));
        add("select-all", Box::new(|d| match d.focused_tab() {
            Some(t) => t.term.select_all(),
            None => d.focus_action("selection.select-all"),
        }));
        add("find", Box::new(|d| {
            if let Some(t) = d.focused_tab().or_else(|| d.active_tab()) {
                t.find();
            }
        }));
        add("zoom-in", Box::new(|d| d.zoom(Some(1.1))));
        add("zoom-out", Box::new(|d| d.zoom(Some(1.0 / 1.1))));
        add("zoom-reset", Box::new(|d| d.zoom(None)));
        add("toggle-sidebar", Box::new(|d| d.wb.toggle_side()));
        add("toggle-panel", Box::new(|d| d.wb.toggle_panel()));
        add("settings", Box::new(|d| Settings::show(d, None)));
        add("accounts", Box::new(|d| Settings::show(d, Some("accounts"))));
        add("about", Box::new(|d| {
            let about = adw::AboutDialog::builder()
                .application_name("CC Deck")
                .application_icon(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("Claude Code sessions on the account you pick, with their canvas beside them.")
                .license_type(gtk::License::MitX11)
                .build();
            about.present(Some(&d.window));
        }));
        add("shortcuts", Box::new(|d| crate::show_shortcuts(&d.window)));

        let d = self.clone();
        self.auto_action.connect_activate(move |a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            d.update_config(|c| c.auto_switch = on);
        });
        win.add_action(&self.auto_action);

        let sel = gio::SimpleAction::new("select-tab", Some(glib::VariantTy::INT32));
        let d = self.clone();
        sel.connect_activate(move |_, p| {
            if let Some(i) = p.and_then(|v| v.get::<i32>()) {
                d.select_index(i as usize - 1);
            }
        });
        win.add_action(&sel);

        for (name, f) in [("tab-claude", 0u8), ("tab-new-here", 1), ("tab-close", 2), ("tab-changes", 3)] {
            let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let d = self.clone();
            a.connect_activate(move |_, p| {
                let Some(id) = p.and_then(|v| v.get::<String>()) else { return };
                let Some(t) = d.tab(&id) else { return };
                match f {
                    0 => {
                        d.select(Sel::Tab(id));
                        t.type_text("claude\r");
                    }
                    1 => d.new_tab(Some(t.current_dir()), None),
                    3 => {
                        let dir = t.current_dir();
                        d.canvas.open_changes(Path::new(&dir), &crate::model::last_component(&dir));
                    }
                    _ => d.close_tab(&id),
                }
            });
            win.add_action(&a);
        }

        let app = &self.app;
        let accels: &[(&str, &[&str])] = &[
            ("win.new-tab", &["<Ctrl><Shift>t"]),
            ("win.new-tab-home", &["<Ctrl><Shift><Alt>t"]),
            ("win.close-tab", &["<Ctrl><Shift>w"]),
            ("win.start-claude", &["<Ctrl><Shift>k"]),
            ("win.dashboard", &["<Ctrl><Shift>d"]),
            ("win.explorer", &["<Ctrl><Shift>e"]),
            ("win.board", &["<Ctrl><Shift>o"]),
            ("win.jira", &["<Ctrl><Shift>j"]),
            ("win.quick-open", &["<Ctrl><Shift>p"]),
            ("win.refresh", &["<Ctrl><Shift>r"]),
            ("win.next-tab", &["<Ctrl>Page_Down", "<Ctrl><Shift>bracketright"]),
            ("win.prev-tab", &["<Ctrl>Page_Up", "<Ctrl><Shift>bracketleft"]),
            ("win.copy", &["<Ctrl><Shift>c"]),
            ("win.find", &["<Ctrl><Shift>f"]),
            ("win.paste", &["<Ctrl><Shift>v"]),
            ("win.toggle-sidebar", &["F9", "<Ctrl><Shift>b"]),
            ("win.toggle-panel", &["<Ctrl>grave"]),
            ("win.settings", &["<Ctrl>comma"]),
            ("win.shortcuts", &["<Ctrl>question"]),
            ("app.quit", &["<Ctrl><Shift>q"]),
        ];
        for (a, keys) in accels {
            app.set_accels_for_action(a, keys);
        }
        for i in 1..=9 {
            app.set_accels_for_action(&format!("win.select-tab({i})"), &[&format!("<Alt>{i}")]);
        }
    }

    // ---------- tabs ----------

    pub fn tab(&self, id: &str) -> Option<Rc<Tab>> {
        self.tabs.borrow().iter().find(|t| t.id == id).cloned()
    }

    /// The terminal that has keyboard focus, if any.
    fn focused_tab(&self) -> Option<Rc<Tab>> {
        let mut w = gtk::prelude::GtkWindowExt::focus(&self.window);
        while let Some(x) = w {
            if let Ok(term) = x.clone().downcast::<vte::Terminal>() {
                return self.tabs.borrow().iter().find(|t| t.term == term).cloned();
            }
            w = x.parent();
        }
        None
    }

    fn focus_action(&self, name: &str) {
        if let Some(w) = gtk::prelude::GtkWindowExt::focus(&self.window) {
            let _ = w.activate_action(name, None);
        }
    }

    /// The terminal last selected (in any group).
    pub fn active_tab(&self) -> Option<Rc<Tab>> {
        let id = self.active_term.borrow().clone();
        id.and_then(|id| self.tab(&id))
    }

    pub fn new_tab(self: &Rc<Self>, cwd: Option<String>, run: Option<String>) {
        let dir = cwd
            .or_else(|| self.active_tab().map(|t| t.cwd.borrow().clone()))
            .filter(|d| Path::new(d).is_dir())
            .unwrap_or_else(|| home().to_string_lossy().to_string());
        let tab = Tab::new(&dir, &self.socket, self.cli.as_deref());
        let id = tab.id.clone();

        let d = Rc::downgrade(self);
        let tid = id.clone();
        tab.term.connect_child_exited(move |_, status| {
            let Some(d) = d.upgrade() else { return };
            let Some(t) = d.tab(&tid) else { return };
            t.exited.set(true);
            // A clean exit closes the tab; otherwise keep it so the error stays readable.
            if status == 0 {
                let tid = tid.clone();
                glib::idle_add_local_once(move || d.close_tab(&tid));
                return;
            }
            let why = if libc::WIFEXITED(status) {
                format!("exited with code {}", libc::WEXITSTATUS(status))
            } else if libc::WIFSIGNALED(status) {
                format!("was stopped by signal {}", libc::WTERMSIG(status))
            } else {
                format!("ended (status {status})")
            };
            t.term.feed(format!("\r\n\x1b[2m[the shell {why} — press Enter to close this tab]\x1b[0m\r\n").as_bytes());
        });
        let d = Rc::downgrade(self);
        let t = Rc::downgrade(&tab);
        tab.term.connect_window_title_changed(move |_| {
            if let (Some(d), Some(t)) = (d.upgrade(), t.upgrade()) {
                if t.sync_title() {
                    d.tab_changed();
                }
            }
        });
        let d = Rc::downgrade(self);
        let t = Rc::downgrade(&tab);
        tab.term.connect_current_directory_uri_changed(move |_| {
            if let (Some(d), Some(t)) = (d.upgrade(), t.upgrade()) {
                if t.sync_cwd() {
                    d.tab_changed();
                }
            }
        });
        // Right-click: copy / paste.
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let d = Rc::downgrade(self);
        click.connect_pressed(move |g, _, x, y| {
            let Some(d) = d.upgrade() else { return };
            let Some(w) = g.widget() else { return };
            // The menu acts on the focused terminal: make that the one clicked.
            w.grab_focus();
            let p = w
                .compute_point(&d.wb.root, &gtk::graphene::Point::new(x as f32, y as f32))
                .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
            d.term_menu.set_pointing_to(Some(&gdk::Rectangle::new(p.x() as i32, p.y() as i32, 1, 1)));
            d.term_menu.popup();
        });
        tab.term.add_controller(click);

        // A new terminal joins the focused group when that group shows a terminal, else the panel.
        let focused = self.wb.focused_group();
        let group = if self.wb.active_in(focused).is_some_and(|k| k.starts_with("term:")) { focused } else { PANEL };
        self.tabs.borrow_mut().push(tab.clone());
        *self.active_term.borrow_mut() = Some(id.clone());
        crate::term::apply_theme(&tab.term, widgets::is_dark(), workbench::is_panel(group));
        self.wb.open(
            Spec {
                key: term_key(&id),
                title: tab.display_title(),
                icon: "utilities-terminal-symbolic".into(),
                tooltip: tab.subtitle(),
                crumb: String::new(),
                widget: tab.page.clone().upcast(),
            },
            group,
            true,
        );
        self.tab_changed();
        if let Some(cmd) = run {
            let t = tab.clone();
            glib::timeout_add_local_once(Duration::from_millis(400), move || t.type_text(&format!("{cmd}\r")));
        }
    }

    pub fn close_tab(self: &Rc<Self>, id: &str) {
        let idx = self.tabs.borrow().iter().position(|t| t.id == id);
        let Some(i) = idx else { return };
        let tab = self.tabs.borrow_mut().remove(i);
        tab.terminate();
        self.wb.remove(&term_key(id));
        if self.active_term.borrow().as_deref() == Some(id) {
            let next = self
                .wb
                .active_in(PANEL)
                .and_then(|k| term_id(&k).map(String::from))
                .or_else(|| self.tabs.borrow().first().map(|t| t.id.clone()));
            *self.active_term.borrow_mut() = next;
        }
        self.tab_changed();
        self.rebuild_accounts();
    }

    fn select_index(self: &Rc<Self>, i: usize) {
        let id = self.tabs.borrow().get(i).map(|t| t.id.clone());
        if let Some(id) = id {
            self.select(Sel::Tab(id));
        }
    }

    fn cycle(self: &Rc<Self>, delta: i32) {
        let n = self.tabs.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let cur = self
            .active_tab()
            .and_then(|t| self.tabs.borrow().iter().position(|x| x.id == t.id))
            .unwrap_or(0) as i32;
        self.select_index(((cur + delta + n) % n) as usize);
    }

    /// One zoom level for all terminals, so a new tab opens at the same size.
    fn zoom(&self, factor: Option<f64>) {
        let s = factor.map(|f| (crate::theme::term_scale() * f).clamp(0.5, 3.0)).unwrap_or(1.0);
        crate::theme::set_term_scale(s);
        for t in self.tabs.borrow().iter() {
            t.term.set_font_scale(s);
        }
    }

    /// Open (or reveal) Board, Jira or the Dashboard as an editor tab.
    fn open_page(self: &Rc<Self>, key: &str) {
        if self.wb.has(key) {
            self.wb.activate(key, true);
            return;
        }
        let (title, icon, widget): (&str, &str, gtk::Widget) = match key {
            "board" => ("Board", "checkbox-checked-symbolic", workbench::contain(&self.board.root)),
            "jira" => ("Jira", "network-server-symbolic", workbench::contain(&self.jira.root)),
            _ => ("Dashboard", "power-profile-performance-symbolic", workbench::contain(&self.dashboard.root)),
        };
        self.wb.open(
            Spec { key: key.into(), title: title.into(), icon: icon.into(), tooltip: title.into(), crumb: String::new(), widget },
            self.editor_group(),
            true,
        );
    }

    /// The editor group new editor tabs go to: the focused one, else the left one.
    fn editor_group(&self) -> usize {
        let g = self.wb.focused_group();
        if workbench::is_panel(g) {
            EDITOR
        } else {
            g
        }
    }

    pub fn select(self: &Rc<Self>, sel: Sel) {
        match sel {
            Sel::Tab(id) => {
                *self.active_term.borrow_mut() = Some(id.clone());
                self.wb.activate(&term_key(&id), true);
                if let Some(t) = self.tab(&id) {
                    let term = t.term.clone();
                    glib::idle_add_local_once(move || {
                        term.grab_focus();
                    });
                }
                self.rebuild_sessions();
            }
            Sel::Canvas => {
                self.wb.show_side_view("explorer");
                if let Some(k) = self.wb.keys().into_iter().find(|k| k.starts_with("screen:")) {
                    self.wb.activate(&k, true);
                }
            }
            Sel::Board => self.open_page("board"),
            Sel::Jira => self.open_page("jira"),
            Sel::Dashboard => self.open_page("dashboard"),
        }
        self.update_chrome();
    }

    fn tab_changed(self: &Rc<Self>) {
        for t in self.tabs.borrow().iter() {
            let claude = t.claude.borrow().is_some();
            let key = term_key(&t.id);
            let title = match t.claude.borrow().as_ref() {
                Some(r) => format!("{} · {}", t.display_title(), r.account),
                None => t.display_title(),
            };
            self.wb.set_title(&key, &title, Some(&format!("{}\n{}", t.display_title(), t.subtitle())));
            self.wb.set_icon(&key, if claude { "starred-symbolic" } else { "utilities-terminal-symbolic" }, claude);
        }
        self.rebuild_sessions();
        self.update_chrome();
        self.follow_files();
    }

    /// The Files section follows the active terminal's folder.
    fn follow_files(&self) {
        let Some(t) = self.active_tab() else { return };
        let dir = PathBuf::from(t.current_dir());
        if dir.is_dir() {
            self.files.set_root(&dir);
            let name = crate::model::last_component(&dir.to_string_lossy());
            self.files_title.set_text(&name.to_uppercase());
        }
    }

    /// A searchable list of everything open; Enter shows it.
    fn quick_open(self: &Rc<Self>) {
        let pop = gtk::Popover::new();
        pop.set_parent(&self.wb.command);
        pop.set_has_arrow(false);
        pop.set_position(gtk::PositionType::Bottom);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        v.set_size_request(460, -1);
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Go to an open tab, terminal or session"));
        v.append(&entry);
        let list = gtk::ListBox::new();
        list.add_css_class("wb-tree");
        let sc = gtk::ScrolledWindow::new();
        sc.set_hscrollbar_policy(gtk::PolicyType::Never);
        sc.set_max_content_height(380);
        sc.set_propagate_natural_height(true);
        sc.set_child(Some(&list));
        v.append(&sc);
        pop.set_child(Some(&v));

        let mut entries: Vec<(String, String, String)> = vec![];
        for t in self.tabs.borrow().iter() {
            entries.push((term_key(&t.id), t.display_title(), t.subtitle()));
        }
        for k in self.wb.keys().into_iter().filter(|k| !k.starts_with("term:")) {
            let title = k.rsplit(['/', ':']).next().unwrap_or(&k).to_string();
            entries.push((k.clone(), title, k.split(':').next().unwrap_or("").to_string()));
        }
        for (k, page) in [("board", "Board"), ("jira", "Jira"), ("dashboard", "Dashboard")] {
            if !entries.iter().any(|e| e.0 == k) {
                entries.push((k.into(), page.into(), "page".into()));
            }
        }
        let keys: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let fill = {
            let list = list.clone();
            let keys = keys.clone();
            move |q: &str| {
                list.remove_all();
                keys.borrow_mut().clear();
                let q = q.to_lowercase();
                for (k, title, sub) in &entries {
                    if !q.is_empty() && !title.to_lowercase().contains(&q) && !sub.to_lowercase().contains(&q) {
                        continue;
                    }
                    let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    h.set_margin_start(8);
                    let t = label(title, &[]);
                    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    h.append(&t);
                    let s = label(sub, &["meta"]);
                    s.set_ellipsize(gtk::pango::EllipsizeMode::Start);
                    s.set_hexpand(true);
                    h.append(&s);
                    list.append(&h);
                    keys.borrow_mut().push(k.clone());
                }
                if let Some(r) = list.row_at_index(0) {
                    list.select_row(Some(&r));
                }
            }
        };
        fill("");
        let go = {
            let d = Rc::downgrade(self);
            let keys = keys.clone();
            let pop = pop.clone();
            move |i: i32| {
                let Some(d) = d.upgrade() else { return };
                let Some(k) = keys.borrow().get(i.max(0) as usize).cloned() else { return };
                pop.popdown();
                match k.as_str() {
                    "board" => d.select(Sel::Board),
                    "jira" => d.select(Sel::Jira),
                    "dashboard" => d.select(Sel::Dashboard),
                    _ => match term_id(&k) {
                        Some(id) => d.select(Sel::Tab(id.to_string())),
                        None => d.wb.activate(&k, true),
                    },
                }
            }
        };
        entry.connect_search_changed(move |e| fill(&e.text()));
        let go2 = go.clone();
        let l2 = list.clone();
        entry.connect_activate(move |_| go2(l2.selected_row().map(|r| r.index()).unwrap_or(0)));
        list.connect_row_activated(move |_, r| go(r.index()));
        let l3 = list.clone();
        let keyc = gtk::EventControllerKey::new();
        keyc.connect_key_pressed(move |_, k, _, _| {
            let cur = l3.selected_row().map(|r| r.index()).unwrap_or(0);
            let next = match k {
                gdk::Key::Down => cur + 1,
                gdk::Key::Up => (cur - 1).max(0),
                _ => return glib::Propagation::Proceed,
            };
            if let Some(r) = l3.row_at_index(next) {
                l3.select_row(Some(&r));
            }
            glib::Propagation::Stop
        });
        entry.add_controller(keyc);
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
        entry.grab_focus();
    }

    fn rebuild_sessions(self: &Rc<Self>) {
        self.syncing.set(true);
        for r in self.session_rows.borrow_mut().drain(..) {
            self.sessions.remove(&r);
        }
        let active = self.active_term.borrow().clone();
        for tab in self.tabs.borrow().iter() {
            let row = gtk::ListBoxRow::new();
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            b.set_margin_start(22);
            let running = tab.claude.borrow().is_some();
            let icon = gtk::Image::from_icon_name(if running { "starred-symbolic" } else { "utilities-terminal-symbolic" });
            icon.add_css_class(if running { "claude-icon" } else { "dim" });
            b.append(&icon);
            let t = label(&tab.display_title(), &[]);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            b.append(&t);
            let agent = self.agent_session(&tab.id);
            let state = agent.as_ref().map(crate::canvas::state_text).unwrap_or_default();
            let mut sub = tab.subtitle();
            if running {
                if let Some(r) = tab.claude.borrow().as_ref() {
                    sub = r.account.clone();
                }
                if !state.is_empty() {
                    sub = format!("{sub} · {state}");
                }
            }
            let needs = agent.as_ref().is_some_and(|a| a.needs_you()) && running;
            let s = label(&sub, &[if needs { "warn" } else { "meta" }]);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            s.set_hexpand(true);
            b.append(&s);
            row.set_child(Some(&b));
            row.set_tooltip_text(Some(&format!("{}\n{}", tab.display_title(), tab.current_dir())));
            self.sessions.append(&row);
            self.session_rows.borrow_mut().push(row.clone());
            if active.as_deref() == Some(&tab.id) {
                self.sessions.select_row(Some(&row));
            }
        }
        self.syncing.set(false);
    }

    fn rebuild_accounts(&self) {
        self.account_list.remove_all();
        let cfg = self.config.borrow();
        let in_use: Vec<String> =
            self.tabs.borrow().iter().filter_map(|t| t.claude.borrow().as_ref().map(|c| c.account.clone())).collect();
        let mut over = 0;
        for acc in self.accounts.borrow().iter() {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.add_css_class("wb-account");
            b.append(&widgets::rings(acc.five_pct(), acc.week_pct(), cfg.threshold, cfg.weekly_threshold, 28, 3.5));
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            h.append(&label(&acc.name, &["name"]));
            if let Some(p) = acc.plan_label() {
                h.append(&widgets::badge(&p));
            }
            if in_use.contains(&acc.name) {
                let i = gtk::Image::from_icon_name("starred-symbolic");
                i.add_css_class("claude-icon");
                i.set_pixel_size(12);
                i.set_tooltip_text(Some("A Claude session is running on this account"));
                h.append(&i);
            }
            if acc.disabled {
                h.append(&widgets::badge("auto off"));
            }
            v.append(&h);
            let s = label(&acc.summary(), &["meta", "numeric"]);
            if acc.over(&cfg) {
                s.add_css_class("over");
                over += 1;
            }
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&s);
            if let Some(e) = &acc.email {
                let el = label(e, &["meta"]);
                el.set_ellipsize(gtk::pango::EllipsizeMode::End);
                v.append(&el);
            }
            b.append(&v);
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            row.set_child(Some(&b));
            self.account_list.append(&row);
        }
        self.accounts_badge.set_text(&over.to_string());
        self.accounts_badge.set_visible(over > 0);
        self.accounts_badge.set_tooltip_text(Some("Accounts near a usage limit"));
        self.auto_label.set_text(if cfg.auto_switch { "Auto-switch on" } else { "Auto-switch off" });
    }

    fn update_chrome(&self) {
        *self.banner_target.borrow_mut() = None;
        self.banner.set_revealed(false);
        let tab = self.active_tab();
        // title bar: where the current item lives
        let current = self.wb.current();
        let cmd = match (&current, &tab) {
            (Some(k), Some(t)) if k.starts_with("term:") => {
                let place = crate::model::short_path(&t.current_dir());
                match t.claude.borrow().as_ref() {
                    Some(r) => format!("{place}  —  Claude · {}", r.account),
                    None => place,
                }
            }
            (Some(k), _) if k.starts_with("screen:") => format!("canvas  —  {}", k.rsplit('/').next().unwrap_or(k)),
            (Some(k), _) if k.starts_with("file:") => crate::model::short_path(k.trim_start_matches("file:")),
            (Some(k), _) if k.starts_with("changes:") => format!("changes  —  {}", crate::model::short_path(k.trim_start_matches("changes:"))),
            (Some(k), _) => match k.as_str() {
                "board" => "Board".into(),
                "jira" => "Jira".into(),
                "dashboard" => "Dashboard".into(),
                _ => k.clone(),
            },
            (None, Some(t)) => crate::model::short_path(&t.current_dir()),
            (None, None) => "CC Deck".into(),
        };
        self.wb.set_command_text(&cmd);
        self.window.set_title(Some(&format!("{cmd} — CC Deck")));

        let Some(tab) = tab else {
            self.branch_btn.set_visible(false);
            self.account_btn.set_visible(false);
            self.start_btn.set_visible(true);
            return;
        };
        let dir = tab.current_dir();
        match git_branch(Path::new(&dir)) {
            Some(b) => {
                self.branch_label.set_text(&b);
                self.branch_btn.set_visible(true);
                self.branch_btn.set_tooltip_text(Some(&format!("{}\nClick for the Git changes of this folder", crate::model::short_path(&dir))));
            }
            None => self.branch_btn.set_visible(false),
        }
        let run = tab.claude.borrow().clone();
        match run {
            Some(run) => {
                let accs = self.accounts.borrow();
                let cfg = self.config.borrow();
                let acc = accs.iter().find(|a| a.name == run.account);
                let txt = match (acc.and_then(|a| a.five_pct()), acc.and_then(|a| a.week_pct())) {
                    (Some(f), Some(w)) => format!("{} · 5h {:.0}% · 7d {:.0}%", run.account, f, w),
                    (Some(f), None) => format!("{} · 5h {:.0}%", run.account, f),
                    _ => run.account.clone(),
                };
                self.account_label.set_text(&txt);
                if acc.is_some_and(|a| a.over(&cfg)) {
                    self.account_label.add_css_class("over");
                } else {
                    self.account_label.remove_css_class("over");
                }
                self.account_btn.set_visible(true);
                self.start_btn.set_visible(false);
                if let Some(acc) = acc {
                    if acc.over(&cfg) && !cfg.auto_switch {
                        if let Some(target) = model::best(&accs, &cfg, Some(&acc.name)) {
                            self.banner.set_title(&format!("<b>{}</b> is near its usage limit.", glib::markup_escape_text(&acc.name)));
                            self.banner.set_button_label(Some(&format!("Move to {}", target.name)));
                            self.banner.set_revealed(true);
                            *self.banner_target.borrow_mut() = Some((tab.id.clone(), target.name.clone()));
                        }
                    }
                }
            }
            None => {
                self.account_btn.set_visible(false);
                self.start_btn.set_visible(true);
            }
        }
    }

    fn fill_account_menu(self: &Rc<Self>) {
        let m = &self.account_menu;
        while let Some(c) = m.first_child() {
            m.remove(&c);
        }
        let Some(tab) = self.active_tab() else { return };
        let Some(run) = tab.claude.borrow().clone() else { return };
        let head = label("Run this conversation on", &["caption-heading", "dim-label"]);
        head.set_margin_start(8);
        head.set_margin_bottom(4);
        m.append(&head);
        let cfg = self.config.borrow().clone();
        for acc in self.accounts.borrow().iter() {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            h.append(&widgets::rings(acc.five_pct(), acc.week_pct(), cfg.threshold, cfg.weekly_threshold, 24, 3.0));
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            v.append(&label(&acc.name, &["heading"]));
            let s = label(&acc.summary(), &["caption", "dim-label", "numeric"]);
            v.append(&s);
            h.append(&v);
            if acc.name == run.account {
                let i = gtk::Image::from_icon_name("object-select-symbolic");
                i.set_hexpand(true);
                i.set_halign(gtk::Align::End);
                h.append(&i);
            }
            b.set_child(Some(&h));
            b.set_sensitive(acc.name != run.account && !acc.logged_out());
            let d = self.clone();
            let t = tab.clone();
            let name = acc.name.clone();
            b.connect_clicked(move |_| {
                if let Some(p) = d.account_btn.popover() {
                    p.popdown();
                }
                d.move_tab(&t, &name, "Moved");
            });
            m.append(&b);
        }
        let foot = label("Moving restarts claude and resumes this chat.", &["caption", "dim-label"]);
        foot.set_margin_start(8);
        foot.set_margin_top(4);
        m.append(&foot);
    }

    // ---------- usage & settings ----------

    pub fn refresh(self: &Rc<Self>, record: bool) {
        if self.refreshing.get() {
            return;
        }
        self.refreshing.set(true);
        self.spinner.start();
        let d = self.clone();
        glib::spawn_future_local(async move {
            let res = gio::spawn_blocking(move || model::fetch_state(record)).await;
            d.refreshing.set(false);
            d.spinner.stop();
            match res {
                Ok(Ok((mut accs, cfg))) => {
                    // A transient error (rate limit, network) keeps the last good reading.
                    {
                        let old = d.accounts.borrow();
                        for a in accs.iter_mut() {
                            if matches!(a.usage, claude_deck::usage::Usage::Error(_)) {
                                if let Some(prev) = old.iter().find(|o| o.name == a.name && o.is_ok()) {
                                    a.usage = prev.usage.clone();
                                }
                            }
                        }
                    }
                    d.auto_action.set_state(&cfg.auto_switch.to_variant());
                    *d.accounts.borrow_mut() = accs;
                    *d.config.borrow_mut() = cfg;
                    d.last_refresh.set(Some(Instant::now()));
                    d.after_state_change();
                }
                Ok(Err(e)) => d.toast(&format!("Couldn't read accounts: {e}")),
                Err(_) => d.toast("Couldn't read accounts"),
            }
        });
    }

    fn after_state_change(self: &Rc<Self>) {
        self.rebuild_accounts();
        self.update_chrome();
        if let Some(p) = self.picker.borrow().clone() {
            p.rebuild();
        }
        if let Some(s) = self.settings.borrow().clone() {
            s.reload();
        }
        self.dashboard.redraw();
    }

    /// Change settings, save them, and update the UI.
    pub fn update_config(self: &Rc<Self>, f: impl FnOnce(&mut Config)) {
        let res = {
            let mut c = self.config.borrow_mut();
            f(&mut c);
            c.save()
        };
        if let Err(e) = res {
            self.toast(&format!("Couldn't save settings: {e:#}"));
        }
        self.auto_action.set_state(&self.config.borrow().auto_switch.to_variant());
        self.rebuild_accounts();
        self.update_chrome();
    }

    pub fn toast(&self, text: &str) {
        let t = adw::Toast::new(text);
        t.set_timeout(6);
        self.toasts.add_toast(t);
    }

    fn notify(&self, title: &str, body: &str) {
        if self.window.is_active() {
            return;
        }
        let n = gio::Notification::new(title);
        n.set_body(Some(body));
        self.app.send_notification(None, &n);
    }

    pub fn account_names(&self) -> Vec<String> {
        self.accounts.borrow().iter().map(|a| a.name.clone()).collect()
    }

    pub fn set_disabled(self: &Rc<Self>, name: &str, disabled: bool) {
        let name = name.to_string();
        self.update_config(|c| {
            c.disabled.retain(|a| a != &name);
            if disabled {
                c.disabled.push(name.clone());
            }
        });
        let dis = self.config.borrow().disabled.clone();
        for a in self.accounts.borrow_mut().iter_mut() {
            a.disabled = dis.contains(&a.name);
        }
        self.after_state_change();
    }

    /// Check the name; the account itself appears once `login` has stored its token.
    pub fn add_account(self: &Rc<Self>, name: &str) -> Result<(), String> {
        accounts::valid_name(name).map_err(|e| format!("{e:#}"))?;
        if name == accounts::DEFAULT || self.accounts.borrow().iter().any(|a| a.name == name) {
            return Err(format!("{name} already exists"));
        }
        Ok(())
    }

    pub fn remove_account(self: &Rc<Self>, name: &str) {
        if let Err(e) = accounts::remove(name) {
            self.toast(&format!("{e:#}"));
            return;
        }
        let n = name.to_string();
        self.update_config(|c| {
            c.maps.retain(|_, a| a != &n);
            c.disabled.retain(|a| a != &n);
        });
        self.accounts.borrow_mut().retain(|a| a.name != name);
        self.after_state_change();
        self.toast(&format!("Removed {name}"));
    }

    /// Open a tab that signs `name` in: /login for `default`, a new token for the others.
    pub fn login(self: &Rc<Self>, name: &str) {
        let bin = if self.cli.is_some() { "\"$CC_DECK_BIN\"" } else { "claude-deck" };
        let (cmd, hint) = if name == accounts::DEFAULT {
            ("env -u CLAUDE_CONFIG_DIR -u CLAUDE_CODE_OAUTH_TOKEN claude".to_string(), format!("Log in to {name} with /login, then keep working or /exit"))
        } else {
            (format!("{bin} add {name}"), format!("Sign in to {name} in the browser, then paste the token"))
        };
        self.new_tab(Some(home().to_string_lossy().to_string()), Some(cmd));
        self.window.present();
        self.toast(&hint);
    }

    pub fn map_dir(self: &Rc<Self>, account: &str, dir: &Path) {
        let d = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let a = account.to_string();
        self.update_config(|c| {
            c.maps.insert(d.to_string_lossy().to_string(), a);
        });
        if let Some(s) = self.settings.borrow().clone() {
            s.reload();
        }
    }

    pub fn unmap_dir(self: &Rc<Self>, dir: &str) {
        let d = dir.to_string();
        self.update_config(|c| {
            c.maps.remove(&d);
        });
        if let Some(s) = self.settings.borrow().clone() {
            s.reload();
        }
    }

    // ---------- launch protocol ----------

    fn tab_for_conn(&self, conn: &Arc<Conn>) -> Option<Rc<Tab>> {
        self.tabs
            .borrow()
            .iter()
            .find(|t| t.conn.borrow().as_ref().is_some_and(|c| c.id == conn.id))
            .cloned()
    }

    fn handle(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::Message(conn, v) => self.handle_message(conn, v),
            Msg::Closed(conn) => {
                if let Some(t) = self.tab_for_conn(&conn) {
                    *t.claude.borrow_mut() = None;
                    *t.conn.borrow_mut() = None;
                    self.tab_changed();
                    self.rebuild_accounts();
                }
                let current = self.picker.borrow().clone();
                if let Some(p) = current {
                    if p.req.conn.id == conn.id {
                        p.dismiss();
                    }
                }
                self.pick_queue.borrow_mut().retain(|r| r.conn.id != conn.id);
            }
        }
    }

    fn handle_message(self: &Rc<Self>, conn: Arc<Conn>, v: Value) {
        match v.get("type").and_then(|t| t.as_str()) {
            Some("hello") => {
                let tab_id = v.get("tab").and_then(|t| t.as_str()).unwrap_or_default().to_string();
                let tab = self.tab(&tab_id);
                if let Some(t) = &tab {
                    *t.conn.borrow_mut() = Some(conn.clone());
                }
                if v.get("pick").and_then(|p| p.as_bool()) == Some(true) {
                    let cwd = v
                        .get("cwd")
                        .and_then(|c| c.as_str())
                        .map(String::from)
                        .or_else(|| tab.as_ref().map(|t| t.cwd.borrow().clone()))
                        .unwrap_or_else(|| home().to_string_lossy().to_string());
                    if let Some(t) = &tab {
                        self.select(Sel::Tab(t.id.clone()));
                    }
                    let req = PickReq { tab_id: tab.map(|t| t.id.clone()), cwd, conn };
                    self.show_pick(req);
                    self.window.present();
                    self.refresh(false);
                }
            }
            Some("started") => {
                let Some(tab) = self.tab_for_conn(&conn) else { return };
                let Some(account) = v.get("account").and_then(|a| a.as_str()) else { return };
                *tab.claude.borrow_mut() = Some(ClaudeRun {
                    account: account.to_string(),
                    session_id: v.get("session_id").and_then(|s| s.as_str()).map(String::from),
                    cwd: v.get("cwd").and_then(|c| c.as_str()).map(String::from).unwrap_or_else(|| tab.cwd.borrow().clone()),
                });
                tab.warned.set(false);
                self.tab_changed();
                self.rebuild_accounts();
                self.refresh(false);
            }
            Some("exited") => {
                if let Some(tab) = self.tab_for_conn(&conn) {
                    *tab.claude.borrow_mut() = None;
                    self.tab_changed();
                    self.rebuild_accounts();
                }
            }
            _ => {}
        }
    }

    fn show_pick(self: &Rc<Self>, req: PickReq) {
        if self.picker.borrow().is_some() {
            self.pick_queue.borrow_mut().push_back(req);
            return;
        }
        let p = Picker::show(self, req);
        *self.picker.borrow_mut() = Some(p);
    }

    pub fn answer_pick(self: &Rc<Self>, req: &PickReq, account: Option<&str>, remember: bool) {
        match account {
            Some(a) => {
                req.conn.send(json!({"account": a}));
                if remember && self.config.borrow().account_for(Path::new(&req.cwd)).as_deref() != Some(a) {
                    self.map_dir(a, Path::new(&req.cwd));
                }
            }
            None => req.conn.send(json!({"cancel": true})),
        }
        self.finish_pick();
        if let Some(t) = req.tab_id.as_deref().and_then(|id| self.tab(id)) {
            let term = t.term.clone();
            glib::idle_add_local_once(move || {
                term.grab_focus();
            });
        }
    }

    pub fn finish_pick(self: &Rc<Self>) {
        *self.picker.borrow_mut() = None;
        let next = self.pick_queue.borrow_mut().pop_front();
        if let Some(next) = next {
            let d = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || d.show_pick(next));
        }
    }

    /// Restart claude in `tab` on another account, resuming the same conversation.
    pub fn move_tab(self: &Rc<Self>, tab: &Rc<Tab>, account: &str, reason: &str) {
        let Some(run) = tab.claude.borrow().clone() else { return };
        let Some(conn) = tab.conn.borrow().clone() else { return };
        if run.account == account {
            return;
        }
        let claimed: Vec<String> = self
            .tabs
            .borrow()
            .iter()
            .filter(|t| t.id != tab.id)
            .filter_map(|t| t.claude.borrow().as_ref().and_then(|c| c.session_id.clone()))
            .collect();
        conn.send(json!({"type": "switch", "account": account, "claimed": claimed}));
        tab.last_switch.set(Some(Instant::now()));
        self.toast(&format!("{reason}: {} → {account}", run.account));
        if reason != "Moved" {
            stats::log_event("auto_switch", Some(account), &format!("{} → {account}", run.account), json!({}));
        }
    }

    fn auto_switch_tick(self: &Rc<Self>) {
        let cfg = self.config.borrow().clone();
        let tabs = self.tabs.borrow().clone();
        for tab in tabs {
            let run = tab.claude.borrow().clone();
            let acc = run.as_ref().and_then(|r| self.accounts.borrow().iter().find(|a| a.name == r.account).cloned());
            let Some(acc) = acc.filter(|a| a.over(&cfg)) else {
                tab.warned.set(false);
                continue;
            };
            if !cfg.auto_switch {
                if !tab.warned.get() {
                    tab.warned.set(true);
                    let msg = format!("{} is near its usage limit.", acc.name);
                    self.toast(&format!("{msg} Use the account menu to move the session."));
                    self.notify("Usage limit", &msg);
                    stats::log_event("limit", Some(&acc.name), &msg, json!({}));
                    self.update_chrome();
                }
                continue;
            }
            if tab.idle_secs() < cfg.idle_secs as f64 {
                continue;
            }
            if tab.last_switch.get().is_some_and(|t| t.elapsed() < Duration::from_secs(cfg.cooldown_secs)) {
                continue;
            }
            let target = model::best(&self.accounts.borrow(), &cfg, Some(&acc.name)).map(|a| a.name.clone());
            match target {
                Some(target) => {
                    self.move_tab(&tab, &target, "Auto-switched");
                    self.notify("Session moved", &format!("{} was near its limit, now on {target}.", acc.name));
                }
                None if !tab.warned.get() => {
                    tab.warned.set(true);
                    self.toast("Every account is near its limit.");
                    self.notify("Usage limit", "Every account is near its limit.");
                }
                None => {}
            }
        }
    }

    pub fn debug_choose_first(self: &Rc<Self>) {
        let p = self.picker.borrow().clone();
        let first = self.accounts.borrow().first().map(|a| a.name.clone());
        if let (Some(p), Some(n)) = (p, first) {
            p.debug_choose(&n);
        }
    }

    pub fn debug_dash_scroll(&self, frac: f64) {
        self.dashboard.scroll_to(frac);
    }

    // ---------- canvas ----------

    fn wire_canvas(self: &Rc<Self>) {
        let d = Rc::downgrade(self);
        let toast: Rc<dyn Fn(&str)> = Rc::new(move |s: &str| {
            if let Some(d) = d.upgrade() {
                d.toast(s);
            }
        });
        *self.canvas.on_toast.borrow_mut() = Some(toast.clone());
        *self.board.on_toast.borrow_mut() = Some(toast.clone());
        *self.jira.on_toast.borrow_mut() = Some(toast);
        // canvas views become editor tabs
        let d = Rc::downgrade(self);
        *self.canvas.on_show.borrow_mut() = Some(Rc::new(move |s: crate::canvas::Shown| {
            let Some(d) = d.upgrade() else { return };
            if d.wb.has(&s.key) {
                if s.focus {
                    d.wb.activate(&s.key, true);
                }
                return;
            }
            let key = s.key.clone();
            let group = d.editor_group();
            let widget = workbench::contain(&s.widget);
            d.wb.open(Spec { key: s.key, title: s.title, icon: s.icon, tooltip: s.tooltip, crumb: s.crumb, widget }, group, s.focus);
            if !s.focus {
                d.wb.set_attention(&key, true);
            }
        }));
        let d = Rc::downgrade(self);
        *self.canvas.on_hide.borrow_mut() = Some(Rc::new(move |key: &str| {
            if let Some(d) = d.upgrade() {
                d.wb.remove(key);
            }
        }));
        let d = Rc::downgrade(self);
        *self.canvas.on_changed.borrow_mut() = Some(Rc::new(move |key: &str| {
            if let Some(d) = d.upgrade() {
                d.wb.set_attention(key, true);
            }
        }));
        let d = Rc::downgrade(self);
        *self.files.on_open.borrow_mut() = Some(Rc::new(move |p: &Path| {
            if let Some(d) = d.upgrade() {
                d.canvas.open_file(p, None);
            }
        }));
        let d = Rc::downgrade(self);
        *self.jira.pick_session.borrow_mut() = Some(Rc::new(move || {
            let _d = d.upgrade()?;
            let sessions = claude_deck::canvas::store::list_sessions();
            let pick = sessions.iter().find(|s| !s.state.state.is_empty() && s.state.state != "ended").or(sessions.first())?;
            Some((pick.meta.id.clone(), pick.dir.clone()))
        }));
        let d = Rc::downgrade(self);
        *self.jira.show_screen.borrow_mut() = Some(Rc::new(move |sid: &str, name: &str| {
            if let Some(d) = d.upgrade() {
                d.canvas_fp.set(0);
                d.poll_canvas();
                d.canvas.open_screen(sid, name, true);
            }
        }));
        let d = Rc::downgrade(self);
        *self.board.on_goto.borrow_mut() = Some(Rc::new(move |s: &claude_deck::canvas::store::Session| {
            if let Some(d) = d.upgrade() {
                d.goto_session(&s.meta.id);
            }
        }));

        // right-click in the canvas tree
        let pop = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        pop.set_parent(self.canvas.explorer());
        pop.set_has_arrow(false);
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let d = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(d) = d.upgrade() else { return };
            let Some((sid, screen)) = d.canvas.target_at(y) else { return };
            let menu = gio::Menu::new();
            match screen {
                Some(name) => {
                    menu.append(Some("Delete Screen"), Some(&format!("win.canvas-delete-screen::{sid}/{name}")));
                }
                None => {
                    menu.append(Some("Git Changes"), Some(&format!("win.canvas-changes::{sid}")));
                    if d.canvas.session(&sid).is_some_and(|s| d.tab(&s.meta.pane).is_some()) {
                        menu.append(Some("Go to Terminal"), Some(&format!("win.canvas-goto::{sid}")));
                    }
                    menu.append(Some("Delete Session"), Some(&format!("win.canvas-delete-session::{sid}")));
                }
            }
            pop.set_menu_model(Some(&menu));
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        });
        self.canvas.explorer().add_controller(click);

        for name in ["canvas-delete-screen", "canvas-delete-session", "canvas-goto", "canvas-changes"] {
            let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let d = Rc::downgrade(self);
            a.connect_activate(move |a, p| {
                let (Some(d), Some(arg)) = (d.upgrade(), p.and_then(|v| v.get::<String>())) else { return };
                match a.name().as_str() {
                    "canvas-delete-screen" => {
                        if let Some((sid, name)) = arg.split_once('/') {
                            d.canvas.delete_screen(sid, name);
                        }
                    }
                    "canvas-delete-session" => d.canvas.delete_session(&arg),
                    "canvas-changes" => {
                        if let Some(s) = d.canvas.session(&arg) {
                            d.canvas.open_changes(Path::new(&s.meta.cwd), &s.meta.project);
                        }
                    }
                    _ => d.goto_session(&arg),
                }
                d.canvas_fp.set(0);
                d.poll_canvas();
            });
            self.window.add_action(&a);
        }
    }

    /// The canvas session of the agent running in a CC Deck tab, if any.
    fn agent_session(&self, tab_id: &str) -> Option<claude_deck::canvas::store::Session> {
        self.canvas
            .sessions()
            .into_iter()
            .filter(|s| s.meta.pane == tab_id && s.state.state != "ended")
            .max_by_key(|s| s.updated)
    }

    /// Its terminal tab when it runs in CC Deck, else its newest screen.
    fn goto_session(self: &Rc<Self>, sid: &str) {
        let Some(s) = self.canvas.session(sid) else { return };
        if self.tab(&s.meta.pane).is_some() {
            self.select(Sel::Tab(s.meta.pane.clone()));
            return;
        }
        if let Some(name) = self.canvas.first_screen(sid) {
            self.canvas.open_screen(sid, &name, true);
        }
    }

    fn poll_canvas(self: &Rc<Self>) {
        let fp = claude_deck::canvas::store::fingerprint();
        if fp == self.canvas_fp.get() {
            return;
        }
        let first = self.canvas_fp.get() == 0 && self.last_states.borrow().is_empty();
        self.canvas_fp.set(fp);
        let sessions = claude_deck::canvas::store::list_sessions();
        let changed = self.canvas.update(sessions.clone());
        self.board.update(sessions.clone());
        self.canvas_note.set_visible(!self.canvas.has_visible());

        let waiting = sessions.iter().filter(|s| s.needs_you()).count();
        self.board_badge.set_text(&waiting.to_string());
        self.board_badge.set_visible(waiting > 0);
        self.needs_label.set_text(&format!("{waiting} waiting"));
        self.needs_btn.set_visible(waiting > 0);
        if let Some((sid, name)) = changed.first().cloned() {
            let key = crate::canvas::screen_key(&sid, &name);
            if !self.wb.is_visible(&key) {
                let t = adw::Toast::builder().title(format!("New on the canvas: {name}")).button_label("Show").timeout(6).build();
                let d = Rc::downgrade(self);
                t.connect_button_clicked(move |_| {
                    if let Some(d) = d.upgrade() {
                        d.canvas.open_screen(&sid, &name, true);
                    }
                });
                self.toasts.add_toast(t);
            }
        }

        // state changes of agents: notifications
        let mut last = self.last_states.borrow_mut();
        for s in &sessions {
            let prev = last.insert(s.meta.id.clone(), s.state.state.clone());
            if first || prev.as_deref() == Some(s.state.state.as_str()) {
                continue;
            }
            let where_ = if s.meta.project.is_empty() { s.label() } else { format!("{} · {}", s.meta.project, s.label()) };
            let in_view = self.wb.is_visible(&term_key(&s.meta.pane)) && self.window.is_active();
            match s.state.state.as_str() {
                "waiting" if !in_view => {
                    self.notify("Claude needs you", &format!("{where_} — {}", if s.state.reason.is_empty() { "waiting" } else { &s.state.reason }));
                }
                "idle" if !in_view && prev.as_deref() == Some("running") => {
                    self.notify("Your turn", &format!("{where_} — done"));
                }
                _ => {}
            }
        }
        drop(last);
        self.rebuild_sessions();
    }

    /// Development aid: `layout:<bottom|right|left|top>[+split]` for snapshots.
    pub fn debug_layout(self: &Rc<Self>, spec: &str) {
        let (pos, split) = spec.split_once('+').map(|(p, s)| (p, !s.is_empty())).unwrap_or((spec, false));
        self.wb.set_panel_pos(match pos {
            "right" => workbench::PanelPos::Right,
            "left" => workbench::PanelPos::Left,
            "top" => workbench::PanelPos::Top,
            _ => workbench::PanelPos::Bottom,
        });
        let first = self.canvas.sessions().into_iter().find(|s| !s.items.is_empty());
        if let Some(s) = first {
            self.canvas.open_screen(&s.meta.id, &s.items[0].name, true);
        }
        if split {
            self.select(Sel::Board);
            self.wb.move_item("board", workbench::EDITOR2, None);
            self.new_tab(None, None);
            if let Some(t) = self.tabs.borrow().last() {
                self.wb.move_item(&term_key(&t.id), workbench::PANEL2, None);
            }
        }
    }

    /// Development aid: type `claude` in the active tab so the real picker flow runs.
    pub fn debug_pick(self: &Rc<Self>) {
        if let Some(t) = self.active_tab() {
            t.type_text("claude\r");
        }
    }

    pub fn shutdown(&self) {
        self.wb.save();
        for t in self.tabs.borrow().iter() {
            t.terminate();
        }
        if !self.socket.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}
