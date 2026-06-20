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

const COMMON_UA_SHORT: &[&str] = &[
    "або", "але", "без", "був", "вас", "ви", "все", "всю", "всі", "від",
    "він", "для", "до", "моя", "моє", "мої", "нам", "нас", "ней", "них",
    "ні", "при", "про", "під", "так", "там", "тим", "тут", "усе", "усі",
    "хто", "це", "цей", "цю", "ця", "чи", "як", "яка", "яке", "які",
    "із", "їм", "їх", "її"
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
/// spelled identically in both languages.  The UA corpus is ~10× larger than
/// the RU corpus, which causes UA bigram scores to be systematically 0.1–0.2
/// higher for shared Slavic vocabulary.  A delta below this threshold is
/// treated as a tie and resolved in favour of RU.
const RU_UA_SCORE_MIN_DELTA: f32 = 0.1;

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
        self.has_switched = false;
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
    /// Returns None only when the buffer is empty or the translation is invalid.
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
        let ua: String = self
            .entries
            .iter()
            .filter_map(|e| layout::vk_to_ua(e.vk, e.is_upper))
            .collect();

        if layout::hkl_is_russian(active_lang) || layout::hkl_is_ukrainian(active_lang) {
            if en.chars().count() != self.entries.len() {
                return None;
            }
            let new_word = apply_case_correction(&self.entries, &en);
            Some(SwitchAction {
                backspaces: self.len(),
                new_word,
                target_lang: layout::LANG_EN_US,
                original_word: if layout::hkl_is_russian(active_lang) { ru } else { ua },
            })
        } else if layout::hkl_is_english(active_lang) {
            let ru_valid = ru.chars().count() == self.entries.len() && ru.chars().all(is_cyrillic_or_ua);
            let ua_valid = ua.chars().count() == self.entries.len() && ua.chars().all(is_cyrillic_or_ua);

            if !ru_valid && !ua_valid {
                return None;
            }

            let to_ru = if ru_valid && ua_valid {
                let ru_in_dict = RU_COMMON_WORDS.binary_search(&ru.to_lowercase().as_str()).is_ok();
                let ua_in_dict = UA_COMMON_WORDS.binary_search(&ua.to_lowercase().as_str()).is_ok();
                if ru_in_dict && !ua_in_dict {
                    true
                } else if ua_in_dict && !ru_in_dict {
                    false
                } else {
                    let score_ru = bigrams::score_ru(&ru.to_lowercase());
                    let score_ua = bigrams::score_ua(&ua.to_lowercase());
                    score_ru >= score_ua
                }
            } else {
                ru_valid
            };

            if to_ru {
                let new_word = apply_case_correction(&self.entries, &ru);
                Some(SwitchAction {
                    backspaces: self.len(),
                    new_word,
                    target_lang: layout::LANG_RU,
                    original_word: en,
                })
            } else {
                let new_word = apply_case_correction(&self.entries, &ua);
                Some(SwitchAction {
                    backspaces: self.len(),
                    new_word,
                    target_lang: layout::LANG_UA,
                    original_word: en,
                })
            }
        } else {
            None
        }
    }

    pub fn detect_mismatch_on_the_fly(&self, active_lang: u16, sensitivity: f32) -> Option<SwitchAction> {
        self.detect_impl_opt(active_lang, sensitivity, true)
    }

    fn detect_impl(&self, active_lang: u16, sensitivity: f32) -> Option<SwitchAction> {
        self.detect_impl_opt(active_lang, sensitivity, false)
    }

    fn detect_impl_opt(&self, active_lang: u16, sensitivity: f32, on_the_fly: bool) -> Option<SwitchAction> {
        if self.has_switched {
            return None;
        }
        let len = self.entries.len();
        let backspaces = if on_the_fly { len - 1 } else { len };

        if on_the_fly {
            if len < 5 {
                return None;
            }
        } else {
            if len == 0 {
                return None;
            }
        }

        // Translate VK codes through EN, RU, and UA.
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

        let en_ok = en.chars().count() == len;
        let ru_ok = ru.chars().count() == len;
        let ua_ok = ua.chars().count() == len;

        let en_lower = en.to_lowercase();
        let ru_lower = ru.to_lowercase();
        let ua_lower = ua.to_lowercase();

        // ── 0. Check whitelisted / ignored words (Undo feedback loop) ────────
        let (ignored_words, dev_exceptions) = crate::globals::SETTINGS
            .get()
            .and_then(|s| s.try_read().ok())
            .map(|s| (s.ignored_words.clone(), s.dev_exceptions.clone()))
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        let word_to_check = if layout::hkl_is_russian(active_lang) {
            &ru_lower
        } else if layout::hkl_is_ukrainian(active_lang) {
            &ua_lower
        } else {
            &en_lower
        };

        if ignored_words.contains(word_to_check) {
            return None;
        }

        // ── 1. Check if the word is a valid word in the current layout ────────
        if layout::hkl_is_russian(active_lang) && ru_ok && RU_COMMON_WORDS.binary_search(&ru_lower.as_str()).is_ok() {
            return None;
        } else if layout::hkl_is_ukrainian(active_lang) && ua_ok && UA_COMMON_WORDS.binary_search(&ua_lower.as_str()).is_ok() {
            return None;
        } else if layout::hkl_is_english(active_lang) && en_ok && EN_COMMON_WORDS.binary_search(&en_lower.as_str()).is_ok() {
            return None;
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
        if !on_the_fly && len == 1 {
            if is_dev_app {
                return None;
            }
            let common_ru_single = ["в", "и", "а", "о", "с", "у", "я", "к"];
            let common_ua_single = ["в", "і", "а", "о", "у", "я", "є", "з"];
            let common_en_single = ["a", "i"];

            if layout::hkl_is_russian(active_lang) {
                if ru_ok && en_ok && !common_ru_single.contains(&ru_lower.as_str()) && common_en_single.contains(&en_lower.as_str()) {
                    let new_word = apply_case_correction(&self.entries, &en);
                    return Some(SwitchAction {
                        backspaces,
                        new_word,
                        target_lang: layout::LANG_EN_US,
                        original_word: ru,
                    });
                }
            } else if layout::hkl_is_ukrainian(active_lang) {
                if ua_ok && en_ok && !common_ua_single.contains(&ua_lower.as_str()) && common_en_single.contains(&en_lower.as_str()) {
                    let new_word = apply_case_correction(&self.entries, &en);
                    return Some(SwitchAction {
                        backspaces,
                        new_word,
                        target_lang: layout::LANG_EN_US,
                        original_word: ua,
                    });
                }
            } else if layout::hkl_is_english(active_lang) {
                if en_ok && !common_en_single.contains(&en_lower.as_str()) {
                    let to_ru = ru_ok && common_ru_single.contains(&ru_lower.as_str());
                    let to_ua = ua_ok && common_ua_single.contains(&ua_lower.as_str());
                    if to_ru && to_ua {
                        let new_word = apply_case_correction(&self.entries, &ru);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_RU,
                            original_word: en,
                        });
                    } else if to_ru {
                        let new_word = apply_case_correction(&self.entries, &ru);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_RU,
                            original_word: en,
                        });
                    } else if to_ua {
                        let new_word = apply_case_correction(&self.entries, &ua);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_UA,
                            original_word: en,
                        });
                    }
                }
            }
            return None;
        }

        // ── 4. Dictionary-based check for short words (2-3 chars) ─────────────
        if !on_the_fly && (len == 2 || len == 3) {
            if is_dev_app {
                return None;
            }
            if layout::hkl_is_russian(active_lang) {
                if ru_ok && en_ok {
                    let is_common_en = COMMON_EN_SHORT.binary_search(&en_lower.as_str()).is_ok();
                    let is_common_ru = COMMON_RU_SHORT.binary_search(&ru_lower.as_str()).is_ok();
                    if is_common_en && !is_common_ru {
                        let new_word = apply_case_correction(&self.entries, &en);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_EN_US,
                            original_word: ru,
                        });
                    }

                    // Cross-Cyrillic check for short words
                    let is_common_ua = COMMON_UA_SHORT.binary_search(&ua_lower.as_str()).is_ok();
                    if is_common_ua && !is_common_ru && has_ua_markers(&ua_lower) {
                        let new_word = apply_case_correction(&self.entries, &ua);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_UA,
                            original_word: ru,
                        });
                    }
                }
            } else if layout::hkl_is_ukrainian(active_lang) {
                if ua_ok && en_ok {
                    let is_common_en = COMMON_EN_SHORT.binary_search(&en_lower.as_str()).is_ok();
                    let is_common_ua = COMMON_UA_SHORT.binary_search(&ua_lower.as_str()).is_ok();
                    if is_common_en && !is_common_ua {
                        let new_word = apply_case_correction(&self.entries, &en);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_EN_US,
                            original_word: ua,
                        });
                    }

                    // Cross-Cyrillic check for short words
                    let is_common_ru = COMMON_RU_SHORT.binary_search(&ru_lower.as_str()).is_ok();
                    if is_common_ru && !is_common_ua && has_ru_markers(&ru_lower) {
                        let new_word = apply_case_correction(&self.entries, &ru);
                        return Some(SwitchAction {
                            backspaces,
                            new_word,
                            target_lang: layout::LANG_RU,
                            original_word: ua,
                        });
                    }
                }
            } else if layout::hkl_is_english(active_lang) {
                if en_ok {
                    let is_common_en = COMMON_EN_SHORT.binary_search(&en_lower.as_str()).is_ok();
                    let is_common_ru = ru_ok && COMMON_RU_SHORT.binary_search(&ru_lower.as_str()).is_ok();
                    let is_common_ua = ua_ok && COMMON_UA_SHORT.binary_search(&ua_lower.as_str()).is_ok();

                    if !is_common_en {
                        if is_common_ru && is_common_ua {
                            let score_ru = bigrams::score_ru(&ru_lower);
                            let score_ua = bigrams::score_ua(&ua_lower);
                            // Identical spelling in both languages: the UA corpus is
                            // larger, making UA scores systematically ~0.1–0.2 higher
                            // for shared vocabulary.  Require a meaningful delta before
                            // choosing UA; otherwise default to RU.
                            let prefer_ru = if ru_lower == ua_lower {
                                score_ua - score_ru < RU_UA_SCORE_MIN_DELTA
                            } else {
                                score_ru >= score_ua
                            };
                            if prefer_ru {
                                let new_word = apply_case_correction(&self.entries, &ru);
                                return Some(SwitchAction {
                                    backspaces,
                                    new_word,
                                    target_lang: layout::LANG_RU,
                                    original_word: en,
                                });
                            } else {
                                let new_word = apply_case_correction(&self.entries, &ua);
                                return Some(SwitchAction {
                                    backspaces,
                                    new_word,
                                    target_lang: layout::LANG_UA,
                                    original_word: en,
                                });
                            }
                        } else if is_common_ru {
                            let new_word = apply_case_correction(&self.entries, &ru);
                            return Some(SwitchAction {
                                    backspaces,
                                    new_word,
                                    target_lang: layout::LANG_RU,
                                    original_word: en,
                            });
                        } else if is_common_ua {
                            let new_word = apply_case_correction(&self.entries, &ua);
                            return Some(SwitchAction {
                                    backspaces,
                                    new_word,
                                    target_lang: layout::LANG_UA,
                                    original_word: en,
                            });
                        }
                    }
                }
            }
            return None;
        }

        // ── 5. Standard bigram/trigram language-model check (>= 4 chars) ──────
        let min_len = if on_the_fly { 5 } else { adjusted_min_len };
        if len < min_len {
            return None;
        }

        let base_threshold = (switching_threshold(len) / sensitivity) * dev_threshold_multiplier;

        if layout::hkl_is_russian(active_lang) {
            if !ru_ok || !en_ok {
                return None;
            }
            if !ru.chars().all(is_cyrillic_or_ua) {
                return None;
            }
            let score_en = bigrams::score_en(&en_lower);
            let score_ru = bigrams::score_ru(&ru_lower);
            let score_ua = if ua_ok && ua.chars().all(is_cyrillic_or_ua) {
                bigrams::score_ua(&ua_lower)
            } else {
                f32::NEG_INFINITY
            };

            let en_threshold = base_threshold * if has_ru_markers(&ru_lower) {
                if on_the_fly { 2.5 } else { 1.5 }
            } else {
                if on_the_fly { 2.0 } else { 1.0 }
            };

            if score_en - score_ru > en_threshold {
                let new_word = apply_case_correction(&self.entries, &en);
                return Some(SwitchAction {
                    backspaces,
                    new_word,
                    target_lang: layout::LANG_EN_US,
                    original_word: ru,
                });
            }

            // Cross-Cyrillic switch RU → UA
            if score_ua.is_finite() {
                let cross_threshold = base_threshold * if has_ua_markers(&ua_lower) {
                    1.0
                } else {
                    1.5
                };
                if score_ua - score_ru > cross_threshold {
                    let new_word = apply_case_correction(&self.entries, &ua);
                    return Some(SwitchAction {
                        backspaces,
                        new_word,
                        target_lang: layout::LANG_UA,
                        original_word: ru,
                    });
                }
            }
        } else if layout::hkl_is_ukrainian(active_lang) {
            if !ua_ok || !en_ok {
                return None;
            }
            if !ua.chars().all(is_cyrillic_or_ua) {
                return None;
            }
            let score_en = bigrams::score_en(&en_lower);
            let score_ua = bigrams::score_ua(&ua_lower);
            let score_ru = if ru_ok && ru.chars().all(is_cyrillic_or_ua) {
                bigrams::score_ru(&ru_lower)
            } else {
                f32::NEG_INFINITY
            };

            let en_threshold = base_threshold * if has_ua_markers(&ua_lower) {
                if on_the_fly { 2.5 } else { 1.5 }
            } else {
                if on_the_fly { 2.0 } else { 1.0 }
            };

            if score_en - score_ua > en_threshold {
                let new_word = apply_case_correction(&self.entries, &en);
                return Some(SwitchAction {
                    backspaces,
                    new_word,
                    target_lang: layout::LANG_EN_US,
                    original_word: ua,
                });
            }

            // Cross-Cyrillic switch UA → RU
            if score_ru.is_finite() {
                let cross_threshold = base_threshold * if has_ru_markers(&ru_lower) {
                    1.0
                } else {
                    1.5
                };
                if score_ru - score_ua > cross_threshold {
                    let new_word = apply_case_correction(&self.entries, &ru);
                    return Some(SwitchAction {
                        backspaces,
                        new_word,
                        target_lang: layout::LANG_RU,
                        original_word: ua,
                    });
                }
            }
        } else if layout::hkl_is_english(active_lang) {
            if !en_ok {
                return None;
            }
            let score_en = bigrams::score_en(&en_lower);

            let ru_candidate = ru_ok && ru.chars().all(is_cyrillic_or_ua);
            let ua_candidate = ua_ok && ua.chars().all(is_cyrillic_or_ua);

            if !ru_candidate && !ua_candidate {
                return None;
            }

            if on_the_fly && ru_lower == ua_lower {
                return None;
            }

            let score_ru = if ru_candidate { bigrams::score_ru(&ru_lower) } else { f32::NEG_INFINITY };
            let score_ua = if ua_candidate { bigrams::score_ua(&ua_lower) } else { f32::NEG_INFINITY };

            let ru_diff = score_ru - score_en;
            let ua_diff = score_ua - score_en;

            let ru_threshold = base_threshold * if has_ru_markers(&ru_lower) {
                if on_the_fly { 1.3 } else { 0.7 }
            } else {
                if on_the_fly { 2.0 } else { 1.0 }
            };

            let ua_threshold = base_threshold * if has_ua_markers(&ua_lower) {
                if on_the_fly { 1.3 } else { 0.7 }
            } else {
                if on_the_fly { 2.0 } else { 1.0 }
            };

            let ru_switches = ru_diff > ru_threshold;
            let ua_switches = ua_diff > ua_threshold;

            if ru_switches && ua_switches {
                let ru_in_dict = RU_COMMON_WORDS.binary_search(&ru_lower.as_str()).is_ok();
                let ua_in_dict = UA_COMMON_WORDS.binary_search(&ua_lower.as_str()).is_ok();
                
                let to_ru = if ru_in_dict && !ua_in_dict {
                    true
                } else if ua_in_dict && !ru_in_dict {
                    false
                } else if ru_lower == ua_lower {
                    // Same spelling in both: prefer RU unless UA has a meaningful
                    // score advantage (guards against corpus-size bias).
                    score_ua - score_ru < RU_UA_SCORE_MIN_DELTA
                } else {
                    score_ru >= score_ua
                };

                if to_ru {
                    let new_word = apply_case_correction(&self.entries, &ru);
                    return Some(SwitchAction {
                        backspaces,
                        new_word,
                        target_lang: layout::LANG_RU,
                        original_word: en,
                    });
                } else {
                    let new_word = apply_case_correction(&self.entries, &ua);
                    return Some(SwitchAction {
                        backspaces,
                        new_word,
                        target_lang: layout::LANG_UA,
                        original_word: en,
                    });
                }
            } else if ru_switches {
                let new_word = apply_case_correction(&self.entries, &ru);
                return Some(SwitchAction {
                    backspaces,
                    new_word,
                    target_lang: layout::LANG_RU,
                    original_word: en,
                });
            } else if ua_switches {
                let new_word = apply_case_correction(&self.entries, &ua);
                return Some(SwitchAction {
                    backspaces,
                    new_word,
                    target_lang: layout::LANG_UA,
                    original_word: en,
                });
            }
        }

        None
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

