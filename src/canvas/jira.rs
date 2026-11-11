//! Jira Cloud client for the canvas: REST v3 + Agile v1 with an API token (Basic auth, the
//! user's own permissions). Configured from JIRA_BASE_URL, JIRA_EMAIL, JIRA_API_TOKEN in the
//! environment, or else from `<canvas home>/jira.env` (`KEY=value` lines, read on every call).
//!
//! Results are JSON values in the same compact shapes the app and `claude-deck canvas jira … --json`
//! use, so the desktop apps and agents read one format.

use super::store;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ISSUE_FIELDS: &[&str] = &["summary", "status", "assignee", "priority", "issuetype", "parent", "updated", "project"];
const OPEN: &str = "statusCategory != Done";
const CACHE: Duration = Duration::from_secs(30);
const STATIC_CACHE: Duration = Duration::from_secs(3600);

pub struct Config {
    pub base: String,
    auth: String,
}

pub fn env_file() -> std::path::PathBuf {
    store::home().join("jira.env")
}

fn file_env() -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in std::fs::read_to_string(env_file()).unwrap_or_default().lines() {
        let l = line.trim().trim_start_matches("export ").trim();
        if let Some((k, v)) = l.split_once('=') {
            if k.starts_with("JIRA_") {
                let v = v.trim();
                let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))).unwrap_or(v);
                out.insert(k.trim().to_string(), v.to_string());
            }
        }
    }
    out
}

pub fn config() -> Option<Config> {
    let env: HashMap<String, String> = if std::env::var("JIRA_API_TOKEN").is_ok() {
        ["JIRA_BASE_URL", "JIRA_EMAIL", "JIRA_API_TOKEN"]
            .iter()
            .filter_map(|k| Some((k.to_string(), std::env::var(k).ok()?)))
            .collect()
    } else {
        file_env()
    };
    let base = env.get("JIRA_BASE_URL")?.trim_end_matches('/').to_string();
    let email = env.get("JIRA_EMAIL")?;
    let token = env.get("JIRA_API_TOKEN")?;
    if base.is_empty() || email.is_empty() || token.is_empty() {
        return None;
    }
    let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}")));
    Some(Config { base, auth })
}

pub fn configured() -> bool {
    config().is_some()
}

pub fn base() -> String {
    config().map(|c| c.base).unwrap_or_default()
}

fn is_static(path: &str) -> bool {
    path.starts_with("/rest/api/3/myself")
        || path.starts_with("/rest/api/3/issue/createmeta")
        || (path.starts_with("/rest/agile/1.0/board") && !path.contains("/issue") && !path.contains("/sprint") && !path.contains("/backlog"))
}

static CACHE_MAP: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

pub fn request(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let cfg = config().ok_or_else(|| anyhow!("Jira is not configured: set JIRA_BASE_URL, JIRA_EMAIL and JIRA_API_TOKEN in {}", env_file().display()))?;
    let read = method == "GET" || path == "/rest/api/3/search/jql";
    let key = read.then(|| format!("{path} {}", body.map(|b| b.to_string()).unwrap_or_default()));
    {
        let mut c = CACHE_MAP.lock().unwrap();
        let map = c.get_or_insert_with(HashMap::new);
        if let Some(k) = &key {
            if let Some((at, v)) = map.get(k) {
                if at.elapsed() < if is_static(path) { STATIC_CACHE } else { CACHE } {
                    return Ok(v.clone());
                }
            }
        } else {
            map.retain(|k, _| is_static(k));
        }
    }
    let url = format!("{}{path}", cfg.base);
    for attempt in 0.. {
        let req = ureq::request(method, &url)
            .set("Authorization", &cfg.auth)
            .set("Accept", "application/json")
            .timeout(Duration::from_secs(30));
        let res = match body {
            Some(b) => req.send_json(b.clone()),
            None => req.call(),
        };
        match res {
            Ok(r) => {
                let text = r.into_string().unwrap_or_default();
                let v: Value = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) };
                if let Some(k) = key {
                    CACHE_MAP.lock().unwrap().get_or_insert_with(HashMap::new).insert(k, (Instant::now(), v.clone()));
                }
                return Ok(v);
            }
            Err(ureq::Error::Status(429, r)) if attempt < 2 => {
                let wait = r.header("retry-after").and_then(|s| s.parse().ok()).unwrap_or(2u64);
                std::thread::sleep(Duration::from_secs(wait.min(10)));
            }
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let mut why: Vec<String> = v["errorMessages"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                if let Some(o) = v["errors"].as_object() {
                    why.extend(o.values().filter_map(|x| x.as_str().map(String::from)));
                }
                bail!("Jira {code}: {}", if why.is_empty() { "request failed".into() } else { why.join("; ") });
            }
            Err(e) => bail!("Jira: {e}"),
        }
    }
    unreachable!()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

pub fn valid_key(k: &str) -> bool {
    let Some((p, n)) = k.rsplit_once('-') else { return false };
    !p.is_empty()
        && p.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
}

