//! Mermaid flowchart subset → graph model (nodes, groups, edges, styles, links).
//!
//! Supported: `flowchart|graph LR|RL|TB|TD|BT`, node shapes `[] () ([]) [[]] [()] (()) {} {{}}
//! [//] >]`, chains `a --> b --> c`, `a & b --> c`, edge kinds `--> --- -.-> ==> ~~~`, labels
//! `-->|text|` and `-- text -->`, `subgraph id [title] … end`, `classDef`, `class`, `:::cls`,
//! `style`, `click id "path[:line]|url" "tip"`, and `%% collapsed: a, b` (or `*`).

use super::layout::Dir;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Round,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    Diamond,
    Hexagon,
    Parallelogram,
    Flag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    Solid,
    Dotted,
    Thick,
    Invisible,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub color: Option<String>,
    pub stroke_width: Option<f64>,
    pub dashed: bool,
}

impl Style {
    pub fn parse(spec: &str) -> Style {
        let mut s = Style::default();
        for part in spec.split(',') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let (k, v) = (k.trim(), v.trim().trim_end_matches(';').to_string());
            if v.is_empty() {
                continue;
            }
            match k {
                "fill" => s.fill = Some(v),
                "stroke" => s.stroke = Some(v),
                "color" => s.color = Some(v),
                "stroke-width" => s.stroke_width = v.trim_end_matches("px").parse().ok(),
                "stroke-dasharray" => s.dashed = true,
                _ => {}
            }
        }
        s
    }

    fn merge(&mut self, o: &Style) {
        if o.fill.is_some() {
            self.fill = o.fill.clone();
        }
        if o.stroke.is_some() {
            self.stroke = o.stroke.clone();
        }
        if o.color.is_some() {
            self.color = o.color.clone();
        }
        if o.stroke_width.is_some() {
            self.stroke_width = o.stroke_width;
        }
        self.dashed |= o.dashed;
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub shape: Shape,
    pub parent: Option<usize>,
    pub group: bool,
    pub classes: Vec<String>,
    pub style: Option<Style>,
    pub link: Option<String>,
    pub tip: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    pub arrow_start: bool,
    pub arrow_end: bool,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Flow {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub class_defs: HashMap<String, Style>,
    /// Group ids to start closed; "*" closes all.
    pub collapsed: Vec<String>,
}

impl Flow {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The node's own style on top of its classes.
    pub fn style_of(&self, i: usize) -> Style {
        let n = &self.nodes[i];
        let mut s = Style::default();
        for c in &n.classes {
            if let Some(d) = self.class_defs.get(c) {
                s.merge(d);
            }
        }
        if let Some(st) = &n.style {
            s.merge(st);
        }
        s
    }

    pub fn starts_collapsed(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        n.group && self.collapsed.iter().any(|c| c == "*" || c == &n.id)
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// Another Mermaid diagram type (sequence, class, …): shown as source.
    Unsupported(String),
    Syntax(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Unsupported(k) => write!(f, "{k} diagrams are shown as source"),
            Error::Syntax(m) => write!(f, "{m}"),
        }
    }
}

pub fn clean_label(raw: &str) -> String {
    let mut t = raw.trim();
    for q in ['"', '`'] {
        if t.len() > 1 && t.starts_with(q) && t.ends_with(q) {
            t = &t[1..t.len() - 1];
        }
    }
    let mut s = t.replace("#quot;", "\"");
    for br in ["<br/>", "<br />", "<br>", "<BR>", "<BR/>"] {
        s = s.replace(br, "\n");
    }
    s.replace("**", "").trim().to_string()
}

/// Split on ';' outside quotes and |labels|.
fn statements(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q: Option<char> = None;
    for ch in line.chars() {
        if let Some(c) = q {
            if ch == c {
                q = None;
            }
            cur.push(ch);
            continue;
        }
        if ch == '"' || ch == '|' {
            q = Some(ch);
            cur.push(ch);
            continue;
        }
        if ch == ';' {
            out.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(ch);
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

const SHAPES: &[(&str, &[&str], Shape)] = &[
    ("([", &["])"], Shape::Stadium),
    ("[[", &["]]"], Shape::Subroutine),
    ("[(", &[")]"], Shape::Cylinder),
    ("((", &["))"], Shape::Circle),
    ("{{", &["}}"], Shape::Hexagon),
    ("[/", &["/]", "\\]"], Shape::Parallelogram),
    ("[\\", &["/]", "\\]"], Shape::Parallelogram),
    (">", &["]"], Shape::Flag),
    ("[", &["]"], Shape::Rect),
    ("(", &[")"], Shape::Round),
    ("{", &["}"], Shape::Diamond),
];

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

struct Builder {
    flow: Flow,
    frames: Vec<(Option<usize>, Vec<usize>)>,
    claimed: Vec<bool>,
}

impl Builder {
    fn node(&mut self, id: &str) -> usize {
        if let Some(i) = self.flow.index(id) {
            return i;
        }
        self.flow.nodes.push(Node {
            id: id.into(),
            label: id.into(),
            shape: Shape::Rect,
            parent: None,
            group: false,
            classes: vec![],
            style: None,
            link: None,
            tip: None,
        });
        self.claimed.push(false);
        self.flow.nodes.len() - 1
    }

    fn mention(&mut self, i: usize) {
        self.frames.last_mut().unwrap().1.push(i);
    }

    fn is_ancestor(&self, a: usize, b: usize) -> bool {
        let mut p = self.flow.nodes[b].parent;
        while let Some(x) = p {
            if x == a {
                return true;
            }
            p = self.flow.nodes[x].parent;
        }
        false
    }
}

struct Cursor<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }
    fn ws(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn id(&mut self) -> Option<&'a str> {
        let r = self.rest();
        let mut end = 0;
        let cs: Vec<(usize, char)> = r.char_indices().collect();
        let mut k = 0;
        while k < cs.len() {
            let (i, c) = cs[k];
            if is_id_char(c) {
                end = i + c.len_utf8();
            } else if c == '-' && end > 0 && cs.get(k + 1).is_some_and(|&(_, n)| is_id_char(n)) {
                end = i + 1;
            } else {
                break;
            }
            k += 1;
        }
        if end == 0 {
            return None;
        }
        self.pos += end;
        Some(&r[..end])
    }
}

fn read_node(c: &mut Cursor, b: &mut Builder) -> Result<usize, String> {
    c.ws();
    let id = c.id().ok_or("expected a node id")?.to_string();
    let i = b.node(&id);
    b.mention(i);
    for (open, closes, shape) in SHAPES {
        if !c.rest().starts_with(open) {
            continue;
        }
        let start = c.pos + open.len();
        let body = &c.s[start..];
        let trimmed_off = body.len() - body.trim_start_matches(' ').len();
        let (label, after) = if body[trimmed_off..].starts_with('"') {
            let q0 = start + trimmed_off;
            let q = c.s[q0 + 1..].find('"').ok_or("unterminated quote")? + q0 + 1;
            let mut p = q + 1;
            while c.s[p..].starts_with(' ') {
                p += 1;
            }
            let close = closes.iter().find(|cl| c.s[p..].starts_with(*cl)).ok_or(format!("expected {open} to close"))?;
            (c.s[q0..=q].to_string(), p + close.len())
        } else {
            let mut found = None;
            for (off, _) in body.char_indices() {
                if let Some(cl) = closes.iter().find(|cl| body[off..].starts_with(*cl)) {
                    found = Some((off, cl.len()));
                    break;
                }
            }
            let (off, len) = found.ok_or(format!("expected {open} to close"))?;
            (body[..off].to_string(), start + off + len)
        };
        c.pos = after;
        b.flow.nodes[i].label = clean_label(&label);
        if !b.flow.nodes[i].group {
            b.flow.nodes[i].shape = *shape;
        }
        break;
    }
    if c.rest().starts_with(":::") {
        c.pos += 3;
        let cls = c.id().ok_or("expected a class name after :::")?;
        b.flow.nodes[i].classes.push(cls.to_string());
    }
    Ok(i)
}

fn read_group(c: &mut Cursor, b: &mut Builder) -> Result<Vec<usize>, String> {
    let mut ids = vec![read_node(c, b)?];
    loop {
        c.ws();
        if c.peek() != Some('&') {
            return Ok(ids);
        }
        c.pos += 1;
        ids.push(read_node(c, b)?);
    }
}

struct EdgeOp {
    kind: EdgeKind,
    arrow_start: bool,
    arrow_end: bool,
    label: String,
}

fn run(s: &str, ch: char) -> usize {
    s.chars().take_while(|&c| c == ch).count()
}

fn arrow_head(c: &mut Cursor) -> bool {
    let r = c.rest();
    if r.starts_with('>') {
        c.pos += 1;
        return true;
    }
    for h in ['o', 'x'] {
        if r.starts_with(h) && r[1..].starts_with(char::is_whitespace) {
            c.pos += 1;
            return true;
        }
    }
    false
}

fn read_edge(c: &mut Cursor) -> Result<EdgeOp, String> {
    c.ws();
    let arrow_start = c.peek() == Some('<');
    if arrow_start {
        c.pos += 1;
    }
    let r = c.rest();
    // Inline label: `-- text -->`, `== text ==>`, `-. text .->`
    for open in ["--", "==", "-."] {
        if r.starts_with(open) && r[2..].starts_with(char::is_whitespace) {
            let body = &r[2..];
            let mut hit = None;
            for (off, _) in body.char_indices() {
                if off == 0 || !body[..off].ends_with(char::is_whitespace) {
                    continue;
                }
                let t = &body[off..];
                if t.starts_with("--") || t.starts_with("==") || t.starts_with(".-") {
                    hit = Some(off);
                    break;
                }
            }
            if let Some(off) = hit {
                let label = body[..off].trim();
                if !label.is_empty() && !label.contains('|') {
                    let t = &body[off..];
                    let n = t.chars().take_while(|&ch| ch == '-' || ch == '=' || ch == '.').count();
                    c.pos += 2 + off + n;
                    let arrow_end = arrow_head(c);
                    let kind = if open == "-." { EdgeKind::Dotted } else if open == "==" { EdgeKind::Thick } else { EdgeKind::Solid };
                    return Ok(EdgeOp { kind, arrow_start, arrow_end, label: clean_label(label) });
                }
            }
        }
    }
    let kind;
    let len;
    if r.starts_with("-.") {
        let dots = run(&r[1..], '.');
        if !r[1 + dots..].starts_with('-') {
            return Err("expected an edge like -->".into());
        }
        kind = EdgeKind::Dotted;
        len = 2 + dots;
    } else if run(r, '-') >= 2 {
        kind = EdgeKind::Solid;
        len = run(r, '-');
    } else if run(r, '=') >= 2 {
        kind = EdgeKind::Thick;
        len = run(r, '=');
    } else if run(r, '~') >= 3 {
        kind = EdgeKind::Invisible;
        len = run(r, '~');
    } else {
        return Err("expected an edge like -->".into());
    }
    c.pos += len;
    let arrow_end = arrow_head(c);
    let save = c.pos;
    c.ws();
    let mut label = String::new();
    if c.peek() == Some('|') {
        let r = c.rest();
        match r[1..].find('|') {
            Some(e) => {
                label = clean_label(&r[1..1 + e]);
                c.pos += e + 2;
            }
            None => return Err("unterminated |label|".into()),
        }
    } else {
        c.pos = save;
    }
    Ok(EdgeOp { kind, arrow_start, arrow_end, label })
}

fn chain(st: &str, b: &mut Builder) -> Result<(), String> {
    let mut c = Cursor { s: st, pos: 0 };
    let mut groups = vec![read_group(&mut c, b)?];
    let mut ops = vec![];
    loop {
        c.ws();
        if c.pos >= st.len() {
            break;
        }
        ops.push(read_edge(&mut c)?);
        groups.push(read_group(&mut c, b)?);
    }
    for (k, op) in ops.iter().enumerate() {
        for &a in &groups[k] {
            for &t in &groups[k + 1] {
                b.flow.edges.push(Edge {
                    from: a,
                    to: t,
                    kind: op.kind,
                    arrow_start: op.arrow_start,
                    arrow_end: op.arrow_end,
                    label: op.label.clone(),
                });
            }
        }
    }
    Ok(())
}

fn args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut it = s.trim();
    while !it.is_empty() {
        if let Some(r) = it.strip_prefix('"') {
            let e = r.find('"').unwrap_or(r.len());
            out.push(r[..e].to_string());
            it = r.get(e + 1..).unwrap_or("").trim_start();
        } else {
            let e = it.find(char::is_whitespace).unwrap_or(it.len());
            out.push(it[..e].to_string());
            it = it[e..].trim_start();
        }
    }
    out
}

pub fn parse(text: &str) -> Result<Flow, Error> {
    let text = text.replace('\r', "");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    if lines.first().map(|l| l.trim()) == Some("---") {
        i = 1;
        while i < lines.len() && lines[i].trim() != "---" {
            i += 1;
        }
        i += 1;
    }
    let mut b = Builder {
        flow: Flow { dir: Dir::TB, nodes: vec![], edges: vec![], class_defs: HashMap::new(), collapsed: vec![] },
        frames: vec![(None, vec![])],
        claimed: vec![],
    };
    let mut header = false;
    while i < lines.len() {
        let raw = lines[i].trim();
        let lineno = i + 1;
        i += 1;
        if raw.is_empty() {
            continue;
        }
        if let Some(c) = raw.strip_prefix("%%") {
            let c = c.trim();
            if let Some(rest) = c.strip_prefix("collapsed").map(|r| r.trim_start()).and_then(|r| r.strip_prefix(':')) {
                b.flow.collapsed.extend(rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            }
            continue;
        }
        if !header {
            let mut parts = raw.trim_end_matches(';').split_whitespace();
            let kw = parts.next().unwrap_or("");
            if kw != "flowchart" && kw != "graph" {
                return Err(Error::Unsupported(kw.to_string()));
            }
            b.flow.dir = match parts.next().map(|d| d.to_ascii_uppercase()) {
                Some(d) if d == "TD" => Dir::TB,
                Some(d) => Dir::parse(&d),
                None => Dir::TB,
            };
            header = true;
            continue;
        }
        for st in statements(raw) {
            let fail = |m: &str| Error::Syntax(format!("line {lineno}: {m}: {st}"));
            if let Some(rest) = st.strip_prefix("subgraph ") {
                let rest = rest.trim();
                let (id, title) = match rest.find('[') {
                    Some(o) if rest.ends_with(']') && !rest[..o].trim().contains(' ') => {
                        (rest[..o].trim().to_string(), clean_label(&rest[o + 1..rest.len() - 1]))
                    }
                    _ => (clean_label(rest), clean_label(rest)),
                };
                let g = b.node(&id);
                b.flow.nodes[g].group = true;
                b.flow.nodes[g].label = title;
                b.mention(g);
                b.frames.push((Some(g), vec![]));
                continue;
            }
            if st == "end" {
                if b.frames.len() < 2 {
                    return Err(fail("end without subgraph"));
                }
                let (g, mentions) = b.frames.pop().unwrap();
                let g = g.unwrap();
                for m in mentions {
                    if m == g || b.claimed[m] || b.is_ancestor(m, g) {
                        continue;
                    }
                    b.flow.nodes[m].parent = Some(g);
                    b.claimed[m] = true;
                }
                continue;
            }
            let first = st.split_whitespace().next().unwrap_or("");
            if ["direction", "linkStyle", "accTitle", "accDescr", "accTitle:", "accDescr:"].contains(&first) {
                continue;
            }
            if first == "classDef" {
                let mut p = st.splitn(3, char::is_whitespace);
                p.next();
                if let (Some(names), Some(spec)) = (p.next(), p.next()) {
                    let style = Style::parse(spec);
                    for n in names.split(',') {
                        b.flow.class_defs.insert(n.to_string(), style.clone());
                    }
                }
                continue;
            }
            if first == "class" {
                let p: Vec<&str> = st.split_whitespace().collect();
                if p.len() >= 3 {
                    for id in p[1].split(',') {
                        let n = b.node(id);
                        b.flow.nodes[n].classes.push(p[2].to_string());
                    }
                }
                continue;
            }
            if first == "style" {
                let mut p = st.splitn(3, char::is_whitespace);
                p.next();
                if let (Some(id), Some(spec)) = (p.next(), p.next()) {
                    let n = b.node(id);
                    b.flow.nodes[n].style = Some(Style::parse(spec));
                }
                continue;
            }
            if first == "click" {
                let mut p = st.splitn(3, char::is_whitespace);
                p.next();
                if let (Some(id), Some(rest)) = (p.next(), p.next()) {
                    let n = b.node(id);
                    let mut a = args(rest);
                    if a.first().map(|s| s.as_str()) == Some("href") {
                        a.remove(0);
                    }
                    if matches!(a.first().map(|s| s.as_str()), Some("call" | "callback")) {
                        continue;
                    }
                    let node = &mut b.flow.nodes[n];
                    let is_cb = a.first().is_some_and(|x| x.chars().all(is_id_char) && !x.starts_with("http"));
                    if is_cb {
                        node.tip = a.get(1).cloned();
                    } else {
                        node.link = a.first().cloned();
                        if let Some(t) = a.get(1).filter(|t| !t.starts_with('_')) {
                            node.tip = Some(t.clone());
                        }
                    }
                }
                continue;
            }
            chain(&st, &mut b).map_err(|m| fail(&m))?;
        }
    }
    if !header {
        return Err(Error::Unsupported("empty".into()));
    }
    if b.frames.len() > 1 {
        let g = b.frames.last().unwrap().0.unwrap();
        return Err(Error::Syntax(format!("subgraph {} is missing its end", b.flow.nodes[g].id)));
    }
    Ok(b.flow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canvas_example() {
        let f = parse(
            r#"flowchart LR
  %% collapsed: staging
  raw[(event_flattened)] --> stg_events
  subgraph staging [Staging]
    stg_events --> stg_sessions
  end
  subgraph marts [marts]
    fct_retention
  end
  stg_sessions -->|daily| fct_retention
  a & b -.-> c
  x -- uses --> y
  click stg_events "models/staging/stg_events.sql" "stg_events"
  classDef hot fill:#f96,stroke:#333
  class fct_retention hot
"#,
        )
        .unwrap();
        assert_eq!(f.dir, Dir::LR);
        let raw = f.index("raw").unwrap();
        assert_eq!(f.nodes[raw].shape, Shape::Cylinder);
        assert_eq!(f.nodes[raw].label, "event_flattened");
        let st = f.index("staging").unwrap();
        assert!(f.nodes[st].group);
        assert_eq!(f.nodes[st].label, "Staging");
        assert_eq!(f.nodes[f.index("stg_sessions").unwrap()].parent, Some(st));
        assert_eq!(f.nodes[f.index("raw").unwrap()].parent, None);
        assert!(f.starts_collapsed(st));
        assert_eq!(f.edges.iter().filter(|e| e.label == "daily").count(), 1);
        assert_eq!(f.edges.iter().filter(|e| e.kind == EdgeKind::Dotted).count(), 2);
        assert_eq!(f.edges.iter().find(|e| e.label == "uses").map(|e| e.arrow_end), Some(true));
        assert_eq!(f.nodes[f.index("stg_events").unwrap()].link.as_deref(), Some("models/staging/stg_events.sql"));
        assert_eq!(f.style_of(f.index("fct_retention").unwrap()).fill.as_deref(), Some("#f96"));
    }

    #[test]
    fn other_types_are_unsupported() {
        assert_eq!(parse("sequenceDiagram\n A->>B: hi").unwrap_err(), Error::Unsupported("sequenceDiagram".into()));
        assert!(matches!(parse("graph TD\n a[unclosed --> b"), Err(Error::Syntax(_))));
    }

    #[test]
    fn hyphenated_ids_and_arrows() {
        let f = parse("graph TD\nmy-node-->other\na---b\nc==>d\ne~~~f").unwrap();
        assert!(f.index("my-node").is_some());
        assert_eq!(f.edges.len(), 4);
        assert!(!f.edges[1].arrow_end);
        assert_eq!(f.edges[2].kind, EdgeKind::Thick);
        assert_eq!(f.edges[3].kind, EdgeKind::Invisible);
    }
}
