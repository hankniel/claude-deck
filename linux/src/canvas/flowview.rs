//! Interactive flowchart: collapsible groups, lineage highlight on click, search, links.

use super::draw::{self, set, Font, SMALL, SMALL_BOLD};
use super::view::{ease, Model, Reaction};
use claude_deck::canvas::flow::{EdgeKind, Flow, Shape};
use claude_deck::canvas::layout::{self, LNode, Layout, Opts, Rect};
use gtk::{cairo, gdk};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use std::f64::consts::PI;

const TITLE_H: f64 = 28.0;

pub struct FlowModel {
    pub flow: Flow,
    collapsed: Vec<bool>,
    sizes: Vec<(f64, f64)>,
    lay: Layout,
    hover: Option<usize>,
    selected: Option<usize>,
    lit_nodes: HashSet<usize>,
    lit_edges: HashSet<usize>,
    matches: Vec<usize>,
    meas: gtk::Label,
    /// Layout transition: where each node was drawn when the layout changed.
    prev: Vec<Option<Rect>>,
    moved_at: Option<Instant>,
    /// Fade of everything outside the traced lineage: level from → to since.
    dim_from: f64,
    dim_to: f64,
    dim_at: Option<Instant>,
}

const MOVE_MS: f64 = 300.0;
const FADE_MS: f64 = 200.0;
const DIM: f64 = 0.18;

const TILE: f64 = 30.0;
const PAD: f64 = 12.0;
const TEXT_MAX: f64 = 300.0;
const TITLE_FONT: Font = Font { scale: 0.95, bold: true, mono: false };

/// Card title and subtitle: the first label line, then the rest.
fn split_label(label: &str) -> (String, String) {
    let mut lines = label.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or("").to_string();
    let rest: Vec<&str> = lines.collect();
    (title, rest.join(" · "))
}

/// Tint of a node's icon tile: its own fill, else one per Mermaid shape.
fn kind_color(shape: Shape) -> draw::Rgb {
    crate::widgets::hex(match shape {
        Shape::Rect => 0x6E8FB5,
        Shape::Round | Shape::Stadium => 0xD97757,
        Shape::Subroutine => 0x8E6CC9,
        Shape::Cylinder => 0x3E8ED0,
        Shape::Circle => 0x30A46C,
        Shape::Diamond => 0xD9A23A,
        Shape::Hexagon => 0x2A9D8F,
        Shape::Parallelogram => 0xC75C9A,
        Shape::Flag => 0xE5484D,
    })
}

/// The node's Mermaid shape, drawn small inside its icon tile.
fn mini_shape(cr: &cairo::Context, shape: Shape, cx: f64, cy: f64, s: f64) {
    let (x, y, w, h) = (cx - s / 2.0, cy - s * 0.38, s, s * 0.76);
    match shape {
        Shape::Rect | Shape::Subroutine => {
            draw::rounded(cr, x, y, w, h, 1.5);
            if shape == Shape::Subroutine {
                cr.move_to(x + w * 0.2, y);
                cr.line_to(x + w * 0.2, y + h);
                cr.move_to(x + w * 0.8, y);
                cr.line_to(x + w * 0.8, y + h);
            }
        }
        Shape::Round => draw::rounded(cr, x, y, w, h, 4.0),
        Shape::Stadium => draw::rounded(cr, x, y, w, h, h / 2.0),
        Shape::Circle => {
            cr.new_sub_path();
            cr.arc(cx, cy, s * 0.42, 0.0, 2.0 * PI);
        }
        Shape::Diamond => {
            cr.move_to(cx, cy - s * 0.5);
            cr.line_to(cx + s * 0.5, cy);
            cr.line_to(cx, cy + s * 0.5);
            cr.line_to(cx - s * 0.5, cy);
            cr.close_path();
        }
        Shape::Hexagon => {
            let k = w * 0.25;
            cr.move_to(x + k, y);
            cr.line_to(x + w - k, y);
            cr.line_to(x + w, cy);
            cr.line_to(x + w - k, y + h);
            cr.line_to(x + k, y + h);
            cr.line_to(x, cy);
            cr.close_path();
        }
        Shape::Parallelogram => {
            let k = w * 0.25;
            cr.move_to(x + k, y);
            cr.line_to(x + w, y);
            cr.line_to(x + w - k, y + h);
            cr.line_to(x, y + h);
            cr.close_path();
        }
        Shape::Flag => {
            cr.move_to(x, y);
            cr.line_to(x + w, y);
            cr.line_to(x + w, y + h);
            cr.line_to(x, y + h);
            cr.line_to(x + w * 0.25, cy);
            cr.close_path();
        }
        Shape::Cylinder => {
            let (top, bot, e) = (cy - s * 0.42, cy + s * 0.42, s * 0.16);
            cr.new_sub_path();
            cr.save().ok();
            cr.translate(cx, top + e);
            cr.scale(s / 2.0, e);
            cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI);
            cr.restore().ok();
            cr.move_to(x, top + e);
            cr.line_to(x, bot - e);
            cr.save().ok();
            cr.translate(cx, bot - e);
            cr.scale(s / 2.0, e);
            cr.arc_negative(-1.0, 0.0, 1.0, PI, 0.0);
            cr.restore().ok();
            cr.line_to(x + w, top + e);
        }
    }
}

