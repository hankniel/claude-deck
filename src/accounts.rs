//! Accounts are logins on the one ~/.claude config. `default` is the login Claude Code keeps
//! there itself; every other account is a long-lived token from `claude setup-token`, given to
//! its sessions as CLAUDE_CODE_OAUTH_TOKEN. Settings, skills, plugins, MCP and history are
//! shared because every session runs on the same config.

use crate::config::{data_dir, default_claude_dir, home, profiles_dir};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const DEFAULT: &str = "default";
const TOKEN_PREFIX: &str = "sk-ant-oat";

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub name: String,
    /// None for the default ~/.claude login.
    pub token_file: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct OAuth {
    pub access_token: String,
    pub expires_at_ms: Option<i64>,
    pub plan: Option<String>,
}

pub fn tokens_dir() -> PathBuf {
    data_dir().join("tokens")
}

impl Account {
    pub fn default_account() -> Account {
        Account { name: DEFAULT.into(), token_file: None }
    }

    pub fn is_default(&self) -> bool {
        self.token_file.is_none()
    }

    /// The Claude Code config every account runs on.
    pub fn dir(&self) -> PathBuf {
        default_claude_dir()
    }

    fn oauth_account(&self) -> Option<serde_json::Value> {
        if !self.is_default() {
            return None;
        }
        let s = std::fs::read_to_string(home().join(".claude.json")).ok()?;
        let mut v: serde_json::Value = serde_json::from_str(&s).ok()?;
        Some(v.get_mut("oauthAccount")?.take())
    }

    /// Known for `default` only: setup-token tokens can't read the profile.
    pub fn email(&self) -> Option<String> {
        self.oauth_account()?.get("emailAddress")?.as_str().map(String::from)
    }

    /// Claude's account id; transcripts of `default` sessions are tagged with it.
    pub fn account_uuid(&self) -> Option<String> {
        self.oauth_account()?.get("accountUuid")?.as_str().map(String::from)
    }

    pub fn token(&self) -> Option<String> {
        let f = self.token_file.as_ref()?;
        let t = std::fs::read_to_string(f).ok()?.trim().to_string();
        (!t.is_empty()).then_some(t)
    }

    /// The full login of `default` (keychain on macOS, else .credentials.json).
    pub fn oauth(&self) -> Result<Option<OAuth>> {
        if !self.is_default() {
            return Ok(None);
        }
        let mut raw = None;
        if cfg!(target_os = "macos") {
            raw = read_keychain("Claude Code-credentials")?;
        }
        if raw.is_none() {
            raw = std::fs::read_to_string(default_claude_dir().join(".credentials.json")).ok();
        }
        let Some(raw) = raw else { return Ok(None) };
        let v: serde_json::Value = serde_json::from_str(&raw).context("parsing credentials")?;
        let Some(o) = v.get("claudeAiOauth") else { return Ok(None) };
        let Some(tok) = o.get("accessToken").and_then(|t| t.as_str()) else { return Ok(None) };
        Ok(Some(OAuth {
            access_token: tok.to_string(),
            expires_at_ms: o.get("expiresAt").and_then(|e| e.as_i64()),
            plan: o.get("subscriptionType").and_then(|e| e.as_str()).map(String::from),
        }))
    }

    /// Environment for a claude process on this account, as (name, Some(value) | None to unset).
    pub fn env(&self) -> Vec<(&'static str, Option<String>)> {
        vec![
            ("CLAUDE_CONFIG_DIR", None),
            ("CLAUDE_CODE_OAUTH_TOKEN", self.token()),
            ("CLAUDE_DECK_ACCOUNT", Some(self.name.clone())),
        ]
    }

    /// Point a std Command at this account.
    pub fn apply_env(&self, cmd: &mut Command) {
        for (k, v) in self.env() {
            match v {
                Some(v) => cmd.env(k, v),
                None => cmd.env_remove(k),
            };
        }
    }

    /// claude's args for a session: the user's, plus a status line that reports this
    /// account's usage (Claude Code hands its rate limits to the status line).
    pub fn claude_args(&self, cwd: &Path, user: &[String]) -> Vec<String> {
        let mut out = user.to_vec();
        if user.iter().any(|a| a == "--settings" || a.starts_with("--settings=")) {
            return out;
        }
        let Ok(exe) = std::env::current_exe() else { return out };
        let mut line = serde_json::json!({
            "type": "command",
            "command": format!("{} statusline", shell_quote(&exe.to_string_lossy())),
        });
        if let Some(p) = user_statusline(cwd).and_then(|s| s.get("padding").cloned()) {
            line["padding"] = p;
        }
        out.push("--settings".into());
        out.push(serde_json::json!({ "statusLine": line }).to_string());
        out
    }
}

