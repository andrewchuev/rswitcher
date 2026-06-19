// Bigram language-model scoring for EN↔RU layout detection.
//
// Frequency tables are generated at compile time by build.rs from the text
// corpora in corpus/ru.txt and corpus/en.txt.  Every bigram probability is
// strictly positive thanks to Laplace (add-1) smoothing.
include!(concat!(env!("OUT_DIR"), "/bigrams_gen.rs"));

// Alphabet parameters ---------------------------------------------------------

const EN_BASE: u32 = 'a' as u32;
const EN_N: u32 = 26;

const RU_BASE: u32 = 'а' as u32; // U+0430
const RU_N: u32 = 32;

// Switching threshold (per-bigram, in log-space) ------------------------------
//
// A switch is proposed only when the ALTERNATIVE language scores this many
// nats (natural-log units) better PER BIGRAM than the current language.
//
// Intuition:
//   e^2.0 ≈ 7.4  — the alternative layout needs to be ~7× more likely per
//   pair for the word to be considered "typed in the wrong layout".
//
// Increase this value to reduce false positives; decrease to catch more cases.
#[allow(dead_code)]
pub const THRESHOLD_PER_BIGRAM: f32 = 1.5;

// Public API ------------------------------------------------------------------

/// Log-probability score of `word` under the English bigram model,
/// normalised by the number of bigrams (word.len() - 1).
/// Returns `f32::NEG_INFINITY` for words shorter than 2 characters.
pub fn score_en(word: &str) -> f32 {
    score(word, &EN_BIGRAMS, EN_BASE, EN_N)
}

/// Log-probability score of `word` under the Russian bigram model.
pub fn score_ru(word: &str) -> f32 {
    score(word, &RU_BIGRAMS, RU_BASE, RU_N)
}

