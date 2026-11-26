//! Tray icon: the duck headphones artwork (assets/duck-headphones.png), scaled to
//! the tray's native size, with the battery percentage in pixel digits on top.

use std::sync::OnceLock;

static DUCK_PNG: &[u8] = include_bytes!("../assets/duck-headphones.png");
/// Purple variant for the macOS menu bar (checkerboard removed by examples/clean_icon.rs).
static DUCK_MAC_PNG: &[u8] = include_bytes!("../assets/duck-headphones-mac.png");

/// Dark brown used for the artwork's outlines; also outlines the digits.
const OUTLINE: (u8, u8, u8) = (0x2b, 0x17, 0x0e);

/// Cropped artwork, premultiplied RGBA in 0..1.
struct Art {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>,
}

static ART: OnceLock<Option<Art>> = OnceLock::new();
static MAC_ART: OnceLock<Option<Art>> = OnceLock::new();

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_art() -> Option<&'static Art> {
    MAC_ART.get_or_init(|| load_art(DUCK_MAC_PNG)).as_ref()
}

fn art() -> Option<&'static Art> {
    ART.get_or_init(|| load_art(DUCK_PNG)).as_ref()
}

fn load_art(bytes: &[u8]) -> Option<Art> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return None,
    };
    let at = |x: usize, y: usize| -> [f32; 4] {
        let i = (y * w + x) * ch;
        let a = if ch == 4 { buf[i + 3] as f32 / 255.0 } else { 1.0 };
        [buf[i] as f32 / 255.0 * a, buf[i + 1] as f32 / 255.0 * a, buf[i + 2] as f32 / 255.0 * a, a]
    };
    // Crop to the opaque content.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if at(x, y)[3] > 0.06 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (cw, chh) = (x1 - x0, y1 - y0);
    let mut px = Vec::with_capacity(cw * chh);
    for y in y0..y1 {
        for x in x0..x1 {
            px.push(at(x, y));
        }
    }
    Some(Art { w: cw, h: chh, px })
}

/// Box-filters the artwork down to `tw`×`th`.
fn downscale(a: &Art, tw: usize, th: usize) -> Vec<[f32; 4]> {
    let mut out = vec![[0f32; 4]; tw * th];
    for ty in 0..th {
        let sy0 = ty * a.h / th;
        let sy1 = ((ty + 1) * a.h / th).max(sy0 + 1);
        for tx in 0..tw {
            let sx0 = tx * a.w / tw;
            let sx1 = ((tx + 1) * a.w / tw).max(sx0 + 1);
            let mut acc = [0f32; 4];
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let p = a.px[sy * a.w + sx];
                    for k in 0..4 {
                        acc[k] += p[k];
                    }
                }
            }
            let cnt = ((sy1 - sy0) * (sx1 - sx0)) as f32;
            out[ty * tw + tx] = acc.map(|v| v / cnt);
        }
    }
    out
}

// ---- pixel digits -------------------------------------------------------------

/// Bold 5×7 digits with 2 px strokes (bit 4 = leftmost column);
/// index 10 = '?', 11 = '-'.
const FONT: [[u8; 7]; 12] = [
    [0b01110, 0b11011, 0b11011, 0b11011, 0b11011, 0b11011, 0b01110], // 0
    [0b00110, 0b01110, 0b00110, 0b00110, 0b00110, 0b00110, 0b01111], // 1
    [0b01110, 0b11011, 0b00011, 0b00110, 0b01100, 0b11000, 0b11111], // 2
    [0b11110, 0b00011, 0b00011, 0b01110, 0b00011, 0b00011, 0b11110], // 3
    [0b00011, 0b00111, 0b01011, 0b11011, 0b11111, 0b00011, 0b00011], // 4
    [0b11111, 0b11000, 0b11110, 0b00011, 0b00011, 0b11011, 0b01110], // 5
    [0b01110, 0b11000, 0b11000, 0b11110, 0b11011, 0b11011, 0b01110], // 6
    [0b11111, 0b00011, 0b00011, 0b00110, 0b00110, 0b01100, 0b01100], // 7
    [0b01110, 0b11011, 0b11011, 0b01110, 0b11011, 0b11011, 0b01110], // 8
    [0b01110, 0b11011, 0b11011, 0b01111, 0b00011, 0b00011, 0b01110], // 9
    [0b01110, 0b11011, 0b00011, 0b00110, 0b01100, 0b00000, 0b01100], // ?
    [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000], // -
];
/// Narrow bold "1" (2 columns) used in "100" so three digits fit in 16 px.
const ONE_NARROW: [u8; 7] = [0b11, 0b11, 0b11, 0b11, 0b11, 0b11, 0b11];
/// Bold 5×7 percent sign.
const PERCENT: [u8; 7] = [0b11001, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b10011];

