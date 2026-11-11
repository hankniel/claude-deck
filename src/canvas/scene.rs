//! Laid-out diagrams as JSON for front ends that don't run the Rust layout themselves (the
//! macOS app): node rectangles, edge routes and the model, with text sized by an estimate.

use super::layout::{self, Dir, LNode, Opts};
use super::{dbml, flow, lineage, sequence};
use serde_json::{json, Value};

/// Width of text at the app's body size (13 px system font), estimated.
pub fn text_w(s: &str, scale: f64) -> f64 {
    s.lines().map(|l| l.chars().map(|c| if c.is_ascii_uppercase() { 8.4 } else if c.is_ascii() { 7.0 } else { 12.5 }).sum::<f64>()).fold(0.0, f64::max) * scale
}

pub fn text_h(s: &str, scale: f64) -> f64 {
    s.lines().count().max(1) as f64 * 17.0 * scale
}

fn rect(r: Option<layout::Rect>) -> Value {
    match r {
        Some(r) => json!([r.x, r.y, r.w, r.h]),
        None => Value::Null,
    }
}

fn shape_name(s: flow::Shape) -> &'static str {
    use flow::Shape::*;
    match s {
        Rect => "rect",
        Round => "round",
        Stadium => "stadium",
        Subroutine => "subroutine",
        Cylinder => "cylinder",
        Circle => "circle",
        Diamond => "diamond",
        Hexagon => "hexagon",
        Parallelogram => "parallelogram",
        Flag => "flag",
    }
}

/// `collapsed`: group ids to show closed (None = the file's `%% collapsed`).
pub fn flow_scene(f: &flow::Flow, collapsed: Option<&[String]>) -> Value {
    let closed: Vec<bool> = f
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| n.group && collapsed.map(|c| c.iter().any(|x| x == &n.id || x == "*")).unwrap_or_else(|| f.starts_collapsed(i)))
        .collect();
    let nodes: Vec<LNode> = f
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (tw, th) = (text_w(&n.label, 1.0), text_h(&n.label, 1.0));
            let (w, h) = if n.group {
                if closed[i] {
                    (tw + 90.0, 44.0)
                } else {
                    (tw + 70.0, 42.0)
                }
            } else {
                let (w, h) = (tw + 28.0, th + 18.0);
                match n.shape {
                    flow::Shape::Diamond => ((w * 1.5).max(70.0), (h * 1.6).max(50.0)),
                    flow::Shape::Circle => (w.max(h) + 6.0, w.max(h) + 6.0),
                    flow::Shape::Hexagon => (w + 24.0, h + 4.0),
                    flow::Shape::Cylinder => (w, h + 14.0),
                    flow::Shape::Parallelogram => (w + 22.0, h),
                    flow::Shape::Stadium => (w + 12.0, h),
                    _ => (w.max(64.0), h.max(36.0)),
                }
            };
            LNode { w, h, parent: n.parent, group: n.group, collapsed: closed[i] }
        })
        .collect();
    let edges: Vec<(usize, usize)> = f.edges.iter().map(|e| (e.from, e.to)).collect();
    let opts = if f.dir.horizontal() {
        Opts { node_gap: 22.0, rank_gap: 64.0, pad: 18.0, title_h: 28.0 }
    } else {
        Opts { node_gap: 30.0, rank_gap: 48.0, pad: 18.0, title_h: 28.0 }
    };
    let l = layout::layout(&nodes, &edges, f.dir, opts);
    json!({
        "kind": "flow",
        "width": l.width, "height": l.height,
        "nodes": f.nodes.iter().enumerate().map(|(i, n)| {
            let st = f.style_of(i);
            json!({
                "id": n.id, "label": n.label, "shape": shape_name(n.shape), "group": n.group, "collapsed": closed[i],
                "parent": n.parent, "link": n.link, "tip": n.tip, "rect": rect(l.rects[i]),
                "fill": st.fill, "stroke": st.stroke, "color": st.color, "dashed": st.dashed,
            })
        }).collect::<Vec<_>>(),
        "edges": f.edges.iter().enumerate().map(|(k, e)| {
            let r = &l.routes[k];
            json!({
                "from": r.as_ref().map(|r| r.from), "to": r.as_ref().map(|r| r.to),
                "points": r.as_ref().map(|r| r.points.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>()),
                "kind": format!("{:?}", e.kind).to_lowercase(), "arrowStart": e.arrow_start, "arrowEnd": e.arrow_end, "label": e.label,
            })
        }).collect::<Vec<_>>(),
    })
}

pub const LINEAGE_W: f64 = 268.0;
pub const LINEAGE_HEAD: f64 = 58.0;
pub const LINEAGE_ROW: f64 = 26.0;

