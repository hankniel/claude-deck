//! Local analytics: token/cost from Claude Code transcripts, usage history samples,
//! and the event log (session starts, account switches, limit warnings).

use crate::accounts;
use crate::config::{data_dir, default_claude_dir};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub fn events_path() -> PathBuf {
    data_dir().join("events.jsonl")
}

pub fn history_path() -> PathBuf {
    data_dir().join("usage-history.jsonl")
}

pub fn append_jsonl(path: &Path, v: &Value) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{v}")?;
    Ok(())
}

/// kinds: start, switch, exit, limit, auto_switch
pub fn log_event(kind: &str, account: Option<&str>, message: &str, extra: Value) {
    let mut v = json!({ "ts": Utc::now().to_rfc3339(), "kind": kind, "account": account, "message": message });
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    let _ = append_jsonl(&events_path(), &v);
}

// ---------- pricing (USD per million tokens, API list price) ----------

struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
}

fn price(model: &str) -> Price {
    let m = model.trim_start_matches("anthropic.");
    let p = |input: f64, output: f64, cache_read: f64| Price { input, output, cache_read };
    let has = |s: &str| m.contains(s);
    if has("fable-5-2") || has("mythos-5-2") {
        p(8.0, 40.0, 0.20)
    } else if has("fable") || has("mythos") {
        p(10.0, 50.0, 1.0)
    } else if has("opus-5-5") {
        p(4.0, 20.0, 0.20)
    } else if has("opus-5") || has("opus-4-8") || has("opus-4-7") || has("opus-4-6") || has("opus-4-5") {
        p(5.0, 25.0, 0.50)
    } else if has("opus") {
        p(15.0, 75.0, 1.50)
    } else if has("sonnet-5-5") {
        p(2.0, 10.0, 0.10)
    } else if has("sonnet-5") {
        p(2.0, 10.0, 0.20)
    } else if has("sonnet") {
        p(3.0, 15.0, 0.30)
    } else if has("haiku-5-5") {
        p(0.10, 0.50, 0.01)
    } else if has("haiku-4-5") {
        p(1.0, 5.0, 0.10)
    } else if has("haiku") {
        p(0.80, 4.0, 0.08)
    } else {
        p(5.0, 25.0, 0.50)
    }
}

// ---------- transcript scan ----------

#[derive(Clone)]
struct Rec {
    ts: DateTime<Utc>,
    model: String,
    input: u64,
    output: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
    account_uuid: Option<String>,
    session: String,
    cwd: String,
}

impl Rec {
    fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }
    fn cost(&self) -> f64 {
        let p = price(&self.model);
        (self.input as f64 * p.input
            + self.output as f64 * p.output
            + self.cache_write_5m as f64 * p.input * 1.25
            + self.cache_write_1h as f64 * p.input * 2.0
            + self.cache_read as f64 * p.cache_read)
            / 1_000_000.0
    }
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn parse_line(line: &str) -> Option<(String, Rec)> {
    if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
        return None;
    }
    let d: Value = serde_json::from_str(line).ok()?;
    if d.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let m = d.get("message")?;
    let model = m.get("model")?.as_str()?.to_string();
    if model.starts_with('<') {
        return None;
    }
    let usage = m.get("usage")?;
    let ts = DateTime::parse_from_rfc3339(d.get("timestamp")?.as_str()?).ok()?.with_timezone(&Utc);
    let key = m
        .get("id")
        .and_then(|x| x.as_str())
        .or_else(|| d.get("requestId").and_then(|x| x.as_str()))
        .or_else(|| d.get("uuid").and_then(|x| x.as_str()))?
        .to_string();
    let cache_create = u(usage, "cache_creation_input_tokens");
    let (w5, w1) = match usage.get("cache_creation") {
        Some(cc) => {
            let a = u(cc, "ephemeral_5m_input_tokens");
            let b = u(cc, "ephemeral_1h_input_tokens");
            if a + b == 0 { (cache_create, 0) } else { (a, b) }
        }
        None => (cache_create, 0),
    };
    Some((
        key,
        Rec {
            ts,
            model,
            input: u(usage, "input_tokens"),
            output: u(usage, "output_tokens"),
            cache_write_5m: w5,
            cache_write_1h: w1,
            cache_read: u(usage, "cache_read_input_tokens"),
            account_uuid: d.get("accountUuid").and_then(|x| x.as_str()).map(String::from),
            session: d.get("sessionId").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            cwd: d.get("cwd").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        },
    ))
}

fn transcript_files(cutoff: SystemTime) -> Vec<PathBuf> {
    let mut roots = vec![default_claude_dir().join("projects")];
    for a in accounts::list() {
        roots.push(a.dir().join("projects"));
    }
    let mut seen = HashSet::new();
    let mut out = vec![];
    fn walk(dir: &Path, depth: usize, cutoff: SystemTime, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                if depth < 3 {
                    walk(&p, depth + 1, cutoff, out);
                }
            } else if p.extension().is_some_and(|x| x == "jsonl")
                && md.modified().map(|t| t >= cutoff).unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    for r in roots {
        let Ok(c) = r.canonicalize() else { continue };
        if seen.insert(c.clone()) {
            walk(&c, 0, cutoff, &mut out);
        }
    }
    out
}

