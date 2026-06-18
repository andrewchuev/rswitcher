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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use eframe::egui;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIconBuilder, TrayIconEvent,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,


    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyboardLayout, GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_LWIN,
            VK_RETURN, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
            GetWindowThreadProcessId, SetForegroundWindow, SetWindowsHookExW, ShowWindow,
            TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, KBDLLHOOKSTRUCT_FLAGS,
            LLKHF_INJECTED, MSG, SW_HIDE, SW_SHOW, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
        },
    },
};


pub use settings::Settings;

// ─────────────────────────────────────────────────────────────────────────────
// Tray watcher
// ─────────────────────────────────────────────────────────────────────────────

// Set to true when the user requests to show the window via the tray menu/double-click.
static WAS_SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);

// HWND of the main window, stored as usize so it can live in a static.
// Written once during window creation; read from the tray watcher thread.
static MAIN_HWND: AtomicUsize = AtomicUsize::new(0);

// ── Tray icon handle shared with the watcher thread ───────────────────────────
//
// eframe's update() is not called reliably for hidden windows, so we update
// the language icon directly from the background watcher thread instead.
//
// SAFETY: On Windows, Shell_NotifyIconW (called by TrayIcon::set_icon) is
// documented as thread-safe and does not require the caller to be on the
// window's owning thread.
struct SendableTray(tray_icon::TrayIcon);
unsafe impl Send for SendableTray {}

static TRAY_ICON: OnceLock<Mutex<SendableTray>> = OnceLock::new();