impl FlowModel {
    pub fn new(flow: Flow) -> FlowModel {
        let collapsed = (0..flow.nodes.len()).map(|i| flow.starts_collapsed(i)).collect();
        let meas = gtk::Label::new(None);
        let mut m = FlowModel {
            flow,
            collapsed,
            sizes: vec![],
            lay: Layout::default(),
            hover: None,
            selected: None,
            lit_nodes: HashSet::new(),
            lit_edges: HashSet::new(),
            matches: vec![],
            meas,
            prev: vec![],
            moved_at: None,
            dim_from: 1.0,
            dim_to: 1.0,
            dim_at: None,
        };
        m.measure();
        m.relayout();
        m
    }

    fn measure(&mut self) {
        self.sizes = self
            .flow
            .nodes
            .iter()
            .map(|n| {
                if n.group {
                    let (tw, _) = draw::measure(&self.meas, &n.label, SMALL_BOLD);
                    return (tw + 90.0, TITLE_H + 14.0);
                }
                let (title, sub) = split_label(&n.label);
                let (tw, _) = draw::measure(&self.meas, &title, TITLE_FONT);
                let sw = if sub.is_empty() { 0.0 } else { draw::measure(&self.meas, &sub, SMALL).0 };
                let text = tw.max(sw).min(TEXT_MAX);
                let w = (PAD + TILE + 10.0 + text + PAD + 4.0).max(150.0);
                (w, if sub.is_empty() { 46.0 } else { 56.0 })
            })
            .collect();
    }

    fn group_note(n: usize) -> String {
        format!("{n} items · click to expand")
    }

    /// Where node i is drawn now: between its old and new place while the layout moves.
    fn shown_rect(&self, i: usize) -> Option<Rect> {
        let new = self.lay.rects.get(i).copied().flatten()?;
        let (Some(at), Some(Some(old))) = (self.moved_at, self.prev.get(i)) else { return Some(new) };
        let k = ease(at, MOVE_MS);
        if k >= 1.0 {
            return Some(new);
        }
        let l = |a: f64, b: f64| a + (b - a) * k;
        Some(Rect { x: l(old.x, new.x), y: l(old.y, new.y), w: l(old.w, new.w), h: l(old.h, new.h) })
    }

    fn moving(&self) -> bool {
        self.moved_at.is_some_and(|at| ease(at, MOVE_MS) < 1.0)
    }

    /// Opacity of what lies outside the traced lineage.
    fn dim_level(&self) -> f64 {
        match self.dim_at {
            Some(at) => self.dim_from + (self.dim_to - self.dim_from) * ease(at, FADE_MS),
            None => self.dim_to,
        }
    }

    fn select(&mut self, sel: Option<usize>) {
        let was = self.selected.is_some();
        self.selected = sel;
        if sel.is_some() {
            self.recompute_lit();
        }
        if was != sel.is_some() {
            self.dim_from = self.dim_level();
            self.dim_to = if sel.is_some() { DIM } else { 1.0 };
            self.dim_at = Some(Instant::now());
        }
    }

    fn hidden_count(&self, g: usize) -> usize {
        self.flow.nodes.iter().enumerate().filter(|(i, n)| !n.group && self.is_under(*i, g)).count()
    }

