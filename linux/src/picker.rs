//! "Run Claude as…" dialog shown when `claude` is typed in a tab.

use crate::control::Conn;
use crate::deck::Deck;
use crate::model;
use crate::widgets::{self, label};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::Arc;

pub struct PickReq {
    pub tab_id: Option<String>,
    pub cwd: String,
    pub conn: Arc<Conn>,
}

pub struct Picker {
    pub req: PickReq,
    deck: Weak<Deck>,
    dialog: adw::Dialog,
    list: gtk::ListBox,
    remember: gtk::CheckButton,
    start: gtk::Button,
    names: RefCell<Vec<String>>,
    selected: RefCell<Option<String>>,
    answered: Cell<bool>,
    building: Cell<bool>,
}

impl Picker {
    pub fn show(deck: &Rc<Deck>, req: PickReq) -> Rc<Picker> {
        let dialog = adw::Dialog::new();
        dialog.set_title("Run Claude as…");
        dialog.set_content_width(500);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
        root.set_margin_top(22);
        root.set_margin_bottom(22);
        root.set_margin_start(22);
        root.set_margin_end(22);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let icon = gtk::Image::from_icon_name("starred-symbolic");
        icon.set_pixel_size(28);
        icon.add_css_class("claude-icon");
        head.append(&icon);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        titles.append(&label("Run Claude as…", &["title-2"]));
        let folder = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let fi = gtk::Image::from_icon_name("folder-symbolic");
        fi.add_css_class("dim-label");
        folder.append(&fi);
        let fl = label(&model::short_path(&req.cwd), &["dim-label"]);
        fl.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        folder.append(&fl);
        titles.append(&folder);
        head.append(&titles);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Start);
        refresh.set_tooltip_text(Some("Refresh usage"));
        head.append(&refresh);
        root.append(&head);

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(false);
        root.append(&list);

        let remember = gtk::CheckButton::with_label("Always use this account in this folder");
        root.append(&remember);

        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let hint = label("↑↓ or 1–9 to choose", &["caption", "dim-label"]);
        hint.set_hexpand(true);
        foot.append(&hint);
        let cancel = gtk::Button::with_label("Cancel");
        let start = gtk::Button::with_label("Start Claude");
        start.add_css_class("suggested-action");
        foot.append(&cancel);
        foot.append(&start);
        root.append(&foot);
        dialog.set_child(Some(&root));
        dialog.set_default_widget(Some(&start));

        let cfg = deck.config.borrow().clone();
        let mapped = cfg.account_for(Path::new(&req.cwd));
        let initial = mapped
            .clone()
            .or_else(|| model::best(&deck.accounts.borrow(), &cfg, None).map(|a| a.name.clone()))
            .or_else(|| deck.accounts.borrow().first().map(|a| a.name.clone()));
        remember.set_active(mapped.is_some());

        let p = Rc::new(Picker {
            req,
            deck: Rc::downgrade(deck),
            dialog,
            list,
            remember,
            start,
            names: RefCell::new(vec![]),
            selected: RefCell::new(initial),
            answered: Cell::new(false),
            building: Cell::new(false),
        });
        p.rebuild();

