//! Removes a baked-in "transparency" checkerboard from pixel art and writes a
//! real transparent, cropped PNG.
//!
//!   cargo run --example clean_icon -- <input.webp|png> <output.png>
//!
//! Background = neutral (grey/white, low chroma) pixels reachable from the
//! image border. Coloured artwork and anything enclosed by it is kept.

use image::{Rgba, RgbaImage};
use std::collections::VecDeque;

fn chroma(p: &Rgba<u8>) -> u8 {
    let [r, g, b, _] = p.0;
    r.max(g).max(b) - r.min(g).min(b)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (input, output) = (&args[1], &args[2]);
    let mut img: RgbaImage = image::open(input).expect("open").to_rgba8();
    let (w, h) = img.dimensions();

    let neutral = |p: &Rgba<u8>| p.0[3] < 16 || chroma(p) < 18;

    // Flood-fill the background from every border pixel through neutral pixels.
    let mut bg = vec![false; (w * h) as usize];
    let mut q = VecDeque::new();
    for x in 0..w {
        q.push_back((x, 0));
        q.push_back((x, h - 1));
    }
    for y in 0..h {
        q.push_back((0, y));
        q.push_back((w - 1, y));
    }
    while let Some((x, y)) = q.pop_front() {
        let i = (y * w + x) as usize;
        if bg[i] || !neutral(img.get_pixel(x, y)) {
            continue;
        }
        bg[i] = true;
        if x > 0 { q.push_back((x - 1, y)); }
        if x + 1 < w { q.push_back((x + 1, y)); }
        if y > 0 { q.push_back((x, y - 1)); }
        if y + 1 < h { q.push_back((x, y + 1)); }
    }

    // Keep only the largest connected piece of artwork (drops stray noise).
    let mut comp = vec![0u32; (w * h) as usize];
    let mut sizes = vec![0usize];
    for sy in 0..h {
        for sx in 0..w {
            let si = (sy * w + sx) as usize;
            if bg[si] || comp[si] != 0 {
                continue;
            }
            let id = sizes.len() as u32;
            let mut n = 0;
            q.push_back((sx, sy));
            while let Some((x, y)) = q.pop_front() {
                let i = (y * w + x) as usize;
                if bg[i] || comp[i] != 0 {
                    continue;
                }
                comp[i] = id;
                n += 1;
                if x > 0 { q.push_back((x - 1, y)); }
                if x + 1 < w { q.push_back((x + 1, y)); }
                if y > 0 { q.push_back((x, y - 1)); }
                if y + 1 < h { q.push_back((x, y + 1)); }
            }
            sizes.push(n);
        }
    }
    let biggest = (1..sizes.len()).max_by_key(|&i| sizes[i]).expect("no artwork") as u32;

    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if comp[i] == biggest {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            } else {
                img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
    let cropped = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
    cropped.save(output).expect("save");
    println!("{}x{} -> cropped {}x{} at ({x0},{y0}), kept {} px", w, h, x1 - x0, y1 - y0, sizes[biggest as usize]);
}
