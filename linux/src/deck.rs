//! The main window: sidebar (sessions, dashboard, accounts), terminal tabs, and the
//! glue to `claude-deck launch` (account picker, moving sessions, auto-switch).

use crate::control::{self, Conn, Msg};
use crate::canvas::board::Board;
use crate::canvas::CanvasPage;
use crate::dashboard::Dashboard;
use crate::model::{self, AccountInfo};
use crate::picker::{PickReq, Picker};
use crate::settings::Settings;
use crate::term::{ClaudeRun, Tab};
use crate::widgets::{self, label};
use adw::prelude::*;
use vte::prelude::*;
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

#[derive(Clone, PartialEq, Debug)]
pub enum Sel {
    None,
    Tab(String),
    Canvas,
    Board,
    Dashboard,
}

const NAV: [Sel; 3] = [Sel::Canvas, Sel::Board, Sel::Dashboard];

pub struct Deck {
    pub app: adw::Application,
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    stack: gtk::Stack,
    term_menu: gtk::PopoverMenu,
    banner: adw::Banner,
    title: adw::WindowTitle,
    account_btn: gtk::MenuButton,
    account_label: gtk::Label,
    account_menu: gtk::Box,
    start_btn: gtk::Button,
    sessions: gtk::ListBox,
    session_rows: RefCell<Vec<gtk::ListBoxRow>>,
    nav: gtk::ListBox,
    account_list: gtk::ListBox,
    spinner: gtk::Spinner,
    dashboard: Rc<Dashboard>,
    pub canvas: Rc<CanvasPage>,
    board: Rc<Board>,
    canvas_fp: Cell<u64>,
    canvas_badge: gtk::Label,
    board_badge: gtk::Label,
    last_states: RefCell<HashMap<String, String>>,
    socket: PathBuf,
    pub cli: Option<PathBuf>,
    _server: Option<control::Server>,
    pub tabs: RefCell<Vec<Rc<Tab>>>,
    sel: RefCell<Sel>,
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

fn section_label(text: &str) -> gtk::Label {
    let l = label(text, &["sidebar-section"]);
    l.set_halign(gtk::Align::Start);
    l
}

impl Deck {
    pub fn new(app: &adw::Application) -> Rc<Deck> {
        crate::shell::install();
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
            .default_width(1240)
            .default_height(800)
            .width_request(360)
            .height_request(360)
            .build();

        // ----- sidebar -----
        let sessions = gtk::ListBox::new();
        sessions.add_css_class("navigation-sidebar");
        let nav = gtk::ListBox::new();
        nav.add_css_class("navigation-sidebar");
        let canvas_badge = gtk::Label::new(None);
        let board_badge = gtk::Label::new(None);
        for (icon, name, badge) in [
            ("view-paged-symbolic", "Canvas", Some(&canvas_badge)),
            ("checkbox-checked-symbolic", "Board", Some(&board_badge)),
            ("power-profile-performance-symbolic", "Dashboard", None),
        ] {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.append(&gtk::Image::from_icon_name(icon));
            let l = label(name, &[]);
            l.set_hexpand(true);
            b.append(&l);
            if let Some(badge) = badge {
                badge.add_css_class("nav-badge");
                badge.set_visible(false);
                b.append(badge);
            }
            b.set_margin_top(4);
            b.set_margin_bottom(4);
            nav.append(&b);
        }
        let account_list = gtk::ListBox::new();
        account_list.add_css_class("navigation-sidebar");
        account_list.set_selection_mode(gtk::SelectionMode::None);

        let spinner = gtk::Spinner::new();
        let acc_menu = gio::Menu::new();
        acc_menu.append(Some("Auto-switch near limits"), Some("win.auto-switch"));
        acc_menu.append(Some("Refresh usage"), Some("win.refresh"));
        acc_menu.append(Some("Manage accounts…"), Some("win.accounts"));
        let acc_menu_btn = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&acc_menu)
            .tooltip_text("Accounts")
            .valign(gtk::Align::Center)
            .build();
        acc_menu_btn.add_css_class("flat");
        let acc_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let acc_title = section_label("Accounts");
        acc_title.set_hexpand(true);
        acc_header.append(&acc_title);
        acc_header.append(&spinner);
        acc_header.append(&acc_menu_btn);
        acc_header.set_margin_end(8);

