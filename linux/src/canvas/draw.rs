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

pub fn theme() -> Theme {
    if is_dark() {
        Theme {
            bg: hex(0x1E1E1E),
            fg: hex(0xE8E6E3),
            muted: hex(0x9A968F),
            card: hex(0x2A2A2A),
            card_alt: hex(0x252525),
            border: hex(0x47443F),
            edge: hex(0x77736C),
            accent: hex(0xD97757),
            blue: hex(0x4C9AE8),
            blue_tint: hex(0x1D3247),
            group: hex(0x262626),
            dark: true,
        }
    } else {
        Theme {
            bg: hex(0xFBFAF8),
            fg: hex(0x1F1E1D),
            muted: hex(0x6F6B66),
            card: hex(0xFFFFFF),
            card_alt: hex(0xF6F5F2),
            border: hex(0xD9D6D0),
            edge: hex(0x9C978F),
            accent: hex(0xC4603F),
            blue: hex(0x2272B4),
            blue_tint: hex(0xE8F1FA),
            group: hex(0xF3F1ED),
            dark: false,
        }
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

/// Where the segment from the rect's centre toward `p` leaves the rect.
pub fn boundary(r: &claude_deck::canvas::layout::Rect, p: (f64, f64)) -> (f64, f64) {
    let (cx, cy) = (r.cx(), r.cy());
    let (dx, dy) = (p.0 - cx, p.1 - cy);
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return (cx, cy);
    }
    let sx = if dx.abs() > 1e-9 { (r.w / 2.0) / dx.abs() } else { f64::INFINITY };
    let sy = if dy.abs() > 1e-9 { (r.h / 2.0) / dy.abs() } else { f64::INFINITY };
    let s = sx.min(sy);
    (cx + dx * s, cy + dy * s)
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

pub fn pill(cr: &cairo::Context, w: &impl IsA<gtk::Widget>, s: &str, cx: f64, cy: f64, f: Font, fg: Rgb, bg: Rgb) {
    let (tw, th) = measure(w, s, f);
    let (pw, ph) = (tw + 10.0, th + 2.0);
    set(cr, bg, 0.95);
    rounded(cr, cx - pw / 2.0, cy - ph / 2.0, pw, ph, ph / 2.0);
    let _ = cr.fill();
    text_centered(cr, w, s, cx, cy, f, fg, 1.0);
}
