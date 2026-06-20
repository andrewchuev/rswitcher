# RSwitcher

A lightweight, modern, and automatic keyboard-layout switcher for Windows 10 & 11, built using Rust and Tauri v2.

Automatically detects when text is typed in the wrong keyboard layout (EN↔RU↔UA) and fixes it in-place — no full-screen popups, no cloud sync, no telemetry. It runs quietly in the system tray and features a premium dark-themed settings panel organized with tabs.

---

## Features

- **Auto-switching** — detects mismatched layout at word boundaries (Space / Enter / Tab) using a bigram statistical language model (EN↔RU↔UA).
- **Dictionary Guard** — compile-time generated list of the top 3000 most common words in English, Russian, and Ukrainian (length $\ge 4$). Correctly typed common words are immune to layout switching, eliminating false-positive switches.
- **Undo Feedback Whitelist** — if you immediately undo an automatic layout switch, the application automatically whitelists that word (`ignored_words` configuration) and avoids switching it again.
- **App-Specific Contexts (Developer Exceptions)** — custom sensitivity filters for IDEs, text editors, and terminals (e.g. `code.exe`, `windowsterminal.exe`). Inside these apps, the minimum word length to trigger auto-switching is raised to 5 and the sensitivity threshold is scaled by 1.5x.
- **Selection-Based Word Deletion** — optional setting (`use_selection_replace`) to replace text by simulating `Ctrl+Shift+Left` and `Backspace` instead of sending multiple sequential `Backspace` keystrokes.
- **Tabs Settings Panel** — a beautiful, responsive, and organized settings window categorized into **General**, **Hotkeys**, and **Exceptions** for easy configuration.
- **Official Tauri v2 Plugins** — native single-instance mutex handling and platform folder opening via `tauri-plugin-single-instance` and `tauri-plugin-opener`.
- **System Diagnostics Logging** — outputs local absolute timestamps, thread labels, OS version (via registry), active keyboard layout codes, and registers a custom panic hook to write fatal panics and backtraces to disk. The logs folder is protected by a 50 MB maximum size quota.
- **Panic-safe FFI hooks** — keyboard and mouse hook callbacks are wrapped in `catch_unwind` so a Rust panic can never cross the `extern "system"` boundary (undefined behaviour); on panic the event is forwarded unchanged and typing continues uninterrupted.
- **Atomic config persistence** — settings are written via a background worker (`rswitcher-persist`) that coalesces burst saves and uses a write-to-temp-then-rename strategy, guaranteeing `config.json` is never partially written even when the hook thread and IPC commands save concurrently.

---

## Requirements

- Windows 10 / 11 (x86-64)
- English, Russian, and Ukrainian keyboard layouts installed

---

## Build & Development

RSwitcher uses **Tauri v2** to render its premium user interface. The build process embeds the HTML/CSS/JS frontend files directly inside a single compiled Rust binary.

### Development Mode
To run the application in development mode with hot-reloading:
```bash
npx tauri dev
```

### Production Build
To build the optimized release binary and installer bundles:
```bash
npx tauri build
```
* The compiled standalone executable is saved to `target/release/rswitcher.exe`.
* Installation bundles (`.exe` setup installers) are generated in `target/release/bundle/nsis/`.

**Build dependencies**: Rust 1.80+ (with MSVC toolchain), Node.js (for `npx tauri` CLI).

---

## Usage

Run `rswitcher.exe`. The application hides to the system tray on startup.

| Action | Result |
|---|---|
| Double-click tray icon | Open settings window |
| Right-click → **Settings** (Настройки / Налаштування) | Open settings window |
| Right-click → **Exit** (Выход / Вихід) | Quit |
| `Win+Shift` (while typing) | Force-convert the current word |
| `Win+Backspace` (after a switch) | Undo the last conversion (automatically whitelists the word) |

Hotkey virtual key codes can be customized by editing `%APPDATA%\rswitcher\config.json` directly or using the settings panel.

---

## Architecture

```
rswitcher
├── Cargo.toml            — root workspace configuration
├── assets/
│   ├── en.raw            — English flag tray icon (32x32 RGBA)
│   ├── ru.raw            — Russian flag tray icon (32x32 RGBA)
│   └── ua.raw            — Ukrainian flag tray icon (32x32 RGBA)
├── corpus/
│   ├── en.txt            — English training corpus (~700 words)
│   ├── ru.txt            — Russian training corpus (~700 words)
│   └── ua.txt            — Ukrainian training corpus (~1.1MB, clean Wikipedia articles)
├── ui/                   — frontend web assets (HTML/CSS/JS)
│   ├── index.html        — tabbed settings panel layout
│   ├── style.css         — glassmorphic style design & animations
│   └── main.js           — IPC backend integration & tab controller scripts
└── src-tauri/            — Rust application backend
    ├── Cargo.toml        — backend dependencies & features
    ├── build.rs          — compile-time dictionary & bigram generator
    ├── tauri.conf.json   — Tauri application configuration
    ├── capabilities/
    │   └── default.json  — window API permissions manifest
    └── src/
        ├── main.rs       — backend entry point, tray icon builder, IPC commands, persistence init
        ├── buffer.rs     — WordBuffer: VK-code accumulation & mismatch detection
        ├── bigrams.rs    — bigram scoring (includes generated tables from OUT_DIR)
        ├── layout.rs     — EN↔RU↔UA VK-code / character mapping, HKL language detection
        ├── switcher.rs   — SendInput sequences: backspace / selection + re-inject + layout change
        ├── settings.rs   — Settings struct, atomic JSON persistence, background save worker
        ├── logger.rs     — per-launch log file with absolute local timestamps & thread names
        ├── exceptions.rs — process name cache for exclusion checks
        └── autostart.rs  — Windows Registry autostart entry helper
```

