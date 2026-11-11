//! Interactive flowchart: collapsible groups, lineage highlight on click, search, links.

use super::draw::{self, set, Font, BODY, SMALL, SMALL_BOLD};
use super::view::{Model, Reaction};
use claude_deck::canvas::flow::{EdgeKind, Flow, Shape};
use claude_deck::canvas::layout::{self, LNode, Layout, Opts, Rect};
use gtk::{cairo, gdk};
use std::collections::HashSet;
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
}

fn shape_size(shape: Shape, tw: f64, th: f64) -> (f64, f64) {
    let (w, h) = (tw + 28.0, th + 18.0);
    match shape {
        Shape::Diamond => ((w * 1.5).max(70.0), (h * 1.6).max(50.0)),
        Shape::Circle => {
            let d = w.max(h) + 6.0;
            (d, d)
        }
        Shape::Hexagon => (w + 24.0, h + 4.0),
        Shape::Cylinder => (w, h + 14.0),
        Shape::Parallelogram => (w + 22.0, h),
        Shape::Stadium => (w + 12.0, h),
        _ => (w.max(64.0), h.max(36.0)),
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
                let (tw, th) = draw::measure(&self.meas, &n.label, if n.group { SMALL_BOLD } else { BODY });
                if n.group {
                    (tw + 70.0, TITLE_H + 14.0)
                } else {
                    shape_size(n.shape, tw, th)
                }
            })
            .collect();
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
                    let (tw, _) = draw::measure(&self.meas, &format!("▸  {}", n.label), BODY);
                    w = tw + 80.0;
                    h = 44.0;
                }
                LNode { w, h, parent: n.parent, group: n.group, collapsed: n.group && self.collapsed[i] }
            })
            .collect();
        let edges: Vec<(usize, usize)> = self.flow.edges.iter().map(|e| (e.from, e.to)).collect();
        let opts = if self.flow.dir.horizontal() {
            Opts { node_gap: 22.0, rank_gap: 64.0, pad: 18.0, title_h: TITLE_H }
        } else {
            Opts { node_gap: 30.0, rank_gap: 48.0, pad: 18.0, title_h: TITLE_H }
        };
        self.lay = layout::layout(&nodes, &edges, self.flow.dir, opts);
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
        self.lit_nodes.clear();
        self.lit_edges.clear();
        let Some(sel) = self.selected else { return };
        if self.lay.rects.get(sel).copied().flatten().is_none() {
            self.selected = None;
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

    fn shape_path(cr: &cairo::Context, shape: Shape, r: &Rect) {
        let (x, y, w, h) = (r.x, r.y, r.w, r.h);
        match shape {
            Shape::Rect | Shape::Subroutine => draw::rounded(cr, x, y, w, h, 6.0),
            Shape::Round => draw::rounded(cr, x, y, w, h, 12.0),
            Shape::Stadium => draw::rounded(cr, x, y, w, h, h / 2.0),
            Shape::Circle => {
                cr.new_sub_path();
                cr.arc(r.cx(), r.cy(), w.min(h) / 2.0, 0.0, 2.0 * PI);
            }
            Shape::Diamond => {
                cr.move_to(r.cx(), y);
                cr.line_to(x + w, r.cy());
                cr.line_to(r.cx(), y + h);
                cr.line_to(x, r.cy());
                cr.close_path();
            }
            Shape::Hexagon => {
                let k = 14.0;
                cr.move_to(x + k, y);
                cr.line_to(x + w - k, y);
                cr.line_to(x + w, r.cy());
                cr.line_to(x + w - k, y + h);
                cr.line_to(x + k, y + h);
                cr.line_to(x, r.cy());
                cr.close_path();
            }
            Shape::Parallelogram => {
                let k = 12.0;
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
                cr.line_to(x + 12.0, r.cy());
                cr.close_path();
            }
            Shape::Cylinder => {
                let e = 7.0;
                cr.new_sub_path();
                cr.move_to(x, y + e);
                cr.curve_to(x, y - e / 3.0, x + w, y - e / 3.0, x + w, y + e);
                cr.line_to(x + w, y + h - e);
                cr.curve_to(x + w, y + h + e / 3.0, x, y + h + e / 3.0, x, y + h - e);
                cr.close_path();
            }
        }
    }
}

