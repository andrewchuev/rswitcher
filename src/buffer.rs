use crate::{bigrams, layout};

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

#[derive(Debug, Default)]
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
    /// `active_lang` is the low 16-bit word of the foreground window's HKL.
    pub fn detect_mismatch(&self, active_lang: u16) -> Option<SwitchAction> {
        self.detect_impl(2, active_lang)
    }

    /// Hotkey forced switch (requires >= 1 buffered key).
    pub fn force_switch(&self, active_lang: u16) -> Option<SwitchAction> {
        self.detect_impl(1, active_lang)
    }

    fn detect_impl(&self, min_len: usize, active_lang: u16) -> Option<SwitchAction> {
        if self.entries.len() < min_len {
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

        if en.chars().count() != self.entries.len() || ru.chars().count() != self.entries.len() {
            return None;
        }

        // Score using lowercase words (bigram tables are built from lowercased text).
        let score_en = bigrams::score_en(&en.to_lowercase());
        let score_ru = bigrams::score_ru(&ru.to_lowercase());

        // Single-char words produce NEG_INFINITY bigram scores (need ≥ 2 chars).
        // This only happens in force_switch (min_len=1). Skip threshold check and
        // decide purely from character type (Cyrillic vs ASCII).
        if score_en == f32::NEG_INFINITY && score_ru == f32::NEG_INFINITY {
            if layout::hkl_is_russian(active_lang) && ru.chars().all(is_cyrillic) {
                return Some(SwitchAction {
                    backspaces: self.len(),
                    new_word: en,
                    to_ru: false,
                    original_word: ru,
                });
            } else if layout::hkl_is_english(active_lang)
                && en.chars().all(|c| c.is_ascii_alphabetic())
            {
                return Some(SwitchAction {
                    backspaces: self.len(),
                    new_word: ru,
                    to_ru: true,
                    original_word: en,
                });
            }
            return None;
        }

        if layout::hkl_is_russian(active_lang) {
            // User typed with RU layout — all chars must be Cyrillic.
            if !ru.chars().all(is_cyrillic) {
                return None;
            }
            // Propose switching to EN only when EN is significantly more plausible.
            if score_en - score_ru > bigrams::THRESHOLD_PER_BIGRAM {
                return Some(SwitchAction {
                    backspaces: self.len(),
                    new_word: en,
                    to_ru: false,
                    original_word: ru,
                });
            }
        } else if layout::hkl_is_english(active_lang) {
            // User typed with EN layout — all chars must be ASCII letters.
            if !en.chars().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            // Propose switching to RU only when RU is significantly more plausible.
            if score_ru - score_en > bigrams::THRESHOLD_PER_BIGRAM {
                return Some(SwitchAction {
                    backspaces: self.len(),
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
    fn single_char_auto_detect_returns_none() {
        let mut buf = WordBuffer::new();
        buf.push(0x48, false); // 'h'
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
}
