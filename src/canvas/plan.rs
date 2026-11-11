//! `.plan` — a work breakdown drafted before it exists in Jira:
//!
//! ```text
//! # [DPL] Lakehouse MVP                  the epic (or `epic: RAID-1` to use an existing one)
//! project: RAID
//! labels: lakehouse
//! ## Story: Ingest events | 5 | @me | ingestion | RAID-12
//! free lines are the description; `AC:` starts acceptance-criteria bullets
//! - Subtask: Write the DAG | 2
//! after: #1, RAID-12                     blocked by item 1 of this plan and by RAID-12
//! ```
//!
//! After `|`: a number is story points, `@me` the assignee, a KEY-1 the issue it became, anything
//! else comma-separated labels. Push creates what has no key yet and writes keys back.

use super::jira::{self, NewIssue};
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Fields {
    pub summary: String,
    pub points: Option<f64>,
    pub assignee: String,
    pub labels: Vec<String>,
    pub key: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Sub {
    pub line: usize,
    #[serde(flatten)]
    pub f: Fields,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Item {
    pub line: usize,
    pub n: usize,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub f: Fields,
    pub description: Vec<String>,
    pub ac: Vec<String>,
    pub subtasks: Vec<Sub>,
    pub after: Vec<String>,
    pub total: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Epic {
    pub line: Option<usize>,
    pub summary: String,
    pub key: String,
    pub description: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Plan {
    pub project: String,
    pub epic: Epic,
    pub labels: Vec<String>,
    pub items: Vec<Item>,
    pub errors: Vec<String>,
    pub total: f64,
}

impl Plan {
    pub fn keys(&self) -> Vec<String> {
        let mut k = vec![self.epic.key.clone()];
        for it in &self.items {
            k.push(it.f.key.clone());
            k.extend(it.subtasks.iter().map(|s| s.f.key.clone()));
        }
        k.retain(|x| !x.is_empty());
        k
    }
}

fn fields(rest: &str) -> Fields {
    let mut parts = rest.split('|').map(str::trim);
    let mut f = Fields { summary: parts.next().unwrap_or("").to_string(), ..Default::default() };
    for p in parts.filter(|p| !p.is_empty()) {
        if let Ok(n) = p.parse::<f64>() {
            f.points = Some(n);
        } else if let Some(a) = p.strip_prefix('@') {
            f.assignee = a.to_string();
        } else if jira::valid_key(p) {
            f.key = p.to_string();
        } else {
            f.labels.extend(p.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()));
        }
    }
    f
}

pub fn parse(text: &str) -> Plan {
    let mut plan = Plan::default();
    let mut in_ac = false;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.trim_end();
        let t = line.trim();
        let has_item = !plan.items.is_empty();
        if !has_item && plan.epic.line.is_none() && line.starts_with("# ") {
            let f = fields(&line[2..]);
            plan.epic.line = Some(i);
            plan.epic.summary = f.summary;
            if !f.key.is_empty() {
                plan.epic.key = f.key;
            }
            continue;
        }
        if !has_item {
            if let Some((k, v)) = line.split_once(':') {
                match k.trim().to_lowercase().as_str() {
                    "project" => {
                        plan.project = v.trim().to_string();
                        continue;
                    }
                    "epic" => {
                        plan.epic.key = v.trim().to_string();
                        continue;
                    }
                    "labels" => {
                        plan.labels = v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
                        continue;
                    }
                    _ => {}
                }
            }
        }
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some((kind, body)) = rest.split_once(':') {
                let k = kind.trim().to_lowercase();
                if ["story", "task", "bug"].contains(&k.as_str()) {
                    let mut c = k.chars();
                    let kind = c.next().unwrap().to_uppercase().collect::<String>() + c.as_str();
                    plan.items.push(Item { line: i, n: plan.items.len() + 1, kind, f: fields(body.trim()), ..Default::default() });
                    in_ac = false;
                    continue;
                }
            }
        }
        let Some(item) = plan.items.last_mut() else {
            if !t.is_empty() {
                plan.epic.description.push(t.to_string());
            }
            continue;
        };
        let bullet = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "));
        if let Some(b) = bullet {
            if let Some(rest) = b.strip_prefix("Subtask:").or_else(|| b.strip_prefix("subtask:")) {
                item.subtasks.push(Sub { line: i, f: fields(rest.trim()) });
                continue;
            }
        }
        if let Some(rest) = t.strip_prefix("after:").or_else(|| t.strip_prefix("After:")) {
            item.after.extend(rest.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()));
            in_ac = false;
            continue;
        }
        if t.eq_ignore_ascii_case("ac:") {
            in_ac = true;
            continue;
        }
        if in_ac {
            if let Some(b) = bullet {
                item.ac.push(b.trim().to_string());
                continue;
            }
        }
        if !t.is_empty() {
            item.description.push(t.to_string());
            in_ac = false;
        }
    }
    if plan.project.is_empty() {
        plan.errors.push("missing `project: KEY`".into());
    }
    if plan.epic.line.is_none() && plan.epic.key.is_empty() {
        plan.errors.push("missing the epic: a `# Title` line or `epic: KEY`".into());
    }
    let n = plan.items.len();
    let mut errs = vec![];
    for it in &plan.items {
        if it.f.summary.is_empty() {
            errs.push(format!("line {}: empty summary", it.line + 1));
        }
        for r in &it.after {
            let ok = match r.strip_prefix('#').and_then(|x| x.parse::<usize>().ok()) {
                Some(k) => k >= 1 && k <= n && k != it.n,
                None => jira::valid_key(r),
            };
            if !ok {
                errs.push(format!("#{} \"{}\": after \"{r}\" is not another item (#n) or an issue key", it.n, it.f.summary));
            }
        }
    }
    plan.errors.extend(errs);
    for it in plan.items.iter_mut() {
        it.total = it.f.points.unwrap_or(0.0) + it.subtasks.iter().map(|s| s.f.points.unwrap_or(0.0)).sum::<f64>();
    }
    plan.total = plan.items.iter().map(|i| i.total).sum();
    plan
}

