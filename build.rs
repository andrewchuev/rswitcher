fn main() {
    generate_bigrams();

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let ico_path = generate_icon(&out_dir);

    winresource::WindowsResource::new()
        .set_manifest(MANIFEST)
        .set_icon(&ico_path)
        .compile()
        .expect("failed to compile Windows resources");
}

// ── Application icon generator ────────────────────────────────────────────────
//
// Generates an .ico file with 16×16, 32×32, and 48×48 variants.
// Uses the same design as the runtime tray icon: dark-blue rounded rectangle
// with "Rs" text in light-gray (R from GLYPH_R, s from GLYPH_S).

fn generate_icon(out_dir: &str) -> String {
    let ico_path = format!("{}/rswitcher.ico", out_dir);
    let images: Vec<(usize, Vec<u8>)> = [16usize, 32, 48]
        .iter()
        .map(|&s| (s, make_icon_rgba(s)))
        .collect();
    write_ico_file(&ico_path, &images);
    ico_path
}

// 5-wide × 7-tall glyph bitmaps (same as in main.rs).
const GLYPH_R: [u8; 7] = [0b11100, 0b10010, 0b11100, 0b10100, 0b10010, 0b00000, 0b00000];
const GLYPH_S: [u8; 7] = [0b01110, 0b10000, 0b01110, 0b00001, 0b01110, 0b00000, 0b00000];

fn make_icon_rgba(size: usize) -> Vec<u8> {
    let scale   = (size / 16).max(1);
    let glyph_w = 5 * scale;
    let glyph_h = 7 * scale;
    let gap     = scale;
    let text_w  = glyph_w * 2 + gap;
    let off_x   = size.saturating_sub(text_w) / 2;
    let off_y   = size.saturating_sub(glyph_h) / 2;
    let radius  = (size as f32 * 0.15_f32).max(1.0_f32);

    let bg: [u8; 3] = [0x1a, 0x2e, 0x6c];
    let fg: [u8; 3] = [0xd0, 0xd8, 0xe0];

    let half_w = glyph_w;
    let glyph_pixel = |gx: usize, gy: usize| -> bool {
        if gy >= glyph_h { return false; }
        let row = gy / scale;
        let (glyph, col_pixel) = if gx < half_w {
            (&GLYPH_R, gx)
        } else if gx < half_w + gap {
            return false;
        } else {
            (&GLYPH_S, gx - half_w - gap)
        };
        if row >= 7 { return false; }
        let col = col_pixel / scale;
        if col >= 5 { return false; }
        (glyph[row] >> (4 - col)) & 1 == 1
    };

    let mut pixels = vec![0u8; size * size * 4];
    let s = size as f32;
    for py in 0..size {
        for px in 0..size {
            let idx = (py * size + px) * 4;
            let fx = px as f32 + 0.5;
            let fy = py as f32 + 0.5;
            let qx = (fx - s * 0.5).abs() - (s * 0.5 - radius);
            let qy = (fy - s * 0.5).abs() - (s * 0.5 - radius);
            let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
            let alpha = (1.0_f32 - dist.clamp(-1.0, 1.0) * 0.5 - 0.5).clamp(0.0, 1.0);
            if alpha <= 0.0 { continue; }
            let is_text = px >= off_x
                && py >= off_y
                && px < off_x + text_w
                && py < off_y + glyph_h
                && glyph_pixel(px - off_x, py - off_y);
            let c = if is_text { fg } else { bg };
            pixels[idx]     = c[0];
            pixels[idx + 1] = c[1];
            pixels[idx + 2] = c[2];
            pixels[idx + 3] = (alpha * 255.0) as u8;
        }
    }
    pixels
}

/// Write a multi-size .ico file.  Each image is stored as a 32bpp DIB
/// (BITMAPINFOHEADER + BGRA pixels, bottom-to-top) with an all-zero AND mask.
fn write_ico_file(path: &str, images: &[(usize, Vec<u8>)]) {
    let n = images.len() as u16;
    let header_size: u32 = 6 + n as u32 * 16;

    let dibs: Vec<Vec<u8>> = images.iter()
        .map(|(size, rgba)| encode_dib(rgba, *size))
        .collect();

    let mut offsets: Vec<u32> = Vec::new();
    let mut offset = header_size;
    for dib in &dibs {
        offsets.push(offset);
        offset += dib.len() as u32;
    }

    let mut data: Vec<u8> = Vec::new();

    // ICONDIR
    data.extend_from_slice(&[0u8, 0]);
    data.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    data.extend_from_slice(&n.to_le_bytes());

    // ICONDIRENTRY per image
    for (i, (size, _)) in images.iter().enumerate() {
        let w = if *size >= 256 { 0u8 } else { *size as u8 };
        data.push(w);
        data.push(w);
        data.push(0); // color count
        data.push(0); // reserved
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&(dibs[i].len() as u32).to_le_bytes());
        data.extend_from_slice(&offsets[i].to_le_bytes());
    }

    for dib in &dibs { data.extend_from_slice(dib); }

    std::fs::write(path, &data).expect("cannot write rswitcher.ico");
}

