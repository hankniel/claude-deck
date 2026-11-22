//! VS Code-style workbench: title bar, activity bar, side bar, two editor groups, a panel
//! (two groups) that docks on any side, and a status bar.
//!
//! Every open thing (terminal, canvas screen, file, Board…) is an `Item` with a unique key.
//! An item lives in exactly one group and can be moved between groups by dragging its tab
//! or from the tab's menu. The owner (Deck) is told when the user closes, selects or moves one.

use crate::prefs;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub type Cb<T> = RefCell<Option<Rc<T>>>;

pub const EDITOR: usize = 0;
pub const EDITOR2: usize = 1;
pub const PANEL: usize = 2;
pub const PANEL2: usize = 3;
const TAB_MIME: &str = "application/x-cc-deck-tab";

pub fn is_panel(g: usize) -> bool {
    g >= PANEL
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelPos {
    Bottom,
    Top,
    Left,
    Right,
}

impl PanelPos {
    fn name(self) -> &'static str {
        match self {
            PanelPos::Bottom => "bottom",
            PanelPos::Top => "top",
            PanelPos::Left => "left",
            PanelPos::Right => "right",
        }
    }
    fn parse(s: &str) -> PanelPos {
        match s {
            "top" => PanelPos::Top,
            "left" => PanelPos::Left,
            "right" => PanelPos::Right,
            _ => PanelPos::Bottom,
        }
    }
    fn vertical(self) -> bool {
        matches!(self, PanelPos::Bottom | PanelPos::Top)
    }
}

/// What the owner hands over to open something.
pub struct Spec {
    pub key: String,
    pub title: String,
    pub icon: String,
    pub tooltip: String,
    pub crumb: String,
    pub widget: gtk::Widget,
}

pub struct Item {
    pub key: String,
    pub widget: gtk::Widget,
    crumb: RefCell<String>,
    group: Cell<usize>,
    tab: gtk::Box,
    label: gtk::Label,
    icon: gtk::Image,
    dot: gtk::Box,
}

struct Group {
    idx: usize,
    root: gtk::Box,
    header: gtk::Box,
    strip: gtk::Box,
    tools: gtk::Box,
    crumb: gtk::Label,
    stack: gtk::Stack,
    holder: gtk::Box,
    order: RefCell<Vec<String>>,
    active: RefCell<Option<String>>,
}

pub struct Workbench {
    pub root: adw::ToolbarView,
    pub command: gtk::Button,
    command_label: gtk::Label,
    activity_top: gtk::Box,
    activity_bottom: gtk::Box,
    activities: RefCell<Vec<(String, gtk::Button)>>,
    side: gtk::Box,
    side_stack: gtk::Stack,
    side_views: RefCell<Vec<String>>,
    side_paned: gtk::Paned,
    work_paned: gtk::Paned,
    editor_paned: gtk::Paned,
    panel_paned: gtk::Paned,
    editor_area: gtk::Box,
    panel_area: gtk::Box,
    pub top_slot: gtk::Box,
    pub status_left: gtk::Box,
    pub status_right: gtk::Box,
    pub panel_tools: gtk::Box,
    groups: Vec<Rc<Group>>,
    items: RefCell<Vec<Rc<Item>>>,
    pos: Cell<PanelPos>,
    panel_shown: Cell<bool>,
    /// The state the panel is going to (it stays shown while it slides away).
    panel_want: Cell<bool>,
    sliding: Cell<bool>,
    panel_max: Cell<bool>,
    panel_size: Cell<i32>,
    side_btn: gtk::ToggleButton,
    panel_btn: gtk::ToggleButton,
    focused: Cell<usize>,
    drag_key: RefCell<Option<String>>,
    sized: Cell<bool>,
    settling: Cell<bool>,
    pub on_closed: Cb<dyn Fn(&str)>,
    pub on_selected: Cb<dyn Fn(&str)>,
    pub on_moved: Cb<dyn Fn(&str, usize)>,
    pub on_menu: Cb<dyn Fn(&str, &gio::Menu)>,
    pub on_side: Cb<dyn Fn(&str)>,
    me: RefCell<Weak<Workbench>>,
}

fn tool(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("wb-strip-tool");
    b.set_valign(gtk::Align::Center);
    b
}

/// Host a page that would otherwise force its minimum width on the whole layout.
pub fn contain(w: &impl IsA<gtk::Widget>) -> gtk::Widget {
    // A page reopened after its tab closed still sits in the old host.
    if let Some(p) = w.as_ref().parent() {
        if let Ok(vp) = p.clone().downcast::<gtk::Viewport>() {
            vp.set_child(None::<&gtk::Widget>);
        } else if let Ok(sc) = p.downcast::<gtk::ScrolledWindow>() {
            sc.set_child(None::<&gtk::Widget>);
        }
    }
    let sc = gtk::ScrolledWindow::new();
    sc.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    sc.set_child(Some(w));
    sc.upcast()
}

pub fn strip_tool(icon: &str, tip: &str, action: &str) -> gtk::Button {
    let b = tool(icon, tip);
    b.set_action_name(Some(action));
    b
}

/// Slide a split to `to` with an ease-out, then run `done`. Shrinking is allowed meanwhile.
fn slide(paned: &gtk::Paned, from: i32, to: i32, done: impl FnOnce() + 'static) {
    let shrink = (paned.shrinks_start_child(), paned.shrinks_end_child());
    paned.set_shrink_start_child(true);
    paned.set_shrink_end_child(true);
    paned.set_position(from);
    let start = std::time::Instant::now();
    let done = RefCell::new(Some(done));
    paned.add_tick_callback(move |p, _| {
        let t = (start.elapsed().as_secs_f64() / 0.22).min(1.0);
        let k = 1.0 - (1.0 - t).powi(3);
        p.set_position(from + ((to - from) as f64 * k).round() as i32);
        if t < 1.0 {
            return glib::ControlFlow::Continue;
        }
        p.set_shrink_start_child(shrink.0);
        p.set_shrink_end_child(shrink.1);
        if let Some(f) = done.borrow_mut().take() {
            f();
        }
        glib::ControlFlow::Break
    });
}

