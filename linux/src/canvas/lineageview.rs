//! Lineage drawn like Databricks Unity Catalog: entity cards with a type icon, namespace and
//! name, expandable column lists, upstream/downstream highlight and column-level lineage.

use super::draw::{self, set, Rgb, BOLD, SMALL};
use super::view::{Model, Reaction};
use crate::widgets::hex;
use claude_deck::canvas::layout::{self, Dir, LNode, Layout, Opts, Rect};
use claude_deck::canvas::lineage::Lineage;
use gtk::{cairo, gdk};
use std::collections::HashSet;
use std::f64::consts::PI;

const W: f64 = 268.0;
const HEAD: f64 = 58.0;
const ROW: f64 = 26.0;

pub fn kind_color(kind: &str) -> Rgb {
    hex(match kind {
        "table" => 0x2272B4,
        "view" => 0x7B61FF,
        "mv" => 0x0E8A6D,
        "streaming" => 0xE8730C,
        "notebook" => 0xE0731A,
        "job" => 0xD6455D,
        "pipeline" => 0x0E8A6D,
        "dashboard" => 0xA855C8,
        "query" => 0x1F8FD1,
        "model" => 0xC69100,
        _ => 0x6B7785,
    })
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "table" => "Table",
        "view" => "View",
        "mv" => "Materialized view",
        "streaming" => "Streaming table",
        "notebook" => "Notebook",
        "job" => "Job",
        "pipeline" => "Pipeline",
        "dashboard" => "Dashboard",
        "query" => "Query",
        "model" => "Model",
        "volume" => "Volume",
        "file" => "File",
        _ => "External",
    }
}

/// A small line icon for an entity type, in a `s`×`s` box at (x, y).
pub fn kind_icon(cr: &cairo::Context, kind: &str, x: f64, y: f64, s: f64, c: Rgb) {
    set(cr, c, 1.0);
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    let (l, t, r, b) = (x + s * 0.18, y + s * 0.2, x + s * 0.82, y + s * 0.8);
    match kind {
        "table" | "mv" | "streaming" => {
            draw::rounded(cr, l, t, r - l, b - t, 2.0);
            cr.move_to(l, t + (b - t) / 3.0);
            cr.line_to(r, t + (b - t) / 3.0);
            cr.move_to(l, t + 2.0 * (b - t) / 3.0);
            cr.line_to(r, t + 2.0 * (b - t) / 3.0);
            cr.move_to(l + (r - l) / 2.5, t + (b - t) / 3.0);
            cr.line_to(l + (r - l) / 2.5, b);
            let _ = cr.stroke();
            if kind == "mv" {
                cr.arc(r, t, s * 0.12, 0.0, 2.0 * PI);
                let _ = cr.fill();
            }
            if kind == "streaming" {
                cr.move_to(r - 2.0, t - 3.0);
                cr.line_to(r + 2.0, t);
                cr.line_to(r - 2.0, t + 3.0);
                let _ = cr.stroke();
            }
        }
        "view" => {
            cr.move_to(l, (t + b) / 2.0);
            cr.curve_to(l + 3.0, t, r - 3.0, t, r, (t + b) / 2.0);
            cr.curve_to(r - 3.0, b, l + 3.0, b, l, (t + b) / 2.0);
            let _ = cr.stroke();
            cr.arc((l + r) / 2.0, (t + b) / 2.0, s * 0.1, 0.0, 2.0 * PI);
            let _ = cr.fill();
        }
        "notebook" => {
            draw::rounded(cr, l + 1.0, t - 1.0, r - l - 2.0, b - t + 2.0, 2.0);
            cr.move_to(l + 4.0, t + 3.0);
            cr.line_to(l + 4.0, b - 1.0);
            cr.move_to(l + 7.0, t + 4.0);
            cr.line_to(r - 3.0, t + 4.0);
            cr.move_to(l + 7.0, t + 7.5);
            cr.line_to(r - 4.0, t + 7.5);
            let _ = cr.stroke();
        }
        "job" | "pipeline" => {
            let p = [(l + 1.0, t + 2.0), (r - 1.0, (t + b) / 2.0), (l + 1.0, b - 2.0)];
            cr.move_to(p[0].0, p[0].1);
            cr.line_to(p[1].0, p[1].1);
            cr.line_to(p[2].0, p[2].1);
            let _ = cr.stroke();
            for q in p {
                cr.arc(q.0, q.1, 2.4, 0.0, 2.0 * PI);
                let _ = cr.fill();
            }
        }
        "dashboard" => {
            for (k, hgt) in [0.45, 0.8, 0.6].iter().enumerate() {
                let bx = l + 1.0 + k as f64 * (r - l) / 3.0;
                cr.rectangle(bx, b - (b - t) * hgt, (r - l) / 3.0 - 2.0, (b - t) * hgt);
            }
            let _ = cr.fill();
        }
        "query" => {
            cr.arc(l + (r - l) * 0.42, t + (b - t) * 0.42, s * 0.2, 0.0, 2.0 * PI);
            cr.move_to(l + (r - l) * 0.62, t + (b - t) * 0.62);
            cr.line_to(r, b);
            let _ = cr.stroke();
        }
        "model" => {
            let (cx, cy, k) = ((l + r) / 2.0, (t + b) / 2.0, (r - l) / 2.0);
            cr.move_to(cx, cy - k);
            cr.line_to(cx + k, cy - k / 2.0);
            cr.line_to(cx + k, cy + k / 2.0);
            cr.line_to(cx, cy + k);
            cr.line_to(cx - k, cy + k / 2.0);
            cr.line_to(cx - k, cy - k / 2.0);
            cr.close_path();
            cr.move_to(cx - k, cy - k / 2.0);
            cr.line_to(cx, cy);
            cr.line_to(cx + k, cy - k / 2.0);
            cr.move_to(cx, cy);
            cr.line_to(cx, cy + k);
            let _ = cr.stroke();
        }
        "volume" | "file" => {
            cr.move_to(l + 1.0, t);
            cr.line_to(r - 4.0, t);
            cr.line_to(r, t + 4.0);
            cr.line_to(r, b);
            cr.line_to(l + 1.0, b);
            cr.close_path();
            let _ = cr.stroke();
        }
        _ => {
            // external / source: a database cylinder
            let e = 2.5;
            cr.save().ok();
            cr.translate((l + r) / 2.0, t + e);
            cr.scale((r - l) / 2.0, e);
            cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI);
            cr.restore().ok();
            cr.move_to(l, t + e);
            cr.line_to(l, b - e);
            cr.curve_to(l, b + e / 2.0, r, b + e / 2.0, r, b - e);
            cr.line_to(r, t + e);
            let _ = cr.stroke();
        }
    }
}

