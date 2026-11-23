use crate::accounts::Account;
use crate::config::Config;
use chrono::{DateTime, Utc};
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Clone, Debug)]
pub struct Window {
    pub name: String,
    pub pct: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

impl Window {
    /// A window whose reset time has passed counts as empty.
    pub fn current_pct(&self) -> f64 {
        match self.resets_at {
            Some(t) if t <= Utc::now() => 0.0,
            _ => self.pct,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Usage {
    Unknown,
    Ok { five: Option<Window>, week: Option<Window>, others: Vec<Window>, plan: Option<String> },
    NotLoggedIn,
    /// A token account none of whose sessions has reported usage yet.
    NoData,
    /// Token expired: Claude Code refreshes it on its next request in that account.
    Expired,
    Error(String),
}

impl Usage {
    /// How close the account is to its switch-away lines (1.0 = at the line).
    pub fn pressure(&self, cfg: &Config) -> Option<f64> {
        match self {
            Usage::Ok { five, week, .. } => {
                let f = five.as_ref().map(|w| w.current_pct() / cfg.threshold).unwrap_or(0.0);
                let w = week.as_ref().map(|w| w.current_pct() / cfg.weekly_threshold).unwrap_or(0.0);
                Some(f.max(w))
            }
            _ => None,
        }
    }

    pub fn over(&self, cfg: &Config) -> bool {
        self.pressure(cfg).map(|p| p >= 1.0).unwrap_or(false)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Usage::Unknown => "loading…",
            Usage::Ok { .. } => "ok",
            Usage::NotLoggedIn => "not logged in",
            Usage::NoData => "no usage yet",
            Usage::Expired => "token expired",
            Usage::Error(_) => "error",
        }
    }
}

pub fn fetch(acc: &Account) -> Usage {
    if !acc.is_default() {
        if acc.token().is_none() {
            return Usage::NotLoggedIn;
        }
        return live(&acc.name).unwrap_or(Usage::NoData);
    }
    let oauth = match acc.oauth() {
        Ok(Some(o)) => o,
        Ok(None) => return Usage::NotLoggedIn,
        Err(e) => return Usage::Error(e.to_string()),
    };
    if let Some(exp) = oauth.expires_at_ms {
        if exp <= Utc::now().timestamp_millis() {
            return Usage::Expired;
        }
    }
    let resp = ureq::get(USAGE_URL)
        .set("Authorization", &format!("Bearer {}", oauth.access_token))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", concat!("claude-deck/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .call();
    let body: serde_json::Value = match resp {
        Ok(r) => match r.into_json() {
            Ok(v) => v,
            Err(e) => return Usage::Error(format!("bad response: {e}")),
        },
        Err(ureq::Error::Status(401, _)) => return Usage::Expired,
        Err(ureq::Error::Status(code, _)) => return cached(acc, oauth.plan).unwrap_or(Usage::Error(format!("HTTP {code}"))),
        Err(e) => return cached(acc, oauth.plan).unwrap_or(Usage::Error(e.to_string())),
    };
    let _ = save_cache(acc, &body);
    parse(&body, oauth.plan)
}

/// The last good answer is kept so a rate limit or a network blip doesn't blank the numbers.
fn cache_path(acc: &Account) -> std::path::PathBuf {
    crate::config::data_dir().join("usage-cache").join(format!("{}.json", acc.name))
}

fn save_cache(acc: &Account, body: &serde_json::Value) -> std::io::Result<()> {
    let p = cache_path(acc);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::json!({ "at": Utc::now().timestamp(), "body": body }).to_string())
}

fn cached(acc: &Account, plan: Option<String>) -> Option<Usage> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(cache_path(acc)).ok()?).ok()?;
    let age = Utc::now().timestamp() - v.get("at")?.as_i64()?;
    if age >= 30 * 60 {
        return None;
    }
    Some(parse(v.get("body")?, plan))
}

