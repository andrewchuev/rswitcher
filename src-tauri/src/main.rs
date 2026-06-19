#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod bigrams;
mod buffer;
mod exceptions;
mod layout;
pub mod logger;
mod settings;
mod switcher;

use std::cell::RefCell;
use std::sync::{Arc, RwLock, OnceLock};

use windows::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyboardLayout, GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_LWIN,
            VK_RETURN, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
            GetWindowThreadProcessId, SetWindowsHookExW,
            TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, KBDLLHOOKSTRUCT_FLAGS,
            LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
            MessageBoxW, MB_OK, MB_ICONWARNING,
        },
    },
};

use tauri::Manager;
pub use settings::Settings;

// ─────────────────────────────────────────────────────────────────────────────
// Tray icon language variants
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangIcon { Ru, En }

fn make_lang_icon(lang: LangIcon, dimmed: bool) -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let bytes: &[u8] = match lang {
        LangIcon::Ru => include_bytes!("../../assets/ru.raw"),
        LangIcon::En => include_bytes!("../../assets/en.raw"),
    };
    let mut rgba = bytes.to_vec();
    if dimmed {
        for px in rgba.chunks_mut(4) {
            px[0] = (px[0] as u32 * 35 / 100) as u8;
            px[1] = (px[1] as u32 * 35 / 100) as u8;
            px[2] = (px[2] as u32 * 35 / 100) as u8;
        }
    }
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

