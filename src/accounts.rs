//! Accounts are logins on the one ~/.claude config. `default` is the login Claude Code keeps
//! there itself; every other account is a long-lived token from `claude setup-token`, given to
//! its sessions as CLAUDE_CODE_OAUTH_TOKEN. Settings, skills, plugins, MCP and history are
//! shared because every session runs on the same config.
//!
//! A config-dir account instead runs on a Claude Code config dir of its own (e.g.
//! ~/.claude-bravestars), given to its sessions as CLAUDE_CONFIG_DIR, with the login, settings,
//! plugins, MCP and history kept there.
//!
//! Profiles are the configs a session can run on: `default` (~/.claude) and the dir of each
//! config-dir account, named after it. A session is an account on a profile: a logged-in
//! account (`default`, config-dir) only runs on its own profile, a token account on any.

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
    /// Set for token accounts.
    pub token_file: Option<PathBuf>,
    /// Set for config-dir accounts.
    pub config_dir: Option<PathBuf>,
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

/// One file per config-dir account, holding the dir's path.
pub fn dirs_dir() -> PathBuf {
    data_dir().join("dirs")
}

impl Account {
    pub fn default_account() -> Account {
        Account { name: DEFAULT.into(), token_file: None, config_dir: None }
    }

    pub fn is_default(&self) -> bool {
        self.token_file.is_none() && self.config_dir.is_none()
    }

    /// `default` and config-dir accounts hold a full login; token accounts only a token.
    pub fn has_login(&self) -> bool {
        self.token_file.is_none()
    }

    /// The Claude Code config this account runs on.
    pub fn dir(&self) -> PathBuf {
        self.config_dir.clone().unwrap_or_else(default_claude_dir)
    }

    /// This account on `profile`: a token account runs on any profile, a logged-in account
    /// only on its own (its login lives in that config), so it is returned unchanged.
    pub fn on(&self, profile: &Account) -> Account {
        if self.has_login() {
            return self.clone();
        }
        Account { config_dir: profile.config_dir.clone(), ..self.clone() }
    }

    /// The name of the profile this account runs on.
    pub fn profile(&self) -> String {
        if self.has_login() {
            return self.name.clone();
        }
        profile_by_dir(&self.dir()).map(|p| p.name).unwrap_or_else(|| DEFAULT.into())
    }

    fn oauth_account(&self) -> Option<serde_json::Value> {
        if !self.has_login() {
            return None;
        }
        // Claude Code keeps .claude.json inside CLAUDE_CONFIG_DIR, else in the home dir.
        let f = match &self.config_dir {
            Some(d) => d.join(".claude.json"),
            None => home().join(".claude.json"),
        };
        let s = std::fs::read_to_string(f).ok()?;
        let mut v: serde_json::Value = serde_json::from_str(&s).ok()?;
        Some(v.get_mut("oauthAccount")?.take())
    }

    /// Unknown for token accounts: setup-token tokens can't read the profile.
    pub fn email(&self) -> Option<String> {
        self.oauth_account()?.get("emailAddress")?.as_str().map(String::from)
    }

    /// Claude's account id; transcripts of logged-in sessions are tagged with it.
    pub fn account_uuid(&self) -> Option<String> {
        self.oauth_account()?.get("accountUuid")?.as_str().map(String::from)
    }

    pub fn token(&self) -> Option<String> {
        let f = self.token_file.as_ref()?;
        let t = std::fs::read_to_string(f).ok()?.trim().to_string();
        (!t.is_empty()).then_some(t)
    }

    /// The full login of `default` (keychain on macOS, else .credentials.json) or of a
    /// config-dir account (its .credentials.json).
    pub fn oauth(&self) -> Result<Option<OAuth>> {
        if !self.has_login() {
            return Ok(None);
        }
        let mut raw = None;
        if cfg!(target_os = "macos") && self.is_default() {
            raw = read_keychain("Claude Code-credentials")?;
        }
        if raw.is_none() {
            raw = std::fs::read_to_string(self.dir().join(".credentials.json")).ok();
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
            ("CLAUDE_CONFIG_DIR", self.config_dir.as_ref().map(|d| d.to_string_lossy().to_string())),
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
        if let Some(p) = user_statusline(cwd, &self.dir()).and_then(|s| s.get("padding").cloned()) {
            line["padding"] = p;
        }
        out.push("--settings".into());
        out.push(serde_json::json!({ "statusLine": line }).to_string());
        out
    }
}

/// The status line the user configured, nearest settings file first; `config_dir` is the
/// session's Claude Code config.
pub fn user_statusline(cwd: &Path, config_dir: &Path) -> Option<serde_json::Value> {
    let files = [
        cwd.join(".claude/settings.local.json"),
        cwd.join(".claude/settings.json"),
        config_dir.join("settings.json"),
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

fn names_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_name(n).is_ok() && n != DEFAULT)
                .collect()
        })
        .unwrap_or_default()
}