pub struct LineageModel {
    pub lin: Lineage,
    expanded: Vec<bool>,
    lay: Layout,
    hover: Option<usize>,
    hover_col: Option<(usize, usize)>,
    sel_col: Option<(usize, usize)>,
    col_lit: HashSet<(usize, usize)>,
    selected: Option<usize>,
}

enum Hit {
    Header(usize),
    Toggle(usize),
    Column(usize, usize),
}

impl LineageModel {
    pub fn new(lin: Lineage) -> LineageModel {
        let mut expanded = vec![false; lin.nodes.len()];
        if let Some(f) = lin.focus {
            expanded[f] = !lin.nodes[f].columns.is_empty() && lin.nodes[f].columns.len() <= 14;
        }
        let mut m = LineageModel {
            lin,
            expanded,
            lay: Layout::default(),
            hover: None,
            hover_col: None,
            sel_col: None,
            col_lit: HashSet::new(),
            selected: None,
        };
        m.relayout();
        m
    }

    fn height(&self, i: usize) -> f64 {
        if self.expanded[i] {
            HEAD + self.lin.nodes[i].columns.len() as f64 * ROW + 8.0
        } else {
            HEAD
        }
    }

    pub fn relayout(&mut self) {
        let nodes: Vec<LNode> = (0..self.lin.nodes.len())
            .map(|i| LNode { w: W, h: self.height(i), parent: None, group: false, collapsed: false })
            .collect();
        self.lay = layout::layout(&nodes, &self.lin.edges, Dir::LR, Opts { node_gap: 26.0, rank_gap: 96.0, pad: 0.0, title_h: 0.0 });
    }

    pub fn set_all_expanded(&mut self, e: bool) {
        for (i, n) in self.lin.nodes.iter().enumerate() {
            self.expanded[i] = e && !n.columns.is_empty();
        }
        self.relayout();
    }

    pub fn focus_center(&self) -> Option<(f64, f64)> {
        self.lin.focus.and_then(|f| self.lay.rects[f]).map(|r| (r.cx(), r.cy()))
    }

    fn rect(&self, i: usize) -> Rect {
        self.lay.rects[i].unwrap_or_default()
    }

    fn row_y(&self, i: usize, c: usize) -> f64 {
        self.rect(i).y + HEAD + c as f64 * ROW + ROW / 2.0
    }

