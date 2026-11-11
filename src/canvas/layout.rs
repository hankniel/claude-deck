//! Layered graph layout (Sugiyama style) with nested, collapsible groups.
//!
//! Each expanded group is laid out on its own and then placed as one block in its parent, so
//! groups never overlap. Edges between groups are routed at the level where their endpoints'
//! groups are siblings. Sizes come from the caller (the UI measures text).

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    LR,
    RL,
    TB,
    BT,
}

impl Dir {
    pub fn parse(s: &str) -> Dir {
        match s.to_ascii_uppercase().as_str() {
            "LR" => Dir::LR,
            "RL" => Dir::RL,
            "BT" => Dir::BT,
            _ => Dir::TB,
        }
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Dir::LR | Dir::RL)
    }
}

#[derive(Clone, Debug)]
pub struct LNode {
    pub w: f64,
    pub h: f64,
    pub parent: Option<usize>,
    pub group: bool,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Opts {
    pub node_gap: f64,
    pub rank_gap: f64,
    pub pad: f64,
    pub title_h: f64,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { node_gap: 28.0, rank_gap: 56.0, pad: 16.0, title_h: 26.0 }
    }
}

/// An edge after collapsing: drawn between two visible nodes through `points`.
#[derive(Clone, Debug)]
pub struct Route {
    pub from: usize,
    pub to: usize,
    pub points: Vec<(f64, f64)>,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// Absolute rectangle of every visible node (groups included); None when hidden.
    pub rects: Vec<Option<Rect>>,
    /// Per input edge; None when it collapsed into a single node.
    pub routes: Vec<Option<Route>>,
    pub width: f64,
    pub height: f64,
}

struct Ctx<'a> {
    nodes: &'a [LNode],
    dir: Dir,
    opts: Opts,
    visible: Vec<bool>,
    rep: Vec<usize>,
    /// Local (block-relative) rects of each visible node.
    local: Vec<Option<Rect>>,
    /// Routes per block level: (container, from child, to child) -> local points.
    level_routes: HashMap<(Option<usize>, usize, usize), Vec<(f64, f64)>>,
    edges: Vec<(usize, usize)>,
}

impl<'a> Ctx<'a> {
    fn children(&self, c: Option<usize>) -> Vec<usize> {
        (0..self.nodes.len()).filter(|&i| self.visible[i] && self.nodes[i].parent == c).collect()
    }

    /// The child of container `c` that contains `x` (x itself if it is a direct child).
    fn child_of(&self, c: Option<usize>, mut x: usize) -> Option<usize> {
        loop {
            if self.nodes[x].parent == c {
                return Some(x);
            }
            x = self.nodes[x].parent?;
        }
    }

    fn expanded_group(&self, i: usize) -> bool {
        self.nodes[i].group && !self.nodes[i].collapsed
    }

    /// Lay out the children of `c`; returns the content size.
    fn block(&mut self, c: Option<usize>) -> (f64, f64) {
        let kids = self.children(c);
        let mut sizes = vec![];
        for &k in &kids {
            if self.expanded_group(k) && !self.children(Some(k)).is_empty() {
                let (w, h) = self.block(Some(k));
                let p = self.opts.pad;
                let w = (w + 2.0 * p).max(self.nodes[k].w);
                sizes.push((w, h + 2.0 * p + self.opts.title_h));
            } else {
                sizes.push((self.nodes[k].w, self.nodes[k].h));
            }
        }
        let index: HashMap<usize, usize> = kids.iter().enumerate().map(|(i, &k)| (k, i)).collect();
        let mut level_edges = vec![];
        for &(a, b) in &self.edges {
            let (ra, rb) = (self.rep[a], self.rep[b]);
            if let (Some(ca), Some(cb)) = (self.child_of(c, ra), self.child_of(c, rb)) {
                if ca != cb && !level_edges.contains(&(index[&ca], index[&cb])) {
                    level_edges.push((index[&ca], index[&cb]));
                }
            }
        }
        let res = layered(&sizes, &level_edges, self.dir, &self.opts);
        for (i, &k) in kids.iter().enumerate() {
            let (x, y) = res.pos[i];
            self.local[k] = Some(Rect { x, y, w: sizes[i].0, h: sizes[i].1 });
        }
        for ((a, b), pts) in res.routes {
            self.level_routes.insert((c, kids[a], kids[b]), pts);
        }
        (res.w, res.h)
    }

