use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::log_error;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_lang")]
    pub lang: String,
    
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,

    #[serde(default)]
    pub window_x: Option<i32>,
    #[serde(default)]
    pub window_y: Option<i32>,
    #[serde(default)]
    pub window_width: Option<u32>,
    #[serde(default)]
    pub window_height: Option<u32>,
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
            lang: "en".to_string(),
            sensitivity: 1.0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        }
    }
}

fn bool_true() -> bool { true }
fn default_hotkey_vk() -> u16 { 0x10 }
fn default_undo_hotkey_vk() -> u16 { 0x08 }
fn default_lang() -> String { "en".to_string() }
fn default_sensitivity() -> f32 { 1.0 }

// ── Persistence ───────────────────────────────────────────────────────────────

pub fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("rswitcher").join("config.json")
}

pub fn load() -> Settings {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(settings) => settings,
            Err(e) => {
                log_error!("Failed to parse config file: {:?}. Using defaults.", e);
                Settings::default()
            }
        },
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log_error!("Failed to read config file: {:?}. Using defaults.", e);
            }
            Settings::default()
        }
    }
}

pub fn save(s: &Settings) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log_error!("Failed to create config directory: {:?}", e);
            return;
        }
    }
    match serde_json::to_string_pretty(s) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log_error!("Failed to write config file: {:?}", e);
            }
        }
        Err(e) => {
            log_error!("Failed to serialize settings: {:?}", e);
        }
    }
}