        let me = p.clone();
        p.list.connect_row_selected(move |_, row| {
            if me.building.get() {
                return;
            }
            if let Some(r) = row {
                let n = me.names.borrow().get(r.index() as usize).cloned();
                *me.selected.borrow_mut() = n;
                me.start.set_sensitive(true);
            }
        });
        let me = p.clone();
        p.list.connect_row_activated(move |_, row| {
            let n = me.names.borrow().get(row.index() as usize).cloned();
            if let Some(n) = n {
                me.choose(&n);
            }
        });
        let me = p.clone();
        p.start.connect_clicked(move |_| {
            let s = me.selected.borrow().clone();
            if let Some(s) = s {
                me.choose(&s);
            }
        });
        let me = p.clone();
        cancel.connect_clicked(move |_| { me.dialog.close(); });
        let me = Rc::downgrade(deck);
        refresh.connect_clicked(move |_| {
            if let Some(d) = me.upgrade() {
                d.refresh(false);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let me = p.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            if state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            if let Some(c) = key.to_unicode().and_then(|c| c.to_digit(10)) {
                let n = me.names.borrow().get((c as usize).wrapping_sub(1)).cloned();
                if let Some(n) = n {
                    me.choose(&n);
                    return glib::Propagation::Stop;
                }
            }
            if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
                let s = me.selected.borrow().clone();
                if let Some(s) = s {
                    me.choose(&s);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        p.dialog.add_controller(keys);
        let me = p.clone();
        p.dialog.connect_closed(move |_| {
            if !me.answered.get() {
                me.answered.set(true);
                if let Some(d) = me.deck.upgrade() {
                    d.answer_pick(&me.req, None, false);
                }
            }
        });

        p.dialog.present(Some(&deck.window));
        let me = p.clone();
        glib::idle_add_local_once(move || {
            if let Some(r) = me.list.selected_row() {
                r.grab_focus();
            }
        });
        p
    }

    /// Re-render the rows from the deck's current accounts, keeping the selection.
    pub fn rebuild(&self) {
        let Some(deck) = self.deck.upgrade() else { return };
        self.building.set(true);
        self.list.remove_all();
        let cfg = deck.config.borrow().clone();
        let mapped = cfg.account_for(Path::new(&self.req.cwd));
        let accs = deck.accounts.borrow().clone();
        *self.names.borrow_mut() = accs.iter().map(|a| a.name.clone()).collect();
        let selected = self.selected.borrow().clone();
        for (i, acc) in accs.iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            h.add_css_class("picker-row");
            h.append(&widgets::rings(acc.five_pct(), acc.week_pct(), cfg.threshold, cfg.weekly_threshold, 34, 4.0));
            let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
            v.set_hexpand(true);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            top.append(&label(&acc.name, &["heading"]));
            if let Some(p) = acc.plan_label() {
                top.append(&widgets::badge(&p));
            }
            if mapped.as_deref() == Some(&acc.name) {
                top.append(&label("Folder default", &["folder-default"]));
            }
            v.append(&top);
            if let Some(e) = &acc.email {
                let l = label(e, &["caption", "dim-label"]);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                v.append(&l);
            }
            let sum = if acc.logged_out() {
                "Not logged in — you can log in inside the session".to_string()
            } else {
                acc.summary()
            };
            let s = label(&sum, &["caption", "numeric"]);
            s.add_css_class(if acc.over(&cfg) { "over" } else { "dim-label" });
            v.append(&s);
            h.append(&v);
            let idx = label(&(i + 1).to_string(), &["dim-label", "numeric"]);
            idx.set_valign(gtk::Align::Center);
            h.append(&idx);
            row.set_child(Some(&h));
            if acc.over(&cfg) {
                row.set_opacity(0.75);
            }
            self.list.append(&row);
            if selected.as_deref() == Some(&acc.name) {
                self.list.select_row(Some(&row));
            }
        }
        self.start.set_sensitive(selected.is_some());
        self.building.set(false);
    }

    fn choose(&self, name: &str) {
        if self.answered.get() {
            return;
        }
        self.answered.set(true);
        if let Some(d) = self.deck.upgrade() {
            d.answer_pick(&self.req, Some(name), self.remember.is_active());
        }
        self.dialog.close();
    }

    pub fn debug_choose(&self, name: &str) {
        self.choose(name);
    }

    /// The launcher went away: close without answering.
    pub fn dismiss(&self) {
        if self.answered.get() {
            return;
        }
        self.answered.set(true);
        self.dialog.close();
        if let Some(d) = self.deck.upgrade() {
            d.finish_pick();
        }
    }
}