    /// Turn local rects into absolute ones.
    fn place(&self, c: Option<usize>, ox: f64, oy: f64, out: &mut Vec<Option<Rect>>, origins: &mut HashMap<Option<usize>, (f64, f64)>) {
        origins.insert(c, (ox, oy));
        for k in self.children(c) {
            let Some(r) = self.local[k] else { continue };
            let abs = Rect { x: ox + r.x, y: oy + r.y, ..r };
            out[k] = Some(abs);
            if self.expanded_group(k) {
                self.place(Some(k), abs.x + self.opts.pad, abs.y + self.opts.pad + self.opts.title_h, out, origins);
            }
        }
    }
}

/// Lay out `nodes` (groups via `parent`) with `edges` in direction `dir`.
pub fn layout(nodes: &[LNode], edges: &[(usize, usize)], dir: Dir, opts: Opts) -> Layout {
    let n = nodes.len();
    let mut visible = vec![true; n];
    let mut rep: Vec<usize> = (0..n).collect();
    for i in 0..n {
        let mut p = nodes[i].parent;
        let mut guard = 0;
        while let Some(q) = p {
            if nodes[q].collapsed {
                visible[i] = false;
                rep[i] = q;
            }
            p = nodes[q].parent;
            guard += 1;
            if guard > n {
                break; // parent cycle: treat as top level
            }
        }
    }
    // rep must be the outermost collapsed ancestor, which the loop above leaves in rep[i].
    let mut ctx = Ctx {
        nodes,
        dir,
        opts,
        visible,
        rep,
        local: vec![None; n],
        level_routes: HashMap::new(),
        edges: edges.to_vec(),
    };
    let (w, h) = ctx.block(None);
    let mut rects = vec![None; n];
    let mut origins = HashMap::new();
    ctx.place(None, 0.0, 0.0, &mut rects, &mut origins);

    let mut routes = vec![];
    for &(a, b) in edges {
        let (ra, rb) = (ctx.rep[a], ctx.rep[b]);
        if ra == rb || rects[ra].is_none() || rects[rb].is_none() {
            routes.push(None);
            continue;
        }
        // The lowest container holding both, where they sit under different children.
        let mut c = nodes[ra].parent;
        let level = loop {
            if let (Some(ca), Some(cb)) = (ctx.child_of(c, ra), ctx.child_of(c, rb)) {
                if ca != cb {
                    break Some((c, ca, cb));
                }
            }
            match c {
                Some(x) => c = nodes[x].parent,
                None => break None,
            }
        };
        let points = match level {
            Some((c, ca, cb)) => {
                let (ox, oy) = origins.get(&c).copied().unwrap_or((0.0, 0.0));
                ctx.level_routes
                    .get(&(c, ca, cb))
                    .map(|pts| pts.iter().map(|(x, y)| (x + ox, y + oy)).collect())
                    .unwrap_or_default()
            }
            None => vec![],
        };
        routes.push(Some(Route { from: ra, to: rb, points }));
    }
    Layout { rects, routes, width: w, height: h }
}

struct Layered {
    pos: Vec<(f64, f64)>,
    routes: HashMap<(usize, usize), Vec<(f64, f64)>>,
    w: f64,
    h: f64,
}

