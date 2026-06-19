use crate::globals::{SETTINGS, TRAY_QUIT_ITEM, TRAY_SHOW_ITEM};
use crate::settings::{self, Settings};
use crate::{autostart, exceptions};

#[tauri::command]
pub fn get_settings() -> Settings {
    let s = SETTINGS.get().unwrap().read().unwrap();
    s.clone()
}

pub fn update_tray_menu_language(lang: &str) {
    let show_title = match lang {
        "ru" => "Настройки",
        "uk" => "Налаштування",
        _ => "Settings",
    };
    let quit_title = match lang {
        "ru" => "Выход",
        "uk" => "Вихід",
        _ => "Exit",
    };
    if let Some(item) = TRAY_SHOW_ITEM.get() {
        let _ = item.set_text(show_title);
    }
    if let Some(item) = TRAY_QUIT_ITEM.get() {
        let _ = item.set_text(quit_title);
    }
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> Settings {
    let settings_arc = SETTINGS.get().unwrap();
    let mut s = settings_arc.write().unwrap();
    let lang_changed = s.lang != settings.lang;
    
    // Copy frontend controlled fields
    s.enabled = settings.enabled;
    s.exceptions = settings.exceptions;
    s.hotkey_enabled = settings.hotkey_enabled;
    s.hotkey_vk = settings.hotkey_vk;
    s.hotkey_win = settings.hotkey_win;
    s.undo_hotkey_enabled = settings.undo_hotkey_enabled;
    s.undo_hotkey_vk = settings.undo_hotkey_vk;
    s.undo_hotkey_win = settings.undo_hotkey_win;
    s.lang = settings.lang;
    s.sensitivity = settings.sensitivity;

    settings::save(&s);
    if lang_changed {
        update_tray_menu_language(&s.lang);
    }
    s.clone()
}

#[tauri::command]
pub fn get_running_apps() -> Vec<exceptions::RunningApp> {
    exceptions::enumerate_visible_apps()
}

#[tauri::command]
pub fn open_config_dir(app: tauri::AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().reveal_item_in_dir(settings::config_path());
}

#[tauri::command]
pub fn add_exception(app: String) -> Settings {
    let settings_arc = SETTINGS.get().unwrap();
    let mut s = settings_arc.write().unwrap();
    let exc = app.trim().to_lowercase();
    if !exc.is_empty() && !s.exceptions.contains(&exc) {
        s.exceptions.push(exc);
        settings::save(&s);
    }
    s.clone()
}

#[tauri::command]
pub fn remove_exception(index: usize) -> Settings {
    let settings_arc = SETTINGS.get().unwrap();
    let mut s = settings_arc.write().unwrap();
    if index < s.exceptions.len() {
        s.exceptions.remove(index);
        settings::save(&s);
    }
    s.clone()
}

#[tauri::command]
pub fn set_enabled(enabled: bool) -> Settings {
    let settings_arc = SETTINGS.get().unwrap();
    let mut s = settings_arc.write().unwrap();
    s.enabled = enabled;
    settings::save(&s);
    s.clone()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) {
    autostart::set_enabled(enabled);
}

#[tauri::command]
pub fn is_autostart_enabled() -> bool {
    autostart::is_enabled()
}
