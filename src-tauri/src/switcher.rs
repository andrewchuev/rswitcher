use std::mem;

use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            GetKeyboardLayoutList, GetKeyState, SendInput, HKL,
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
            VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_LWIN, VK_RWIN, VK_SHIFT, VK_LEFT,
            VK_LCONTROL, VK_RCONTROL, VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_RMENU,
        },
        WindowsAndMessaging::{
            GetForegroundWindow, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
        },
    },
};

use crate::buffer::SwitchAction;

/// Execute the complete switch sequence in one atomic `SendInput` call:
///
///  1. N × Backspace         — erase the mistyped word
///  2. Correct word          — injected via `KEYEVENTF_UNICODE` (layout-agnostic)
///  3. Optional boundary key — the Space / Enter that triggered detection
///
/// After injecting the text we send `WM_INPUTLANGCHANGEREQUEST` to the active
/// window so its language-bar indicator flips to match.  Apps that ignore this
/// message still receive the correct text because Unicode injection does not
/// depend on the active layout.
///
/// # Safety
/// Must be called only from the hook thread.  The injected events carry
/// `LLKHF_INJECTED` in their hook-proc flags, so our hook_proc will skip them
/// (preventing infinite recursion).
pub fn perform_switch(action: &SwitchAction, boundary_vk: Option<VIRTUAL_KEY>) {
    let use_selection = crate::globals::SETTINGS
        .get()
        .and_then(|s| s.try_read().ok())
        .map(|s| s.use_selection_replace)
        .unwrap_or(false);

    let mut inputs: Vec<INPUT> = Vec::new();

    // ── 0. Release modifiers if held ──────────────────────────────────────────
    // When a force/undo hotkey is physically held while we send backspaces,
    // Windows delivers injected keys with those modifiers active. E.g. Ctrl
    // held down causes Backspaces to be seen as Ctrl+Backspace, deleting
    // whole words instead of characters.
    // Fix: temporarily release Win, Ctrl, Shift, Alt, and restore them at the end.
    let (lwin, rwin, lctrl, rctrl, lshift, rshift, lalt, ralt) = unsafe {
        (
            GetKeyState(VK_LWIN.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_RWIN.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_LCONTROL.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_RCONTROL.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_LSHIFT.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_RSHIFT.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_LMENU.0 as i32) as u16 & 0x8000 != 0,
            GetKeyState(VK_RMENU.0 as i32) as u16 & 0x8000 != 0,
        )
    };

    if lwin || rwin {
        inputs.push(make_vk(VIRTUAL_KEY(0xE8), KEYBD_EVENT_FLAGS(0)));
        inputs.push(make_vk(VIRTUAL_KEY(0xE8), KEYEVENTF_KEYUP));
        if lwin { inputs.push(make_vk(VK_LWIN, KEYEVENTF_KEYUP)); }
        if rwin { inputs.push(make_vk(VK_RWIN, KEYEVENTF_KEYUP)); }
    }
    if lctrl { inputs.push(make_vk(VK_LCONTROL, KEYEVENTF_KEYUP)); }
    if rctrl { inputs.push(make_vk(VK_RCONTROL, KEYEVENTF_KEYUP)); }
    if lshift { inputs.push(make_vk(VK_LSHIFT, KEYEVENTF_KEYUP)); }
    if rshift { inputs.push(make_vk(VK_RSHIFT, KEYEVENTF_KEYUP)); }
    if lalt { inputs.push(make_vk(VK_LMENU, KEYEVENTF_KEYUP)); }
    if ralt { inputs.push(make_vk(VK_RMENU, KEYEVENTF_KEYUP)); }

    // ── 1. Erase ─────────────────────────────────────────────────────────────
    if action.backspaces > 0 {
        if use_selection {
            // Select word using Ctrl + Shift + Left Arrow
            inputs.push(make_vk(VK_CONTROL, KEYBD_EVENT_FLAGS(0)));
            inputs.push(make_vk(VK_SHIFT, KEYBD_EVENT_FLAGS(0)));
            inputs.push(make_vk(VK_LEFT, KEYBD_EVENT_FLAGS(0)));
            inputs.push(make_vk(VK_LEFT, KEYEVENTF_KEYUP));
            inputs.push(make_vk(VK_SHIFT, KEYEVENTF_KEYUP));
            inputs.push(make_vk(VK_CONTROL, KEYEVENTF_KEYUP));
            // Delete selection using Backspace
            inputs.push(make_vk(VK_BACK, KEYBD_EVENT_FLAGS(0)));
            inputs.push(make_vk(VK_BACK, KEYEVENTF_KEYUP));
        } else {
            for _ in 0..action.backspaces {
                inputs.push(make_vk(VK_BACK, KEYBD_EVENT_FLAGS(0)));
                inputs.push(make_vk(VK_BACK, KEYEVENTF_KEYUP));
            }
        }
    }

    // ── 2. Re-type ───────────────────────────────────────────────────────────
    for ch in action.new_word.chars() {
        let mut buf = [0u16; 2];
        let units = ch.encode_utf16(&mut buf);
        for &unit in units.iter() {
            inputs.push(make_unicode_unit(unit, KEYBD_EVENT_FLAGS(0)));
            inputs.push(make_unicode_unit(unit, KEYEVENTF_KEYUP));
        }
    }

    // ── 3. Re-inject the word-boundary key ───────────────────────────────────
    if let Some(vk) = boundary_vk {
        inputs.push(make_vk(vk, KEYBD_EVENT_FLAGS(0)));
        inputs.push(make_vk(vk, KEYEVENTF_KEYUP));
    }

    // ── 4. Restore modifiers ──────────────────────────────────────────────────
    if lalt { inputs.push(make_vk(VK_LMENU, KEYBD_EVENT_FLAGS(0))); }
    if ralt { inputs.push(make_vk(VK_RMENU, KEYBD_EVENT_FLAGS(0))); }
    if lshift { inputs.push(make_vk(VK_LSHIFT, KEYBD_EVENT_FLAGS(0))); }
    if rshift { inputs.push(make_vk(VK_RSHIFT, KEYBD_EVENT_FLAGS(0))); }
    if lctrl { inputs.push(make_vk(VK_LCONTROL, KEYBD_EVENT_FLAGS(0))); }
    if rctrl { inputs.push(make_vk(VK_RCONTROL, KEYBD_EVENT_FLAGS(0))); }
    if lwin { inputs.push(make_vk(VK_LWIN, KEYBD_EVENT_FLAGS(0))); }
    if rwin { inputs.push(make_vk(VK_RWIN, KEYBD_EVENT_FLAGS(0))); }

    unsafe {
        // Deliver all events in a single call; Windows preserves ordering.
        SendInput(&inputs, mem::size_of::<INPUT>() as i32);

        // Ask the foreground window to update its layout indicator.
        let hwnd = GetForegroundWindow();
        if !hwnd.0.is_null() {
            let lang = action.target_lang;
            if let Some(hkl) = find_hkl(lang) {
                // HKL is *mut c_void; cast to isize to fit LPARAM.
                PostMessageW(
                    hwnd,
                    WM_INPUTLANGCHANGEREQUEST,
                    WPARAM(0),
                    LPARAM(hkl.0 as isize),
                )
                .ok();
            }
        }
    }
}

