use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex, OnceLock};
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
    pub ignored_words: Vec<String>,

    #[serde(default = "default_use_selection_replace")]
    pub use_selection_replace: bool,

    #[serde(default)]
    pub window_x: Option<i32>,
    #[serde(default)]
    pub window_y: Option<i32>,
    #[serde(default)]
    pub window_width: Option<u32>,
    #[serde(default)]
    pub window_height: Option<u32>,

    /// User-confirmed layout corrections: maps EN key sequence (lowercase) →
    /// Windows LANGID of the preferred target language.  Populated automatically
    /// when the user force-switches a word.  Checked before the statistical model
    /// so the user's explicit preference always wins.
    #[serde(default)]
    pub word_corrections: HashMap<String, u16>,

    /// Per-word success counts for the adaptive whitelisting mechanism.
    /// When a word is typed N times without triggering a switch, it is added to
    /// `ignored_words`.  This map persists counts across restarts so the
    /// threshold is cumulative, not per-session.
    #[serde(default)]
    pub adaptive_counts: HashMap<String, u32>,
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
            ignored_words: Vec::new(),
            use_selection_replace: false,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
            word_corrections: HashMap::new(),
            adaptive_counts: HashMap::new(),
        }
    }
}

fn bool_true() -> bool { true }
fn default_hotkey_vk() -> u16 { 0x10 }
fn default_undo_hotkey_vk() -> u16 { 0x08 }
fn default_lang() -> String { "en".to_string() }
fn default_sensitivity() -> f32 { 1.0 }
fn default_use_selection_replace() -> bool { false }

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

/// Serializes all writes to `config.json` so concurrent saves from the hook
/// thread, IPC commands, and window events can never interleave on disk.
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// Channel to the background persistence worker.  Lets the hot path (the
/// keyboard hook) request a save without blocking on disk I/O and without
/// spawning a fresh thread per keystroke.
static SAVE_TX: OnceLock<mpsc::Sender<Settings>> = OnceLock::new();

/// Spawn the persistence worker.  Must be called once from `main()` after
/// `SETTINGS` is initialised and before the hook thread starts.
pub fn init_persistence() {
    let (tx, rx) = mpsc::channel::<Settings>();
    if SAVE_TX.set(tx).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("rswitcher-persist".into())
        .spawn(move || {
            // Coalesce bursts: block for one request, then drain any queued
            // requests and persist only the most recent state.
            while let Ok(mut latest) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    latest = newer;
                }
                save(&latest);
            }
        });
}

/// Request an asynchronous save from the hot path (keyboard hook).  Never
/// blocks on disk I/O.  Falls back to a synchronous save if the worker is not
/// running yet.
pub fn save_async(s: &Settings) {
    match SAVE_TX.get() {
        Some(tx) => {
            let _ = tx.send(s.clone());
        }
        None => save(s),
    }
}

/// Persist settings to disk atomically (write to a temp file, then rename over
/// the target), serialised through `FILE_LOCK`.  Safe to call from any thread.
pub fn save(s: &Settings) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log_error!("Failed to create config directory: {:?}", e);
            return;
        }
    }

    let json = match serde_json::to_string_pretty(s) {
        Ok(json) => json,
        Err(e) => {
            log_error!("Failed to serialize settings: {:?}", e);
            return;
        }
    };

    // Recover from a poisoned lock: the protected data is just the filesystem,
    // so a previous panic mid-write does not leave shared state inconsistent.
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, &json) {
        log_error!("Failed to write temp config file: {:?}", e);
        return;
    }
    // `std::fs::rename` replaces the destination atomically on Windows.
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log_error!("Failed to replace config file: {:?}", e);
        let _ = std::fs::remove_file(&tmp);
    }
}
