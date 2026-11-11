//! Database diagram in the style of dbdiagram.io: table cards with coloured headers, column
//! rows with types and keys, crow's-foot relationship lines, table groups and enums.
//! Tables can be dragged; positions are kept beside the screen file.

use super::draw::{self, set, Rgb, BOLD, MONO, SMALL, SMALL_BOLD};
use super::view::{Model, Reaction};
use crate::widgets::hex;
use claude_deck::canvas::dbml::{Card, Erd};
use claude_deck::canvas::layout::{self, Dir, LNode, Opts, Rect};
use gtk::{cairo, gdk};
use serde_json::{json, Value};
use std::path::PathBuf;

const HEAD: f64 = 36.0;
const ROW: f64 = 28.0;
const GROUP_PAD: f64 = 22.0;
const GROUP_TITLE: f64 = 30.0;

#[derive(Clone, Copy, PartialEq)]
enum Item {
    Table(usize),
    Enum(usize),
}

pub struct ErdModel {
    pub erd: Erd,
    rects: Vec<Rect>,
    enum_rects: Vec<Rect>,
    hover: Option<Item>,
    hover_row: Option<(usize, usize)>,
    hover_ref: Option<usize>,
    dragging: Vec<Item>,
    save_to: Option<PathBuf>,
    meas: gtk::Label,
    org: (f64, f64),
}

fn default_header(dark: bool) -> Rgb {
    hex(if dark { 0x3B5A86 } else { 0x316896 })
}

impl ErdModel {
    pub fn new(erd: Erd, save_to: Option<PathBuf>) -> ErdModel {
        let meas = gtk::Label::new(None);
        let mut m = ErdModel {
            erd,
            rects: vec![],
            enum_rects: vec![],
            hover: None,
            hover_row: None,
            hover_ref: None,
            dragging: vec![],
            save_to,
            meas,
            org: (0.0, 0.0),
        };
        m.size_cards();
        if !m.load_positions() {
            m.auto_layout();
        }
        m.org = m.compute_origin();
        m
    }

    fn card_width(&self, title: &str, rows: &[(String, String)]) -> f64 {
        let mut w = draw::measure(&self.meas, title, BOLD).0 + 36.0;
        for (a, b) in rows {
            let wa = draw::measure(&self.meas, a, SMALL_BOLD).0;
            let wb = draw::measure(&self.meas, b, MONO).0;
            w = w.max(wa + wb + 64.0);
        }
        w.clamp(190.0, 420.0)
    }

    fn size_cards(&mut self) {
        let mut rects = vec![];
        for t in &self.erd.tables {
            let rows: Vec<(String, String)> = t.columns.iter().map(|c| (c.name.clone(), c.ty.clone())).collect();
            let w = self.card_width(&t.name, &rows);
            rects.push(Rect { x: 0.0, y: 0.0, w, h: HEAD + t.columns.len().max(1) as f64 * ROW + 4.0 });
        }
        let mut enum_rects = vec![];
        for e in &self.erd.enums {
            let rows: Vec<(String, String)> = e.values.iter().map(|v| (v.clone(), String::new())).collect();
            let w = self.card_width(&format!("enum {}", e.name), &rows);
            enum_rects.push(Rect { x: 0.0, y: 0.0, w, h: HEAD + e.values.len().max(1) as f64 * ROW + 4.0 });
        }
        self.rects = rects;
        self.enum_rects = enum_rects;
    }