    fn hit(&self, p: (f64, f64)) -> Option<Hit> {
        for i in 0..self.lin.nodes.len() {
            let r = self.rect(i);
            if !r.contains(p.0, p.1) {
                continue;
            }
            if p.1 < r.y + HEAD {
                let has_cols = !self.lin.nodes[i].columns.is_empty();
                return Some(if has_cols && p.0 > r.right() - 40.0 { Hit::Toggle(i) } else { Hit::Header(i) });
            }
            if self.expanded[i] {
                let c = ((p.1 - r.y - HEAD) / ROW).floor() as usize;
                if c < self.lin.nodes[i].columns.len() {
                    return Some(Hit::Column(i, c));
                }
            }
            return Some(Hit::Header(i));
        }
        None
    }

    /// Nodes lit by the hovered / selected entity (itself plus everything up and down).
    fn lit(&self) -> Option<HashSet<usize>> {
        let at = self.selected.or(self.hover)?;
        let (up, down) = self.lin.related(at);
        let mut s: HashSet<usize> = up.union(&down).copied().collect();
        s.insert(at);
        Some(s)
    }

    fn select_column(&mut self, at: Option<(usize, usize)>) {
        self.sel_col = at;
        self.col_lit = at.map(|a| self.lin.column_related(a)).unwrap_or_default();
        let mut changed = false;
        for &(n, _) in &self.col_lit {
            if !self.expanded[n] {
                self.expanded[n] = true;
                changed = true;
            }
        }
        if changed {
            self.relayout();
        }
    }
}

impl Model for LineageModel {
    fn size(&self) -> (f64, f64) {
        (self.lay.width, self.lay.height)
    }

    fn draw(&self, cr: &cairo::Context, w: &gtk::DrawingArea) {
        let t = draw::theme();
        let lit = self.lit();
        let on = |i: usize| lit.as_ref().is_none_or(|s| s.contains(&i));
        let blue = t.blue;

        // table-level edges
        cr.set_line_cap(cairo::LineCap::Round);
        for &(a, b) in &self.lin.edges {
            let (ra, rb) = (self.rect(a), self.rect(b));
            let p1 = (ra.right(), ra.y + HEAD / 2.0);
            let p2 = (rb.x - 1.0, rb.y + HEAD / 2.0);
            let hot = lit.as_ref().is_some_and(|s| s.contains(&a) && s.contains(&b));
            let alpha = if lit.is_some() && !hot { 0.18 } else { 1.0 };
            set(cr, if hot { blue } else { t.edge }, alpha);
            cr.set_line_width(if hot { 2.0 } else { 1.4 });
            draw::s_curve(cr, p1, p2, 1.0, 1.0);
            let _ = cr.stroke();
            draw::arrow_head(cr, p2, (p2.0 - 10.0, p2.1), 8.0);
        }

        // column-level edges
        if self.sel_col.is_some() {
            for e in &self.lin.col_edges {
                if !(self.col_lit.contains(&e.from) && self.col_lit.contains(&e.to)) {
                    continue;
                }
                if !self.expanded[e.from.0] || !self.expanded[e.to.0] {
                    continue;
                }
                let (ra, rb) = (self.rect(e.from.0), self.rect(e.to.0));
                let p1 = (ra.right(), self.row_y(e.from.0, e.from.1));
                let p2 = (rb.x - 1.0, self.row_y(e.to.0, e.to.1));
                set(cr, blue, 0.95);
                cr.set_line_width(1.8);
                draw::s_curve(cr, p1, p2, 1.0, 1.0);
                let _ = cr.stroke();
                draw::arrow_head(cr, p2, (p2.0 - 10.0, p2.1), 7.0);
            }
        }

        // cards
        for (i, n) in self.lin.nodes.iter().enumerate() {
            let r = self.rect(i);
            let focus = self.lin.focus == Some(i);
            let alpha = if on(i) { 1.0 } else { 0.35 };
            cr.push_group();
            // shadow
            set(cr, (0.0, 0.0, 0.0), if t.dark { 0.35 } else { 0.07 });
            draw::rounded(cr, r.x, r.y + 2.0, r.w, r.h, 10.0);
            let _ = cr.fill();
            set(cr, t.card, 1.0);
            draw::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
            let _ = cr.fill();
            if focus {
                set(cr, t.blue_tint, 1.0);
                draw::rounded_top(cr, r.x, r.y, r.w, HEAD, 10.0);
                if !self.expanded[i] {
                    draw::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
                }
                let _ = cr.fill();
            }
            let hovered = self.hover == Some(i) || self.selected == Some(i);
            set(cr, if focus || hovered { blue } else { t.border }, 1.0);
            cr.set_line_width(if focus { 2.0 } else if hovered { 1.6 } else { 1.0 });
            draw::rounded(cr, r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 10.0);
            let _ = cr.stroke();

            // icon tile
            let kc = kind_color(&n.kind);
            set(cr, kc, if t.dark { 0.22 } else { 0.11 });
            draw::rounded(cr, r.x + 13.0, r.y + 13.0, 32.0, 32.0, 8.0);
            let _ = cr.fill();
            kind_icon(cr, &n.kind, r.x + 13.0, r.y + 13.0, 32.0, kc);

            let (ns, short) = n.split_name();
            let top = if ns.is_empty() { kind_label(&n.kind).to_string() } else { ns };
            let has_cols = !n.columns.is_empty();
            let text_w = r.w - 56.0 - if has_cols { 40.0 } else { 12.0 };
            draw::text(cr, w, &top, r.x + 56.0, r.y + 11.0, SMALL, t.muted, 1.0, 0.0, Some(text_w));
            draw::text(cr, w, &short, r.x + 56.0, r.y + 28.0, BOLD, t.fg, 1.0, 0.0, Some(text_w));
            if has_cols {
                let chev = if self.expanded[i] { "▾" } else { "▸" };
                draw::text(cr, w, &format!("{} {chev}", n.columns.len()), r.right() - 12.0, r.y + 20.0, SMALL, t.muted, 1.0, 1.0, None);
            }

            if self.expanded[i] {
                set(cr, t.border, 1.0);
                cr.set_line_width(1.0);
                cr.move_to(r.x, r.y + HEAD + 0.5);
                cr.line_to(r.right(), r.y + HEAD + 0.5);
                let _ = cr.stroke();
                for (c, col) in n.columns.iter().enumerate() {
                    let y = r.y + HEAD + c as f64 * ROW;
                    let sel = self.sel_col == Some((i, c));
                    let lit_c = self.col_lit.contains(&(i, c));
                    let hov = self.hover_col == Some((i, c));
                    if sel || lit_c || hov {
                        set(cr, if sel || lit_c { t.blue_tint } else { t.card_alt }, 1.0);
                        cr.rectangle(r.x + 1.0, y + 1.0, r.w - 2.0, ROW - 1.0);
                        let _ = cr.fill();
                    }
                    let ink = if sel || lit_c { blue } else { t.fg };
                    draw::text(cr, w, &col.name, r.x + 16.0, y + 5.0, SMALL, ink, 1.0, 0.0, Some(r.w * 0.6));
                    if !col.ty.is_empty() {
                        draw::text(cr, w, &col.ty, r.right() - 14.0, y + 5.0, SMALL, t.muted, 1.0, 1.0, Some(r.w * 0.35));
                    }
                }
            }
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(alpha);
        }
    }