fn apply_case_correction(original_entries: &[Entry], word: &str) -> String {
    if original_entries.len() >= 3
        && !original_entries[0].is_upper
        && original_entries[1..].iter().all(|e| e.is_upper)
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            let mut result = first.to_uppercase().to_string();
            for c in chars {
                result.push_str(&c.to_lowercase().to_string());
            }
            result
        } else {
            word.to_string()
        }
    } else {
        word.to_string()
    }
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
        
        buf.push(0x44, false); // 'd' -> 'в'
        let action = buf.detect_mismatch(LANG_EN).expect("should switch single char d -> в");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "в");
        buf.clear();

        buf.push(0x41, false); // 'a'
        assert!(buf.detect_mismatch(LANG_EN).is_none());
        buf.clear();

        buf.push(0x41, false); // 'a' maps to 'ф'
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

        // User typed 'данніе' (which in EN keys is 'lfyyst') in UA layout (LANG_UA)
        // RU candidate is 'данные' which is a common RU word.
        push_en_chars(&mut buf, "lfyyst");
        let action = buf.detect_mismatch(LANG_UA).expect("should cross-switch UA -> RU");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.new_word, "данные");
    }

    #[test]
    fn test_on_the_fly_switching() {
        let mut buf = WordBuffer::new();
        // User types 'ghbdt' (first 5 chars of 'ghbdtn' -> 'приве' in RU layout)
        push_en_chars(&mut buf, "ghbdt");
        let action = buf.detect_mismatch_on_the_fly(LANG_EN, 1.0).expect("should switch on-the-fly");
        assert_eq!(action.target_lang, LANG_RU);
        assert_eq!(action.backspaces, 4);
        assert_eq!(action.new_word, "приве");
    }
}