/// Spawn a background thread that updates the language icon directly.
fn spawn_tray_watcher(app_handle: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("rswitcher-tray".into())
        .spawn(move || {
            let mut last_icon: Option<(LangIcon, bool)> = None;
            loop {
                let lang_word = unsafe { foreground_lang() };
                let new_lang = if layout::hkl_is_russian(lang_word) {
                    Some(LangIcon::Ru)
                } else if layout::hkl_is_english(lang_word) {
                    Some(LangIcon::En)
                } else {
                    None
                };

                let dimmed = SETTINGS
                    .get()
                    .and_then(|s| s.try_read().ok())
                    .map(|s| {
                        !s.exceptions.is_empty()
                            && exceptions::foreground_exe_name()
                                .map(|name| s.exceptions.iter().any(|e| *e == name))
                                .unwrap_or(false)
                    })
                    .unwrap_or(false);

                let new_state = new_lang.map(|l| (l, dimmed));
                if new_state.is_some() && new_state != last_icon {
                    if let Some((lang, dim)) = new_state {
                        if let Some(tray) = app_handle.tray_by_id("main-tray") {
                            let _ = tray.set_icon(Some(make_lang_icon(lang, dim)));
                        }
                    }
                    last_icon = new_state;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .expect("failed to spawn tray watcher thread");
}

// ─────────────────────────────────────────────────────────────────────────────
// Process-global settings handle (hook_proc is a bare fn pointer, no captures)
// ─────────────────────────────────────────────────────────────────────────────

static SETTINGS: OnceLock<Arc<RwLock<Settings>>> = OnceLock::new();
static TRAY_SHOW_ITEM: OnceLock<tauri::menu::MenuItem<tauri::Wry>> = OnceLock::new();
static TRAY_QUIT_ITEM: OnceLock<tauri::menu::MenuItem<tauri::Wry>> = OnceLock::new();

// ─────────────────────────────────────────────────────────────────────────────
// Hook-thread local storage
// ─────────────────────────────────────────────────────────────────────────────

thread_local! {
    static WORD_BUF: RefCell<buffer::WordBuffer> =
        RefCell::new(buffer::WordBuffer::new());

    static PREV_WORD_BUF: RefCell<Option<(buffer::WordBuffer, VIRTUAL_KEY)>> =
        const { RefCell::new(None) };

    static UNDO: RefCell<Option<UndoState>> = const { RefCell::new(None) };
}

struct UndoState {
    original_word: String,
    erase_len: usize,
    restore_to_ru: bool,
    boundary_vk: Option<VIRTUAL_KEY>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Low-level keyboard hook
// ─────────────────────────────────────────────────────────────────────────────

unsafe extern "system" fn hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code < 0 {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);

    if kb.flags & LLKHF_INJECTED != KBDLLHOOKSTRUCT_FLAGS(0) {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let is_down = matches!(
        w_param.0 as u32,
        w if w == WM_KEYDOWN || w == WM_SYSKEYDOWN
    );
    if !is_down {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let (enabled, excluded, excluded_name, hotkey, undo_hotkey) = SETTINGS
        .get()
        .and_then(|s| s.try_read().ok())
        .map(|s| {
            let excluded_name = if s.exceptions.is_empty() {
                None
            } else {
                exceptions::foreground_exe_name()
                    .filter(|name| s.exceptions.iter().any(|e| e == name))
            };
            let excluded = excluded_name.is_some();
            let hotkey = s.hotkey_enabled.then_some((VIRTUAL_KEY(s.hotkey_vk), s.hotkey_win));
            let undo_hotkey = s.undo_hotkey_enabled.then_some((VIRTUAL_KEY(s.undo_hotkey_vk), s.undo_hotkey_win));
            (s.enabled, excluded, excluded_name, hotkey, undo_hotkey)
        })
        .unwrap_or((false, false, None, None, None));

    if !enabled || excluded {
        if excluded {
            if let Some(name) = excluded_name {
                WORD_BUF.with(|c| {
                    let mut buf = c.borrow_mut();
                    if !buf.is_empty() {
                        log_info!("[EXCLUDED] foreground={} — buffer cleared", name);
                        buf.clear();
                    }
                });
            }
        }
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let vk = VIRTUAL_KEY(kb.vkCode as u16);
    let swallow = WORD_BUF.with(|cell| {
        let mut buf = cell.borrow_mut();
        process_key(vk, &mut buf, hotkey, undo_hotkey)
    });

    if swallow {
        LRESULT(1)
    } else {
        CallNextHookEx(None, n_code, w_param, l_param)
    }
}

unsafe fn foreground_lang() -> u16 {
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return 0;
    }
    let tid = GetWindowThreadProcessId(hwnd, None);
    let hkl = GetKeyboardLayout(tid);
    (hkl.0 as usize) as u16
}

fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk,
        0x5B | 0x5C           // VK_LWIN, VK_RWIN
        | 0x10 | 0xA0 | 0xA1 // VK_SHIFT, VK_LSHIFT, VK_RSHIFT
        | 0x11 | 0xA2 | 0xA3 // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
        | 0x12 | 0xA4 | 0xA5 // VK_MENU, VK_LMENU, VK_RMENU
    )
}

unsafe fn key_is_held(configured: u16) -> bool {
    let held = |vk: i32| GetAsyncKeyState(vk) < 0;
    match configured {
        0x10 => held(0xA0) || held(0xA1), // VK_SHIFT
        0x11 => held(0xA2) || held(0xA3), // VK_CONTROL
        0x12 => held(0xA4) || held(0xA5), // VK_MENU
        v    => held(v as i32),
    }
}

fn vk_matches(actual: u16, configured: u16) -> bool {
    match configured {
        0x10 => matches!(actual, 0xA0 | 0xA1), // VK_SHIFT
        0x11 => matches!(actual, 0xA2 | 0xA3), // VK_CONTROL
        0x12 => matches!(actual, 0xA4 | 0xA5), // VK_MENU
        _ => actual == configured,
    }
}

unsafe fn process_key(
    vk: VIRTUAL_KEY,
    buf: &mut buffer::WordBuffer,
    hotkey: Option<(VIRTUAL_KEY, bool)>,
    undo_hotkey: Option<(VIRTUAL_KEY, bool)>,
) -> bool {
    let win_held = GetAsyncKeyState(VK_LWIN.0 as i32) < 0
        || GetAsyncKeyState(VK_RWIN.0 as i32) < 0;

    log_debug!(
        "[KEY] vk={:#04x} win={} buf={}",
        vk.0, win_held as u8, buf.len()
    );

    let is_modifier = matches!(vk.0,
        0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 | 0x5B | 0x5C | 0x10 | 0x11 | 0x12
    );
    let is_hotkey_vk = if let Some((hk_vk, _)) = hotkey { vk.0 == hk_vk.0 } else { false };
    let is_undo_vk = if let Some((uh_vk, _)) = undo_hotkey { vk.0 == uh_vk.0 } else { false };

    if !is_modifier && !is_hotkey_vk && !is_undo_vk {
        PREV_WORD_BUF.with(|p| *p.borrow_mut() = None);
    }

    let hotkey_fires = |vk: VIRTUAL_KEY, hk_vk: VIRTUAL_KEY, hk_win: bool| -> bool {
        let is_win_vk = vk.0 == VK_LWIN.0 || vk.0 == VK_RWIN.0;
        if hk_win {
            let order_a = vk_matches(vk.0, hk_vk.0) && win_held;
            let order_b = is_win_vk && key_is_held(hk_vk.0);
            order_a || order_b
        } else {
            vk_matches(vk.0, hk_vk.0) && !win_held
        }
    };

    if let Some((uh_vk, uh_win)) = undo_hotkey {
        if hotkey_fires(vk, uh_vk, uh_win) {
            let state = UNDO.with(|u| u.borrow_mut().take());
            if let Some(s) = state {
                log_info!(
                    "[UNDO] restoring {:?}, erase={}, restore_to_ru={}",
                    s.original_word, s.erase_len, s.restore_to_ru
                );
                let undo_action = buffer::SwitchAction {
                    backspaces: s.erase_len,
                    new_word: s.original_word,
                    to_ru: s.restore_to_ru,
                    original_word: String::new(),
                };
                switcher::perform_switch(&undo_action, s.boundary_vk);
            }
            buf.clear();
            if uh_win {
                switcher::suppress_start_menu();
            }
            return true;
        }
    }

    if let Some((hk_vk, hk_win)) = hotkey {
        if hotkey_fires(vk, hk_vk, hk_win) {
            let lang = foreground_lang();
            let snap = buf.detection_snapshot();
            if let Some(action) = buf.force_switch(lang) {
                if let Some(ref s) = snap {
                    log_info!(
                        "[FORCE] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} → {}→{}",
                        lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                        if action.to_ru { "EN" } else { "RU" },
                        if action.to_ru { "RU" } else { "EN" },
                    );
                }
                save_undo(&action, None);
                buf.clear();
                PREV_WORD_BUF.with(|p| *p.borrow_mut() = None);
                switcher::perform_switch(&action, None);
            } else {
                let prev = PREV_WORD_BUF.with(|p| p.borrow_mut().take());
                if let Some((prev_buf, boundary_vk)) = prev {
                    let prev_snap = prev_buf.detection_snapshot();
                    if let Some(action) = prev_buf.force_switch(lang) {
                        if let Some(ref s) = prev_snap {
                            log_info!(
                                "[FORCE-PREV] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} → {}→{} boundary={:#04x}",
                                lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                                if action.to_ru { "EN" } else { "RU" },
                                if action.to_ru { "RU" } else { "EN" },
                                boundary_vk.0
                            );
                        }
                        let mut force_action = action.clone();
                        force_action.backspaces += 1;
                        save_undo(&force_action, Some(boundary_vk));
                        buf.clear();
                        switcher::perform_switch(&force_action, Some(boundary_vk));
                    }
                } else {
                    buf.clear();
                }
            }
            if hk_win {
                switcher::suppress_start_menu();
            }
            UNDO.with(|u| u.borrow_mut().take());
            return true;
        }
    }

    match vk {
        VK_BACK => {
            buf.pop();
            false
        }

        VK_SPACE | VK_RETURN | VK_TAB => {
            let lang = foreground_lang();
            let snap = buf.detection_snapshot();
            let result = buf.detect_mismatch(lang);

            if let Some(ref s) = snap {
                match &result {
                    Some(action) => log_info!(
                        "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → SWITCH_{} boundary={:#04x}",
                        lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                        s.score_en - s.score_ru,
                        if action.to_ru { "EN→RU" } else { "RU→EN" },
                        vk.0
                    ),
                    None => {
                        let reason = if s.len == 1 {
                            "skip (single-char)".to_string()
                        } else if s.len == 2 || s.len == 3 {
                            "skip (dictionary)".to_string()
                        } else {
                            format!("skip (threshold={:.1})", buffer::switching_threshold(s.len))
                        };
                        log_info!(
                            "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → {}",
                            lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                            s.score_en - s.score_ru,
                            reason
                        );
                    }
                }
            }

            if let Some(action) = result {
                save_undo(&action, Some(vk));
                buf.clear();
                PREV_WORD_BUF.with(|p| *p.borrow_mut() = None);
                switcher::perform_switch(&action, Some(vk));
                true
            } else {
                if !buf.is_empty() {
                    PREV_WORD_BUF.with(|p| *p.borrow_mut() = Some((buf.clone(), vk)));
                }
                buf.clear();
                UNDO.with(|u| *u.borrow_mut() = None);
                false
            }
        }

        _ => {
            if layout::is_translatable_vk(vk.0) {
                let ctrl_held = GetAsyncKeyState(0xA2_i32) < 0  // VK_LCONTROL
                    || GetAsyncKeyState(0xA3_i32) < 0;           // VK_RCONTROL
                if ctrl_held {
                    buf.clear();
                    UNDO.with(|u| *u.borrow_mut() = None);
                } else {
                    let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                    let caps = GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0;
                    buf.push(vk.0, shift ^ caps);
                    UNDO.with(|u| *u.borrow_mut() = None);
                }
            } else if is_modifier_vk(vk.0) {
                // Keep buffer intact
            } else {
                let lang = foreground_lang();
                let snap = buf.detection_snapshot();
                let result = buf.detect_mismatch(lang);
                if let Some(ref s) = snap {
                    match &result {
                        Some(action) => log_info!(
                            "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → SWITCH_{} boundary=vk{:#04x}",
                            lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                            s.score_en - s.score_ru,
                            if action.to_ru { "EN→RU" } else { "RU→EN" },
                            vk.0
                        ),
                        None => {
                            let reason = if s.len == 1 {
                                "skip (single-char)".to_string()
                            } else if s.len == 2 || s.len == 3 {
                                "skip (dictionary)".to_string()
                            } else {
                                format!("skip (threshold={:.1})", buffer::switching_threshold(s.len))
                            };
                            log_info!(
                                "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → {}",
                                lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                                s.score_en - s.score_ru,
                                reason
                            );
                        }
                    }
                }
                if let Some(action) = result {
                    save_undo(&action, None);
                    buf.clear();
                    switcher::perform_switch(&action, None);
                } else {
                    buf.clear();
                    UNDO.with(|u| *u.borrow_mut() = None);
                }
            }
            false
        }
    }
}

fn save_undo(action: &buffer::SwitchAction, boundary_vk: Option<VIRTUAL_KEY>) {
    UNDO.with(|u| {
        *u.borrow_mut() = Some(UndoState {
            original_word: action.original_word.clone(),
            erase_len: action.new_word.chars().count() + if boundary_vk.is_some() { 1 } else { 0 },
            restore_to_ru: !action.to_ru,
            boundary_vk,
        });
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Hook thread (owns the message loop required by WH_KEYBOARD_LL)
// ─────────────────────────────────────────────────────────────────────────────

fn start_hook_thread() -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("rswitcher-hook".into())
        .spawn(|| {
            let hook = unsafe {
                let hmod = GetModuleHandleW(None).expect("GetModuleHandleW failed");
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), HINSTANCE(hmod.0), 0)
                    .expect("SetWindowsHookExW failed")
            };
            log_info!("[hook] installed");

            let mut msg = MSG::default();
            loop {
                let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                if ret.0 <= 0 {
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            unsafe { UnhookWindowsHookEx(hook).ok() };
            log_info!("[hook] uninstalled");
        })
        .expect("failed to spawn hook thread")
}

fn vk_display_name(vk: u16, win: bool) -> String {
    let base = match vk {
        0x10 | 0xA0 | 0xA1 => "Shift",
        0x08 => "Backspace",
        0x13 => "Pause",
        0x91 => "Scroll Lock",
        0x14 => "Caps Lock",
        0x7B => "F12",
        0x7A => "F11",
        0x79 => "F10",
        0x78 => "F9",
        _ => "Custom",
    };
    if win { format!("Win+{}", base) } else { base.to_string() }
}

fn check_single_instance() -> bool {
    unsafe {
        let name = windows::core::w!("Global\\RSwitcher_SingleInstance_Mutex_98a72b");
        let result: Result<windows::Win32::Foundation::HANDLE, _> =
            CreateMutexW(None, windows::Win32::Foundation::BOOL(1), name);
        match result {
            Ok(handle) => {
                if handle.is_invalid() {
                    return false;
                }
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(handle);
                    false
                } else {
                    true
                }
            }
            Err(_) => false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings() -> Settings {
    let s = SETTINGS.get().unwrap().read().unwrap();
    s.clone()
}

fn update_tray_menu_language(lang: &str) {
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
fn save_settings(settings: Settings) -> Settings {
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

    settings::save(&s);
    if lang_changed {
        update_tray_menu_language(&s.lang);
    }
    s.clone()
}

#[tauri::command]
fn get_running_apps() -> Vec<exceptions::RunningApp> {
    exceptions::enumerate_visible_apps()
}

#[tauri::command]
fn open_config_dir() {
    let _ = std::process::Command::new("explorer")
        .arg(settings::config_path())
        .spawn();
}

#[tauri::command]
fn add_exception(app: String) -> Settings {
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
fn remove_exception(index: usize) -> Settings {
    let settings_arc = SETTINGS.get().unwrap();
    let mut s = settings_arc.write().unwrap();
    if index < s.exceptions.len() {
        s.exceptions.remove(index);
        settings::save(&s);
    }
    s.clone()
}

#[tauri::command]
fn set_enabled(enabled: bool) -> Settings {
    let settings_arc = SETTINGS.get().unwrap();
    let mut s = settings_arc.write().unwrap();
    s.enabled = enabled;
    settings::save(&s);
    s.clone()
}

#[tauri::command]
fn set_autostart(enabled: bool) {
    autostart::set_enabled(enabled);
}

#[tauri::command]
fn is_autostart_enabled() -> bool {
    autostart::is_enabled()
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    if !check_single_instance() {
        unsafe {
            let _ = MessageBoxW(
                HWND::default(),
                windows::core::w!("Приложение RSwitcher уже запущено и работает в системном трее."),
                windows::core::w!("RSwitcher"),
                MB_OK | MB_ICONWARNING,
            );
        }
        std::process::exit(0);
    }

    logger::init();

    let settings = Arc::new(RwLock::new(settings::load()));
    SETTINGS
        .set(Arc::clone(&settings))
        .expect("SETTINGS already initialised");

    {
        let s = settings.read().unwrap();
        log_info!(
            "=== RSwitcher started (pid={}, path={:?}) ===",
            std::process::id(),
            std::env::current_exe().ok()
        );
        log_info!(
            "settings: enabled={} exceptions={:?} hotkey={} undo_hotkey={}",
            s.enabled,
            s.exceptions,
            if s.hotkey_enabled {
                format!("{} ({:#04x})", vk_display_name(s.hotkey_vk, s.hotkey_win), s.hotkey_vk)
            } else {
                "off".into()
            },
            if s.undo_hotkey_enabled {
                format!("{} ({:#04x})", vk_display_name(s.undo_hotkey_vk, s.undo_hotkey_win), s.undo_hotkey_vk)
            } else {
                "off".into()
            },
        );
    }

    let _hook_thread = start_hook_thread();

    tauri::Builder::default()
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
                            std::process::exit(0);
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

            // 3. Restore window size and position from saved settings
            let window = app.get_webview_window("main").unwrap();
            let (saved_x, saved_y, saved_w, saved_h) = {
                let s = settings_arc.read().unwrap();
                (s.window_x, s.window_y, s.window_width, s.window_height)
            };
            if let (Some(x), Some(y)) = (saved_x, saved_y) {
                let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)));
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
            is_autostart_enabled
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