pub(crate) fn score(word: &str, table: &[f32], base: u32, n: u32) -> f32 {
    let chars: Vec<Option<u32>> = word
        .chars()
        .map(|c| {
            c.to_lowercase().next().and_then(|lc| {
                // Normalise ё→е so it falls in the RU table range.
                let lc = if lc == 'ё' { 'е' } else { lc };
                let d = (lc as u32).checked_sub(base)?;
                if d < n { Some(d) } else { None }
            })
        })
        .collect();

    // If there are less than 2 valid alphabetical characters, we don't have
    // enough layout-specific bigram information, so return NEG_INFINITY.
    let valid_count = chars.iter().filter(|c| c.is_some()).count();
    if valid_count < 2 {
        return f32::NEG_INFINITY;
    }

    let penalty_ln = -10.0f32; // Log-probability penalty for invalid bigrams

    let sum: f32 = chars
        .windows(2)
        .map(|w| {
            match (w[0], w[1]) {
                (Some(c1), Some(c2)) => table[(c1 * n + c2) as usize].ln(),
                _ => penalty_ln,
            }
        })
        .sum();

    sum / (chars.len() - 1) as f32
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sanity: scores are finite for non-trivial inputs ─────────────────────

    #[test]
    fn score_en_is_finite_for_real_word() {
        let s = score_en("hello");
        assert!(s.is_finite(), "score_en('hello') = {}", s);
    }

    #[test]
    fn score_ru_is_finite_for_real_word() {
        let s = score_ru("привет");
        assert!(s.is_finite(), "score_ru('привет') = {}", s);
    }

    #[test]
    fn short_word_returns_neg_infinity() {
        assert_eq!(score_en("a"), f32::NEG_INFINITY);
        assert_eq!(score_ru("а"), f32::NEG_INFINITY);
    }

    // ── Real words outscore their wrong-layout gibberish ─────────────────────

    /// "notepad" in English must score better than the nonsense "тщеузфв" in
    /// Russian.  This is the fundamental property the switcher relies on.
    #[test]
    fn notepad_en_beats_tshcheyuzfv_ru() {
        let en = score_en("notepad");
        let ru = score_ru("тщеузфв"); // RU rendering of N O T E P A D keys
        assert!(
            en > ru + THRESHOLD_PER_BIGRAM,
            "score_en('notepad')={:.2} should exceed score_ru('тщеузфв')={:.2} by >{:.1}",
            en, ru, THRESHOLD_PER_BIGRAM
        );
    }

    /// "привет" in Russian must score better than the nonsense "ghbdtn" in
    /// English.
    #[test]
    fn privet_ru_beats_ghbdtn_en() {
        let ru = score_ru("привет");
        let en = score_en("ghbdtn"); // EN rendering of G H B D T N keys
        assert!(
            ru > en + THRESHOLD_PER_BIGRAM,
            "score_ru('привет')={:.2} should exceed score_en('ghbdtn')={:.2} by >{:.1}",
            ru, en, THRESHOLD_PER_BIGRAM
        );
    }

    /// "hello" is a real English word; its RU rendering "руддщ" must NOT
    /// outscore it by the switching threshold (so we don't false-positive).
    #[test]
    fn hello_en_not_false_positive() {
        let en = score_en("hello");
        let ru = score_ru("руддщ"); // RU rendering of H E L L O keys
        assert!(
            ru - en <= THRESHOLD_PER_BIGRAM,
            "score_ru('руддщ')={:.2} should NOT exceed score_en('hello')={:.2} by >{:.1}",
            ru, en, THRESHOLD_PER_BIGRAM
        );
    }

    /// "мир" is a real Russian word; its EN rendering "vbh" must NOT outscore
    /// it by the switching threshold.
    #[test]
    fn mir_ru_not_false_positive() {
        let ru = score_ru("мир");
        let en = score_en("vbh"); // EN rendering of V B H keys
        assert!(
            en - ru <= THRESHOLD_PER_BIGRAM,
            "score_en('vbh')={:.2} should NOT exceed score_ru('мир')={:.2} by >{:.1}",
            en, ru, THRESHOLD_PER_BIGRAM
        );
    }

    // ── Common words score higher than uncommon letter sequences ─────────────

    #[test]
    fn common_en_bigrams_beat_rare_ones() {
        // "the" — two of the most common EN bigrams (th, he)
        // "qzj" — extremely rare letter sequence
        let common = score_en("the");
        let rare = score_en("qzj");
        assert!(common > rare, "score_en('the')={:.2} score_en('qzj')={:.2}", common, rare);
    }

    #[test]
    fn common_ru_bigrams_beat_rare_ones() {
        // "ст" — very common Russian bigram
        // "щф" — extremely rare
        let common = score_ru("стол");
        let rare = score_ru("щфщф");
        assert!(common > rare, "score_ru('стол')={:.2} score_ru('щфщф')={:.2}", common, rare);
    }

    // ── Symmetry: EN model is ignorant of Cyrillic and vice-versa ────────────

    #[test]
    fn en_model_returns_neg_infinity_for_cyrillic() {
        // Cyrillic chars are outside the a-z range → no valid bigram pairs
        assert_eq!(score_en("привет"), f32::NEG_INFINITY);
    }

    #[test]
    fn ru_model_returns_neg_infinity_for_latin() {
        assert_eq!(score_ru("hello"), f32::NEG_INFINITY);
    }

    #[test]
    fn test_vverhu_does_not_false_switch() {
        let ru = score_ru("вверху");
        let en = score_en("ddth[e"); // "вверху" in EN layout
        // en should NOT exceed ru + 1.5, preventing false switch
        assert!(
            en - ru <= THRESHOLD_PER_BIGRAM,
            "score_en('ddth[e')={:.2} should NOT exceed score_ru('вверху')={:.2} by >{:.1}",
            en, ru, THRESHOLD_PER_BIGRAM
        );
    }

    #[test]
    fn test_dvuh_does_not_false_switch() {
        let ru = score_ru("двух");
        let en = score_en("lde["); // "двух" in EN layout
        // en should NOT exceed ru + 1.5, preventing false switch
        assert!(
            en - ru <= THRESHOLD_PER_BIGRAM,
            "score_en('lde[')={:.2} should NOT exceed score_ru('двух')={:.2} by >{:.1}",
            en, ru, THRESHOLD_PER_BIGRAM
        );
    }
}
