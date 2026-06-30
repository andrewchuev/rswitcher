use std::collections::{HashMap, HashSet};
use crate::{bigrams, layout};

const COMMON_RU_SHORT: &[&str] = &[
    "без", "был", "быт", "вас", "век", "все", "всю", "всё", "вы", "где",
    "да", "два", "для", "до", "дом", "его", "ее", "ей", "ему", "еще",
    "ещё", "её", "же", "за", "из", "изо", "или", "им", "ими", "имя",
    "их", "как", "кто", "ли", "мин", "мне", "мог", "мои", "моя", "мы", "на",
    "нам", "нас", "не", "ней", "нет", "них", "но", "об", "обо", "он", "она",
    "они", "оно", "от", "ото", "по", "под", "при", "про", "раз", "сам", "сих",
    "со", "так", "там", "тв", "те", "тем", "тех", "тип", "то", "той", "том", "тот",
    "три", "тут", "ты", "уж", "уже", "чем", "чер", "что", "это", "эту"
];

const COMMON_UA_SHORT: &[&str] = &[
    "або", "але", "без", "був", "вас", "ви", "все", "всю", "всі", "від",
    "він", "для", "до", "моя", "моє", "мої", "нам", "нас", "ней", "них",
    "ні", "при", "про", "під", "так", "там", "тим", "тут", "усе", "усі",
    "хто", "це", "цей", "цю", "ця", "чи", "як", "яка", "яке", "які",
    "із", "їм", "їх", "її"
];

const COMMON_EN_SHORT: &[&str] = &[
    "add", "ali", "am", "an", "and", "any", "api", "app", "are", "as", "at",
    "aws", "bad", "bat", "be", "big", "bin", "box", "but", "by", "can", "cd",
    "cfg", "cli", "cmd", "con", "crm", "css", "csv", "day", "db", "dev",
    "did", "dir", "dns", "do", "doc", "dom", "env", "err", "few", "flux",
    "for", "fpv", "ftp", "get", "git", "go", "had", "has", "he", "her", "him",
    "his", "how", "hub", "id", "if", "in", "io", "ip", "is", "it",
    "its", "js", "key", "let", "lib", "lin", "log", "low", "ls", "mac", "mad",
    "map", "may", "md", "me", "mr", "my", "net", "new", "no", "not", "now",
    "npm", "of", "off", "ok", "old", "on", "one", "or", "org", "os", "our",
    "out", "own", "pdf", "pkg", "png", "pr", "py", "red", "rs", "run",
    "sad", "say", "see", "sh", "she", "so", "sql", "src", "ssh", "ssl",
    "sys", "tcp", "the", "tls", "too", "try", "ts", "two", "txt", "udp",
    "ui", "up", "uri", "url", "usb", "use", "ux", "val", "vat", "vpn", "vps",
    "was", "way", "wc", "we", "web", "who", "win", "wp", "wsl", "xml", "yes",
    "you", "zip"
];

include!(concat!(env!("OUT_DIR"), "/dictionaries_gen.rs"));

/// Settings snapshot passed into detection functions so they don't need to
/// re-acquire the RwLock on every call.  Built once per keypress in hook.rs.
#[derive(Debug, Default)]
pub struct DetectionConfig {
    pub ignored_words: HashSet<String>,
    pub word_corrections: HashMap<String, u16>,
    pub preferred_cyrillic: crate::settings::PreferredCyrillic,
    /// Dominant layout inferred from the last few typed words, if any.
    /// When set and equal to `active_lang`, the detector applies a stricter
    /// threshold (context_factor) before switching away from that layout.
    pub context_lang: Option<u16>,
}

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
    /// The target layout LANGID.
    pub target_lang: u16,
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
    /// What they would produce in the Ukrainian layout (lowercase).
    pub ua_word: String,
    /// Per-bigram log-probability score for the English interpretation.
    pub score_en: f32,
    /// Per-bigram log-probability score for the Russian interpretation.
    pub score_ru: f32,
    /// Per-bigram log-probability score for the Ukrainian interpretation.
    pub score_ua: f32,
    /// Number of buffered VK entries.
    pub len: usize,
}

/// Minimum score advantage UA must have over RU to be chosen when the word is
/// spelled identically in both languages.  The UA corpus causes UA bigram
/// scores to be systematically higher for shared Slavic vocabulary.  A delta
/// below this threshold is treated as a tie and resolved in favour of RU.
const RU_UA_SCORE_MIN_DELTA: f32 = 0.25;

/// Stronger threshold used when the word has no Ukrainian-specific letters
/// (і, ї, є, ґ) AND is spelled identically in both languages.  Without any
/// disambiguating marker the word is almost certainly Russian or language-
/// neutral, so we require a larger score gap before choosing UA.
const RU_UA_SCORE_STRONG_DELTA: f32 = 0.40;

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
    pub has_switched: bool,
    /// Layout LANGID active when the first key of this word was pushed.
    /// 0 = unset (buffer was never pushed to after last clear).
    /// Stored so force_switch can use the ORIGINAL typing direction even after
    /// on-the-fly detection has already changed foreground_lang() to a new layout.
    entry_lang: u16,
}