        let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side_box.append(&section_label("Sessions"));
        side_box.append(&sessions);
        side_box.append(&section_label("Insights"));
        side_box.append(&nav);
        side_box.append(&acc_header);
        side_box.append(&account_list);
        let side_scroll = gtk::ScrolledWindow::new();
        side_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        side_scroll.set_vexpand(true);
        side_scroll.set_child(Some(&side_box));

        let main_menu = gio::Menu::new();
        let s1 = gio::Menu::new();
        s1.append(Some("New Terminal"), Some("win.new-tab"));
        s1.append(Some("New Terminal in Home Folder"), Some("win.new-tab-home"));
        main_menu.append_section(None, &s1);
        let s2 = gio::Menu::new();
        s2.append(Some("Settings"), Some("win.settings"));
        s2.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
        s2.append(Some("About CC Deck"), Some("win.about"));
        main_menu.append_section(None, &s2);
        let s3 = gio::Menu::new();
        s3.append(Some("Quit"), Some("app.quit"));
        main_menu.append_section(None, &s3);
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu)
            .tooltip_text("Main Menu")
            .primary(true)
            .build();
        let side_header = adw::HeaderBar::new();
        side_header.set_title_widget(Some(&adw::WindowTitle::new("CC Deck", "")));
        side_header.pack_end(&menu_btn);
        let side_view = adw::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&side_scroll));

        // ----- content -----
        let title = adw::WindowTitle::new("CC Deck", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar (F9)")
            .build();
        header.pack_start(&toggle);
        let new_btn = gtk::Button::builder()
            .icon_name("tab-new-symbolic")
            .tooltip_text("New Terminal (Ctrl+Shift+T)")
            .action_name("win.new-tab")
            .build();
        let start_btn = gtk::Button::builder()
            .tooltip_text("Run claude in this terminal (Ctrl+Shift+K)")
            .action_name("win.start-claude")
            .build();
        {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            b.append(&gtk::Image::from_icon_name("starred-symbolic"));
            b.append(&gtk::Label::new(Some("Start Claude")));
            start_btn.set_child(Some(&b));
        }
        let account_label = gtk::Label::new(None);
        let account_btn = gtk::MenuButton::new();
        {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            b.append(&gtk::Image::from_icon_name("avatar-default-symbolic"));
            b.append(&account_label);
            account_btn.set_child(Some(&b));
            account_btn.set_tooltip_text(Some("Account for this Claude session"));
        }
        let account_menu = gtk::Box::new(gtk::Orientation::Vertical, 4);
        account_menu.set_margin_top(6);
        account_menu.set_margin_bottom(6);
        account_menu.set_margin_start(6);
        account_menu.set_margin_end(6);
        let pop = gtk::Popover::new();
        pop.set_child(Some(&account_menu));
        account_btn.set_popover(Some(&pop));
        header.pack_end(&account_btn);
        header.pack_end(&start_btn);
        header.pack_end(&new_btn);

        let banner = adw::Banner::new("");
        banner.set_use_markup(true);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let empty = adw::StatusPage::builder()
            .icon_name("utilities-terminal-symbolic")
            .title("No terminal open")
            .description("Open a terminal, then type <tt>claude</tt> to pick the account for that session.")
            .build();
        let empty_btn = gtk::Button::builder()
            .label("New Terminal")
            .action_name("win.new-tab")
            .halign(gtk::Align::Center)
            .build();
        empty_btn.add_css_class("pill");
        empty_btn.add_css_class("suggested-action");
        empty.set_child(Some(&empty_btn));
        stack.add_named(&empty, Some("empty"));
        let dashboard = Dashboard::new();
        stack.add_named(&dashboard.root, Some("dashboard"));
        let canvas = CanvasPage::new();
        stack.add_named(&canvas.root, Some("canvas"));
        let board = Board::new();
        stack.add_named(&board.root, Some("board"));
        let term_model = gio::Menu::new();
        term_model.append(Some("Copy"), Some("win.copy"));
        term_model.append(Some("Paste"), Some("win.paste"));
        let term_menu = gtk::PopoverMenu::from_model(Some(&term_model));
        term_menu.set_parent(&stack);
        term_menu.set_has_arrow(false);
        term_menu.set_halign(gtk::Align::Start);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&banner);
        body.append(&stack);
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&header);
        content_view.set_content(Some(&body));

        let split = adw::OverlaySplitView::new();
        split.set_sidebar(Some(&side_view));
        split.set_content(Some(&content_view));
        split.set_min_sidebar_width(230.0);
        split.set_max_sidebar_width(340.0);
        split.set_sidebar_width_fraction(0.22);
        split.bind_property("show-sidebar", &toggle, "active").bidirectional().sync_create().build();

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        window.set_content(Some(&toasts));

        let bp = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 720sp").unwrap());
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        let (accounts, config) = model::quick_state();
        let auto_action = gio::SimpleAction::new_stateful("auto-switch", None, &config.auto_switch.to_variant());

        let deck = Rc::new(Deck {
            app: app.clone(),
            window,
            toasts,
            split,
            stack,
            term_menu,
            banner,
            title,
            account_btn,
            account_label,
            account_menu,
            start_btn,
            sessions,
            session_rows: RefCell::new(vec![]),
            nav,
            account_list,
            spinner,
            dashboard,
            canvas,
            board,
            canvas_fp: Cell::new(0),
            canvas_badge,
            board_badge,
            last_states: RefCell::new(HashMap::new()),
            socket,
            cli,
            _server: server,
            tabs: RefCell::new(vec![]),
            sel: RefCell::new(Sel::None),
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
        deck.wire_canvas();
        deck.connect_signals();
        deck.install_actions();

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

        deck.rebuild_accounts();
        deck.new_tab(None, None);
        deck.refresh(true);
        if let Some(e) = server_err {
            deck.toast(&e);
        }
        if deck.cli.is_none() {
            deck.toast("claude-deck CLI not found: `claude` in tabs runs without the account picker");
        }
        deck
    }

    fn connect_signals(self: &Rc<Self>) {
        let d = self.clone();
        self.sessions.connect_row_selected(move |_, row| {
            if d.syncing.get() {
                return;
            }
            let Some(row) = row else { return };
            let id = d.tabs.borrow().get(row.index() as usize).map(|t| t.id.clone());
            if let Some(id) = id {
                d.select(Sel::Tab(id));
            }
        });
        // Right-click on a session row.
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
            let m2 = gio::Menu::new();
            m2.append(Some("Close Terminal"), Some(&format!("win.tab-close::{id}")));
            menu.append_section(None, &m2);
            pop.set_menu_model(Some(&menu));
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
            pop.present();
        });
        self.sessions.add_controller(click);

        let d = self.clone();
        self.nav.connect_row_selected(move |_, row| {
            if d.syncing.get() {
                return;
            }
            if let Some(sel) = row.and_then(|r| NAV.get(r.index() as usize)) {
                d.select(sel.clone());
            }
        });
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
        adw::StyleManager::default().connect_dark_notify(move |sm| {
            for t in d.tabs.borrow().iter() {
                crate::term::apply_theme(&t.term, sm.is_dark());
            }
            d.rebuild_accounts();
            d.dashboard.redraw();
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
        add("canvas", Box::new(|d| d.select(Sel::Canvas)));
        add("board", Box::new(|d| d.select(Sel::Board)));
        add("close-tab", Box::new(|d| {
            if *d.sel.borrow() == Sel::Canvas {
                d.canvas.close_current();
                return;
            }
            if let Some(t) = d.active_tab() {
                d.close_tab(&t.id);
            }
        }));
        add("start-claude", Box::new(|d| {
            if d.active_tab().is_none() {
                d.new_tab(None, None);
            }
            if let Some(t) = d.active_tab() {
                t.type_text("claude\r");
                t.term.grab_focus();
            }
        }));
        add("dashboard", Box::new(|d| d.select(Sel::Dashboard)));
        add("refresh", Box::new(|d| d.refresh(false)));
        add("next-tab", Box::new(|d| d.cycle(1)));
        add("prev-tab", Box::new(|d| d.cycle(-1)));
        add("copy", Box::new(|d| {
            if let Some(t) = d.active_tab() {
                t.term.copy_clipboard_format(vte::Format::Text);
            }
        }));
        add("paste", Box::new(|d| {
            if let Some(t) = d.active_tab() {
                t.last_input.set(Instant::now());
                vte::prelude::TerminalExt::paste_clipboard(&t.term);
            }
        }));
        add("zoom-in", Box::new(|d| d.zoom(Some(1.1))));
        add("zoom-out", Box::new(|d| d.zoom(Some(1.0 / 1.1))));
        add("zoom-reset", Box::new(|d| d.zoom(None)));
        add("toggle-sidebar", Box::new(|d| d.split.set_show_sidebar(!d.split.shows_sidebar())));
        add("settings", Box::new(|d| Settings::show(d, None)));
        add("accounts", Box::new(|d| Settings::show(d, Some("accounts"))));
        add("about", Box::new(|d| {
            let about = adw::AboutDialog::builder()
                .application_name("CC Deck")
                .application_icon(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("Terminal tabs where each Claude Code session runs on the account you pick.")
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

        for (name, f) in [
            ("tab-claude", 0u8),
            ("tab-new-here", 1),
            ("tab-close", 2),
        ] {
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
            ("win.canvas", &["<Ctrl><Shift>e"]),
            ("win.board", &["<Ctrl><Shift>o"]),
            ("win.refresh", &["<Ctrl><Shift>r"]),
            ("win.next-tab", &["<Ctrl>Page_Down", "<Ctrl><Shift>bracketright"]),
            ("win.prev-tab", &["<Ctrl>Page_Up", "<Ctrl><Shift>bracketleft"]),
            ("win.copy", &["<Ctrl><Shift>c"]),
            ("win.paste", &["<Ctrl><Shift>v"]),
            ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]),
            ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
            ("win.zoom-reset", &["<Ctrl>0"]),
            ("win.toggle-sidebar", &["F9"]),
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

    pub fn active_tab(&self) -> Option<Rc<Tab>> {
        match &*self.sel.borrow() {
            Sel::Tab(id) => self.tab(id),
            _ => None,
        }
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
        tab.term.connect_child_exited(move |_, _| {
            let Some(d) = d.upgrade() else { return };
            if let Some(t) = d.tab(&tid) {
                t.exited.set(true);
            }
            let tid = tid.clone();
            glib::idle_add_local_once(move || d.close_tab(&tid));
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
        // Right-click: copy / paste (one shared popover, parented to the stack).
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let d = Rc::downgrade(self);
        click.connect_pressed(move |g, _, x, y| {
            let Some(d) = d.upgrade() else { return };
            let Some(w) = g.widget() else { return };
            let p = w
                .compute_point(&d.stack, &gtk::graphene::Point::new(x as f32, y as f32))
                .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
            d.term_menu.set_pointing_to(Some(&gdk::Rectangle::new(p.x() as i32, p.y() as i32, 1, 1)));
            d.term_menu.popup();
            d.term_menu.present();
        });
        tab.term.add_controller(click);

        self.stack.add_named(&tab.page, Some(&id));
        self.tabs.borrow_mut().push(tab.clone());
        self.rebuild_sessions();
        self.select(Sel::Tab(id));
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
        self.stack.remove(&tab.page);
        let was_active = *self.sel.borrow() == Sel::Tab(id.to_string());
        self.rebuild_sessions();
        if was_active {
            let next = {
                let tabs = self.tabs.borrow();
                if tabs.is_empty() {
                    None
                } else {
                    Some(tabs[i.min(tabs.len() - 1)].id.clone())
                }
            };
            self.select(next.map(Sel::Tab).unwrap_or(Sel::None));
        }
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

    fn zoom(&self, factor: Option<f64>) {
        for t in self.tabs.borrow().iter() {
            let s = factor.map(|f| (t.term.font_scale() * f).clamp(0.5, 3.0)).unwrap_or(1.0);
            t.term.set_font_scale(s);
        }
    }

    pub fn select(self: &Rc<Self>, sel: Sel) {
        *self.sel.borrow_mut() = sel.clone();
        self.syncing.set(true);
        match &sel {
            Sel::Tab(id) => {
                self.nav.unselect_all();
                let i = self.tabs.borrow().iter().position(|t| &t.id == id);
                if let Some(row) = i.and_then(|i| self.sessions.row_at_index(i as i32)) {
                    self.sessions.select_row(Some(&row));
                }
                self.stack.set_visible_child_name(id);
            }
            Sel::Dashboard | Sel::Canvas | Sel::Board => {
                self.sessions.unselect_all();
                let i = NAV.iter().position(|n| n == &sel).unwrap_or(0);
                if let Some(row) = self.nav.row_at_index(i as i32) {
                    self.nav.select_row(Some(&row));
                }
                match sel {
                    Sel::Dashboard => {
                        self.stack.set_visible_child_name("dashboard");
                        self.dashboard.load_if_stale();
                    }
                    Sel::Canvas => self.stack.set_visible_child_name("canvas"),
                    _ => self.stack.set_visible_child_name("board"),
                }
            }
            Sel::None => {
                self.sessions.unselect_all();
                self.nav.unselect_all();
                self.stack.set_visible_child_name("empty");
            }
        }
        self.syncing.set(false);
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
        self.update_chrome();
        if let Sel::Tab(_) = sel {
            if let Some(t) = self.active_tab() {
                let term = t.term.clone();
                glib::idle_add_local_once(move || {
                    term.grab_focus();
                });
            }
        }
    }

    fn tab_changed(self: &Rc<Self>) {
        self.rebuild_sessions();
        self.update_chrome();
    }

    fn rebuild_sessions(self: &Rc<Self>) {
        self.syncing.set(true);
        for r in self.session_rows.borrow_mut().drain(..) {
            self.sessions.remove(&r);
        }
        let active = match &*self.sel.borrow() {
            Sel::Tab(id) => Some(id.clone()),
            _ => None,
        };
        for tab in self.tabs.borrow().iter() {
            let row = gtk::ListBoxRow::new();
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.set_margin_top(3);
            b.set_margin_bottom(3);
            let running = tab.claude.borrow().is_some();
            let icon = gtk::Image::from_icon_name(if running { "starred-symbolic" } else { "utilities-terminal-symbolic" });
            if running {
                icon.add_css_class("claude-icon");
            } else {
                icon.add_css_class("dim-label");
            }
            b.append(&icon);
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let t = label(&tab.display_title(), &[]);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let agent = self.agent_session(&tab.id);
            let state = agent.as_ref().map(crate::canvas::state_text).unwrap_or_default();
            let sub = if state.is_empty() || tab.claude.borrow().is_none() { tab.subtitle() } else { format!("{} · {state}", tab.subtitle()) };
            let s = label(&sub, &["caption", "dim-label"]);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if agent.as_ref().is_some_and(|a| a.needs_you()) && tab.claude.borrow().is_some() {
                s.remove_css_class("dim-label");
                s.add_css_class("warning");
            }
            v.append(&t);
            v.append(&s);
            b.append(&v);
            row.set_child(Some(&b));
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
        for acc in self.accounts.borrow().iter() {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.set_margin_top(3);
            b.set_margin_bottom(3);
            b.append(&widgets::rings(acc.five_pct(), acc.week_pct(), cfg.threshold, cfg.weekly_threshold, 26, 3.5));
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let n = label(&acc.name, &["heading"]);
            h.append(&n);
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
            let s = label(&acc.summary(), &["caption", "numeric"]);
            s.add_css_class(if acc.over(&cfg) { "over" } else { "dim-label" });
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&s);
            b.append(&v);
            b.set_tooltip_text(Some(acc.email.as_deref().unwrap_or(&acc.name)));
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            row.set_child(Some(&b));
            self.account_list.append(&row);
        }
    }

    fn update_chrome(&self) {
        let sel = self.sel.borrow().clone();
        *self.banner_target.borrow_mut() = None;
        self.banner.set_revealed(false);
        match sel {
            Sel::Tab(id) => {
                let Some(tab) = self.tab(&id) else { return };
                self.title.set_title(&tab.display_title());
                self.title.set_subtitle(&model::short_path(&tab.current_dir()));
                let run = tab.claude.borrow().clone();
                match run {
                    Some(run) => {
                        let accs = self.accounts.borrow();
                        let cfg = self.config.borrow();
                        let acc = accs.iter().find(|a| a.name == run.account);
                        let txt = match acc.and_then(|a| a.five_pct()) {
                            Some(p) => format!("{} · {:.0}%", run.account, p),
                            None => run.account.clone(),
                        };
                        self.account_label.set_text(&txt);
                        self.account_btn.set_visible(true);
                        self.start_btn.set_visible(false);
                        if let Some(acc) = acc {
                            if acc.over(&cfg) && !cfg.auto_switch {
                                if let Some(target) = model::best(&accs, &cfg, Some(&acc.name)) {
                                    self.banner.set_title(&format!(
                                        "<b>{}</b> is near its usage limit.",
                                        glib::markup_escape_text(&acc.name)
                                    ));
                                    self.banner.set_button_label(Some(&format!("Move to {}", target.name)));
                                    self.banner.set_revealed(true);
                                    *self.banner_target.borrow_mut() = Some((id.clone(), target.name.clone()));
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
            Sel::Canvas => {
                self.title.set_title("Canvas");
                self.title.set_subtitle(&self.canvas.current_title());
                self.account_btn.set_visible(false);
                self.start_btn.set_visible(false);
            }
            Sel::Board => {
                self.title.set_title("Board");
                self.title.set_subtitle("");
                self.account_btn.set_visible(false);
                self.start_btn.set_visible(false);
            }
            Sel::Dashboard => {
                self.title.set_title("Dashboard");
                self.title.set_subtitle("");
                self.account_btn.set_visible(false);
                self.start_btn.set_visible(false);
            }
            Sel::None => {
                self.title.set_title("CC Deck");
                self.title.set_subtitle("");
                self.account_btn.set_visible(false);
                self.start_btn.set_visible(false);
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

    pub fn add_account(self: &Rc<Self>, name: &str) -> Result<(), String> {
        let cfg = self.config.borrow().clone();
        accounts::create(name, &cfg).map_err(|e| format!("{e:#}"))?;
        let (accs, _) = model::quick_state();
        *self.accounts.borrow_mut() = accs;
        self.after_state_change();
        self.refresh(false);
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

    /// Open a tab that runs claude on `name` so the user can /login.
    pub fn login(self: &Rc<Self>, name: &str) {
        let cmd = match &self.cli {
            Some(_) => format!("\"$CC_DECK_BIN\" launch --account {name}"),
            None => match accounts::find(name).ok().and_then(|a| a.config_dir) {
                Some(dir) => format!("CLAUDE_CONFIG_DIR='{}' command claude", dir.display()),
                None => "env -u CLAUDE_CONFIG_DIR claude".into(),
            },
        };
        self.new_tab(Some(home().to_string_lossy().to_string()), Some(cmd));
        self.window.present();
        self.toast(&format!("Log in to {name} with /login, then keep working or /exit"));
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
        *self.board.on_toast.borrow_mut() = Some(toast);
        let d = Rc::downgrade(self);
        *self.canvas.on_title.borrow_mut() = Some(Rc::new(move |t: &str| {
            if let Some(d) = d.upgrade() {
                if *d.sel.borrow() == Sel::Canvas {
                    d.title.set_subtitle(t);
                }
            }
        }));
        let d = Rc::downgrade(self);
        *self.board.on_goto.borrow_mut() = Some(Rc::new(move |s: &claude_deck::canvas::store::Session| {
            if let Some(d) = d.upgrade() {
                d.goto_session(&s.meta.id);
            }
        }));

        // right-click in the canvas explorer
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
                    if d.canvas.session(&sid).is_some_and(|s| d.tab(&s.meta.pane).is_some()) {
                        menu.append(Some("Go to Terminal"), Some(&format!("win.canvas-goto::{sid}")));
                    }
                    menu.append(Some("Delete Session"), Some(&format!("win.canvas-delete-session::{sid}")));
                }
            }
            pop.set_menu_model(Some(&menu));
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
            pop.present();
        });
        self.canvas.explorer().add_controller(click);

        for name in ["canvas-delete-screen", "canvas-delete-session", "canvas-goto"] {
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

    /// Its terminal tab when it runs in CC Deck, else its screens on the canvas.
    fn goto_session(self: &Rc<Self>, sid: &str) {
        let Some(s) = self.canvas.session(sid) else { return };
        if self.tab(&s.meta.pane).is_some() {
            self.select(Sel::Tab(s.meta.pane.clone()));
            return;
        }
        self.select(Sel::Canvas);
        if let Some(it) = s.items.first() {
            self.canvas.open_screen(sid, &it.name, true);
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
        let visible = *self.sel.borrow() == Sel::Canvas;
        let changed = self.canvas.update(sessions.clone(), visible);
        self.board.update(sessions.clone());

        let waiting = sessions.iter().filter(|s| s.needs_you()).count();
        self.board_badge.set_text(&waiting.to_string());
        self.board_badge.set_visible(waiting > 0);
        if !changed.is_empty() && !visible {
            let n = self.canvas_badge.text().parse::<usize>().unwrap_or(0) + changed.len();
            self.canvas_badge.set_text(&n.to_string());
            self.canvas_badge.set_visible(true);
            let (sid, name) = changed[0].clone();
            let t = adw::Toast::builder().title(format!("New on the canvas: {name}")).button_label("Show").timeout(8).build();
            let d = Rc::downgrade(self);
            t.connect_button_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    d.select(Sel::Canvas);
                    d.canvas.open_screen(&sid, &name, true);
                }
            });
            self.toasts.add_toast(t);
        }
        if visible {
            self.canvas_badge.set_visible(false);
            self.canvas_badge.set_text("0");
        }

        // state changes of agents: tab subtitles, notifications
        let mut last = self.last_states.borrow_mut();
        for s in &sessions {
            let prev = last.insert(s.meta.id.clone(), s.state.state.clone());
            if first || prev.as_deref() == Some(s.state.state.as_str()) {
                continue;
            }
            let where_ = if s.meta.project.is_empty() { s.label() } else { format!("{} · {}", s.meta.project, s.label()) };
            let in_view = matches!(&*self.sel.borrow(), Sel::Tab(id) if *id == s.meta.pane) && self.window.is_active();
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
        if *self.sel.borrow() == Sel::Canvas {
            self.title.set_subtitle(&self.canvas.current_title());
        }
    }

    /// Development aid: type `claude` in the active tab so the real picker flow runs.
    pub fn debug_pick(self: &Rc<Self>) {
        if let Some(t) = self.active_tab() {
            t.type_text("claude\r");
        }
    }

    pub fn shutdown(&self) {
        for t in self.tabs.borrow().iter() {
            t.terminate();
        }
        if !self.socket.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}
