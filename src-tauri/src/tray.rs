use crate::globals::SETTINGS;
use crate::{exceptions, layout};

use windows::Win32::UI::{
    Input::KeyboardAndMouse::GetKeyboardLayout,
    WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangIcon {
    Ru,
    En,
    Ua,
}

pub fn make_lang_icon(lang: LangIcon, dimmed: bool) -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let bytes: &[u8] = match lang {
        LangIcon::Ru => include_bytes!("../../assets/ru.raw"),
        LangIcon::En => include_bytes!("../../assets/en.raw"),
        LangIcon::Ua => include_bytes!("../../assets/ua.raw"),
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

pub unsafe fn foreground_lang() -> u16 {
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return 0;
    }
    let tid = GetWindowThreadProcessId(hwnd, None);
    let hkl = GetKeyboardLayout(tid);
    (hkl.0 as usize) as u16
}

/// Spawn a background thread that updates the language icon directly.
pub fn spawn_tray_watcher(app_handle: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("rswitcher-tray".into())
        .spawn(move || {
            let is_self_elevated = exceptions::is_current_process_elevated();
            let mut last_icon: Option<(LangIcon, bool)> = None;
            loop {
                let lang_word = unsafe { foreground_lang() };
                let new_lang = if layout::hkl_is_russian(lang_word) {
                    Some(LangIcon::Ru)
                } else if layout::hkl_is_english(lang_word) {
                    Some(LangIcon::En)
                } else if layout::hkl_is_ukrainian(lang_word) {
                    Some(LangIcon::Ua)
                } else {
                    None
                };

                let is_exception = SETTINGS
                    .get()
                    .and_then(|s| s.try_read().ok())
                    .map(|s| {
                        !s.exceptions.is_empty()
                            && exceptions::foreground_exe_name()
                                .map(|name| s.exceptions.contains(&name))
                                .unwrap_or(false)
                    })
                    .unwrap_or(false);

                let is_active_elevated = !is_self_elevated && exceptions::is_active_window_elevated();
                let dimmed = is_exception || is_active_elevated;

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