/// (rows, width) for each character of `text`.
fn glyphs(text: &str) -> Vec<(&'static [u8; 7], usize)> {
    let digits = text.bytes().filter(u8::is_ascii_digit).count();
    let narrow_one = digits >= 3 && !text.contains('%');
    text.bytes()
        .map(|b| match b {
            b'1' if narrow_one => (&ONE_NARROW, 2),
            b'0'..=b'9' => (&FONT[(b - b'0') as usize], 5),
            b'-' => (&FONT[11], 5),
            b'%' => (&PERCENT, 5),
            _ => (&FONT[10], 5),
        })
        .collect()
}

/// Width of `text` in font pixels (1 px gap between characters).
fn text_width(text: &str) -> usize {
    let g = glyphs(text);
    g.iter().map(|(_, w)| w).sum::<usize>() + g.len().saturating_sub(1)
}

/// Text mask on a `w`×`h` canvas, font pixels scaled by `k`, top-left at (x0, y0).
fn text_mask_at(w: usize, h: usize, text: &str, k: usize, x0: usize, y0: usize) -> Vec<bool> {
    let mut m = vec![false; w * h];
    let mut gx = x0;
    for (rows, gw) in glyphs(text) {
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..gw {
                if bits & (1 << (gw - 1 - col)) == 0 {
                    continue;
                }
                for yy in 0..k {
                    for xx in 0..k {
                        let (x, y) = (gx + col * k + xx, y0 + row * k + yy);
                        if x < w && y < h {
                            m[y * w + x] = true;
                        }
                    }
                }
            }
        }
        gx += (gw + 1) * k;
    }
    m
}

/// Text mask on an `n`×`n` grid, centred on (cx, cy).
fn text_mask(n: usize, text: &str, k: usize, cx: f32, cy: f32) -> Vec<bool> {
    let tw = text_width(text) * k;
    let th = 7 * k;
    let x0 = (cx - tw as f32 / 2.0).round().max(0.0) as usize;
    let y0 = (cy - th as f32 / 2.0).round().max(0.0) as usize;
    text_mask_at(n, n, text, k, x0, y0)
}

fn dilate_rect(w: usize, h: usize, m: &[bool], r: usize) -> Vec<bool> {
    let (r, wi, hi) = (r as i32, w as i32, h as i32);
    let mut out = vec![false; w * h];
    for y in 0..hi {
        for x in 0..wi {
            'o: for dy in -r..=r {
                for dx in -r..=r {
                    let (sx, sy) = (x + dx, y + dy);
                    if (0..wi).contains(&sx) && (0..hi).contains(&sy) && m[(sy * wi + sx) as usize] {
                        out[(y * wi + x) as usize] = true;
                        break 'o;
                    }
                }
            }
        }
    }
    out
}

fn dilate(n: usize, m: &[bool], r: usize) -> Vec<bool> {
    dilate_rect(n, n, m, r)
}

/// Straight RGBA `n`×`n` icon. `color` = digit colour, `alpha` < 1 dims the digits (blink).
pub fn render(n: usize, text: &str, color: (u8, u8, u8), alpha: f32) -> Option<Vec<u8>> {
    let a = art()?;
    // Stretch the artwork over the whole square: the tray slot is tiny, every
    // pixel counts more than the exact aspect ratio.
    let mut px = downscale(a, n, n);

    // Largest whole-pixel digit scale that fits (with the 1 px outline), but no
    // more than n/16 so the ducks on the ear cups stay visible at larger sizes.
    let tw = text_width(text);
    let fit = ((n - 2) / tw).min((n - 2) / 7).max(1);
    let k = fit.min((n / 16).max(1));
    let digits = text_mask(n, text, k, n as f32 / 2.0, n as f32 * 0.52);
    let outline = dilate(n, &digits, 1);

    let o = [OUTLINE.0 as f32 / 255.0, OUTLINE.1 as f32 / 255.0, OUTLINE.2 as f32 / 255.0];
    let c = [color.0 as f32 / 255.0, color.1 as f32 / 255.0, color.2 as f32 / 255.0];
    for i in 0..n * n {
        if outline[i] {
            px[i] = [o[0], o[1], o[2], 1.0];
        }
        if digits[i] {
            // digit over outline (premultiplied "over")
            let base = px[i];
            px[i] = [
                c[0] * alpha + base[0] * (1.0 - alpha),
                c[1] * alpha + base[1] * (1.0 - alpha),
                c[2] * alpha + base[2] * (1.0 - alpha),
                alpha + base[3] * (1.0 - alpha),
            ];
        }
    }

    let mut out = vec![0u8; n * n * 4];
    for (i, p) in px.iter().enumerate() {
        if p[3] <= 0.0 {
            continue;
        }
        for k in 0..3 {
            out[i * 4 + k] = ((p[k] / p[3]).min(1.0) * 255.0).round() as u8;
        }
        out[i * 4 + 3] = (p[3].min(1.0) * 255.0).round() as u8;
    }
    Some(out)
}

