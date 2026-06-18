use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "bool_true")]
    pub enabled: bool,

    #[serde(default)]
    pub exceptions: Vec<String>,

    /// Hotkey for forced (manual) layout switch of the current word.
    /// Default: Win+Shift (0x10 = VK_SHIFT, hotkey_win = true).
    #[serde(default)]
    pub hotkey_enabled: bool,
    #[serde(default = "default_hotkey_vk")]
    pub hotkey_vk: u16,
    /// Require Win modifier for the force-switch hotkey.
    #[serde(default)]
    pub hotkey_win: bool,

    /// Hotkey to undo the last automatic switch and restore the original word.
    /// Default: Win+Backspace (0x08 = VK_BACK, undo_hotkey_win = true).
    #[serde(default)]
    pub undo_hotkey_enabled: bool,
    #[serde(default = "default_undo_hotkey_vk")]
    pub undo_hotkey_vk: u16,
    /// Require Win modifier for the undo hotkey.
    #[serde(default)]
    pub undo_hotkey_win: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            exceptions: Vec::new(),
            hotkey_enabled: false,
            hotkey_vk: 0x10,   // VK_SHIFT
            hotkey_win: true,
            undo_hotkey_enabled: false,
            undo_hotkey_vk: 0x08, // VK_BACK
            undo_hotkey_win: true,
        }
    }
}

fn bool_true() -> bool { true }
fn default_hotkey_vk() -> u16 { 0x10 }
fn default_undo_hotkey_vk() -> u16 { 0x08 }

// ── Persistence ───────────────────────────────────────────────────────────────

pub fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("rswitcher").join("config.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(&path, json);
    }
}
