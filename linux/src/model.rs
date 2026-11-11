//! Account/usage snapshot for the UI, read straight from the claude-deck core.

use chrono::{DateTime, Local, Utc};
use claude_deck::accounts::{self, Account};
use claude_deck::config::{home, Config};
use claude_deck::stats;
use claude_deck::usage::{self, Usage, Window};

#[derive(Clone, Debug)]
pub struct AccountInfo {
    pub name: String,
    pub email: Option<String>,
    pub disabled: bool,
    pub usage: Usage,
}

impl AccountInfo {
    fn new(acc: &Account, usage: Usage, cfg: &Config) -> AccountInfo {
        AccountInfo {
            name: acc.name.clone(),
            email: acc.email(),
            disabled: cfg.disabled.contains(&acc.name),
            usage,
        }
    }

    pub fn five(&self) -> Option<&Window> {
        match &self.usage {
            Usage::Ok { five, .. } => five.as_ref(),
            _ => None,
        }
    }

    pub fn week(&self) -> Option<&Window> {
        match &self.usage {
            Usage::Ok { week, .. } => week.as_ref(),
            _ => None,
        }
    }

    pub fn five_pct(&self) -> Option<f64> {
        self.five().map(|w| w.current_pct())
    }

    pub fn week_pct(&self) -> Option<f64> {
        self.week().map(|w| w.current_pct())
    }

    pub fn is_ok(&self) -> bool {
        matches!(self.usage, Usage::Ok { .. })
    }

    pub fn logged_out(&self) -> bool {
        matches!(self.usage, Usage::NotLoggedIn)
    }

    pub fn needs_login(&self) -> bool {
        matches!(self.usage, Usage::NotLoggedIn | Usage::Expired)
    }

    pub fn over(&self, cfg: &Config) -> bool {
        self.usage.over(cfg)
    }

    pub fn pressure(&self, cfg: &Config) -> Option<f64> {
        self.usage.pressure(cfg)
    }

    pub fn usable(&self, cfg: &Config) -> bool {
        self.is_ok() && !self.over(cfg)
    }

    pub fn plan_label(&self) -> Option<String> {
        let Usage::Ok { plan: Some(p), .. } = &self.usage else { return None };
        let mut c = p.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str())
    }

    pub fn state_label(&self) -> String {
        match &self.usage {
            Usage::Ok { .. } => "Ready".into(),
            Usage::Unknown => "Checking usage…".into(),
            Usage::NotLoggedIn => "Not logged in".into(),
            Usage::Expired => "Refreshes on next use".into(),
            Usage::Error(e) => format!("Couldn't read usage: {e}"),
        }
    }

    /// "5h 62% · 7d 21% · ↻ 2h10m"
    pub fn summary(&self) -> String {
        if !self.is_ok() {
            return self.state_label();
        }
        let mut parts = vec![];
        if let Some(w) = self.five() {
            parts.push(format!("5h {:.0}%", w.current_pct()));
        }
        if let Some(w) = self.week() {
            parts.push(format!("7d {:.0}%", w.current_pct()));
        }
        if let Some(t) = self.five().and_then(|w| w.resets_at) {
            parts.push(format!("↻ {}", usage::until(t)));
        }
        if parts.is_empty() {
            "No limits reported".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// Accounts with usage still unknown (instant, no network).
pub fn quick_state() -> (Vec<AccountInfo>, Config) {
    let cfg = Config::load().unwrap_or_default();
    let accs = accounts::list().iter().map(|a| AccountInfo::new(a, Usage::Unknown, &cfg)).collect();
    (accs, cfg)
}

/// Fetch every account's usage in parallel. With `record`, append a history sample for the dashboard.
pub fn fetch_state(record: bool) -> Result<(Vec<AccountInfo>, Config), String> {
    let cfg = Config::load().map_err(|e| format!("{e:#}"))?;
    let accs = accounts::list();
    let handles: Vec<_> = accs.iter().cloned().map(|a| std::thread::spawn(move || usage::fetch(&a))).collect();
    let now = Utc::now().to_rfc3339();
    let mut out = vec![];
    for (acc, h) in accs.iter().zip(handles) {
        let u = h.join().unwrap_or(Usage::Error("panic".into()));
        if record {
            if let Usage::Ok { five, week, .. } = &u {
                let _ = stats::append_jsonl(
                    &stats::history_path(),
                    &serde_json::json!({"ts": now, "account": acc.name,
                        "five": five.as_ref().map(|w| w.current_pct()),
                        "week": week.as_ref().map(|w| w.current_pct())}),
                );
            }
        }
        out.push(AccountInfo::new(acc, u, &cfg));
    }
    Ok((out, cfg))
}

/// The usable, enabled account with the most headroom.
pub fn best<'a>(accs: &'a [AccountInfo], cfg: &Config, excluding: Option<&str>) -> Option<&'a AccountInfo> {
    accs.iter()
        .filter(|a| Some(a.name.as_str()) != excluding && !a.disabled && a.usable(cfg))
        .min_by(|a, b| {
            a.pressure(cfg).unwrap_or(9.0).total_cmp(&b.pressure(cfg).unwrap_or(9.0))
        })
}

