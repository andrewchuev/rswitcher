use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
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

struct TimingStats {
    samples: Vec<Duration>,
}

impl TimingStats {
    fn new() -> Self { Self { samples: Vec::new() } }
    fn record(&mut self, d: Duration) { self.samples.push(d); }
    fn count(&self) -> usize { self.samples.len() }
    fn total(&self) -> Duration { self.samples.iter().sum() }
    fn mean_ns(&self) -> f64 {
        if self.samples.is_empty() { return 0.0; }
        self.total().as_nanos() as f64 / self.samples.len() as f64
    }
    fn median_ns(&self) -> f64 {
        if self.samples.is_empty() { return 0.0; }
        let mut sorted = self.samples.clone();
        sorted.sort();
        let mid = sorted.len() / 2;
        sorted[mid].as_nanos() as f64
    }
    fn p99_ns(&self) -> f64 {
        if self.samples.is_empty() { return 0.0; }
        let mut sorted = self.samples.clone();
        sorted.sort();
        let idx = (sorted.len() as f64 * 0.99) as usize;
        sorted[idx.min(sorted.len() - 1)].as_nanos() as f64
    }
    fn max_ns(&self) -> f64 {
        self.samples.iter().map(|d| d.as_nanos()).max().unwrap_or(0) as f64
    }
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
    let limit = 5000;

    let en_words = match read_corpus_file("en.txt") {
        Ok(c) => extract_words(&c, limit),
        Err(e) => { println!("  [ERROR] Failed to read en.txt: {}", e); return; }
    };
    let ru_words = match read_corpus_file("ru.txt") {
        Ok(c) => extract_words(&c, limit),
        Err(e) => { println!("  [ERROR] Failed to read ru.txt: {}", e); return; }
    };
    let ua_words = match read_corpus_file("ua.txt") {
        Ok(c) => extract_words(&c, limit),
        Err(e) => { println!("  [ERROR] Failed to read ua.txt: {}", e); return; }
    };

    println!("Corpus words loaded:");
    println!("  English:    {}", en_words.len());
    println!("  Russian:    {}", ru_words.len());
    println!("  Ukrainian:  {}", ua_words.len());

    // ── Static data size estimates ────────────────────────────────────────────
    // These are compile-time constants baked into the binary.
    let en_bi  = 26 * 26;
    let ru_bi  = 32 * 32;
    let ua_bi  = 33 * 33;
    let en_tri = 26 * 26 * 26;
    let ru_tri = 32 * 32 * 32;
    let ua_tri = 33 * 33 * 33;
    let model_bytes = (en_bi + ru_bi + ua_bi + en_tri + ru_tri + ua_tri) * 4;
    println!("\nStatic model size (baked into binary): {:.1} KB", model_bytes as f64 / 1024.0);

    // ── Helper to simulate typing a word and record detection latency ─────────
    // `active_lang` = the layout currently active in the OS.
    // `expected_target` = the layout we expect the algorithm to switch to (None = expect no switch).
    let simulate = |words: &[String],
                    map: &HashMap<char, (u16, bool)>,
                    active_lang: u16,
                    expected_target: Option<u16>|
        -> (usize, usize, usize, usize, TimingStats, TimingStats)
    {
        let mut total = 0usize;
        let mut correct = 0usize;
        let mut wrong = 0usize;
        let mut missed = 0usize;
        let mut t_boundary = TimingStats::new();
        let mut t_otf = TimingStats::new();

        for word in words {
            let mut chars_mapped = Vec::new();
            let mut has_unmapped = false;
            for ch in word.chars() {
                if let Some(&entry) = map.get(&ch) {
                    chars_mapped.push(entry);
                } else {
                    has_unmapped = true;
                    break;
                }
            }
            if has_unmapped { continue; }

            total += 1;
            let mut buf = WordBuffer::new();
            let mut switched = None;

            for (vk, is_upper) in &chars_mapped {
                buf.push(*vk, *is_upper);
                let t0 = Instant::now();
                let result = buf.detect_mismatch_on_the_fly(active_lang, 1.0);
                t_otf.record(t0.elapsed());
                if result.is_some() {
                    switched = result;
                    break;
                }
            }

            if switched.is_none() {
                let t0 = Instant::now();
                switched = buf.detect_mismatch_with_sensitivity(active_lang, 1.0);
                t_boundary.record(t0.elapsed());
            }

            match (switched, expected_target) {
                (None, None) => correct += 1,
                (None, Some(_)) => missed += 1,
                (Some(action), Some(target)) if action.target_lang == target => correct += 1,
                (Some(_), _) => wrong += 1,
            }
        }

        (total, correct, wrong, missed, t_boundary, t_otf)
    };

    // 4. English in EN layout (correct) → expect no switch
    println!("\nRunning English layout test (typing EN in EN layout)...");
    let (en_total, en_ok, en_wrong, _en_missed, en_tb, en_otf) =
        simulate(&en_words, &en_map, layout::LANG_EN_US, None);
    let en_false_pos = en_total - en_ok;