fn parse(body: &serde_json::Value, plan: Option<String>) -> Usage {
    let mut five = None;
    let mut week = None;
    let mut others = vec![];
    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            let Some(pct) = v.get("utilization").and_then(|u| u.as_f64()) else { continue };
            let resets_at = v
                .get("resets_at")
                .and_then(|r| r.as_str())
                .and_then(|r| DateTime::parse_from_rfc3339(r).ok())
                .map(|d| d.with_timezone(&Utc));
            let w = Window { name: k.clone(), pct, resets_at };
            match k.as_str() {
                "five_hour" => five = Some(w),
                "seven_day" => week = Some(w),
                _ => others.push(w),
            }
        }
    }
    others.sort_by(|a, b| a.name.cmp(&b.name));
    Usage::Ok { five, week, others, plan }
}

/// Token accounts can't call the usage endpoint; their sessions' status line saves the
/// rate limits Claude Code reads from its own API responses here instead.
pub fn live_path(account: &str) -> std::path::PathBuf {
    crate::config::data_dir().join("usage-live").join(format!("{account}.json"))
}

/// Save the `rate_limits` of a status line payload for `account`.
pub fn record_live(account: &str, status: &serde_json::Value) -> std::io::Result<()> {
    let Some(rl) = status.get("rate_limits").filter(|r| r.is_object()) else { return Ok(()) };
    let p = live_path(account);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::json!({ "at": Utc::now().timestamp(), "rate_limits": rl }).to_string())?;
    std::fs::rename(tmp, p)
}

/// Session id -> account, for stats: token sessions' transcripts carry no account id.
pub fn session_accounts_dir() -> std::path::PathBuf {
    crate::config::data_dir().join("session-accounts")
}

pub fn record_session_account(session: &str, account: &str) -> std::io::Result<()> {
    if session.is_empty() || session.contains('/') {
        return Ok(());
    }
    let p = session_accounts_dir().join(session);
    if std::fs::read_to_string(&p).is_ok_and(|a| a == account) {
        return Ok(());
    }
    std::fs::create_dir_all(session_accounts_dir())?;
    std::fs::write(p, account)
}

fn live(account: &str) -> Option<Usage> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(live_path(account)).ok()?).ok()?;
    Some(parse_live(v.get("rate_limits")?))
}

/// `{"five_hour": {"used_percentage": 12, "resets_at": <unix secs>}, "seven_day": {...}}`
fn parse_live(rl: &serde_json::Value) -> Usage {
    let win = |k: &str| {
        let w = rl.get(k)?;
        Some(Window {
            name: k.to_string(),
            pct: w.get("used_percentage")?.as_f64()?,
            resets_at: w.get("resets_at").and_then(|r| r.as_i64()).and_then(|t| DateTime::from_timestamp(t, 0)),
        })
    };
    Usage::Ok { five: win("five_hour"), week: win("seven_day"), others: vec![], plan: None }
}

/// "2h13m", "3d4h", "12m".
pub fn until(t: DateTime<Utc>) -> String {
    let secs = (t - Utc::now()).num_seconds().max(0);
    let (d, h, m) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows() {
        let v = serde_json::json!({
            "five_hour": {"utilization": 42.0, "resets_at": "2099-01-01T00:00:00Z"},
            "seven_day": {"utilization": 61.5, "resets_at": null},
            "seven_day_opus": {"utilization": 10},
            "extra_usage": {"is_enabled": false}
        });
        let Usage::Ok { five, week, others, .. } = parse(&v, None) else { panic!() };
        assert_eq!(five.unwrap().pct, 42.0);
        assert_eq!(week.unwrap().pct, 61.5);
        assert_eq!(others.len(), 1);
    }

    #[test]
    fn parses_status_line_limits() {
        let v = serde_json::json!({
            "five_hour": {"used_percentage": 7, "resets_at": 4102444800i64},
            "seven_day": {"used_percentage": 33.5, "resets_at": 4102444800i64}
        });
        let Usage::Ok { five, week, .. } = parse_live(&v) else { panic!() };
        assert_eq!(five.unwrap().pct, 7.0);
        let w = week.unwrap();
        assert_eq!(w.pct, 33.5);
        assert!(w.resets_at.is_some());
    }

    #[test]
    fn expired_window_counts_as_empty() {
        let w = Window { name: "x".into(), pct: 99.0, resets_at: Some(Utc::now() - chrono::Duration::minutes(1)) };
        assert_eq!(w.current_pct(), 0.0);
    }
}