/// One block: items with sizes and edges between them (indices into `sizes`).
fn layered(sizes: &[(f64, f64)], edges: &[(usize, usize)], dir: Dir, o: &Opts) -> Layered {
    let n = sizes.len();
    if n == 0 {
        return Layered { pos: vec![], routes: HashMap::new(), w: 0.0, h: 0.0 };
    }
    let horiz = dir.horizontal();
    // cross = size across layers, main = size along the flow.
    let cross: Vec<f64> = sizes.iter().map(|s| if horiz { s.1 } else { s.0 }).collect();
    let main: Vec<f64> = sizes.iter().map(|s| if horiz { s.0 } else { s.1 }).collect();

    // 1. break cycles with a DFS; reversed edges keep their original direction for routing.
    let mut out_adj = vec![vec![]; n];
    for &(a, b) in edges {
        if a != b && a < n && b < n && !out_adj[a].contains(&b) {
            out_adj[a].push(b);
        }
    }
    let mut dag: Vec<(usize, usize, bool)> = vec![]; // (u, v, reversed)
    let mut color = vec![0u8; n];
    for s in 0..n {
        if color[s] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        color[s] = 1;
        while let Some(top) = stack.last_mut() {
            let u = top.0;
            if top.1 < out_adj[u].len() {
                let v = out_adj[u][top.1];
                top.1 += 1;
                match color[v] {
                    0 => {
                        dag.push((u, v, false));
                        color[v] = 1;
                        stack.push((v, 0));
                    }
                    1 => dag.push((v, u, true)),
                    _ => dag.push((u, v, false)),
                }
            } else {
                color[u] = 2;
                stack.pop();
            }
        }
    }
    dag.sort();
    dag.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);

    // 2. layers: longest path, then pull sources next to their first successor.
    let mut indeg = vec![0; n];
    let mut succ = vec![vec![]; n];
    let mut pred = vec![vec![]; n];
    for &(u, v, _) in &dag {
        indeg[v] += 1;
        succ[u].push(v);
        pred[v].push(u);
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let u = queue[qi];
        qi += 1;
        for &v in &succ[u] {
            layer[v] = layer[v].max(layer[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    for &u in queue.iter().rev() {
        if pred[u].is_empty() && !succ[u].is_empty() {
            let m = succ[u].iter().map(|&v| layer[v]).min().unwrap();
            layer[u] = m.saturating_sub(1).max(layer[u]);
        }
    }

    // 3. virtual graph with dummy nodes on long edges.
    let mut vlayer: Vec<usize> = layer.clone();
    let mut vcross: Vec<f64> = cross.clone();
    let mut vup: Vec<Vec<usize>> = vec![vec![]; n];
    let mut vdown: Vec<Vec<usize>> = vec![vec![]; n];
    let mut chains: Vec<(usize, usize, bool, Vec<usize>)> = vec![];
    for &(u, v, rev) in &dag {
        let mut prev = u;
        let mut chain = vec![];
        for l in layer[u] + 1..layer[v] {
            let d = vlayer.len();
            vlayer.push(l);
            vcross.push(2.0);
            vup.push(vec![]);
            vdown.push(vec![]);
            vdown[prev].push(d);
            vup[d].push(prev);
            chain.push(d);
            prev = d;
        }
        vdown[prev].push(v);
        vup[v].push(prev);
        chains.push((u, v, rev, chain));
    }
    let vn = vlayer.len();
    let nl = vlayer.iter().max().copied().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![vec![]; nl];
    for i in 0..vn {
        layers[vlayer[i]].push(i);
    }

    // 4. ordering: barycenter sweeps, keeping the order with the fewest crossings.
    let mut order = vec![0f64; vn];
    for l in &layers {
        for (i, &x) in l.iter().enumerate() {
            order[x] = i as f64;
        }
    }
    let crossings = |layers: &Vec<Vec<usize>>, order: &Vec<f64>| -> usize {
        let mut c = 0;
        for l in 0..layers.len().saturating_sub(1) {
            let mut es = vec![];
            for &u in &layers[l] {
                for &v in &vdown[u] {
                    es.push((order[u], order[v]));
                }
            }
            for i in 0..es.len() {
                for j in i + 1..es.len() {
                    if (es[i].0 - es[j].0) * (es[i].1 - es[j].1) < 0.0 {
                        c += 1;
                    }
                }
            }
        }
        c
    };
    let mut best = layers.clone();
    let mut best_c = crossings(&layers, &order);
    for it in 0..16 {
        let down = it % 2 == 0;
        let range: Vec<usize> = if down { (1..nl).collect() } else { (0..nl.saturating_sub(1)).rev().collect() };
        for l in range {
            let mut keyed: Vec<(f64, usize)> = layers[l]
                .iter()
                .map(|&x| {
                    let nb = if down { &vup[x] } else { &vdown[x] };
                    let b = if nb.is_empty() { order[x] } else { nb.iter().map(|&y| order[y]).sum::<f64>() / nb.len() as f64 };
                    (b, x)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(order[a.1].total_cmp(&order[b.1])));
            layers[l] = keyed.into_iter().map(|k| k.1).collect();
            for (i, &x) in layers[l].iter().enumerate() {
                order[x] = i as f64;
            }
        }
        let c = crossings(&layers, &order);
        if c < best_c {
            best_c = c;
            best = layers.clone();
        }
        if best_c == 0 {
            break;
        }
    }
    let layers = best;

    // 5. coordinates. Main axis: layers; cross axis: pack, then pull toward neighbours.
    let is_dummy = |i: usize| i >= n;
    let mut layer_main = vec![0f64; nl];
    for (l, xs) in layers.iter().enumerate() {
        layer_main[l] = xs.iter().filter(|&&x| !is_dummy(x)).map(|&x| main[x]).fold(0.0, f64::max);
    }
    let mut main_pos = vec![0f64; nl];
    let mut acc = 0.0;
    for l in 0..nl {
        main_pos[l] = acc + layer_main[l] / 2.0;
        acc += layer_main[l] + o.rank_gap;
    }
    let total_main = (acc - o.rank_gap).max(0.0);
    let sep = |a: usize, b: usize| {
        let g = if is_dummy(a) || is_dummy(b) { o.node_gap * 0.45 } else { o.node_gap };
        (vcross[a] + vcross[b]) / 2.0 + g
    };
    let mut x = vec![0f64; vn];
    for l in &layers {
        let mut cur = 0.0;
        for (i, &v) in l.iter().enumerate() {
            if i > 0 {
                cur += sep(l[i - 1], v);
            } else {
                cur = vcross[v] / 2.0;
            }
            x[v] = cur;
        }
    }
    // Centre layers on each other first so the pulls start from a balanced picture.
    let widest = layers
        .iter()
        .map(|l| l.last().map(|&v| x[v] + vcross[v] / 2.0).unwrap_or(0.0))
        .fold(0.0, f64::max);
    for l in &layers {
        let w = l.last().map(|&v| x[v] + vcross[v] / 2.0).unwrap_or(0.0);
        let off = (widest - w) / 2.0;
        for &v in l {
            x[v] += off;
        }
    }
    for it in 0..24 {
        let down = it % 2 == 0;
        let range: Vec<usize> = if down { (1..nl).collect() } else { (0..nl.saturating_sub(1)).rev().collect() };
        for l in range {
            let xs = &layers[l];
            if xs.is_empty() {
                continue;
            }
            let desired: Vec<f64> = xs
                .iter()
                .map(|&v| {
                    let nb: Vec<usize> = if down { vup[v].clone() } else { vdown[v].clone() };
                    let both: Vec<usize> = vup[v].iter().chain(vdown[v].iter()).copied().collect();
                    let use_ = if nb.is_empty() { both } else { nb };
                    if use_.is_empty() {
                        x[v]
                    } else {
                        use_.iter().map(|&y| x[y]).sum::<f64>() / use_.len() as f64
                    }
                })
                .collect();
            // Place in order honouring separation, then shift to the least total deviation.
            let mut p = desired.clone();
            for i in 1..xs.len() {
                p[i] = p[i].max(p[i - 1] + sep(xs[i - 1], xs[i]));
            }
            for i in (0..xs.len().saturating_sub(1)).rev() {
                p[i] = p[i].min(p[i + 1] - sep(xs[i], xs[i + 1])).max(desired[i].min(p[i]));
            }
            for i in 1..xs.len() {
                p[i] = p[i].max(p[i - 1] + sep(xs[i - 1], xs[i]));
            }
            let shift = desired.iter().zip(&p).map(|(d, q)| d - q).sum::<f64>() / xs.len() as f64;
            for (i, &v) in xs.iter().enumerate() {
                x[v] = p[i] + shift;
            }
        }
    }
    let min_x = (0..vn).map(|v| x[v] - vcross[v] / 2.0).fold(f64::INFINITY, f64::min);
    let max_x = (0..vn).map(|v| x[v] + vcross[v] / 2.0).fold(f64::NEG_INFINITY, f64::max);
    let total_cross = (max_x - min_x).max(0.0);

    // 6. back to x/y in the requested direction.
    let to_xy = |main_c: f64, cross_c: f64| -> (f64, f64) {
        let m = match dir {
            Dir::RL | Dir::BT => total_main - main_c,
            _ => main_c,
        };
        let c = cross_c - min_x;
        if horiz {
            (m, c)
        } else {
            (c, m)
        }
    };
    let mut pos = vec![(0.0, 0.0); n];
    for i in 0..n {
        let (cx, cy) = to_xy(main_pos[vlayer[i]], x[i]);
        pos[i] = (cx - sizes[i].0 / 2.0, cy - sizes[i].1 / 2.0);
    }
    let mut routes = HashMap::new();
    for (u, v, rev, chain) in chains {
        let mut pts: Vec<(f64, f64)> = chain.iter().map(|&d| to_xy(main_pos[vlayer[d]], x[d])).collect();
        let key = if rev {
            pts.reverse();
            (v, u)
        } else {
            (u, v)
        };
        routes.insert(key, pts);
    }
    let (w, h) = if horiz { (total_main, total_cross) } else { (total_cross, total_main) };
    Layered { pos, routes, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(parent: Option<usize>) -> LNode {
        LNode { w: 80.0, h: 30.0, parent, group: false, collapsed: false }
    }

    #[test]
    fn chain_goes_left_to_right() {
        let nodes = vec![leaf(None), leaf(None), leaf(None)];
        let l = layout(&nodes, &[(0, 1), (1, 2)], Dir::LR, Opts::default());
        let r: Vec<Rect> = l.rects.iter().map(|r| r.unwrap()).collect();
        assert!(r[0].x < r[1].x && r[1].x < r[2].x);
        assert!((r[0].cy() - r[2].cy()).abs() < 1.0);
    }

    #[test]
    fn groups_contain_children_and_collapse() {
        let mut nodes = vec![
            LNode { w: 60.0, h: 20.0, parent: None, group: true, collapsed: false },
            leaf(Some(0)),
            leaf(Some(0)),
            leaf(None),
        ];
        let l = layout(&nodes, &[(1, 2), (2, 3)], Dir::TB, Opts::default());
        let g = l.rects[0].unwrap();
        for c in [1, 2] {
            let r = l.rects[c].unwrap();
            assert!(r.x >= g.x && r.right() <= g.right() && r.y >= g.y && r.bottom() <= g.bottom());
        }
        nodes[0].collapsed = true;
        let l = layout(&nodes, &[(1, 2), (2, 3)], Dir::TB, Opts::default());
        assert!(l.rects[1].is_none());
        assert!(l.routes[0].is_none());
        assert_eq!(l.routes[1].as_ref().unwrap().from, 0);
    }

    #[test]
    fn cycles_and_long_edges() {
        let nodes = vec![leaf(None), leaf(None), leaf(None), leaf(None)];
        let l = layout(&nodes, &[(0, 1), (1, 2), (2, 0), (0, 3), (3, 2)], Dir::TB, Opts::default());
        assert!(l.rects.iter().all(|r| r.is_some()));
        assert_eq!(l.routes.len(), 5);
    }
}
