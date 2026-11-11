//! Accounts are Claude Code config dirs. `default` is the plain ~/.claude login;
//! every other account is a profile dir passed to Claude Code as CLAUDE_CONFIG_DIR.
//! Claude Code itself owns each profile's login (keychain entry or .credentials.json),
//! so tokens are never copied between accounts and only read for usage checks.

use crate::config::{default_claude_dir, home, profiles_dir, Config};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT: &str = "default";

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub name: String,
    /// None for the default ~/.claude login.
    pub config_dir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct OAuth {
    pub access_token: String,
    pub expires_at_ms: Option<i64>,
    pub plan: Option<String>,
}

impl Account {
    pub fn default_account() -> Account {
        Account { name: DEFAULT.into(), config_dir: None }
    }

    pub fn dir(&self) -> PathBuf {
        self.config_dir.clone().unwrap_or_else(default_claude_dir)
    }

    /// Claude Code's global state file (holds `oauthAccount`).
    fn global_json(&self) -> PathBuf {
        match &self.config_dir {
            Some(d) => d.join(".claude.json"),
            None => home().join(".claude.json"),
        }
    }

    fn oauth_account(&self) -> Option<serde_json::Value> {
        let s = std::fs::read_to_string(self.global_json()).ok()?;
        let mut v: serde_json::Value = serde_json::from_str(&s).ok()?;
        Some(v.get_mut("oauthAccount")?.take())
    }

    pub fn email(&self) -> Option<String> {
        self.oauth_account()?.get("emailAddress")?.as_str().map(String::from)
    }

    /// Claude's account id; transcripts tag each message with it.
    pub fn account_uuid(&self) -> Option<String> {
        self.oauth_account()?.get("accountUuid")?.as_str().map(String::from)
    }

    /// Mirrors Claude Code: `Claude Code-credentials` plus `-<sha256(dir)[..8]>`
    /// when CLAUDE_CONFIG_DIR is set.
    pub fn keychain_service(&self) -> String {
        match &self.config_dir {
            None => "Claude Code-credentials".into(),
            Some(d) => {
                let s: String = d.to_string_lossy().nfc().collect();
                let h = Sha256::digest(s.as_bytes());
                let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
                format!("Claude Code-credentials-{}", &hex[..8])
            }
        }
    }

    pub fn oauth(&self) -> Result<Option<OAuth>> {
        let mut raw = None;
        if cfg!(target_os = "macos") {
            raw = read_keychain(&self.keychain_service())?;
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

    /// Point a std Command at this account.
    pub fn apply_env(&self, cmd: &mut Command) {
        match &self.config_dir {
            Some(d) => cmd.env("CLAUDE_CONFIG_DIR", d),
            None => cmd.env_remove("CLAUDE_CONFIG_DIR"),
        };
        cmd.env("CLAUDE_DECK_ACCOUNT", &self.name);
    }
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

/// `default` first, then profiles sorted by name.
pub fn list() -> Vec<Account> {
    let mut out = vec![Account::default_account()];
    let mut names: Vec<String> = std::fs::read_dir(profiles_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_name(n).is_ok() && n != DEFAULT)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    out.extend(names.into_iter().map(|n| Account { config_dir: Some(profiles_dir().join(&n)), name: n }));
    out
}

pub fn find(name: &str) -> Result<Account> {
    list()
        .into_iter()
        .find(|a| a.name == name || a.email().as_deref() == Some(name))
        .with_context(|| format!("no account named {name:?} (see `claude-deck list`)"))
}

pub fn create(name: &str, cfg: &Config) -> Result<Account> {
    valid_name(name)?;
    if name == DEFAULT {
        bail!("`default` is the plain ~/.claude login and always exists");
    }
    let dir = profiles_dir().join(name);
    if dir.exists() {
        bail!("account {name:?} already exists");
    }
    std::fs::create_dir_all(&dir)?;
    let acc = Account { name: name.into(), config_dir: Some(dir) };
    link_shared(&acc, cfg)?;
    Ok(acc)
}

pub fn remove(name: &str) -> Result<()> {
    if name == DEFAULT {
        bail!("cannot remove the default account");
    }
    let acc = find(name)?;
    let dir = acc.config_dir.clone().unwrap();
    if cfg!(target_os = "macos") {
        let _ = Command::new("security")
            .args(["delete-generic-password", "-s", &acc.keychain_service()])
            .output();
    }
    // Remove symlinks first so remove_dir_all never walks into shared data.
    for e in std::fs::read_dir(&dir)?.flatten() {
        if e.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
            std::fs::remove_file(e.path())?;
        }
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// Symlink each shared ~/.claude item into the profile, leaving existing entries alone.
/// Returns the items that could not be linked because the profile has its own copy.
pub fn link_shared(acc: &Account, cfg: &Config) -> Result<Vec<String>> {
    let Some(dir) = &acc.config_dir else { return Ok(vec![]) };
    let src_root = default_claude_dir();
    let mut conflicts = vec![];
    for item in &cfg.shared {
        let src = src_root.join(item);
        let dst = dir.join(item);
        if item == "projects" && !src.exists() {
            std::fs::create_dir_all(&src)?;
        }
        if !src.exists() {
            continue;
        }
        match std::fs::symlink_metadata(&dst) {
            Ok(m) if m.file_type().is_symlink() => {}
            Ok(_) => conflicts.push(item.clone()),
            Err(_) => std::os::unix::fs::symlink(&src, &dst)
                .with_context(|| format!("linking {}", dst.display()))?,
        }
    }
    Ok(conflicts)
}
