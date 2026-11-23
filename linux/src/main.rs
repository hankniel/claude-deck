//! CC Deck for Linux (GTK 4 + libadwaita + VTE).
//!
//! A terminal app: typing `claude` in a tab asks which account to run it on (through
//! `claude-deck launch`), the session can be moved to another account and resumed,
//! usage is shown per account, and idle sessions can auto-switch near a limit.

mod bg;
mod canvas;
mod control;
mod dashboard;
mod deck;
mod files;
mod jira;
mod model;
mod picker;
mod prefs;
mod settings;
mod shell;
mod term;
mod theme;
mod widgets;
mod workbench;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

pub const APP_ID: &str = "dev.claudedeck.CCDeck";

thread_local! {
    static DECK: RefCell<Option<Rc<deck::Deck>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    // Tabs must never inherit an account from the shell that started the app.
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("CLAUDE_DECK_ACCOUNT");

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(if std::env::var_os("CC_DECK_SNAPSHOT").is_some() {
            // a snapshot run must not hand over to an already open CC Deck
            gio::ApplicationFlags::NON_UNIQUE
        } else {
            gio::ApplicationFlags::empty()
        })
        .build();

    app.connect_startup(|app| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(widgets::CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        prefs::apply_appearance(prefs::appearance());
        gtk::Window::set_default_icon_name(APP_ID);

        let quit = gio::SimpleAction::new("quit", None);
        let a = app.clone();
        quit.connect_activate(move |_, _| {
            DECK.with(|d| {
                if let Some(d) = d.borrow().as_ref() {
                    d.shutdown();
                }
            });
            a.quit();
        });
        app.add_action(&quit);
    });

    app.connect_activate(|app| {
        let existing = DECK.with(|d| d.borrow().clone());
        if let Some(d) = existing {
            d.window.present();
            return;
        }
        let d = deck::Deck::new(app);
        d.window.present();
        snapshot_if_requested(&d);
        DECK.with(|cell| *cell.borrow_mut() = Some(d));
    });

    app.connect_shutdown(|_| {
        DECK.with(|d| {
            if let Some(d) = d.borrow().as_ref() {
                d.shutdown();
            }
        });
    });

    app.run()
}

const SHORTCUTS_UI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<interface>
  <object class="GtkShortcutsWindow" id="shortcuts">
    <property name="modal">1</property>
    <child>
      <object class="GtkShortcutsSection">
        <property name="section-name">main</property>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Terminals</property>
            <child><object class="GtkShortcutsShortcut"><property name="title">New terminal</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;t</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">New terminal in home folder</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;&lt;Alt&gt;t</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Close tab</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;w</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Next / previous terminal</property><property name="accelerator">&lt;Ctrl&gt;Page_Down &lt;Ctrl&gt;Page_Up</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Go to terminal 1–9</property><property name="accelerator">&lt;Alt&gt;1...9</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Copy / paste</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;c &lt;Ctrl&gt;&lt;Shift&gt;v</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Find in terminal (Enter: older, Shift+Enter: newer)</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;f</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Zoom in / out / reset</property><property name="accelerator">&lt;Ctrl&gt;plus &lt;Ctrl&gt;minus &lt;Ctrl&gt;0</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Newline in Claude's prompt</property><property name="accelerator">&lt;Shift&gt;Return</property></object></child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Claude</property>
            <child><object class="GtkShortcutsShortcut"><property name="title">Start Claude in this terminal</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;k</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Refresh usage</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;r</property></object></child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Layout</property>
            <child><object class="GtkShortcutsShortcut"><property name="title">Show or hide the panel</property><property name="accelerator">&lt;Ctrl&gt;grave</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Show or hide the side bar</property><property name="accelerator">F9 &lt;Ctrl&gt;&lt;Shift&gt;b</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Explorer</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;e</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Go to an open tab</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;p</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Board</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;o</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Jira</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;j</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Dashboard</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;d</property></object></child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Window</property>
            <child><object class="GtkShortcutsShortcut"><property name="title">Settings</property><property name="accelerator">&lt;Ctrl&gt;comma</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Keyboard shortcuts</property><property name="accelerator">&lt;Ctrl&gt;question</property></object></child>
            <child><object class="GtkShortcutsShortcut"><property name="title">Quit</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;q</property></object></child>
          </object>
        </child>
      </object>
    </child>
  </object>
</interface>
"#;

/// Development aid: CC_DECK_SNAPSHOT=out.png renders the window to PNG and quits.
/// CC_DECK_SNAPSHOT_MODE: terminal (default), dashboard, settings, picker.
fn snapshot_if_requested(d: &Rc<deck::Deck>) {
    let Ok(out) = std::env::var("CC_DECK_SNAPSHOT") else { return };
    let mode = std::env::var("CC_DECK_SNAPSHOT_MODE").unwrap_or_default();
    let d = d.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
        match mode.as_str() {
            "dashboard" => d.select(deck::Sel::Dashboard),
            "settings" => settings::Settings::show(&d, None),
            "picker" => d.debug_pick(),
            "board" => d.select(deck::Sel::Board),
            m if m.starts_with("layout:") => d.debug_layout(&m[7..]),
            "jira" => d.select(deck::Sel::Jira),
            m if m.starts_with("changes:") => {
                d.select(deck::Sel::Canvas);
                d.canvas.open_changes(std::path::Path::new(&m[8..]), "repo");
            }
            m if m.starts_with("canvas") => {
                d.select(deck::Sel::Canvas);
                if let Some((sid, name)) = m.strip_prefix("canvas:").and_then(|r| r.split_once('/')) {
                    d.canvas.open_screen(sid, name, true);
                }
            }
            "claude" => {
                d.debug_pick();
                let d2 = d.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || d2.debug_choose_first());
            }
            m if m.starts_with("dash") => {
                d.select(deck::Sel::Dashboard);
                let frac: f64 = m.trim_start_matches("dash").parse().unwrap_or(0.0);
                let d2 = d.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || d2.debug_dash_scroll(frac));
            }
            _ => {}
        }
        glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
            let w = &d.window;
            let (width, height) = (w.width(), w.height());
            let paintable = gtk::WidgetPaintable::new(Some(w));
            let snap = gtk::Snapshot::new();
            paintable.snapshot(&snap, width as f64, height as f64);
            if let (Some(node), Some(native)) = (snap.to_node(), w.native()) {
                let tex = native.renderer().unwrap().render_texture(&node, None);
                if let Err(e) = tex.save_to_png(&out) {
                    eprintln!("snapshot: {e}");
                }
            } else {
                eprintln!("snapshot: nothing rendered ({width}x{height})");
            }
            d.shutdown();
            d.app.quit();
        });
    });
}

pub fn show_shortcuts(parent: &adw::ApplicationWindow) {
    let b = gtk::Builder::from_string(SHORTCUTS_UI);
    if let Some(w) = b.object::<gtk::ShortcutsWindow>("shortcuts") {
        w.set_transient_for(Some(parent));
        w.present();
    }
}
