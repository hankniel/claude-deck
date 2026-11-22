//! Workbench look: theme presets (each with a light and a dark palette) applied over
//! libadwaita's named colours, an optional background image behind the editor area, fonts,
//! and the styles of the workbench chrome (activity bar, tabs, panel, status bar).

use crate::prefs;
use gtk::gdk;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};

/// Palette of one appearance. Hex RGB.
pub struct Palette {
    pub chrome: u32,
    pub editor: u32,
    pub border: u32,
    pub hover: u32,
    pub fg: u32,
    pub strong: u32,
    pub dim: u32,
    pub accent: u32,
    pub accent_press: u32,
    pub accent_fg: u32,
    /// ANSI colours 0–15 for terminals.
    pub term: [u32; 16],
}

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: Palette,
    pub light: Palette,
}

const fn pal(c: [u32; 10], term: [u32; 16]) -> Palette {
    Palette {
        chrome: c[0],
        editor: c[1],
        border: c[2],
        hover: c[3],
        fg: c[4],
        strong: c[5],
        dim: c[6],
        accent: c[7],
        accent_press: c[8],
        accent_fg: c[9],
        term,
    }
}

//                chrome    editor    border    hover     fg        strong    dim       accent    press     on-accent
pub const PRESETS: [Preset; 6] = [
    Preset {
        id: "claude",
        name: "Claude (warm)",
        dark: pal(
            [0x181715, 0x1f1e1b, 0x2d2a26, 0x252320, 0xe4e0d8, 0xfaf9f5, 0xa09d96, 0xd97757, 0xe08a6d, 0x1f1e1b],
            [0x3a3a38, 0xf0745f, 0x7cc27e, 0xe6be5a, 0x6fa4e8, 0xc795e0, 0x63c3c0, 0xd4d4d0, 0x7a7a76, 0xff8d78, 0x96d497, 0xf2cf76, 0x8db9f0, 0xd8aeeb, 0x82d4d1, 0xf5f5f2],
        ),
        light: pal(
            [0xf3efe7, 0xfaf9f5, 0xe3dcd2, 0xebe5da, 0x3d3d3a, 0x141413, 0x6c6a64, 0xcc785c, 0xa9583e, 0xffffff],
            [0x1d1d1f, 0xc0392b, 0x2e7d32, 0x9a6700, 0x2563b8, 0x8e44ad, 0x16808a, 0xbdbdb8, 0x6e6e73, 0xd9534f, 0x3c9a40, 0xb07d12, 0x3b7dd8, 0xa35cc2, 0x1f99a3, 0x8e8e93],
        ),
    },
    Preset {
        id: "vscode",
        name: "VS Code Modern",
        dark: pal(
            [0x181818, 0x1f1f1f, 0x2b2b2b, 0x2a2d2e, 0xcccccc, 0xffffff, 0x9d9d9d, 0x0078d4, 0x4daafc, 0xffffff],
            [0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xe5e5e5],
        ),
        light: pal(
            [0xf8f8f8, 0xffffff, 0xe5e5e5, 0xf2f2f2, 0x3b3b3b, 0x1f1f1f, 0x6f6f6f, 0x005fb8, 0x0258a8, 0xffffff],
            [0x000000, 0xcd3131, 0x107c10, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555, 0x666666, 0xcd3131, 0x14ce14, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5],
        ),
    },
    Preset {
        id: "nord",
        name: "Nord",
        dark: pal(
            [0x272c36, 0x2e3440, 0x3b4252, 0x353c4a, 0xd8dee9, 0xeceff4, 0x8c97ad, 0x88c0d0, 0x8fbcbb, 0x2e3440],
            [0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, 0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4],
        ),
        light: pal(
            [0xe5e9f0, 0xeceff4, 0xd3d9e3, 0xdfe3ea, 0x2e3440, 0x242933, 0x4c566a, 0x5e81ac, 0x4c6d96, 0xffffff],
            [0x3b4252, 0xbf616a, 0x6b8f4e, 0xb8860b, 0x5e81ac, 0xb48ead, 0x4f8e9b, 0xd8dee9, 0x4c566a, 0xbf616a, 0x6b8f4e, 0xb8860b, 0x5e81ac, 0xb48ead, 0x4f8e9b, 0xe5e9f0],
        ),
    },
    Preset {
        id: "solarized",
        name: "Solarized",
        dark: pal(
            [0x00212b, 0x002b36, 0x0a3f4c, 0x073642, 0x93a1a1, 0xeee8d5, 0x657b83, 0x268bd2, 0x2aa198, 0xfdf6e3],
            [0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0xfdf6e3],
        ),
        light: pal(
            [0xeee8d5, 0xfdf6e3, 0xe0d8c0, 0xf5efdc, 0x586e75, 0x073642, 0x839496, 0x268bd2, 0x1f6fa8, 0xffffff],
            [0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0x93a1a1, 0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0x839496],
        ),
    },
    Preset {
        id: "github",
        name: "GitHub",
        dark: pal(
            [0x010409, 0x0d1117, 0x30363d, 0x161b22, 0xc9d1d9, 0xf0f6fc, 0x8b949e, 0x2f81f7, 0x58a6ff, 0xffffff],
            [0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4, 0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff],
        ),
        light: pal(
            [0xf6f8fa, 0xffffff, 0xd0d7de, 0xeaeef2, 0x1f2328, 0x0d1117, 0x656d76, 0x0969da, 0x0550ae, 0xffffff],
            [0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f],
        ),
    },
    Preset {
        id: "catppuccin",
        name: "Catppuccin",
        dark: pal(
            [0x181825, 0x1e1e2e, 0x313244, 0x28283c, 0xcdd6f4, 0xf5f7ff, 0xa6adc8, 0xcba6f7, 0xb4befe, 0x1e1e2e],
            [0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8],
        ),
        light: pal(
            [0xe6e9ef, 0xeff1f5, 0xccd0da, 0xdce0e8, 0x4c4f69, 0x1e1e2e, 0x6c6f85, 0x8839ef, 0x7287fd, 0xffffff],
            [0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be, 0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc],
        ),
    },
];