fn read_jsonl(path: &Path, since: DateTime<Utc>) -> Vec<Value> {
    let Ok(f) = std::fs::File::open(path) else { return vec![] };
    BufReader::new(f)
        .lines()
        .map_while(|l| l.ok())
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .filter(|v| {
            v.get("ts")
                .and_then(|t| t.as_str())
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| t.with_timezone(&Utc) >= since)
        })
        .collect()
}

// ---------- output ----------

#[derive(Serialize, Default, Clone)]
pub struct DayRow {
    pub date: String,
    pub account: String,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub tokens: u64,
    pub cost: f64,
    pub messages: u64,
}

#[derive(Serialize, Default)]
pub struct AccountRow {
    pub account: String,
    pub sessions: usize,
    pub messages: u64,
    pub tokens: u64,
    pub cost: f64,
    pub active_minutes: f64,
}

#[derive(Serialize, Default)]
pub struct ProjectRow {
    pub project: String,
    pub sessions: usize,
    pub tokens: u64,
    pub cost: f64,
    pub last_used: String,
}

#[derive(Serialize, Default)]
pub struct Totals {
    pub tokens: u64,
    pub cost: f64,
    pub sessions: usize,
    pub messages: u64,
    pub active_minutes: f64,
    pub today_cost: f64,
    pub today_tokens: u64,
}

#[derive(Serialize)]
pub struct Stats {
    pub generated_at: String,
    pub days: u32,
    pub totals: Totals,
    pub daily: Vec<DayRow>,
    pub accounts: Vec<AccountRow>,
    pub projects: Vec<ProjectRow>,
    pub models: Vec<DayRow>,
    pub usage_history: Vec<Value>,
    pub events: Vec<Value>,
}

/// Active time: gaps between consecutive messages in a session, each capped at 5 minutes.
fn active_minutes(recs: &[&Rec]) -> f64 {
    let mut by_session: HashMap<&str, Vec<DateTime<Utc>>> = HashMap::new();
    for r in recs {
        by_session.entry(r.session.as_str()).or_default().push(r.ts);
    }
    by_session
        .values_mut()
        .map(|ts| {
            ts.sort();
            1.0 + ts.windows(2).map(|w| ((w[1] - w[0]).num_seconds() as f64 / 60.0).min(5.0)).sum::<f64>()
        })
        .sum()
}

