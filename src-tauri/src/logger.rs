//! Per-launch rotating log file.
//!
//! Writes to `%APPDATA%\rswitcher\logs\rswitcher_<unix>_<pid>.log`.
//! Files older than 7 days are removed on startup.
//! Safe to call from any thread; uses a non-blocking channel to offload all I/O
//! to a background writer thread.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::{
    core::w,
    Win32::{
        System::{
            Registry::{
                RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
            },
            SystemInformation::GetLocalTime,
        },
    },
};

const CURRENT_VERSION_KEY: windows::core::PCWSTR = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
const PRODUCT_NAME: windows::core::PCWSTR = w!("ProductName");
const CURRENT_BUILD: windows::core::PCWSTR = w!("CurrentBuild");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Error => "ERROR",
        }
    }
}

pub struct LogMessage {
    pub level: LogLevel,
    pub message: String,
    pub elapsed: Duration,
    pub thread: String,
    pub local_time: String,
}

static LOG_SENDER: OnceLock<mpsc::SyncSender<LogMessage>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static LOGGER_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialise the logger.  Must be called once from `main()` before any hooks
/// are installed.  Subsequent calls are silently ignored.
pub fn init() {
    if LOGGER_INITIALIZED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    START.get_or_init(Instant::now);

    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let log_dir = std::path::PathBuf::from(appdata)
        .join("rswitcher")
        .join("logs");

    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }

    cleanup_old(&log_dir);
    limit_logs_dir_size(&log_dir, 50 * 1024 * 1024); // Limit total logs size to 50 MB

    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let pid = std::process::id();
    let path = log_dir.join(format!("rswitcher_{}_{}.log", unix_secs, pid));

    let file = match File::create(path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let (tx, rx) = mpsc::sync_channel::<LogMessage>(1024);
    if LOG_SENDER.set(tx).is_err() {
        return;
    }

    std::thread::Builder::new()
        .name("rswitcher-logger".into())
        .spawn(move || {
            let mut writer = BufWriter::new(file);
            while let Ok(msg) = rx.recv() {
                let stamp = fmt_elapsed(msg.elapsed);
                let _ = writeln!(
                    writer,
                    "[{}] [{}] [{}] [{}] {}",
                    msg.local_time, stamp, msg.thread, msg.level.as_str(), msg.message
                );
                let _ = writer.flush();
            }
        })
        .expect("failed to spawn logger thread");
}

/// Write one message to the log queue. Prepends elapsed duration.
/// Uses a non-blocking `try_send` to ensure the calling thread (e.g. hook thread)
/// is never blocked by slow disk I/O.
pub fn write(level: LogLevel, msg: String) {
    let elapsed = START.get_or_init(Instant::now).elapsed();
    let thread = {
        let t = std::thread::current();
        t.name().unwrap_or(&format!("{:?}", t.id())).to_string()
    };
    let local_time = current_local_time_str();
    if let Some(sender) = LOG_SENDER.get() {
        let _ = sender.try_send(LogMessage {
            level,
            message: msg,
            elapsed,
            thread,
            local_time,
        });
    }
}

/// Set up a custom panic hook to log fatal application panics along with backtrace.
pub fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "unknown"
        };
        let location = info.location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".into());

        let backtrace = std::backtrace::Backtrace::force_capture();

        write(LogLevel::Error, format!("FATAL PANIC in {}: {}\nBacktrace:\n{}", location, payload, backtrace));
        eprintln!("FATAL PANIC in {}: {}\nBacktrace:\n{}", location, payload, backtrace);

        // Allow some time for logger background thread to write and flush the buffer
        std::thread::sleep(Duration::from_millis(200));
    }));
}

