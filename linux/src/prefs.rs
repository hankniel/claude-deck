//! App-only preferences (not part of claude-deck's config.toml).

use claude_deck::config::config_dir;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

fn path() -> PathBuf {
    config_dir().join("cc-deck-linux.json")
}

fn load() -> serde_json::Value {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}))
}

/// A remembered UI choice (Jira board, mode…).
pub fn get(key: &str) -> Option<String> {
    load().get(key).and_then(|v| v.as_str()).map(String::from)
}

pub fn set(key: &str, value: &str) {
    let mut v = load();
    if !v.is_object() {
        v = serde_json::json!({});
    }
    v[key] = serde_json::json!(value);
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(path(), v.to_string());
}

pub fn appearance() -> Appearance {
    let v = load();
    match v.get("appearance").and_then(|a| a.as_str()) {
        Some("light") => Appearance::Light,
        Some("dark") => Appearance::Dark,
        _ => Appearance::System,
    }
}

pub fn set_appearance(a: Appearance) {
    let s = match a {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    };
    set("appearance", s);
}

pub fn apply_appearance(a: Appearance) {
    adw::StyleManager::default().set_color_scheme(match a {
        Appearance::System => adw::ColorScheme::Default,
        Appearance::Light => adw::ColorScheme::ForceLight,
        Appearance::Dark => adw::ColorScheme::ForceDark,
    });
}