impl Group {
    fn new(idx: usize) -> Rc<Group> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("wb-group");
        if is_panel(idx) {
            root.add_css_class("panel");
        }
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.add_css_class("wb-strip");
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let sc = gtk::ScrolledWindow::new();
        sc.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        sc.set_child(Some(&strip));
        sc.set_propagate_natural_width(true);
        sc.set_hexpand(false);
        // Vertical wheel scrolls the tab strip sideways.
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let sc2 = sc.clone();
        wheel.connect_scroll(move |_, _, dy| {
            let a = sc2.hadjustment();
            a.set_value(a.value() + dy * 40.0);
            glib::Propagation::Stop
        });
        sc.add_controller(wheel);
        if is_panel(idx) && idx == PANEL {
            let t = crate::widgets::label("TERMINAL", &["wb-panel-title"]);
            t.set_valign(gtk::Align::Center);
            header.append(&t);
        }
        header.append(&sc);
        let fill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fill.add_css_class("wb-strip-fill");
        fill.set_hexpand(true);
        header.append(&fill);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tools.set_margin_end(6);
        tools.add_css_class("wb-strip-fill");
        header.append(&tools);
        root.append(&header);
        let crumb = crate::widgets::label("", &["wb-crumb"]);
        crumb.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        crumb.set_visible(false);
        root.append(&crumb);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_hexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(140);
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.set_vexpand(true);
        stack.add_named(&holder, Some("__empty"));
        root.append(&stack);
        Rc::new(Group {
            idx,
            root,
            header,
            strip,
            tools,
            crumb,
            stack,
            holder,
            order: RefCell::new(vec![]),
            active: RefCell::new(None),
        })
    }

    fn set_placeholder(&self, w: &impl IsA<gtk::Widget>) {
        while let Some(c) = self.holder.first_child() {
            self.holder.remove(&c);
        }
        w.as_ref().set_vexpand(true);
        self.holder.append(w);
    }

    fn is_empty(&self) -> bool {
        self.order.borrow().is_empty()
    }
}

fn drop_note(text: &str) -> gtk::Widget {
    let l = crate::widgets::label(text, &["wb-drop-note"]);
    l.set_halign(gtk::Align::Center);
    l.set_valign(gtk::Align::Center);
    l.set_vexpand(true);
    l.upcast()
}

impl Workbench {
    pub fn new() -> Rc<Workbench> {
        // ---- title bar ----
        let titlebar = adw::HeaderBar::new();
        titlebar.add_css_class("wb-titlebar");
        let brand = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let logo = gtk::Image::from_icon_name(crate::APP_ID);
        logo.set_pixel_size(18);
        brand.append(&logo);
        brand.append(&crate::widgets::label("CC Deck", &["wb-brand"]));
        titlebar.pack_start(&brand);
        let command_label = gtk::Label::new(Some("CC Deck"));
        command_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let command = gtk::Button::new();
        command.add_css_class("wb-command");
        command.add_css_class("flat");
        {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            b.set_halign(gtk::Align::Center);
            let i = gtk::Image::from_icon_name("folder-symbolic");
            i.set_pixel_size(13);
            b.append(&i);
            b.append(&command_label);
            command.set_child(Some(&b));
        }
        titlebar.set_title_widget(Some(&command));
        let side_btn = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text("Toggle Side Bar (Ctrl+B)").build();
        let panel_btn = gtk::ToggleButton::builder().icon_name("utilities-terminal-symbolic").tooltip_text("Toggle Panel (Ctrl+`)").build();
        for b in [&side_btn, &panel_btn] {
            b.add_css_class("flat");
            b.add_css_class("wb-layout-btn");
        }
        let pos_menu = gio::Menu::new();
        for (l, p) in [("Panel Bottom", "bottom"), ("Panel Right", "right"), ("Panel Left", "left"), ("Panel Top", "top")] {
            pos_menu.append(Some(l), Some(&format!("wb.panel-pos::{p}")));
        }
        let more = gio::Menu::new();
        more.append(Some("Split Editor"), Some("wb.split-editor"));
        more.append(Some("Maximize Panel"), Some("wb.panel-max"));
        pos_menu.append_section(None, &more);
        let pos_btn = gtk::MenuButton::builder()
            .icon_name("view-dual-symbolic")
            .menu_model(&pos_menu)
            .tooltip_text("Customize Layout")
            .build();
        pos_btn.add_css_class("flat");
        pos_btn.add_css_class("wb-layout-btn");

        // ---- activity bar ----
        let activity = gtk::Box::new(gtk::Orientation::Vertical, 0);
        activity.add_css_class("wb-activity");
        let activity_top = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let activity_bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        activity_top.set_vexpand(true);
        activity.append(&activity_top);
        activity.append(&activity_bottom);

        // ---- side bar ----
        let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side.add_css_class("wb-side");
        let side_stack = gtk::Stack::new();
        side_stack.set_vexpand(true);
        side_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        side_stack.set_transition_duration(120);
        side.append(&side_stack);
        side.set_size_request(170, -1);

        // ---- groups ----
        let groups: Vec<Rc<Group>> = (0..4).map(Group::new).collect();
        let editor_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        editor_paned.add_css_class("wb-split");
        editor_paned.set_start_child(Some(&groups[EDITOR].root));
        editor_paned.set_end_child(Some(&groups[EDITOR2].root));
        editor_paned.set_shrink_start_child(false);
        editor_paned.set_shrink_end_child(false);
        let panel_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        panel_paned.add_css_class("wb-split");
        panel_paned.set_start_child(Some(&groups[PANEL].root));
        panel_paned.set_end_child(Some(&groups[PANEL2].root));
        panel_paned.set_shrink_start_child(false);
        panel_paned.set_shrink_end_child(false);
        let top_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let editor_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        editor_area.add_css_class("wb-editor-area");
        editor_area.append(&top_slot);
        editor_area.append(&editor_paned);
        editor_paned.set_vexpand(true);
        let panel_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel_area.append(&panel_paned);
        panel_paned.set_vexpand(true);
        let work_paned = gtk::Paned::new(gtk::Orientation::Vertical);
        work_paned.add_css_class("wb-split");
        work_paned.set_wide_handle(false);

        let side_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        side_paned.add_css_class("wb-split");
        side_paned.set_start_child(Some(&side));
        side_paned.set_end_child(Some(&work_paned));
        side_paned.set_resize_start_child(false);
        side_paned.set_shrink_start_child(false);
        side_paned.set_shrink_end_child(false);
        side_paned.set_position(prefs::get("wb.side_width").and_then(|s| s.parse().ok()).unwrap_or(270));

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("wb-body");
        body.append(&activity);
        body.append(&side_paned);
        side_paned.set_hexpand(true);

        // ---- status bar ----
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status.add_css_class("wb-status");
        let status_left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let status_right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status_left.set_hexpand(true);
        status.append(&status_left);
        status.append(&status_right);

        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.append(&body);
        body.set_vexpand(true);
        main.append(&status);

        let root = adw::ToolbarView::new();
        root.add_top_bar(&titlebar);
        root.set_content(Some(&main));
        root.set_top_bar_style(adw::ToolbarStyle::Flat);

        titlebar.pack_end(&pos_btn);
        titlebar.pack_end(&panel_btn);
        titlebar.pack_end(&side_btn);

        let panel_tools = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        groups[PANEL].tools.append(&panel_tools);
        let pos = PanelPos::parse(&prefs::get("wb.panel_pos").unwrap_or_default());
        let panel_size = prefs::get(if pos.vertical() { "wb.panel_size" } else { "wb.panel_width" })
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let wb = Rc::new(Workbench {
            root,
            command,
            command_label,
            activity_top,
            activity_bottom,
            activities: RefCell::new(vec![]),
            side,
            side_stack,
            side_views: RefCell::new(vec![]),
            side_paned,
            work_paned,
            editor_paned,
            panel_paned,
            editor_area,
            panel_area,
            top_slot,
            status_left,
            status_right,
            panel_tools,
            groups,
            items: RefCell::new(vec![]),
            pos: Cell::new(pos),
            panel_shown: Cell::new(prefs::get("wb.panel_shown").as_deref() != Some("0")),
            panel_want: Cell::new(prefs::get("wb.panel_shown").as_deref() != Some("0")),
            sliding: Cell::new(false),
            panel_max: Cell::new(false),
            panel_size: Cell::new(panel_size),
            side_btn,
            panel_btn,
            focused: Cell::new(EDITOR),
            drag_key: RefCell::new(None),
            sized: Cell::new(false),
            settling: Cell::new(false),
            on_closed: RefCell::new(None),
            on_selected: RefCell::new(None),
            on_moved: RefCell::new(None),
            on_menu: RefCell::new(None),
            on_side: RefCell::new(None),
            me: RefCell::new(Weak::new()),
        });
        *wb.me.borrow_mut() = Rc::downgrade(&wb);
        wb.wire();
        wb.relayout();
        wb.refresh_groups();
        wb
    }