impl WordBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the active layout for the start of this word.
    /// Must be called before the first `push` after a `clear` (i.e. when the
    /// buffer is empty).  Subsequent calls while the buffer is non-empty are
    /// ignored so that on-the-fly layout switches do not overwrite the value.
    pub fn set_entry_lang(&mut self, lang: u16) {
        if self.entries.is_empty() {
            self.entry_lang = lang;
        }
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
        self.has_switched = false;
        self.entry_lang = 0;
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Compute EN/RU/UA translations and bigram scores for the current buffer.
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
        let ua: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_ua(e.vk, false))
            .collect();
        Some(DetectionSnapshot {
            score_en: bigrams::score_en(&en),
            score_ru: bigrams::score_ru(&ru),
            score_ua: bigrams::score_ua(&ua),
            len: self.entries.len(),
            en_word: en,
            ru_word: ru,
            ua_word: ua,
        })
    }

    /// Hotkey forced switch — backward-compat wrapper using default config.
    /// Production code calls `force_switch_with_config` directly.
    #[cfg(test)]
    pub fn force_switch(&self, active_lang: u16) -> Option<SwitchAction> {
        self.force_switch_with_config(active_lang, &DetectionConfig::default())
    }

    /// Hotkey forced switch (requires >= 1 buffered key).
    ///
    /// Unlike auto-detection, bypasses the bigram score threshold entirely —
    /// the user explicitly requested the switch, so we trust their intent.
    /// Returns None only when the buffer is empty or the translation is invalid.
    /// Uses `is_upper` from entries so mixed-case words round-trip correctly.
    pub fn force_switch_with_config(&self, active_lang: u16, config: &DetectionConfig) -> Option<SwitchAction> {
        if self.entries.is_empty() {
            return None;
        }
        // Use the layout that was active when the user STARTED typing this word.
        // After an on-the-fly mid-word switch, foreground_lang() already reports
        // the target layout (e.g. EN), which would send force_switch in the wrong
        // direction.  entry_lang is set before any mid-word switch happens.
        let active_lang = if self.entry_lang != 0 { self.entry_lang } else { active_lang };

        // Compute case-correct translations so mixed-case words round-trip exactly.
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
        let ua: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_ua(e.vk, e.is_upper))
            .collect();

        if layout::hkl_is_russian(active_lang) || layout::hkl_is_ukrainian(active_lang) {
            if en.chars().count() != self.entries.len() {
                return None;
            }
            let new_word = apply_case_correction(&self.entries, &en.to_lowercase());
            Some(SwitchAction {
                backspaces: self.len(),
                new_word,
                target_lang: layout::LANG_EN_US,
                original_word: if layout::hkl_is_russian(active_lang) { ru } else { ua },
            })
        } else if layout::hkl_is_english(active_lang) {
            let ru_lower = ru.to_lowercase();
            let ua_lower = ua.to_lowercase();
            let ru_valid = ru.chars().count() == self.entries.len() && ru_lower.chars().all(is_cyrillic_or_ua);
            let ua_valid = ua.chars().count() == self.entries.len() && ua_lower.chars().all(is_cyrillic_or_ua);

            if !ru_valid && !ua_valid {
                return None;
            }

            let to_ru: Option<bool> = if ru_valid && ua_valid {
                let ru_in_dict = RU_COMMON_WORDS.binary_search(&ru_lower.as_str()).is_ok();
                let ua_in_dict = UA_COMMON_WORDS.binary_search(&ua_lower.as_str()).is_ok();
                resolve_cyrillic_preference(
                    &config.preferred_cyrillic,
                    true,
                    &ua_lower,
                    || {
                        if ru_in_dict && !ua_in_dict {
                            true
                        } else if ua_in_dict && !ru_in_dict {
                            false
                        } else {
                            let score_ru = bigrams::score_ru(&ru_lower);
                            let score_ua = bigrams::score_ua(&ua_lower);
                            let required_delta = if has_ua_markers(&ua_lower) {
                                RU_UA_SCORE_MIN_DELTA
                            } else {
                                RU_UA_SCORE_STRONG_DELTA
                            };
                            score_ua - score_ru < required_delta
                        }
                    },
                )
            } else {
                Some(ru_valid)
            };

            match to_ru {
                Some(true) => {
                    let new_word = apply_case_correction(&self.entries, &ru_lower);
                    Some(SwitchAction {
                        backspaces: self.len(),
                        new_word,
                        target_lang: layout::LANG_RU,
                        original_word: en,
                    })
                }
                Some(false) => {
                    let new_word = apply_case_correction(&self.entries, &ua_lower);
                    Some(SwitchAction {
                        backspaces: self.len(),
                        new_word,
                        target_lang: layout::LANG_UA,
                        original_word: en,
                    })
                }
                None => None,
            }
        } else {
            None
        }
    }

    /// Combined boundary detection + snapshot for hook.rs (avoids double computation).
    pub fn detect_with_snapshot(
        &self, active_lang: u16, sensitivity: f32, config: &DetectionConfig,
    ) -> (Option<SwitchAction>, Option<DetectionSnapshot>) {
        self.detect_impl_opt(active_lang, sensitivity, false, config)
    }

    /// Combined on-the-fly detection + snapshot for hook.rs.
    pub fn detect_on_the_fly_with_snapshot(
        &self, active_lang: u16, sensitivity: f32, config: &DetectionConfig,
    ) -> (Option<SwitchAction>, Option<DetectionSnapshot>) {
        self.detect_impl_opt(active_lang, sensitivity, true, config)
    }

    // ── Backward-compat wrappers (used by unit tests) ────────────────────────

    #[allow(dead_code)]
    pub fn detect_mismatch(&self, active_lang: u16) -> Option<SwitchAction> {
        self.detect_impl_opt(active_lang, 1.0, false, &DetectionConfig::default()).0
    }

    pub fn detect_mismatch_with_sensitivity(&self, active_lang: u16, sensitivity: f32) -> Option<SwitchAction> {
        self.detect_impl_opt(active_lang, sensitivity, false, &DetectionConfig::default()).0
    }

    pub fn detect_mismatch_on_the_fly(&self, active_lang: u16, sensitivity: f32) -> Option<SwitchAction> {
        self.detect_impl_opt(active_lang, sensitivity, true, &DetectionConfig::default()).0
    }

    fn detect_impl_opt(
        &self, active_lang: u16, sensitivity: f32, on_the_fly: bool, config: &DetectionConfig,
    ) -> (Option<SwitchAction>, Option<DetectionSnapshot>) {
        // ── Early exits before any translation work ──────────────────────────
        if self.has_switched {
            return (None, None);
        }
        let len = self.entries.len();

        if on_the_fly {
            if len < 5 { return (None, None); }
        } else {
            if len == 0 { return (None, None); }
        }

        // Skip repeated-key runs (e.g. "jjjjj" → "ооооо"); bigram scores are
        // near-identical across layouts and trigger spurious switches.
        if len >= 2 {
            let first_vk = self.entries[0].vk;
            if self.entries.iter().all(|e| e.vk == first_vk) {
                return (None, None);
            }
        }

        // ── Compute translations once (always lowercase for scoring/snapshot) ──
        let en: String = self.entries.iter()
            .filter_map(|e| layout::vk_to_en(e.vk, false)).collect();
        let ru: String = self.entries.iter()
            .filter_map(|e| layout::vk_to_ru(e.vk, false)).collect();
        let ua: String = self.entries.iter()
            .filter_map(|e| layout::vk_to_ua(e.vk, false)).collect();

        let en_ok = en.chars().count() == len;
        let ru_ok = ru.chars().count() == len;
        let ua_ok = ua.chars().count() == len;

        // Build snapshot once; reuse pre-computed scores below.
        let score_en = bigrams::score_en(&en);
        let score_ru = bigrams::score_ru(&ru);
        let score_ua = bigrams::score_ua(&ua);
        let snap = DetectionSnapshot {
            en_word: en.clone(),
            ru_word: ru.clone(),
            ua_word: ua.clone(),
            score_en,
            score_ru,
            score_ua,
            len,
        };

        let backspaces = if on_the_fly { len - 1 } else { len };

        // Macro to wrap a SwitchAction with the shared snapshot.
        macro_rules! sw {
            ($action:expr) => { return (Some($action), Some(snap)) };
        }
        macro_rules! skip {
            () => { return (None, Some(snap)) };
        }

        // ── 0. Whitelisted / ignored words ───────────────────────────────────
        let word_to_check = if layout::hkl_is_russian(active_lang) { &ru }
            else if layout::hkl_is_ukrainian(active_lang) { &ua }
            else { &en };
        if config.ignored_words.contains(word_to_check.as_str()) {
            skip!();
        }

        // ── 0b. User-confirmed corrections override the statistical model ─────
        if let Some(&target_lang) = config.word_corrections.get(en.as_str()) {
            if target_lang != active_lang {
                if target_lang == layout::LANG_RU && ru_ok && ru.chars().all(is_cyrillic_or_ua) {
                    sw!(SwitchAction {
                        backspaces,
                        new_word: apply_case_correction(&self.entries, &ru),
                        target_lang,
                        original_word: if layout::hkl_is_english(active_lang) { en.clone() }
                                       else if layout::hkl_is_russian(active_lang) { ru.clone() }
                                       else { ua.clone() },
                    });
                } else if target_lang == layout::LANG_UA && ua_ok && ua.chars().all(is_cyrillic_or_ua) {
                    sw!(SwitchAction {
                        backspaces,
                        new_word: apply_case_correction(&self.entries, &ua),
                        target_lang,
                        original_word: if layout::hkl_is_english(active_lang) { en.clone() }
                                       else if layout::hkl_is_russian(active_lang) { ru.clone() }
                                       else { ua.clone() },
                    });
                } else if target_lang == layout::LANG_EN_US && en_ok {
                    sw!(SwitchAction {
                        backspaces,
                        new_word: apply_case_correction(&self.entries, &en),
                        target_lang,
                        original_word: if layout::hkl_is_russian(active_lang) { ru.clone() }
                                       else { ua.clone() },
                    });
                }
            }
            // Correction present but translation invalid — fall through to model.
        }

        // ── 1. Valid word in the current layout ──────────────────────────────
        if layout::hkl_is_russian(active_lang) && ru_ok
            && RU_COMMON_WORDS.binary_search(&ru.as_str()).is_ok() { skip!(); }
        else if layout::hkl_is_ukrainian(active_lang) && ua_ok
            && UA_COMMON_WORDS.binary_search(&ua.as_str()).is_ok() { skip!(); }
        else if layout::hkl_is_english(active_lang) && en_ok
            && EN_COMMON_WORDS.binary_search(&en.as_str()).is_ok() { skip!(); }

        // ── 1b. Cross-layout EN dict check for Cyrillic-active layouts ────────
        // Short words (≤4 chars) like "ctrl", "exit", "sudo" are often inline
        // tech references in Cyrillic text, not mistyped EN.  When the recent
        // context confirms we're in Cyrillic mode, defer to the bigram model
        // (step 5) so the normal threshold applies rather than auto-switching.
        let skip_short_dict = len <= 4 && config.context_lang == Some(active_lang);
        if !on_the_fly && !skip_short_dict && en_ok && EN_COMMON_WORDS.binary_search(&en.as_str()).is_ok() {
            if layout::hkl_is_russian(active_lang) && ru_ok && ru.chars().all(is_cyrillic_or_ua) {
                sw!(SwitchAction {
                    backspaces,
                    new_word: apply_case_correction(&self.entries, &en),
                    target_lang: layout::LANG_EN_US,
                    original_word: ru.clone(),
                });
            } else if layout::hkl_is_ukrainian(active_lang) && ua_ok && ua.chars().all(is_cyrillic_or_ua) {
                sw!(SwitchAction {
                    backspaces,
                    new_word: apply_case_correction(&self.entries, &en),
                    target_lang: layout::LANG_EN_US,
                    original_word: ua.clone(),
                });
            }
        }

        // ── 3. Single-letter words ────────────────────────────────────────────
        if !on_the_fly && len == 1 {
            let common_ru_single = ["в", "и", "а", "о", "с", "у", "я", "к"];
            let common_ua_single = ["в", "і", "а", "о", "у", "я", "є", "з"];
            let common_en_single = ["a", "i"];
            if layout::hkl_is_russian(active_lang) {
                if ru_ok && en_ok && !common_ru_single.contains(&ru.as_str())
                    && common_en_single.contains(&en.as_str())
                {
                    sw!(SwitchAction {
                        backspaces,
                        new_word: apply_case_correction(&self.entries, &en),
                        target_lang: layout::LANG_EN_US,
                        original_word: ru,
                    });
                }
            } else if layout::hkl_is_ukrainian(active_lang) {
                if ua_ok && en_ok && !common_ua_single.contains(&ua.as_str())
                    && common_en_single.contains(&en.as_str())
                {
                    sw!(SwitchAction {
                        backspaces,
                        new_word: apply_case_correction(&self.entries, &en),
                        target_lang: layout::LANG_EN_US,
                        original_word: ua,
                    });
                }
            } else if layout::hkl_is_english(active_lang) {
                if ru_ok && en_ok && !common_en_single.contains(&en.as_str()) {
                    let ru_common = common_ru_single.contains(&ru.as_str());
                    let ua_common = common_ua_single.contains(&ua.as_str());
                    if ru_common || ua_common {
                        let to_ru = resolve_cyrillic_preference(
                            &config.preferred_cyrillic,
                            ru_common,
                            &ua,
                            || {
                                if ru_common && !ua_common {
                                    true
                                } else if ua_common && !ru_common {
                                    false
                                } else {
                                    true
                                }
                            },
                        );
                        if to_ru == Some(true) {
                            sw!(SwitchAction {
                                backspaces,
                                new_word: apply_case_correction(&self.entries, &ru),
                                target_lang: layout::LANG_RU,
                                original_word: en,
                            });
                        } else if to_ru == Some(false) {
                            sw!(SwitchAction {
                                backspaces,
                                new_word: apply_case_correction(&self.entries, &ua),
                                target_lang: layout::LANG_UA,
                                original_word: en,
                            });
                        }
                    }
                }
            }
            skip!();
        }

        // ── 4. Dictionary-based check for short words (2-3 chars) ─────────────
        if !on_the_fly && (len == 2 || len == 3) {
            if layout::hkl_is_russian(active_lang) {
                if ru_ok && en_ok {
                    let is_common_en = COMMON_EN_SHORT.binary_search(&en.as_str()).is_ok();
                    let is_common_ru = COMMON_RU_SHORT.binary_search(&ru.as_str()).is_ok();
                    if is_common_en && !is_common_ru {
                        sw!(SwitchAction {
                            backspaces,
                            new_word: apply_case_correction(&self.entries, &en),
                            target_lang: layout::LANG_EN_US,
                            original_word: ru,
                        });
                    }
                    let is_common_ua = COMMON_UA_SHORT.binary_search(&ua.as_str()).is_ok();
                    if is_common_ua && !is_common_ru && has_ua_markers(&ua) {
                        sw!(SwitchAction {
                            backspaces,
                            new_word: apply_case_correction(&self.entries, &ua),
                            target_lang: layout::LANG_UA,
                            original_word: ru,
                        });
                    }
                }
            } else if layout::hkl_is_ukrainian(active_lang) {
                if ua_ok && en_ok {
                    let is_common_en = COMMON_EN_SHORT.binary_search(&en.as_str()).is_ok();
                    let is_common_ua = COMMON_UA_SHORT.binary_search(&ua.as_str()).is_ok();
                    if is_common_en && !is_common_ua {
                        sw!(SwitchAction {
                            backspaces,
                            new_word: apply_case_correction(&self.entries, &en),
                            target_lang: layout::LANG_EN_US,
                            original_word: ua,
                        });
                    }
                    let is_common_ru = COMMON_RU_SHORT.binary_search(&ru.as_str()).is_ok();
                    if is_common_ru && !is_common_ua && has_ru_markers(&ru) {
                        sw!(SwitchAction {
                            backspaces,
                            new_word: apply_case_correction(&self.entries, &ru),
                            target_lang: layout::LANG_RU,
                            original_word: ua,
                        });
                    }
                }
            } else if layout::hkl_is_english(active_lang) {
                if en_ok {
                    let is_common_en = COMMON_EN_SHORT.binary_search(&en.as_str()).is_ok();
                    let is_common_ru = ru_ok && COMMON_RU_SHORT.binary_search(&ru.as_str()).is_ok();
                    let is_common_ua = ua_ok && COMMON_UA_SHORT.binary_search(&ua.as_str()).is_ok();
                    if !is_common_en {
                        if is_common_ru && is_common_ua {
                            let to_ru = resolve_cyrillic_preference(
                                &config.preferred_cyrillic, true, &ua,
                                || {
                                    if ru == ua { score_ua - score_ru < RU_UA_SCORE_MIN_DELTA }
                                    else { score_ru >= score_ua }
                                },
                            );
                            if to_ru == Some(true) {
                                sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ru), target_lang: layout::LANG_RU, original_word: en });
                            } else if to_ru == Some(false) {
                                sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ua), target_lang: layout::LANG_UA, original_word: en });
                            }
                        } else if is_common_ru {
                            sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ru), target_lang: layout::LANG_RU, original_word: en });
                        } else if is_common_ua {
                            let allow_ua = match &config.preferred_cyrillic {
                                crate::settings::PreferredCyrillic::Ua => true,
                                crate::settings::PreferredCyrillic::Ru => false,
                                crate::settings::PreferredCyrillic::Auto => has_ua_markers(&ua),
                            };
                            if allow_ua {
                                sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ua), target_lang: layout::LANG_UA, original_word: en });
                            }
                        }
                    }
                }
            }
            skip!();
        }

        // ── 5. Bigram/trigram model (>= 4 chars, on-the-fly >= 5) ────────────
        let min_len = if on_the_fly { 5 } else { 4 };
        if len < min_len { skip!(); }

        // When the recent context confirms the active layout, require a larger
        // score gap before switching away.  Factor 1.3 means the rival layout
        // must outscore the current one by 30% more than the base threshold.
        let context_factor: f32 = if config.context_lang == Some(active_lang) { 1.3 } else { 1.0 };
        let base_threshold = switching_threshold(len) / sensitivity * context_factor;

        if layout::hkl_is_russian(active_lang) {
            if !ru_ok || !en_ok || !ru.chars().all(is_cyrillic_or_ua) { skip!(); }
            let score_ua_eff = if ua_ok && ua.chars().all(is_cyrillic_or_ua) { score_ua } else { f32::NEG_INFINITY };
            let en_thr = base_threshold * if has_ru_markers(&ru) {
                if on_the_fly { if len >= 8 { 1.5 } else { 2.5 } } else { 1.5 }
            } else {
                if on_the_fly { if len >= 8 { 1.2 } else { 2.0 } } else { 1.0 }
            };
            if score_en - score_ru > en_thr {
                sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &en), target_lang: layout::LANG_EN_US, original_word: ru });
            }
            let allow_cross = match &config.preferred_cyrillic {
                crate::settings::PreferredCyrillic::Ru => false,
                crate::settings::PreferredCyrillic::Auto => has_ua_markers(&ua),
                crate::settings::PreferredCyrillic::Ua => true,
            };
            if allow_cross && score_ua_eff.is_finite() {
                let cross_thr = base_threshold * if has_ua_markers(&ua) { 1.5 } else { 2.0 };
                if score_ua_eff - score_ru > cross_thr {
                    sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ua), target_lang: layout::LANG_UA, original_word: ru });
                }
            }
        } else if layout::hkl_is_ukrainian(active_lang) {
            if !ua_ok || !en_ok || !ua.chars().all(is_cyrillic_or_ua) { skip!(); }
            let score_ru_eff = if ru_ok && ru.chars().all(is_cyrillic_or_ua) { score_ru } else { f32::NEG_INFINITY };
            let en_thr = base_threshold * if has_ua_markers(&ua) {
                if on_the_fly { if len >= 8 { 1.5 } else { 2.5 } } else { 1.5 }
            } else {
                if on_the_fly { if len >= 8 { 1.2 } else { 2.0 } } else { 1.0 }
            };
            if score_en - score_ua > en_thr {
                sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &en), target_lang: layout::LANG_EN_US, original_word: ua });
            }
            if score_ru_eff.is_finite() {
                let cross_thr = base_threshold * if has_ru_markers(&ru) { 1.5 } else { 2.0 };
                if score_ru_eff - score_ua > cross_thr {
                    sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ru), target_lang: layout::LANG_RU, original_word: ua });
                }
            }
        } else if layout::hkl_is_english(active_lang) {
            if !en_ok { skip!(); }
            let ru_candidate = ru_ok && ru.chars().all(is_cyrillic_or_ua);
            let ua_candidate = ua_ok && ua.chars().all(is_cyrillic_or_ua);
            if !ru_candidate && !ua_candidate { skip!(); }

            let en_has_punct = en.chars().any(|c| !c.is_alphabetic());
            if on_the_fly && ru == ua && !en_has_punct { skip!(); }

            if len <= 6 && en_has_punct {
                let ru_in_dict = ru_candidate && RU_COMMON_WORDS.binary_search(&ru.as_str()).is_ok();
                let ua_in_dict = ua_candidate && UA_COMMON_WORDS.binary_search(&ua.as_str()).is_ok();
                if ru_in_dict || ua_in_dict {
                    let to_ru = resolve_cyrillic_preference(
                        &config.preferred_cyrillic, ru_candidate, &ua,
                        || {
                            if ru_in_dict && !ua_in_dict { true }
                            else if ua_in_dict && !ru_in_dict { false }
                            else {
                                let d = if has_ua_markers(&ua) { RU_UA_SCORE_MIN_DELTA } else { RU_UA_SCORE_STRONG_DELTA };
                                score_ua - score_ru < d
                            }
                        },
                    );
                    match to_ru {
                        Some(true)  => sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ru), target_lang: layout::LANG_RU, original_word: en }),
                        Some(false) => sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ua), target_lang: layout::LANG_UA, original_word: en }),
                        None => skip!(),
                    }
                }
            }

            let score_ru_eff = if ru_candidate { score_ru } else { f32::NEG_INFINITY };
            let score_ua_eff = if ua_candidate { score_ua } else { f32::NEG_INFINITY };
            let ru_diff = score_ru_eff - score_en;
            let ua_diff = score_ua_eff - score_en;
            let ru_thr = base_threshold * if has_ru_markers(&ru) {
                if on_the_fly { 1.3 } else { 0.7 }
            } else {
                if on_the_fly { 2.5 } else { 1.0 }
            };
            let ua_thr = base_threshold * if has_ua_markers(&ua) {
                if on_the_fly { 1.3 } else { 0.7 }
            } else {
                if on_the_fly { 2.5 } else { 1.0 }
            };

            if ru_diff > ru_thr || ua_diff > ua_thr {
                let ru_in_dict = RU_COMMON_WORDS.binary_search(&ru.as_str()).is_ok();
                let ua_in_dict = UA_COMMON_WORDS.binary_search(&ua.as_str()).is_ok();
                let to_ru = resolve_cyrillic_preference(
                    &config.preferred_cyrillic, ru_candidate, &ua,
                    || {
                        if ru_in_dict && !ua_in_dict { true }
                        else if ua_in_dict && !ru_in_dict { false }
                        else {
                            let d = if has_ua_markers(&ua) { RU_UA_SCORE_MIN_DELTA } else { RU_UA_SCORE_STRONG_DELTA };
                            if ru_candidate { score_ua_eff - score_ru_eff < d } else { false }
                        }
                    },
                );
                if to_ru == Some(true) {
                    sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ru), target_lang: layout::LANG_RU, original_word: en });
                } else if to_ru == Some(false) {
                    sw!(SwitchAction { backspaces, new_word: apply_case_correction(&self.entries, &ua), target_lang: layout::LANG_UA, original_word: en });
                }
            }
        }

        (None, Some(snap))
    }
}