/// Artwork shown in the macOS menu bar.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy)]
pub enum MacArt {
    Headset,
    Mouse,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static MOUSE_MAC_PNG: &[u8] = include_bytes!("../assets/mouse-mac.png");
static MOUSE_ART: OnceLock<Option<Art>> = OnceLock::new();

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mouse_art() -> Option<&'static Art> {
    MOUSE_ART.get_or_init(|| load_art(MOUSE_MAC_PNG)).as_ref()
}

/// System UI font for the menu-bar text: San Francisco on macOS (as the clock
/// uses), Helvetica as fallback; Segoe UI elsewhere (used for previews/tests).
static UI_FONT: OnceLock<Option<ab_glyph::FontVec>> = OnceLock::new();

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn ui_font() -> Option<&'static ab_glyph::FontVec> {
    use ab_glyph::{Font, VariableFont};
    UI_FONT
        .get_or_init(|| {
            let win = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
            let candidates = [
                ("/System/Library/Fonts/SFNS.ttf".to_string(), 0),
                ("/System/Library/Fonts/SFNSText.ttf".to_string(), 0),
                ("/System/Library/Fonts/Helvetica.ttc".to_string(), 1), // index 1 = Bold
                ("/Library/Fonts/Arial.ttf".to_string(), 0),
                (format!(r"{win}\Fonts\segoeuisb.ttf"), 0),
                (format!(r"{win}\Fonts\seguisb.ttf"), 0),
                (format!(r"{win}\Fonts\segoeui.ttf"), 0),
            ];
            for (path, index) in candidates {
                let Ok(bytes) = std::fs::read(&path) else { continue };
                let font = ab_glyph::FontVec::try_from_vec_and_index(bytes.clone(), index)
                    .or_else(|_| ab_glyph::FontVec::try_from_vec(bytes));
                if let Ok(mut f) = font {
                    // SF is a variable font: use a semibold weight like menu-bar text.
                    let _ = f.set_variation(b"wght", 600.0);
                    if f.glyph_id('0').0 != 0 {
                        return Some(f);
                    }
                }
            }
            None
        })
        .as_ref()
}

/// Anti-aliased coverage of `text` drawn into a `w`×`h` map at x0, vertically
/// centred on the digits. Returns the advance width used.
fn draw_ui_text(cov: &mut [f32], w: usize, h: usize, font: &ab_glyph::FontVec, text: &str, size: f32, x0: f32) -> f32 {
    use ab_glyph::{point, Font, PxScale, ScaleFont};
    let scale = PxScale::from(size);
    let sf = font.as_scaled(scale);
    // Centre on the digit height so "%" and "?" do not shift the baseline.
    let zero = font.outline_glyph(font.glyph_id('0').with_scale_and_position(scale, point(0.0, 0.0)));
    let (top, bottom) = zero.map(|o| (o.px_bounds().min.y, o.px_bounds().max.y)).unwrap_or((-size * 0.7, 0.0));
    let baseline = (h as f32 - (bottom - top)) / 2.0 - top;
    let mut x = x0;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(scale, point(x, baseline.round()));
        if let Some(o) = font.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, c| {
                let px = b.min.x as i32 + gx as i32;
                let py = b.min.y as i32 + gy as i32;
                if (0..w as i32).contains(&px) && (0..h as i32).contains(&py) {
                    let i = py as usize * w + px as usize;
                    cov[i] = (cov[i] + c).min(1.0);
                }
            });
        }
        x += sf.h_advance(id);
        prev = Some(id);
    }
    x - x0
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn ui_text_width(font: &ab_glyph::FontVec, text: &str, size: f32) -> f32 {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let sf = font.as_scaled(PxScale::from(size));
    let mut prev = None;
    let mut x = 0.0;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        x += sf.h_advance(id);
        prev = Some(id);
    }
    x
}

