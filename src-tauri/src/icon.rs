//! Tray icon: the duck headphones artwork (assets/duck-headphones.png), scaled to
//! the tray's native size, with the battery percentage in pixel digits on top.

use std::sync::OnceLock;

static DUCK_PNG: &[u8] = include_bytes!("../assets/duck-headphones.png");

/// Dark brown used for the artwork's outlines; also outlines the digits.
const OUTLINE: (u8, u8, u8) = (0x2b, 0x17, 0x0e);

/// Cropped artwork, premultiplied RGBA in 0..1.
struct Art {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>,
}

static ART: OnceLock<Option<Art>> = OnceLock::new();

fn art() -> Option<&'static Art> {
    ART.get_or_init(load_art).as_ref()
}

fn load_art() -> Option<Art> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(DUCK_PNG));
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

/// Box-filters the artwork down to `tw`Ă—`th`.
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

/// Bold 5Ă—7 digits with 2 px strokes (bit 4 = leftmost column);
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

/// (rows, width) for each character of `text`.
fn glyphs(text: &str) -> Vec<(&'static [u8; 7], usize)> {
    let three = text.len() >= 3;
    text.bytes()
        .map(|b| match b {
            b'1' if three => (&ONE_NARROW, 2),
            b'0'..=b'9' => (&FONT[(b - b'0') as usize], 5),
            b'-' => (&FONT[11], 5),
            _ => (&FONT[10], 5),
        })
        .collect()
}

/// Width of `text` in font pixels (1 px gap between characters).
fn text_width(text: &str) -> usize {
    let g = glyphs(text);
    g.iter().map(|(_, w)| w).sum::<usize>() + g.len().saturating_sub(1)
}

/// Text mask on an `n`Ă—`n` grid, font pixels scaled by `k`, centred on (cx, cy).
fn text_mask(n: usize, text: &str, k: usize, cx: f32, cy: f32) -> Vec<bool> {
    let w = text_width(text) * k;
    let h = 7 * k;
    let x0 = (cx - w as f32 / 2.0).round().max(0.0) as usize;
    let y0 = (cy - h as f32 / 2.0).round().max(0.0) as usize;
    let mut m = vec![false; n * n];
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
                        if x < n && y < n {
                            m[y * n + x] = true;
                        }
                    }
                }
            }
        }
        gx += (gw + 1) * k;
    }
    m
}

fn dilate(n: usize, m: &[bool], r: usize) -> Vec<bool> {
    let r = r as i32;
    let mut out = vec![false; n * n];
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            'o: for dy in -r..=r {
                for dx in -r..=r {
                    let (sx, sy) = (x + dx, y + dy);
                    if (0..n as i32).contains(&sx) && (0..n as i32).contains(&sy) && m[(sy * n as i32 + sx) as usize] {
                        out[(y * n as i32 + x) as usize] = true;
                        break 'o;
                    }
                }
            }
        }
    }
    out
}

/// Straight RGBA `n`Ă—`n` icon. `color` = digit colour, `alpha` < 1 dims the digits (blink).
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