### Thread model

```
main thread (Tauri v2 / WebView2 runtime)
│   • Runs the system event loop
│   • Displays the HTML/CSS settings UI inside Edge WebView2 when requested
│   • Listens to frontend requests via Tauri IPC Commands
│
├── rswitcher-hook  (Win32 message loop)
│   • Installs WH_KEYBOARD_LL + WH_MOUSE_LL global hooks
│   • Callbacks are protected by catch_unwind — panic never crosses FFI boundary
│   • Calls process_key() on every physical key-down event
│   • Owns WORD_BUF, PREV_WORD_BUF, UNDO and LAST_HWND thread-locals
│   • Calls switcher::perform_switch() → SendInput
│   • Routes hot-path saves through save_async() — never blocks on disk I/O
│
├── rswitcher-persist  (config persistence worker)
│   • Receives Settings via mpsc::channel from any thread
│   • Coalesces burst saves; only writes the latest snapshot
│   • Writes atomically: temp file + rename (FILE_LOCK guarantees serialisation)
│
└── rswitcher-tray  (background language watcher, 100 ms sleep)
    • Polls the foreground window's HKL layout state
    • Calls tray.set_icon() to dynamically update tray flags
```

### Switching algorithm

1. **Buffer** — the hook records every physical key-down as a `(vk: u16, is_upper: bool)` pair in a thread-local `WordBuffer`.
2. **Boundary** — on Space / Enter / Tab / non-translatable key, the buffer is evaluated.
3. **Translate** — all buffered VK codes are mapped through EN, RU, and UA layout tables to produce candidate strings.
4. **Dictionary Check** — checks if the candidate is in the common word dictionary (`EN_COMMON_WORDS`, `RU_COMMON_WORDS`, or `UA_COMMON_WORDS`) or user whitelist (`ignored_words`). If yes, layout switching is bypassed.
5. **Score** — each candidate is scored with a per-bigram log-probability under its respective language model:
   ```
   score = Σ ln P(cₙ | cₙ₋₁)  /  (len - 1)
   ```
   The bigram probability tables are built at compile time from `corpus/*.txt` with Laplace (add-1) smoothing.
7. **Decide** — if the active layout is Russian or Ukrainian, a switch to English is proposed if the English candidate scores better than the threshold. If the active layout is English, both Russian and Ukrainian candidates are scored and compared, and the best-scoring candidate that also beats the threshold is selected.
8. **Execute** — `switcher::perform_switch` deletes the word (using standard Backspaces or selection-based highlights), re-injects the corrected word as `KEYEVENTF_UNICODE` events, then posts `WM_INPUTLANGCHANGEREQUEST` to switch the active layout.
9. **Undo Whitelist** — pressing the Undo hotkey restores the original word and asynchronously appends it to `ignored_words` via the persistence worker.

---

## Configuration

Settings are stored in `%APPDATA%\rswitcher\config.json`:

```json
{
  "enabled": true,
  "exceptions": ["windowsterminal.exe"],
  "ignored_words": [],
  "hotkey_enabled": true,
  "hotkey_vk": 16,
  "hotkey_win": true,
  "undo_hotkey_enabled": true,
  "undo_hotkey_vk": 8,
  "undo_hotkey_win": true,
  "lang": "en",
  "sensitivity": 1.0,
  "use_selection_replace": false
}
```

---

## Logs

Each run creates a file `%APPDATA%\rswitcher\logs\rswitcher_<unix>_<pid>.log`. Example:

```
[2026-06-19 21:56:15.519] [  0:00.000] [main] [INFO] === RSwitcher started (pid=1234, path="C:\\Program Files\\RSwitcher\\rswitcher.exe") ===
[2026-06-19 21:56:15.525] [  0:00.006] [main] [INFO] OS: Windows 11 Pro (Build 22631)
[2026-06-19 21:56:15.530] [  0:00.011] [main] [INFO] Active keyboard layouts: [0x0409 (English), 0x0419 (Russian), 0x0422 (Ukrainian)]
[2026-06-19 21:56:15.535] [  0:00.016] [main] [INFO] settings: enabled=true exceptions=["windowsterminal.exe"] ignored_words_count=0 sensitivity=1.0 use_selection_replace=false
[2026-06-19 21:57:44.846] [  1:29.327] [rswitcher-hook] [INFO] [DETECT] lang=0x0409 en="ghbdtn" ru="привет" ua="гривдн" score_en=-7.29 score_ru=-5.21 score_ua=-9.30 → SWITCH_EN→RU boundary=0x20
[2026-06-19 21:57:55.120] [  1:39.601] [rswitcher-hook] [INFO] [DETECT] lang=0x0409 en="scyedfyyz" ru="сыудукыннз" ua="існування" score_en=-8.45 score_ru=-12.30 score_ua=-4.12 → SWITCH_EN→UA boundary=0x20
[2026-06-19 21:58:33.626] [  2:18.107] [rswitcher-hook] [INFO] [DETECT] lang=0x0419 en="hello" ru="руддщ" ua="рллли" score_en=-5.28 score_ru=-7.11 score_ua=-8.92 → SWITCH_RU→EN boundary=0x20
[2026-06-19 21:59:23.978] [  3:08.459] [main] [INFO] === RSwitcher quit via tray menu ===
```

Log files older than 7 days, or exceeding the 50 MB total folder quota size, are cleaned up automatically on the next startup.

---

## License

MIT
