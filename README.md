# RSwitcher

A lightweight [Punto Switcher](https://yandex.ru/soft/punto/) alternative for Windows 11, written in Rust.

Automatically detects when text is typed in the wrong keyboard layout (EN↔RU) and fixes it in-place — no full-screen popups, no cloud sync, no telemetry. Lives quietly in the system tray.

---

## Features

- **Auto-switching** — detects mismatched layout at word boundaries (Space / Enter / Tab) using a bigram language model
- **Force switch** — hotkey to manually convert the current word at any time (default: `Win+Shift`)
- **Undo** — hotkey to revert the last automatic or forced conversion (default: `Win+Backspace`)
- **Dynamic tray icon** — shows the active layout at a glance (`Ru` on dark-blue, `En` on dark-red)
- **Exclusions** — per-process exceptions (e.g. skip switching inside terminal emulators)
- **Autostart** — optional Windows Registry entry for launch at login
- **Structured log** — per-session log file in `%APPDATA%\rswitcher\logs\` for debugging, auto-rotated after 7 days

---

## Requirements

- Windows 10 / 11 (x86-64)
- Russian and English keyboard layouts installed

---

## Build

```
cargo build --release
```

The release binary is in `target/release/rswitcher.exe`. No installer required — copy anywhere and run.

**Build dependencies**: Rust 1.70+, MSVC toolchain (for `windows` crate and `winresource`).

---

## Usage

Run `rswitcher.exe`. The app hides to the system tray on startup.

| Action | Result |
|---|---|
| Double-click tray icon | Open settings window |
| Right-click → **Настройки** | Open settings window |
| Right-click → **Выход** | Quit |
| `Win+Shift` (while typing) | Force-convert the current word |
| `Win+Backspace` (after a switch) | Undo the last conversion |

Hotkey virtual key codes can be changed by editing `%APPDATA%\rswitcher\config.json` directly (fields: `hotkey_vk`, `hotkey_win`, `undo_hotkey_vk`, `undo_hotkey_win`).

---

## Architecture

```
rswitcher
├── build.rs              — compile-time bigram table generator
├── corpus/
│   ├── en.txt            — English training corpus (~700 words)
│   └── ru.txt            — Russian training corpus (~700 words)
└── src/
    ├── main.rs           — entry point, hook installation, eframe app, tray watcher
    ├── buffer.rs         — WordBuffer: VK-code accumulation + mismatch detection
    ├── bigrams.rs        — bigram scoring (includes generated tables from OUT_DIR)
    ├── layout.rs         — EN↔RU VK-code / character mapping, HKL language detection
    ├── switcher.rs       — SendInput sequences: backspace + re-inject + layout change
    ├── settings.rs       — Settings struct, JSON load/save
    ├── logger.rs         — per-launch log file with elapsed timestamps
    ├── exceptions.rs     — foreground process name cache for exclusion checks
    └── autostart.rs      — Windows Registry autostart entry
```

### Thread model

```
main thread (eframe / egui)
│   • Runs the settings UI at ~10 Hz when window is visible
│   • Updates tray icon language on each frame
│
├── rswitcher-hook  (Win32 message loop)
│   • Installs WH_KEYBOARD_LL global keyboard hook
│   • Calls process_key() on every physical key-down event
│   • Owns WORD_BUF and UNDO thread-locals
│   • Calls switcher::perform_switch() → SendInput
│
└── rswitcher-tray  (background poller, 100 ms sleep)
    • Drains MenuEvent and TrayIconEvent channels
    • Quit → std::process::exit(0)  (instant, no eframe round-trip)
    • Show → sets SHOW_WINDOW atomic + ctx.request_repaint()
    • Periodic ctx.request_repaint() keeps eframe alive when window is hidden
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

### Injection guard

All `SendInput` events are tagged with a synthetic scan code. The hook checks `LLKHF_INJECTED` on every event and skips its own injected keystrokes, preventing infinite re-processing.

### Language detection

`foreground_lang()` reads the keyboard layout handle (`HKL`) of the thread that owns the foreground window via `GetKeyboardLayout(GetWindowThreadProcessId(GetForegroundWindow()))`. The low 16 bits of the HKL encode the Windows LANGID; the primary language ID (low 10 bits, mask `0x3FF`) is compared against `PRIMARY_EN = 0x0009` and `PRIMARY_RU = 0x0019`, which covers all regional English and Russian variants.

---

## Configuration

Settings are stored in `%APPDATA%\rswitcher\config.json`:

```json
{
  "enabled": true,
  "exceptions": ["WindowsTerminal.exe", "Code.exe"],
  "hotkey_enabled": true,
  "hotkey_vk": 16,
  "hotkey_win": true,
  "undo_hotkey_enabled": true,
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