fn encode_dib(rgba: &[u8], size: usize) -> Vec<u8> {
    // Convert RGBA → BGRA and flip rows (DIB = bottom-to-top).
    let stride = size * 4;
    let mut bgra = vec![0u8; rgba.len()];
    for row in 0..size {
        let src = (size - 1 - row) * stride;
        let dst = row * stride;
        for col in 0..size {
            bgra[dst + col * 4]     = rgba[src + col * 4 + 2]; // B
            bgra[dst + col * 4 + 1] = rgba[src + col * 4 + 1]; // G
            bgra[dst + col * 4 + 2] = rgba[src + col * 4];     // R
            bgra[dst + col * 4 + 3] = rgba[src + col * 4 + 3]; // A
        }
    }

    let mask_row_bytes = size.div_ceil(32) * 4;
    let mask_size = mask_row_bytes * size;

    let mut dib: Vec<u8> = Vec::new();
    // BITMAPINFOHEADER
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(size as i32).to_le_bytes());
    dib.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // height doubled for AND mask
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&[0u8; 24]); // biCompression..biClrImportant all zero

    dib.extend_from_slice(&bgra);                         // XOR mask (pixels)
    dib.extend(std::iter::repeat_n(0u8, mask_size));   // AND mask (all visible)
    dib
}

// ── Bigram frequency table generator ─────────────────────────────────────────
//
// Reads corpus/ru.txt and corpus/en.txt, counts adjacent letter pairs, applies
// Laplace (add-1) smoothing, and emits normalised f32 probability tables to
// $OUT_DIR/bigrams_gen.rs.  Included by src/bigrams.rs via include!().

fn generate_bigrams() {
    use std::io::Write as _;

    println!("cargo:rerun-if-changed=corpus/ru.txt");
    println!("cargo:rerun-if-changed=corpus/en.txt");

    let ru_text = std::fs::read_to_string("corpus/ru.txt").unwrap_or_default();
    let en_text = std::fs::read_to_string("corpus/en.txt").unwrap_or_default();

    // Russian: а-я = U+0430..U+044F, 32 letters.  ё → е normalisation applied.
    let ru = build_table(&ru_text, 'а', 32, |c| if c == 'ё' { 'е' } else { c });
    // English: a-z, 26 letters.
    let en = build_table(&en_text, 'a', 26, |c| c);

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = std::path::Path::new(&out_dir).join("bigrams_gen.rs");
    let mut f = std::fs::File::create(&out_path).expect("cannot create bigrams_gen.rs");

    writeln!(f, "// Auto-generated by build.rs — do not edit manually.").unwrap();
    writeln!(f, "#[allow(clippy::excessive_precision)]").unwrap();
    writeln!(f, "pub static EN_BIGRAMS: [f32; {}] = {};", en.len(), fmt_f32_array(&en)).unwrap();
    writeln!(f, "#[allow(clippy::excessive_precision)]").unwrap();
    writeln!(f, "pub static RU_BIGRAMS: [f32; {}] = {};", ru.len(), fmt_f32_array(&ru)).unwrap();
}

fn build_table(text: &str, base: char, n: usize, norm: impl Fn(char) -> char) -> Vec<f32> {
    let base_u = base as u32;
    let indices: Vec<usize> = text
        .chars()
        .filter_map(|c| {
            let lc = norm(c.to_lowercase().next()?);
            let d = (lc as u32).checked_sub(base_u)? as usize;
            if d < n { Some(d) } else { None }
        })
        .collect();

    let mut counts = vec![0u32; n * n];
    for w in indices.windows(2) {
        counts[w[0] * n + w[1]] += 1;
    }

    // Laplace (add-1) smoothing — every bigram gets probability > 0.
    let total: u64 = counts.iter().map(|&c| c as u64).sum::<u64>() + (n * n) as u64;
    counts.iter().map(|&c| (c + 1) as f32 / total as f32).collect()
}

fn fmt_f32_array(v: &[f32]) -> String {
    let entries: Vec<String> = v.iter().map(|&x| format!("{:.10}f32", x)).collect();
    format!("[{}]", entries.join(", "))
}

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">

  <!-- Tell Windows which OS versions this app explicitly supports.
       Required for correct DPI scaling, theming, and API behaviour on Win10/11. -->
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/> <!-- Windows 10 / 11 -->
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/> <!-- Windows 8.1 -->
    </application>
  </compatibility>

  <!-- Request Common Controls v6 (comctl32.dll version 6).
       Without this Windows silently uses v5, which lacks TaskDialogIndirect
       and the visual styles required by tray-icon and eframe. -->
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"/>
    </dependentAssembly>
  </dependency>

  <!-- Per-monitor DPI awareness v2: crisp UI on high-DPI / mixed-DPI setups. -->
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">
        PerMonitorV2
      </dpiAwareness>
    </windowsSettings>
  </application>

</assembly>"#;
