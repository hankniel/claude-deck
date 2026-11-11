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

pub fn appearance() -> Appearance {
    let v: serde_json::Value = std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
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
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(path(), serde_json::json!({ "appearance": s }).to_string());
}

pub fn apply_appearance(a: Appearance) {
    adw::StyleManager::default().set_color_scheme(match a {
        Appearance::System => adw::ColorScheme::Default,
        Appearance::Light => adw::ColorScheme::ForceLight,
        Appearance::Dark => adw::ColorScheme::ForceDark,
    });
}
