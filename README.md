# RSwitcher

A lightweight, modern, and automatic keyboard-layout switcher for Windows 10 & 11, built using Rust and Tauri v2.

Automatically detects when text is typed in the wrong keyboard layout (EN↔RU) and fixes it in-place — no full-screen popups, no cloud sync, no telemetry. It runs quietly in the system tray and features a premium dark-themed settings panel.

---

## Features

- **Auto-switching** — detects mismatched layout at word boundaries (Space / Enter / Tab) using a bigram language model.
- **Force switch** — hotkey to manually convert the current word at any time (default: `Win+Shift`).
- **Undo** — hotkey to revert the last automatic or forced conversion (default: `Win+Backspace`).
- **Dynamic tray icon** — shows the active layout flag at a glance (`Ru` or `En`), dimmed when in an excluded application.
- **Exclusions** — per-process exceptions (e.g. skip switching inside terminal emulators or code editors).
- **Autostart** — optional Windows Registry entry to automatically launch RSwitcher at login.
- **Structured log** — per-session log file in `%APPDATA%\rswitcher\logs\` for debugging, auto-rotated after 7 days.

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
* Installation bundles (`.msi` and `.exe` setup installers) are generated in `target/release/bundle/`.

**Build dependencies**: Rust 1.80+ (with MSVC toolchain), Node.js (for `npx tauri` CLI, though no node_modules are required for run-time execution).

---

## Usage

Run `rswitcher.exe`. The application hides to the system tray on startup.

| Action | Result |
|---|---|
| Double-click tray icon | Open settings window |
| Right-click → **Settings** (Настройки / Налаштування) | Open settings window |
| Right-click → **Exit** (Выход / Вихід) | Quit |
| `Win+Shift` (while typing) | Force-convert the current word |
| `Win+Backspace` (after a switch) | Undo the last conversion |

Hotkey virtual key codes can be customized by editing `%APPDATA%\rswitcher\config.json` directly (fields: `hotkey_vk`, `hotkey_win`, `undo_hotkey_vk`, `undo_hotkey_win`).

---

## Architecture

```
rswitcher
├── Cargo.toml            — root workspace configuration
├── corpus/
│   ├── en.txt            — English training corpus (~700 words)
│   └── ru.txt            — Russian training corpus (~700 words)
├── ui/                   — frontend web assets (HTML/CSS/JS)
│   ├── index.html        — settings panel layout
│   ├── style.css         — glassmorphic style design
│   └── main.js           — IPC backend integration scripts
└── src-tauri/            — Rust application backend
    ├── Cargo.toml        — backend dependencies & features
    ├── build.rs          — bigram generator & resource compiler
    ├── tauri.conf.json   — Tauri application configuration
    ├── capabilities/
    │   └── default.json  — window API permissions manifest
    └── src/
        ├── main.rs       — backend entry point, tray icon builder, IPC commands
        ├── buffer.rs     — WordBuffer: VK-code accumulation & mismatch detection
        ├── bigrams.rs    — bigram scoring (includes generated tables from OUT_DIR)
        ├── layout.rs     — EN↔RU VK-code / character mapping, HKL language detection
        ├── switcher.rs   — SendInput sequences: backspace + re-inject + layout change
        ├── settings.rs   — Settings struct, JSON load/save persistence
        ├── logger.rs     — per-launch log file with elapsed timestamps
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
4. **Score** — each candidate is scored with a per-bigram log-probability under its respective language model:

   ```
   score = Σ ln P(cₙ | cₙ₋₁)  /  (len - 1)
   ```

   The bigram probability tables are built at compile time from `corpus/*.txt` with Laplace (add-1) smoothing.

5. **Decide** — a switch is proposed only when the alternative language scores more than `THRESHOLD_PER_BIGRAM = 1.5` nats better per bigram than the current layout's interpretation.
6. **Execute** — `switcher::perform_switch` sends the required `Backspace` keystrokes, re-injects the corrected word as `KEYEVENTF_UNICODE` events, then posts `WM_INPUTLANGCHANGEREQUEST` to switch the active layout.
7. **Undo** — before switching, the original word and erase length are saved in a thread-local `UndoState`; the undo hotkey replays the inverse action.

---

## Configuration

Settings are stored in `%APPDATA%\rswitcher\config.json`:

```json
{
  "enabled": true,
  "exceptions": ["windowsterminal.exe", "code.exe"],
  "hotkey_enabled": true,
  "hotkey_vk": 16,
  "hotkey_win": true,
  "undo_hotkey_toggle": true,
  "undo_hotkey_vk": 8,
  "undo_hotkey_win": true
}
```

| Field | Default | Description |
|---|---|---|
| `enabled` | `true` | Master on/off toggle |
| `exceptions` | `[]` | Lowercase exe names to skip |
| `hotkey_vk` | `16` (Shift) | Force-switch virtual key code |
| `hotkey_win` | `true` | Require Win modifier for force-switch |
| `undo_hotkey_vk` | `8` (Backspace) | Undo virtual key code |
| `undo_hotkey_win` | `true` | Require Win modifier for undo |

---

## Logs

Each run creates a file `%APPDATA%\rswitcher\logs\rswitcher_<unix>_<pid>.log`. Example:

```
[  0:00.000] === RSwitcher started (pid=1234) ===
[  0:00.000] settings: enabled=true exceptions=[] hotkey=Win+Shift (0x10)
[  0:00.000] bigrams: threshold=1.5 nat/bigram  (EN_BIGRAMS[676], RU_BIGRAMS[1024])
[  1:29.846] [DETECT] lang=0x0409 en="ghbdtn" ru="привет" score_en=-7.29 score_ru=-5.21 diff=-2.08 → SWITCH_EN→RU boundary=0x20
[  1:33.626] [DETECT] lang=0x0419 en="hello" ru="руддщ" score_en=-5.28 score_ru=-7.11 diff=+1.82 → SWITCH_RU→EN boundary=0x20
[  2:23.978] === RSwitcher quit via tray menu ===
```

Log files older than 7 days are deleted automatically on the next startup.

---

## License

MIT