    fn is_under(&self, mut i: usize, g: usize) -> bool {
        while let Some(p) = self.flow.nodes[i].parent {
            if p == g {
                return true;
            }
            i = p;
        }
        false
    }

    pub fn relayout(&mut self) {
        let nodes: Vec<LNode> = self
            .flow
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let (mut w, mut h) = self.sizes[i];
                if n.group && self.collapsed[i] {
                    let (tw, _) = draw::measure(&self.meas, &n.label, TITLE_FONT);
                    let (sw, _) = draw::measure(&self.meas, &Self::group_note(self.hidden_count(i)), SMALL);
                    w = (PAD + TILE + 10.0 + tw.max(sw).min(TEXT_MAX) + PAD + 46.0).max(190.0);
                    h = 54.0;
                }
                LNode { w, h, parent: n.parent, group: n.group, collapsed: n.group && self.collapsed[i] }
            })
            .collect();
        let edges: Vec<(usize, usize)> = self.flow.edges.iter().map(|e| (e.from, e.to)).collect();
        // leave room between ranks for the widest edge label (wide in LR, tall in TB)
        let label_w = self
            .flow
            .edges
            .iter()
            .filter(|e| !e.label.is_empty())
            .map(|e| draw::measure(&self.meas, &e.label, SMALL).0.min(220.0))
            .fold(0.0, f64::max);
        let opts = if self.flow.dir.horizontal() {
            Opts { node_gap: 26.0, rank_gap: (label_w + 56.0).max(72.0), pad: 18.0, title_h: TITLE_H }
        } else {
            Opts { node_gap: 34.0, rank_gap: if label_w > 0.0 { 70.0 } else { 54.0 }, pad: 18.0, title_h: TITLE_H }
        };
        let had = !self.lay.rects.is_empty();
        let before: Vec<Option<Rect>> = (0..self.flow.nodes.len()).map(|i| if had { self.shown_rect(i) } else { None }).collect();
        self.lay = layout::layout(&nodes, &edges, self.flow.dir, opts);
        if had {
            // nodes that appear grow out of the card they were folded into
            let mut prev = before.clone();
            for i in 0..prev.len() {
                if prev[i].is_some() {
                    continue;
                }
                let Some(new) = self.lay.rects[i] else { continue };
                let mut p = self.flow.nodes[i].parent;
                while let Some(g) = p {
                    if let Some(gr) = before[g] {
                        prev[i] = Some(Rect { x: gr.cx() - new.w * 0.3, y: gr.cy() - new.h * 0.3, w: new.w * 0.6, h: new.h * 0.6 });
                        break;
                    }
                    p = self.flow.nodes[g].parent;
                }
            }
            self.prev = prev;
            self.moved_at = Some(Instant::now());
        }
        self.recompute_lit();
    }

    pub fn set_all_collapsed(&mut self, c: bool) {
        for (i, n) in self.flow.nodes.iter().enumerate() {
            if n.group {
                self.collapsed[i] = c;
            }
        }
        self.relayout();
    }

    /// Highlight matches; returns the centre of the first one.
    pub fn search(&mut self, q: &str) -> Option<(f64, f64)> {
        let q = q.trim().to_lowercase();
        self.matches.clear();
        if q.is_empty() {
            return None;
        }
        for (i, n) in self.flow.nodes.iter().enumerate() {
            if n.label.to_lowercase().contains(&q) || n.id.to_lowercase().contains(&q) {
                // reveal: expand collapsed ancestors
                let mut p = n.parent;
                while let Some(g) = p {
                    if self.collapsed[g] {
                        self.collapsed[g] = false;
                    }
                    p = self.flow.nodes[g].parent;
                }
                self.matches.push(i);
            }
        }
        self.relayout();
        self.matches.first().and_then(|&i| self.lay.rects[i]).map(|r| (r.cx(), r.cy()))
    }

    fn visible_rep(&self, i: usize) -> usize {
        let mut r = i;
        let mut p = self.flow.nodes[i].parent;
        while let Some(g) = p {
            if self.collapsed[g] {
                r = g;
            }
            p = self.flow.nodes[g].parent;
        }
        r
    }

    fn recompute_lit(&mut self) {
        let Some(sel) = self.selected else { return };
        self.lit_nodes.clear();
        self.lit_edges.clear();
        if self.lay.rects.get(sel).copied().flatten().is_none() {
            self.select(None);
            return;
        }
        let vis: Vec<(usize, usize)> = self.flow.edges.iter().map(|e| (self.visible_rep(e.from), self.visible_rep(e.to))).collect();
        self.lit_nodes.insert(sel);
        for forward in [true, false] {
            let mut stack = vec![sel];
            let mut seen = HashSet::from([sel]);
            while let Some(x) = stack.pop() {
                for (k, &(a, b)) in vis.iter().enumerate() {
                    let (from, to) = if forward { (a, b) } else { (b, a) };
                    if from == x && from != to {
                        self.lit_edges.insert(k);
                        if seen.insert(to) {
                            self.lit_nodes.insert(to);
                            stack.push(to);
                        }
                    }
                }
            }
        }
    }

    /// The node under p: leaves and collapsed groups first, then expanded group title bars.
    fn hit(&self, p: (f64, f64)) -> Option<(usize, bool)> {
        let mut best: Option<(usize, f64)> = None;
        for (i, r) in self.lay.rects.iter().enumerate() {
            let Some(r) = r else { continue };
            let n = &self.flow.nodes[i];
            if n.group && !self.collapsed[i] {
                continue;
            }
            if r.contains(p.0, p.1) {
                let area = r.w * r.h;
                if best.is_none_or(|b| area < b.1) {
                    best = Some((i, area));
                }
            }
        }
        if let Some((i, _)) = best {
            return Some((i, false));
        }
        let mut title: Option<(usize, f64)> = None;
        for (i, r) in self.lay.rects.iter().enumerate() {
            let Some(r) = r else { continue };
            if self.flow.nodes[i].group && !self.collapsed[i] {
                let bar = Rect { x: r.x, y: r.y, w: r.w, h: TITLE_H };
                if bar.contains(p.0, p.1) && title.is_none_or(|t| r.w * r.h < t.1) {
                    title = Some((i, r.w * r.h));
                }
            }
        }
        title.map(|(i, _)| (i, true))
    }

    fn depth(&self, mut i: usize) -> usize {
        let mut d = 0;
        while let Some(p) = self.flow.nodes[i].parent {
            d += 1;
            i = p;
        }
        d
    }
}