/// Retrieve the OS version string via Windows registry.
pub fn get_windows_version() -> String {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            CURRENT_VERSION_KEY,
            0u32,
            KEY_READ,
            &mut key,
        )
        .ok()
        .is_ok()
        {
            let mut buf = vec![0u16; 256];
            let mut val_type = REG_SZ;

            // Query ProductName
            let mut len = (buf.len() * 2) as u32;
            let product_name = if RegQueryValueExW(
                key,
                PRODUCT_NAME,
                None,
                Some(&mut val_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut len),
            )
            .ok()
            .is_ok()
            {
                String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)])
            } else {
                "Windows".to_string()
            };

            // Query CurrentBuild
            let mut build_len = (buf.len() * 2) as u32;
            let build_number = if RegQueryValueExW(
                key,
                CURRENT_BUILD,
                None,
                Some(&mut val_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut build_len),
            )
            .ok()
            .is_ok()
            {
                format!("(Build {})", String::from_utf16_lossy(&buf[..(build_len as usize / 2).saturating_sub(1)]).trim())
            } else {
                String::new()
            };

            let _ = RegCloseKey(key);
            format!("{} {}", product_name.trim(), build_number).trim().to_string()
        } else {
            "Windows (Unknown version)".to_string()
        }
    }
}

/// Retrieve and log active keyboard layouts.
pub fn log_keyboard_layouts() {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList;
    unsafe {
        let count = GetKeyboardLayoutList(None);
        if count > 0 {
            let mut layouts = vec![windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()); count as usize];
            let count2 = GetKeyboardLayoutList(Some(&mut layouts));
            let mut names = Vec::new();
            for hkl in layouts.iter().take(count2 as usize) {
                let lang_id = (hkl.0 as usize) as u16;
                let name = if crate::layout::hkl_is_russian(lang_id) {
                    format!("{:#06x} (Russian)", lang_id)
                } else if crate::layout::hkl_is_english(lang_id) {
                    format!("{:#06x} (English)", lang_id)
                } else {
                    format!("{:#06x}", lang_id)
                };
                names.push(name);
            }
            write(LogLevel::Info, format!("Active keyboard layouts: [{}]", names.join(", ")));
        } else {
            write(LogLevel::Error, "Failed to query keyboard layout list".to_string());
        }
    }
}

// Convenience macros ──────────────────────────────────────────────────────────

/// Log a debug message (only captured in debug builds).
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        $crate::logger::write($crate::logger::LogLevel::Debug, format!($($arg)*))
    };
}

/// Log an informational message.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logger::write($crate::logger::LogLevel::Info, format!($($arg)*))
    };
}

/// Log an error message.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logger::write($crate::logger::LogLevel::Error, format!($($arg)*))
    };
}

// Helpers ─────────────────────────────────────────────────────────────────────

fn fmt_elapsed(d: Duration) -> String {
    let total_s = d.as_secs();
    let ms = d.subsec_millis();
    let mins = total_s / 60;
    let secs = total_s % 60;
    format!("{:3}:{:02}.{:03}", mins, secs, ms)
}

fn current_local_time_str() -> String {
    unsafe {
        let st = GetLocalTime();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }
}

fn cleanup_old(dir: &std::path::Path) {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(7 * 24 * 3600))
        .unwrap_or(UNIX_EPOCH);

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

fn limit_logs_dir_size(dir: &std::path::Path, max_size_bytes: u64) {
    if let Ok(entries) = fs::read_dir(dir) {
        let mut files: Vec<(std::path::PathBuf, u64, SystemTime)> = Vec::new();
        let mut total_size: u64 = 0;

        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    let path = entry.path();
                    let size = meta.len();
                    let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                    total_size += size;
                    files.push((path, size, modified));
                }
            }
        }

        if total_size > max_size_bytes {
            files.sort_by_key(|f| f.2); // sort by modified time ascending (oldest first)
            for (path, size, _) in files {
                if total_size <= max_size_bytes {
                    break;
                }
                if fs::remove_file(&path).is_ok() {
                    total_size = total_size.saturating_sub(size);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger_diagnostics() {
        init();
        write(LogLevel::Info, "Testing logging system!".to_string());
        
        let os = get_windows_version();
        assert!(!os.is_empty(), "OS version should not be empty");
        
        // Wait a brief moment for the background thread to write
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    #[should_panic(expected = "Intentional diagnostic panic")]
    fn test_panic_hook() {
        init();
        setup_panic_hook();
        panic!("Intentional diagnostic panic");
    }
}