thread_local! {
    static PRESET: Cell<usize> = const { Cell::new(0) };
    static WATCHERS: RefCell<Vec<Box<dyn Fn() -> bool>>> = const { RefCell::new(vec![]) };
    static COLORS: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static BG: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static SYSTEM_FONT: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn preset_index() -> usize {
    PRESET.with(|p| p.get())
}

pub fn palette(dark: bool) -> &'static Palette {
    let p = &PRESETS[preset_index()];
    if dark {
        &p.dark
    } else {
        &p.light
    }
}

/// Call `f` after every look change (theme, light/dark, image, fonts) while it returns true.
pub fn watch(f: impl Fn() -> bool + 'static) {
    WATCHERS.with(|w| w.borrow_mut().push(Box::new(f)));
}

fn notify() {
    let list = WATCHERS.with(|w| std::mem::take(&mut *w.borrow_mut()));
    let keep: Vec<Box<dyn Fn() -> bool>> = list.into_iter().filter(|f| f()).collect();
    WATCHERS.with(|w| {
        let mut w = w.borrow_mut();
        let added = std::mem::take(&mut *w);
        *w = keep;
        w.extend(added);
    });
}

pub fn set_preset(id: &str) {
    let i = PRESETS.iter().position(|p| p.id == id).unwrap_or(0);
    PRESET.with(|p| p.set(i));
    prefs::set("theme", PRESETS[i].id);
    apply(crate::widgets::is_dark());
}

// ---------- background image ----------

pub fn bg_image() -> Option<String> {
    prefs::get("bg.image").filter(|p| !p.is_empty() && std::path::Path::new(p).is_file())
}

/// How much of the image shows through, 0.05–0.6.
pub fn bg_strength() -> f64 {
    prefs::get("bg.strength").and_then(|s| s.parse().ok()).unwrap_or(0.22f64).clamp(0.05, 0.6)
}

pub fn bg_active() -> bool {
    BG.with(|b| b.borrow().is_some())
}

pub fn set_bg_image(path: Option<&str>) {
    prefs::set("bg.image", path.unwrap_or(""));
    apply_bg(crate::widgets::is_dark());
    notify();
}

pub fn set_bg_strength(v: f64) {
    prefs::set("bg.strength", &format!("{:.2}", v.clamp(0.05, 0.6)));
    apply_bg(crate::widgets::is_dark());
    notify();
}

fn apply_bg(dark: bool) {
    let Some(display) = gdk::Display::default() else { return };
    BG.with(|b| {
        if let Some(old) = b.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
    });
    let Some(path) = bg_image() else { return };
    let editor = palette(dark).editor;
    let (r, g, bl) = ((editor >> 16) & 0xff, (editor >> 8) & 0xff, editor & 0xff);
    let veil = 1.0 - bg_strength();
    let url = gtk::gio::File::for_path(&path).uri();
    let css = format!(
        r#"
.wb-editor-area {{
  background-image: linear-gradient(rgba({r},{g},{bl},{veil:.2}), rgba({r},{g},{bl},{veil:.2})), url("{url}");
  background-size: cover, cover;
  background-position: center, center;
  background-repeat: no-repeat;
}}
.wb-editor-area .wb-group:not(.panel), .wb-editor-area .wb-crumb, .wb-editor-area stack, .wb-editor-area scrolledwindow,
.wb-editor-area viewport {{ background: none; }}
"#
    );
    let p = gtk::CssProvider::new();
    p.load_from_string(&css);
    gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
    BG.with(|b| *b.borrow_mut() = Some(p));
}