// ---------- Atlassian Document Format ----------

pub fn adf_to_text(node: &Value) -> String {
    match node["type"].as_str().unwrap_or("") {
        "text" => node["text"].as_str().unwrap_or("").to_string(),
        "hardBreak" => "\n".into(),
        "mention" => node["attrs"]["text"].as_str().unwrap_or("@someone").to_string(),
        "inlineCard" => node["attrs"]["url"].as_str().unwrap_or("").to_string(),
        t => {
            let inner: String = node["content"].as_array().map(|a| a.iter().map(adf_to_text).collect()).unwrap_or_default();
            match t {
                "paragraph" | "heading" => format!("{inner}\n"),
                "listItem" => format!("- {}\n", inner.trim()),
                "codeBlock" => format!("```\n{inner}\n```\n"),
                _ => inner,
            }
        }
    }
}

pub fn text_to_adf(text: &str) -> Value {
    let paras: Vec<Value> = text
        .split("\n\n")
        .map(|para| {
            let mut content = vec![];
            for (i, line) in para.split('\n').enumerate() {
                if i > 0 {
                    content.push(json!({"type": "hardBreak"}));
                }
                if !line.is_empty() {
                    content.push(json!({"type": "text", "text": line}));
                }
            }
            json!({"type": "paragraph", "content": content})
        })
        .collect();
    json!({"type": "doc", "version": 1, "content": paras})
}

// ---------- compact shapes ----------

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

pub fn brief(i: &Value) -> Value {
    let f = &i["fields"];
    json!({
        "key": s(&i["key"]),
        "summary": s(&f["summary"]),
        "status": s(&f["status"]["name"]),
        "category": s(&f["status"]["statusCategory"]["key"]),
        "type": s(&f["issuetype"]["name"]),
        "priority": s(&f["priority"]["name"]),
        "assignee": s(&f["assignee"]["displayName"]),
        "parent": s(&f["parent"]["key"]),
        "project": s(&f["project"]["key"]),
        "updated": s(&f["updated"]),
    })
}

pub fn myself() -> Result<Value> {
    let me = request("GET", "/rest/api/3/myself", None)?;
    Ok(json!({"accountId": s(&me["accountId"]), "name": s(&me["displayName"]), "email": s(&me["emailAddress"])}))
}

pub fn search(jql: &str, max: usize) -> Result<Vec<Value>> {
    let d = request("POST", "/rest/api/3/search/jql", Some(&json!({"jql": jql, "fields": ISSUE_FIELDS, "maxResults": max})))?;
    Ok(d["issues"].as_array().map(|a| a.iter().map(brief).collect()).unwrap_or_default())
}

pub fn mine() -> Result<Vec<Value>> {
    search(&format!("assignee = currentUser() AND {OPEN} ORDER BY updated DESC"), 50)
}

/// Every page of a search, up to a cap (search/jql pages by token).
pub fn search_all(jql: &str, fields: &[&str], cap: usize) -> Result<Vec<Value>> {
    let mut out = vec![];
    let mut token: Option<String> = None;
    loop {
        let mut body = json!({"jql": jql, "fields": fields, "maxResults": 100});
        if let Some(t) = &token {
            body["nextPageToken"] = json!(t);
        }
        let d = request("POST", "/rest/api/3/search/jql", Some(&body))?;
        out.extend(d["issues"].as_array().cloned().unwrap_or_default());
        token = d["nextPageToken"].as_str().map(String::from).filter(|_| d["isLast"] != json!(true));
        if token.is_none() || out.len() >= cap {
            return Ok(out);
        }
    }
}

fn agile_all(path: &str, cap: usize) -> Result<Vec<Value>> {
    let mut out = vec![];
    let sep = if path.contains('?') { '&' } else { '?' };
    let mut start = 0;
    while out.len() < cap {
        let d = request("GET", &format!("{path}{sep}startAt={start}&maxResults=50"), None)?;
        let page = d["issues"].as_array().or(d["values"].as_array()).cloned().unwrap_or_default();
        let n = page.len();
        out.extend(page);
        start += n;
        let total = d["total"].as_u64().unwrap_or(u64::MAX) as usize;
        if n == 0 || start >= total || d["isLast"] == json!(true) {
            break;
        }
    }
    Ok(out)
}