    fn hover(&mut self, p: Option<(f64, f64)>) -> bool {
        let (h, hc) = match p.and_then(|p| self.hit(p)) {
            Some(Hit::Header(i)) | Some(Hit::Toggle(i)) => (Some(i), None),
            Some(Hit::Column(i, c)) => (Some(i), Some((i, c))),
            None => (None, None),
        };
        if h != self.hover || hc != self.hover_col {
            self.hover = h;
            self.hover_col = hc;
            return true;
        }
        false
    }

    fn tooltip(&self, p: (f64, f64)) -> Option<String> {
        match self.hit(p)? {
            Hit::Header(i) => {
                let n = &self.lin.nodes[i];
                let (up, down) = self.lin.related(i);
                Some(format!("{}\n{} · {} upstream · {} downstream\nClick to highlight its lineage", n.name, kind_label(&n.kind), up.len(), down.len()))
            }
            Hit::Toggle(i) => Some(if self.expanded[i] { "Hide columns".into() } else { "Show columns".into() }),
            Hit::Column(i, c) => {
                let col = &self.lin.nodes[i].columns[c];
                let n = self.lin.column_related((i, c)).len() - 1;
                Some(format!("{}{}\n{n} related columns — click to trace", col.name, if col.ty.is_empty() { String::new() } else { format!(": {}", col.ty) }))
            }
        }
    }

    fn click(&mut self, p: (f64, f64), _n: i32, _mods: gdk::ModifierType) -> Reaction {
        match self.hit(p) {
            Some(Hit::Toggle(i)) => {
                self.expanded[i] = !self.expanded[i];
                if !self.expanded[i] && self.sel_col.is_some_and(|(n, _)| n == i) {
                    self.select_column(None);
                }
                self.relayout();
            }
            Some(Hit::Header(i)) => {
                self.selected = if self.selected == Some(i) { None } else { Some(i) };
            }
            Some(Hit::Column(i, c)) => {
                let at = if self.sel_col == Some((i, c)) { None } else { Some((i, c)) };
                self.select_column(at);
            }
            None => {
                self.selected = None;
                self.select_column(None);
            }
        }
        Reaction::Redraw
    }
}
