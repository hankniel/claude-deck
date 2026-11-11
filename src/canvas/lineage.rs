//! `.lineage` — data lineage drawn like Databricks Unity Catalog, in a compact text format:
//!
//! ```text
//! title: Orders pipeline
//! focus: main.silver.orders                       # highlighted, like the table you opened
//! main.bronze.orders   table     id, user_id, amount:decimal, ts
//! etl/clean_orders     notebook
//! nightly_etl          job
//! main.bronze.orders, main.bronze.fx > etl/clean_orders > main.silver.orders > sales_dash
//! main.silver.orders.amount_usd < main.bronze.orders.amount, main.bronze.fx.rate
//! ```
//!
//! Lines are declarations (`name [kind] [columns]`), edges (`a, b > c > d`, `->` works too) or
//! column lineage (`target.col < source.col, …`). Any name used only in an edge is a table.
//! Kinds: table view mv streaming notebook job pipeline dashboard query model volume file external.

use std::collections::HashSet;

pub const KINDS: &[&str] = &[
    "table", "view", "mv", "streaming", "notebook", "job", "pipeline", "dashboard", "query", "model", "volume", "file",
    "external", "source",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub ty: String,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub kind: String,
    pub columns: Vec<Column>,
}

impl Node {
    /// ("main.bronze", "orders") for three-part names, ("etl", "clean") for paths.
    pub fn split_name(&self) -> (String, String) {
        let n = self.name.trim_end_matches('/');
        if let Some(i) = n.rfind('/') {
            return (n[..i].to_string(), n[i + 1..].to_string());
        }
        if ["notebook", "job", "pipeline", "dashboard", "query", "model", "external", "source"].contains(&self.kind.as_str()) {
            return (String::new(), n.to_string());
        }
        match n.rfind('.') {
            Some(i) => (n[..i].to_string(), n[i + 1..].to_string()),
            None => (String::new(), n.to_string()),
        }
    }

    pub fn is_data(&self) -> bool {
        matches!(self.kind.as_str(), "table" | "view" | "mv" | "streaming" | "volume" | "file")
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColEdge {
    pub from: (usize, usize),
    pub to: (usize, usize),
}

#[derive(Clone, Debug, Default)]
pub struct Lineage {
    pub title: String,
    pub focus: Option<usize>,
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize)>,
    pub col_edges: Vec<ColEdge>,
    pub errors: Vec<String>,
}

impl Lineage {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    fn node(&mut self, name: &str, kind: Option<&str>) -> usize {
        if let Some(i) = self.index(name) {
            if let Some(k) = kind {
                self.nodes[i].kind = k.to_string();
            }
            return i;
        }
        self.nodes.push(Node { name: name.to_string(), kind: kind.unwrap_or("table").to_string(), columns: vec![] });
        self.nodes.len() - 1
    }

    fn add_edge(&mut self, a: usize, b: usize) {
        if a != b && !self.edges.contains(&(a, b)) {
            self.edges.push((a, b));
        }
    }

    /// Everything upstream (sources) and downstream (consumers) of `i`.
    pub fn related(&self, i: usize) -> (HashSet<usize>, HashSet<usize>) {
        let walk = |forward: bool| {
            let mut seen = HashSet::new();
            let mut stack = vec![i];
            while let Some(x) = stack.pop() {
                for &(a, b) in &self.edges {
                    let (from, to) = if forward { (a, b) } else { (b, a) };
                    if from == x && seen.insert(to) {
                        stack.push(to);
                    }
                }
            }
            seen
        };
        (walk(false), walk(true))
    }

    /// Column-level closure: every column feeding or fed by (node, col).
    pub fn column_related(&self, at: (usize, usize)) -> HashSet<(usize, usize)> {
        let mut seen = HashSet::from([at]);
        for forward in [true, false] {
            let mut stack = vec![at];
            while let Some(x) = stack.pop() {
                for e in &self.col_edges {
                    let (from, to) = if forward { (e.from, e.to) } else { (e.to, e.from) };
                    if from == x && seen.insert(to) {
                        stack.push(to);
                    }
                }
            }
        }
        seen
    }
}

fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
    }
    line
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

fn column(spec: &str) -> Column {
    match spec.split_once(':') {
        Some((n, t)) => Column { name: n.trim().into(), ty: t.trim().into() },
        None => Column { name: spec.trim().into(), ty: String::new() },
    }
}

