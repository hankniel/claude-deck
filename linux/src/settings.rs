//! Preferences: appearance, auto-switch and limit lines, accounts, folder defaults.

use crate::deck::Deck;
use crate::model;
use crate::prefs::{self, Appearance};
use crate::widgets;
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

const POLLS: [u64; 4] = [60, 120, 300, 900];

pub struct Settings {
    deck: Weak<Deck>,
    dialog: adw::PreferencesDialog,
    auto_row: adw::SwitchRow,
    accounts_group: adw::PreferencesGroup,
    account_rows: RefCell<Vec<gtk::Widget>>,
    folders_group: adw::PreferencesGroup,
    folder_rows: RefCell<Vec<gtk::Widget>>,
    folder_account: adw::ComboRow,
    folder_names: RefCell<Vec<String>>,
    loading: Cell<bool>,
}

impl Settings {
    pub fn show(deck: &Rc<Deck>, page: Option<&str>) {
        if let Some(s) = deck.settings.borrow().clone() {
            if let Some(p) = page {
                s.dialog.set_visible_page_name(p);
            }
            return;
        }
        let s = Self::build(deck);
        if let Some(p) = page {
            s.dialog.set_visible_page_name(p);
        }
        *deck.settings.borrow_mut() = Some(s.clone());
        let d = Rc::downgrade(deck);
        s.dialog.connect_closed(move |_| {
            if let Some(d) = d.upgrade() {
                *d.settings.borrow_mut() = None;
            }
        });
        s.dialog.present(Some(&deck.window));
    }

