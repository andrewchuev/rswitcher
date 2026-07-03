# RSwitcher
![Presentation](https://img.shields.io/badge/GitHub-Presentation_Mirror-blue)
> **Note**: Development happens on [GitLab](https://gitlab.com/andrew.chuev/rswitcher). This repository is a presentation mirror.


[![Download Latest Release](https://img.shields.io/github/v/release/andrewchuev/rswitcher?style=for-the-badge&label=Download&color=blue)](https://github.com/andrewchuev/rswitcher/releases/latest)

A lightweight, modern, and automatic keyboard-layout switcher for Windows 10 & 11, built using Rust and Tauri v2.

Automatically detects when text is typed in the wrong keyboard layout (EN↔RU↔UA) and fixes it in-place — no full-screen popups, no cloud sync, no telemetry. It runs quietly in the system tray and features a premium dark-themed settings panel organized with tabs.

---

## Features

- **Auto-switching** — detects mismatched layout at word boundaries (Space / Enter / Tab) and on non-translatable keys using a bigram+trigram statistical language model (EN↔RU↔UA).
- **On-the-fly detection** — mid-word switching fires as soon as 4+ characters are buffered. For longer words (8+ characters), the layout recognition threshold is dynamically relaxed to auto-switch them mid-word more reliably.
- **Context-Aware Heuristics** — automatically disables switching when typing programming constructs (e.g., `CamelCase`, `snake_case`) in English layout, and aggressively force-switches when typing such constructs in Cyrillic by mistake.
- **Impossible Sequences Guard** — instantly penalizes physically impossible character sequences (e.g., `ьь`, `аь`) to instantly trigger a layout switch without waiting for further input.
- **Cross-Cyrillic switching** — detects RU↔UA mismatches in addition to Cyrillic↔Latin, using UA-specific letter markers (і / ї / є / ґ) and bigram score deltas to disambiguate.
- **Dictionary Guard** — compile-time generated sorted lists of the top common words in English (3 000), Russian (5 000), and Ukrainian (3 000) for length ≥ 4. Correctly typed common words are immune to layout switching, eliminating false-positive switches.
- **Cross-Layout Dictionary Heuristic** — if a word typed in the English layout is missing from the English common dictionary, but its Cyrillic transliteration exists in the Russian/Ukrainian dictionary, RSwitcher automatically switches the layout, bypassing the bigram score check to prevent false negatives for common Cyrillic words (e.g., `inere` → `штуку`).
- **Short-word dictionary** — dedicated 2–3 character dictionaries (including terms like `usb`, `box`, `ok`, `wc`, `git`, `rl`, `ats`, `тв`, `чер`, `api`, `pdf`, `vat`, `см`, `рдс`, `ндс`) cover short words that bigrams cannot score reliably.
- **Trailing & Internal Punctuation Handling** — bigram models trim trailing punctuation before scoring. Additionally, any non-alphabetic characters found *inside* the word heavily penalize the score, preventing garbled typings (e.g., `htp.vt` for `резюме`) from outscoring purely alphabetic words.
- **User-confirmed corrections** (`word_corrections`) — force-switching a word records its EN key sequence → target language as a permanent correction. Subsequent occurrences of that sequence are switched instantly without consulting the statistical model.
- **Adaptive whitelisting** — words typed 3 times in a row without triggering a switch are automatically added to `ignored_words`. Counts are persisted in `adaptive_counts` across restarts so the threshold accumulates over time.
- **Undo Feedback Whitelist** — pressing the Undo hotkey immediately after an automatic switch restores the original word and adds it to `ignored_words` so it is never switched again.
- **Preferred Cyrillic** (`preferred_cyrillic`) — controls how ambiguous EN→Cyrillic detections are resolved: `auto` (default) requires UA-specific letters to choose Ukrainian; `ru` / `ua` always resolve ties in that direction.
- **App Exceptions** — per-app exclusion list. Processes listed in `exceptions` (e.g. `windowsterminal.exe`) are entirely excluded from auto-switching.
- **Selection-Based Word Deletion** — optional setting (`use_selection_replace`) to erase the mistyped word via `Ctrl+Shift+Left` + `Backspace` instead of sequential `Backspace` keystrokes.
- **Tabs Settings Panel** — a responsive settings window organized into **General**, **Hotkeys**, and **Exceptions** tabs.
- **Multi-modifier hotkey chords** — both the force-switch and undo-switch hotkeys can be assigned any combination of modifier keys (`Ctrl`, `Alt`, `Shift`, `Win`) along with a triggering key, or modifier-only combinations (e.g. `Win+Shift`). Modifiers are temporarily released and restored during input simulation to avoid interference.
- **Official Tauri v2 Plugins** — native single-instance mutex handling and platform folder opening via `tauri-plugin-single-instance` and `tauri-plugin-opener`.
- **System Diagnostics Logging** — per-launch log files with absolute local timestamps, thread labels, OS version (via registry), active keyboard layout codes. A custom panic hook writes fatal panics and backtraces to disk. The logs folder is capped at 50 MB; files older than 7 days are cleaned up on startup.
- **Panic-safe FFI hooks** — keyboard and mouse hook callbacks are wrapped in `catch_unwind` so a Rust panic can never cross the `extern "system"` boundary; on panic the event is forwarded unchanged and typing continues uninterrupted.
- **Atomic config persistence** — settings are written by a background worker (`rswitcher-persist`) that coalesces burst saves and uses a write-to-temp-then-rename strategy, guaranteeing `config.json` is never partially written even under concurrent saves from the hook thread and IPC commands.

---

## Requirements

- Windows 10 / 11 (x86-64)
- English, Russian, and/or Ukrainian keyboard layouts installed

---

## Build & Development

RSwitcher uses **Tauri v2** to render its settings UI. The build process embeds all HTML/CSS/JS frontend assets directly inside the compiled Rust binary.

### Development Mode
```bash
npx tauri dev
```

### Production Build
```bash
npx tauri build
```

- Standalone executable: `target/release/rswitcher.exe`
- NSIS installer bundles: `target/release/bundle/nsis/`

**Build dependencies**: Rust 1.80+ (MSVC toolchain), Node.js (for `npx tauri` CLI).

---

## Usage

Run `rswitcher.exe`. The application hides to the system tray on startup.

| Action | Result |
|---|---|
| Double-click tray icon | Open settings window |
| Right-click → **Settings** (Настройки / Налаштування) | Open settings window |
| Right-click → **Exit** (Выход / Вихід) | Quit |
| `Win+Shift` (default hotkey, customizable to any chord) | Force-convert the current word to the next layout |
| `Win+Backspace` (default hotkey, customizable to any chord) | Undo the last conversion and whitelist the word |

Hotkey virtual key codes and active modifiers (Win, Ctrl, Shift, Alt) can be changed in the settings panel or directly in `%APPDATA%\rswitcher\config.json`.

---

## Architecture

```
rswitcher
├── Cargo.toml            — root workspace configuration
├── assets/
│   ├── en.raw            — English flag tray icon (32×32 RGBA)
│   ├── ru.raw            — Russian flag tray icon (32×32 RGBA)
│   └── ua.raw            — Ukrainian flag tray icon (32×32 RGBA)
├── corpus/
│   ├── en.txt            — English training corpus
│   ├── ru.txt            — Russian training corpus
│   └── ua.txt            — Ukrainian training corpus
├── ui/                   — frontend web assets (HTML/CSS/JS)
│   ├── index.html        — tabbed settings panel layout
│   ├── style.css         — glassmorphic design & animations
│   └── main.js           — IPC backend integration & tab controller
└── src-tauri/            — Rust application backend
    ├── Cargo.toml        — backend dependencies & features
    ├── build.rs          — compile-time bigram/trigram tables & dictionary generator
    ├── tauri.conf.json   — Tauri application configuration
    ├── capabilities/
    │   └── default.json  — window API permissions manifest
    └── src/
        ├── main.rs       — entry point, tray builder, IPC commands, persistence init
        ├── buffer.rs     — WordBuffer: VK-code accumulation, DetectionConfig/Snapshot, mismatch detection
        ├── bigrams.rs    — bigram+trigram scoring (generated tables from OUT_DIR)
        ├── layout.rs     — EN↔RU↔UA VK-code / character mapping, HKL language detection
        ├── switcher.rs   — SendInput sequences: erase + re-inject + layout change
        ├── settings.rs   — Settings struct, HashSet ignored_words, atomic JSON persistence, background save worker
        ├── commands.rs   — Tauri IPC command handlers (save_settings, add/remove exception, …)
        ├── logger.rs     — per-launch log file with absolute timestamps & thread names
        ├── exceptions.rs — foreground process name cache for exclusion checks
        └── autostart.rs  — Windows Registry autostart entry helper
```

### Thread model

```
main thread (Tauri v2 / WebView2 runtime)
│   • Runs the system event loop
│   • Renders the HTML/CSS settings UI inside Edge WebView2 on demand
│   • Handles Tauri IPC commands from the frontend
│
├── rswitcher-hook  (Win32 message loop)
│   • Installs WH_KEYBOARD_LL + WH_MOUSE_LL global hooks
│   • Callbacks wrapped in catch_unwind — panic never crosses FFI boundary
│   • Reads DetectionConfig from SETTINGS once per keystroke (single RwLock acquire)
│   • Calls process_key() on every physical key-down event
│   • Owns WORD_BUF, PREV_WORD_BUF, UNDO, LAST_HWND thread-locals
│   • Calls switcher::perform_switch() → SendInput
│   • Routes hot-path saves through save_async() — never blocks on disk I/O
│
├── rswitcher-persist  (config persistence worker)
│   • Receives Settings via mpsc::channel from any thread
│   • Coalesces burst saves; only writes the latest snapshot
│   • Writes atomically: temp file + rename (FILE_LOCK serialises concurrent callers)
│
└── rswitcher-tray  (background language watcher, 100 ms sleep)
    • Polls the foreground window's HKL layout state
    • Calls tray.set_icon() to update the tray flag dynamically
```

### Switching algorithm

1. **Buffer** — every physical key-down is stored as a `(vk: u16, is_upper: bool, entry_lang: u16)` triple in a thread-local `WordBuffer`.

2. **Early exits** — the buffer is not evaluated if it contains only repeated key-presses (e.g. `jjjjj`), or if the word was already switched mid-word (`has_switched` flag).

3. **Translate** — all buffered VK codes are mapped through the EN, RU, and UA layout tables producing three lowercase candidate strings in one pass. Bigram and trigram scores for all three candidates are computed once and stored in a `DetectionSnapshot`, shared between detection and logging.

4. **Config snapshot** — `DetectionConfig` (containing `ignored_words`, `word_corrections`, `preferred_cyrillic`) is read from `SETTINGS` once at the top of the hook callback and passed through to detection, avoiding repeated `RwLock` acquisitions.

5. **Whitelist check** — if the candidate word for the active layout is found in `ignored_words` (O(1) `HashSet` lookup), no switch is proposed.

6. **User corrections** — if the EN key sequence appears in `word_corrections`, the stored target language is applied immediately, bypassing the statistical model. Force-switching a word records a new entry here.

7. **Dictionary check** — sorted compile-time arrays (`EN_COMMON_WORDS`, `RU_COMMON_WORDS`, `UA_COMMON_WORDS`) are binary-searched. A known-good word in the active layout is left unchanged. A known EN word detected in a Cyrillic-active layout triggers an EN switch without needing the statistical model.

8. **Short-word dictionaries** — 1-character words use a hardcoded allow-list; 2–3-character words are resolved against `COMMON_EN_SHORT`, `COMMON_RU_SHORT`, and `COMMON_UA_SHORT`.

9. **Bigram+trigram scoring** — for words of 3+ characters (4+ for on-the-fly), each candidate is scored with a per-transition weighted log-probability:
   ```
   score = Σ ln P(cₙ | cₙ₋₁, cₙ₋₂)  /  (len - 1)
   ```
   Tables are built at compile time from `corpus/*.txt` with Laplace (add-1) smoothing. Scoring uses fixed-size stack arrays (no heap allocation per call).

10. **Decide** — the score delta is compared against a length-adjusted threshold divided by the `sensitivity` setting. Cross-Cyrillic (RU↔UA) candidates are additionally filtered by UA marker letters and score delta constants (`RU_UA_SCORE_MIN_DELTA`, `RU_UA_SCORE_STRONG_DELTA`). `preferred_cyrillic` breaks ties when both Cyrillic candidates are plausible.

11. **Execute** — `switcher::perform_switch` erases the word (via Backspaces or selection-based delete), re-injects the corrected word as `KEYEVENTF_UNICODE` events, and posts `WM_INPUTLANGCHANGEREQUEST` to flip the active layout.

12. **Case correction** — the output word derives its casing from the original `is_upper` flags: ALL_CAPS → all uppercase, title-case inversion (`hELLO`) → first-uppercase-rest-lowercase, default → lowercase.

13. **Adaptive whitelist** — words typed 3 times without triggering a switch increment a per-word counter in `adaptive_counts` (persisted across restarts). On the third success the word is promoted to `ignored_words` and the counter is removed.

14. **Undo** — the Undo hotkey restores the original word, adds it to `ignored_words`, and asynchronously saves both via `save_async`.

---

## Configuration

Settings are stored in `%APPDATA%\rswitcher\config.json`:

```json
{
  "enabled": true,
  "exceptions": ["windowsterminal.exe"],
  "hotkey_enabled": true,
  "hotkey_vk": 16,
  "hotkey_win": true,
  "undo_hotkey_enabled": true,
  "undo_hotkey_vk": 8,
  "undo_hotkey_win": true,
  "lang": "en",
  "sensitivity": 1.0,
  "use_selection_replace": false,
  "preferred_cyrillic": "auto",
  "ignored_words": ["docker", "kubectl"],
  "word_corrections": {
    "ghbdtn": 1049
  },
  "adaptive_counts": {}
}
```

| Field | Type | Description |
|---|---|---|
| `enabled` | bool | Master on/off switch |
| `exceptions` | string[] | Process names excluded from auto-switching |
| `sensitivity` | float | Threshold multiplier (0.5 = more aggressive, 2.0 = more conservative) |
| `ignored_words` | string[] | Words permanently exempt from switching (deduped, O(1) lookup) |
| `word_corrections` | object | EN key sequence → Windows LANGID (user-confirmed corrections) |
| `adaptive_counts` | object | Intermediate per-word success counters (auto-managed) |
| `preferred_cyrillic` | `"auto"` \| `"ru"` \| `"ua"` | Tie-breaking rule for ambiguous EN→Cyrillic detections |
| `use_selection_replace` | bool | Use `Ctrl+Shift+Left`+`Backspace` to erase instead of multiple Backspaces |
| `hotkey_vk` | int | Virtual key code for the force-switch hotkey (default: 16 = VK_SHIFT) |
| `undo_hotkey_vk` | int | Virtual key code for the undo hotkey (default: 8 = VK_BACK) |

---

## Logs

Each run creates a file `%APPDATA%\rswitcher\logs\rswitcher_<unix>_<pid>.log`. Example:

```
[2026-06-24 10:12:01.003] [  0:00.000] [main] [INFO] === RSwitcher started (pid=5678, path="C:\Program Files\RSwitcher\rswitcher.exe") ===
[2026-06-24 10:12:01.009] [  0:00.006] [main] [INFO] OS: Windows 11 Pro (Build 26200)
[2026-06-24 10:12:01.014] [  0:00.011] [main] [INFO] Active keyboard layouts: [0x0409 (English), 0x0419 (Russian), 0x0422 (Ukrainian)]
[2026-06-24 10:12:01.019] [  0:00.016] [main] [INFO] settings: enabled=true exceptions=[] ignored_words_count=2 sensitivity=1.0
[2026-06-24 10:13:15.482] [  1:14.479] [rswitcher-hook] [INFO] [DETECT] lang=0x0409 en="ghbdtn" ru="привет" ua="гривдн" score_en=-7.29 score_ru=-5.21 score_ua=-9.30 → SWITCH_EN→RU boundary=0x20
[2026-06-24 10:13:28.710] [  1:27.707] [rswitcher-hook] [INFO] [DETECT] lang=0x0409 en="scyedfyyz" ru="сыудукыннз" ua="існування" score_en=-8.45 score_ru=-12.30 score_ua=-4.12 → SWITCH_EN→UA boundary=0x20
[2026-06-24 10:14:05.334] [  2:04.331] [rswitcher-hook] [INFO] [DETECT] lang=0x0419 en="hello" ru="руддщ" ua="рллли" score_en=-5.28 score_ru=-7.11 score_ua=-8.92 → SWITCH_RU→EN boundary=0x20
[2026-06-24 10:14:22.019] [  2:21.016] [rswitcher-hook] [INFO] [FLY-DETECT] lang=0x0409 en="ghbdtn" ru="привет" ua="гривдн" score_en=-7.29 score_ru=-5.21 score_ua=-9.30 → SWITCH_EN→RU (on-the-fly)
[2026-06-24 10:15:01.887] [  3:00.884] [rswitcher-hook] [INFO] [FORCE] lang=0x0409 en="fkujhbnv" ru="алгоритм" ua="алгоритм" score_en=-9.10 score_ru=-4.32 score_ua=-4.41 → EN→RU (0x0419)
[2026-06-24 10:16:00.001] [  3:59.998] [main] [INFO] === RSwitcher quit via tray menu ===
```

Log files older than 7 days, or when the folder exceeds the 50 MB quota, are cleaned up automatically on the next startup.

---

## License

MIT
