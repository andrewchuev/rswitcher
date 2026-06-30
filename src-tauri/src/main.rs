#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod bigrams;
mod buffer;
mod commands;
mod exceptions;
mod globals;
mod hook;
mod layout;
pub mod logger;
mod settings;
mod switcher;
mod tray;
mod benchmark;

use std::sync::{Arc, RwLock};
use tauri::Manager;

use globals::{SETTINGS, TRAY_QUIT_ITEM, TRAY_SHOW_ITEM};
use tray::{foreground_lang, make_lang_icon, LangIcon, spawn_tray_watcher};
use hook::start_hook_thread;

use commands::{
    get_settings, save_settings, get_running_apps, open_config_dir,
    add_exception, remove_exception, set_enabled, set_autostart, is_autostart_enabled,
    is_elevated, restart_as_admin, open_url, get_app_version,
};

fn is_position_visible(x: i32, y: i32, app: &tauri::App) -> bool {
    if let Ok(monitors) = app.available_monitors() {
        for monitor in monitors {
            let pos = monitor.position();
            let size = monitor.size();
            let monitor_left = pos.x;
            let monitor_top = pos.y;
            let monitor_right = pos.x + size.width as i32;
            let monitor_bottom = pos.y + size.height as i32;
            if x >= monitor_left && x < monitor_right && y >= monitor_top && y < monitor_bottom {
                return true;
            }
        }
    }
    false
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "benchmark" || args[1] == "--benchmark") {
        benchmark::run();
        return;
    }

    logger::init();
    logger::setup_panic_hook();

    let settings = Arc::new(RwLock::new(settings::load()));
    SETTINGS
        .set(Arc::clone(&settings))
        .expect("SETTINGS already initialised");

    // Start the persistence worker so hot-path saves (keyboard hook) never
    // block on disk I/O and all writes are serialised atomically.
    settings::init_persistence();

    {
        let s = settings.read().unwrap();
        log_info!(
            "=== RSwitcher started (pid={}, path={:?}) ===",
            std::process::id(),
            std::env::current_exe().ok()
        );
        log_info!("OS: {}", logger::get_windows_version());
        logger::log_keyboard_layouts();
        log_info!(
            "settings: enabled={} exceptions={:?} ignored_words_count={} sensitivity={:.1} use_selection_replace={}",
            s.enabled,
            s.exceptions,
            s.ignored_words.len(),
            s.sensitivity,
            s.use_selection_replace
        );
    }

    let _hook_thread = start_hook_thread();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 1. Spawning language icon tray watcher
            spawn_tray_watcher(app.handle().clone());

            // 2. Creating tray icon natively via MenuBuilder & TrayIconBuilder
            let settings_arc = SETTINGS.get().unwrap();
            let lang = {
                let s = settings_arc.read().unwrap();
                s.lang.clone()
            };
            
            let show_title = match lang.as_str() {
                "ru" => "Настройки",
                "uk" => "Налаштування",
                _ => "Settings",
            };
            let quit_title = match lang.as_str() {
                "ru" => "Выход",
                "uk" => "Вихід",
                _ => "Exit",
            };

            let show_item = tauri::menu::MenuItemBuilder::with_id("show", show_title).build(app)?;
            let quit_item = tauri::menu::MenuItemBuilder::with_id("quit", quit_title).build(app)?;

            let _ = TRAY_SHOW_ITEM.set(show_item.clone());
            let _ = TRAY_QUIT_ITEM.set(quit_item.clone());
            
            let menu = tauri::menu::MenuBuilder::new(app)
                .items(&[&show_item, &tauri::menu::PredefinedMenuItem::separator(app)?, &quit_item])
                .build()?;

            let initial_lang = {
                let w = unsafe { foreground_lang() };
                if layout::hkl_is_russian(w) { LangIcon::Ru } else { LangIcon::En }
            };

            let _tray = tauri::tray::TrayIconBuilder::with_id("main-tray")
                .tooltip("RSwitcher")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .icon(make_lang_icon(initial_lang, false))
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "quit" => {
                            log_info!("=== RSwitcher quit via tray menu ===");
                            let settings_arc = SETTINGS.get().unwrap();
                            if let Ok(s) = settings_arc.read() {
                                settings::save(&s);
                            }
                            app.exit(0);
                        }
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 3. Restore window size and position from saved settings (with monitor bounds check)
            let window = app.get_webview_window("main").unwrap();
            let (saved_x, saved_y, saved_w, saved_h) = {
                let s = settings_arc.read().unwrap();
                (s.window_x, s.window_y, s.window_width, s.window_height)
            };
            if let (Some(x), Some(y)) = (saved_x, saved_y) {
                if is_position_visible(x, y, app) {
                    let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)));
                } else {
                    log_info!("Saved window position ({}, {}) is off-screen. Centering window.", x, y);
                    let _ = window.center();
                }
            }
            if let (Some(w), Some(h)) = (saved_w, saved_h) {
                let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(w, h)));
            }

            // 4. Handle window events: save size and position in memory, save to disk on hide
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                match event {
                    tauri::WindowEvent::Moved(pos) => {
                        let settings_arc = SETTINGS.get().unwrap();
                        if let Ok(mut s) = settings_arc.write() {
                            s.window_x = Some(pos.x);
                            s.window_y = Some(pos.y);
                        }
                    }
                    tauri::WindowEvent::Resized(size) => {
                        let settings_arc = SETTINGS.get().unwrap();
                        if let Ok(mut s) = settings_arc.write() {
                            s.window_width = Some(size.width);
                            s.window_height = Some(size.height);
                        }
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        
                        // Save latest window state to disk before hiding
                        let settings_arc = SETTINGS.get().unwrap();
                        if let Ok(s) = settings_arc.read() {
                            settings::save(&s);
                        }
                        
                        let _ = window_clone.hide();
                    }
                    _ => {}
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_running_apps,
            open_config_dir,
            add_exception,
            remove_exception,
            set_enabled,
            set_autostart,
            is_autostart_enabled,
            is_elevated,
            restart_as_admin,
            open_url,
            get_app_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
