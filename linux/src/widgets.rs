//! Theme colours and small drawn widgets (usage rings, bars, badges).

use gtk::prelude::*;
use gtk::{cairo, gdk};
use std::f64::consts::PI;

pub type Rgb = (f64, f64, f64);

pub const fn hex(h: u32) -> Rgb {
    (((h >> 16) & 0xff) as f64 / 255.0, ((h >> 8) & 0xff) as f64 / 255.0, (h & 0xff) as f64 / 255.0)
}

pub const ACCENT: Rgb = hex(0xCC6B4A);
pub const RED: Rgb = hex(0xE5484D);
pub const ORANGE: Rgb = hex(0xF0902E);

/// Validated categorical palette (light, dark). Series colour follows the entity's position.
const SERIES: [(u32, u32); 8] = [
    (0x2A78D6, 0x3987E5),
    (0xEB6834, 0xD95926),
    (0x1BAF7A, 0x199E70),
    (0xEDA100, 0xC98500),
    (0xE87BA4, 0xD55181),
    (0x008300, 0x008300),
    (0x6250D6, 0x9085E9),
    (0xE34948, 0xE66767),
];

pub fn is_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

pub fn series(i: usize) -> Rgb {
    let (l, d) = SERIES[i % SERIES.len()];
    hex(if is_dark() { d } else { l })
}

pub fn other_series() -> Rgb {
    hex(if is_dark() { 0x6E6C67 } else { 0x9A9893 })
}

/// Colour of an account in charts: fixed by its position in the account list.
pub fn account_color(name: &str, accounts: &[String]) -> Rgb {
    match accounts.iter().position(|a| a == name) {
        Some(i) => series(i),
        None => other_series(),
    }
}

pub fn usage_color(pct: f64, line: f64) -> Rgb {
    if pct >= line {
        RED
    } else if pct >= line * 0.75 {
        ORANGE
    } else {
        ACCENT
    }
}

pub fn fg(w: &impl IsA<gtk::Widget>) -> Rgb {
    let c: gdk::RGBA = w.as_ref().color();
    (c.red() as f64, c.green() as f64, c.blue() as f64)
}

pub fn set(cr: &cairo::Context, c: Rgb, a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

pub fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Concentric rings: outer = 5-hour window, inner = 7-day window.
pub fn rings(five: Option<f64>, week: Option<f64>, threshold: f64, weekly: f64, size: i32, line: f64) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_valign(gtk::Align::Center);
    area.set_draw_func(move |a, cr, w, h| {
        let c = (w.min(h) as f64) / 2.0;
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let muted = fg(a);
        for (i, (pct, limit)) in [(five, threshold), (week, weekly)].into_iter().enumerate() {
            let r = c - line / 2.0 - i as f64 * (line + 1.5);
            if r <= 0.0 {
                continue;
            }
            let color = match pct {
                Some(p) => usage_color(p, limit),
                None => muted,
            };
            cr.set_line_width(line);
            set(cr, color, if pct.is_some() { 0.18 } else { 0.12 });
            cr.arc(cx, cy, r, 0.0, 2.0 * PI);
            let _ = cr.stroke();
            let p = (pct.unwrap_or(0.0).clamp(0.0, 100.0)) / 100.0;
            if p > 0.0 {
                set(cr, color, 1.0);
                cr.set_line_cap(cairo::LineCap::Round);
                cr.arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + 2.0 * PI * p);
                let _ = cr.stroke();
            }
        }
    });
    area
}

/// A thin horizontal progress bar.
pub fn thin_bar(pct: f64, color: Rgb, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(height);
    area.set_hexpand(true);
    area.set_valign(gtk::Align::Center);
    area.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        set(cr, fg(a), 0.10);
        rounded_rect(cr, 0.0, 0.0, w, h, h / 2.0);
        let _ = cr.fill();
        let fw = (w * pct.clamp(0.0, 100.0) / 100.0).max(h);
        set(cr, color, 1.0);
        rounded_rect(cr, 0.0, 0.0, fw, h, h / 2.0);
        let _ = cr.fill();
    });
    area
}

pub fn dot(color: Rgb, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_valign(gtk::Align::Center);
    area.set_draw_func(move |_, cr, w, h| {
        set(cr, color, 1.0);
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0, 0.0, 2.0 * PI);
        let _ = cr.fill();
    });
    area
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn badge(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("plan-badge");
    l.set_valign(gtk::Align::Center);
    l
}

pub const CSS: &str = r#"
@define-color accent_bg_color #cc6b4a;
@define-color accent_color #b8573a;
@define-color accent_fg_color #ffffff;

.plan-badge {
  font-size: 0.72em;
  font-weight: 600;
  padding: 0 6px;
  border-radius: 999px;
  background-color: alpha(currentColor, 0.10);
  color: alpha(currentColor, 0.75);
}
.folder-default { font-size: 0.75em; font-weight: 600; color: @accent_color; }
.sidebar-section {
  font-size: 0.8em;
  font-weight: 700;
  opacity: 0.55;
  margin: 14px 14px 4px 14px;
}
.deck-card { padding: 18px; border-radius: 16px; }
.kpi-value { font-size: 24pt; font-weight: 600; font-feature-settings: "tnum"; }
.numeric { font-feature-settings: "tnum"; }
.dash-title { font-size: 1.45em; font-weight: 800; }
.over { color: #e5484d; }
.picker-row { padding: 10px 12px; }
.claude-icon { color: @accent_color; }
.event-icon { border-radius: 999px; min-width: 24px; min-height: 24px; background-color: alpha(@accent_bg_color, 0.14); color: @accent_color; }
.event-icon.limit { background-color: alpha(#f0902e, 0.16); color: #d0731a; }
vte-terminal { padding: 8px 4px 4px 14px; }
.nav-badge { font-size: 0.75em; font-weight: 700; padding: 0 7px; border-radius: 999px; background-color: @accent_bg_color; color: @accent_fg_color; }
.osd-tools { background-color: alpha(@window_bg_color, 0.92); border-radius: 8px; box-shadow: 0 1px 4px alpha(black, 0.18); }
.osd-tools button { min-height: 28px; min-width: 30px; }
.osd-title { padding: 5px 10px; border-radius: 8px; background-color: alpha(@window_bg_color, 0.92); box-shadow: 0 1px 4px alpha(black, 0.14); }
.sev { font-size: 0.75em; font-weight: 700; padding: 1px 8px; border-radius: 999px; min-width: 52px; }
.sev-high { background-color: alpha(#e5484d, 0.16); color: #d13a3f; }
.sev-medium { background-color: alpha(#f0902e, 0.18); color: #c26a10; }
.sev-low, .sev-note { background-color: alpha(currentColor, 0.10); }
.code-block { padding: 10px 12px; border-radius: 8px; background-color: alpha(currentColor, 0.06); font-size: 0.92em; }
.doc-list, .doc-list row { background: none; }
.doc-list row.commented { box-shadow: inset 3px 0 @accent_bg_color; }
.task-state { min-width: 26px; padding: 0 4px; font-size: 1.15em; }
.task-doing { color: @accent_color; }
.task-done { color: #30a46c; }
.task-blocked { color: #e5484d; }
.session-chip { padding: 6px 10px; }
"#;