    fn build(deck: &Rc<Deck>) -> Rc<Settings> {
        let cfg = deck.config.borrow().clone();
        let dialog = adw::PreferencesDialog::new();
        dialog.set_search_enabled(false);

        // ----- General -----
        let general = adw::PreferencesPage::builder().title("General").icon_name("emblem-system-symbolic").name("general").build();

        let win_group = adw::PreferencesGroup::builder().title("Window").build();
        let appearance = adw::ComboRow::builder()
            .title("Appearance")
            .subtitle("System follows the desktop's light/dark style")
            .model(&gtk::StringList::new(&["System", "Light", "Dark"]))
            .build();
        appearance.set_selected(match prefs::appearance() {
            Appearance::System => 0,
            Appearance::Light => 1,
            Appearance::Dark => 2,
        });
        appearance.connect_selected_notify(|r| {
            let a = match r.selected() {
                1 => Appearance::Light,
                2 => Appearance::Dark,
                _ => Appearance::System,
            };
            prefs::set_appearance(a);
            prefs::apply_appearance(a);
        });
        win_group.add(&appearance);
        general.add(&win_group);

        let auto_group = adw::PreferencesGroup::builder().title("Auto-switch").build();
        let auto_row = adw::SwitchRow::builder()
            .title("Move idle Claude sessions near a limit")
            .subtitle("The conversation resumes on the account with the most room left. Without it, CC Deck only warns you.")
            .active(cfg.auto_switch)
            .build();
        auto_group.add(&auto_row);
        general.add(&auto_group);

        let lines = adw::PreferencesGroup::builder().title("Limit lines").build();
        let spin = |title: &str, sub: &str, lo: f64, hi: f64, step: f64, v: f64| {
            let r = adw::SpinRow::with_range(lo, hi, step);
            r.set_title(title);
            r.set_subtitle(sub);
            r.set_value(v);
            lines.add(&r);
            r
        };
        let five = spin("5-hour line", "Percent of the 5-hour window", 50.0, 100.0, 5.0, cfg.threshold);
        let week = spin("7-day line", "Percent of the 7-day window", 50.0, 100.0, 1.0, cfg.weekly_threshold);
        let idle = spin("Idle before moving", "Seconds without input or output", 2.0, 120.0, 2.0, cfg.idle_secs as f64);
        let poll = adw::ComboRow::builder()
            .title("Check usage every")
            .model(&gtk::StringList::new(&["1 minute", "2 minutes", "5 minutes", "15 minutes"]))
            .build();
        poll.set_selected(POLLS.iter().position(|p| *p >= cfg.poll_secs).unwrap_or(1) as u32);
        lines.add(&poll);
        general.add(&lines);

        let shell = adw::PreferencesGroup::builder()
            .title("Shell")
            .description("In CC Deck terminals, `claude` asks which account to use and `claude-deck` is on the PATH.")
            .build();
        let cli = adw::ActionRow::builder()
            .title("Command-line tool")
            .subtitle(match &deck.cli {
                Some(p) => p.to_string_lossy().to_string(),
                None => "Not found — run claude-deck/linux/build.sh to install it".into(),
            })
            .subtitle_selectable(true)
            .build();
        shell.add(&cli);
        let sh = adw::ActionRow::builder()
            .title("Shell")
            .subtitle(crate::shell::user_shell())
            .build();
        shell.add(&sh);
        general.add(&shell);
        dialog.add(&general);

        // ----- Accounts -----
        let accounts_page = adw::PreferencesPage::builder().title("Accounts").icon_name("system-users-symbolic").name("accounts").build();
        let accounts_group = adw::PreferencesGroup::builder()
            .title("Accounts")
            .description("“default” is your usual ~/.claude login. Other accounts are separate Claude Code profiles that share your settings, skills and history.")
            .build();
        accounts_page.add(&accounts_group);
        let add_group = adw::PreferencesGroup::builder().title("Add an account").build();
        let entry = adw::EntryRow::builder().title("Name, e.g. personal").show_apply_button(true).build();
        add_group.add(&entry);
        accounts_page.add(&add_group);
        dialog.add(&accounts_page);

        // ----- Folders -----
        let folders_page = adw::PreferencesPage::builder().title("Folders").icon_name("folder-symbolic").name("folders").build();
        let folders_group = adw::PreferencesGroup::builder()
            .title("Folder defaults")
            .description("The account picker pre-selects a folder's default. Subfolders inherit it.")
            .build();
        folders_page.add(&folders_group);
        let fadd = adw::PreferencesGroup::builder().title("Add").build();
        let folder_account = adw::ComboRow::builder().title("Account").build();
        fadd.add(&folder_account);
        let choose = adw::ActionRow::builder().title("Choose Folder…").activatable(true).build();
        choose.add_prefix(&gtk::Image::from_icon_name("folder-open-symbolic"));
        choose.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        fadd.add(&choose);
        folders_page.add(&fadd);
        dialog.add(&folders_page);

        let s = Rc::new(Settings {
            deck: Rc::downgrade(deck),
            dialog,
            auto_row,
            accounts_group,
            account_rows: RefCell::new(vec![]),
            folders_group,
            folder_rows: RefCell::new(vec![]),
            folder_account,
            folder_names: RefCell::new(vec![]),
            loading: Cell::new(false),
        });
        s.reload();

        let me = Rc::downgrade(&s);
        s.auto_row.connect_active_notify(move |r| {
            let Some(me) = me.upgrade() else { return };
            if me.loading.get() {
                return;
            }
            let on = r.is_active();
            if let Some(d) = me.deck.upgrade() {
                d.update_config(|c| c.auto_switch = on);
            }
        });
        let bind = |row: &adw::SpinRow, f: fn(&mut claude_deck::config::Config, f64)| {
            let d = Rc::downgrade(deck);
            row.connect_value_notify(move |r| {
                let v = r.value();
                if let Some(d) = d.upgrade() {
                    d.update_config(|c| f(c, v));
                }
            });
        };
        bind(&five, |c, v| c.threshold = v);
        bind(&week, |c, v| c.weekly_threshold = v);
        bind(&idle, |c, v| c.idle_secs = v as u64);
        let d = Rc::downgrade(deck);
        poll.connect_selected_notify(move |r| {
            let v = POLLS[(r.selected() as usize).min(POLLS.len() - 1)];
            if let Some(d) = d.upgrade() {
                d.update_config(|c| c.poll_secs = v);
            }
        });

        let me = Rc::downgrade(&s);
        entry.connect_apply(move |e| {
            let Some(me) = me.upgrade() else { return };
            let Some(d) = me.deck.upgrade() else { return };
            let name = e.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            match d.add_account(&name) {
                Ok(()) => {
                    e.set_text("");
                    me.dialog.close();
                    d.login(&name);
                }
                Err(err) => {
                    me.dialog.add_toast(adw::Toast::new(&err));
                }
            }
        });

        let me = Rc::downgrade(&s);
        choose.connect_activated(move |_| {
            let Some(me) = me.upgrade() else { return };
            let Some(d) = me.deck.upgrade() else { return };
            let idx = me.folder_account.selected() as usize;
            let Some(account) = me.folder_names.borrow().get(idx).cloned() else { return };
            let fd = gtk::FileDialog::builder().title("Choose a folder").modal(true).build();
            let me2 = Rc::downgrade(&me);
            fd.select_folder(Some(&d.window), None::<&gio::Cancellable>, move |res| {
                let (Some(me), Ok(file)) = (me2.upgrade(), res) else { return };
                let (Some(d), Some(path)) = (me.deck.upgrade(), file.path()) else { return };
                d.map_dir(&account, &path);
            });
        });
        s
    }

