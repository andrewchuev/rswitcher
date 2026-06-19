/// Mapping: (EN lowercase char, RU lowercase char) for the standard Windows RU layout.
static MAP: &[(char, char)] = &[
    ('q', 'й'), ('w', 'ц'), ('e', 'у'), ('r', 'к'), ('t', 'е'),
    ('y', 'н'), ('u', 'г'), ('i', 'ш'), ('o', 'щ'), ('p', 'з'),
    ('[', 'х'), (']', 'ъ'),
    ('a', 'ф'), ('s', 'ы'), ('d', 'в'), ('f', 'а'), ('g', 'п'),
    ('h', 'р'), ('j', 'о'), ('k', 'л'), ('l', 'д'), (';', 'ж'), ('\'', 'э'),
    ('z', 'я'), ('x', 'ч'), ('c', 'с'), ('v', 'м'), ('b', 'и'),
    ('n', 'т'), ('m', 'ь'), (',', 'б'), ('.', 'ю'),
];

pub fn en_to_ru(ch: char) -> Option<char> {
    let lower = ch.to_lowercase().next()?;
    let ru = MAP.iter().find(|&&(en, _)| en == lower)?.1;
    if ch.is_uppercase() { ru.to_uppercase().next() } else { Some(ru) }
}

#[cfg(test)]
pub fn ru_to_en(ch: char) -> Option<char> {
    let lower = ch.to_lowercase().next()?;
    let en = MAP.iter().find(|&&(_, ru)| ru == lower)?.0;
    if ch.is_uppercase() { en.to_uppercase().next() } else { Some(en) }
}

// ── VK-code based translation ─────────────────────────────────────────────────
// VK codes for letters A-Z are 0x41-0x5A, matching ASCII 'A'-'Z'.
// Punctuation VK codes (OEM keys) on a US-QWERTY keyboard:

/// Returns the EN character produced by this VK code, or `None` if not in our table.
pub fn vk_to_en(vk: u16, is_upper: bool) -> Option<char> {
    let base: char = match vk {
        0x41..=0x5A => (vk as u8 | 0x20) as char, // 'a'..'z'
        0xBA => ';',
        0xBC => ',',
        0xBE => '.',
        0xDB => '[',
        0xDD => ']',
        0xDE => '\'',
        _ => return None,
    };
    if is_upper { base.to_uppercase().next() } else { Some(base) }
}

/// Returns the RU character produced by this VK code in the Russian QWERTY layout.
pub fn vk_to_ru(vk: u16, is_upper: bool) -> Option<char> {
    let en_base: char = match vk {
        0x41..=0x5A => (vk as u8 | 0x20) as char,
        0xBA => ';',
        0xBC => ',',
        0xBE => '.',
        0xDB => '[',
        0xDD => ']',
        0xDE => '\'',
        _ => return None,
    };
    let ru_base = en_to_ru(en_base)?;
    if is_upper { ru_base.to_uppercase().next() } else { Some(ru_base) }
}

/// Returns `true` if the VK code corresponds to a key that has an EN↔RU mapping.
pub fn is_translatable_vk(vk: u16) -> bool {
    matches!(vk, 0x41..=0x5A | 0xBA | 0xBC | 0xBE | 0xDB | 0xDD | 0xDE)
}

// ── Language constants ────────────────────────────────────────────────────────

/// Windows LANGID for Russian (Russia).
pub const LANG_RU: u16 = 0x0419;
/// Windows LANGID for English (United States).
pub const LANG_EN_US: u16 = 0x0409;

// ── Language detection from HKL low-word ─────────────────────────────────────
// The low 16 bits of HKL encode the input locale identifier.
// For standard (non-IME) layouts this equals the Windows LANGID.
// Primary language ID is the low 10 bits (mask 0x3FF).

/// Primary language ID for any English variant (EN-US, EN-GB, …).
const PRIMARY_EN: u16 = 0x0009;
/// Primary language ID for Russian.
const PRIMARY_RU: u16 = 0x0019;