    fn auto_layout(&mut self) {
        let nt = self.rects.len();
        let ne = self.enum_rects.len();
        let mut nodes: Vec<LNode> = vec![];
        for (i, r) in self.rects.iter().enumerate() {
            nodes.push(LNode { w: r.w, h: r.h, parent: self.erd.tables[i].group.map(|g| nt + ne + g), group: false, collapsed: false });
        }
        for r in &self.enum_rects {
            nodes.push(LNode { w: r.w, h: r.h, parent: None, group: false, collapsed: false });
        }
        for g in &self.erd.groups {
            let tw = draw::measure(&self.meas, &g.name, SMALL_BOLD).0 + 30.0;
            nodes.push(LNode { w: tw, h: 20.0, parent: None, group: true, collapsed: false });
        }
        // the "one" side on the left, the "many" side on the right
        let mut edges = vec![];
        for r in &self.erd.refs {
            let (a, b) = if r.from_card == Card::Many && r.to_card == Card::One { (r.to.0, r.from.0) } else { (r.from.0, r.to.0) };
            if a != b {
                edges.push((a, b));
            }
        }
        // enums sit next to the first table using them
        for (e, en) in self.erd.enums.iter().enumerate() {
            let user = self
                .erd
                .tables
                .iter()
                .position(|t| t.columns.iter().any(|c| c.ty == en.name || c.ty.rsplit('.').next() == Some(en.name.as_str())));
            if let Some(u) = user {
                edges.push((nt + e, u));
            }
        }
        let l = layout::layout(&nodes, &edges, Dir::LR, Opts { node_gap: 40.0, rank_gap: 110.0, pad: GROUP_PAD, title_h: GROUP_TITLE });
        for i in 0..nt {
            if let Some(r) = l.rects[i] {
                self.rects[i].x = r.x;
                self.rects[i].y = r.y;
            }
        }
        for e in 0..ne {
            if let Some(r) = l.rects[nt + e] {
                self.enum_rects[e].x = r.x;
                self.enum_rects[e].y = r.y;
            }
        }
    }

    fn load_positions(&mut self) -> bool {
        let Some(p) = &self.save_to else { return false };
        let Ok(s) = std::fs::read_to_string(p) else { return false };
        let Ok(v) = serde_json::from_str::<Value>(&s) else { return false };
        let get = |kind: &str, name: &str| -> Option<(f64, f64)> {
            let a = v.get(kind)?.get(name)?.as_array()?;
            Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
        };
        let mut all = true;
        for (i, t) in self.erd.tables.iter().enumerate() {
            match get("tables", &t.name) {
                Some((x, y)) => {
                    self.rects[i].x = x;
                    self.rects[i].y = y;
                }
                None => all = false,
            }
        }
        for (i, e) in self.erd.enums.iter().enumerate() {
            match get("enums", &e.name) {
                Some((x, y)) => {
                    self.enum_rects[i].x = x;
                    self.enum_rects[i].y = y;
                }
                None => all = false,
            }
        }
        all
    }

    fn save_positions(&self) {
        let Some(p) = &self.save_to else { return };
        let tables: serde_json::Map<String, Value> =
            self.erd.tables.iter().zip(&self.rects).map(|(t, r)| (t.name.clone(), json!([r.x.round(), r.y.round()]))).collect();
        let enums: serde_json::Map<String, Value> =
            self.erd.enums.iter().zip(&self.enum_rects).map(|(e, r)| (e.name.clone(), json!([r.x.round(), r.y.round()]))).collect();
        let _ = std::fs::write(p, json!({ "tables": tables, "enums": enums }).to_string());
    }

    /// Forget dragged positions and lay out again.
    pub fn reset_layout(&mut self) {
        if let Some(p) = &self.save_to {
            let _ = std::fs::remove_file(p);
        }
        self.auto_layout();
        self.org = self.compute_origin();
    }