/// The status line the user configured, nearest settings file first.
pub fn user_statusline(cwd: &Path) -> Option<serde_json::Value> {
    let files = [
        cwd.join(".claude/settings.local.json"),
        cwd.join(".claude/settings.json"),
        default_claude_dir().join("settings.json"),
    ];
    files.iter().find_map(|f| {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        v.get("statusLine").filter(|s| s.get("command").is_some()).cloned()
    })
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn read_keychain(service: &str) -> Result<Option<String>> {
    let user = std::env::var("USER").unwrap_or_default();
    for with_user in [true, false] {
        let mut c = Command::new("security");
        c.arg("find-generic-password");
        if with_user && !user.is_empty() {
            c.args(["-a", &user]);
        }
        c.args(["-w", "-s", service]);
        let out = c.output().context("running `security`")?;
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Ok(Some(unhex(s)));
            }
        }
    }
    Ok(None)
}

/// `security -w` prints non-ASCII payloads as hex.
fn unhex(s: String) -> String {
    if s.starts_with('{') || !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return s;
    }
    let bytes: Vec<u8> = (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes).unwrap_or(s)
}

pub fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 40
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        || name.starts_with('.')
    {
        bail!("account names use letters, digits, '.', '_' and '-' (max 40)");
    }
    Ok(())
}

/// `default` first, then token accounts sorted by name.
pub fn list() -> Vec<Account> {
    let mut out = vec![Account::default_account()];
    let mut names: Vec<String> = std::fs::read_dir(tokens_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_name(n).is_ok() && n != DEFAULT)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    out.extend(names.into_iter().map(|n| Account { token_file: Some(tokens_dir().join(&n)), name: n }));
    out
}

pub fn find(name: &str) -> Result<Account> {
    list()
        .into_iter()
        .find(|a| a.name == name || a.email().as_deref() == Some(name))
        .with_context(|| format!("no account named {name:?} (see `claude-deck list`)"))
}

/// Whitespace is dropped so a token copied across wrapped terminal lines still works.
pub fn clean_token(raw: &str) -> Result<String> {
    let t: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if !t.starts_with(TOKEN_PREFIX) {
        bail!("that doesn't look like a `claude setup-token` token (it starts with {TOKEN_PREFIX}…)");
    }
    Ok(t)
}

pub enum TokenCheck {
    Valid,
    Rejected,
    Unknown(String),
}

/// Ask the API whether the token is accepted, without using any quota.
pub fn check_token(token: &str) -> TokenCheck {
    let r = ureq::post("https://api.anthropic.com/v1/messages/count_tokens")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-version", "2023-06-01")
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", concat!("claude-deck/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "model": "claude-haiku-5-5",
            "messages": [{"role": "user", "content": "hi"}],
        }));
    match r {
        Ok(_) => TokenCheck::Valid,
        Err(ureq::Error::Status(401 | 403, _)) => TokenCheck::Rejected,
        Err(e) => TokenCheck::Unknown(e.to_string()),
    }
}

/// Store (or replace) the token of account `name`.
pub fn save(name: &str, token: &str) -> Result<Account> {
    valid_name(name)?;
    if name == DEFAULT {
        bail!("`default` is your ~/.claude login; log in there with `claude` and /login");
    }
    let token = clean_token(token)?;
    let dir = tokens_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let path = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(token.as_bytes())?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(Account { name: name.into(), token_file: Some(path) })
}

pub fn remove(name: &str) -> Result<()> {
    if name == DEFAULT {
        bail!("cannot remove the default account");
    }
    let acc = find(name)?;
    std::fs::remove_file(acc.token_file.unwrap())?;
    let _ = std::fs::remove_file(crate::usage::live_path(name));
    Ok(())
}

/// accountUuid -> name from the profile dirs older versions created, so their sessions
/// stay attributed to the account of the same name.
pub fn legacy_uuids() -> HashMap<String, String> {
    let Ok(rd) = std::fs::read_dir(profiles_dir()) else { return HashMap::new() };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let s = std::fs::read_to_string(e.path().join(".claude.json")).ok()?;
            let v: serde_json::Value = serde_json::from_str(&s).ok()?;
            Some((v.get("oauthAccount")?.get("accountUuid")?.as_str()?.to_string(), name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_cleanup() {
        assert_eq!(clean_token(" sk-ant-oat01-ab\n  cd \n").unwrap(), "sk-ant-oat01-abcd");
        assert!(clean_token("sk-ant-api03-x").is_err());
    }
}