pub fn parse(text: &str) -> Lineage {
    let mut l = Lineage::default();
    let mut focus = None;
    let mut col_lines: Vec<(usize, String, Vec<String>)> = vec![];
    for (no, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(t) = line.strip_prefix("title:") {
            l.title = t.trim().to_string();
            continue;
        }
        if let Some(f) = line.strip_prefix("focus:") {
            focus = Some(f.trim().to_string());
            continue;
        }
        if let Some((target, sources)) = line.split_once(" < ").or_else(|| line.split_once("<-")) {
            col_lines.push((no + 1, target.trim().to_string(), split_list(sources)));
            continue;
        }
        if line.contains('>') {
            let steps: Vec<Vec<String>> = line.split("->").flat_map(|p| p.split('>')).map(split_list).collect();
            if steps.iter().any(|s| s.is_empty()) {
                l.errors.push(format!("line {}: empty step in {line}", no + 1));
                continue;
            }
            let ids: Vec<Vec<usize>> = steps.iter().map(|s| s.iter().map(|n| l.node(n, None)).collect()).collect();
            for w in ids.windows(2) {
                for &a in &w[0] {
                    for &b in &w[1] {
                        l.add_edge(a, b);
                    }
                }
            }
            continue;
        }
        // declaration: name [kind] [columns]
        let mut it = line.splitn(2, char::is_whitespace);
        let name = it.next().unwrap_or("").to_string();
        let rest = it.next().unwrap_or("").trim();
        let (kind, cols) = match rest.split_once(char::is_whitespace) {
            Some((k, c)) if KINDS.contains(&k) => (Some(k), c.trim()),
            _ if KINDS.contains(&rest) => (Some(rest), ""),
            _ => (None, rest),
        };
        let i = l.node(&name, kind);
        for c in split_list(cols) {
            let c = column(&c);
            if l.nodes[i].column(&c.name).is_none() {
                l.nodes[i].columns.push(c);
            }
        }
    }

    // Column references: the longest known node name followed by ".col"; else all but the last part.
    let resolve = |l: &mut Lineage, r: &str| -> Option<(usize, usize)> {
        let mut best: Option<usize> = None;
        for (i, n) in l.nodes.iter().enumerate() {
            if r.len() > n.name.len() + 1 && r.starts_with(&n.name) && r.as_bytes()[n.name.len()] == b'.' {
                if best.is_none_or(|b| l.nodes[b].name.len() < n.name.len()) {
                    best = Some(i);
                }
            }
        }
        let (ni, col) = match best {
            Some(i) => (i, r[l.nodes[i].name.len() + 1..].to_string()),
            None => {
                let (t, c) = r.rsplit_once('.')?;
                (l.node(t, None), c.to_string())
            }
        };
        let ci = match l.nodes[ni].column(&col) {
            Some(c) => c,
            None => {
                l.nodes[ni].columns.push(Column { name: col, ty: String::new() });
                l.nodes[ni].columns.len() - 1
            }
        };
        Some((ni, ci))
    };
    for (no, target, sources) in col_lines {
        let Some(to) = resolve(&mut l, &target) else {
            l.errors.push(format!("line {no}: expected table.column, got {target}"));
            continue;
        };
        for s in sources {
            let Some(from) = resolve(&mut l, &s) else {
                l.errors.push(format!("line {no}: expected table.column, got {s}"));
                continue;
            };
            let ce = ColEdge { from, to };
            if !l.col_edges.contains(&ce) {
                l.col_edges.push(ce);
            }
            // Tables linked by columns are linked, unless already through something in between.
            let (up, _) = l.related(to.0);
            if from.0 != to.0 && !up.contains(&from.0) {
                l.add_edge(from.0, to.0);
            }
        }
    }
    l.focus = focus.and_then(|f| l.index(&f));
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    const EX: &str = "title: Orders\nfocus: main.silver.orders\n\
        main.bronze.orders table id, amount:decimal  # raw\n\
        etl/clean notebook\n\
        main.bronze.orders, main.bronze.fx > etl/clean > main.silver.orders -> dash\n\
        dash dashboard\n\
        main.silver.orders.amount_usd < main.bronze.orders.amount, main.bronze.fx.rate\n";

    #[test]
    fn parses_example() {
        let l = parse(EX);
        assert_eq!(l.title, "Orders");
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        let silver = l.index("main.silver.orders").unwrap();
        assert_eq!(l.focus, Some(silver));
        assert_eq!(l.nodes[l.index("etl/clean").unwrap()].kind, "notebook");
        assert_eq!(l.nodes[l.index("dash").unwrap()].kind, "dashboard");
        assert_eq!(l.nodes[l.index("main.bronze.orders").unwrap()].columns[1].ty, "decimal");
        assert_eq!(l.edges.len(), 4); // no shortcut edge: bronze reaches silver through etl/clean
        assert_eq!(l.col_edges.len(), 2);
        let fx = l.index("main.bronze.fx").unwrap();
        assert_eq!(l.nodes[fx].columns[0].name, "rate");
        let (up, down) = l.related(silver);
        assert!(up.contains(&fx) && down.contains(&l.index("dash").unwrap()));
        assert_eq!(l.nodes[silver].split_name(), ("main.silver".into(), "orders".into()));
    }
}