impl Model for FlowModel {
    fn animating(&self) -> bool {
        self.moving() || self.dim_at.is_some_and(|at| ease(at, FADE_MS) < 1.0)
    }

    fn size(&self) -> (f64, f64) {
        (self.lay.width, self.lay.height)
    }

    fn draw(&self, cr: &cairo::Context, w: &gtk::DrawingArea) {
        let t = draw::theme();
        let sel = self.selected.is_some();
        let level = self.dim_level();
        let dim = |lit: bool| if lit { 1.0 } else { level };
        let moving = self.moving();
        let horizontal = self.flow.dir.horizontal();

        // groups, outermost first: soft containers with a header row
        let mut groups: Vec<usize> = (0..self.flow.nodes.len())
            .filter(|&i| self.flow.nodes[i].group && !self.collapsed[i] && self.shown_rect(i).is_some())
            .collect();
        groups.sort_by_key(|&i| self.depth(i));
        for &g in &groups {
            let r = self.shown_rect(g).unwrap();
            let st = self.flow.style_of(g);
            let tint = st.fill.as_deref().and_then(draw::parse_color);
            cr.push_group();
            let base = tint.map(|c| draw::mix(t.bg, c, if t.dark { 0.10 } else { 0.08 })).unwrap_or(t.group);
            set(cr, draw::mix(base, t.fg, 0.015 * self.depth(g) as f64), 1.0);
            draw::rounded(cr, r.x, r.y, r.w, r.h, 12.0);
            let _ = cr.fill_preserve();
            set(cr, st.stroke.as_deref().and_then(draw::parse_color).unwrap_or(t.border), 1.0);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            // header divider
            set(cr, t.border, 0.6);
            cr.move_to(r.x + 1.0, r.y + TITLE_H + 0.5);
            cr.line_to(r.right() - 1.0, r.y + TITLE_H + 0.5);
            let _ = cr.stroke();
            let hovered = self.hover == Some(g);
            let ink = if hovered { t.accent } else { t.muted };
            draw::text(cr, w, "▾", r.x + 12.0, r.y + 6.0, SMALL, ink, 1.0, 0.0, None);
            let (tw, _) = draw::text(cr, w, &self.flow.nodes[g].label, r.x + 28.0, r.y + 7.0, SMALL_BOLD, if hovered { t.accent } else { t.fg }, 0.9, 0.0, Some(r.w - 80.0));
            let n = self.hidden_count(g);
            draw::badge(cr, w, &n.to_string(), (r.x + 28.0 + tw + 18.0).min(r.right() - 20.0), r.y + TITLE_H / 2.0, Font { scale: 0.72, ..SMALL }, t.muted, t.card, t.border);
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(0.7 + 0.3 * (level - DIM) / (1.0 - DIM));
        }

        // ports: where edges leave and enter cards
        let port = |r: &Rect, toward: (f64, f64)| -> (f64, f64) {
            if horizontal {
                if toward.0 >= r.cx() { (r.right(), r.cy()) } else { (r.x, r.cy()) }
            } else if toward.1 >= r.cy() {
                (r.cx(), r.bottom())
            } else {
                (r.cx(), r.y)
            }
        };
        let mut ports: Vec<((f64, f64), f64)> = vec![];

        // edges
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        let mut labels = vec![];
        // Edges folded into the same pair of cards are drawn once, their labels merged.
        let mut bundles: Vec<(usize, usize, Vec<usize>)> = vec![];
        let mut index: HashMap<(usize, usize), usize> = HashMap::new();
        for (k, route) in self.lay.routes.iter().enumerate() {
            let Some(route) = route else { continue };
            if self.flow.edges[k].kind == EdgeKind::Invisible {
                continue;
            }
            match index.get(&(route.from, route.to)) {
                Some(&b) => bundles[b].2.push(k),
                None => {
                    index.insert((route.from, route.to), bundles.len());
                    bundles.push((route.from, route.to, vec![k]));
                }
            }
        }
        for (from, to, ks) in &bundles {
            let k = ks[0];
            let route = self.lay.routes[k].as_ref().unwrap();
            let e = &self.flow.edges[k];
            let (Some(ra), Some(rb)) = (self.shown_rect(*from), self.shown_rect(*to)) else { continue };
            let direct = route.points.is_empty() || moving;
            let first = if direct { (rb.cx(), rb.cy()) } else { route.points[0] };
            let last = if direct { (ra.cx(), ra.cy()) } else { *route.points.last().unwrap() };
            let (mut a, mut b) = (port(&ra, first), port(&rb, last));
            // a pair linked both ways: bend the two curves apart
            if index.contains_key(&(*to, *from)) {
                let off = if from < to { -7.0 } else { 7.0 };
                if horizontal {
                    a.1 += off;
                    b.1 += off;
                } else {
                    a.0 += off;
                    b.0 += off;
                }
            }
            let lit = ks.iter().any(|k| self.lit_edges.contains(k));
            let shown_lit = lit && sel;
            let hot = self.hover.is_some_and(|h| h == *from || h == *to);
            let alpha = dim(lit);
            ports.push((a, alpha));
            ports.push((b, alpha));
            set(cr, if shown_lit || hot { t.accent } else { t.edge }, alpha);
            let thick = ks.iter().any(|k| self.flow.edges[*k].kind == EdgeKind::Thick);
            cr.set_line_width(if thick {
                2.8
            } else if shown_lit {
                2.2
            } else if hot || ks.len() > 1 {
                1.8
            } else {
                1.25
            });
            if ks.iter().all(|k| self.flow.edges[*k].kind == EdgeKind::Dotted) {
                cr.set_dash(&[3.0, 4.0], 0.0);
            }
            // the curve leaves and enters along the flow axis
            let axis = |p: (f64, f64), r: &Rect, d: f64| -> (f64, f64) {
                if horizontal {
                    (p.0 + if p.0 >= r.cx() { d } else { -d }, p.1)
                } else {
                    (p.0, p.1 + if p.1 >= r.cy() { d } else { -d })
                }
            };
            let (mid, tail) = if direct {
                draw::port_curve(cr, a, b, horizontal);
                (((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), axis(b, &rb, 10.0))
            } else {
                let mut pts = vec![a, axis(a, &ra, 14.0)];
                pts.extend(route.points.iter().copied());
                pts.push(axis(b, &rb, 14.0));
                pts.push(b);
                draw::smooth_path(cr, &pts);
                let n = pts.len();
                (pts[n / 2], pts[n - 2])
            };
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            if ks.iter().any(|k| self.flow.edges[*k].arrow_end) {
                draw::arrow_head(cr, b, tail, 7.0);
            }
            if e.arrow_start {
                draw::arrow_head(cr, a, axis(a, &ra, if direct { 10.0 } else { 14.0 }), 7.0);
            }
            let mut texts: Vec<&str> = ks.iter().map(|k| self.flow.edges[*k].label.as_str()).filter(|l| !l.is_empty()).collect();
            texts.dedup();
            let text = match texts.len() {
                0 if ks.len() > 1 => format!("{} links", ks.len()),
                0 => String::new(),
                1 if ks.len() == 1 => texts[0].to_string(),
                1 | 2 => texts.join(" · "),
                _ => format!("{} links", ks.len()),
            };
            if !text.is_empty() {
                labels.push((text, mid, alpha, shown_lit));
            }
        }
        for (text, mid, alpha, lit) in labels {
            cr.push_group();
            draw::badge(cr, w, &text, mid.0, mid.1, SMALL, if lit { t.accent } else { t.muted }, t.card, if lit { t.accent } else { t.border });
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(alpha);
        }

        // cards
        for (i, n) in self.flow.nodes.iter().enumerate() {
            let Some(r) = self.shown_rect(i) else { continue };
            if n.group && !self.collapsed[i] {
                continue;
            }
            let lit = self.lit_nodes.contains(&i);
            let st = self.flow.style_of(i);
            let kc = st.fill.as_deref().and_then(draw::parse_color).unwrap_or_else(|| kind_color(n.shape));
            let stroke = st.stroke.as_deref().and_then(draw::parse_color);
            let hot = self.hover == Some(i);
            let selected = self.selected == Some(i);
            cr.push_group();
            if n.group {
                // collapsed group: a stack of cards
                for k in [2.0, 1.0] {
                    set(cr, draw::mix(t.card, t.bg, 0.35), 1.0);
                    draw::rounded(cr, r.x + 4.0 * k, r.y - 3.0 * k + 3.0, r.w - 8.0 * k, r.h, 9.0);
                    let _ = cr.fill_preserve();
                    set(cr, t.border, 1.0);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
            }
            draw::shadow(cr, r.x, r.y, r.w, r.h, 9.0, t.dark);
            set(cr, t.card, 1.0);
            draw::rounded(cr, r.x, r.y, r.w, r.h, 9.0);
            let _ = cr.fill();
            if selected {
                set(cr, t.accent, 0.18);
                cr.set_line_width(6.0);
                draw::rounded(cr, r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0, 11.0);
                let _ = cr.stroke();
            }
            let border = if selected || hot || lit && sel { t.accent } else { stroke.unwrap_or(t.border) };
            set(cr, border, 1.0);
            cr.set_line_width(if selected { 2.0 } else if hot { 1.5 } else { 1.0 });
            if st.dashed || n.group {
                cr.set_dash(&[4.0, 3.0], 0.0);
            }
            draw::rounded(cr, r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 9.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);

            // icon tile with the node's shape
            let (tx, ty) = (r.x + PAD, r.cy() - TILE / 2.0);
            set(cr, kc, if t.dark { 0.20 } else { 0.12 });
            draw::rounded(cr, tx, ty, TILE, TILE, 7.0);
            let _ = cr.fill();
            set(cr, kc, 1.0);
            cr.set_line_width(1.5);
            if n.group {
                mini_shape(cr, Shape::Rect, tx + TILE / 2.0 + 2.0, ty + TILE / 2.0 + 2.0, 12.0);
                let _ = cr.stroke();
                set(cr, kc, if t.dark { 0.20 } else { 0.12 });
                draw::rounded(cr, tx + 7.0, ty + 7.0, 13.0, 10.0, 1.5);
                let _ = cr.fill();
                set(cr, kc, 1.0);
                draw::rounded(cr, tx + 7.0, ty + 7.0, 13.0, 10.0, 1.5);
                let _ = cr.stroke();
            } else {
                mini_shape(cr, n.shape, tx + TILE / 2.0, ty + TILE / 2.0, 14.0);
                let _ = cr.stroke();
            }

            // text
            let ink = st.color.as_deref().and_then(draw::parse_color).unwrap_or(t.fg);
            let x = tx + TILE + 10.0;
            let max = r.right() - x - PAD - if n.group { 44.0 } else { 0.0 };
            if n.group {
                draw::text(cr, w, &n.label, x, r.cy() - 17.0, TITLE_FONT, ink, 1.0, 0.0, Some(max));
                draw::text(cr, w, &Self::group_note(self.hidden_count(i)), x, r.cy() + 1.0, SMALL, t.muted, 1.0, 0.0, Some(max));
                draw::badge(cr, w, &format!("+{}", self.hidden_count(i)), r.right() - 26.0, r.cy(), Font { scale: 0.75, ..SMALL_BOLD }, t.accent, t.card, t.border);
            } else {
                let (title, subtitle) = split_label(&n.label);
                if subtitle.is_empty() {
                    draw::text(cr, w, &title, x, r.cy() - 9.0, TITLE_FONT, ink, 1.0, 0.0, Some(max));
                } else {
                    draw::text(cr, w, &title, x, r.cy() - 18.0, TITLE_FONT, ink, 1.0, 0.0, Some(max));
                    draw::text(cr, w, &subtitle, x, r.cy() + 1.0, SMALL, t.muted, 1.0, 0.0, Some(max));
                }
                if n.link.is_some() {
                    draw::text(cr, w, "↗", r.right() - 8.0, r.y + 4.0, Font { scale: 0.72, ..SMALL }, t.accent, 1.0, 1.0, None);
                }
            }
            if self.matches.contains(&i) {
                set(cr, t.accent, 1.0);
                cr.set_line_width(2.0);
                cr.set_dash(&[5.0, 3.0], 0.0);
                draw::rounded(cr, r.x - 5.0, r.y - 5.0, r.w + 10.0, r.h + 10.0, 13.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(dim(lit));
        }

        // port dots on top of the card borders
        for (p, alpha) in ports {
            set(cr, t.card, alpha);
            // text leaves the pen where the last label was; never join it to the dot
            cr.new_path();
            cr.arc(p.0, p.1, 3.5, 0.0, 2.0 * PI);
            let _ = cr.fill_preserve();
            set(cr, t.edge, alpha);
            cr.set_line_width(1.2);
            let _ = cr.stroke();
        }
    }

    fn hover(&mut self, p: Option<(f64, f64)>) -> bool {
        let h = p.and_then(|p| self.hit(p)).map(|(i, _)| i);
        if h != self.hover {
            self.hover = h;
            return true;
        }
        false
    }

    fn tooltip(&self, p: (f64, f64)) -> Option<String> {
        let (i, title) = self.hit(p)?;
        let n = &self.flow.nodes[i];
        if n.group {
            return Some(if title || self.collapsed[i] { "Click to expand or collapse".into() } else { n.label.clone() });
        }
        let mut s = n.tip.clone().unwrap_or_default();
        if let Some(l) = &n.link {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&format!("{l}\nDouble-click or Ctrl+click to open"));
        }
        (!s.is_empty()).then_some(s)
    }

    fn click(&mut self, p: (f64, f64), n: i32, mods: gdk::ModifierType) -> Reaction {
        match self.hit(p) {
            Some((i, true)) => {
                self.collapsed[i] = true;
                self.relayout();
                Reaction::Redraw
            }
            Some((i, false)) if self.flow.nodes[i].group => {
                self.collapsed[i] = false;
                self.relayout();
                Reaction::Redraw
            }
            Some((i, false)) => {
                let node = &self.flow.nodes[i];
                if let Some(link) = node.link.clone() {
                    if n >= 2 || mods.contains(gdk::ModifierType::CONTROL_MASK) {
                        let (path, line) = match link.rsplit_once(':') {
                            Some((p, l)) if l.chars().all(|c| c.is_ascii_digit()) && !l.is_empty() && !link.starts_with("http") => {
                                (p.to_string(), l.parse().ok())
                            }
                            _ => (link.clone(), None),
                        };
                        return Reaction::Open(path, line);
                    }
                }
                self.select(if self.selected == Some(i) { None } else { Some(i) });
                Reaction::Redraw
            }
            None => {
                if self.selected.is_some() || !self.matches.is_empty() {
                    self.matches.clear();
                    self.select(None);
                    return Reaction::Redraw;
                }
                Reaction::None
            }
        }
    }
}