pub fn compute(days: u32) -> Result<Stats> {
    let since = Utc::now() - chrono::Duration::days(days as i64);
    let cutoff = SystemTime::now() - Duration::from_secs(days as u64 * 86400 + 86400);

    // accountUuid -> account name; session id -> account (from our own start/switch events).
    let uuid_names: HashMap<String, String> = accounts::list()
        .into_iter()
        .filter_map(|a| Some((a.account_uuid()?, a.name)))
        .collect();
    let events = read_jsonl(&events_path(), since - chrono::Duration::days(30));
    let mut session_acc: HashMap<String, String> = HashMap::new();
    for e in &events {
        if let (Some(s), Some(a)) = (e.get("session_id").and_then(|x| x.as_str()), e.get("account").and_then(|x| x.as_str())) {
            session_acc.insert(s.to_string(), a.to_string());
        }
    }

    let files = transcript_files(cutoff);
    // Subagent transcripts carry their parent's sessionId but no owner line of their own.
    let mut global_owner: HashMap<String, String> = HashMap::new();
    let mut recs: HashMap<String, Rec> = HashMap::new();
    for f in &files {
        let Ok(file) = std::fs::File::open(f) else { continue };
        // `session-owner` lines say which account owns the session from that point on.
        let mut owner: HashMap<String, String> = HashMap::new();
        for line in BufReader::new(file).lines().map_while(|l| l.ok()) {
            if line.contains("\"session-owner\"") {
                if let Ok(d) = serde_json::from_str::<Value>(&line) {
                    if let (Some(s), Some(u)) = (
                        d.get("sessionId").and_then(|x| x.as_str()),
                        d.get("accountUuid").and_then(|x| x.as_str()),
                    ) {
                        owner.insert(s.to_string(), u.to_string());
                        global_owner.entry(s.to_string()).or_insert_with(|| u.to_string());
                    }
                }
                continue;
            }
            if let Some((k, mut r)) = parse_line(&line) {
                if r.ts >= since {
                    if r.account_uuid.is_none() {
                        r.account_uuid = owner.get(&r.session).cloned();
                    }
                    recs.insert(k, r);
                }
            }
        }
    }

    for r in recs.values_mut() {
        if r.account_uuid.is_none() {
            r.account_uuid = global_owner.get(&r.session).cloned();
        }
    }

    let account_of = |r: &Rec| -> String {
        r.account_uuid
            .as_ref()
            .and_then(|u| uuid_names.get(u).cloned())
            .or_else(|| session_acc.get(&r.session).cloned())
            .unwrap_or_else(|| "unattributed".into())
    };

    let today = Local::now().date_naive().to_string();
    let mut daily: BTreeMap<(String, String, String), DayRow> = BTreeMap::new();
    let mut models: BTreeMap<String, DayRow> = BTreeMap::new();
    let mut acc_recs: HashMap<String, Vec<&Rec>> = HashMap::new();
    let mut proj_recs: HashMap<String, Vec<&Rec>> = HashMap::new();
    let mut totals = Totals::default();
    for r in recs.values() {
        let acc = account_of(r);
        let date = r.ts.with_timezone(&Local).date_naive().to_string();
        let (tokens, cost) = (r.tokens(), r.cost());
        let row = daily.entry((date.clone(), acc.clone(), r.model.clone())).or_insert_with(|| DayRow {
            date: date.clone(),
            account: acc.clone(),
            model: r.model.clone(),
            ..Default::default()
        });
        for row in [row, models.entry(r.model.clone()).or_insert_with(|| DayRow { model: r.model.clone(), ..Default::default() })] {
            row.input += r.input;
            row.output += r.output;
            row.cache_write += r.cache_write_5m + r.cache_write_1h;
            row.cache_read += r.cache_read;
            row.tokens += tokens;
            row.cost += cost;
            row.messages += 1;
        }
        totals.tokens += tokens;
        totals.cost += cost;
        totals.messages += 1;
        if date == today {
            totals.today_cost += cost;
            totals.today_tokens += tokens;
        }
        acc_recs.entry(acc).or_default().push(r);
        proj_recs.entry(r.cwd.clone()).or_default().push(r);
    }

    let mut accounts: Vec<AccountRow> = acc_recs
        .iter()
        .map(|(a, rs)| AccountRow {
            account: a.clone(),
            sessions: rs.iter().map(|r| &r.session).collect::<HashSet<_>>().len(),
            messages: rs.len() as u64,
            tokens: rs.iter().map(|r| r.tokens()).sum(),
            cost: rs.iter().map(|r| r.cost()).sum(),
            active_minutes: active_minutes(rs),
        })
        .collect();
    accounts.sort_by(|a, b| b.cost.total_cmp(&a.cost));
    totals.sessions = recs.values().map(|r| &r.session).collect::<HashSet<_>>().len();
    totals.active_minutes = accounts.iter().map(|a| a.active_minutes).sum();

    let mut projects: Vec<ProjectRow> = proj_recs
        .iter()
        .map(|(p, rs)| ProjectRow {
            project: p.clone(),
            sessions: rs.iter().map(|r| &r.session).collect::<HashSet<_>>().len(),
            tokens: rs.iter().map(|r| r.tokens()).sum(),
            cost: rs.iter().map(|r| r.cost()).sum(),
            last_used: rs.iter().map(|r| r.ts).max().map(|t| t.to_rfc3339()).unwrap_or_default(),
        })
        .collect();
    projects.sort_by(|a, b| b.cost.total_cmp(&a.cost));
    projects.truncate(12);

    let mut models: Vec<DayRow> = models.into_values().collect();
    models.sort_by(|a, b| b.cost.total_cmp(&a.cost));

    let mut events: Vec<Value> = events
        .into_iter()
        .filter(|e| e.get("ts").and_then(|t| t.as_str()).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).is_some_and(|t| t >= since))
        .collect();
    events.reverse();
    events.truncate(200);

    Ok(Stats {
        generated_at: Utc::now().to_rfc3339(),
        days,
        totals,
        daily: daily.into_values().collect(),
        accounts,
        projects,
        models,
        usage_history: read_jsonl(&history_path(), since),
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assistant_usage() {
        let line = r#"{"type":"assistant","timestamp":"2026-11-11T04:51:34.817Z","sessionId":"s1","cwd":"/x","accountUuid":"u1","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":50,"cache_creation":{"ephemeral_1h_input_tokens":100,"ephemeral_5m_input_tokens":0}}}}"#;
        let (k, r) = parse_line(line).unwrap();
        assert_eq!(k, "m1");
        assert_eq!(r.cache_write_1h, 100);
        assert_eq!(r.tokens(), 1152);
        // 2*4 + 50*20 + 100*4*2 + 1000*0.2 = 8+1000+800+200 = 2008 per M
        assert!((r.cost() - 0.002008).abs() < 1e-9);
    }

    #[test]
    fn skips_non_assistant() {
        assert!(parse_line(r#"{"type":"user","message":{"usage":{}}}"#).is_none());
    }
}