    fn group_rect(&self, g: usize) -> Option<Rect> {
        let tabs = &self.erd.groups[g].tables;
        if tabs.is_empty() {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &t in tabs {
            let r = self.rects[t];
            x0 = x0.min(r.x);
            y0 = y0.min(r.y);
            x1 = x1.max(r.right());
            y1 = y1.max(r.bottom());
        }
        Some(Rect {
            x: x0 - GROUP_PAD,
            y: y0 - GROUP_PAD - GROUP_TITLE,
            w: x1 - x0 + 2.0 * GROUP_PAD,
            h: y1 - y0 + 2.0 * GROUP_PAD + GROUP_TITLE,
        })
    }

    /// Top-left of everything at load time; fixed so dragging never shifts the view.
    fn origin(&self) -> (f64, f64) {
        self.org
    }

    fn compute_origin(&self) -> (f64, f64) {
        let mut all: Vec<Rect> = self.rects.iter().chain(&self.enum_rects).copied().collect();
        all.extend((0..self.erd.groups.len()).filter_map(|g| self.group_rect(g)));
        let x0 = all.iter().map(|r| r.x).fold(f64::MAX, f64::min);
        let y0 = all.iter().map(|r| r.y).fold(f64::MAX, f64::min);
        if all.is_empty() { (0.0, 0.0) } else { (x0, y0) }
    }

    fn extent(&self) -> (f64, f64) {
        let mut all: Vec<Rect> = self.rects.iter().chain(&self.enum_rects).copied().collect();
        all.extend((0..self.erd.groups.len()).filter_map(|g| self.group_rect(g)));
        let x1 = all.iter().map(|r| r.right()).fold(100.0, f64::max);
        let y1 = all.iter().map(|r| r.bottom()).fold(100.0, f64::max);
        (x1, y1)
    }

    fn local(&self, p: (f64, f64)) -> (f64, f64) {
        let (x0, y0) = self.origin();
        (p.0 + x0, p.1 + y0)
    }

    fn row_y(&self, t: usize, c: usize) -> f64 {
        self.rects[t].y + HEAD + c as f64 * ROW + ROW / 2.0
    }

    /// Anchors and side directions of a ref: (a, b, out_dir, in_dir).
    fn anchors(&self, k: usize) -> ((f64, f64), (f64, f64), f64, f64) {
        let r = &self.erd.refs[k];
        let (ra, rb) = (self.rects[r.from.0], self.rects[r.to.0]);
        let (ya, yb) = (self.row_y(r.from.0, r.from.1), self.row_y(r.to.0, r.to.1));
        if r.from.0 == r.to.0 {
            return ((ra.right(), ya), (rb.right(), yb), 1.0, -1.0);
        }
        if rb.x >= ra.right() - 20.0 {
            ((ra.right(), ya), (rb.x, yb), 1.0, 1.0)
        } else if ra.x >= rb.right() - 20.0 {
            ((ra.x, ya), (rb.right(), yb), -1.0, -1.0)
        } else {
            ((ra.right(), ya), (rb.right(), yb), 1.0, -1.0)
        }
    }

    fn hit(&self, p: (f64, f64)) -> Option<(Item, Option<usize>)> {
        for (i, r) in self.rects.iter().enumerate().rev() {
            if r.contains(p.0, p.1) {
                let row = if p.1 >= r.y + HEAD {
                    Some(((p.1 - r.y - HEAD) / ROW) as usize).filter(|&c| c < self.erd.tables[i].columns.len())
                } else {
                    None
                };
                return Some((Item::Table(i), row));
            }
        }
        for (i, r) in self.enum_rects.iter().enumerate().rev() {
            if r.contains(p.0, p.1) {
                return Some((Item::Enum(i), None));
            }
        }
        None
    }

    fn group_title_hit(&self, p: (f64, f64)) -> Option<usize> {
        (0..self.erd.groups.len())
            .find(|&g| self.group_rect(g).is_some_and(|r| Rect { x: r.x, y: r.y, w: r.w, h: GROUP_TITLE + 6.0 }.contains(p.0, p.1)))
    }

    fn ref_hit(&self, p: (f64, f64)) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for k in 0..self.erd.refs.len() {
            let (a, b, o, i) = self.anchors(k);
            let d = draw::s_curve_dist(a, b, o, i, p);
            if d < 6.0 && best.is_none_or(|x| d < x.1) {
                best = Some((k, d));
            }
        }
        best.map(|b| b.0)
    }

    fn marker(cr: &cairo::Context, at: (f64, f64), dir: f64, card: Card) {
        let (x, y) = at;
        match card {
            Card::One => {
                cr.move_to(x + dir * 9.0, y - 6.0);
                cr.line_to(x + dir * 9.0, y + 6.0);
            }
            Card::Many => {
                for dy in [-7.0, 0.0, 7.0] {
                    cr.move_to(x + dir * 13.0, y);
                    cr.line_to(x, y + dy);
                }
            }
        }
        let _ = cr.stroke();
    }