/// `default` first, then token and config-dir accounts sorted by name.
pub fn list() -> Vec<Account> {
    let mut out: Vec<Account> = names_in(&tokens_dir())
        .into_iter()
        .map(|n| Account { token_file: Some(tokens_dir().join(&n)), config_dir: None, name: n })
        .collect();
    for n in names_in(&dirs_dir()) {
        if out.iter().any(|a| a.name == n) {
            continue;
        }
        let Ok(p) = std::fs::read_to_string(dirs_dir().join(&n)) else { continue };
        out.push(Account { token_file: None, config_dir: Some(PathBuf::from(p.trim())), name: n });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.insert(0, Account::default_account());
    out
}

/// The configs sessions can run on, `default` first: the logged-in accounts.
pub fn profiles() -> Vec<Account> {
    list().into_iter().filter(|a| a.has_login()).collect()
}

pub fn find_profile(name: &str) -> Result<Account> {
    profiles()
        .into_iter()
        .find(|p| p.name == name)
        .with_context(|| format!("no profile named {name:?} (see `claude-deck profiles`)"))
}

pub fn profile_by_dir(dir: &Path) -> Option<Account> {
    profiles().into_iter().find(|p| p.dir() == dir)
}

/// Account `name` on profile `profile` (None: the token account's `default` profile).
pub fn find_on(name: &str, profile: Option<&str>) -> Result<Account> {
    let acc = find(name)?;
    match profile {
        Some(p) => {
            let p = find_profile(p)?;
            if acc.has_login() && acc.name != p.name {
                bail!("{} is logged in on its own profile {:?}; only token accounts run on another profile", acc.name, acc.name);
            }
            Ok(acc.on(&p))
        }
        None => Ok(acc),
    }
}

/// Account `name` for a session in `cwd`: on `profile` when given, else a token account on
/// the folder's mapped profile (or `default`).
pub fn for_session(name: &str, profile: Option<&str>, cfg: &crate::config::Config, cwd: &Path) -> Result<Account> {
    if profile.is_some() {
        return find_on(name, profile);
    }
    let acc = find(name)?;
    match cfg.profile_for(cwd).and_then(|p| find_profile(&p).ok()) {
        Some(p) => Ok(acc.on(&p)),
        None => Ok(acc),
    }
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
    let _ = std::fs::remove_file(dirs_dir().join(name));
    Ok(Account { name: name.into(), token_file: Some(path), config_dir: None })
}

/// Store (or replace) config-dir account `name`, running on the Claude Code config `dir`.
pub fn save_dir(name: &str, dir: &Path) -> Result<Account> {
    valid_name(name)?;
    if name == DEFAULT {
        bail!("`default` is your ~/.claude login; log in there with `claude` and /login");
    }
    let dir = dir.canonicalize().with_context(|| format!("{} does not exist", dir.display()))?;
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    if dir == default_claude_dir().canonicalize().unwrap_or_else(|_| default_claude_dir()) {
        bail!("{} is the `default` account's config", dir.display());
    }
    std::fs::create_dir_all(dirs_dir())?;
    std::fs::write(dirs_dir().join(name), dir.to_string_lossy().as_bytes())?;
    let _ = std::fs::remove_file(tokens_dir().join(name));
    Ok(Account { name: name.into(), token_file: None, config_dir: Some(dir) })
}

pub fn remove(name: &str) -> Result<()> {
    if name == DEFAULT {
        bail!("cannot remove the default account");
    }
    let acc = find(name)?;
    match acc.token_file {
        Some(f) => std::fs::remove_file(f)?,
        None => std::fs::remove_file(dirs_dir().join(name))?,
    }
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
    fn token_accounts_take_the_profile_logged_in_ones_keep_theirs() {
        let work = Account { name: "work".into(), token_file: None, config_dir: Some("/x/.claude-work".into()) };
        let tok = Account { name: "tok".into(), token_file: Some("/t/tok".into()), config_dir: None };
        let on = tok.on(&work);
        assert_eq!(on.dir(), PathBuf::from("/x/.claude-work"));
        assert_eq!(on.token_file, tok.token_file);
        assert_eq!(Account::default_account().on(&work), Account::default_account());
        assert_eq!(on.on(&Account::default_account()).dir(), default_claude_dir());
    }

    #[test]
    fn token_cleanup() {
        assert_eq!(clean_token(" sk-ant-oat01-ab\n  cd \n").unwrap(), "sk-ant-oat01-abcd");
        assert!(clean_token("sk-ant-api03-x").is_err());
    }
}
