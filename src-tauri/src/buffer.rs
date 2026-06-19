use crate::{bigrams, layout};

const COMMON_RU_SHORT: &[&str] = &[
    "без", "был", "быт", "вас", "век", "все", "всю", "всё", "вы", "где",
    "да", "два", "для", "до", "дом", "его", "ее", "ей", "ему", "еще",
    "ещё", "её", "же", "за", "из", "изо", "или", "им", "ими", "имя",
    "их", "как", "кто", "ли", "мне", "мог", "мои", "моя", "мы", "на",
    "нам", "нас", "ней", "нет", "них", "но", "об", "обо", "он", "она",
    "они", "оно", "от", "ото", "под", "при", "про", "раз", "сам", "сих",
    "со", "так", "там", "те", "тем", "тех", "то", "той", "том", "тот",
    "три", "тут", "ты", "уж", "уже", "чем", "что", "это", "эту"
];

const COMMON_EN_SHORT: &[&str] = &[
    "add", "am", "an", "and", "any", "api", "app", "are", "as", "at",
    "bad", "bat", "be", "big", "bin", "but", "by", "can", "cd", "cfg",
    "cli", "cmd", "css", "csv", "day", "db", "dev", "did", "dir", "dns",
    "do", "doc", "dom", "env", "err", "few", "for", "ftp", "get", "git",
    "go", "had", "has", "he", "her", "him", "his", "how", "hub", "id",
    "if", "in", "io", "ip", "is", "it", "its", "js", "key", "let",
    "lib", "log", "low", "ls", "mac", "mad", "map", "may", "md", "me",
    "my", "net", "new", "no", "not", "now", "npm", "of", "off", "old",
    "on", "one", "or", "org", "os", "our", "out", "own", "pdf", "pkg",
    "png", "pr", "py", "red", "rs", "run", "sad", "say", "see", "sh",
    "she", "so", "sql", "src", "ssh", "ssl", "sys", "tcp", "the", "tls",
    "too", "try", "ts", "two", "txt", "udp", "ui", "up", "uri", "url",
    "use", "ux", "val", "vps", "was", "way", "we", "web", "who", "win",
    "wp", "xml", "yes", "you", "zip"
];

include!(concat!(env!("OUT_DIR"), "/dictionaries_gen.rs"));

#[derive(Debug, Clone)]
struct Entry {
    vk: u16,
    is_upper: bool,
}

#[derive(Debug, Clone)]
pub struct SwitchAction {
    /// Number of Backspace events to send before the replacement text.
    pub backspaces: usize,
    /// The replacement word (injected via KEYEVENTF_UNICODE).
    pub new_word: String,
    /// `true`  → switch layout to Russian after injection.
    /// `false` → switch to English.
    pub to_ru: bool,
    /// The original (mistyped) word — kept for the undo hotkey.
    pub original_word: String,
}

/// Snapshot of a detection attempt, suitable for structured logging.
/// Computed cheaply without a switch decision; call `detection_snapshot` before
/// `detect_mismatch` so the log entry can cover both the "switch" and "skip" paths.
#[derive(Debug)]
pub struct DetectionSnapshot {
    /// What the buffered VK codes would produce in the English layout (lowercase).
    pub en_word: String,
    /// What they would produce in the Russian layout (lowercase).
    pub ru_word: String,
    /// Per-bigram log-probability score for the English interpretation.
    pub score_en: f32,
    /// Per-bigram log-probability score for the Russian interpretation.
    pub score_ru: f32,
    /// Number of buffered VK entries.
    pub len: usize,
}

pub fn switching_threshold(len: usize) -> f32 {
    if len <= 4 {
        1.5
    } else if len == 5 {
        1.2
    } else if len == 6 {
        1.0
    } else if len == 7 {
        0.9
    } else {
        0.8
    }
}

#[derive(Debug, Default, Clone)]
pub struct WordBuffer {
    entries: Vec<Entry>,
}

