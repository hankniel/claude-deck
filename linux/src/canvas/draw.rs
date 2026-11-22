//! Drawing helpers shared by the diagram views: theme, text, shapes, colours.

pub use crate::widgets::Rgb;
use crate::widgets::{hex, is_dark};
use gtk::prelude::*;
use gtk::{cairo, pango};
use std::f64::consts::PI;

pub struct Theme {
    pub bg: Rgb,
    pub fg: Rgb,
    pub muted: Rgb,
    pub card: Rgb,
    pub card_alt: Rgb,
    pub border: Rgb,
    pub edge: Rgb,
    pub accent: Rgb,
    pub blue: Rgb,
    pub blue_tint: Rgb,
    pub group: Rgb,
    pub dark: bool,
}

/// Diagram colours, derived from the workbench theme.
pub fn theme() -> Theme {
    let dark = is_dark();
    let p = crate::theme::palette(dark);
    let (bg, fg, strong) = (hex(p.editor), hex(p.fg), hex(p.strong));
    let blue = hex(p.term[if dark { 12 } else { 4 }]);
    Theme {
        bg,
        fg: mix(fg, strong, 0.4),
        muted: hex(p.dim),
        card: if dark { mix(bg, strong, 0.07) } else { mix(bg, (1.0, 1.0, 1.0), 0.85) },
        card_alt: if dark { mix(bg, strong, 0.11) } else { mix(bg, fg, 0.04) },
        border: mix(bg, fg, if dark { 0.20 } else { 0.16 }),
        edge: mix(bg, fg, if dark { 0.45 } else { 0.42 }),
        accent: hex(p.accent),
        blue,
        blue_tint: mix(bg, blue, if dark { 0.16 } else { 0.10 }),
        group: mix(bg, hex(p.chrome), 0.6),
        dark,
    }
}

/// Soft layered shadow under a rounded card.
pub fn shadow(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64, dark: bool) {
    let base = if dark { 0.20 } else { 0.035 };
    for (i, (dy, grow)) in [(1.0, 0.0), (2.0, 1.5), (4.0, 3.5), (7.0, 6.0)].iter().enumerate() {
        set(cr, (0.0, 0.0, 0.0), base / (1.0 + i as f64 * 0.9));
        rounded(cr, x - grow, y + dy - grow * 0.4, w + grow * 2.0, h + grow * 1.2, r + grow);
        let _ = cr.fill();
    }
}

/// A pill with a hairline border (edge labels, counts).
pub fn badge(cr: &cairo::Context, w: &impl IsA<gtk::Widget>, s: &str, cx: f64, cy: f64, f: Font, fg: Rgb, bg: Rgb, border: Rgb) {
    let (tw, th) = measure(w, s, f);
    let (pw, ph) = (tw + 14.0, th + 6.0);
    set(cr, bg, 1.0);
    rounded(cr, cx - pw / 2.0, cy - ph / 2.0, pw, ph, ph / 2.0);
    let _ = cr.fill_preserve();
    set(cr, border, 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    text_centered(cr, w, s, cx, cy, f, fg, 1.0);
}

/// Cubic curve between two ports whose tangents follow the flow axis.
pub fn port_curve(cr: &cairo::Context, a: (f64, f64), b: (f64, f64), horizontal: bool) {
    cr.move_to(a.0, a.1);
    if horizontal {
        let d = ((b.0 - a.0).abs() * 0.5).max(28.0) * if b.0 >= a.0 { 1.0 } else { -1.0 };
        cr.curve_to(a.0 + d, a.1, b.0 - d, b.1, b.0, b.1);
    } else {
        let d = ((b.1 - a.1).abs() * 0.5).max(24.0) * if b.1 >= a.1 { 1.0 } else { -1.0 };
        cr.curve_to(a.0, a.1 + d, b.0, b.1 - d, b.0, b.1);
    }
}

pub fn set(cr: &cairo::Context, c: Rgb, a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

/// "#rgb", "#rrggbb" or a few CSS names.
pub fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
    if let Some(h) = s.strip_prefix('#') {
        let v = match h.len() {
            3 => {
                let c: Vec<u32> = h.chars().filter_map(|c| c.to_digit(16)).collect();
                if c.len() != 3 {
                    return None;
                }
                (c[0] * 17) << 16 | (c[1] * 17) << 8 | c[2] * 17
            }
            6 | 8 => u32::from_str_radix(&h[..6], 16).ok()?,
            _ => return None,
        };
        return Some(hex(v));
    }
    Some(hex(match s.to_ascii_lowercase().as_str() {
        "red" => 0xE5484D,
        "green" => 0x30A46C,
        "blue" => 0x3E63DD,
        "orange" => 0xF76B15,
        "yellow" => 0xFFC53D,
        "purple" => 0x8E4EC6,
        "pink" => 0xD6409F,
        "gray" | "grey" => 0x8B8D98,
        "black" => 0x111111,
        "white" => 0xFFFFFF,
        _ => return None,
    }))
}

pub fn luminance(c: Rgb) -> f64 {
    0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
}

pub fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

pub fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    crate::widgets::rounded_rect(cr, x, y, w, h, r);
}

/// Rounded only on the top corners (table headers).
pub fn rounded_top(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h);
    cr.new_sub_path();
    cr.move_to(x, y + h);
    cr.line_to(x, y + r);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.line_to(x + w, y + h);
    cr.close_path();
}