    fn wire(self: &Rc<Self>) {
        // group tools: split / maximize / close panel
        let ed = &self.groups[EDITOR].tools;
        ed.append(&strip_tool("view-dual-symbolic", "Split Editor Right", "wb.split-editor"));
        let pt = &self.groups[PANEL].tools;
        pt.append(&strip_tool("view-dual-symbolic", "Split Terminal", "wb.split-panel"));
        pt.append(&strip_tool("go-up-symbolic", "Maximize Panel", "wb.panel-max"));
        pt.append(&strip_tool("window-close-symbolic", "Hide Panel (Ctrl+`)", "wb.panel-toggle"));
        self.groups[EDITOR2].tools.append(&strip_tool("window-close-symbolic", "Close Group", "wb.close-group::1"));
        self.groups[PANEL2].tools.append(&strip_tool("window-close-symbolic", "Close Group", "wb.close-group::3"));

        for g in &self.groups {
            // focus tracking
            let f = gtk::EventControllerFocus::new();
            let me = Rc::downgrade(self);
            let idx = g.idx;
            f.connect_enter(move |_| {
                if let Some(w) = me.upgrade() {
                    w.set_focused(idx);
                }
            });
            g.root.add_controller(f);
            let click = gtk::GestureClick::new();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let me = Rc::downgrade(self);
            click.connect_pressed(move |_, _, _, _| {
                if let Some(w) = me.upgrade() {
                    w.set_focused(idx);
                }
            });
            g.root.add_controller(click);
            // drop target for tabs
            self.add_drop(&g.root.clone().upcast(), idx, None);
        }
        self.side_btn.set_active(prefs::get("wb.side_shown").as_deref() != Some("0"));
        self.side.set_visible(self.side_btn.is_active());
        let me = Rc::downgrade(self);
        self.side_btn.connect_toggled(move |b| {
            let Some(w) = me.upgrade() else { return };
            let on = b.is_active();
            prefs::set("wb.side_shown", if on { "1" } else { "0" });
            if !w.sized.get() {
                w.side.set_visible(on);
                return;
            }
            let width = if w.side.is_visible() && w.side_paned.position() > 120 {
                w.side_paned.position()
            } else {
                prefs::get("wb.side_width").and_then(|s| s.parse().ok()).unwrap_or(270)
            };
            w.side.set_size_request(0, -1);
            let side = w.side.clone();
            let restore = move |visible: bool| {
                move || {
                    side.set_size_request(170, -1);
                    side.set_visible(visible);
                }
            };
            if on {
                w.side.set_visible(true);
                let paned = w.side_paned.clone();
                slide(&w.side_paned, 0, width, {
                    let r = restore(true);
                    move || {
                        r();
                        paned.set_position(width);
                    }
                });
            } else {
                prefs::set("wb.side_width", &width.to_string());
                let paned = w.side_paned.clone();
                slide(&w.side_paned, width, 0, {
                    let r = restore(false);
                    move || {
                        r();
                        paned.set_position(width);
                    }
                });
            }
        });
        let me = Rc::downgrade(self);
        self.panel_btn.connect_toggled(move |b| {
            if let Some(w) = me.upgrade() {
                if w.panel_want.get() != b.is_active() {
                    w.set_panel_shown(b.is_active());
                }
            }
        });
        self.panel_btn.set_active(self.panel_shown.get());
        let me = Rc::downgrade(self);
        self.work_paned.connect_position_notify(move |_| {
            if let Some(w) = me.upgrade() {
                w.record_panel_size();
            }
        });
        // first allocation: size the panel
        let me = Rc::downgrade(self);
        self.work_paned.add_tick_callback(move |p, _| {
            let Some(w) = me.upgrade() else { return glib::ControlFlow::Break };
            if p.width() > 0 && p.height() > 0 {
                w.sized.set(true);
                w.apply_panel_size();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Window actions of the workbench (`wb.*`).
    pub fn install_actions(self: &Rc<Self>, win: &impl IsA<gtk::Widget>) {
        let grp = gio::SimpleActionGroup::new();
        let add = |name: &str, ty: Option<&glib::VariantTy>, f: Box<dyn Fn(&Rc<Workbench>, Option<String>)>| {
            let a = gio::SimpleAction::new(name, ty);
            let me = Rc::downgrade(self);
            a.connect_activate(move |_, p| {
                if let Some(w) = me.upgrade() {
                    f(&w, p.and_then(|v| v.get::<String>()));
                }
            });
            grp.add_action(&a);
        };
        let s = Some(glib::VariantTy::STRING);
        add("close", s, Box::new(|w, k| {
            if let Some(k) = k {
                w.close(&k);
            }
        }));
        add("close-others", s, Box::new(|w, k| {
            let Some(k) = k else { return };
            let Some(g) = w.item(&k).map(|i| i.group.get()) else { return };
            let others: Vec<String> = w.groups[g].order.borrow().iter().filter(|x| **x != k).cloned().collect();
            for o in others {
                w.close(&o);
            }
        }));
        add("move-editor", s, Box::new(|w, k| {
            if let Some(k) = k {
                w.move_item(&k, EDITOR, None);
            }
        }));
        add("move-panel", s, Box::new(|w, k| {
            if let Some(k) = k {
                w.move_item(&k, PANEL, None);
            }
        }));
        add("move-split", s, Box::new(|w, k| {
            let Some(k) = k else { return };
            let Some(g) = w.item(&k).map(|i| i.group.get()) else { return };
            let to = match g {
                EDITOR => EDITOR2,
                EDITOR2 => EDITOR,
                PANEL => PANEL2,
                _ => PANEL,
            };
            w.move_item(&k, to, None);
        }));
        add("panel-pos", s, Box::new(|w, p| {
            if let Some(p) = p {
                w.set_panel_pos(PanelPos::parse(&p));
            }
        }));
        add("close-group", s, Box::new(|w, g| {
            let g: usize = g.and_then(|g| g.parse().ok()).unwrap_or(EDITOR2);
            let into = if is_panel(g) { PANEL } else { EDITOR };
            let keys = w.groups[g].order.borrow().clone();
            for k in keys {
                w.move_item(&k, into, None);
            }
        }));
        add("split-editor", None, Box::new(|w, _| {
            let g = w.focused.get();
            let src = if is_panel(g) { EDITOR } else { g };
            let to = if src == EDITOR { EDITOR2 } else { EDITOR };
            if let Some(k) = w.groups[src].active.borrow().clone() {
                w.move_item(&k, to, None);
            }
        }));
        add("split-panel", None, Box::new(|w, _| {
            if let Some(k) = w.groups[PANEL].active.borrow().clone() {
                if w.groups[PANEL].order.borrow().len() > 1 {
                    w.move_item(&k, PANEL2, None);
                }
            }
        }));
        add("panel-toggle", None, Box::new(|w, _| w.toggle_panel()));
        add("panel-max", None, Box::new(|w, _| {
            w.panel_max.set(!w.panel_max.get());
            if w.panel_max.get() && !w.panel_want.get() {
                w.set_panel_shown(true);
            }
            w.relayout();
        }));
        add("side-toggle", None, Box::new(|w, _| w.side_btn.set_active(!w.side_btn.is_active())));
        win.as_ref().insert_action_group("wb", Some(&grp));
    }

    // ---------- activity bar & side bar ----------

    /// An activity bar button. Returns the button and its badge label.
    pub fn add_activity(&self, id: &str, icon: &str, tip: &str, bottom: bool) -> (gtk::Button, gtk::Label) {
        let b = gtk::Button::new();
        b.add_css_class("wb-act");
        b.set_tooltip_text(Some(tip));
        let ov = gtk::Overlay::new();
        let img = gtk::Image::from_icon_name(icon);
        img.set_halign(gtk::Align::Center);
        img.set_valign(gtk::Align::Center);
        ov.set_child(Some(&img));
        let badge = gtk::Label::new(None);
        badge.add_css_class("wb-badge");
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.set_visible(false);
        badge.set_can_target(false);
        ov.add_overlay(&badge);
        b.set_child(Some(&ov));
        if bottom {
            self.activity_bottom.append(&b);
        } else {
            self.activity_top.append(&b);
        }
        self.activities.borrow_mut().push((id.to_string(), b.clone()));
        (b, badge)
    }

    /// A side bar view: title row (with optional tools) over `content`.
    pub fn add_side_view(&self, id: &str, title: &str, tools: &[gtk::Widget], content: &impl IsA<gtk::Widget>) {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let t = crate::widgets::label(&title.to_uppercase(), &["wb-side-title"]);
        t.set_hexpand(true);
        head.append(&t);
        for w in tools {
            head.append(w);
        }
        head.set_margin_end(8);
        v.append(&head);
        content.as_ref().set_vexpand(true);
        v.append(content);
        self.side_stack.add_named(&v, Some(id));
        self.side_views.borrow_mut().push(id.to_string());
    }

    /// Show a side view; clicking the active one hides the side bar (VS Code behaviour).
    pub fn toggle_side_view(&self, id: &str) {
        let current = self.side_stack.visible_child_name().map(|s| s.to_string());
        if self.side.is_visible() && current.as_deref() == Some(id) {
            self.side_btn.set_active(false);
        } else {
            self.show_side_view(id);
        }
    }

    pub fn show_side_view(&self, id: &str) {
        self.side_btn.set_active(true);
        self.pick_side_view(id);
    }

    /// Choose the side view without changing whether the side bar shows.
    pub fn pick_side_view(&self, id: &str) {
        self.side_stack.set_visible_child_name(id);
        prefs::set("wb.side_view", id);
        for (aid, b) in self.activities.borrow().iter() {
            if aid == id {
                b.add_css_class("on");
            } else if self.side_views.borrow().contains(aid) {
                b.remove_css_class("on");
            }
        }
        if let Some(f) = self.on_side.borrow().clone() {
            f(id);
        }
    }

    pub fn set_command_text(&self, text: &str) {
        self.command_label.set_text(text);
    }

    // ---------- layout ----------

    fn record_panel_size(&self) {
        if !self.sized.get() || self.settling.get() || !self.panel_shown.get() || self.panel_max.get() {
            return;
        }
        let p = &self.work_paned;
        let total = if self.pos.get().vertical() { p.height() } else { p.width() };
        if total <= 0 {
            return;
        }
        let size = match self.pos.get() {
            PanelPos::Bottom | PanelPos::Right => total - p.position(),
            PanelPos::Top | PanelPos::Left => p.position(),
        };
        if size > 60 && size < total - 60 {
            self.panel_size.set(size);
        }
    }

    fn apply_panel_size(&self) {
        if self.sliding.get() {
            return;
        }
        let p = &self.work_paned;
        let total = if self.pos.get().vertical() { p.height() } else { p.width() };
        if total <= 0 {
            return;
        }
        // 0 = never sized by the user: a share of the window
        let want = match self.panel_size.get() {
            0 => (total as f64 * if self.pos.get().vertical() { 0.36 } else { 0.42 }) as i32,
            s => s,
        };
        let size = want.clamp(80, (total - 320).max(80));
        p.set_position(match self.pos.get() {
            PanelPos::Bottom | PanelPos::Right => total - size,
            PanelPos::Top | PanelPos::Left => size,
        });
    }

    fn relayout(&self) {
        let p = &self.work_paned;
        let pos = self.pos.get();
        let saved = self.panel_size.get();
        p.set_start_child(None::<&gtk::Widget>);
        p.set_end_child(None::<&gtk::Widget>);
        p.set_orientation(if pos.vertical() { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        let (start, end): (&gtk::Box, &gtk::Box) = match pos {
            PanelPos::Bottom | PanelPos::Right => (&self.editor_area, &self.panel_area),
            PanelPos::Top | PanelPos::Left => (&self.panel_area, &self.editor_area),
        };
        p.set_start_child(Some(start));
        p.set_end_child(Some(end));
        let panel_first = matches!(pos, PanelPos::Top | PanelPos::Left);
        p.set_resize_start_child(!panel_first);
        p.set_resize_end_child(panel_first);
        p.set_shrink_start_child(false);
        p.set_shrink_end_child(false);
        for c in ["wb-panel-border-top", "wb-panel-border-left"] {
            self.panel_area.remove_css_class(c);
        }
        self.panel_area.add_css_class(if pos.vertical() { "wb-panel-border-top" } else { "wb-panel-border-left" });
        // A panel at the side stacks its two groups; at the bottom they sit side by side.
        self.panel_paned.set_orientation(if pos.vertical() { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical });
        self.panel_area.set_visible(self.panel_shown.get());
        self.editor_area.set_visible(!(self.panel_max.get() && self.panel_shown.get()));
        self.panel_size.set(saved);
        if self.sized.get() {
            // Positions reported while the children re-allocate are not the user's choice.
            self.settling.set(true);
            let me = self.me.borrow().clone();
            let frames = Cell::new(0);
            self.work_paned.add_tick_callback(move |_, _| {
                let Some(w) = me.upgrade() else { return glib::ControlFlow::Break };
                frames.set(frames.get() + 1);
                w.apply_panel_size();
                if frames.get() < 3 {
                    return glib::ControlFlow::Continue;
                }
                w.settling.set(false);
                glib::ControlFlow::Break
            });
        }
    }

    pub fn set_panel_pos(&self, pos: PanelPos) {
        self.record_panel_size();
        // a side panel and a bottom panel want different sizes
        if pos.vertical() != self.pos.get().vertical() {
            let alt = prefs::get(if pos.vertical() { "wb.panel_size" } else { "wb.panel_width" }).and_then(|s| s.parse().ok());
            if self.panel_size.get() > 0 {
                prefs::set(if pos.vertical() { "wb.panel_width" } else { "wb.panel_size" }, &self.panel_size.get().to_string());
            }
            self.panel_size.set(alt.unwrap_or(0));
        }
        self.pos.set(pos);
        prefs::set("wb.panel_pos", pos.name());
        self.relayout();
    }

    pub fn set_panel_shown(&self, on: bool) {
        if on == self.panel_want.get() {
            return;
        }
        if !on {
            self.record_panel_size();
        }
        self.panel_want.set(on);
        self.panel_btn.set_active(on);
        prefs::set("wb.panel_shown", if on { "1" } else { "0" });
        let p = &self.work_paned;
        let total = if self.pos.get().vertical() { p.height() } else { p.width() };
        let animate = self.sized.get() && total > 0 && !self.panel_max.get();
        // the edge the panel slides out of
        let edge = match self.pos.get() {
            PanelPos::Bottom | PanelPos::Right => total,
            PanelPos::Top | PanelPos::Left => 0,
        };
        if on {
            self.sliding.set(animate);
            self.panel_shown.set(true);
            self.relayout();
            if animate {
                let size = self.panel_size.get().max(0);
                let want = match size {
                    0 => (total as f64 * if self.pos.get().vertical() { 0.36 } else { 0.42 }) as i32,
                    s => s,
                }
                .clamp(80, (total - 320).max(80));
                let target = if edge == 0 { want } else { total - want };
                self.settling.set(true);
                let me = self.me.borrow().clone();
                slide(p, edge, target, move || {
                    if let Some(w) = me.upgrade() {
                        w.sliding.set(false);
                        w.settling.set(false);
                    }
                });
            }
        } else {
            self.panel_max.set(false);
            let finish = {
                let me = self.me.borrow().clone();
                move || {
                    if let Some(w) = me.upgrade() {
                        // shown again while it slid away
                        if w.panel_want.get() {
                            return;
                        }
                        w.panel_shown.set(false);
                        w.sliding.set(false);
                        w.settling.set(false);
                        w.relayout();
                    }
                }
            };
            if animate {
                self.settling.set(true);
                self.sliding.set(true);
                slide(p, p.position(), edge, finish);
            } else {
                finish();
            }
            if is_panel(self.focused.get()) {
                self.focus_group(EDITOR);
            }
        }
    }

    pub fn toggle_panel(&self) {
        let on = !self.panel_want.get();
        self.set_panel_shown(on);
        if on {
            self.focus_group(PANEL);
        }
    }

    pub fn toggle_side(&self) {
        self.side_btn.set_active(!self.side_btn.is_active());
    }

    pub fn panel_shown(&self) -> bool {
        self.panel_want.get()
    }

    /// Remember sizes for the next launch.
    pub fn save(&self) {
        self.record_panel_size();
        let key = if self.pos.get().vertical() { "wb.panel_size" } else { "wb.panel_width" };
        if self.panel_size.get() > 0 {
            prefs::set(key, &self.panel_size.get().to_string());
        }
        if self.side.is_visible() && self.side_paned.position() > 120 {
            prefs::set("wb.side_width", &self.side_paned.position().to_string());
        }
    }

    pub fn set_placeholder(&self, group: usize, w: &impl IsA<gtk::Widget>) {
        self.groups[group].set_placeholder(w);
    }

    /// Hide empty secondary groups (shown while dragging, as drop zones).
    fn refresh_groups(&self) {
        let dragging = self.drag_key.borrow().is_some();
        for (g, other) in [(EDITOR2, EDITOR), (PANEL2, PANEL)] {
            let empty = self.groups[g].is_empty();
            let show = !empty || (dragging && !self.groups[other].is_empty());
            let was = self.groups[g].root.is_visible();
            self.groups[g].root.set_visible(show);
            if empty {
                self.groups[g].set_placeholder(&drop_note("Drop a tab here to split"));
            }
            if show && !was {
                let paned = if is_panel(g) { &self.panel_paned } else { &self.editor_paned };
                let frames = Cell::new(0);
                paned.add_tick_callback(move |p, _| {
                    let total = if p.orientation() == gtk::Orientation::Horizontal { p.width() } else { p.height() };
                    frames.set(frames.get() + 1);
                    if total > 0 {
                        p.set_position(total / 2);
                    }
                    if frames.get() < 2 {
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                });
            }
        }
        for g in &self.groups {
            let has = !g.is_empty();
            if !is_panel(g.idx) {
                g.header.set_visible(has);
            }
            g.crumb.set_visible(has && !is_panel(g.idx) && !g.crumb.text().is_empty());
            if !has {
                g.stack.set_visible_child_name("__empty");
            }
        }
    }

    // ---------- items ----------

    pub fn item(&self, key: &str) -> Option<Rc<Item>> {
        self.items.borrow().iter().find(|i| i.key == key).cloned()
    }

    pub fn has(&self, key: &str) -> bool {
        self.item(key).is_some()
    }

    pub fn group_of(&self, key: &str) -> Option<usize> {
        self.item(key).map(|i| i.group.get())
    }

    pub fn keys(&self) -> Vec<String> {
        self.items.borrow().iter().map(|i| i.key.clone()).collect()
    }

    /// The active item of the focused group.
    pub fn current(&self) -> Option<String> {
        self.groups[self.focused.get()].active.borrow().clone()
    }

    pub fn active_in(&self, group: usize) -> Option<String> {
        self.groups[group].active.borrow().clone()
    }

    /// Is the item the visible one of its group (and its area visible)?
    pub fn is_visible(&self, key: &str) -> bool {
        let Some(g) = self.group_of(key) else { return false };
        let area_ok = if is_panel(g) { self.panel_shown.get() } else { self.editor_area.is_visible() };
        area_ok && self.groups[g].active.borrow().as_deref() == Some(key)
    }

    /// Open (or reveal) an item in `group`.
    pub fn open(self: &Rc<Self>, spec: Spec, group: usize, focus: bool) -> Rc<Item> {
        if let Some(it) = self.item(&spec.key) {
            self.activate(&spec.key, focus);
            return it;
        }
        let tab = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tab.add_css_class("wb-tab");
        let icon = gtk::Image::from_icon_name(&spec.icon);
        icon.add_css_class("tab-icon");
        tab.append(&icon);
        let label = gtk::Label::new(Some(&spec.title));
        label.set_max_width_chars(28);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        tab.append(&label);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("tab-dot");
        dot.set_valign(gtk::Align::Center);
        dot.set_visible(false);
        tab.append(&dot);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("tab-close");
        close.add_css_class("flat");
        close.set_valign(gtk::Align::Center);
        close.set_tooltip_text(Some("Close"));
        tab.append(&close);
        tab.set_tooltip_text(Some(&spec.tooltip));

        let item = Rc::new(Item {
            key: spec.key.clone(),
            widget: spec.widget.clone(),
            crumb: RefCell::new(spec.crumb),
            group: Cell::new(group),
            tab: tab.clone(),
            label,
            icon,
            dot,
        });
        let me = Rc::downgrade(self);
        let k = spec.key.clone();
        close.connect_clicked(move |_| {
            if let Some(w) = me.upgrade() {
                w.close(&k);
            }
        });
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let me = Rc::downgrade(self);
        let k = spec.key.clone();
        click.connect_pressed(move |g, _, x, y| {
            let Some(w) = me.upgrade() else { return };
            match g.current_button() {
                2 => w.close(&k),
                3 => w.tab_menu(&k, x, y),
                _ => w.activate(&k, true),
            }
        });
        tab.add_controller(click);
        self.add_drag(&item);
        self.add_drop(&tab.clone().upcast(), group, Some(spec.key.clone()));

        spec.widget.set_vexpand(true);
        spec.widget.set_hexpand(true);
        let g = &self.groups[group];
        g.stack.add_named(&spec.widget, Some(&spec.key));
        g.strip.append(&tab);
        g.order.borrow_mut().push(spec.key.clone());
        self.items.borrow_mut().push(item.clone());
        if focus || g.active.borrow().is_none() {
            self.activate(&spec.key, focus);
        }
        self.refresh_groups();
        item
    }

    fn sync_group(&self, g: &Group) {
        let active = g.active.borrow().clone();
        for it in self.items.borrow().iter().filter(|i| i.group.get() == g.idx) {
            if Some(&it.key) == active.as_ref() {
                it.tab.add_css_class("active");
            } else {
                it.tab.remove_css_class("active");
            }
        }
        match active.and_then(|k| self.item(&k)) {
            Some(it) => {
                g.stack.set_visible_child_name(&it.key);
                let c = it.crumb.borrow().clone();
                g.crumb.set_text(&c);
                g.crumb.set_visible(!is_panel(g.idx) && !c.is_empty());
            }
            None => {
                g.stack.set_visible_child_name("__empty");
                g.crumb.set_visible(false);
            }
        }
    }

    pub fn activate(&self, key: &str, focus: bool) {
        let Some(it) = self.item(key) else { return };
        let gi = it.group.get();
        let g = self.groups[gi].clone();
        *g.active.borrow_mut() = Some(key.to_string());
        it.dot.set_visible(false);
        it.tab.remove_css_class("attention");
        self.sync_group(&g);
        if focus {
            if is_panel(gi) && !self.panel_want.get() {
                self.set_panel_shown(true);
            }
            if !is_panel(gi) && self.panel_max.get() {
                self.panel_max.set(false);
                self.relayout();
            }
            self.set_focused(gi);
            let tab = it.tab.clone();
            glib::idle_add_local_once(move || {
                // keep the active tab in view
                if let Some(sc) = tab.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() {
                    if let Some(p) = tab.compute_point(&sc, &gtk::graphene::Point::new(0.0, 0.0)) {
                        let a = sc.hadjustment();
                        let (x, w) = (p.x() as f64 + a.value(), tab.width() as f64);
                        if x < a.value() {
                            a.set_value(x);
                        } else if x + w > a.value() + a.page_size() {
                            a.set_value(x + w - a.page_size());
                        }
                    }
                }
            });
        }
        if let Some(f) = self.on_selected.borrow().clone() {
            if focus || self.focused.get() == gi {
                f(key);
            }
        }
    }

    fn set_focused(&self, g: usize) {
        if self.focused.get() == g {
            return;
        }
        self.focused.set(g);
        for gr in &self.groups {
            if gr.idx == g {
                gr.root.add_css_class("focused");
            } else {
                gr.root.remove_css_class("focused");
            }
        }
        if let Some(k) = self.groups[g].active.borrow().clone() {
            if let Some(f) = self.on_selected.borrow().clone() {
                f(&k);
            }
        }
    }

    pub fn focus_group(&self, g: usize) {
        self.set_focused(g);
        if let Some(k) = self.groups[g].active.borrow().clone() {
            self.activate(&k, true);
        }
    }

    pub fn focused_group(&self) -> usize {
        self.focused.get()
    }

    /// Close by the user: the owner is told and cleans up.
    pub fn close(&self, key: &str) {
        if self.remove(key) {
            if let Some(f) = self.on_closed.borrow().clone() {
                f(key);
            }
        }
    }

    /// Remove without telling the owner (the owner asked for it).
    pub fn remove(&self, key: &str) -> bool {
        let Some(it) = self.item(key) else { return false };
        let gi = it.group.get();
        let g = self.groups[gi].clone();
        let idx = g.order.borrow().iter().position(|k| k == key).unwrap_or(0);
        g.order.borrow_mut().retain(|k| k != key);
        g.strip.remove(&it.tab);
        g.stack.remove(&it.widget);
        self.items.borrow_mut().retain(|i| i.key != key);
        if g.active.borrow().as_deref() == Some(key) {
            let next = {
                let o = g.order.borrow();
                if o.is_empty() {
                    None
                } else {
                    Some(o[idx.min(o.len() - 1)].clone())
                }
            };
            *g.active.borrow_mut() = None;
            if let Some(n) = next {
                self.activate(&n, self.focused.get() == gi);
            }
        }
        self.sync_group(&g);
        self.consolidate();
        self.refresh_groups();
        true
    }

    /// A primary group left empty takes over its secondary group's items.
    fn consolidate(&self) {
        for (main, second) in [(EDITOR, EDITOR2), (PANEL, PANEL2)] {
            if self.groups[main].is_empty() && !self.groups[second].is_empty() {
                let keys = self.groups[second].order.borrow().clone();
                for k in keys {
                    self.move_item(&k, main, None);
                }
            }
        }
    }

    /// Move an item to another group (before `before`, else at the end).
    pub fn move_item(&self, key: &str, to: usize, before: Option<&str>) {
        let Some(it) = self.item(key) else { return };
        let from = it.group.get();
        if from == to && before.is_none() {
            return;
        }
        let src = self.groups[from].clone();
        let dst = self.groups[to].clone();
        let was_active = src.active.borrow().as_deref() == Some(key);
        // detach
        src.order.borrow_mut().retain(|k| k != key);
        src.strip.remove(&it.tab);
        src.stack.remove(&it.widget);
        if was_active {
            let next = src.order.borrow().first().cloned();
            *src.active.borrow_mut() = next;
        }
        // attach
        it.group.set(to);
        dst.stack.add_named(&it.widget, Some(key));
        let pos = before.and_then(|b| dst.order.borrow().iter().position(|k| k == b));
        match pos {
            Some(i) => {
                dst.order.borrow_mut().insert(i, key.to_string());
                let sibling = if i == 0 { None } else { self.item(&dst.order.borrow()[i - 1]).map(|x| x.tab.clone()) };
                dst.strip.insert_child_after(&it.tab, sibling.as_ref());
            }
            None => {
                dst.order.borrow_mut().push(key.to_string());
                dst.strip.append(&it.tab);
            }
        }
        self.sync_group(&src);
        if is_panel(to) && !self.panel_want.get() {
            self.set_panel_shown(true);
        }
        self.refresh_groups();
        self.activate(key, true);
        if let Some(f) = self.on_moved.borrow().clone() {
            f(key, to);
        }
        if from != to {
            self.consolidate();
            self.refresh_groups();
        }
    }

    pub fn set_title(&self, key: &str, title: &str, tooltip: Option<&str>) {
        if let Some(it) = self.item(key) {
            it.label.set_text(title);
            if let Some(t) = tooltip {
                it.tab.set_tooltip_text(Some(t));
            }
        }
    }

    pub fn set_icon(&self, key: &str, icon: &str, claude: bool) {
        if let Some(it) = self.item(key) {
            it.icon.set_icon_name(Some(icon));
            if claude {
                it.icon.add_css_class("claude");
            } else {
                it.icon.remove_css_class("claude");
            }
        }
    }

    /// Mark an item that changed while not visible.
    pub fn set_attention(&self, key: &str, on: bool) {
        if let Some(it) = self.item(key) {
            let on = on && !self.is_visible(key);
            it.dot.set_visible(on);
            if on {
                it.tab.add_css_class("attention");
            } else {
                it.tab.remove_css_class("attention");
            }
        }
    }

    fn tab_menu(&self, key: &str, x: f64, y: f64) {
        let Some(it) = self.item(key) else { return };
        let menu = gio::Menu::new();
        if let Some(f) = self.on_menu.borrow().clone() {
            let own = gio::Menu::new();
            f(key, &own);
            if own.n_items() > 0 {
                menu.append_section(None, &own);
            }
        }
        let mv = gio::Menu::new();
        let g = it.group.get();
        if is_panel(g) {
            mv.append(Some("Move to Editor Area"), Some(&format!("wb.move-editor::{key}")));
            mv.append(Some(if g == PANEL { "Split Panel" } else { "Move to Main Panel Group" }), Some(&format!("wb.move-split::{key}")));
        } else {
            mv.append(Some("Move to Panel"), Some(&format!("wb.move-panel::{key}")));
            mv.append(Some(if g == EDITOR { "Split Right" } else { "Move to Left Group" }), Some(&format!("wb.move-split::{key}")));
        }
        menu.append_section(None, &mv);
        let cl = gio::Menu::new();
        cl.append(Some("Close"), Some(&format!("wb.close::{key}")));
        cl.append(Some("Close Others"), Some(&format!("wb.close-others::{key}")));
        menu.append_section(None, &cl);
        let pop = gtk::PopoverMenu::from_model(Some(&menu));
        pop.set_parent(&it.tab);
        pop.set_has_arrow(false);
        pop.set_halign(gtk::Align::Start);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
    }

    // ---------- drag and drop of tabs ----------

    fn add_drag(self: &Rc<Self>, it: &Rc<Item>) {
        let src = gtk::DragSource::new();
        src.set_actions(gdk::DragAction::MOVE);
        let key = it.key.clone();
        src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_bytes(TAB_MIME, &glib::Bytes::from(key.as_bytes()))));
        let me = Rc::downgrade(self);
        let key = it.key.clone();
        let tab = it.tab.clone();
        src.connect_drag_begin(move |s, _| {
            if let Some(w) = me.upgrade() {
                *w.drag_key.borrow_mut() = Some(key.clone());
                w.refresh_groups();
            }
            let paintable = gtk::WidgetPaintable::new(Some(&tab));
            s.set_icon(Some(&paintable), 10, 10);
        });
        let me = Rc::downgrade(self);
        src.connect_drag_end(move |_, _, _| {
            if let Some(w) = me.upgrade() {
                *w.drag_key.borrow_mut() = None;
                for g in &w.groups {
                    g.root.remove_css_class("wb-drop");
                }
                w.refresh_groups();
            }
        });
        it.tab.add_controller(src);
    }

    /// Tabs drop onto a group (append) or onto a tab (insert before it).
    fn add_drop(self: &Rc<Self>, w: &gtk::Widget, group: usize, before: Option<String>) {
        let t = gtk::DropTargetAsync::new(Some(gdk::ContentFormats::new(&[TAB_MIME])), gdk::DragAction::MOVE);
        let me = Rc::downgrade(self);
        let target_widget = w.clone();
        t.connect_drag_enter(move |_, _, _, _| {
            if let Some(wb) = me.upgrade() {
                if wb.drag_key.borrow().is_some() {
                    wb.groups[group].root.add_css_class("wb-drop");
                    let _ = &target_widget;
                    return gdk::DragAction::MOVE;
                }
            }
            gdk::DragAction::empty()
        });
        let me = Rc::downgrade(self);
        t.connect_drag_leave(move |_, _| {
            if let Some(wb) = me.upgrade() {
                wb.groups[group].root.remove_css_class("wb-drop");
            }
        });
        let me = Rc::downgrade(self);
        t.connect_drop(move |_, drop, _, _| {
            let Some(wb) = me.upgrade() else { return false };
            wb.groups[group].root.remove_css_class("wb-drop");
            let Some(key) = wb.drag_key.borrow().clone() else { return false };
            drop.finish(gdk::DragAction::MOVE);
            let before = before.clone().filter(|b| *b != key);
            let me2 = Rc::downgrade(&wb);
            // after the drag ends, so the source tab is no longer mid-drag
            glib::idle_add_local_once(move || {
                if let Some(wb) = me2.upgrade() {
                    wb.move_item(&key, group, before.as_deref());
                }
            });
            true
        });
        w.add_controller(t);
    }
}