fn describe(lines: &[String], ac: &[String]) -> String {
    let mut parts = vec![];
    if !lines.is_empty() {
        parts.push(lines.join("\n"));
    }
    if !ac.is_empty() {
        parts.push(format!("Acceptance criteria:\n{}", ac.iter().map(|a| format!("- {a}")).collect::<Vec<_>>().join("\n")));
    }
    parts.join("\n\n")
}

#[derive(Debug, Default, Serialize)]
pub struct PushResult {
    pub created: Vec<String>,
    pub linked: Vec<String>,
    pub epic: String,
}

/// Create, in order, the epic, each item under it, then its subtasks; every new key is written
/// into the file at once so a failure halfway keeps what was made and a re-push skips it.
pub fn push(file: &Path) -> Result<PushResult> {
    let mut lines: Vec<String> = std::fs::read_to_string(file)?.split('\n').map(String::from).collect();
    let plan = parse(&lines.join("\n"));
    if !plan.errors.is_empty() {
        bail!("{}", plan.errors.join("; "));
    }
    let mut res = PushResult::default();
    let mark = |lines: &mut Vec<String>, line: usize, key: &str| -> Result<()> {
        lines[line] = format!("{} | {key}", lines[line].trim_end());
        std::fs::write(file, lines.join("\n"))?;
        Ok(())
    };
    let wants_me = plan.items.iter().any(|i| i.f.assignee == "me" || i.subtasks.iter().any(|s| s.f.assignee == "me"));
    let me = if wants_me { Some(jira::myself()?["accountId"].as_str().unwrap_or("").to_string()) } else { None };
    let make = |kind: &str, f: &Fields, parent: Option<&str>, desc: &str, created: &mut Vec<String>| -> Result<String> {
        let mut labels = plan.labels.clone();
        labels.extend(f.labels.iter().cloned());
        let key = jira::create(&NewIssue {
            project: &plan.project,
            summary: &f.summary,
            kind,
            parent,
            description: Some(desc),
            points: f.points,
            labels,
            assignee: if f.assignee == "me" { me.clone() } else { None },
        })?;
        created.push(format!("{key}  {kind}: {}", f.summary));
        Ok(key)
    };
    let mut epic = plan.epic.key.clone();
    if epic.is_empty() {
        let f = Fields { summary: plan.epic.summary.clone(), ..Default::default() };
        epic = make("Epic", &f, None, &plan.epic.description.join("\n"), &mut res.created)?;
        mark(&mut lines, plan.epic.line.unwrap(), &epic)?;
    }
    for it in &plan.items {
        let mut key = it.f.key.clone();
        if key.is_empty() {
            key = make(&it.kind, &it.f, Some(&epic), &describe(&it.description, &it.ac), &mut res.created)?;
            mark(&mut lines, it.line, &key)?;
        }
        for st in &it.subtasks {
            if st.f.key.is_empty() {
                let k = make("Subtask", &st.f, Some(&key), "", &mut res.created)?;
                mark(&mut lines, st.line, &k)?;
            }
        }
    }
    let after = parse(&lines.join("\n"));
    for it in after.items.iter().filter(|i| !i.after.is_empty() && !i.f.key.is_empty()) {
        let have: Vec<String> = jira::issue(&it.f.key)?["links"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|l| l["type"] == "Blocks" && l["label"] == "is blocked by")
                    .filter_map(|l| l["key"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for r in &it.after {
            let dep = match r.strip_prefix('#').and_then(|x| x.parse::<usize>().ok()) {
                Some(n) => after.items[n - 1].f.key.clone(),
                None => r.clone(),
            };
            if dep.is_empty() || have.contains(&dep) {
                continue;
            }
            res.linked.push(jira::link(&dep, "blocks", &it.f.key)?);
        }
    }
    res.epic = epic;
    Ok(res)
}

/// The plan's dependencies as Mermaid: items by key once pushed, by #n before.
pub fn graph(plan: &Plan, live: &HashMap<String, Value>, base: &str) -> String {
    let id = |it: &Item| if it.f.key.is_empty() { format!("n{}", it.n) } else { it.f.key.clone() };
    let url = |k: &str| if k.is_empty() || base.is_empty() { String::new() } else { format!("{base}/browse/{k}") };
    let mut nodes: Vec<Value> = plan
        .items
        .iter()
        .map(|it| {
            let tag = if it.f.key.is_empty() { format!("#{}", it.n) } else { it.f.key.clone() };
            json!({"id": id(it), "label": format!("{tag} {}", it.f.summary), "category": live.get(&it.f.key).map(|l| l["category"].clone()).unwrap_or(Value::Null), "url": url(&it.f.key)})
        })
        .collect();
    let mut edges = vec![];
    for it in &plan.items {
        for r in &it.after {
            let from = match r.strip_prefix('#').and_then(|x| x.parse::<usize>().ok()) {
                Some(n) => match plan.items.get(n.wrapping_sub(1)) {
                    Some(d) => id(d),
                    None => continue,
                },
                None => {
                    if !nodes.iter().any(|x| x["id"] == json!(r)) {
                        nodes.push(json!({"id": r, "label": format!("{r} ↗"), "url": url(r)}));
                    }
                    r.clone()
                }
            };
            edges.push((from, id(it)));
        }
    }
    let title = if plan.epic.summary.is_empty() { plan.epic.key.clone() } else { plan.epic.summary.clone() };
    jira::mermaid_deps(&format!("dependencies of {title}"), &nodes, &edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EX: &str = "# [DPL] Lakehouse MVP\nproject: RAID\nlabels: lake\nWhy we do it.\n\n## Story: Ingest events | 5 | @me | ingestion\nRead from Kafka.\nAC:\n- lands in bronze\n- dedup by id\n- Subtask: Write the DAG | 2\n- Subtask: Alerts | 1 | RAID-9\n## Task: Model silver | 3 | RAID-12\nafter: #1, RAID-3\n## Bug: Fix | x\nafter: #9\n";

    #[test]
    fn parses_example() {
        let p = parse(EX);
        assert_eq!(p.project, "RAID");
        assert_eq!(p.epic.summary, "[DPL] Lakehouse MVP");
        assert_eq!(p.labels, vec!["lake"]);
        assert_eq!(p.epic.description, vec!["Why we do it."]);
        assert_eq!(p.items.len(), 3);
        let s = &p.items[0];
        assert_eq!((s.kind.as_str(), s.f.points, s.f.assignee.as_str()), ("Story", Some(5.0), "me"));
        assert_eq!(s.f.labels, vec!["ingestion"]);
        assert_eq!(s.ac, vec!["lands in bronze", "dedup by id"]);
        assert_eq!(s.subtasks.len(), 2);
        assert_eq!(s.subtasks[1].f.key, "RAID-9");
        assert_eq!(s.total, 8.0);
        assert_eq!(p.items[1].f.key, "RAID-12");
        assert_eq!(p.items[1].after, vec!["#1", "RAID-3"]);
        assert_eq!(p.errors.len(), 1, "{:?}", p.errors); // `after: #9`
        assert_eq!(p.total, 11.0);
        let g = graph(&p, &HashMap::new(), "https://x");
        assert!(g.contains("n1 --> RAID-12") && g.contains("RAID-3 --> RAID-12"));
    }
}
