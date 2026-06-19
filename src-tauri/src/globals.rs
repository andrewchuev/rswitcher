use std::sync::{Arc, RwLock, OnceLock};
use crate::settings::Settings;

pub static SETTINGS: OnceLock<Arc<RwLock<Settings>>> = OnceLock::new();
pub static TRAY_SHOW_ITEM: OnceLock<tauri::menu::MenuItem<tauri::Wry>> = OnceLock::new();
pub static TRAY_QUIT_ITEM: OnceLock<tauri::menu::MenuItem<tauri::Wry>> = OnceLock::new();
