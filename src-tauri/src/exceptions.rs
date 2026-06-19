use std::cell::RefCell;

use windows::core::PWSTR;
use windows::Win32::{
    Foundation::{CloseHandle, BOOL, HWND, LPARAM},
    System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    },
};

// ─────────────────────────────────────────────────────────────────────────────
// Running-process enumeration (for the exceptions picker)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct RunningApp {
    pub exe:   String,
    pub title: String,
}

/// Enumerate visible top-level windows and return a de-duplicated, sorted list
/// of (exe_name, window_title) pairs.  Our own process is excluded.
pub fn enumerate_visible_apps() -> Vec<RunningApp> {
    let self_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();

    let mut raw: Vec<RunningApp> = Vec::new();
    let ptr = &mut raw as *mut Vec<RunningApp> as isize;
    unsafe { let _ = EnumWindows(Some(enum_callback), LPARAM(ptr)); }

    // De-duplicate by exe name; keep first seen title.
    let mut seen = std::collections::HashMap::<String, String>::new();
    for app in raw {
        if app.exe != self_exe {
            seen.entry(app.exe).or_insert(app.title);
        }
    }

    let mut result: Vec<RunningApp> = seen
        .into_iter()
        .map(|(exe, title)| RunningApp { exe, title })
        .collect();
    result.sort_by(|a, b| a.exe.cmp(&b.exe));
    result
}

unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut buf = [0u16; 256];
    let len = GetWindowTextW(hwnd, &mut buf);
    if len <= 0 {
        return BOOL(1);
    }
    if let Some(exe) = exe_name_for_hwnd(hwnd) {
        let title = String::from_utf16_lossy(&buf[..len as usize]);
        let list = &mut *(lparam.0 as *mut Vec<RunningApp>);
        list.push(RunningApp { exe, title });
    }
    BOOL(1)
}

// ── Per-thread cache ─────────────────────────────────────────────────────────
// Key: foreground HWND as usize.  Invalidated when the active window changes.
// Only the hook thread calls this code, so no cross-thread coordination needed.

thread_local! {
    static CACHE: RefCell<Option<(usize, String)>> = const { RefCell::new(None) };
}


/// Returns the foreground window's executable name (lowercase, no path), or
/// `None` if the window handle is null or the name cannot be resolved.
/// Used by the logging layer in `main.rs`.
pub fn foreground_exe_name() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let name = cached_foreground_exe();
    if name.is_empty() { None } else { Some(name) }
}

fn cached_foreground_exe() -> String {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return String::new();
    }
    let key = hwnd.0 as usize;

    CACHE.with(|cell| {
        // Check cache under a short-lived immutable borrow.
        {
            if let Some((k, ref name)) = *cell.borrow() {
                if k == key {
                    return name.clone();
                }
            }
        }
        // Cache miss: query the OS and update.
        let name = exe_name_for_hwnd(hwnd).unwrap_or_default();
        *cell.borrow_mut() = Some((key, name.clone()));
        name
    })
}

/// Query the executable name (lowercase, no path) for the window `hwnd`.
///
/// # Safety
/// All Win32 calls respect the documented preconditions.
fn exe_name_for_hwnd(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        // PROCESS_QUERY_LIMITED_INFORMATION is enough for QueryFullProcessImageNameW
        // and does not require SeDebugPrivilege.
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid).ok()?;

        let mut buf = [0u16; 260]; // MAX_PATH
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32, // Win32-style path (e.g. C:\Windows\notepad.exe)
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();

        // Always close the handle, even if the query failed.
        CloseHandle(handle).ok();

        if !ok || size == 0 {
            return None;
        }

        let safe_size = (size as usize).min(buf.len());
        let full_path = String::from_utf16_lossy(&buf[..safe_size]);
        // Extract just "notepad.exe" from the full path.
        full_path.split('\\').next_back().map(|s| s.to_lowercase())
    }
}

pub fn is_current_process_elevated() -> bool {
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_QUERY, TOKEN_ELEVATION};
    use windows::Win32::Foundation::HANDLE;

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut return_length = 0u32;

        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        ).is_ok();

        let _ = CloseHandle(token);

        ok && elevation.TokenIsElevated != 0
    }
}

pub fn is_active_window_elevated() -> bool {
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_QUERY, TOKEN_ELEVATION};
    use windows::Win32::Foundation::HANDLE;

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return false;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return false;
        }

        let process_handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid) {
            Ok(h) => h,
            Err(_) => {
                // If we cannot open the process, it might be elevated and we are standard.
                return true;
            }
        };

        let mut token = HANDLE::default();
        if OpenProcessToken(process_handle, TOKEN_QUERY, &mut token).is_err() {
            let _ = CloseHandle(process_handle);
            // Opening token fails with ACCESS_DENIED if the target process is elevated and we are not.
            return true;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut return_length = 0u32;

        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        ).is_ok();

        let _ = CloseHandle(token);
        let _ = CloseHandle(process_handle);

        ok && elevation.TokenIsElevated != 0
    }
}