/// Resolve the RU-vs-UA decision respecting `preferred_cyrillic`.
///
/// Returns:
/// - `Some(true)`  → switch to RU
/// - `Some(false)` → switch to UA
/// - `None`        → skip (neither allowed by preference)
///
/// `ru_candidate`: whether a valid RU translation exists.
/// `ua_lower`: the lowercase UA translation (used for marker detection).
/// `auto_fn`: closure that returns the bigram/dict decision when `Auto`.
fn resolve_cyrillic_preference(
    pref: &crate::settings::PreferredCyrillic,
    ru_candidate: bool,
    ua_lower: &str,
    auto_fn: impl FnOnce() -> bool,
) -> Option<bool> {
    match pref {
        crate::settings::PreferredCyrillic::Ru => {
            if ru_candidate { Some(true) } else { None }
        }
        crate::settings::PreferredCyrillic::Ua => {
            Some(false)
        }
        crate::settings::PreferredCyrillic::Auto => {
            // Variant B: without UA-specific letters, never switch to UA.
            let ua_allowed = has_ua_markers(ua_lower);
            if !ua_allowed {
                if ru_candidate { Some(true) } else { None }
            } else {
                Some(auto_fn())
            }
        }
    }
}

fn is_cyrillic_or_ua(c: char) -> bool {
    matches!(c, '\u{0410}'..='\u{044F}' | '\u{0401}' | '\u{0451}' | '\u{0404}' | '\u{0454}' | '\u{0406}' | '\u{0456}' | '\u{0407}' | '\u{0457}' | '\u{0490}' | '\u{0491}')
}