/// macOS menu-bar image: for each part, its purple artwork with the text
/// (e.g. "22%") beside it in white system-font text. The menu bar shows icons
/// 18 pt tall and keeps the aspect ratio, so this is a wide 36 px tall image
/// (Retina). Each part has its own text alpha (< 1 = dimmed, for blinking).
/// Returns (straight RGBA, width, height).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn render_mac(parts: &[(MacArt, &str, f32)]) -> Option<(Vec<u8>, usize, usize)> {
    const H: usize = 36;
    const FONT_PX: f32 = 26.0;
    const ART_GAP: usize = 5; // artwork → text
    const PART_GAP: usize = 16; // between headset and mouse
    let font = ui_font();

    // Layout pass.
    struct Laid {
        art: Vec<[f32; 4]>,
        aw: usize,
        ah: usize,
        text_x: usize,
        text_w: usize,
    }
    let mut laid = vec![];
    let mut x = 0usize;
    for (i, (kind, text, _)) in parts.iter().enumerate() {
        if i > 0 {
            x += PART_GAP;
        }
        let a = match kind {
            MacArt::Headset => mac_art()?,
            MacArt::Mouse => mouse_art()?,
        };
        // The mouse is tall: keep a little air above and below it.
        let ah = match kind {
            MacArt::Headset => H,
            MacArt::Mouse => H - 4,
        };
        let aw = ((ah * a.w) as f32 / a.h as f32).round().max(1.0) as usize;
        let art = downscale(a, aw, ah);
        let text_x = x + aw + ART_GAP;
        let text_w = match font {
            Some(f) => ui_text_width(f, text, FONT_PX).ceil() as usize,
            None => text_width(text) * 3,
        };
        laid.push(Laid { art, aw, ah, text_x, text_w });
        x = text_x + text_w;
    }
    let w = x + 2;

    // Paint pass.
    let mut px = vec![[0f32; 4]; w * H];
    let mut art_x = 0usize;
    for (i, l) in laid.iter().enumerate() {
        if i > 0 {
            art_x = laid[i - 1].text_x + laid[i - 1].text_w + PART_GAP;
        }
        let oy = (H - l.ah) / 2;
        for y in 0..l.ah {
            for x in 0..l.aw {
                px[(y + oy) * w + art_x + x] = l.art[y * l.aw + x];
            }
        }
        let (_, text, alpha) = parts[i];
        let mut cov = vec![0f32; w * H];
        match font {
            Some(f) => {
                draw_ui_text(&mut cov, w, H, f, text, FONT_PX, l.text_x as f32);
            }
            None => {
                let m = text_mask_at(w, H, text, 3, l.text_x, (H - 21) / 2);
                for (c, on) in cov.iter_mut().zip(m) {
                    if on {
                        *c = 1.0;
                    }
                }
            }
        }
        // White text with a soft dark edge so it stays readable on a light menu bar.
        for y in 0..H {
            for x in l.text_x.saturating_sub(2)..(l.text_x + l.text_w + 2).min(w) {
                let i = y * w + x;
                let mut edge = 0f32;
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                    if (0..w as i32).contains(&sx) && (0..H as i32).contains(&sy) {
                        edge = edge.max(cov[sy as usize * w + sx as usize]);
                    }
                }
                let t = cov[i] * alpha;
                let s = edge * 0.30 * (1.0 - cov[i]);
                if t <= 0.0 && s <= 0.0 {
                    continue;
                }
                // white text over shadow, premultiplied
                let a = t + s * (1.0 - t);
                px[i] = [t, t, t, a];
            }
        }
    }

    let mut out = vec![0u8; w * H * 4];
    for (i, p) in px.iter().enumerate() {
        if p[3] <= 0.0 {
            continue;
        }
        for c in 0..3 {
            out[i * 4 + c] = ((p[c] / p[3]).min(1.0) * 255.0).round() as u8;
        }
        out[i * 4 + 3] = (p[3].min(1.0) * 255.0).round() as u8;
    }
    Some((out, w, H))
}