// ---------- fonts ----------

/// A font chosen in Settings ("ui", "mono", "diagram"), as a Pango description string.
pub fn font(kind: &str) -> Option<String> {
    prefs::get(&format!("font.{kind}")).filter(|s| !s.is_empty())
}

pub fn mono_font() -> gtk::pango::FontDescription {
    gtk::pango::FontDescription::from_string(&font("mono").unwrap_or_else(|| "JetBrains Mono, Ubuntu Mono, monospace 11".into()))
}

pub fn set_font(kind: &str, desc: Option<&str>) {
    prefs::set(&format!("font.{kind}"), desc.unwrap_or(""));
    apply_fonts();
    notify();
}

fn apply_fonts() {
    let Some(settings) = gtk::Settings::default() else { return };
    SYSTEM_FONT.with(|s| {
        if s.borrow().is_none() {
            *s.borrow_mut() = settings.gtk_font_name().map(|f| f.to_string());
        }
    });
    let ui = font("ui").or_else(|| SYSTEM_FONT.with(|s| s.borrow().clone()));
    if let Some(f) = ui {
        settings.set_gtk_font_name(Some(&f));
    }
}

fn colors_css(p: &Palette, dark: bool) -> String {
    let c = |h: u32| format!("#{h:06x}");
    let (chrome, editor, border, hover, fg, strong, dim) =
        (c(p.chrome), c(p.editor), c(p.border), c(p.hover), c(p.fg), c(p.strong), c(p.dim));
    let (accent, press, afg) = (c(p.accent), c(p.accent_press), c(p.accent_fg));
    let shade = if dark { "rgba(0,0,0,0.36)" } else { "rgba(0,0,0,0.07)" };
    format!(
        r#"
@define-color wb_chrome {chrome};
@define-color wb_editor {editor};
@define-color wb_border {border};
@define-color wb_hover {hover};
@define-color wb_fg {fg};
@define-color wb_strong {strong};
@define-color wb_dim {dim};
@define-color wb_accent {accent};
@define-color wb_accent_press {press};
@define-color accent_bg_color {accent};
@define-color accent_color {press};
@define-color accent_fg_color {afg};
@define-color window_bg_color {editor};
@define-color window_fg_color {fg};
@define-color view_bg_color {editor};
@define-color view_fg_color {fg};
@define-color headerbar_bg_color {chrome};
@define-color headerbar_fg_color {fg};
@define-color headerbar_border_color {border};
@define-color headerbar_backdrop_color {chrome};
@define-color headerbar_shade_color {shade};
@define-color sidebar_bg_color {chrome};
@define-color sidebar_fg_color {fg};
@define-color sidebar_backdrop_color {chrome};
@define-color sidebar_shade_color {shade};
@define-color secondary_sidebar_bg_color {chrome};
@define-color card_bg_color {hover};
@define-color card_fg_color {fg};
@define-color popover_bg_color {chrome};
@define-color popover_fg_color {fg};
@define-color dialog_bg_color {editor};
@define-color dialog_fg_color {fg};
@define-color shade_color {shade};
"#
    )
}

const BASE: &str = r#"
window.wb, window.wb .wb-body { background-color: @wb_editor; color: @wb_fg; }

/* ---- title bar ---- */
.wb-titlebar { min-height: 36px; padding: 0 4px; background-color: @wb_chrome; box-shadow: inset 0 -1px @wb_border; }
.wb-titlebar headerbar, headerbar.wb-titlebar { min-height: 36px; }
.wb-titlebar button { min-height: 26px; min-width: 26px; padding: 2px 5px; border-radius: 5px; }
.wb-brand { font-weight: 700; font-size: 13px; color: @wb_strong; margin: 0 6px 0 4px; }
.wb-command { min-height: 24px; min-width: 340px; padding: 0 12px; border-radius: 6px; font-size: 12.5px;
  background-color: alpha(@wb_fg, 0.06); color: @wb_dim; box-shadow: inset 0 0 0 1px @wb_border; }
.wb-command:hover { background-color: alpha(@wb_fg, 0.10); color: @wb_fg; }
.wb-layout-btn { color: @wb_dim; }
.wb-layout-btn:checked { color: @wb_strong; background-color: alpha(@wb_fg, 0.10); }