impl WordBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a translatable key.  `is_upper` = shift_held XOR caps_lock_on.
    pub fn push(&mut self, vk: u16, is_upper: bool) {
        self.entries.push(Entry { vk, is_upper });
    }

    pub fn pop(&mut self) {
        self.entries.pop();
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Compute EN/RU translations and bigram scores for the current buffer.
    /// Returns `None` when the buffer is empty.  Always uses lowercase for
    /// scoring (case does not affect bigram statistics).
    pub fn detection_snapshot(&self) -> Option<DetectionSnapshot> {
        if self.entries.is_empty() {
            return None;
        }
        let en: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_en(e.vk, false))
            .collect();
        let ru: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_ru(e.vk, false))
            .collect();
        Some(DetectionSnapshot {
            score_en: bigrams::score_en(&en),
            score_ru: bigrams::score_ru(&ru),
            len: self.entries.len(),
            en_word: en,
            ru_word: ru,
        })
    }

    /// Auto-detect mismatch at a word boundary (requires >= 2 buffered keys).
    ///
    /// Long words (>= 4 chars) are scored using the bigram model.
    /// Short words (2-3 chars) are matched against a high-frequency dictionary.
    ///
    /// `active_lang` is the low 16-bit word of the foreground window's HKL.
    #[allow(dead_code)]
    pub fn detect_mismatch(&self, active_lang: u16) -> Option<SwitchAction> {
        self.detect_impl(active_lang, 1.0)
    }

    pub fn detect_mismatch_with_sensitivity(&self, active_lang: u16, sensitivity: f32) -> Option<SwitchAction> {
        self.detect_impl(active_lang, sensitivity)
    }

    /// Hotkey forced switch (requires >= 1 buffered key).
    ///
    /// Unlike auto-detection, bypasses the bigram score threshold entirely —
    /// the user explicitly requested the switch, so we trust their intent.
    /// Returns None only when the buffer is empty or the translation is invalid
    /// (e.g. the keys don't map to valid Cyrillic in the RU interpretation).
    pub fn force_switch(&self, active_lang: u16) -> Option<SwitchAction> {
        if self.entries.is_empty() {
            return None;
        }
        let en: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_en(e.vk, e.is_upper))
            .collect();
        let ru: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_ru(e.vk, e.is_upper))
            .collect();
        if en.chars().count() != self.entries.len() || ru.chars().count() != self.entries.len() {
            return None;
        }
        if !ru.chars().all(is_cyrillic) {
            return None;
        }
        if layout::hkl_is_russian(active_lang) {
            Some(SwitchAction {
                backspaces: self.len(),
                new_word: en,
                to_ru: false,
                original_word: ru,
            })
        } else if layout::hkl_is_english(active_lang) {
            Some(SwitchAction {
                backspaces: self.len(),
                new_word: ru,
                to_ru: true,
                original_word: en,
            })
        } else {
            None
        }
    }

    fn detect_impl(&self, active_lang: u16, sensitivity: f32) -> Option<SwitchAction> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }

        // Translate VK codes through both layouts (preserving case for the output word).
        let en: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_en(e.vk, e.is_upper))
            .collect();
        let ru: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_ru(e.vk, e.is_upper))
            .collect();

        if en.chars().count() != len || ru.chars().count() != len {
            return None;
        }

        let en_lower = en.to_lowercase();
        let ru_lower = ru.to_lowercase();

        // ── 0. Check whitelisted / ignored words (Undo feedback loop) ────────
        let (ignored_words, dev_exceptions) = crate::globals::SETTINGS
            .get()
            .and_then(|s| s.try_read().ok())
            .map(|s| (s.ignored_words.clone(), s.dev_exceptions.clone()))
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        let word_to_check = if layout::hkl_is_russian(active_lang) {
            &ru_lower
        } else {
            &en_lower
        };

        if ignored_words.contains(word_to_check) {
            return None;
        }

        // ── 1. Check if the word is a valid word in the current layout ────────
        // (Dictionary Guard using EN_COMMON_WORDS / RU_COMMON_WORDS)
        if layout::hkl_is_russian(active_lang) && RU_COMMON_WORDS.binary_search(&ru_lower.as_str()).is_ok() {
            return None; // Valid Russian word, do not switch
        } else if layout::hkl_is_english(active_lang) && EN_COMMON_WORDS.binary_search(&en_lower.as_str()).is_ok() {
            return None; // Valid English word, do not switch
        }

        // ── 2. Check active app specific adjustments ─────────────────────────
        let active_exe = crate::exceptions::foreground_exe_name();
        let is_dev_app = active_exe
            .as_ref()
            .map(|exe| dev_exceptions.contains(exe))
            .unwrap_or(false);

        let adjusted_min_len = if is_dev_app { 5 } else { 4 };
        let dev_threshold_multiplier = if is_dev_app { 1.5 } else { 1.0 };

        // ── 3. Check for single-letter words (1 char) ────────────────────────
        if len == 1 {
            if is_dev_app {
                return None;
            }
            let common_ru_single = ["в", "и", "а", "о", "с", "у", "я", "к"];
            let common_en_single = ["a", "i"];
            if layout::hkl_is_russian(active_lang) {
                if !common_ru_single.contains(&ru_lower.as_str()) && common_en_single.contains(&en_lower.as_str()) {
                    return Some(SwitchAction {
                        backspaces: len,
                        new_word: en,
                        to_ru: false,
                        original_word: ru,
                    });
                }
            } else if layout::hkl_is_english(active_lang)
                && !common_en_single.contains(&en_lower.as_str())
                && common_ru_single.contains(&ru_lower.as_str())
            {
                return Some(SwitchAction {
                    backspaces: len,
                    new_word: ru,
                    to_ru: true,
                    original_word: en,
                });
            }
            return None;
        }

        // ── 4. Dictionary-based check for short words (2-3 chars) ─────────────
        if len == 2 || len == 3 {
            if is_dev_app {
                return None;
            }
            if layout::hkl_is_russian(active_lang) {
                let is_common_en = COMMON_EN_SHORT.binary_search(&en_lower.as_str()).is_ok();
                let is_common_ru = COMMON_RU_SHORT.binary_search(&ru_lower.as_str()).is_ok();
                if is_common_en && !is_common_ru {
                    return Some(SwitchAction {
                        backspaces: len,
                        new_word: en,
                        to_ru: false,
                        original_word: ru,
                    });
                }
            } else if layout::hkl_is_english(active_lang) {
                let is_common_ru = COMMON_RU_SHORT.binary_search(&ru_lower.as_str()).is_ok();
                let is_common_en = COMMON_EN_SHORT.binary_search(&en_lower.as_str()).is_ok();
                if is_common_ru && !is_common_en {
                    return Some(SwitchAction {
                        backspaces: len,
                        new_word: ru,
                        to_ru: true,
                        original_word: en,
                    });
                }
            }
            return None;
        }

        // ── 5. Standard trigram language-model check (>= 4 chars) ──────────────
        if len < adjusted_min_len {
            return None;
        }

        // Score using lowercase words (trigram tables are built from lowercased text).
        let score_en = bigrams::score_en(&en_lower);
        let score_ru = bigrams::score_ru(&ru_lower);

        let threshold = (switching_threshold(len) / sensitivity) * dev_threshold_multiplier;

        if layout::hkl_is_russian(active_lang) {
            // User typed with RU layout — all chars must be Cyrillic.
            if !ru.chars().all(is_cyrillic) {
                return None;
            }
            // Propose switching to EN only when EN is significantly more plausible.
            if score_en - score_ru > threshold {
                return Some(SwitchAction {
                    backspaces: len,
                    new_word: en,
                    to_ru: false,
                    original_word: ru,
                });
            }
        } else if layout::hkl_is_english(active_lang) {
            // The RU interpretation must be fully Cyrillic to be a plausible Russian word.
            if !ru.chars().all(is_cyrillic) {
                return None;
            }
            // Propose switching to RU only when RU is significantly more plausible.
            if score_ru - score_en > threshold {
                return Some(SwitchAction {
                    backspaces: len,
                    new_word: ru,
                    to_ru: true,
                    original_word: en,
                });
            }
        }

        None
    }
}

