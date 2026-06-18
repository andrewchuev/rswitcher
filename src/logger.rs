//! Per-launch rotating log file.
//!
//! Writes to `%APPDATA%\rswitcher\logs\rswitcher_<unix>_<pid>.log`.
//! Files older than 7 days are removed on startup.
//! Safe to call from any thread; flushes after every write so nothing is lost
//! if the hook thread is killed unexpectedly.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Mutex<Option<…>> so it can be a const-initialised static.
// None = not yet initialised; Some = file is open and ready.
static LOG: Mutex<Option<BufWriter<File>>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();

/// Initialise the logger.  Must be called once from `main()` before any hooks
/// are installed.  Subsequent calls are silently ignored.
pub fn init() {
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

    if let Ok(file) = File::create(path) {
        if let Ok(mut guard) = LOG.lock() {
            *guard = Some(BufWriter::new(file));
        }
    }
}

/// Write one line to the log.  Prepends an elapsed-time stamp `[MM:SS.mmm]`.
/// No-ops silently if the logger was never initialised (e.g. in unit tests).
pub fn write(msg: &str) {
    let elapsed = START.get_or_init(Instant::now).elapsed();
    let stamp = fmt_elapsed(elapsed);

    if let Ok(mut guard) = LOG.lock() {
        if let Some(ref mut w) = *guard {
            let _ = writeln!(w, "[{}] {}", stamp, msg);
            let _ = w.flush();
        }
    }
}

// Convenience macro ────────────────────────────────────────────────────────────

/// Log a formatted message to the current session log file.
///
/// ```ignore
/// log!("switch: {:?} → {:?}", original, new_word);
/// ```
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logger::write(&format!($($arg)*))
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
