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
                let _ = writeln!(writer, "[{}] [{}] {}", stamp, msg.level.as_str(), msg.message);
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
    if let Some(sender) = LOG_SENDER.get() {
        let _ = sender.try_send(LogMessage {
            level,
            message: msg,
            elapsed,
        });
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