/* ---- activity bar ---- */
.wb-activity { background-color: @wb_chrome; box-shadow: inset -1px 0 @wb_border; padding: 4px 0; min-width: 48px; }
.wb-act { min-width: 48px; min-height: 44px; padding: 0; border-radius: 0; background: none; color: @wb_dim; }
.wb-act image { -gtk-icon-size: 20px; }
.wb-act:hover { color: @wb_strong; background: none; }
.wb-act.on { color: @wb_strong; box-shadow: inset 2px 0 @wb_accent; }
.wb-badge { font-size: 9.5px; font-weight: 800; min-width: 15px; min-height: 15px; padding: 0 3px; border-radius: 999px;
  background-color: @wb_accent; color: @accent_fg_color; margin: 7px 7px 0 0; }

/* ---- side bar ---- */
.wb-side { background-color: @wb_chrome; }
.wb-side-title { font-size: 11px; font-weight: 600; letter-spacing: 1px; color: @wb_dim; margin: 0 8px 0 18px; min-height: 35px; }
.wb-section { min-height: 24px; padding: 0 4px 0 2px; border-radius: 0; font-size: 11px; font-weight: 700; letter-spacing: 0.6px;
  color: @wb_fg; background: none; box-shadow: inset 0 1px @wb_border; }
.wb-section:hover { background-color: alpha(@wb_fg, 0.04); }
.wb-section image { -gtk-icon-size: 12px; color: @wb_dim; }
.wb-section-tool { min-height: 20px; min-width: 20px; padding: 0; border-radius: 4px; color: @wb_dim; }
.wb-section-tool:hover { color: @wb_strong; background-color: alpha(@wb_fg, 0.08); }
.wb-tree, .wb-tree row { background: none; }
.wb-tree row { min-height: 22px; padding: 0 8px 0 0; border-radius: 0; margin: 0; color: @wb_fg; font-size: 13px; }
.wb-tree row:hover { background-color: alpha(@wb_fg, 0.05); }
.wb-tree row:selected { background-color: alpha(@wb_accent, 0.16); color: @wb_strong; }
.wb-tree row:selected:focus-within { box-shadow: inset 0 0 0 1px alpha(@wb_accent, 0.55); }
.wb-tree .dim, .wb-dim { color: @wb_dim; }
.wb-tree .meta { color: @wb_dim; font-size: 11.5px; }
.wb-tree .warn { color: #e09b45; font-size: 11.5px; }
.wb-tree image { -gtk-icon-size: 14px; }
.wb-empty-note { color: @wb_dim; font-size: 12px; margin: 6px 18px 10px 22px; }
.wb-account { padding: 6px 12px 6px 18px; }
.wb-account .name { font-weight: 600; color: @wb_strong; font-size: 13px; }
.wb-account .meta { color: @wb_dim; font-size: 11.5px; }

/* ---- editor / panel groups ---- */
.wb-group { background-color: @wb_editor; }
.wb-group.panel { background-color: @wb_chrome; }
.wb-strip { background-color: @wb_chrome; min-height: 35px; }
.wb-strip-fill { box-shadow: inset 0 -1px @wb_border; }
.wb-tab { min-height: 35px; padding: 0 6px 0 12px; color: @wb_dim; font-size: 13px;
  box-shadow: inset -1px 0 @wb_border, inset 0 -1px @wb_border; background-color: @wb_chrome; }
.wb-tab:hover { background-color: @wb_hover; }
.wb-tab.active { background-color: @wb_editor; color: @wb_strong; box-shadow: inset -1px 0 @wb_border, inset 0 1px @wb_accent; }
.wb-tab.attention label { font-style: italic; }
.wb-tab .tab-icon { -gtk-icon-size: 14px; color: @wb_dim; }
.wb-tab.active .tab-icon { color: @wb_accent; }
.wb-tab .tab-icon.claude { color: @wb_accent; }
.wb-tab .tab-close { min-width: 20px; min-height: 20px; padding: 0; border-radius: 4px; opacity: 0; color: @wb_dim; }
.wb-tab:hover .tab-close, .wb-tab.active .tab-close { opacity: 1; }
.wb-tab .tab-close:hover { background-color: alpha(@wb_fg, 0.12); color: @wb_strong; }
.wb-tab .tab-dot { min-width: 7px; min-height: 7px; border-radius: 999px; background-color: @wb_accent; margin: 0 6px; }
.wb-group.panel .wb-tab { background: none; box-shadow: none; padding: 0 4px 0 10px; text-transform: none; }
.wb-group.panel .wb-tab.active { color: @wb_strong; box-shadow: inset 0 -1px @wb_accent; background: none; }
.wb-group.panel .wb-strip { box-shadow: none; padding-left: 8px; }
.wb-group.panel .wb-strip-fill { box-shadow: none; }
.wb-panel-title { font-size: 11px; font-weight: 600; letter-spacing: 1px; color: @wb_dim; margin: 0 10px 0 6px; }
.wb-strip-tool { min-height: 24px; min-width: 24px; padding: 0 3px; border-radius: 5px; color: @wb_dim; background: none; }
.wb-strip-tool:hover { color: @wb_strong; background-color: alpha(@wb_fg, 0.08); }
.wb-crumb { min-height: 22px; padding: 0 14px; font-size: 12px; color: @wb_dim; background-color: @wb_editor; }
.wb-drop { background-color: alpha(@wb_accent, 0.10); box-shadow: inset 0 0 0 2px alpha(@wb_accent, 0.55); }
.wb-drop-note { color: @wb_dim; font-size: 13px; }
.wb-group.focused .wb-tab.active { color: @wb_strong; }
.wb-group.panel.focused .wb-tab.active { box-shadow: inset 0 -1px @wb_accent; }
.wb-workarea { box-shadow: none; }
.wb-panel-border-top { box-shadow: inset 0 1px @wb_border; }
.wb-panel-border-left { box-shadow: inset 1px 0 @wb_border; }

/* ---- welcome ---- */
.wb-welcome-title { font-size: 26px; font-weight: 300; color: @wb_strong; }
.wb-welcome-sub { color: @wb_dim; font-size: 13px; }
.wb-welcome-head { font-size: 11px; font-weight: 700; letter-spacing: 1px; color: @wb_dim; margin-top: 18px; }
.wb-link { padding: 3px 0; min-height: 0; background: none; color: @wb_accent; font-size: 13px; }
.wb-link:hover { color: @wb_accent_press; background: none; }
.wb-key { font-family: "JetBrains Mono", monospace; font-size: 11.5px; padding: 1px 6px; border-radius: 4px;
  background-color: alpha(@wb_fg, 0.07); box-shadow: inset 0 -1px alpha(@wb_fg, 0.12); color: @wb_fg; }

/* ---- status bar ---- */
.wb-status { min-height: 24px; background-color: @wb_chrome; box-shadow: inset 0 1px @wb_border; font-size: 12px; color: @wb_dim; }
.wb-status button, .wb-status menubutton > button { min-height: 22px; padding: 0 8px; border-radius: 0; background: none; color: @wb_dim; font-size: 12px; font-weight: 400; }
.wb-status button:hover, .wb-status menubutton > button:hover { background-color: alpha(@wb_fg, 0.08); color: @wb_strong; }
.wb-status image { -gtk-icon-size: 13px; }
.wb-status .warn { color: #e09b45; }
.wb-status .claude { color: @wb_accent; }
.wb-status .over { color: #e5484d; }
.wb-status .home { background-color: @wb_accent; color: @accent_fg_color; padding: 0 10px; }
.wb-status .home:hover { background-color: @wb_accent_press; color: @accent_fg_color; }

/* ---- splitters ---- */
paned.wb-split > separator { min-width: 1px; min-height: 1px; background-color: @wb_border; background-image: none; }
paned.wb-split.nogap > separator { background-color: transparent; }

/* terminals */
vte-terminal { padding: 6px 4px 4px 12px; }

/* lists inside pages follow the palette */
.navigation-sidebar row:selected { background-color: alpha(@wb_accent, 0.16); }
popover > contents { border-radius: 8px; }
banner > revealer > widget { background-color: alpha(#e09b45, 0.16); }
"#;

/// Install the base styles once and the chosen theme, image and fonts.
pub fn install() {
    let Some(display) = gdk::Display::default() else { return };
    let i = prefs::get("theme").and_then(|id| PRESETS.iter().position(|p| p.id == id)).unwrap_or(0);
    PRESET.with(|p| p.set(i));
    let base = gtk::CssProvider::new();
    base.load_from_string(BASE);
    gtk::style_context_add_provider_for_display(&display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    apply_fonts();
    apply(crate::widgets::is_dark());
    adw::StyleManager::default().connect_dark_notify(|sm| apply(sm.is_dark()));
}

fn apply(dark: bool) {
    let Some(display) = gdk::Display::default() else { return };
    COLORS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(old) = c.take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        let p = gtk::CssProvider::new();
        p.load_from_string(&colors_css(palette(dark), dark));
        gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        *c = Some(p);
    });
    apply_bg(dark);
    notify();
}
