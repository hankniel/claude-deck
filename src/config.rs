use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is not set"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("claude-deck")
}

/// Where profiles, usage history and the event log live.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("claude-deck")
}

pub fn profiles_dir() -> PathBuf {
    data_dir().join("profiles")
}

/// The default Claude Code config dir (used when CLAUDE_CONFIG_DIR is unset).
pub fn default_claude_dir() -> PathBuf {
    home().join(".claude")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Switch away when the 5-hour window reaches this percent.
    pub threshold: f64,
    /// Switch away when the 7-day window reaches this percent.
    pub weekly_threshold: f64,
    /// Automatically move idle sessions to another account when over a threshold.
    pub auto_switch: bool,
    /// A session counts as idle after this many seconds without output or input.
    pub idle_secs: u64,
    /// Minimum seconds between two automatic switches of the same tab.
    pub cooldown_secs: u64,
    /// How often usage is polled.
    pub poll_secs: u64,
    /// The Claude Code executable.
    pub claude_bin: String,
    /// Items under ~/.claude that every profile symlinks to (settings, skills, history...).
    pub shared: Vec<String>,
    /// Accounts never chosen automatically.
    pub disabled: Vec<String>,
    /// Directory -> account. Subdirectories inherit the nearest mapped ancestor.
    pub maps: BTreeMap<String, String>,
    /// Recently used session directories, newest first (desktop app).
    pub recent: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            threshold: 90.0,
            weekly_threshold: 95.0,
            auto_switch: false,
            idle_secs: 8,
            cooldown_secs: 300,
            poll_secs: 120,
            claude_bin: "claude".into(),
            shared: [
                "settings.json",
                "CLAUDE.md",
                "skills",
                "agents",
                "commands",
                "plugins",
                "hooks",
                "output-styles",
                "keybindings.json",
                "projects",
                "history.jsonl",
                "todos",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            disabled: vec![],
            maps: BTreeMap::new(),
            recent: vec![],
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> Result<Config> {
        let p = Self::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(config_dir())?;
        let tmp = Self::path().with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, Self::path())?;
        Ok(())
    }

    /// The account mapped to `dir` or its nearest mapped ancestor.
    pub fn account_for(&self, dir: &Path) -> Option<String> {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        dir.ancestors()
            .find_map(|a| self.maps.get(a.to_string_lossy().as_ref()).cloned())
    }

    pub fn push_recent(&mut self, dir: &Path) {
        let d = dir.to_string_lossy().to_string();
        self.recent.retain(|r| r != &d);
        self.recent.insert(0, d);
        self.recent.truncate(8);
    }

    /// Set a scalar setting by name, e.g. `threshold 80` or `auto_switch true`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let mut table = toml::Value::try_from(&*self)?;
        let t = table.as_table_mut().unwrap();
        let old = match t.get(key) {
            Some(v) if !v.is_table() && !v.is_array() => v.clone(),
            _ => bail!("unknown or non-scalar setting: {key}"),
        };
        let new = match old {
            toml::Value::Boolean(_) => toml::Value::Boolean(value.parse()?),
            toml::Value::Integer(_) => toml::Value::Integer(value.parse()?),
            toml::Value::Float(_) => toml::Value::Float(value.parse()?),
            _ => toml::Value::String(value.to_string()),
        };
        t.insert(key.to_string(), new);
        *self = table.try_into()?;
        Ok(())
    }
}
