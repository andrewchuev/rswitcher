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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use eframe::egui;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIconBuilder, TrayIconEvent,
};
use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyboardLayout, GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_LWIN,
            VK_RETURN, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
            GetWindowThreadProcessId, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
            KBDLLHOOKSTRUCT, KBDLLHOOKSTRUCT_FLAGS, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL,
            WM_KEYDOWN, WM_SYSKEYDOWN,
        },
    },
};

pub use settings::Settings;

// ─────────────────────────────────────────────────────────────────────────────
// Tray watcher
// ─────────────────────────────────────────────────────────────────────────────

// Set to true by the tray watcher thread; cleared + acted on in update().
static SHOW_WINDOW: AtomicBool = AtomicBool::new(false);

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
            let mut last_icon: Option<LangIcon> = None;
            loop {
                // ── Menu events ───────────────────────────────────────────────
                let mut need_repaint = false;
                while let Ok(ev) = MenuEvent::receiver().try_recv() {
                    if ev.id == quit_id {
                        log_info!("=== RSwitcher quit via tray menu ===");
                        std::process::exit(0);
                    } else if ev.id == show_id {
                        SHOW_WINDOW.store(true, Ordering::Relaxed);
                        need_repaint = true;
                    }
                }
                // ── Tray icon events (double-click → show settings) ───────────
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if matches!(ev, TrayIconEvent::DoubleClick { .. }) {
                        SHOW_WINDOW.store(true, Ordering::Relaxed);
                        need_repaint = true;
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
                if new_lang.is_some() && new_lang != last_icon {
                    if let Some(lang) = new_lang {
                        if let Some(tray) = TRAY_ICON.get() {
                            if let Ok(t) = tray.lock() {
                                let _ = t.0.set_icon(Some(make_lang_icon(lang)));
                            }
                        }
                    }
                    last_icon = new_lang;
                }
                if need_repaint {
                    ctx.request_repaint();
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .expect("failed to spawn tray watcher thread");
}

// ─────────────────────────────────────────────────────────────────────────────
// Tray icon language variants
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangIcon { Ru, En }

// 5-wide × 7-tall pixel glyphs.  Each [u8; 7] is one row (top→bottom).
// Bit 4 = leftmost column, bit 0 = rightmost column (only bits 4..0 used).
#[rustfmt::skip]
const GLYPH_R: [u8; 7] = [
    0b11100, // ███..
    0b10010, // █..█.
    0b11100, // ███..
    0b10100, // █.█..
    0b10010, // █..█.
    0b00000,
    0b00000,
];


// 's' glyph — used in the app icon ("Rs" = RSwitcher)
#[rustfmt::skip]
const GLYPH_S: [u8; 7] = [
    0b01110, // .███.
    0b10000, // █....
    0b01110, // .███.
    0b00001, // ....█
    0b01110, // .███.
    0b00000,
    0b00000,
];

/// Generate 32×32 RGBA pixels for the application icon ("Rs" on dark-blue).
/// Used both as the eframe window icon and (via build.rs) as the .exe resource.
pub fn make_app_icon_rgba(size: usize) -> Vec<u8> {
    let scale = (size / 16).max(1);
    let glyph_w = 5 * scale;
    let glyph_h = 7 * scale;
    let gap     = scale;
    let text_w  = glyph_w * 2 + gap;
    let off_x   = size.saturating_sub(text_w) / 2;
    let off_y   = size.saturating_sub(glyph_h) / 2;
    let radius  = (size as f32 * 0.15).max(1.0);

    let bg: [u8; 3] = [0x1a, 0x2e, 0x6c];
    let fg: [u8; 3] = [0xd0, 0xd8, 0xe0];

    let half_w = glyph_w;
    let glyph_pixel = |gx: usize, gy: usize| -> bool {
        if gy >= glyph_h { return false; }
        let row = gy / scale;
        let (glyph, col_pixel) = if gx < half_w {
            (&GLYPH_R, gx)
        } else if gx < half_w + gap {
            return false;
        } else {
            (&GLYPH_S, gx - half_w - gap)
        };
        if row >= 7 { return false; }
        let col = col_pixel / scale;
        if col >= 5 { return false; }
        (glyph[row] >> (4 - col)) & 1 == 1
    };

    let mut pixels = vec![0u8; size * size * 4];
    let s = size as f32;
    for py in 0..size {
        for px in 0..size {
            let idx = (py * size + px) * 4;
            let fx = px as f32 + 0.5;
            let fy = py as f32 + 0.5;
            let qx = (fx - s * 0.5).abs() - (s * 0.5 - radius);
            let qy = (fy - s * 0.5).abs() - (s * 0.5 - radius);
            let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
            let alpha = (1.0 - dist.clamp(-1.0, 1.0) * 0.5 - 0.5).clamp(0.0, 1.0);
            if alpha <= 0.0 { continue; }
            let is_text = px >= off_x
                && py >= off_y
                && px < off_x + text_w
                && py < off_y + glyph_h
                && glyph_pixel(px - off_x, py - off_y);
            let c = if is_text { fg } else { bg };
            pixels[idx]     = c[0];
            pixels[idx + 1] = c[1];
            pixels[idx + 2] = c[2];
            pixels[idx + 3] = (alpha * 255.0) as u8;
        }
    }
    pixels
}

fn make_lang_icon(lang: LangIcon) -> tray_icon::Icon {
    const SIZE: u32 = 32;
    let bytes: &[u8] = match lang {
        LangIcon::Ru => include_bytes!("../assets/ru.raw"),
        LangIcon::En => include_bytes!("../assets/en.raw"),
    };
    tray_icon::Icon::from_rgba(bytes.to_vec(), SIZE, SIZE).unwrap()
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
            eprintln!("[hook] installed");

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
            eprintln!("[hook] uninstalled");
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
}

impl RswitcherApp {
    fn new(settings: Arc<RwLock<Settings>>) -> Self {
        Self {
            settings,
            new_exception: String::new(),
            autostart_cached: autostart::is_enabled(),
        }
    }

    fn show_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}

impl eframe::App for RswitcherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Show-window request from tray watcher thread ─────────────────────
        if SHOW_WINDOW.swap(false, Ordering::Relaxed) {
            self.show_window(ctx);
        }

        // ── Close / hide logic (window X button → hide, not quit) ───────────
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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
            ui.label("Исключения (имя .exe, без пути):");
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
        .with_icon(make_lang_icon(initial_lang))
        .build()
        .expect("failed to create tray icon");

    TRAY_ICON
        .set(Mutex::new(SendableTray(tray)))
        .ok();

    (show_id, quit_id)
}