    // 5. Russian typed in EN layout → expect switch to RU
    println!("Running Russian layout test (typing RU in EN layout)...");
    let (ru_total, ru_correct, ru_wrong, ru_missed, ru_tb, ru_otf) =
        simulate(&ru_words, &ru_map, layout::LANG_EN_US, Some(layout::LANG_RU));

    // 6. Ukrainian typed in EN layout → expect switch to UA
    println!("Running Ukrainian layout test (typing UA in EN layout)...");
    let (ua_total, ua_correct, ua_wrong, ua_missed, ua_tb, ua_otf) =
        simulate(&ua_words, &ua_map, layout::LANG_EN_US, Some(layout::LANG_UA));

    // 7. Results
    println!("\n============================================================");
    println!("                    BENCHMARK RESULTS");
    println!("============================================================");

    println!("1. ENGLISH (Correct layout, target: 0% switches)");
    println!("   Total tested:     {}", en_total);
    println!("   False switches:   {} ({:.2}%)", en_false_pos, en_false_pos as f64 / en_total as f64 * 100.0);
    println!("   Correct (no-op):  {} ({:.2}%)", en_ok, en_ok as f64 / en_total as f64 * 100.0);

    println!("\n2. RUSSIAN (Mistyped layout, target: 100% RU switches)");
    println!("   Total tested:     {}", ru_total);
    println!("   Correct switches: {} ({:.2}%)", ru_correct, ru_correct as f64 / ru_total as f64 * 100.0);
    println!("   Wrong switches:   {} ({:.2}%) [switched to UA]", ru_wrong, ru_wrong as f64 / ru_total as f64 * 100.0);
    println!("   Missed switches:  {} ({:.2}%)", ru_missed, ru_missed as f64 / ru_total as f64 * 100.0);

    println!("\n3. UKRAINIAN (Mistyped layout, target: 100% UA switches)");
    println!("   Total tested:     {}", ua_total);
    println!("   Correct switches: {} ({:.2}%)", ua_correct, ua_correct as f64 / ua_total as f64 * 100.0);
    println!("   Wrong switches:   {} ({:.2}%) [switched to RU]", ua_wrong, ua_wrong as f64 / ua_total as f64 * 100.0);
    println!("   Missed switches:  {} ({:.2}%)", ua_missed, ua_missed as f64 / ua_total as f64 * 100.0);

    println!("\n============================================================");
    println!("                    LATENCY PROFILE");
    println!("============================================================");
    println!("detect_mismatch_with_sensitivity() — called at word boundary (Space/Enter):");
    let tb_all: Vec<Duration> = [en_tb.samples.as_slice(), ru_tb.samples.as_slice(), ua_tb.samples.as_slice()].concat();
    let mut tb_merged = TimingStats { samples: tb_all };
    println!("  Samples:  {}", tb_merged.count());
    println!("  Mean:     {:.0} ns  ({:.3} µs)", tb_merged.mean_ns(), tb_merged.mean_ns() / 1000.0);
    println!("  Median:   {:.0} ns  ({:.3} µs)", tb_merged.median_ns(), tb_merged.median_ns() / 1000.0);
    println!("  p99:      {:.0} ns  ({:.3} µs)", tb_merged.p99_ns(), tb_merged.p99_ns() / 1000.0);
    println!("  Max:      {:.0} ns  ({:.3} µs)", tb_merged.max_ns(), tb_merged.max_ns() / 1000.0);

    println!("\ndetect_mismatch_on_the_fly() — called on every keystroke (≥5 chars):");
    let otf_all: Vec<Duration> = [en_otf.samples.as_slice(), ru_otf.samples.as_slice(), ua_otf.samples.as_slice()].concat();
    let mut otf_merged = TimingStats { samples: otf_all };
    println!("  Samples:  {}", otf_merged.count());
    println!("  Mean:     {:.0} ns  ({:.3} µs)", otf_merged.mean_ns(), otf_merged.mean_ns() / 1000.0);
    println!("  Median:   {:.0} ns  ({:.3} µs)", otf_merged.median_ns(), otf_merged.median_ns() / 1000.0);
    println!("  p99:      {:.0} ns  ({:.3} µs)", otf_merged.p99_ns(), otf_merged.p99_ns() / 1000.0);
    println!("  Max:      {:.0} ns  ({:.3} µs)", otf_merged.max_ns(), otf_merged.max_ns() / 1000.0);

    let total_words = en_total + ru_total + ua_total;
    let total_time = tb_merged.total() + otf_merged.total();
    println!("\nTotal words processed: {}", total_words);
    println!("Total detection time:  {:.2} ms", total_time.as_secs_f64() * 1000.0);
    println!("Throughput:            {:.0} words/sec (single-threaded)", total_words as f64 / total_time.as_secs_f64());
    println!("============================================================");
}
