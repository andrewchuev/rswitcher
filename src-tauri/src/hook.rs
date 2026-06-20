use std::cell::RefCell;
use crate::globals::SETTINGS;
use crate::tray::foreground_lang;
use crate::{buffer, exceptions, layout, switcher};
use crate::{log_debug, log_info, log_error};

use windows::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_LWIN,
            VK_RETURN, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, GetForegroundWindow,
            SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
            KBDLLHOOKSTRUCT, KBDLLHOOKSTRUCT_FLAGS, LLKHF_INJECTED, MSG,
            WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
            WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_MBUTTONDOWN,
            WM_NCLBUTTONDOWN, WM_NCRBUTTONDOWN,
            MessageBoxW, MB_ICONWARNING, MB_OK,
        },
    },
};

thread_local! {
    static WORD_BUF: RefCell<buffer::WordBuffer> =
        RefCell::new(buffer::WordBuffer::new());

    static PREV_WORD_BUF: RefCell<Option<(buffer::WordBuffer, VIRTUAL_KEY)>> =
        const { RefCell::new(None) };

    static UNDO: RefCell<Option<UndoState>> = const { RefCell::new(None) };

    static LAST_HWND: RefCell<HWND> = RefCell::new(HWND::default());

    static SUCCESS_COUNTS: RefCell<std::collections::HashMap<String, u32>> =
        RefCell::new(std::collections::HashMap::new());
}

struct UndoState {
    original_word: String,
    erase_len: usize,
    restore_lang: u16,
    boundary_vk: Option<VIRTUAL_KEY>,
}

/// Windows calls this on the hook thread for every low-level keyboard event.
/// A panic unwinding across this `extern "system"` FFI boundary is undefined
/// behavior, so the real work happens in `hook_proc_inner` wrapped in
/// `catch_unwind`.  On panic we fall through to `CallNextHookEx`, keeping the
/// app (and the user's typing) alive; the panic hook still logs the backtrace.
unsafe extern "system" fn hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        hook_proc_inner(n_code, w_param, l_param)
    }));
    match result {
        Ok(lresult) => lresult,
        Err(_) => CallNextHookEx(None, n_code, w_param, l_param),
    }
}

unsafe fn hook_proc_inner(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code < 0 {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);

    if kb.flags & LLKHF_INJECTED != KBDLLHOOKSTRUCT_FLAGS(0) && kb.dwExtraInfo == 0x53574954 {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let is_down = matches!(
        w_param.0 as u32,
        w if w == WM_KEYDOWN || w == WM_SYSKEYDOWN
    );
    if !is_down {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    // Check if the active window has changed since the last keystroke
    let hwnd = GetForegroundWindow();
    let window_changed = LAST_HWND.with(|cell| {
        let mut last = cell.borrow_mut();
        if *last != hwnd {
            *last = hwnd;
            true
        } else {
            false
        }
    });

    if window_changed {
        WORD_BUF.with(|c| {
            let mut buf = c.borrow_mut();
            if !buf.is_empty() {
                log_info!("[FOCUS_CHANGE] window changed — buffer cleared");
                buf.clear();
            }
        });
        UNDO.with(|u| *u.borrow_mut() = None);
    }

    let (enabled, excluded, excluded_name, hotkey, undo_hotkey, sensitivity) = SETTINGS
        .get()
        .and_then(|s| s.try_read().ok())
        .map(|s| {
            let excluded_name = if s.exceptions.is_empty() {
                None
            } else {
                exceptions::foreground_exe_name()
                    .filter(|name| s.exceptions.contains(name))
            };
            let excluded = excluded_name.is_some();
            let hotkey = s.hotkey_enabled.then_some((VIRTUAL_KEY(s.hotkey_vk), s.hotkey_win));
            let undo_hotkey = s.undo_hotkey_enabled.then_some((VIRTUAL_KEY(s.undo_hotkey_vk), s.undo_hotkey_win));
            (s.enabled, excluded, excluded_name, hotkey, undo_hotkey, s.sensitivity)
        })
        .unwrap_or((false, false, None, None, None, 1.0));

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
        process_key(vk, &mut buf, hotkey, undo_hotkey, sensitivity)
    });

    if swallow {
        LRESULT(1)
    } else {
        CallNextHookEx(None, n_code, w_param, l_param)
    }
}

fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk,
        0x5B | 0x5C           // VK_LWIN, VK_RWIN
        | 0x10 | 0xA0 | 0xA1 // VK_SHIFT, VK_LSHIFT, VK_RSHIFT
        | 0x11 | 0xA2 | 0xA3 // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
        | 0x12 | 0xA4 | 0xA5 // VK_MENU, VK_LMENU, VK_RMENU
    )
}