#[derive(Clone, Copy)]
pub struct Font {
    pub scale: f64,
    pub bold: bool,
    pub mono: bool,
}

pub const BODY: Font = Font { scale: 1.0, bold: false, mono: false };
pub const BOLD: Font = Font { scale: 1.0, bold: true, mono: false };
pub const SMALL: Font = Font { scale: 0.82, bold: false, mono: false };
pub const SMALL_BOLD: Font = Font { scale: 0.82, bold: true, mono: false };
pub const MONO: Font = Font { scale: 0.88, bold: false, mono: true };

pub fn layout(w: &impl IsA<gtk::Widget>, s: &str, f: Font) -> pango::Layout {
    let l = w.as_ref().create_pango_layout(Some(s));
    let mut fd = w.as_ref().pango_context().font_description().unwrap_or_default();
    if let Some(custom) = crate::theme::font("diagram") {
        let c = pango::FontDescription::from_string(&custom);
        if let Some(fam) = c.family() {
            fd.set_family(&fam);
        }
        if c.size() > 0 {
            fd.set_size(c.size());
        }
    }
    if f.mono {
        fd.set_family("Monospace");
    }
    fd.set_size(((fd.size() as f64) * f.scale) as i32);
    if f.bold {
        fd.set_weight(pango::Weight::Semibold);
    }
    l.set_font_description(Some(&fd));
    l
}

pub fn measure(w: &impl IsA<gtk::Widget>, s: &str, f: Font) -> (f64, f64) {
    let (a, b) = layout(w, s, f).pixel_size();
    (a as f64, b as f64)
}

/// Draw text with its top-left (align 0), centre (0.5) or right (1) at x; `max_w` ellipsizes.
pub fn text(cr: &cairo::Context, w: &impl IsA<gtk::Widget>, s: &str, x: f64, y: f64, f: Font, c: Rgb, a: f64, align: f64, max_w: Option<f64>) -> (f64, f64) {
    let l = layout(w, s, f);
    if let Some(m) = max_w {
        l.set_width((m.max(1.0) * pango::SCALE as f64) as i32);
        l.set_ellipsize(pango::EllipsizeMode::End);
    }
    let (tw, th) = l.pixel_size();
    let tw = tw as f64;
    cr.move_to(x - tw * align, y);
    set(cr, c, a);
    pangocairo::functions::show_layout(cr, &l);
    (tw, th as f64)
}

/// Text centred on (cx, cy), multi-line aware.
pub fn text_centered(cr: &cairo::Context, w: &impl IsA<gtk::Widget>, s: &str, cx: f64, cy: f64, f: Font, c: Rgb, a: f64) {
    let l = layout(w, s, f);
    l.set_alignment(pango::Alignment::Center);
    let (tw, th) = l.pixel_size();
    cr.move_to(cx - tw as f64 / 2.0, cy - th as f64 / 2.0);
    set(cr, c, a);
    pangocairo::functions::show_layout(cr, &l);
}

pub fn arrow_head(cr: &cairo::Context, tip: (f64, f64), from: (f64, f64), size: f64) {
    let ang = (tip.1 - from.1).atan2(tip.0 - from.0);
    let (a1, a2) = (ang + PI - 0.45, ang + PI + 0.45);
    cr.move_to(tip.0, tip.1);
    cr.line_to(tip.0 + size * a1.cos(), tip.1 + size * a1.sin());
    cr.line_to(tip.0 + size * a2.cos(), tip.1 + size * a2.sin());
    cr.close_path();
    let _ = cr.fill();
}

/// A smooth curve through points (Catmull-Rom as cubic Béziers).
pub fn smooth_path(cr: &cairo::Context, pts: &[(f64, f64)]) {
    if pts.len() < 2 {
        return;
    }
    cr.move_to(pts[0].0, pts[0].1);
    if pts.len() == 2 {
        cr.line_to(pts[1].0, pts[1].1);
        return;
    }
    for i in 0..pts.len() - 1 {
        let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { p2 };
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        cr.curve_to(c1.0, c1.1, c2.0, c2.1, p2.0, p2.1);
    }
}

/// Horizontal S-curve between two side anchors (lineage / ERD links).
pub fn s_curve(cr: &cairo::Context, a: (f64, f64), b: (f64, f64), out_dir: f64, in_dir: f64) {
    let d = ((b.0 - a.0).abs() * 0.5).max(40.0);
    cr.move_to(a.0, a.1);
    cr.curve_to(a.0 + d * out_dir, a.1, b.0 - d * in_dir, b.1, b.0, b.1);
}

/// Distance from p to a cubic S-curve, sampled.
pub fn s_curve_dist(a: (f64, f64), b: (f64, f64), out_dir: f64, in_dir: f64, p: (f64, f64)) -> f64 {
    let d = ((b.0 - a.0).abs() * 0.5).max(40.0);
    let c1 = (a.0 + d * out_dir, a.1);
    let c2 = (b.0 - d * in_dir, b.1);
    let mut best = f64::INFINITY;
    for i in 0..=40 {
        let t = i as f64 / 40.0;
        let u = 1.0 - t;
        let x = u * u * u * a.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * b.0;
        let y = u * u * u * a.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * b.1;
        best = best.min(((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt());
    }
    best
}