fn links(f: &Value) -> Vec<Value> {
    f["issuelinks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| {
                    let out = !l["outwardIssue"].is_null();
                    let o = if out { &l["outwardIssue"] } else { &l["inwardIssue"] };
                    json!({
                        "id": s(&l["id"]),
                        "type": s(&l["type"]["name"]),
                        "label": s(if out { &l["type"]["outward"] } else { &l["type"]["inward"] }),
                        "key": s(&o["key"]),
                        "summary": s(&o["fields"]["summary"]),
                        "status": s(&o["fields"]["status"]["name"]),
                        "category": s(&o["fields"]["status"]["statusCategory"]["key"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

static META: Mutex<Option<HashMap<String, Vec<Value>>>> = Mutex::new(None);

/// Issue types of a project with their story-point field and available fields.
pub fn project_meta(project: &str) -> Result<Vec<Value>> {
    if let Some(v) = META.lock().unwrap().as_ref().and_then(|m| m.get(project)) {
        return Ok(v.clone());
    }
    let d = request("GET", &format!("/rest/api/3/issue/createmeta/{}/issuetypes", enc(project)), None)?;
    let mut types = vec![];
    for t in d["issueTypes"].as_array().or(d["values"].as_array()).cloned().unwrap_or_default() {
        let id = s(&t["id"]);
        let f = request("GET", &format!("/rest/api/3/issue/createmeta/{}/issuetypes/{id}?maxResults=200", enc(project)), None)?;
        let flds = f["fields"].as_array().or(f["values"].as_array()).cloned().unwrap_or_default();
        let points = flds
            .iter()
            .find(|x| {
                let c = s(&x["schema"]["custom"]);
                (c.contains("jsw-story-points") || c.ends_with("float")) && s(&x["name"]).to_lowercase().contains("story point")
            })
            .map(|x| s(&x["fieldId"]));
        types.push(json!({
            "id": id, "name": s(&t["name"]), "subtask": t["subtask"].as_bool().unwrap_or(false),
            "points": points, "fields": flds.iter().map(|x| s(&x["fieldId"])).collect::<Vec<_>>(),
        }));
    }
    META.lock().unwrap().get_or_insert_with(HashMap::new).insert(project.to_string(), types.clone());
    Ok(types)
}

fn points_field(project: &str) -> Option<String> {
    project_meta(project).ok()?.iter().find_map(|t| t["points"].as_str().map(String::from))
}

pub fn transitions(key: &str) -> Result<Vec<Value>> {
    let tr = request("GET", &format!("/rest/api/3/issue/{}/transitions", enc(key)), None)?;
    Ok(tr["transitions"]
        .as_array()
        .map(|a| a.iter().map(|t| json!({"id": s(&t["id"]), "name": s(&t["name"]), "to": s(&t["to"]["name"]), "toId": s(&t["to"]["id"])})).collect())
        .unwrap_or_default())
}

pub fn issue(key: &str) -> Result<Value> {
    let d = request("GET", &format!("/rest/api/3/issue/{}?fields=*all", enc(key)), None)?;
    let tr = transitions(key)?;
    let f = &d["fields"];
    let ty = project_meta(&s(&f["project"]["key"])).unwrap_or_default().into_iter().find(|t| t["id"] == f["issuetype"]["id"]);
    let pf = ty.as_ref().and_then(|t| t["points"].as_str().map(String::from));
    let mut b = brief(&d);
    let o = b.as_object_mut().unwrap();
    o.insert("points".into(), pf.as_ref().map(|p| f[p].clone()).unwrap_or(Value::Null));
    o.insert("pointsField".into(), json!(pf.is_some()));
    o.insert("duedate".into(), json!(s(&f["duedate"])));
    o.insert("links".into(), json!(links(f)));
    o.insert("url".into(), json!(format!("{}/browse/{}", base(), s(&d["key"]))));
    o.insert("description".into(), json!(adf_to_text(&f["description"]).trim()));
    o.insert("labels".into(), f["labels"].clone());
    o.insert("reporter".into(), json!(s(&f["reporter"]["displayName"])));
    o.insert("created".into(), json!(s(&f["created"])));
    o.insert("subtasks".into(), json!(f["subtasks"].as_array().map(|a| a.iter().map(brief).collect::<Vec<_>>()).unwrap_or_default()));
    o.insert(
        "comments".into(),
        json!(f["comment"]["comments"]
            .as_array()
            .map(|a| a
                .iter()
                .map(|c| json!({"author": s(&c["author"]["displayName"]), "created": s(&c["created"]), "text": adf_to_text(&c["body"]).trim()}))
                .collect::<Vec<_>>())
            .unwrap_or_default()),
    );
    o.insert("transitions".into(), json!(tr));
    Ok(b)
}

/// Move to a status (or transition) by name or id, as this issue's workflow allows now.
pub fn move_to(key: &str, to: &str) -> Result<String> {
    let tr = transitions(key)?;
    let want = to.to_lowercase();
    let t = tr
        .iter()
        .find(|x| s(&x["id"]) == to || s(&x["name"]).to_lowercase() == want || s(&x["to"]).to_lowercase() == want || s(&x["toId"]) == to)
        .ok_or_else(|| anyhow!("{key} cannot move to \"{to}\"; it can go to: {}", tr.iter().map(|x| s(&x["to"])).collect::<Vec<_>>().join(", ")))?;
    request("POST", &format!("/rest/api/3/issue/{}/transitions", enc(key)), Some(&json!({"transition": {"id": s(&t["id"])}})))?;
    Ok(format!("{key} → {}", s(&t["to"])))
}

pub fn comment(key: &str, text: &str) -> Result<String> {
    if text.trim().is_empty() {
        bail!("empty comment");
    }
    request("POST", &format!("/rest/api/3/issue/{}/comment", enc(key)), Some(&json!({"body": text_to_adf(text)})))?;
    Ok(format!("commented on {key}"))
}

pub fn assign(key: &str, who: &str) -> Result<String> {
    let id = match who {
        "me" => json!(s(&myself()?["accountId"])),
        "none" | "" => Value::Null,
        x => json!(x),
    };
    request("PUT", &format!("/rest/api/3/issue/{}/assignee", enc(key)), Some(&json!({"accountId": id})))?;
    Ok(format!("{key} assigned to {who}"))
}

pub struct NewIssue<'a> {
    pub project: &'a str,
    pub summary: &'a str,
    pub kind: &'a str,
    pub parent: Option<&'a str>,
    pub description: Option<&'a str>,
    pub points: Option<f64>,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
}

pub fn create(n: &NewIssue) -> Result<String> {
    if n.project.is_empty() || n.summary.is_empty() {
        bail!("project and summary are required");
    }
    let types = project_meta(n.project)?;
    let sub = n.kind.eq_ignore_ascii_case("subtask") || n.kind.eq_ignore_ascii_case("sub-task");
    let t = types
        .iter()
        .find(|t| if sub { t["subtask"] == json!(true) } else { s(&t["name"]).eq_ignore_ascii_case(n.kind) })
        .ok_or_else(|| anyhow!("{} has no issue type \"{}\"; it has: {}", n.project, n.kind, types.iter().map(|t| s(&t["name"])).collect::<Vec<_>>().join(", ")))?;
    let has = |f: &str| t["fields"].as_array().is_some_and(|a| a.iter().any(|x| x == f));
    let mut fields = json!({"project": {"key": n.project}, "summary": n.summary, "issuetype": {"id": s(&t["id"])}});
    if let Some(p) = n.parent {
        fields["parent"] = json!({"key": p});
    }
    if let Some(d) = n.description.filter(|d| !d.is_empty()) {
        fields["description"] = text_to_adf(d);
    }
    if let (Some(p), Some(pf)) = (n.points, t["points"].as_str()) {
        fields[pf] = json!(p);
    }
    if !n.labels.is_empty() && has("labels") {
        fields["labels"] = json!(n.labels.iter().map(|l| l.split_whitespace().collect::<Vec<_>>().join("-")).collect::<Vec<_>>());
    }
    if let Some(a) = n.assignee.as_ref().filter(|_| has("assignee")) {
        fields["assignee"] = json!({"accountId": a});
    }
    let d = request("POST", "/rest/api/3/issue", Some(&json!({"fields": fields})))?;
    Ok(s(&d["key"]))
}

/// Fields: summary, description, points (null clears), labels (list or "a,b"), duedate, assignee.
pub fn edit(key: &str, changes: &Value) -> Result<String> {
    let cur = request("GET", &format!("/rest/api/3/issue/{}?fields=project,issuetype", enc(key)), None)?;
    let ty = project_meta(&s(&cur["fields"]["project"]["key"]))?.into_iter().find(|t| t["id"] == cur["fields"]["issuetype"]["id"]);
    let mut fields = serde_json::Map::new();
    let mut said = vec![];
    if let Some(v) = changes.get("summary").and_then(|v| v.as_str()) {
        fields.insert("summary".into(), json!(v.trim()));
        said.push("summary");
    }
    if let Some(v) = changes.get("description") {
        fields.insert("description".into(), match v.as_str() { Some(t) if !t.is_empty() => text_to_adf(t), _ => Value::Null });
        said.push("description");
    }
    if let Some(v) = changes.get("points") {
        let pf = ty.as_ref().and_then(|t| t["points"].as_str().map(String::from)).ok_or_else(|| anyhow!("{key} has no story-point field"))?;
        let p = match v {
            Value::Number(n) => json!(n.as_f64()),
            Value::String(t) if !t.trim().is_empty() => json!(t.trim().parse::<f64>()?),
            _ => Value::Null,
        };
        fields.insert(pf, p);
        said.push("points");
    }
    if let Some(v) = changes.get("labels") {
        let list: Vec<String> = match v {
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            Value::String(t) => t.split(',').map(String::from).collect(),
            _ => vec![],
        };
        fields.insert("labels".into(), json!(list.iter().map(|l| l.trim().replace(' ', "-")).filter(|l| !l.is_empty()).collect::<Vec<_>>()));
        said.push("labels");
    }
    if let Some(v) = changes.get("duedate") {
        fields.insert("duedate".into(), match v.as_str() { Some(d) if !d.is_empty() => json!(d), _ => Value::Null });
        said.push("due date");
    }
    if !fields.is_empty() {
        request("PUT", &format!("/rest/api/3/issue/{}", enc(key)), Some(&json!({"fields": fields})))?;
    }
    if let Some(a) = changes.get("assignee").and_then(|v| v.as_str()).filter(|a| !a.is_empty()) {
        assign(key, a)?;
        said.push("assignee");
    }
    if said.is_empty() {
        bail!("nothing to change");
    }
    Ok(format!("{key}: updated {}", said.join(", ")))
}

fn link_kind(k: &str) -> Option<&'static str> {
    Some(match k {
        "blocks" | "blocked-by" | "is-blocked-by" => "Blocks",
        "relates" | "relates-to" => "Relates",
        "duplicates" => "Duplicate",
        "clones" => "Cloners",
        _ => return None,
    })
}

/// "A blocks B" — read back and fixed if Jira stored it the other way round.
pub fn link(a: &str, kind: &str, b: &str) -> Result<String> {
    let k = kind.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    let ty = link_kind(&k).ok_or_else(|| anyhow!("unknown link \"{kind}\"; use blocks, blocked-by, relates, duplicates, clones"))?;
    let (subject, object) = if k == "blocked-by" || k == "is-blocked-by" { (b, a) } else { (a, b) };
    let make = |inward: &str, outward: &str| {
        request("POST", "/rest/api/3/issueLink", Some(&json!({"type": {"name": ty}, "inwardIssue": {"key": inward}, "outwardIssue": {"key": outward}})))
    };
    make(subject, object)?;
    let found = request("GET", &format!("/rest/api/3/issue/{}?fields=issuelinks", enc(subject)), None)?;
    let ls = found["fields"]["issuelinks"].as_array().cloned().unwrap_or_default();
    let ok = ls.iter().any(|l| s(&l["type"]["name"]) == ty && s(&l["outwardIssue"]["key"]) == object);
    if !ok && ty != "Relates" {
        if let Some(w) = ls.iter().find(|l| s(&l["type"]["name"]) == ty && s(&l["inwardIssue"]["key"]) == object) {
            request("DELETE", &format!("/rest/api/3/issueLink/{}", s(&w["id"])), None)?;
        }
        make(object, subject)?;
    }
    Ok(format!("{a} {} {b}", k.replace('-', " ")))
}

pub fn unlink(a: &str, b: &str) -> Result<String> {
    let found = request("GET", &format!("/rest/api/3/issue/{}?fields=issuelinks", enc(a)), None)?;
    let hit: Vec<String> = found["fields"]["issuelinks"]
        .as_array()
        .map(|ls| {
            ls.iter()
                .filter(|l| s(&l["outwardIssue"]["key"]) == b || s(&l["inwardIssue"]["key"]) == b)
                .map(|l| s(&l["id"]))
                .collect()
        })
        .unwrap_or_default();
    if hit.is_empty() {
        bail!("{a} has no link to {b}");
    }
    for id in &hit {
        request("DELETE", &format!("/rest/api/3/issueLink/{id}"), None)?;
    }
    Ok(format!("removed {} link{} between {a} and {b}", hit.len(), if hit.len() > 1 { "s" } else { "" }))
}

/// Dependencies as Mermaid: nodes coloured by status category, A --> B for "A blocks B".
pub fn mermaid_deps(title: &str, nodes: &[Value], edges: &[(String, String)]) -> String {
    let id = |k: &str| k.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect::<String>();
    let label = |t: &str| {
        let c: String = t.chars().map(|c| if "\"[]{}()<>|".contains(c) { ' ' } else { c }).collect();
        c.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect::<String>()
    };
    let cls = |c: &str| match c {
        "indeterminate" => "doing",
        "done" => "done",
        _ => "todo",
    };
    let mut out = vec!["flowchart LR".to_string(), format!("  %% {}", label(title))];
    for n in nodes {
        out.push(format!("  {}[\"{}\"]:::{}", id(&s(&n["id"])), label(&s(&n["label"])), cls(&s(&n["category"]))));
    }
    for (a, b) in edges {
        out.push(format!("  {} --> {}", id(a), id(b)));
    }
    for n in nodes.iter().filter(|n| !s(&n["url"]).is_empty()) {
        let tip = if s(&n["tip"]).is_empty() { s(&n["label"]) } else { s(&n["tip"]) };
        out.push(format!("  click {} \"{}\" \"{}\"", id(&s(&n["id"])), s(&n["url"]), label(&tip)));
    }
    out.push("  classDef todo fill:#3a3631,stroke:#8a847a,color:#f2efe9".into());
    out.push("  classDef doing fill:#1e3a5f,stroke:#6cb0ff,color:#e8f1fa".into());
    out.push("  classDef done fill:#1d4a2c,stroke:#5ee08f,color:#e7f6ec".into());
    out.join("\n") + "\n"
}

fn node(key: &str, summary: &str, cat: &str, outside: bool, tip: &str) -> Value {
    json!({"id": key, "label": format!("{key} {summary}{}", if outside { " ↗" } else { "" }), "category": cat,
        "url": format!("{}/browse/{key}", base()), "tip": tip})
}

/// An epic's children and the Blocks links between them (and the outside issues they touch).
pub fn epic_graph(epic: &str) -> Result<(String, usize, usize)> {
    let mut f: Vec<&str> = ISSUE_FIELDS.to_vec();
    f.push("issuelinks");
    let kids = search_all(&format!("parent = {epic} ORDER BY rank"), &f, 200)?;
    let in_epic: HashSet<String> = kids.iter().map(|i| s(&i["key"])).collect();
    let mut nodes: Vec<Value> = kids
        .iter()
        .map(|i| node(&s(&i["key"]), &s(&i["fields"]["summary"]), &s(&i["fields"]["status"]["statusCategory"]["key"]), false, &s(&i["fields"]["status"]["name"])))
        .collect();
    let mut edges = vec![];
    for i in &kids {
        let k = s(&i["key"]);
        for l in i["fields"]["issuelinks"].as_array().cloned().unwrap_or_default() {
            if s(&l["type"]["name"]) != "Blocks" {
                continue;
            }
            let (o, out) = if !l["outwardIssue"].is_null() { (&l["outwardIssue"], true) } else { (&l["inwardIssue"], false) };
            let ok = s(&o["key"]);
            if !out && in_epic.contains(&ok) {
                continue; // drawn from the other side
            }
            if !in_epic.contains(&ok) && !nodes.iter().any(|n| s(&n["id"]) == ok) {
                nodes.push(node(&ok, &s(&o["fields"]["summary"]), &s(&o["fields"]["status"]["statusCategory"]["key"]), true, ""));
            }
            edges.push(if out { (k.clone(), ok) } else { (ok, k.clone()) });
        }
    }
    edges.sort();
    edges.dedup();
    Ok((mermaid_deps(&format!("dependencies of {epic}"), &nodes, &edges), kids.len(), edges.len()))
}

/// Blocks links among any set of issues; only issues on an edge are drawn.
pub fn keys_graph(keys: &[String], title: &str) -> Result<(String, usize, usize)> {
    if keys.is_empty() {
        bail!("no issues to draw");
    }
    let set: HashSet<&String> = keys.iter().collect();
    let mut nodes: Vec<Value> = vec![];
    let mut edges: Vec<(String, String)> = vec![];
    for chunk in keys.chunks(100) {
        for x in search_all(&format!("key in ({})", chunk.join(",")), &["summary", "status", "issuelinks"], 1000)? {
            let k = s(&x["key"]);
            if !nodes.iter().any(|n| s(&n["id"]) == k) {
                nodes.push(node(&k, &s(&x["fields"]["summary"]), &s(&x["fields"]["status"]["statusCategory"]["key"]), false, ""));
            }
            for l in x["fields"]["issuelinks"].as_array().cloned().unwrap_or_default() {
                if s(&l["type"]["name"]) != "Blocks" {
                    continue;
                }
                let out = !l["outwardIssue"].is_null();
                let o = if out { &l["outwardIssue"] } else { &l["inwardIssue"] };
                let ok = s(&o["key"]);
                if !nodes.iter().any(|n| s(&n["id"]) == ok) {
                    nodes.push(node(&ok, &s(&o["fields"]["summary"]), &s(&o["fields"]["status"]["statusCategory"]["key"]), !set.contains(&ok), ""));
                }
                let e = if out { (k.clone(), ok) } else { (ok, k.clone()) };
                if !edges.contains(&e) {
                    edges.push(e);
                }
            }
        }
    }
    let linked: HashSet<String> = edges.iter().flat_map(|(a, b)| [a.clone(), b.clone()]).collect();
    if linked.is_empty() {
        bail!("no Blocks links among these {} issues", keys.len());
    }
    nodes.retain(|n| linked.contains(&s(&n["id"])));
    Ok((mermaid_deps(title, &nodes, &edges), linked.len(), edges.len()))
}

/// Live status of issues, in as few searches as possible.
pub fn statuses(keys: &[String]) -> Result<HashMap<String, Value>> {
    let mut out = HashMap::new();
    for chunk in keys.chunks(100) {
        for x in search_all(&format!("key in ({})", chunk.join(",")), &["status", "assignee"], 1000)? {
            out.insert(
                s(&x["key"]),
                json!({"status": s(&x["fields"]["status"]["name"]), "category": s(&x["fields"]["status"]["statusCategory"]["key"]), "assignee": s(&x["fields"]["assignee"]["displayName"])}),
            );
        }
    }
    Ok(out)
}

pub fn boards(project: &str) -> Result<Vec<Value>> {
    let q = if project.is_empty() { String::new() } else { format!("?projectKeyOrId={}", enc(project)) };
    let d = request("GET", &format!("/rest/agile/1.0/board{q}"), None)?;
    Ok(d["values"]
        .as_array()
        .map(|a| a.iter().map(|b| json!({"id": b["id"], "name": s(&b["name"]), "type": s(&b["type"]), "project": s(&b["location"]["projectKey"])})).collect())
        .unwrap_or_default())
}

pub fn sprints(board: u64) -> Result<Vec<Value>> {
    let d = request("GET", &format!("/rest/agile/1.0/board/{board}/sprint?state=active,future"), None).unwrap_or(json!({"values": []}));
    Ok(d["values"]
        .as_array()
        .map(|a| a.iter().map(|x| json!({"id": x["id"], "name": s(&x["name"]), "state": s(&x["state"]), "end": s(&x["endDate"])})).collect())
        .unwrap_or_default())
}

pub fn issue_sprint(key: &str) -> Option<Value> {
    let d = request("GET", &format!("/rest/agile/1.0/issue/{}?fields=sprint", enc(key)), None).ok()?;
    let sp = &d["fields"]["sprint"];
    (!sp.is_null()).then(|| json!({"id": sp["id"], "name": s(&sp["name"]), "state": s(&sp["state"])}))
}

/// Into a sprint (id) or `backlog`.
pub fn set_sprint(keys: &[String], sprint: &str) -> Result<String> {
    if keys.is_empty() {
        bail!("no issues given");
    }
    if sprint == "backlog" {
        request("POST", "/rest/agile/1.0/backlog/issue", Some(&json!({"issues": keys})))?;
        return Ok(format!("{} → backlog", keys.join(", ")));
    }
    if !sprint.chars().all(|c| c.is_ascii_digit()) {
        bail!("sprint must be an id or `backlog`");
    }
    request("POST", &format!("/rest/agile/1.0/sprint/{sprint}/issue"), Some(&json!({"issues": keys})))?;
    Ok(format!("{} → sprint {sprint}", keys.join(", ")))
}

fn is_epic(t: &Value) -> bool {
    t["hierarchyLevel"] == json!(1) || s(&t["name"]) == "Epic"
}

fn card(i: &Value, pf: Option<&str>) -> Value {
    let f = &i["fields"];
    let ls = f["issuelinks"].as_array().cloned().unwrap_or_default();
    let blocks = |inward: bool| -> Vec<String> {
        ls.iter()
            .filter(|l| s(&l["type"]["name"]) == "Blocks" && !l[if inward { "inwardIssue" } else { "outwardIssue" }].is_null())
            .map(|l| s(&l[if inward { "inwardIssue" } else { "outwardIssue" }]["key"]))
            .collect()
    };
    let mut b = brief(i);
    let o = b.as_object_mut().unwrap();
    o.insert("statusId".into(), json!(s(&f["status"]["id"])));
    o.insert("assigneeId".into(), json!(s(&f["assignee"]["accountId"])));
    o.insert("points".into(), pf.map(|p| f[p].clone()).unwrap_or(Value::Null));
    o.insert("duedate".into(), json!(s(&f["duedate"])));
    o.insert("labels".into(), f["labels"].clone());
    o.insert("sprintId".into(), f["sprint"]["id"].clone());
    o.insert("subtask".into(), json!(f["issuetype"]["subtask"].as_bool().unwrap_or(false)));
    o.insert(
        "epic".into(),
        if is_epic(&f["parent"]["fields"]["issuetype"]) { json!({"key": s(&f["parent"]["key"]), "summary": s(&f["parent"]["fields"]["summary"])}) } else { Value::Null },
    );
    o.insert(
        "blocked".into(),
        json!(ls.iter().any(|l| s(&l["type"]["name"]) == "Blocks" && !l["inwardIssue"].is_null() && s(&l["inwardIssue"]["fields"]["status"]["statusCategory"]["key"]) != "done")),
    );
    o.insert("blockedBy".into(), json!(blocks(true)));
    o.insert("blocks".into(), json!(blocks(false)));
    b
}

/// The board's own columns, filled from the active sprint (scope `sprint`), the backlog, or `all`.
pub fn board(id: u64, scope: &str) -> Result<Value> {
    let conf = request("GET", &format!("/rest/agile/1.0/board/{id}/configuration"), None)?;
    let info = request("GET", &format!("/rest/agile/1.0/board/{id}"), None)?;
    let sp = request("GET", &format!("/rest/agile/1.0/board/{id}/sprint?state=active"), None).unwrap_or(json!({"values": []}));
    let project = s(&info["location"]["projectKey"]);
    let pf = if project.is_empty() { None } else { points_field(&project) };
    let mut fields: Vec<String> = ISSUE_FIELDS.iter().map(|x| x.to_string()).collect();
    fields.extend(["duedate", "labels", "issuelinks", "sprint"].iter().map(|x| x.to_string()));
    if let Some(p) = &pf {
        fields.push(p.clone());
    }
    let fields = fields.join(",");
    let active = sp["values"].get(0).cloned();
    let sprint = active.as_ref().map(|a| json!({"id": a["id"], "name": s(&a["name"]), "end": s(&a["endDate"])}));
    let effective = if scope == "sprint" && sprint.is_none() { "all" } else { scope };
    let path = match effective {
        "sprint" => format!("/rest/agile/1.0/sprint/{}/issue?fields={fields}", active.as_ref().unwrap()["id"]),
        "backlog" => format!("/rest/agile/1.0/board/{id}/backlog?jql={}&fields={fields}", enc(OPEN)),
        _ => format!("/rest/agile/1.0/board/{id}/issue?jql={}&fields={fields}", enc(OPEN)),
    };
    let mut issues: Vec<Value> = agile_all(&path, 500)?.iter().map(|i| card(i, pf.as_deref())).collect();
    // subtasks inherit their story's epic
    let epics: HashMap<String, Value> = issues.iter().filter(|i| !i["epic"].is_null()).map(|i| (s(&i["key"]), i["epic"].clone())).collect();
    for i in issues.iter_mut() {
        if i["subtask"] == json!(true) && i["epic"].is_null() {
            if let Some(e) = epics.get(&s(&i["parent"])) {
                i["epic"] = e.clone();
            }
        }
    }
    let types: Vec<String> = if project.is_empty() {
        vec![]
    } else {
        project_meta(&project)
            .unwrap_or_default()
            .iter()
            .filter(|t| t["subtask"] != json!(true) && s(&t["name"]) != "Epic")
            .map(|t| s(&t["name"]))
            .collect()
    };
    let columns: Vec<Value> = conf["columnConfig"]["columns"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            let st: Vec<String> = c["statuses"].as_array().map(|a| a.iter().map(|x| s(&x["id"])).collect()).unwrap_or_default();
            let iss: Vec<Value> = issues.iter().filter(|i| st.contains(&s(&i["statusId"]))).cloned().collect();
            json!({"name": s(&c["name"]), "statuses": st, "issues": iss})
        })
        .collect();
    Ok(json!({"id": id, "name": s(&conf["name"]), "project": project, "sprint": sprint, "scope": effective, "types": types, "columns": columns}))
}

/// Each epic of a project with its children's progress.
pub fn epics(project: &str) -> Result<Vec<Value>> {
    let pf = points_field(project);
    let list = search_all(&format!("project = {project} AND issuetype = Epic ORDER BY created DESC"), &["summary", "status"], 500)?;
    if list.is_empty() {
        return Ok(vec![]);
    }
    let mut f = vec!["status", "parent", "issuelinks", "issuetype"];
    if let Some(p) = &pf {
        f.push(p);
    }
    let keys: Vec<String> = list.iter().map(|e| s(&e["key"])).collect();
    let kids = search_all(&format!("parent in ({})", keys.join(",")), &f, 2000)?;
    Ok(list
        .iter()
        .map(|e| {
            let k = s(&e["key"]);
            let mine: Vec<Value> = kids.iter().filter(|x| s(&x["fields"]["parent"]["key"]) == k).map(|x| card(x, pf.as_deref())).collect();
            let done: Vec<&Value> = mine.iter().filter(|x| x["category"] == "done").collect();
            let pts = |xs: &[&Value]| xs.iter().map(|x| x["points"].as_f64().unwrap_or(0.0)).sum::<f64>();
            let all: Vec<&Value> = mine.iter().collect();
            json!({
                "key": k, "summary": s(&e["fields"]["summary"]), "status": s(&e["fields"]["status"]["name"]),
                "category": s(&e["fields"]["status"]["statusCategory"]["key"]),
                "total": mine.len(), "done": done.len(), "points": pts(&all), "pointsDone": pts(&done),
                "blocked": mine.iter().filter(|x| x["blocked"] == json!(true) && x["category"] != "done").count(),
                "doing": mine.iter().filter(|x| x["category"] == "indeterminate").count(),
            })
        })
        .collect())
}

pub fn issue_line(i: &Value) -> String {
    let mut out = format!("{}  [{}]  {}", s(&i["key"]), s(&i["status"]), s(&i["summary"]));
    if !s(&i["assignee"]).is_empty() {
        out.push_str(&format!("  · {}", s(&i["assignee"])));
    }
    if !s(&i["parent"]).is_empty() {
        out.push_str(&format!("  ↑{}", s(&i["parent"])));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adf_round_trip_and_keys() {
        let a = text_to_adf("one\ntwo\n\nthree");
        assert_eq!(adf_to_text(&a).trim(), "onetwo\nthree".replace("onetwo", "one\ntwo"));
        assert!(valid_key("RAID-12") && valid_key("A1_B-3") && !valid_key("raid-1") && !valid_key("RAID-x"));
        let m = mermaid_deps("t", &[json!({"id": "A-1", "label": "A-1 do [x]", "category": "done", "url": "u"})], &[("A-1".into(), "B-2".into())]);
        assert!(m.contains("A-1[\"A-1 do  x\"]:::done".replace("do  x", "do x").as_str()));
        assert!(m.contains("A-1 --> B-2"));
    }
}