    fn header_color(&self, t: usize, dark: bool) -> Rgb {
        let tb = &self.erd.tables[t];
        tb.color
            .as_deref()
            .and_then(draw::parse_color)
            .or_else(|| tb.group.and_then(|g| self.erd.groups[g].color.as_deref()).and_then(draw::parse_color))
            .unwrap_or_else(|| default_header(dark))
    }

    fn involved(&self, k: usize) -> bool {
        let r = &self.erd.refs[k];
        if self.hover_ref == Some(k) {
            return true;
        }
        if let Some((t, c)) = self.hover_row {
            return r.from == (t, c) || r.to == (t, c);
        }
        match self.hover {
            Some(Item::Table(t)) => r.from.0 == t || r.to.0 == t,
            _ => false,
        }
    }
}

impl Model for ErdModel {
    fn size(&self) -> (f64, f64) {
        let (x0, y0) = self.origin();
        let (x1, y1) = self.extent();
        (x1 - x0, y1 - y0)
    }

    fn draw(&self, cr: &cairo::Context, w: &gtk::DrawingArea) {
        let t = draw::theme();
        let (x0, y0) = self.origin();
        cr.translate(-x0, -y0);
        let focus = self.hover.is_some() || self.hover_ref.is_some();

        for (g, grp) in self.erd.groups.iter().enumerate() {
            let Some(r) = self.group_rect(g) else { continue };
            let c = grp.color.as_deref().and_then(draw::parse_color).unwrap_or(t.muted);
            set(cr, c, if t.dark { 0.10 } else { 0.07 });
            draw::rounded(cr, r.x, r.y, r.w, r.h, 14.0);
            let _ = cr.fill_preserve();
            set(cr, c, 0.45);
            cr.set_line_width(1.0);
            cr.set_dash(&[6.0, 4.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            draw::text(cr, w, &grp.name, r.x + 14.0, r.y + 8.0, SMALL_BOLD, c, 1.0, 0.0, Some(r.w - 28.0));
        }

        cr.set_line_cap(cairo::LineCap::Round);
        for k in 0..self.erd.refs.len() {
            let (a, b, o, i) = self.anchors(k);
            let hot = self.involved(k);
            let alpha = if focus && !hot { 0.22 } else { 1.0 };
            set(cr, if hot { t.accent } else { t.edge }, alpha);
            cr.set_line_width(if hot { 2.0 } else { 1.4 });
            draw::s_curve(cr, a, b, o, i);
            let _ = cr.stroke();
            let r = &self.erd.refs[k];
            Self::marker(cr, a, o, r.from_card);
            Self::marker(cr, b, -i, r.to_card);
        }

        for (e, en) in self.erd.enums.iter().enumerate() {
            let r = self.enum_rects[e];
            let alpha = if focus && self.hover != Some(Item::Enum(e)) { 0.5 } else { 1.0 };
            cr.push_group();
            set(cr, (0.0, 0.0, 0.0), if t.dark { 0.3 } else { 0.08 });
            draw::rounded(cr, r.x, r.y + 2.0, r.w, r.h, 8.0);
            let _ = cr.fill();
            set(cr, t.card, 1.0);
            draw::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
            let _ = cr.fill();
            set(cr, hex(0x7A6F9B), 1.0);
            draw::rounded_top(cr, r.x, r.y, r.w, HEAD, 8.0);
            let _ = cr.fill();
            draw::text(cr, w, &format!("enum  {}", en.name), r.x + 12.0, r.y + 9.0, BOLD, (1.0, 1.0, 1.0), 1.0, 0.0, Some(r.w - 24.0));
            for (k, v) in en.values.iter().enumerate() {
                draw::text(cr, w, v, r.x + 14.0, r.y + HEAD + k as f64 * ROW + 6.0, SMALL, t.fg, 1.0, 0.0, Some(r.w - 28.0));
            }
            set(cr, t.border, 1.0);
            draw::rounded(cr, r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 8.0);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(alpha);
        }

        let lit_tables: Vec<usize> = (0..self.erd.refs.len())
            .filter(|&k| self.involved(k))
            .flat_map(|k| [self.erd.refs[k].from.0, self.erd.refs[k].to.0])
            .collect();
        for (i, tb) in self.erd.tables.iter().enumerate() {
            let r = self.rects[i];
            let on = !focus || self.hover == Some(Item::Table(i)) || lit_tables.contains(&i);
            cr.push_group();
            set(cr, (0.0, 0.0, 0.0), if t.dark { 0.35 } else { 0.09 });
            draw::rounded(cr, r.x, r.y + 3.0, r.w, r.h, 8.0);
            let _ = cr.fill();
            set(cr, t.card, 1.0);
            draw::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
            let _ = cr.fill();
            let hc = self.header_color(i, t.dark);
            set(cr, hc, 1.0);
            draw::rounded_top(cr, r.x, r.y, r.w, HEAD, 8.0);
            let _ = cr.fill();
            let ink = if draw::luminance(hc) > 0.62 { hex(0x1F1E1D) } else { (1.0, 1.0, 1.0) };
            let (schema, name) = tb.split_name();
            let title = if schema.is_empty() || schema == "public" { name } else { format!("{schema}.{name}") };
            draw::text(cr, w, &title, r.x + 12.0, r.y + 9.0, BOLD, ink, 1.0, 0.0, Some(r.w - 24.0));
            for (c, col) in tb.columns.iter().enumerate() {
                let y = r.y + HEAD + c as f64 * ROW;
                let hot_row = self.hover_row == Some((i, c))
                    || (0..self.erd.refs.len())
                        .any(|k| self.involved(k) && (self.erd.refs[k].from == (i, c) || self.erd.refs[k].to == (i, c)));
                if hot_row {
                    set(cr, t.accent, if t.dark { 0.18 } else { 0.10 });
                    cr.rectangle(r.x + 1.0, y, r.w - 2.0, ROW);
                    let _ = cr.fill();
                }
                if c > 0 {
                    set(cr, t.border, 0.6);
                    cr.set_line_width(1.0);
                    cr.move_to(r.x + 8.0, y + 0.5);
                    cr.line_to(r.right() - 8.0, y + 0.5);
                    let _ = cr.stroke();
                }
                let mut x = r.x + 12.0;
                if col.pk {
                    set(cr, hex(0xD9A40B), 1.0);
                    cr.set_line_width(1.6);
                    cr.new_sub_path();
                    cr.arc(x + 4.0, y + ROW / 2.0, 3.5, 0.0, std::f64::consts::TAU);
                    let _ = cr.stroke();
                    cr.move_to(x + 7.5, y + ROW / 2.0);
                    cr.line_to(x + 13.0, y + ROW / 2.0);
                    cr.move_to(x + 11.0, y + ROW / 2.0);
                    cr.line_to(x + 11.0, y + ROW / 2.0 + 3.0);
                    let _ = cr.stroke();
                    x += 18.0;
                }
                let fk = self.erd.refs.iter().any(|rf| rf.from == (i, c) || rf.to == (i, c));
                let font = if col.pk { SMALL_BOLD } else { SMALL };
                let name_ink = if fk && !col.pk { t.blue } else { t.fg };
                let (nw, _) = draw::text(cr, w, &col.name, x, y + 6.0, font, name_ink, 1.0, 0.0, Some(r.w * 0.55));
                if col.not_null {
                    draw::text(cr, w, "*", x + nw + 2.0, y + 4.0, SMALL, t.accent, 0.9, 0.0, None);
                }
                let ty_ink = if self.erd.is_enum(&col.ty) { hex(0x8A70C8) } else { t.muted };
                draw::text(cr, w, &col.ty, r.right() - 12.0, y + 7.0, MONO, ty_ink, 1.0, 1.0, Some(r.w * 0.42));
            }
            let hovered = self.hover == Some(Item::Table(i));
            set(cr, if hovered { t.accent } else { t.border }, 1.0);
            cr.set_line_width(if hovered { 1.6 } else { 1.0 });
            draw::rounded(cr, r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 8.0);
            let _ = cr.stroke();
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(if on { 1.0 } else { 0.4 });
        }
    }

    fn hover(&mut self, p: Option<(f64, f64)>) -> bool {
        let p = p.map(|p| self.local(p));
        let (h, row) = match p.and_then(|p| self.hit(p)) {
            Some((it, row)) => (Some(it), row.and_then(|c| if let Item::Table(t) = it { Some((t, c)) } else { None })),
            None => (None, None),
        };
        let rf = if h.is_none() { p.and_then(|p| self.ref_hit(p)) } else { None };
        if h != self.hover || row != self.hover_row || rf != self.hover_ref {
            self.hover = h;
            self.hover_row = row;
            self.hover_ref = rf;
            return true;
        }
        false
    }

    fn tooltip(&self, p: (f64, f64)) -> Option<String> {
        let p = self.local(p);
        if let Some((item, row)) = self.hit(p) {
            return match (item, row) {
                (Item::Table(t), Some(c)) => {
                    let col = &self.erd.tables[t].columns[c];
                    let mut flags = vec![];
                    if col.pk {
                        flags.push("primary key".to_string());
                    }
                    if col.not_null {
                        flags.push("not null".into());
                    }
                    if col.unique {
                        flags.push("unique".into());
                    }
                    if col.increment {
                        flags.push("increment".into());
                    }
                    if let Some(d) = &col.default {
                        flags.push(format!("default {d}"));
                    }
                    for r in &self.erd.refs {
                        let other = if r.from == (t, c) {
                            Some(r.to)
                        } else if r.to == (t, c) {
                            Some(r.from)
                        } else {
                            None
                        };
                        if let Some((ot, oc)) = other {
                            flags.push(format!("→ {}.{}", self.erd.tables[ot].name, self.erd.tables[ot].columns[oc].name));
                        }
                    }
                    if let Some(e) = self.erd.enums.iter().find(|e| e.name == col.ty) {
                        flags.push(format!("enum: {}", e.values.join(", ")));
                    }
                    let mut s = format!("{} {}", col.name, col.ty);
                    if !flags.is_empty() {
                        s.push_str(&format!("\n{}", flags.join(" · ")));
                    }
                    if let Some(n) = &col.note {
                        s.push_str(&format!("\n{n}"));
                    }
                    Some(s)
                }
                (Item::Table(t), None) => {
                    let tb = &self.erd.tables[t];
                    Some(format!(
                        "{}{}{}\nDrag to move",
                        tb.name,
                        tb.alias.as_ref().map(|a| format!(" as {a}")).unwrap_or_default(),
                        tb.note.as_ref().map(|n| format!("\n{n}")).unwrap_or_default()
                    ))
                }
                (Item::Enum(e), _) => Some(format!("enum {}", self.erd.enums[e].name)),
            };
        }
        let k = self.ref_hit(p)?;
        let r = &self.erd.refs[k];
        let card = |c: Card| if c == Card::One { "1" } else { "*" };
        Some(format!(
            "{}.{} ({}) → {}.{} ({}){}",
            self.erd.tables[r.from.0].name,
            self.erd.tables[r.from.0].columns[r.from.1].name,
            card(r.from_card),
            self.erd.tables[r.to.0].name,
            self.erd.tables[r.to.0].columns[r.to.1].name,
            card(r.to_card),
            r.name.as_ref().map(|n| format!("\n{n}")).unwrap_or_default()
        ))
    }

    fn grab(&mut self, p: (f64, f64)) -> bool {
        let p = self.local(p);
        if let Some((item, _)) = self.hit(p) {
            self.dragging = vec![item];
            return true;
        }
        if let Some(g) = self.group_title_hit(p) {
            self.dragging = self.erd.groups[g].tables.iter().map(|&t| Item::Table(t)).collect();
            return true;
        }
        false
    }

    fn drag(&mut self, dx: f64, dy: f64) {
        for it in self.dragging.clone() {
            let r = match it {
                Item::Table(t) => &mut self.rects[t],
                Item::Enum(e) => &mut self.enum_rects[e],
            };
            r.x += dx;
            r.y += dy;
        }
    }

    fn release(&mut self) {
        if !self.dragging.is_empty() {
            self.dragging.clear();
            self.save_positions();
        }
    }

    fn click(&mut self, _p: (f64, f64), _n: i32, _mods: gdk::ModifierType) -> Reaction {
        Reaction::None
    }
}