fn is_navigation_vk(vk: u16) -> bool {
    matches!(vk,
        0x21 | 0x22           // VK_PRIOR (Page Up), VK_NEXT (Page Down)
        | 0x23 | 0x24         // VK_END, VK_HOME
        | 0x25 | 0x26 | 0x27 | 0x28 // VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN
        | 0x2D | 0x2E         // VK_INSERT, VK_DELETE
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
    sensitivity: f32,
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
                    "[UNDO] restoring {:?}, erase={}, restore_lang={:#06x}",
                    s.original_word, s.erase_len, s.restore_lang
                );
                
                // Add the restored word to the ignored_words whitelist
                let word_to_ignore = s.original_word.to_lowercase();
                if !word_to_ignore.is_empty() {
                    if let Some(settings_arc) = SETTINGS.get() {
                        if let Ok(mut settings) = settings_arc.write() {
                            if !settings.ignored_words.contains(&word_to_ignore) {
                                settings.ignored_words.push(word_to_ignore.clone());
                                crate::settings::save_async(&settings);
                                log_info!("[UNDO] Added '{}' to ignored_words whitelist", word_to_ignore);
                            }
                        }
                    }
                }

                let undo_action = buffer::SwitchAction {
                    backspaces: s.erase_len,
                    new_word: s.original_word,
                    target_lang: s.restore_lang,
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
                        "[FORCE] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → target={:#06x}",
                        lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                        action.target_lang
                    );
                    record_force_correction(&s.en_word, action.target_lang);
                }

                save_undo(&action, None, lang);
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
                                "[FORCE-PREV] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → target={:#06x} boundary={:#04x}",
                                lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                                action.target_lang,
                                boundary_vk.0
                            );
                            record_force_correction(&s.en_word, action.target_lang);
                        }
                        let mut force_action = action.clone();
                        force_action.backspaces += 1;
                        save_undo(&force_action, Some(boundary_vk), lang);
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
            let result = buf.detect_mismatch_with_sensitivity(lang, sensitivity);

            if let Some(ref s) = snap {
                match &result {
                    Some(action) => {
                        let switch_direction = if action.target_lang == layout::LANG_EN_US {
                            if layout::hkl_is_russian(lang) { "RU→EN" } else { "UA→EN" }
                        } else if action.target_lang == layout::LANG_RU {
                            "EN→RU"
                        } else {
                            "EN→UA"
                        };
                        log_info!(
                            "[DETECT] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → SWITCH_{} boundary={:#04x}",
                            lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                            switch_direction,
                            vk.0
                        );
                    }
                    None => {
                        let reason = if s.len == 1 {
                            "skip (single-char)".to_string()
                        } else if s.len == 2 || s.len == 3 {
                            "skip (dictionary)".to_string()
                        } else {
                            format!("skip (threshold={:.2})", buffer::switching_threshold(s.len) / sensitivity)
                        };
                        log_info!(
                            "[DETECT] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → {}",
                            lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                            reason
                        );
                    }
                }
            }

            if let Some(action) = result {
                save_undo(&action, Some(vk), lang);
                buf.clear();
                PREV_WORD_BUF.with(|p| *p.borrow_mut() = None);
                switcher::perform_switch(&action, Some(vk));
                true
            } else {
                if !buf.is_empty() {
                    PREV_WORD_BUF.with(|p| *p.borrow_mut() = Some((buf.clone(), vk)));

                    // Adaptive dictionary check:
                    // If a word is successfully typed (no switch), we count it.
                    // If it is typed 3 times, we automatically whitelist it.
                    if let Some(ref s) = snap {
                        let word_to_check = if layout::hkl_is_russian(lang) {
                            if s.ru_word.chars().all(|c| c.is_alphabetic()) && s.ru_word.chars().count() >= 4 {
                                Some((s.ru_word.clone(), buffer::RU_COMMON_WORDS.binary_search(&s.ru_word.as_str()).is_ok()))
                            } else {
                                None
                            }
                        } else if layout::hkl_is_ukrainian(lang) {
                            if s.ua_word.chars().all(|c| c.is_alphabetic()) && s.ua_word.chars().count() >= 4 {
                                Some((s.ua_word.clone(), buffer::UA_COMMON_WORDS.binary_search(&s.ua_word.as_str()).is_ok()))
                            } else {
                                None
                            }
                        } else if layout::hkl_is_english(lang) {
                            if s.en_word.chars().all(|c| c.is_alphabetic()) && s.en_word.chars().count() >= 4 {
                                Some((s.en_word.clone(), buffer::EN_COMMON_WORDS.binary_search(&s.en_word.as_str()).is_ok()))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some((word, is_common)) = word_to_check {
                            // Only count toward the adaptive whitelist when the
                            // rival layout actually scored higher than the active
                            // one. If the current layout wins outright (like
                            // "функционал" in RU with score delta ~4.5), the model
                            // was never tempted to switch — whitelisting is useless.
                            let rival_was_competitive = if layout::hkl_is_russian(lang) {
                                s.score_en > s.score_ru
                            } else if layout::hkl_is_ukrainian(lang) {
                                s.score_en > s.score_ua
                            } else {
                                s.score_ru.max(s.score_ua) > s.score_en
                            };
                            if !is_common && rival_was_competitive {
                                SUCCESS_COUNTS.with(|sc| {
                                    let mut counts = sc.borrow_mut();
                                    // Seed in-session count from persistent adaptive_counts
                                    // the first time we see this word (entry absent).
                                    let entry = counts.entry(word.clone()).or_insert_with(|| {
                                        SETTINGS.get()
                                            .and_then(|s| s.try_read().ok())
                                            .and_then(|s| s.adaptive_counts.get(&word).copied())
                                            .unwrap_or(0)
                                    });
                                    *entry += 1;
                                    let new_count = *entry;
                                    if new_count >= 3 {
                                        if let Some(settings_arc) = SETTINGS.get() {
                                            if let Ok(mut settings) = settings_arc.write() {
                                                if !settings.ignored_words.contains(&word) {
                                                    settings.ignored_words.push(word.clone());
                                                    log_info!("[ADAPTIVE] Added '{}' to ignored_words whitelist (typed 3× total)", word);
                                                }
                                                settings.adaptive_counts.remove(&word);
                                                crate::settings::save_async(&settings);
                                            }
                                        }
                                        counts.remove(&word);
                                    } else {
                                        // Persist intermediate count so it survives a restart.
                                        if let Some(settings_arc) = SETTINGS.get() {
                                            if let Ok(mut settings) = settings_arc.write() {
                                                settings.adaptive_counts.insert(word.clone(), new_count);
                                                crate::settings::save_async(&settings);
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }
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

                    // On-the-fly layout detection check
                    let lang = foreground_lang();
                    if let Some(action) = buf.detect_mismatch_on_the_fly(lang, sensitivity) {
                        let snap = buf.detection_snapshot();
                        if let Some(ref s) = snap {
                            let switch_direction = if action.target_lang == layout::LANG_EN_US {
                                if layout::hkl_is_russian(lang) { "RU→EN" } else { "UA→EN" }
                            } else if action.target_lang == layout::LANG_RU {
                                "EN→RU"
                            } else {
                                "EN→UA"
                            };
                            log_info!(
                                "[FLY-DETECT] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → SWITCH_{} (on-the-fly)",
                                lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                                switch_direction
                            );
                        }
                        save_undo(&action, None, lang);
                        buf.has_switched = true;
                        PREV_WORD_BUF.with(|p| *p.borrow_mut() = None);
                        switcher::perform_switch(&action, None);
                        return true; // Suppress current key event
                    }
                }
            } else if is_modifier_vk(vk.0) {
                // Keep buffer intact
            } else if is_navigation_vk(vk.0) {
                // Cursor-positioning keys (arrows, End, Home, Page Up/Down,
                // Insert, Delete) move the caret but do not end a word — clear
                // the buffer without running layout detection so the next word
                // starts fresh.
                buf.clear();
                UNDO.with(|u| *u.borrow_mut() = None);
            } else {
                let lang = foreground_lang();
                let snap = buf.detection_snapshot();
                let result = buf.detect_mismatch_with_sensitivity(lang, sensitivity);
                if let Some(ref s) = snap {
                    match &result {
                        Some(action) => {
                            let switch_direction = if action.target_lang == layout::LANG_EN_US {
                                if layout::hkl_is_russian(lang) { "RU→EN" } else { "UA→EN" }
                            } else if action.target_lang == layout::LANG_RU {
                                "EN→RU"
                            } else {
                                "EN→UA"
                            };
                            log_info!(
                                "[DETECT] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → SWITCH_{} boundary=vk{:#04x}",
                                lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                                switch_direction,
                                vk.0
                            );
                        }
                        None => {
                            let reason = if s.len == 1 {
                                "skip (single-char)".to_string()
                            } else if s.len == 2 || s.len == 3 {
                                "skip (dictionary)".to_string()
                            } else {
                                format!("skip (threshold={:.2})", buffer::switching_threshold(s.len) / sensitivity)
                            };
                            log_info!(
                                "[DETECT] lang={:#06x} en={:?} ru={:?} ua={:?} score_en={:.2} score_ru={:.2} score_ua={:.2} → {}",
                                lang, s.en_word, s.ru_word, s.ua_word, s.score_en, s.score_ru, s.score_ua,
                                reason
                            );
                        }
                    }
                }
                if let Some(action) = result {
                    save_undo(&action, None, lang);
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

/// Record a user-confirmed force-switch as a persistent correction.
/// `en_key` must be the EN keyboard sequence (VK codes → EN chars) — always
/// available from the detection snapshot regardless of the active layout.
fn record_force_correction(en_key: &str, target_lang: u16) {
    let key = en_key.to_lowercase();
    if key.is_empty() || key.chars().count() < 2 {
        return;
    }
    if let Some(settings_arc) = SETTINGS.get() {
        if let Ok(mut settings) = settings_arc.write() {
            let prev = settings.word_corrections.get(&key).copied();
            if prev != Some(target_lang) {
                settings.word_corrections.insert(key.clone(), target_lang);
                crate::settings::save_async(&settings);
                log_info!(
                    "[LEARN] Recorded correction: {:?} → lang={:#06x} (was {:?})",
                    key, target_lang,
                    prev.map(|l| format!("{:#06x}", l))
                );
            }
        }
    }
}

fn save_undo(action: &buffer::SwitchAction, boundary_vk: Option<VIRTUAL_KEY>, original_lang: u16) {
    UNDO.with(|u| {
        *u.borrow_mut() = Some(UndoState {
            original_word: action.original_word.clone(),
            erase_len: action.new_word.chars().count() + if boundary_vk.is_some() { 1 } else { 0 },
            restore_lang: original_lang,
            boundary_vk,
        });
    });
}

unsafe extern "system" fn mouse_hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        mouse_hook_proc_inner(n_code, w_param, l_param)
    }));
    match result {
        Ok(lresult) => lresult,
        Err(_) => CallNextHookEx(None, n_code, w_param, l_param),
    }
}

unsafe fn mouse_hook_proc_inner(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        let msg = w_param.0 as u32;
        if msg == WM_LBUTTONDOWN
            || msg == WM_RBUTTONDOWN
            || msg == WM_MBUTTONDOWN
            || msg == WM_NCLBUTTONDOWN
            || msg == WM_NCRBUTTONDOWN
        {
            WORD_BUF.with(|c| {
                let mut buf = c.borrow_mut();
                if !buf.is_empty() {
                    log_debug!("[MOUSE_CLICK] buffer cleared");
                    buf.clear();
                }
            });
            UNDO.with(|u| *u.borrow_mut() = None);
        }
    }
    CallNextHookEx(None, n_code, w_param, l_param)
}

pub fn start_hook_thread() -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("rswitcher-hook".into())
        .spawn(|| {
            let hmod = unsafe { GetModuleHandleW(None).expect("GetModuleHandleW failed") };

            let kb_hook = unsafe {
                match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), HINSTANCE(hmod.0), 0) {
                    Ok(h) => h,
                    Err(e) => {
                        log_error!("SetWindowsHookExW (keyboard) failed: {:?}", e);
                        let _ = MessageBoxW(
                            HWND::default(),
                            windows::core::w!("Не удалось запустить клавиатурный перехватчик RSwitcher (SetWindowsHookExW failed).\nПриложение будет закрыто."),
                            windows::core::w!("RSwitcher — Ошибка"),
                            MB_OK | MB_ICONWARNING,
                        );
                        std::process::exit(1);
                    }
                }
            };

            let mouse_hook = unsafe {
                match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), HINSTANCE(hmod.0), 0) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        log_error!("SetWindowsHookExW (mouse) failed: {:?}", e);
                        None
                    }
                }
            };

            log_info!("[hook] installed (keyboard={:?}, mouse={:?})", kb_hook, mouse_hook);

            let mut msg = MSG::default();
            loop {
                let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                if ret.0 <= 0 {
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    let _ = DispatchMessageW(&msg);
                }
            }

            unsafe {
                UnhookWindowsHookEx(kb_hook).ok();
                if let Some(h) = mouse_hook {
                    UnhookWindowsHookEx(h).ok();
                }
            };
            log_info!("[hook] uninstalled");
        })
        .expect("failed to spawn hook thread")
}