pub fn hkl_is_english(lang_word: u16) -> bool { lang_word & 0x3FF == PRIMARY_EN }
pub fn hkl_is_russian(lang_word: u16) -> bool { lang_word & 0x3FF == PRIMARY_RU }

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── en_to_ru / ru_to_en round-trips ──────────────────────────────────────

    #[test]
    fn en_to_ru_spot_checks() {
        assert_eq!(en_to_ru('n'), Some('т'));
        assert_eq!(en_to_ru('f'), Some('а'));
        assert_eq!(en_to_ru('q'), Some('й'));
        assert_eq!(en_to_ru('['), Some('х'));
        assert_eq!(en_to_ru('.'), Some('ю'));
    }

    #[test]
    fn en_to_ru_preserves_case() {
        assert_eq!(en_to_ru('N'), Some('Т'));
        assert_eq!(en_to_ru('A'), Some('Ф'));
    }

    #[test]
    fn full_map_round_trips() {
        for &(en, ru) in MAP {
            assert_eq!(en_to_ru(en), Some(ru), "en_to_ru({:?}) failed", en);
            assert_eq!(ru_to_en(ru), Some(en), "ru_to_en({:?}) failed", ru);
        }
    }

    #[test]
    fn en_to_ru_returns_none_for_unmapped() {
        assert!(en_to_ru('1').is_none());
        assert!(en_to_ru('!').is_none());
        assert!(en_to_ru(' ').is_none());
    }

    // ── vk_to_en ─────────────────────────────────────────────────────────────

    #[test]
    fn vk_to_en_letter_keys() {
        // VK_A = 0x41 → 'a'
        assert_eq!(vk_to_en(0x41, false), Some('a'));
        // VK_Z = 0x5A → 'z'
        assert_eq!(vk_to_en(0x5A, false), Some('z'));
        // VK_N = 0x4E → 'n'
        assert_eq!(vk_to_en(0x4E, false), Some('n'));
    }

    #[test]
    fn vk_to_en_uppercase() {
        assert_eq!(vk_to_en(0x4E, true), Some('N'));
        assert_eq!(vk_to_en(0x41, true), Some('A'));
    }

    #[test]
    fn vk_to_en_punctuation() {
        assert_eq!(vk_to_en(0xBA, false), Some(';'));
        assert_eq!(vk_to_en(0xBC, false), Some(','));
        assert_eq!(vk_to_en(0xBE, false), Some('.'));
        assert_eq!(vk_to_en(0xDB, false), Some('['));
        assert_eq!(vk_to_en(0xDD, false), Some(']'));
    }

    #[test]
    fn vk_to_en_returns_none_for_digits_and_specials() {
        assert!(vk_to_en(0x30, false).is_none()); // VK_0
        assert!(vk_to_en(0x20, false).is_none()); // VK_SPACE
        assert!(vk_to_en(0x0D, false).is_none()); // VK_RETURN
    }

    // ── vk_to_ru ─────────────────────────────────────────────────────────────

    #[test]
    fn vk_to_ru_letter_keys() {
        // VK_N (0x4E) → 'т' in Russian layout
        assert_eq!(vk_to_ru(0x4E, false), Some('т'));
        // VK_F (0x46) → 'а'
        assert_eq!(vk_to_ru(0x46, false), Some('а'));
        // VK_Q (0x51) → 'й'
        assert_eq!(vk_to_ru(0x51, false), Some('й'));
    }

    #[test]
    fn vk_to_ru_uppercase() {
        assert_eq!(vk_to_ru(0x4E, true), Some('Т'));
    }

    #[test]
    fn vk_to_en_and_vk_to_ru_agree_with_map() {
        // For every (en_char, ru_char) in MAP, the corresponding VK code must
        // map correctly through both functions.
        for &(en, ru) in MAP {
            if !en.is_ascii_alphabetic() {
                continue; // punctuation VKs tested separately
            }
            let vk = en.to_ascii_uppercase() as u16;
            assert_eq!(
                vk_to_en(vk, false),
                Some(en),
                "vk_to_en({:#04x}) should be {:?}",
                vk,
                en
            );
            assert_eq!(
                vk_to_ru(vk, false),
                Some(ru),
                "vk_to_ru({:#04x}) should be {:?}",
                vk,
                ru
            );
        }
    }

    // ── is_translatable_vk ────────────────────────────────────────────────────

    #[test]
    fn translatable_vk_accepts_letters() {
        assert!(is_translatable_vk(0x41)); // A
        assert!(is_translatable_vk(0x5A)); // Z
        assert!(is_translatable_vk(0x4E)); // N
    }

    #[test]
    fn translatable_vk_accepts_mapped_punctuation() {
        assert!(is_translatable_vk(0xBA)); // ;
        assert!(is_translatable_vk(0xBC)); // ,
        assert!(is_translatable_vk(0xBE)); // .
        assert!(is_translatable_vk(0xDB)); // [
        assert!(is_translatable_vk(0xDD)); // ]
        assert!(is_translatable_vk(0xDE)); // '
    }

    #[test]
    fn translatable_vk_rejects_specials() {
        assert!(!is_translatable_vk(0x20)); // VK_SPACE
        assert!(!is_translatable_vk(0x0D)); // VK_RETURN
        assert!(!is_translatable_vk(0x08)); // VK_BACK
        assert!(!is_translatable_vk(0x30)); // '0'
        assert!(!is_translatable_vk(0x31)); // '1'
    }

    // ── HKL language detection ────────────────────────────────────────────────

    #[test]
    fn hkl_english_variants() {
        assert!(hkl_is_english(0x0409)); // en-US
        assert!(hkl_is_english(0x0809)); // en-GB
        assert!(hkl_is_english(0x0C09)); // en-AU
        assert!(!hkl_is_english(0x0419)); // ru
        assert!(!hkl_is_english(0x0000));
    }

    #[test]
    fn hkl_russian() {
        assert!(hkl_is_russian(0x0419));  // ru-RU
        assert!(!hkl_is_russian(0x0409)); // en-US
    }
}