/// Spawn a background thread that owns all tray / menu event polling AND
/// updates the language icon directly (bypassing the eframe update loop).
fn spawn_tray_watcher(ctx: egui::Context, show_id: MenuId, quit_id: MenuId) {
    std::thread::Builder::new()
        .name("rswitcher-tray".into())
        .spawn(move || {
            // (lang, dimmed) — dimmed=true when the foreground app is in exceptions
            let mut last_icon: Option<(LangIcon, bool)> = None;
            loop {
                // ── Menu events ───────────────────────────────────────────────
                while let Ok(ev) = MenuEvent::receiver().try_recv() {
                    if ev.id == quit_id {
                        log_info!("=== RSwitcher quit via tray menu ===");
                        std::process::exit(0);
                    } else if ev.id == show_id {
                        show_main_window(&ctx);
                    }
                }
                // ── Tray icon events (double-click → show settings) ───────────
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if matches!(ev, TrayIconEvent::DoubleClick { .. }) {
                        show_main_window(&ctx);
                    }
                }
                // ── Language icon update ──────────────────────────────────────
                // Read the foreground window's layout directly here so the icon
                // updates regardless of whether eframe's update() is running.
                let lang_word = unsafe { foreground_lang() };
                let new_lang = if layout::hkl_is_russian(lang_word) {
                    Some(LangIcon::Ru)
                } else if layout::hkl_is_english(lang_word) {
                    Some(LangIcon::En)
                } else {
                    None
                };

                // Dim the icon when the foreground app is in the exceptions list.
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
                        if let Some(tray) = TRAY_ICON.get() {
                            if let Ok(t) = tray.lock() {
                                let _ = t.0.set_icon(Some(make_lang_icon(lang, dim)));
                            }
                        }
                    }
                    last_icon = new_state;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .expect("failed to spawn tray watcher thread");
}

/// Show and focus the main settings window.
///
/// ViewportCommand::Visible sent from outside eframe's update() is processed
/// between frames and may be discarded by egui's begin_frame() reset.  Instead
/// we use Win32 ShowWindow directly (guaranteed to work) and then signal
/// WAS_SHOW_REQUESTED so the next update() call renders the UI and sends its
/// own ViewportCommand::Focus from the correct frame context.
fn hide_main_window() {
    let hwnd_raw = MAIN_HWND.load(Ordering::Relaxed);
    if hwnd_raw != 0 {
        unsafe {
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn show_main_window(ctx: &egui::Context) {
    WAS_SHOW_REQUESTED.store(true, Ordering::Relaxed);
    let hwnd_raw = MAIN_HWND.load(Ordering::Relaxed);
    if hwnd_raw != 0 {
        unsafe {
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
    // Wake up eframe's event loop so update() runs and renders the UI promptly.
    ctx.request_repaint();
}

// ─────────────────────────────────────────────────────────────────────────────
// Tray icon language variants
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangIcon { Ru, En }

// 5-wide × 7-tall pixel glyphs.  Each [u8; 7] is one row (top→bottom).
// Bit 4 = leftmost column, bit 0 = rightmost column (only bits 4..0 used).
pub fn make_app_icon_rgba(size: usize) -> Vec<u8> {
    assert_eq!(size, 32);
    include_bytes!("../assets/app_32.raw").to_vec()
}

fn make_lang_icon(lang: LangIcon, dimmed: bool) -> tray_icon::Icon {
    const SIZE: u32 = 32;
    let bytes: &[u8] = match lang {
        LangIcon::Ru => include_bytes!("../assets/ru.raw"),
        LangIcon::En => include_bytes!("../assets/en.raw"),
    };
    let mut rgba = bytes.to_vec();
    if dimmed {
        for px in rgba.chunks_mut(4) {
            px[0] = (px[0] as u32 * 35 / 100) as u8;
            px[1] = (px[1] as u32 * 35 / 100) as u8;
            px[2] = (px[2] as u32 * 35 / 100) as u8;
            // px[3] = alpha, leave unchanged
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────
// Process-global settings handle (hook_proc is a bare fn pointer, no captures)
// ─────────────────────────────────────────────────────────────────────────────

static SETTINGS: OnceLock<Arc<RwLock<Settings>>> = OnceLock::new();

// ─────────────────────────────────────────────────────────────────────────────
// Hook-thread local storage
// ─────────────────────────────────────────────────────────────────────────────

thread_local! {
    static WORD_BUF: RefCell<buffer::WordBuffer> =
        RefCell::new(buffer::WordBuffer::new());

    /// State needed to undo the last automatic switch.
    /// Cleared as soon as the user starts typing the next word.
    static UNDO: RefCell<Option<UndoState>> = const { RefCell::new(None) };
}

/// Everything required to reverse a `perform_switch` call.
struct UndoState {
    /// The original (mistyped) word to re-inject.
    original_word: String,
    /// How many characters to backspace before re-injecting
    /// (= new_word.len() + 1 if a boundary key was included).
    erase_len: usize,
    /// Layout to restore to (`true` = RU, `false` = EN).
    /// This is the OPPOSITE of `SwitchAction::to_ru`.
    restore_to_ru: bool,
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

    // Skip events injected by our own SendInput calls.
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

/// Low-word of the foreground window's HKL (≈ Windows LANGID).
unsafe fn foreground_lang() -> u16 {
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return 0;
    }
    let tid = GetWindowThreadProcessId(hwnd, None);
    let hkl = GetKeyboardLayout(tid);
    (hkl.0 as usize) as u16
}

/// Returns true if `vk` is a modifier key (Win, Shift, Ctrl, Alt).
/// Modifier key presses must not clear the word buffer — they may be the first
/// half of a hotkey combo (e.g. Win key down before Shift fires Win+Shift).
fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk,
        0x5B | 0x5C           // VK_LWIN, VK_RWIN
        | 0x10 | 0xA0 | 0xA1 // VK_SHIFT, VK_LSHIFT, VK_RSHIFT
        | 0x11 | 0xA2 | 0xA3 // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
        | 0x12 | 0xA4 | 0xA5 // VK_MENU, VK_LMENU, VK_RMENU
    )
}

/// Returns true if the physical key for `configured` vk is currently held.
/// Expands logical VKs (VK_SHIFT/CTRL/MENU) to their left+right variants.
unsafe fn key_is_held(configured: u16) -> bool {
    let held = |vk: i32| GetAsyncKeyState(vk) < 0;
    match configured {
        0x10 => held(0xA0) || held(0xA1), // VK_SHIFT
        0x11 => held(0xA2) || held(0xA3), // VK_CONTROL
        0x12 => held(0xA4) || held(0xA5), // VK_MENU
        v    => held(v as i32),
    }
}

/// Match an actual low-level VK code against a configured (possibly logical) VK.
/// VK_SHIFT (0x10) matches both VK_LSHIFT (0xA0) and VK_RSHIFT (0xA1).
fn vk_matches(actual: u16, configured: u16) -> bool {
    match configured {
        0x10 => matches!(actual, 0xA0 | 0xA1), // VK_SHIFT → either shift
        0x11 => matches!(actual, 0xA2 | 0xA3), // VK_CONTROL → either ctrl
        0x12 => matches!(actual, 0xA4 | 0xA5), // VK_MENU → either alt
        _ => actual == configured,
    }
}

unsafe fn process_key(
    vk: VIRTUAL_KEY,
    buf: &mut buffer::WordBuffer,
    hotkey: Option<(VIRTUAL_KEY, bool)>,
    undo_hotkey: Option<(VIRTUAL_KEY, bool)>,
) -> bool {
    // GetAsyncKeyState reads the physical key state in real time.
    // GetKeyState inside a WH_KEYBOARD_LL hook can return stale state for
    // the Win key because Windows handles VK_LWIN/VK_RWIN at a lower level.
    let win_held = GetAsyncKeyState(VK_LWIN.0 as i32) < 0
        || GetAsyncKeyState(VK_RWIN.0 as i32) < 0;

    // ── Debug: log every key with modifiers and buffer state ─────────────────
    log_debug!(
        "[KEY] vk={:#04x} win={} buf={}",
        vk.0, win_held as u8, buf.len()
    );

    // Returns true when `vk` matches the configured hotkey key and the Win
    // modifier state is satisfied — in EITHER press order:
    //   Order A: Win↓ first, then trigger key↓  (win_held already true)
    //   Order B: trigger key↓ first, then Win↓  (vk is Win, trigger key held)
    let hotkey_fires = |vk: VIRTUAL_KEY, hk_vk: VIRTUAL_KEY, hk_win: bool| -> bool {
        let is_win_vk = vk.0 == VK_LWIN.0 || vk.0 == VK_RWIN.0;
        if hk_win {
            // Order A: trigger key pressed while Win already held
            let order_a = vk_matches(vk.0, hk_vk.0) && win_held;
            // Order B: Win pressed while trigger key is already held
            let order_b = is_win_vk && key_is_held(hk_vk.0);
            order_a || order_b
        } else {
            // No Win modifier required — simple VK match, Win must NOT be held
            vk_matches(vk.0, hk_vk.0) && !win_held
        }
    };

    // ── Undo last switch ─────────────────────────────────────────────────────
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
                switcher::perform_switch(&undo_action, None);
            } else {
                log_info!("[UNDO] hotkey pressed but no undo state available");
            }
            buf.clear();
            if uh_win {
                // Suppress Start menu: Win was held but we swallowed the key.
                switcher::suppress_start_menu();
            }
            return true;
        }
    }

    // ── Force switch hotkey ──────────────────────────────────────────────────
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
                save_undo(&action, false);
                buf.clear();
                switcher::perform_switch(&action, None);
            } else {
                log_info!("[FORCE] lang={:#06x} buf_len={} → no action (untranslatable or wrong layout)", lang, buf.len());
                buf.clear();
            }
            if hk_win {
                // Suppress Start menu: Win was held but we swallowed the key.
                switcher::suppress_start_menu();
            }
            UNDO.with(|u| u.borrow_mut().take());
            return true;
        }
    }

    // ── Normal key handling ──────────────────────────────────────────────────
    match vk {
        VK_BACK => {
            buf.pop();
            false
        }

        VK_SPACE | VK_RETURN | VK_TAB => {
            let lang = foreground_lang();
            let snap = buf.detection_snapshot();
            let result = buf.detect_mismatch(lang);

            // Log the detection attempt with full scores.
            if let Some(ref s) = snap {
                match &result {
                    Some(action) => log_info!(
                        "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → SWITCH_{} boundary={:#04x}",
                        lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                        s.score_en - s.score_ru,
                        if action.to_ru { "EN→RU" } else { "RU→EN" },
                        vk.0
                    ),
                    None => log_info!(
                        "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → skip (threshold={:.1})",
                        lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                        s.score_en - s.score_ru,
                        bigrams::THRESHOLD_PER_BIGRAM
                    ),
                }
            }

            if let Some(action) = result {
                save_undo(&action, true);
                buf.clear();
                switcher::perform_switch(&action, Some(vk));
                true
            } else {
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
                    // Ctrl+letter is a shortcut (Ctrl+C/V/Z/A/…), not text input.
                    buf.clear();
                    UNDO.with(|u| *u.borrow_mut() = None);
                } else {
                    let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                    let caps = GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0;
                    buf.push(vk.0, shift ^ caps);
                    UNDO.with(|u| *u.borrow_mut() = None);
                }
            } else if is_modifier_vk(vk.0) {
                // Modifier key (Win/Shift/Ctrl/Alt) — keep the buffer intact so
                // the user can follow with a hotkey combo without clearing state.
            } else {
                // Non-translatable key = word boundary (dash, bracket, digit, …).
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
                        None if s.len >= 2 => log_info!(
                            "[DETECT] lang={:#06x} en={:?} ru={:?} score_en={:.2} score_ru={:.2} diff={:+.2} → skip",
                            lang, s.en_word, s.ru_word, s.score_en, s.score_ru,
                            s.score_en - s.score_ru,
                        ),
                        _ => {}
                    }
                }
                if let Some(action) = result {
                    save_undo(&action, false);
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

/// Persist an undo snapshot after a successful switch.
///
/// `had_boundary` — whether a boundary key (Space) was re-injected after the
/// new word; if so, the cursor is one character further right and we need one
/// extra Backspace to undo.
fn save_undo(action: &buffer::SwitchAction, had_boundary: bool) {
    UNDO.with(|u| {
        *u.borrow_mut() = Some(UndoState {
            original_word: action.original_word.clone(),
            erase_len: action.new_word.len() + if had_boundary { 1 } else { 0 },
            restore_to_ru: !action.to_ru,
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


// ─────────────────────────────────────────────────────────────────────────────
// eframe application
// ─────────────────────────────────────────────────────────────────────────────

struct RswitcherApp {
    settings: Arc<RwLock<Settings>>,
    new_exception: String,
    autostart_cached: bool,
    first_frame: bool,
    window_visible: bool,
    // ── Running-apps picker ───────────────────────────────────────────────────
    show_picker: bool,
    running_apps: Vec<exceptions::RunningApp>,
}

impl RswitcherApp {
    fn new(settings: Arc<RwLock<Settings>>) -> Self {
        Self {
            settings,
            new_exception: String::new(),
            autostart_cached: autostart::is_enabled(),
            first_frame: true,
            window_visible: false,
            show_picker: false,
            running_apps: Vec::new(),
        }
    }
}

impl eframe::App for RswitcherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Start hidden on the first frame unless explicitly requested ──────
        if self.first_frame {
            self.first_frame = false;
            if !WAS_SHOW_REQUESTED.load(Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        }

        // ── Close / hide logic (window X button → hide, not quit) ───────────
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            WAS_SHOW_REQUESTED.store(false, Ordering::Relaxed);
            self.window_visible = false;
            // ViewportCommand::Visible(false) has the same inter-frame discard
            // problem as Visible(true) — use Win32 directly for a reliable hide.
            hide_main_window();
        }

        let should_be_visible = WAS_SHOW_REQUESTED.load(Ordering::Relaxed);

        // ── Hidden → visible transition ──────────────────────────────────────
        // show_main_window() uses Win32 ShowWindow directly (reliable for hidden
        // windows) and sets WAS_SHOW_REQUESTED.  Here we keep eframe's internal
        // state in sync by sending ViewportCommand::Focus from within update(),
        // which is the only frame context where viewport commands are processed.
        if should_be_visible && !self.window_visible {
            self.window_visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // ── Wakeup & Repaint for background processing (hidden state) ─────────
        if !should_be_visible {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }

        // ── UI ───────────────────────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("RSwitcher");
            ui.separator();

            // ── Master toggle ────────────────────────────────────────────────
            {
                let mut s = self.settings.write().unwrap();
                let before = s.enabled;
                ui.checkbox(&mut s.enabled, "Включить автопереключение раскладки");
                if s.enabled != before {
                    settings::save(&s);
                }
            }

            ui.add_space(8.0);

            // ── Exclusions ───────────────────────────────────────────────────
            ui.label("Исключения:");
            ui.add_space(2.0);

            let mut to_remove: Option<usize> = None;
            {
                let s = self.settings.read().unwrap();
                egui::ScrollArea::vertical()
                    .id_salt("exc")
                    .max_height(90.0)
                    .show(ui, |ui| {
                        if s.exceptions.is_empty() {
                            ui.weak("(список пуст)");
                        }
                        for (i, exc) in s.exceptions.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(exc);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("✕").clicked() {
                                            to_remove = Some(i);
                                        }
                                    },
                                );
                            });
                        }
                    });
            }
            if let Some(i) = to_remove {
                let mut s = self.settings.write().unwrap();
                s.exceptions.remove(i);
                settings::save(&s);
            }

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let resp = ui.text_edit_singleline(&mut self.new_exception);
                let add = ui.button("Добавить").clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if add {
                    let exc = self.new_exception.trim().to_lowercase();
                    if !exc.is_empty() {
                        let mut s = self.settings.write().unwrap();
                        if !s.exceptions.contains(&exc) {
                            s.exceptions.push(exc);
                            settings::save(&s);
                        }
                        self.new_exception.clear();
                        resp.request_focus();
                    }
                }
                if ui.button("Выбрать из запущенных…").clicked() {
                    self.running_apps = exceptions::enumerate_visible_apps();
                    self.show_picker = true;
                }
            });

            ui.add_space(8.0);
            ui.separator();

            // ── Hotkeys ──────────────────────────────────────────────────────
            ui.label("Горячие клавиши:");
            ui.add_space(4.0);

            {
                let mut s = self.settings.write().unwrap();

                let before = (s.hotkey_enabled, s.undo_hotkey_enabled);

                egui::Grid::new("hotkeys")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.checkbox(&mut s.hotkey_enabled, "Принудительное переключение");
                        ui.label("→");
                        let name = vk_display_name(s.hotkey_vk, s.hotkey_win);
                        if s.hotkey_enabled {
                            ui.label(egui::RichText::new(name).monospace());
                        } else {
                            ui.weak(name);
                        }
                        ui.end_row();

                        ui.checkbox(&mut s.undo_hotkey_enabled, "Отмена последнего переключения");
                        ui.label("→");
                        let name = vk_display_name(s.undo_hotkey_vk, s.undo_hotkey_win);
                        if s.undo_hotkey_enabled {
                            ui.label(egui::RichText::new(name).monospace());
                        } else {
                            ui.weak(name);
                        }
                        ui.end_row();
                    });

                if (s.hotkey_enabled, s.undo_hotkey_enabled) != before {
                    settings::save(&s);
                }
            }

            ui.add_space(4.0);
            ui.weak("Изменить клавишу: отредактируйте config.json (hotkey_vk, hotkey_win, undo_hotkey_vk, undo_hotkey_win).");

            ui.add_space(8.0);
            ui.separator();

            // ── System / autostart ───────────────────────────────────────────
            let prev = self.autostart_cached;
            ui.checkbox(&mut self.autostart_cached, "Запускать при старте Windows");
            if self.autostart_cached != prev {
                autostart::set_enabled(self.autostart_cached);
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.weak("Настройки сохраняются автоматически.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Открыть конфиг").clicked() {
                        let _ = std::process::Command::new("explorer")
                            .arg(settings::config_path())
                            .spawn();
                    }
                });
            });
        });

        // ── Running-apps picker popup ─────────────────────────────────────────
        // Rendered outside CentralPanel so it floats on top of the main UI.
        if self.show_picker {
            let current_exceptions: Vec<String> =
                self.settings.read().unwrap().exceptions.clone();
            let mut to_add:     Option<String> = None;
            let mut do_refresh: bool           = false;
            let mut close:      bool           = false;

            egui::Window::new("Запущенные программы")
                .collapsible(false)
                .resizable(false)
                .default_width(460.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} программ", self.running_apps.len()));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.button("↻ Обновить").clicked() {
                                    do_refresh = true;
                                }
                            },
                        );
                    });
                    ui.separator();

                    egui::ScrollArea::vertical()
                        .id_salt("picker")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            if self.running_apps.is_empty() {
                                ui.weak("Нет запущенных приложений.");
                            }
                            for app in &self.running_apps {
                                let already = current_exceptions.contains(&app.exe);
                                ui.horizontal(|ui| {
                                    // Right side first (right_to_left layout) so the
                                    // button doesn't squeeze the labels on small windows.
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if already {
                                                ui.weak("✓");
                                            } else if ui.small_button("+").clicked() {
                                                to_add = Some(app.exe.clone());
                                            }
                                            // Truncate long titles so exe name is readable.
                                            let title: &str = &app.title;
                                            let short = if title.len() > 38 {
                                                &title[..38]
                                            } else {
                                                title
                                            };
                                            ui.label(
                                                egui::RichText::new(short)
                                                    .weak()
                                                    .size(11.0),
                                            );
                                            ui.label(
                                                egui::RichText::new(&app.exe).monospace(),
                                            );
                                        },
                                    );
                                });
                            }
                        });

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.weak("Кликните + чтобы добавить в исключения.");
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.button("Закрыть").clicked() {
                                    close = true;
                                }
                            },
                        );
                    });
                });

            if do_refresh {
                self.running_apps = exceptions::enumerate_visible_apps();
            }
            if let Some(exe) = to_add {
                let mut s = self.settings.write().unwrap();
                if !s.exceptions.contains(&exe) {
                    s.exceptions.push(exe);
                    settings::save(&s);
                }
            }
            if close {
                self.show_picker = false;
            }
        }
    }
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

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
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
        log_info!(
            "bigrams: threshold={:.1} nat/bigram  (EN_BIGRAMS[676], RU_BIGRAMS[1024])",
            bigrams::THRESHOLD_PER_BIGRAM
        );
    }

    let _hook_thread = start_hook_thread();
    let (show_id, quit_id) = build_tray();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RSwitcher — Настройки")
            .with_inner_size([460.0, 400.0])
            .with_visible(false)
            .with_resizable(false)
            .with_icon(egui::IconData {
                rgba:   make_app_icon_rgba(32),
                width:  32,
                height: 32,
            }),
        ..Default::default()
    };

    eframe::run_native(
        "RSwitcher",
        native_options,
        Box::new(move |cc| {
            // Store HWND for use by the tray watcher, then hide immediately to
            // prevent startup flicker (with_visible(false) alone can still show
            // a brief flash before the first frame hides the window).
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                    let hwnd_raw = win32_handle.hwnd.get();
                    MAIN_HWND.store(hwnd_raw as usize, Ordering::Relaxed);
                    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
            }

            spawn_tray_watcher(cc.egui_ctx.clone(), show_id, quit_id);

            Ok(Box::new(RswitcherApp::new(settings)))
        }),
    )

}

fn build_tray() -> (MenuId, MenuId) {
    let show_item = MenuItem::new("Настройки", true, None);
    let quit_item = MenuItem::new("Выход", true, None);
    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();

    let menu = Menu::new();
    menu.append_items(&[&show_item, &PredefinedMenuItem::separator(), &quit_item])
        .unwrap();

    // Detect the initial foreground language so the icon is correct from the start.
    let initial_lang = {
        let w = unsafe { foreground_lang() };
        if layout::hkl_is_russian(w) { LangIcon::Ru } else { LangIcon::En }
    };

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("RSwitcher")
        .with_icon(make_lang_icon(initial_lang, false))
        .build()
        .expect("failed to create tray icon");

    TRAY_ICON
        .set(Mutex::new(SendableTray(tray)))
        .ok();

    (show_id, quit_id)
}
