use std::collections::HashMap;
use std::sync::Arc;
use crate::buffer::WordBuffer;
use crate::layout;
use crate::settings::Settings;
use crate::globals::SETTINGS;

fn read_corpus_file(filename: &str) -> std::io::Result<String> {
    if let Ok(content) = std::fs::read_to_string(format!("corpus/{}", filename)) {
        Ok(content)
    } else {
        std::fs::read_to_string(format!("../corpus/{}", filename))
    }
}

fn extract_words(content: &str, limit: usize) -> Vec<String> {
    let mut words = Vec::new();
    for line in content.lines() {
        for raw_word in line.split_whitespace() {
            let cleaned: String = raw_word
                .chars()
                .filter(|c| c.is_alphabetic())
                .collect();
            // We only benchmark words of length >= 2 (single char words have simple manual checks)
            if cleaned.chars().count() >= 2 {
                words.push(cleaned);
                if words.len() >= limit {
                    return words;
                }
            }
        }
    }
    words
}

pub fn run() {
    println!("============================================================");
    println!("             RSwitcher In-Memory Algorithm Benchmark");
    println!("============================================================");

    // 1. Initialise Settings
    let settings = Arc::new(std::sync::RwLock::new(Settings::default()));
    SETTINGS.set(settings).expect("failed to set SETTINGS");

    // 2. Build character-to-VK maps dynamically
    let mut en_map: HashMap<char, (u16, bool)> = HashMap::new();
    let mut ru_map: HashMap<char, (u16, bool)> = HashMap::new();
    let mut ua_map: HashMap<char, (u16, bool)> = HashMap::new();

    for vk in 0..=0xFF {
        if layout::is_translatable_vk(vk) {
            for is_upper in &[false, true] {
                if let Some(ch) = layout::vk_to_en(vk, *is_upper) {
                    en_map.insert(ch, (vk, *is_upper));
                }
                if let Some(ch) = layout::vk_to_ru(vk, *is_upper) {
                    ru_map.insert(ch, (vk, *is_upper));
                }
                if let Some(ch) = layout::vk_to_ua(vk, *is_upper) {
                    ua_map.insert(ch, (vk, *is_upper));
                }
            }
        }
    }

    println!("Layout maps initialised:");
    println!("  English keys:    {}", en_map.len());
    println!("  Russian keys:    {}", ru_map.len());
    println!("  Ukrainian keys:  {}", ua_map.len());

    // 3. Load corpora
    println!("\nLoading corpus files...");
    let limit = 5000; // 5k words per language is plenty for statistical significance

    let en_words = match read_corpus_file("en.txt") {
        Ok(c) => extract_words(&c, limit),
        Err(e) => {
            println!("  [ERROR] Failed to read en.txt: {}", e);
            return;
        }
    };
    let ru_words = match read_corpus_file("ru.txt") {
        Ok(c) => extract_words(&c, limit),
        Err(e) => {
            println!("  [ERROR] Failed to read ru.txt: {}", e);
            return;
        }
    };
    let ua_words = match read_corpus_file("ua.txt") {
        Ok(c) => extract_words(&c, limit),
        Err(e) => {
            println!("  [ERROR] Failed to read ua.txt: {}", e);
            return;
        }
    };

    println!("Corpus words loaded:");
    println!("  English:    {}", en_words.len());
    println!("  Russian:    {}", ru_words.len());
    println!("  Ukrainian:  {}", ua_words.len());

    // 4. Test English (Correct Layout - expect 0% switches)
    println!("\nRunning English layout test (typing EN in EN layout)...");
    let mut en_total = 0;
    let mut en_false_positives = 0;
    let mut en_to_ru = 0;
    let mut en_to_ua = 0;

    for word in &en_words {
        let mut buf = WordBuffer::new();
        let mut chars_mapped = Vec::new();
        let mut has_unmapped = false;

        for ch in word.chars() {
            if let Some(&entry) = en_map.get(&ch) {
                chars_mapped.push(entry);
            } else {
                has_unmapped = true;
                break;
            }
        }
        if has_unmapped {
            continue;
        }

        en_total += 1;
        let mut switched_action = None;

        // Simulate typing character by character
        for (vk, is_upper) in chars_mapped {
            buf.push(vk, is_upper);
            if let Some(action) = buf.detect_mismatch_on_the_fly(layout::LANG_EN_US, 1.0) {
                switched_action = Some(action);
                break;
            }
        }

        // If not switched on-the-fly, test word boundary (Space)
        if switched_action.is_none() {
            if let Some(action) = buf.detect_mismatch_with_sensitivity(layout::LANG_EN_US, 1.0) {
                switched_action = Some(action);
            }
        }

        if let Some(action) = switched_action {
            en_false_positives += 1;
            if action.target_lang == layout::LANG_RU {
                en_to_ru += 1;
            } else if action.target_lang == layout::LANG_UA {
                en_to_ua += 1;
            }
        }
    }

    // 5. Test Russian (Mistyped Layout - expect switch to RU)
    println!("Running Russian layout test (typing RU in EN layout)...");
    let mut ru_total = 0;
    let mut ru_correct_switches = 0;   // Switched to RU
    let mut ru_incorrect_switches = 0; // Switched to UA
    let mut ru_missed = 0;             // Did not switch

    for word in &ru_words {
        let mut buf = WordBuffer::new();
        let mut chars_mapped = Vec::new();
        let mut has_unmapped = false;

        for ch in word.chars() {
            if let Some(&entry) = ru_map.get(&ch) {
                chars_mapped.push(entry);
            } else {
                has_unmapped = true;
                break;
            }
        }
        if has_unmapped {
            continue;
        }

        ru_total += 1;
        let mut switched_action = None;

        for (vk, is_upper) in chars_mapped {
            buf.push(vk, is_upper);
            if let Some(action) = buf.detect_mismatch_on_the_fly(layout::LANG_EN_US, 1.0) {
                switched_action = Some(action);
                break;
            }
        }

        if switched_action.is_none() {
            if let Some(action) = buf.detect_mismatch_with_sensitivity(layout::LANG_EN_US, 1.0) {
                switched_action = Some(action);
            }
        }

        match switched_action {
            Some(action) => {
                if action.target_lang == layout::LANG_RU {
                    ru_correct_switches += 1;
                } else {
                    ru_incorrect_switches += 1; // Switched to UA instead
                }
            }
            None => {
                ru_missed += 1;
            }
        }
    }

    // 6. Test Ukrainian (Mistyped Layout - expect switch to UA)
    println!("Running Ukrainian layout test (typing UA in EN layout)...");
    let mut ua_total = 0;
    let mut ua_correct_switches = 0;   // Switched to UA
    let mut ua_incorrect_switches = 0; // Switched to RU
    let mut ua_missed = 0;             // Did not switch

    for word in &ua_words {
        let mut buf = WordBuffer::new();
        let mut chars_mapped = Vec::new();
        let mut has_unmapped = false;

        for ch in word.chars() {
            if let Some(&entry) = ua_map.get(&ch) {
                chars_mapped.push(entry);
            } else {
                has_unmapped = true;
                break;
            }
        }
        if has_unmapped {
            continue;
        }

        ua_total += 1;
        let mut switched_action = None;

        for (vk, is_upper) in chars_mapped {
            buf.push(vk, is_upper);
            if let Some(action) = buf.detect_mismatch_on_the_fly(layout::LANG_EN_US, 1.0) {
                switched_action = Some(action);
                break;
            }
        }

        if switched_action.is_none() {
            if let Some(action) = buf.detect_mismatch_with_sensitivity(layout::LANG_EN_US, 1.0) {
                switched_action = Some(action);
            }
        }

        match switched_action {
            Some(action) => {
                if action.target_lang == layout::LANG_UA {
                    ua_correct_switches += 1;
                } else {
                    ua_incorrect_switches += 1; // Switched to RU instead
                }
            }
            None => {
                ua_missed += 1;
            }
        }
    }

    // 7. Render Report
    println!("\n============================================================");
    println!("                    BENCHMARK RESULTS");
    println!("============================================================");
    
    println!("1. ENGLISH (Correct layout, target: 0% switches)");
    println!("   Total tested:     {}", en_total);
    println!("   False Switches:   {} ({:.2}%)", en_false_positives, (en_false_positives as f64 / en_total as f64) * 100.0);
    println!("     - Switched to RU: {} ({:.2}%)", en_to_ru, (en_to_ru as f64 / en_total as f64) * 100.0);
    println!("     - Switched to UA: {} ({:.2}%)", en_to_ua, (en_to_ua as f64 / en_total as f64) * 100.0);

    println!("\n2. RUSSIAN (Mistyped layout, target: 100% RU switches)");
    println!("   Total tested:     {}", ru_total);
    println!("   Correct Switches: {} ({:.2}%)", ru_correct_switches, (ru_correct_switches as f64 / ru_total as f64) * 100.0);
    println!("   Wrong switches:   {} ({:.2}%) [switched to UA]", ru_incorrect_switches, (ru_incorrect_switches as f64 / ru_total as f64) * 100.0);
    println!("   Missed switches:  {} ({:.2}%)", ru_missed, (ru_missed as f64 / ru_total as f64) * 100.0);

    println!("\n3. UKRAINIAN (Mistyped layout, target: 100% UA switches)");
    println!("   Total tested:     {}", ua_total);
    println!("   Correct Switches: {} ({:.2}%)", ua_correct_switches, (ua_correct_switches as f64 / ua_total as f64) * 100.0);
    println!("   Wrong switches:   {} ({:.2}%) [switched to RU]", ua_incorrect_switches, (ua_incorrect_switches as f64 / ua_total as f64) * 100.0);
    println!("   Missed switches:  {} ({:.2}%)", ua_missed, (ua_missed as f64 / ua_total as f64) * 100.0);
    println!("============================================================");
}