fn has_ua_markers(word: &str) -> bool {
    word.chars().any(|c| matches!(c, 'і' | 'І' | 'ї' | 'Ї' | 'є' | 'Є' | 'ґ' | 'Ґ'))
}

fn has_ru_markers(word: &str) -> bool {
    word.chars().any(|c| matches!(c, 'ы' | 'Ы' | 'э' | 'Э' | 'ъ' | 'Ъ' | 'ё' | 'Ё'))
}

/// Apply case correction to a lowercase `word` based on how the original keys
/// were typed (capitalisation stored in `original_entries`).
///
/// Three patterns are handled:
/// * ALL_CAPS  — every key held Shift/Caps → output ALL_CAPS
/// * Inversion — first key lower, rest upper (Caps Lock was on, then turned off)
///               → output Title case  e.g. "gHBDTN" → "Привет"
/// * Default   — return the word unchanged (already lowercase)
fn apply_case_correction(original_entries: &[Entry], word: &str) -> String {
    if original_entries.is_empty() {
        return word.to_string();
    }
    // CapsLock inversion: first key was lowered by Shift (shift XOR caps = false),
    // rest uppercase from CapsLock alone. User intended sentence-case.
    let inversion = original_entries.len() >= 3
        && !original_entries[0].is_upper
        && original_entries[1..].iter().all(|e| e.is_upper);
    if inversion {
        let mut chars = word.chars();
        let first = chars.next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let rest: String = chars.flat_map(|c| c.to_lowercase()).collect();
        return first + &rest;
    }
    // General case: apply per-character case directly from entries.
    // Handles ALL_CAPS, Sentence case, camelCase, and all-lower uniformly.
    // `word` is always lowercase on entry; entries supply the intended case.
    word.chars()
        .zip(original_entries.iter())
        .map(|(c, e)| if e.is_upper { c.to_uppercase().next().unwrap_or(c) } else { c })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const LANG_EN: u16 = 0x0409; // English (US)
    const LANG_RU: u16 = 0x0419; // Russian
    const LANG_UA: u16 = 0x0422; // Ukrainian

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
                '\\' => 0xDC,
                _ => panic!("untranslatable char {}", c),
            };
            buf.push(vk, is_upper);
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
        push_en_word(&mut buf, "abc");
        assert!(buf.detect_mismatch(LANG_RU).is_none());
        assert!(buf.detect_mismatch(LANG_EN).is_none());
        assert!(buf.detect_mismatch(LANG_UA).is_none());
    }

    #[test]
    fn single_char_force_switch_returns_some() {
        let mut buf = WordBuffer::new();
        buf.push(0x48, false); // 'h' in EN → 'р' in RU/UA
        assert!(buf.force_switch(LANG_RU).is_some());
        assert!(buf.force_switch(LANG_UA).is_some());
    }

    #[test]
    fn empty_buffer_returns_none() {
        let buf = WordBuffer::new();
        assert!(buf.detect_mismatch(LANG_RU).is_none());
        assert!(buf.force_switch(LANG_EN).is_none());
    }

    // ── Core switching scenarios ──────────────────────────────────────────────

    #[test]
    fn notepad_typed_in_ru_layout_switches_to_en() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "notepad");
        let action = buf.detect_mismatch(LANG_RU).expect("should detect mismatch");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word.to_lowercase(), "notepad");
        assert_eq!(action.backspaces, 7);
        assert!(!action.original_word.is_empty());
    }

    #[test]
    fn ghbdtn_typed_in_en_layout_switches_to_ru_privet() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "ghbdtn");
        let action = buf.detect_mismatch(LANG_EN).expect("should detect mismatch");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word.to_lowercase(), "привет");
        assert_eq!(action.original_word.to_lowercase(), "ghbdtn");
    }

    #[test]
    fn scyedfyyz_typed_in_en_layout_switches_to_ua_isnuvannya() {
        let mut buf = WordBuffer::new();
        push_en_chars(&mut buf, "scyedfyyz"); // існування
        let action = buf.detect_mismatch(LANG_EN).expect("should detect mismatch");
        assert_eq!(action.target_lang, LANG_UA);
        assert_eq!(action.new_word.to_lowercase(), "існування");
        assert_eq!(action.original_word.to_lowercase(), "scyedfyyz");
    }

    #[test]
    fn hello_in_en_layout_does_not_switch() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "hello");
        assert!(buf.detect_mismatch(LANG_EN).is_none());
    }

    #[test]
    fn privet_in_ru_layout_does_not_switch() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "ghbdtn");
        assert!(buf.detect_mismatch(LANG_RU).is_none());
    }

    #[test]
    fn eto_typed_in_en_layout_switches_to_ru() {
        let mut buf = WordBuffer::new();
        buf.push(0xDE, false); // '\'' -> 'э'
        buf.push(0x4E, false); // 'n' -> 'т'
        buf.push(0x4A, false); // 'j' -> 'о'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch 'это' from EN to RU");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "это");
    }

    #[test]
    fn test_dictionary_sorting() {
        for window in COMMON_RU_SHORT.windows(2) {
            assert!(window[0] < window[1], "COMMON_RU_SHORT not sorted: {} >= {}", window[0], window[1]);
        }
        for window in COMMON_EN_SHORT.windows(2) {
            assert!(window[0] < window[1], "COMMON_EN_SHORT not sorted: {} >= {}", window[0], window[1]);
        }
        for window in COMMON_UA_SHORT.windows(2) {
            assert!(window[0] < window[1], "COMMON_UA_SHORT not sorted: {} >= {}", window[0], window[1]);
        }
    }

    #[test]
    fn single_letter_words_switch() {
        let mut buf = WordBuffer::new();

        // EN→Cyrillic single-char switching: 'd' (→ RU 'в') must switch to RU 'в'.
        buf.push(0x44, false); // 'd'
        let action = buf.detect_mismatch(LANG_EN).expect("single 'd' in EN layout should switch to RU 'в'");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "в");
        buf.clear();

        // 'a' is in common_en_single — must not switch from EN layout.
        buf.push(0x41, false); // 'a'
        assert!(buf.detect_mismatch(LANG_EN).is_none());
        buf.clear();

        // Cyrillic→EN single-char switching is still active:
        // 'a' in RU layout maps to 'ф', which is not a common RU single-char word,
        // and the EN result 'a' IS in common_en_single → switch to EN.
        buf.push(0x41, false); // 'a' key in RU layout → 'ф' which is not a RU word
        let action = buf.detect_mismatch(LANG_RU).expect("should switch single char ф -> a");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "a");
        buf.clear();
    }

    #[test]
    fn technical_abbreviations_switch() {
        let mut buf = WordBuffer::new();

        buf.push(0x43, false); // 'c' -> 'с'
        buf.push(0x4C, false); // 'l' -> 'д'
        buf.push(0x49, false); // 'i' -> 'ш'
        let action = buf.detect_mismatch(LANG_RU).expect("should switch 'сдш' -> 'cli'");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "cli");
        buf.clear();

        // Test the new vpn short abbreviation
        buf.push(0x56, false); // 'v' -> 'м'
        buf.push(0x50, false); // 'p' -> 'з'
        buf.push(0x4E, false); // 'n' -> 'т'
        let action = buf.detect_mismatch(LANG_RU).expect("should switch 'мзт' -> 'vpn'");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "vpn");
        buf.clear();

        // Test the new мин short word
        buf.push(0x56, false); // 'v' -> 'м'
        buf.push(0x42, false); // 'b' -> 'и'
        buf.push(0x59, false); // 'y' -> 'н'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch 'vby' -> 'мин'");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "мин");
        buf.clear();
    }

    #[test]
    fn test_long_ru_words_switch() {
        let mut buf = WordBuffer::new();

        // 1. ekexitybz -> улучшения
        push_en_chars(&mut buf, "ekexitybz");
        let action = buf.detect_mismatch(LANG_EN).expect("should switch ekexitybz");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "улучшения");
        buf.clear();

        // 2. hf,jnf -> работа
        push_en_chars(&mut buf, "hf,jnf");
        let action = buf.detect_mismatch(LANG_EN).expect("should switch hf,jnf");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "работа");
        buf.clear();
    }

    #[test]
    fn test_sensitivity_levels() {
        let candidates = &[
            ("ghbdtn", LANG_EN),    // привет
            ("notepad", LANG_RU),   // notepad
        ];
        
        let mut found = false;
        for &(word, lang) in candidates {
            let mut buf = WordBuffer::new();
            if lang == LANG_EN {
                push_en_chars(&mut buf, word);
            } else {
                push_en_word(&mut buf, word);
            }
            if buf.detect_mismatch_with_sensitivity(lang, 1.0).is_some() {
                assert!(buf.detect_mismatch_with_sensitivity(lang, 1.5).is_some());

                let snap = buf.detection_snapshot().unwrap();
                let diff = (snap.score_ru - snap.score_en).abs();
                let threshold = switching_threshold(snap.len);
                let s_low = (threshold / diff) * 0.8;
                assert!(buf.detect_mismatch_with_sensitivity(lang, s_low).is_none());
                found = true;
                break;
            }
        }
        assert!(found, "Could not find a test word satisfying sensitivity threshold.");
    }

    #[test]
    fn test_case_inversion_correction() {
        let mut buf = WordBuffer::new();
        // gHBDTN -> Привет
        buf.push(0x47, false); // 'g'
        buf.push(0x48, true);  // 'H'
        buf.push(0x42, true);  // 'B'
        buf.push(0x44, true);  // 'D'
        buf.push(0x54, true);  // 'T'
        buf.push(0x4E, true);  // 'N'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "Привет");
    }

    #[test]
    fn test_cross_cyrillic_switching() {
        let mut buf = WordBuffer::new();
        // User typed 'ыснування' (which in EN keys is 'scyedfyyz') in RU layout (LANG_RU)
        // UA candidate is 'існування' which contains Ukrainian marker 'і' and is a common UA word.
        push_en_chars(&mut buf, "scyedfyyz");
        let action = buf.detect_mismatch(LANG_RU).expect("should cross-switch RU -> UA");
        assert_eq!(action.target_lang, LANG_UA);
        assert_eq!(action.new_word, "існування");
        buf.clear();

        // User typed 'єто' (VK sequence "'nj") in UA layout (LANG_UA).
        // '\'' → є(UA) / э(RU); both are Cyrillic.
        // "это" IS in COMMON_RU_SHORT and has the RU marker 'э';
        // "єто" is NOT in COMMON_UA_SHORT → short-word dictionary path
        // fires and cross-switches UA → RU.
        push_en_chars(&mut buf, "'nj");
        let action = buf.detect_mismatch(LANG_UA).expect("should cross-switch UA -> RU");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "это");
    }

    #[test]
    fn repeated_key_does_not_switch() {
        // "jjjjj" in EN layout → "ооооо" in RU. Repeating a single key must
        // never trigger a switch regardless of how many times it is pressed.
        let mut buf = WordBuffer::new();
        for _ in 0..5 {
            buf.push(0x4A, false); // VK_J → 'j' in EN, 'о' in RU
        }
        assert!(buf.detect_mismatch(LANG_EN).is_none(), "repeated key must not switch");
        buf.clear();

        // Same check for Cyrillic layout direction
        for _ in 0..5 {
            buf.push(0x4A, false);
        }
        assert!(buf.detect_mismatch(LANG_RU).is_none(), "repeated key in RU must not switch");
    }

    #[test]
    fn jpeg_in_en_layout_does_not_switch() {
        // "jpeg" has a terrible EN bigram score (j→p is rare in text corpora)
        // but is a valid English abbreviation — must not switch to Cyrillic.
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "jpeg");
        assert!(buf.detect_mismatch(LANG_EN).is_none(), "jpeg must not switch");
    }

    #[test]
    fn nujno_with_punct_en_transliteration_switches_at_boundary() {
        // "нужно" → "ye;yj" in EN layout (';' = VK_OEM_1 maps to 'ж' in RU).
        // Bigram delta is too small (~0.26) to cross the threshold; the punct
        // dict shortcut should catch it instead.
        let mut buf = WordBuffer::new();
        push_en_chars(&mut buf, "ye;yj");
        let action = buf.detect_mismatch(LANG_EN).expect("punct dict shortcut should switch нужно");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word.to_lowercase(), "нужно");
        assert_eq!(action.backspaces, 5);
    }

    #[test]
    fn nujno_with_punct_switches_on_the_fly() {
        // "нужно" → "ye;yj": the punct dict shortcut must also fire on-the-fly
        // (at 5 chars mid-word) because "ye;yj" contains ';' and cannot be a
        // real English word, even though ru_lower == ua_lower == "нужно".
        let mut buf = WordBuffer::new();
        push_en_chars(&mut buf, "ye;yj");
        let action = buf.detect_mismatch_on_the_fly(LANG_EN, 1.0)
            .expect("punct dict shortcut should fire on-the-fly for нужно");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word.to_lowercase(), "нужно");
        assert_eq!(action.backspaces, 4);
    }

    #[test]
    fn wsl_in_ru_layout_switches_to_en() {
        let mut buf = WordBuffer::new();
        // "wsl" in RU layout: VK_W→ц, VK_S→ы, VK_L→д → ru="цыд", not in dict
        // while en="wsl" IS in COMMON_EN_SHORT → switch RU→EN.
        push_en_word(&mut buf, "wsl");
        let action = buf.detect_mismatch(LANG_RU).expect("wsl in RU layout should switch to EN");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "wsl");
    }

    #[test]
    fn bulk_in_ru_layout_switches_to_en() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "bulk"); // VK_B/U/L/K → RU: и/г/д/л = "игдл"
        let action = buf.detect_mismatch(LANG_RU).expect("bulk in RU layout should switch to EN");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "bulk");
    }

    #[test]
    fn po_in_en_layout_switches_to_ru() {
        let mut buf = WordBuffer::new();
        // "по" typed from EN layout: VK_G (→ п) + VK_J (→ о) = en="gj", ru="по"
        push_en_word(&mut buf, "gj");
        let action = buf.detect_mismatch(LANG_EN).expect("по in EN layout should switch to RU");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "по");
    }

    #[test]
    fn ne_in_en_layout_switches_to_ru() {
        let mut buf = WordBuffer::new();
        // "не" typed from EN layout: VK_Y (→ н) + VK_T (→ е) = en="yt", ru="не"
        push_en_word(&mut buf, "yt");
        let action = buf.detect_mismatch(LANG_EN).expect("не in EN layout should switch to RU");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "не");
    }

    // ── force_switch direction after on-the-fly ───────────────────────────────

    #[test]
    fn force_switch_uses_entry_lang_not_current_lang() {
        // Simulates the scenario where on-the-fly detection already switched the
        // foreground layout to EN (so foreground_lang() would return LANG_EN),
        // but the user's original typing started in RU layout.
        // force_switch must use entry_lang (RU), not the current lang (EN).
        let mut buf = WordBuffer::new();
        buf.set_entry_lang(LANG_RU); // set as if first key was typed in RU layout
        // "пшерги" = VK_G/I/T/H/U/B → en="github"
        push_en_word(&mut buf, "github");
        buf.has_switched = true; // simulate on-the-fly already fired

        // Even though we pass LANG_EN (current foreground lang after on-the-fly switch),
        // force_switch must use entry_lang=LANG_RU and go Cyrillic→EN.
        let action = buf.force_switch(LANG_EN).expect("should switch to EN using entry_lang");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "github");
    }

    #[test]
    fn force_switch_entry_lang_fallback_when_unset() {
        // When entry_lang is 0 (unset, e.g. in tests that call push directly),
        // force_switch falls back to the passed active_lang.
        let mut buf = WordBuffer::new();
        // entry_lang is 0 by default — no set_entry_lang call
        push_en_word(&mut buf, "bulk"); // VK_B/U/L/K → en="bulk", ru="игдл"
        let action = buf.force_switch(LANG_RU).expect("fallback to active_lang=LANG_RU");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word, "bulk");
    }

    #[test]
    fn on_the_fly_switching() {
        let mut buf = WordBuffer::new();
        // User types the first 5 keys of "scyedfyyz" (→ "існування" in UA layout).
        // 's'→і(UA)/ы(RU) ensures RU and UA translations differ, bypassing the
        // `on_the_fly && ru_lower == ua_lower` guard.
        // "існув" has UA marker 'і'; "ыснув" starts with 'ы' (rare in RU), so
        // UA model wins and the on-the-fly switch fires.
        push_en_chars(&mut buf, "scyed");
        let action = buf.detect_mismatch_on_the_fly(LANG_EN, 1.0).expect("should switch on-the-fly");
        assert_eq!(action.target_lang, LANG_UA);
        assert_eq!(action.backspaces, 4);
        assert_eq!(action.new_word, "існув");
    }

    #[test]
    fn usb_short_word_boundary_switch() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "usb");
        let action = buf.detect_mismatch(LANG_RU).expect("should detect mismatch for usb");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.new_word.to_lowercase(), "usb");
    }

    #[test]
    fn long_word_on_the_fly_switch() {
        let mut buf = WordBuffer::new();
        push_en_word(&mut buf, "bacvkspace");
        let action = buf.detect_mismatch_on_the_fly(LANG_RU, 1.0).expect("should switch on-the-fly for long word with RU marker");
        assert_eq!(action.target_lang, LANG_EN);
        assert_eq!(action.backspaces, 9);
        assert_eq!(action.new_word, "bacvkspace");
    }

    // ── Context-aware detection ───────────────────────────────────────────────

    #[test]
    fn sentence_case_preserved_on_switch() {
        // Bug regression: "Привет" typed in EN layout should switch to RU and
        // preserve the capital first letter.
        let mut buf = WordBuffer::new();
        // VK for G=0x47, H=0x48, B=0x42, D=0x44, T=0x54, N=0x4E
        // In EN layout: gHBDTN → гРИВЕТ (wrong) — but with is_upper on first, should be "Привет"
        buf.push(0x47, true);  // G (upper) → 'П' in RU
        buf.push(0x48, false); // H (lower) → 'р'
        buf.push(0x42, false); // B (lower) → 'и'
        buf.push(0x44, false); // D (lower) → 'в'
        buf.push(0x54, false); // T (lower) → 'е'
        buf.push(0x4E, false); // N (lower) → 'т'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch to RU");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "Привет", "first letter must remain capitalised");
    }

    #[test]
    fn context_suppresses_short_dict_switch() {
        // When Cyrillic context is dominant, short EN words (≤4) from the
        // dictionary should not trigger an automatic switch — they fall through
        // to the bigram model, which correctly skips them.
        let config = DetectionConfig {
            context_lang: Some(LANG_RU),
            ..DetectionConfig::default()
        };
        let mut buf = WordBuffer::new();
        // "ctrl" in RU layout → "секд"; step 1b would normally switch to EN,
        // but context=RU must suppress the dict switch.
        push_en_chars(&mut buf, "ctrl");
        let (action, _) = buf.detect_with_snapshot(LANG_RU, 1.0, &config);
        assert!(action.is_none(), "context=RU must suppress dict switch for 'ctrl'");
    }

    #[test]
    fn context_does_not_suppress_long_words() {
        // Context suppression only applies to ≤4 char words.
        // A long clearly-English word must still switch even with Cyrillic context.
        let config = DetectionConfig {
            context_lang: Some(LANG_RU),
            ..DetectionConfig::default()
        };
        let mut buf = WordBuffer::new();
        // "function" in RU layout → "aeyrwbzy"; strongly EN bigrams.
        push_en_chars(&mut buf, "function");
        let (action, _) = buf.detect_with_snapshot(LANG_RU, 1.0, &config);
        assert!(action.is_some(), "long EN word must still switch despite Cyrillic context");
        assert_eq!(action.unwrap().target_lang, LANG_EN);
    }
}