// ---------- formatting ----------

pub fn short_path(p: &str) -> String {
    let h = home().to_string_lossy().to_string();
    match p.strip_prefix(&h) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

pub fn last_component(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

pub fn tokens(n: u64) -> String {
    let d = n as f64;
    if d >= 1e9 {
        format!("{:.1}B", d / 1e9)
    } else if d >= 1e6 {
        format!("{:.1}M", d / 1e6)
    } else if d >= 1e3 {
        format!("{:.1}K", d / 1e3)
    } else {
        n.to_string()
    }
}

pub fn money(v: f64) -> String {
    if v >= 100.0 {
        format!("${v:.0}")
    } else {
        format!("${v:.2}")
    }
}

pub fn minutes(m: f64) -> String {
    let (h, mm) = ((m as i64) / 60, (m as i64) % 60);
    if h > 0 {
        format!("{h}h {mm}m")
    } else {
        format!("{mm}m")
    }
}

pub fn relative(t: DateTime<Utc>) -> String {
    let s = (Utc::now() - t).num_seconds().max(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86399 => format!("{} h ago", s / 3600),
        86400..=172799 => "yesterday".into(),
        _ => t.with_timezone(&Local).format("%b %-d").to_string(),
    }
}

pub fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// "claude-opus-5-5" → "Opus 5.5", "claude-haiku-4-5-20251001" → "Haiku 4.5".
pub fn pretty_model(id: &str) -> String {
    let parts: Vec<&str> = id.trim_start_matches("claude-").split('-').collect();
    let mut out: Vec<String> = vec![];
    let mut prev_num = false;
    for (i, p) in parts.iter().enumerate() {
        let num = !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit());
        if p.len() >= 8 && p.chars().all(|c| c.is_ascii_digit()) {
            continue; // date suffix
        }
        if i == 0 {
            let mut c = p.chars();
            out.push(c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default());
        } else if num && prev_num {
            let last = out.pop().unwrap_or_default();
            out.push(format!("{last}.{p}"));
        } else {
            out.push(p.to_string());
        }
        prev_num = num;
    }
    out.join(" ")
}

pub fn account_label(name: &str) -> String {
    match name {
        "unattributed" => "Before tracking".into(),
        "other" => "Other".into(),
        _ => name.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names() {
        assert_eq!(pretty_model("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(pretty_model("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(pretty_model("claude-sonnet-4"), "Sonnet 4");
    }

    #[test]
    fn formats() {
        assert_eq!(tokens(1_234_567), "1.2M");
        assert_eq!(money(3.456), "$3.46");
        assert_eq!(minutes(125.0), "2h 5m");
    }
}
