# RSwitcher

A lightweight, modern, and automatic keyboard-layout switcher for Windows 10 & 11, built using Rust and Tauri v2.

Automatically detects when text is typed in the wrong keyboard layout (EN↔RU) and fixes it in-place — no full-screen popups, no cloud sync, no telemetry. It runs quietly in the system tray and features a premium dark-themed settings panel organized with tabs.

---

## Features

- **Auto-switching** — detects mismatched layout at word boundaries (Space / Enter / Tab) using a bigram statistical language model.
- **Dictionary Guard** — compile-time generated list of the top 3000 most common words in English and Russian (length $\ge 4$). Correctly typed common words are immune to layout switching, eliminating false-positive switches.
- **Undo Feedback Whitelist** — if you immediately undo an automatic layout switch, the application automatically whitelists that word (`ignored_words` configuration) and avoids switching it again.
- **App-Specific Contexts (Developer Exceptions)** — custom sensitivity filters for IDEs, text editors, and terminals (e.g. `code.exe`, `windowsterminal.exe`). Inside these apps, the minimum word length to trigger auto-switching is raised to 5 and the sensitivity threshold is scaled by 1.5x.
- **Selection-Based Word Deletion** — optional setting (`use_selection_replace`) to replace text by simulating `Ctrl+Shift+Left` and `Backspace` instead of sending multiple sequential `Backspace` keystrokes.
- **Tabs Settings Panel** — a beautiful, responsive, and organized settings window categorized into **General**, **Hotkeys**, and **Exceptions** for easy configuration.
- **Official Tauri v2 Plugins** — native single-instance mutex handling and platform folder opening via `tauri-plugin-single-instance` and `tauri-plugin-opener`.
- **System Diagnostics Logging** — outputs local absolute timestamps, thread labels, OS version (via registry), active keyboard layout codes, and registers a custom panic hook to write fatal panics and backtraces to disk. The logs folder is protected by a 50 MB maximum size quota.

---

## Requirements

- Windows 10 / 11 (x86-64)
- Russian and English keyboard layouts installed

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
├── corpus/
│   ├── en.txt            — English training corpus (~700 words)
│   └── ru.txt            — Russian training corpus (~700 words)
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
        ├── main.rs       — backend entry point, tray icon builder, IPC commands, panic hook
        ├── buffer.rs     — WordBuffer: VK-code accumulation & mismatch detection
        ├── bigrams.rs    — bigram scoring (includes generated tables from OUT_DIR)
        ├── layout.rs     — EN↔RU VK-code / character mapping, HKL language detection
        ├── switcher.rs   — SendInput sequences: backspace / selection + re-inject + layout change
        ├── settings.rs   — Settings struct, JSON load/save persistence
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
│   • Installs WH_KEYBOARD_LL global keyboard hook
│   • Calls process_key() on every physical key-down event
│   • Owns WORD_BUF and UNDO thread-locals
│   • Calls switcher::perform_switch() → SendInput
│
└── rswitcher-tray  (background language watcher, 100 ms sleep)
    • Polls the foreground window's HKL layout state
    • Calls tray.set_icon() to dynamically update tray flags
```

### Switching algorithm

1. **Buffer** — the hook records every physical key-down as a `(vk: u16, is_upper: bool)` pair in a thread-local `WordBuffer`.
2. **Boundary** — on Space / Enter / Tab / non-translatable key, the buffer is evaluated.
3. **Translate** — all buffered VK codes are mapped through both the EN and RU layout tables to produce two candidate strings.
4. **Dictionary Check** — checks if the candidate is in the common word dictionary (`EN_COMMON_WORDS` or `RU_COMMON_WORDS`) or user whitelist (`ignored_words`). If yes, layout switching is bypassed.
5. **Context Check** — if the active window matches developer exceptions (`dev_exceptions`):
   - Minimum switching length is raised from 4 to 5 characters.
   - Sensitivity threshold required to trigger a switch is multiplied by 1.5x.
6. **Score** — each candidate is scored with a per-bigram log-probability under its respective language model:
   ```
   score = Σ ln P(cₙ | cₙ₋₁)  /  (len - 1)
   ```
   The bigram probability tables are built at compile time from `corpus/*.txt` with Laplace (add-1) smoothing.
7. **Decide** — a switch is proposed only when the alternative language scores better than the threshold.
8. **Execute** — `switcher::perform_switch` deletes the word (using standard Backspaces or selection-based highlights), re-injects the corrected word as `KEYEVENTF_UNICODE` events, then posts `WM_INPUTLANGCHANGEREQUEST` to switch the active layout.
9. **Undo Whitelist** — pressing the Undo hotkey restores the original word and appends it to `ignored_words` on a background thread.

---

## Configuration

Settings are stored in `%APPDATA%\rswitcher\config.json`:

```json
{
  "enabled": true,
  "exceptions": ["windowsterminal.exe"],
  "dev_exceptions": ["code.exe", "idea64.exe", "visualstudio.exe", "cargo.exe"],
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
[2026-06-19 21:56:15.530] [  0:00.011] [main] [INFO] Active keyboard layouts: [0x0409 (English), 0x0419 (Russian)]
[2026-06-19 21:56:15.535] [  0:00.016] [main] [INFO] settings: enabled=true exceptions=["windowsterminal.exe"] dev_exceptions=["code.exe"] ignored_words_count=0 sensitivity=1.0 use_selection_replace=false
[2026-06-19 21:57:44.846] [  1:29.327] [rswitcher-hook] [INFO] [DETECT] lang=0x0409 en="ghbdtn" ru="привет" score_en=-7.29 score_ru=-5.21 diff=-2.08 → SWITCH_EN→RU
[2026-06-19 21:58:33.626] [  2:18.107] [rswitcher-hook] [INFO] [DETECT] lang=0x0419 en="hello" ru="руддщ" score_en=-5.28 score_ru=-7.11 diff=+1.82 → SWITCH_RU→EN
[2026-06-19 21:59:23.978] [  3:08.459] [main] [INFO] === RSwitcher quit via tray menu ===
```

Log files older than 7 days, or exceeding the 50 MB total folder quota size, are cleaned up automatically on the next startup.

---

## License

MIT