fn is_cyrillic(c: char) -> bool {
    matches!(c, '\u{0410}'..='\u{044F}' | '\u{0401}' | '\u{0451}')
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // LANGID constants used by the tests (identical to layout::LANG_* but local
    // so tests don't depend on the layout module's public surface).
    const LANG_EN: u16 = 0x0409; // English (US)
    const LANG_RU: u16 = 0x0419; // Russian

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Push English letter VK codes into a buffer.
    /// VK_A..VK_Z = 0x41..0x5A, matching ASCII 'A'..'Z'.
    fn push_en_word(buf: &mut WordBuffer, word: &str) {
        for c in word.chars() {
            let lc = c.to_ascii_lowercase();
            assert!(lc.is_ascii_alphabetic(), "push_en_word: non-alpha '{}'", c);
            let vk = lc as u16 - b'a' as u16 + 0x41;
            buf.push(vk, c.is_uppercase());
        }
    }

    // ── Buffer state ─────────────────────────────────────────────────────────

    #[test]
    fn push_increases_len() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "abc");
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn pop_removes_last() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "abc");
        buf.pop();
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn pop_on_empty_does_not_panic() {
        let mut buf = WordBuffer::new();
        buf.pop(); // must not panic
        assert!(buf.is_empty());
    }

    #[test]
    fn clear_empties_buffer() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "hello");
        buf.clear();
        assert!(buf.is_empty());
    }

    // ── Minimum-length guard ─────────────────────────────────────────────────

    #[test]
    fn short_word_auto_detect_returns_none() {
        let mut buf = WordBuffer::new();
        // 3-char word — below the min_len=4 threshold for auto-detection
        push_en_word(&mut buf, "abc");
        assert!(buf.detect_mismatch(LANG_RU).is_none());
        assert!(buf.detect_mismatch(LANG_EN).is_none());
    }

    #[test]
    fn single_char_force_switch_returns_some() {
        let mut buf = WordBuffer::new();
        buf.push(0x48, false); // 'h' in EN → 'р' in RU
        // force_switch accepts words of length ≥ 1
        // Under RU layout, 'р' is Cyrillic → suggest EN 'h'
        assert!(buf.force_switch(LANG_RU).is_some());
    }

    #[test]
    fn empty_buffer_returns_none() {
        let buf = WordBuffer::new();
        assert!(buf.detect_mismatch(LANG_RU).is_none());
        assert!(buf.force_switch(LANG_EN).is_none());
    }

    // ── Core switching scenarios ──────────────────────────────────────────────

    /// The canonical Punto-Switcher test case:
    /// user presses N O T E P A D keys while in Russian layout.
    /// In RU those produce "тщеузфв" (gibberish) → must switch to EN "notepad".
    #[test]
    fn notepad_typed_in_ru_layout_switches_to_en() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "notepad"); // same VK codes, different layout interpretation
        let action = buf.detect_mismatch(LANG_RU).expect("should detect mismatch");
        assert!(!action.to_ru, "must switch to EN");
        assert_eq!(action.new_word.to_lowercase(), "notepad");
        assert_eq!(action.backspaces, 7);
        // original_word is the RU interpretation that appeared on screen
        assert!(!action.original_word.is_empty());
        assert!(action.original_word.chars().all(is_cyrillic));
    }

    /// User types "ghbdtn" in EN layout, which is what you get when you type
    /// "привет" with an accidentally-wrong layout → must switch to RU.
    #[test]
    fn ghbdtn_typed_in_en_layout_switches_to_ru_privet() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "ghbdtn"); // VK_G VK_H VK_B VK_D VK_T VK_N
        let action = buf.detect_mismatch(LANG_EN).expect("should detect mismatch");
        assert!(action.to_ru, "must switch to RU");
        assert_eq!(action.new_word.to_lowercase(), "привет");
        assert_eq!(action.original_word.to_lowercase(), "ghbdtn");
    }

    /// User intentionally types "hello" in English.
    /// "руддщ" (RU interpretation of VK_H VK_E VK_L VK_L VK_O) scores much
    /// lower than "hello" → no switch.
    #[test]
    fn hello_in_en_layout_does_not_switch() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "hello");
        assert!(
            buf.detect_mismatch(LANG_EN).is_none(),
            "should NOT switch a real English word"
        );
    }

    /// User intentionally types "привет" in Russian.
    /// VK codes G H B D T N → in RU produce "привет" (high RU score) vs
    /// "ghbdtn" in EN (very low EN score) → no switch.
    #[test]
    fn privet_in_ru_layout_does_not_switch() {
        let mut buf = WordBuffer::new();
        // VK_G VK_H VK_B VK_D VK_T VK_N — same physical keys as "ghbdtn"
        push_en_word(&mut buf, "ghbdtn");
        assert!(
            buf.detect_mismatch(LANG_RU).is_none(),
            "should NOT switch a real Russian word typed in RU layout"
        );
    }

    #[test]
    fn eto_typed_in_en_layout_switches_to_ru() {
        let mut buf = WordBuffer::new();
        buf.push(0xDE, false); // '\'' -> 'э'
        buf.push(0x4E, false); // 'n' -> 'т'
        buf.push(0x4A, false); // 'j' -> 'о'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch 'это' from EN to RU");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "это");
    }

    #[test]
    fn eto_capital_typed_in_en_layout_switches_to_ru() {
        let mut buf = WordBuffer::new();
        buf.push(0xDE, true); // '\'' -> 'Э'
        buf.push(0x4E, false); // 'n' -> 'т'
        buf.push(0x4A, false); // 'j' -> 'о'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch 'Это' from EN to RU");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "Это");
    }

    #[test]
    fn chto_typed_in_en_layout_switches_to_ru() {
        let mut buf = WordBuffer::new();
        buf.push(0x58, false); // 'x' -> 'ч'
        buf.push(0x4E, false); // 'n' -> 'т'
        buf.push(0x4A, false); // 'j' -> 'о'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch 'что' from EN to RU");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "что");
    }

    #[test]
    fn test_dictionary_sorting() {
        for window in COMMON_RU_SHORT.windows(2) {
            assert!(window[0] < window[1], "COMMON_RU_SHORT not sorted: {} >= {}", window[0], window[1]);
        }
        for window in COMMON_EN_SHORT.windows(2) {
            assert!(window[0] < window[1], "COMMON_EN_SHORT not sorted: {} >= {}", window[0], window[1]);
        }
        assert!(COMMON_RU_SHORT.binary_search(&"это").is_ok(), "это should be found");
        assert!(COMMON_RU_SHORT.binary_search(&"что").is_ok(), "что should be found");
    }

    #[test]
    fn short_common_word_typed_correctly_does_not_switch() {
        let mut buf = WordBuffer::new();
        buf.push(0x54, false); // 't'
        buf.push(0x48, false); // 'h'
        buf.push(0x45, false); // 'e'
        assert!(buf.detect_mismatch(LANG_EN).is_none());
    }

    // ── Case preservation ─────────────────────────────────────────────────────

    #[test]
    fn switch_preserves_capitalisation() {
        let mut buf = WordBuffer::new();
        // "Notepad" with capital N
        push_en_word(&mut buf, "Notepad");
        let action = buf.detect_mismatch(LANG_RU).expect("should switch");
        // First char should be uppercase, rest lowercase
        let mut chars = action.new_word.chars();
        let first = chars.next().unwrap();
        assert!(first.is_uppercase(), "first char should be uppercase, got '{}'", first);
    }

    // ── detection_snapshot ────────────────────────────────────────────────────

    #[test]
    fn snapshot_is_none_for_empty_buffer() {
        let buf = WordBuffer::new();
        assert!(buf.detection_snapshot().is_none());
    }

    #[test]
    fn snapshot_contains_correct_words() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "notepad");
        let snap = buf.detection_snapshot().unwrap();
        assert_eq!(snap.en_word, "notepad");
        assert_eq!(snap.ru_word, "тщеузфв"); // what appeared on screen
        assert_eq!(snap.len, 7);
        // score_en("notepad") should be higher than score_ru("тщеузфв")
        assert!(
            snap.score_en > snap.score_ru,
            "EN score {:.2} should beat RU score {:.2} for 'notepad' VK sequence",
            snap.score_en,
            snap.score_ru
        );
    }

    #[test]
    fn single_letter_words_switch() {
        let mut buf = WordBuffer::new();
        
        // 1. EN layout, user types 'd' (which is 'в' in RU layout) -> should switch to RU 'в'
        buf.push(0x44, false); // 'd'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch single char d -> в");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "в");
        buf.clear();

        // 2. EN layout, user types 'a' (common EN single char) -> should NOT switch
        buf.push(0x41, false); // 'a'
        assert!(buf.detect_mismatch(LANG_EN).is_none());
        buf.clear();

        // 3. RU layout, user types 'ф' (which is 'a' in EN layout) -> should switch to EN 'a'
        buf.push(0x41, false); // 'a' VK key under RU layout maps to 'ф'
        let action = buf.detect_mismatch(LANG_RU).expect("should switch single char ф -> a");
        assert!(!action.to_ru);
        assert_eq!(action.new_word, "a");
        buf.clear();

        // 4. RU layout, user types 'в' (common RU single char) -> should NOT switch
        buf.push(0x44, false); // 'd' VK key under RU layout maps to 'в'
        assert!(buf.detect_mismatch(LANG_RU).is_none());
        buf.clear();
    }

    #[test]
    fn technical_abbreviations_switch() {
        let mut buf = WordBuffer::new();

        // 'cli' typed as 'сдш' in RU layout (active layout RU) -> should switch to EN 'cli'
        buf.push(0x43, false); // 'c' -> 'с'
        buf.push(0x4C, false); // 'l' -> 'д'
        buf.push(0x49, false); // 'i' -> 'ш'
        let action = buf.detect_mismatch(LANG_RU).expect("should switch 'сдш' -> 'cli'");
        assert!(!action.to_ru);
        assert_eq!(action.new_word, "cli");
        buf.clear();

        // 'wp' typed as 'цз' in RU layout -> should switch to EN 'wp'
        buf.push(0x57, false); // 'w' -> 'ц'
        buf.push(0x50, false); // 'p' -> 'з'
        let action = buf.detect_mismatch(LANG_RU).expect("should switch 'цз' -> 'wp'");
        assert!(!action.to_ru);
        assert_eq!(action.new_word, "wp");
        buf.clear();
    }

    fn push_en_chars(buf: &mut WordBuffer, s: &str) {
        for c in s.chars() {
            let is_upper = c.is_uppercase();
            let lc = c.to_lowercase().next().unwrap();
            let vk = match lc {
                'a'..='z' => lc as u16 - b'a' as u16 + 0x41,
                ';' => 0xBA,
                ',' => 0xBC,
                '.' => 0xBE,
                '[' => 0xDB,
                ']' => 0xDD,
                '\'' => 0xDE,
                _ => panic!("untranslatable char {}", c),
            };
            buf.push(vk, is_upper);
        }
    }

    #[test]
    fn test_long_ru_words_switch() {
        let mut buf = WordBuffer::new();

        // 1. htfkbpeq -> реализуй
        push_en_chars(&mut buf, "htfkbpeq");
        let action = buf.detect_mismatch(LANG_EN).expect("should switch htfkbpeq");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "реализуй");
        buf.clear();

        // 2. ekexitybz -> улучшения
        push_en_chars(&mut buf, "ekexitybz");
        let action = buf.detect_mismatch(LANG_EN).expect("should switch ekexitybz");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "улучшения");
        buf.clear();

        // 3. ekexitybq -> улучшений
        push_en_chars(&mut buf, "ekexitybq");
        let action = buf.detect_mismatch(LANG_EN).expect("should switch ekexitybq");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "улучшений");
        buf.clear();

        // 4. levf. -> думаю
        push_en_chars(&mut buf, "levf.");
        let action = buf.detect_mismatch(LANG_EN).expect("should switch levf.");
        assert!(action.to_ru);
        assert_eq!(action.new_word, "думаю");
        buf.clear();
    }

    #[test]
    fn test_sensitivity_levels() {
        let candidates = &[
            ("ghbdtn", LANG_EN),    // привет (len=6, threshold=1.0)
            ("notepad", LANG_RU),   // notepad (len=7, threshold=0.9)
            ("htfkbpeq", LANG_EN),  // реализуй (len=8, threshold=0.8)
            ("ekexitybz", LANG_EN), // улучшения (len=9, threshold=0.8)
        ];
        
        let mut found = false;
        for &(word, lang) in candidates {
            let mut buf = WordBuffer::new();
            if lang == LANG_EN {
                push_en_chars(&mut buf, word);
            } else {
                push_en_word(&mut buf, word);
            }
            let snap = buf.detection_snapshot().unwrap();
            let diff = (snap.score_ru - snap.score_en).abs();
            let threshold = switching_threshold(snap.len);
            
            if diff > threshold && diff < threshold / 0.6 {
                assert!(buf.detect_mismatch_with_sensitivity(lang, 1.0).is_some());
                assert!(buf.detect_mismatch_with_sensitivity(lang, 1.4).is_some());
                assert!(buf.detect_mismatch_with_sensitivity(lang, 0.6).is_none());
                found = true;
                break;
            }
        }
        assert!(found, "Could not find a test word satisfying sensitivity threshold window.");
    }
}