/// Find an installed HKL whose LANGID (low WORD of the pointer value) matches `lang_id`.
///
/// In `windows 0.58`, `GetKeyboardLayoutList` takes a single `Option<&mut [HKL]>`:
/// - `None`       → returns the number of installed layouts (no fill).
/// - `Some(slice)` → fills the slice, returns the number filled.
fn find_hkl(lang_id: u16) -> Option<HKL> {
    unsafe {
        let count = GetKeyboardLayoutList(None) as usize;
        if count == 0 {
            return None;
        }
        // HKL(0) is a null *mut c_void placeholder.
        let mut list = vec![HKL(std::ptr::null_mut()); count];
        GetKeyboardLayoutList(Some(list.as_mut_slice()));
        // The Windows LANGID is the low WORD of the HKL pointer value.
        list.into_iter().find(|h| (h.0 as usize) as u16 == lang_id)
    }
}

/// Inject a harmless, unassigned virtual key (0xE8) to prevent Windows from
/// opening the Start menu after we swallow a Win+X combo.
///
/// Windows suppresses the Start menu if any key event is observed between the
/// Win key-down and Win key-up.  When our hotkey fires and the action produces
/// no injection (empty buffer), we call this to satisfy that condition.
pub fn suppress_start_menu() {
    let vk = VIRTUAL_KEY(0xE8); // vkE8 — unassigned virtual key code
    let inputs = [
        make_vk(vk, KEYBD_EVENT_FLAGS(0)),
        make_vk(vk, KEYEVENTF_KEYUP),
    ];
    unsafe { SendInput(&inputs, mem::size_of::<INPUT>() as i32); }
}

// ── Input-event constructors ─────────────────────────────────────────────────

fn make_vk(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk:         vk,
                wScan:       0,
                dwFlags:     flags,
                time:        0,
                dwExtraInfo: 0x53574954,
            },
        },
    }
}

fn make_unicode_unit(unit: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                // wVk must be 0 when KEYEVENTF_UNICODE is set.
                wVk:         VIRTUAL_KEY(0),
                // wScan carries the UTF-16 code unit.
                wScan:       unit,
                dwFlags:     KEYEVENTF_UNICODE | flags,
                time:        0,
                dwExtraInfo: 0x53574954,
            },
        },
    }
}