impl Model for FlowModel {
    fn size(&self) -> (f64, f64) {
        (self.lay.width, self.lay.height)
    }

    fn draw(&self, cr: &cairo::Context, w: &gtk::DrawingArea) {
        let t = draw::theme();
        let sel = self.selected.is_some();
        let dim = |lit: bool| if sel && !lit { 0.28 } else { 1.0 };

        // groups, outermost first
        let mut groups: Vec<usize> = (0..self.flow.nodes.len())
            .filter(|&i| self.flow.nodes[i].group && !self.collapsed[i] && self.lay.rects[i].is_some())
            .collect();
        groups.sort_by_key(|&i| self.depth(i));
        for &g in &groups {
            let r = self.lay.rects[g].unwrap();
            let st = self.flow.style_of(g);
            let base = st.fill.as_deref().and_then(draw::parse_color).unwrap_or(t.group);
            let shade = draw::mix(base, t.fg, 0.025 * self.depth(g) as f64);
            set(cr, shade, if st.fill.is_some() { 0.35 } else { 1.0 });
            draw::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
            let _ = cr.fill_preserve();
            set(cr, st.stroke.as_deref().and_then(draw::parse_color).unwrap_or(t.border), 1.0);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let hovered = self.hover == Some(g);
            let title = format!("▾  {}", self.flow.nodes[g].label);
            draw::text(cr, w, &title, r.x + 12.0, r.y + 7.0, SMALL_BOLD, if hovered { t.accent } else { t.muted }, 1.0, 0.0, Some(r.w - 24.0));
        }

        // edges
        cr.set_line_cap(cairo::LineCap::Round);
        for (k, route) in self.lay.routes.iter().enumerate() {
            let Some(route) = route else { continue };
            let e = &self.flow.edges[k];
            if e.kind == EdgeKind::Invisible {
                continue;
            }
            let (Some(ra), Some(rb)) = (self.lay.rects[route.from], self.lay.rects[route.to]) else { continue };
            let first = route.points.first().copied().unwrap_or((rb.cx(), rb.cy()));
            let last = route.points.last().copied().unwrap_or((ra.cx(), ra.cy()));
            let mut pts = vec![draw::boundary(&ra, first)];
            pts.extend(route.points.iter().copied());
            pts.push(draw::boundary(&rb, last));
            let lit = self.lit_edges.contains(&k);
            let hot = self.hover.is_some_and(|h| h == route.from || h == route.to);
            let color = if lit || hot { t.accent } else { t.edge };
            let a = dim(lit);
            set(cr, color, a);
            cr.set_line_width(match e.kind {
                EdgeKind::Thick => 2.6,
                _ if lit || hot => 1.8,
                _ => 1.3,
            });
            if e.kind == EdgeKind::Dotted {
                cr.set_dash(&[4.0, 4.0], 0.0);
            }
            draw::smooth_path(cr, &pts);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            let n = pts.len();
            if e.arrow_end {
                draw::arrow_head(cr, pts[n - 1], pts[n - 2], 8.0);
            }
            if e.arrow_start {
                draw::arrow_head(cr, pts[0], pts[1], 8.0);
            }
            if !e.label.is_empty() {
                let mid = if n % 2 == 1 {
                    pts[n / 2]
                } else {
                    ((pts[n / 2 - 1].0 + pts[n / 2].0) / 2.0, (pts[n / 2 - 1].1 + pts[n / 2].1) / 2.0)
                };
                cr.push_group();
                draw::pill(cr, w, &e.label, mid.0, mid.1, SMALL, t.muted, t.bg);
                let _ = cr.pop_group_to_source();
                let _ = cr.paint_with_alpha(a);
            }
        }

        // nodes
        for (i, n) in self.flow.nodes.iter().enumerate() {
            let Some(r) = self.lay.rects[i] else { continue };
            if n.group && !self.collapsed[i] {
                continue;
            }
            let lit = self.lit_nodes.contains(&i);
            let a = dim(lit);
            let st = self.flow.style_of(i);
            let fill = st.fill.as_deref().and_then(draw::parse_color).unwrap_or(t.card);
            let stroke = st.stroke.as_deref().and_then(draw::parse_color).unwrap_or(t.border);
            let ink = st
                .color
                .as_deref()
                .and_then(draw::parse_color)
                .unwrap_or(if st.fill.is_none() {
                    t.fg
                } else if draw::luminance(fill) < 0.5 {
                    (1.0, 1.0, 1.0)
                } else {
                    crate::widgets::hex(0x1F1E1D)
                });
            cr.push_group();
            if n.group {
                // collapsed group
                set(cr, t.group, 1.0);
                draw::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
                let _ = cr.fill_preserve();
                set(cr, t.muted, 0.8);
                cr.set_line_width(1.2);
                cr.set_dash(&[5.0, 3.0], 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                draw::text(cr, w, &format!("▸  {}", n.label), r.x + 12.0, r.cy() - 9.0, BODY, t.fg, 1.0, 0.0, Some(r.w - 56.0));
                draw::pill(cr, w, &format!("+{}", self.hidden_count(i)), r.right() - 22.0, r.cy(), SMALL, t.muted, t.card);
            } else {
                Self::shape_path(cr, n.shape, &r);
                set(cr, fill, 1.0);
                let _ = cr.fill_preserve();
                let hot = self.hover == Some(i) || self.selected == Some(i);
                set(cr, if hot { t.accent } else { stroke }, 1.0);
                cr.set_line_width(st.stroke_width.unwrap_or(if hot { 1.8 } else { 1.0 }));
                if st.dashed {
                    cr.set_dash(&[4.0, 3.0], 0.0);
                }
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                if n.shape == Shape::Subroutine {
                    set(cr, stroke, 1.0);
                    cr.move_to(r.x + 7.0, r.y);
                    cr.line_to(r.x + 7.0, r.bottom());
                    cr.move_to(r.right() - 7.0, r.y);
                    cr.line_to(r.right() - 7.0, r.bottom());
                    let _ = cr.stroke();
                }
                if n.shape == Shape::Cylinder {
                    set(cr, stroke, 1.0);
                    cr.move_to(r.x, r.y + 7.0);
                    cr.curve_to(r.x, r.y + 7.0 + 9.0, r.right(), r.y + 7.0 + 9.0, r.right(), r.y + 7.0);
                    let _ = cr.stroke();
                }
                let cy = if n.shape == Shape::Cylinder { r.cy() + 4.0 } else { r.cy() };
                draw::text_centered(cr, w, &n.label, r.cx(), cy, BODY, ink, 1.0);
                if n.link.is_some() {
                    draw::text(cr, w, "↗", r.right() - 5.0, r.y + 2.0, Font { scale: 0.7, ..SMALL }, t.accent, 1.0, 1.0, None);
                }
            }
            if self.matches.contains(&i) {
                set(cr, t.accent, 1.0);
                cr.set_line_width(2.5);
                draw::rounded(cr, r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0, 9.0);
                let _ = cr.stroke();
            }
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(a);
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
                self.selected = if self.selected == Some(i) { None } else { Some(i) };
                self.recompute_lit();
                Reaction::Redraw
            }
            None => {
                if self.selected.take().is_some() || !self.matches.is_empty() {
                    self.matches.clear();
                    self.recompute_lit();
                    return Reaction::Redraw;
                }
                Reaction::None
            }
        }
    }
}