/// `expanded`: node indices showing their columns (None = the focus node).
pub fn lineage_scene(l: &lineage::Lineage, expanded: Option<&[usize]>) -> Value {
    let open: Vec<bool> = (0..l.nodes.len())
        .map(|i| match expanded {
            Some(e) => e.contains(&i),
            None => l.focus == Some(i) && !l.nodes[i].columns.is_empty() && l.nodes[i].columns.len() <= 14,
        })
        .collect();
    let nodes: Vec<LNode> = l
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| LNode {
            w: LINEAGE_W,
            h: if open[i] { LINEAGE_HEAD + n.columns.len() as f64 * LINEAGE_ROW + 8.0 } else { LINEAGE_HEAD },
            parent: None,
            group: false,
            collapsed: false,
        })
        .collect();
    let lay = layout::layout(&nodes, &l.edges, Dir::LR, Opts { node_gap: 26.0, rank_gap: 96.0, pad: 0.0, title_h: 0.0 });
    json!({
        "kind": "lineage", "title": l.title, "focus": l.focus, "width": lay.width, "height": lay.height,
        "head": LINEAGE_HEAD, "row": LINEAGE_ROW,
        "nodes": l.nodes.iter().enumerate().map(|(i, n)| {
            let (ns, short) = n.split_name();
            json!({"name": n.name, "kind": n.kind, "namespace": ns, "short": short, "expanded": open[i], "rect": rect(lay.rects[i]),
                "columns": n.columns.iter().map(|c| json!({"name": c.name, "type": c.ty})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>(),
        "edges": l.edges,
        "columnEdges": l.col_edges.iter().map(|e| json!([e.from.0, e.from.1, e.to.0, e.to.1])).collect::<Vec<_>>(),
        "errors": l.errors,
    })
}

pub const ERD_HEAD: f64 = 36.0;
pub const ERD_ROW: f64 = 28.0;

/// Tables, enums and groups with an automatic layout (positions the user saved override it).
pub fn erd_scene(e: &dbml::Erd) -> Value {
    let width = |title: &str, rows: &[(String, String)]| -> f64 {
        let mut w = text_w(title, 1.0) + 36.0;
        for (a, b) in rows {
            w = w.max(text_w(a, 0.85) + text_w(b, 0.9) + 64.0);
        }
        w.clamp(190.0, 420.0)
    };
    let nt = e.tables.len();
    let ne = e.enums.len();
    let mut nodes = vec![];
    for t in &e.tables {
        let rows: Vec<(String, String)> = t.columns.iter().map(|c| (c.name.clone(), c.ty.clone())).collect();
        nodes.push(LNode { w: width(&t.name, &rows), h: ERD_HEAD + t.columns.len().max(1) as f64 * ERD_ROW + 4.0, parent: t.group.map(|g| nt + ne + g), group: false, collapsed: false });
    }
    for en in &e.enums {
        let rows: Vec<(String, String)> = en.values.iter().map(|v| (v.clone(), String::new())).collect();
        nodes.push(LNode { w: width(&format!("enum {}", en.name), &rows), h: ERD_HEAD + en.values.len().max(1) as f64 * ERD_ROW + 4.0, parent: None, group: false, collapsed: false });
    }
    for g in &e.groups {
        nodes.push(LNode { w: text_w(&g.name, 0.85) + 30.0, h: 20.0, parent: None, group: true, collapsed: false });
    }
    let mut edges = vec![];
    for r in &e.refs {
        let (a, b) = if r.from_card == dbml::Card::Many && r.to_card == dbml::Card::One { (r.to.0, r.from.0) } else { (r.from.0, r.to.0) };
        if a != b {
            edges.push((a, b));
        }
    }
    for (k, en) in e.enums.iter().enumerate() {
        if let Some(u) = e.tables.iter().position(|t| t.columns.iter().any(|c| c.ty == en.name)) {
            edges.push((nt + k, u));
        }
    }
    let lay = layout::layout(&nodes, &edges, Dir::LR, Opts { node_gap: 40.0, rank_gap: 110.0, pad: 22.0, title_h: 30.0 });
    let card = |c: dbml::Card| if c == dbml::Card::One { "one" } else { "many" };
    json!({
        "kind": "erd", "project": e.project, "head": ERD_HEAD, "row": ERD_ROW, "width": lay.width, "height": lay.height,
        "tables": e.tables.iter().enumerate().map(|(i, t)| json!({
            "name": t.name, "alias": t.alias, "note": t.note, "color": t.color, "group": t.group, "rect": rect(lay.rects[i]),
            "columns": t.columns.iter().map(|c| json!({"name": c.name, "type": c.ty, "pk": c.pk, "notNull": c.not_null, "unique": c.unique,
                "increment": c.increment, "default": c.default, "note": c.note})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "enums": e.enums.iter().enumerate().map(|(i, en)| json!({"name": en.name, "values": en.values, "rect": rect(lay.rects[nt + i])})).collect::<Vec<_>>(),
        "groups": e.groups.iter().map(|g| json!({"name": g.name, "color": g.color, "tables": g.tables})).collect::<Vec<_>>(),
        "refs": e.refs.iter().map(|r| json!({"from": [r.from.0, r.from.1], "to": [r.to.0, r.to.1], "fromCard": card(r.from_card), "toCard": card(r.to_card), "name": r.name})).collect::<Vec<_>>(),
        "errors": e.errors,
    })
}

pub fn sequence_scene(s: &sequence::Sequence) -> Value {
    let head = |h: sequence::Head| match h {
        sequence::Head::Arrow => "arrow",
        sequence::Head::Open => "open",
        sequence::Head::Cross => "cross",
        sequence::Head::None => "none",
    };
    json!({
        "kind": "sequence", "title": s.title,
        "participants": s.participants.iter().map(|p| json!({"id": p.id, "label": p.label, "actor": p.actor})).collect::<Vec<_>>(),
        "events": s.events.iter().map(|e| match e {
            sequence::Event::Message { from, to, text, dashed, head: h, number } =>
                json!({"type": "message", "from": from, "to": to, "text": text, "dashed": dashed, "head": head(*h), "number": number}),
            sequence::Event::Note { side, over, text } =>
                json!({"type": "note", "side": format!("{side:?}").to_lowercase(), "over": over, "text": text}),
            sequence::Event::Start { kind, label } => json!({"type": "start", "kind": kind, "label": label}),
            sequence::Event::Else { label } => json!({"type": "else", "label": label}),
            sequence::Event::End => json!({"type": "end"}),
        }).collect::<Vec<_>>(),
        "errors": s.errors,
    })
}