    /// Rebuild the account and folder lists from the deck's current state.
    pub fn reload(self: &Rc<Self>) {
        let Some(deck) = self.deck.upgrade() else { return };
        self.loading.set(true);
        let cfg = deck.config.borrow().clone();
        self.auto_row.set_active(cfg.auto_switch);

        for w in self.account_rows.borrow_mut().drain(..) {
            self.accounts_group.remove(&w);
        }
        for acc in deck.accounts.borrow().iter() {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&acc.name).as_str())
                .subtitle(glib::markup_escape_text(&acc.email.clone().unwrap_or_else(|| acc.state_label())).as_str())
                .build();
            row.add_prefix(&widgets::rings(acc.five_pct(), acc.week_pct(), cfg.threshold, cfg.weekly_threshold, 30, 3.5));
            if let Some(p) = acc.plan_label() {
                row.add_suffix(&widgets::badge(&p));
            }
            if acc.needs_login() {
                let b = gtk::Button::with_label("Log in");
                b.set_valign(gtk::Align::Center);
                let me = Rc::downgrade(self);
                let name = acc.name.clone();
                b.connect_clicked(move |_| {
                    let Some(me) = me.upgrade() else { return };
                    me.dialog.close();
                    if let Some(d) = me.deck.upgrade() {
                        d.login(&name);
                    }
                });
                row.add_suffix(&b);
            }
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            sw.set_active(!acc.disabled);
            sw.set_tooltip_text(Some("Allow auto-switch to move sessions onto this account"));
            let d = Rc::downgrade(&deck);
            let name = acc.name.clone();
            sw.connect_state_set(move |_, on| {
                if let Some(d) = d.upgrade() {
                    let name = name.clone();
                    glib::idle_add_local_once(move || d.set_disabled(&name, !on));
                }
                glib::Propagation::Proceed
            });
            row.add_suffix(&sw);
            if acc.name != claude_deck::accounts::DEFAULT {
                let b = gtk::Button::from_icon_name("user-trash-symbolic");
                b.add_css_class("flat");
                b.set_valign(gtk::Align::Center);
                b.set_tooltip_text(Some("Remove account"));
                let me = Rc::downgrade(self);
                let name = acc.name.clone();
                b.connect_clicked(move |_| {
                    if let Some(me) = me.upgrade() {
                        me.confirm_remove(&name);
                    }
                });
                row.add_suffix(&b);
            }
            self.accounts_group.add(&row);
            self.account_rows.borrow_mut().push(row.upcast());
        }

        for w in self.folder_rows.borrow_mut().drain(..) {
            self.folders_group.remove(&w);
        }
        if cfg.maps.is_empty() {
            let r = adw::ActionRow::builder().title("No folder defaults yet").build();
            r.add_css_class("dim-label");
            self.folders_group.add(&r);
            self.folder_rows.borrow_mut().push(r.upcast());
        }
        for (dir, acc) in &cfg.maps {
            let r = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&model::short_path(dir)).as_str())
                .subtitle(glib::markup_escape_text(acc).as_str())
                .build();
            r.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let b = gtk::Button::from_icon_name("list-remove-symbolic");
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some("Remove folder default"));
            let d = Rc::downgrade(&deck);
            let dir = dir.clone();
            b.connect_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    let dir = dir.clone();
                    glib::idle_add_local_once(move || d.unmap_dir(&dir));
                }
            });
            r.add_suffix(&b);
            self.folders_group.add(&r);
            self.folder_rows.borrow_mut().push(r.upcast());
        }

        let names = deck.account_names();
        if *self.folder_names.borrow() != names {
            let keep = self.folder_names.borrow().get(self.folder_account.selected() as usize).cloned();
            let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            self.folder_account.set_model(Some(&gtk::StringList::new(&refs)));
            let i = keep.and_then(|k| names.iter().position(|n| *n == k)).unwrap_or(0);
            self.folder_account.set_selected(i as u32);
            *self.folder_names.borrow_mut() = names;
        }
        self.loading.set(false);
    }

    fn confirm_remove(self: &Rc<Self>, name: &str) {
        let alert = adw::AlertDialog::new(
            Some(&format!("Remove {name}?")),
            Some("Deletes the profile and its stored login. Shared settings, skills and history stay."),
        );
        alert.add_response("cancel", "Cancel");
        alert.add_response("remove", "Remove");
        alert.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let d = self.deck.clone();
        let name = name.to_string();
        alert.connect_response(None, move |_, resp| {
            if resp == "remove" {
                if let Some(d) = d.upgrade() {
                    d.remove_account(&name);
                }
            }
        });
        alert.present(Some(&self.dialog));
    }
}