#[cfg(test)]
mod tests {
    use super::render;

    /// Writes enlarged previews to target/icon-preview/icons.png on dark and light taskbar colours.
    #[test]
    fn preview_png() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/icon-preview");
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            ("22", (0xfa, 0xcc, 0x15)),
            ("100", (0x22, 0xc5, 0x5e)),
            ("75", (0x22, 0xc5, 0x5e)),
            ("7", (0xef, 0x44, 0x44)),
            ("?", (0xd0, 0xd0, 0xd6)),
        ];
        let zoom = 8usize;
        let sizes = [16usize, 24, 44];
        let cell = 44 * zoom + 16;
        let (w, h) = (cell * cases.len(), cell * sizes.len() * 2);
        let mut img = vec![0u8; w * h * 3];
        for (ri, &n) in sizes.iter().enumerate() {
            for (light, bg) in [(0usize, (0x20u8, 0x20u8, 0x24u8)), (1, (0xf3, 0xf3, 0xf3))] {
                let row = ri * 2 + light;
                for (col, (t, c)) in cases.iter().enumerate() {
                    let px = render(n, t, *c, 1.0).expect("artwork");
                    for y in 0..cell {
                        for x in 0..cell {
                            let mut rgb = [bg.0, bg.1, bg.2];
                            let (ix, iy) = (x / zoom, y / zoom);
                            if ix < n && iy < n {
                                let i = (iy * n + ix) * 4;
                                let a = px[i + 3] as f32 / 255.0;
                                for k in 0..3 {
                                    rgb[k] = (px[i + k] as f32 * a + rgb[k] as f32 * (1.0 - a)) as u8;
                                }
                            }
                            let o = ((row * cell + y) * w + col * cell + x) * 3;
                            img[o..o + 3].copy_from_slice(&rgb);
                        }
                    }
                }
            }
        }
        let file = std::fs::File::create(dir.join("icons.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.write_header().unwrap().write_image_data(&img).unwrap();
    }
}

#[cfg(test)]
mod mac_tests {
    // `image` is a dev-dependency for examples/clean_icon.rs only.
    use image as _;
    use super::{render_mac, MacArt};

    /// Writes target/icon-preview/mac.png: the menu-bar image (zoomed 4×) on a
    /// dark and a light menu bar.
    #[test]
    fn preview_mac_png() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/icon-preview");
        std::fs::create_dir_all(&dir).unwrap();
        let cases: Vec<Vec<(MacArt, &str, f32)>> = vec![
            vec![(MacArt::Headset, "22%", 1.0)],
            vec![(MacArt::Headset, "100%", 1.0), (MacArt::Mouse, "80%", 1.0)],
            vec![(MacArt::Headset, "7%", 1.0), (MacArt::Mouse, "--", 1.0)],
        ];
        let zoom = 4usize;
        let renders: Vec<_> = cases.iter().map(|c| render_mac(c).expect("artwork")).collect();
        let cell_w = renders.iter().map(|r| r.1).max().unwrap() * zoom + 24;
        let cell_h = 36 * zoom + 24;
        let (w, h) = (cell_w * cases.len(), cell_h * 2);
        let mut img = vec![0u8; w * h * 3];
        for (row, bg) in [(0usize, [0x1e, 0x1e, 0x22]), (1, [0xec, 0xec, 0xee])] {
            for (col, (px, iw, ih)) in renders.iter().enumerate() {
                for y in 0..cell_h {
                    for x in 0..cell_w {
                        let mut rgb = bg;
                        let (ix, iy) = ((x.saturating_sub(12)) / zoom, (y.saturating_sub(12)) / zoom);
                        if x >= 12 && y >= 12 && ix < *iw && iy < *ih {
                            let i = (iy * iw + ix) * 4;
                            let a = px[i + 3] as f32 / 255.0;
                            for k in 0..3 {
                                rgb[k] = (px[i + k] as f32 * a + rgb[k] as f32 * (1.0 - a)) as u8;
                            }
                        }
                        let o = ((row * cell_h + y) * w + col * cell_w + x) * 3;
                        img[o..o + 3].copy_from_slice(&rgb);
                    }
                }
            }
        }
        let file = std::fs::File::create(dir.join("mac.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.write_header().unwrap().write_image_data(&img).unwrap();
    }
}
