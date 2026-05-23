//! Procedural window icon — a stylized "w" on an off-white card.
//!
//! Generated in code so we don't need to ship binary assets in the repo.
//! Resolution is 64×64 RGBA, which winit / Linux taskbars accept directly.
//! Designed to read against both light and dark OS chrome by carrying its
//! own border.

const SIZE: u32 = 64;

const GLYPH: [[u8; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 1, 0, 1, 0, 0],
    [1, 0, 0, 1, 0, 1, 0, 0],
    [1, 0, 1, 0, 1, 0, 1, 0],
    [1, 0, 1, 0, 1, 0, 1, 0],
    [0, 1, 0, 0, 0, 1, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
];

pub fn rgba_64() -> (u32, u32, Vec<u8>) {
    let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];
    for chunk in buf.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[251, 251, 251, 255]);
    }
    for x in 0..SIZE {
        let top = (x * 4) as usize;
        let bottom = ((SIZE - 1) * SIZE * 4 + x * 4) as usize;
        buf[top..top + 4].copy_from_slice(&[40, 40, 40, 255]);
        buf[bottom..bottom + 4].copy_from_slice(&[40, 40, 40, 255]);
    }
    for y in 0..SIZE {
        let left = (y * SIZE * 4) as usize;
        let right = (y * SIZE * 4 + (SIZE - 1) * 4) as usize;
        buf[left..left + 4].copy_from_slice(&[40, 40, 40, 255]);
        buf[right..right + 4].copy_from_slice(&[40, 40, 40, 255]);
    }
    let glyph_scale = 6u32;
    let glyph_size_px = 8 * glyph_scale;
    let pad = (SIZE - glyph_size_px) / 2;
    for gy in 0..8 {
        for gx in 0..8 {
            if GLYPH[gy as usize][gx as usize] == 0 {
                continue;
            }
            for dy in 0..glyph_scale {
                for dx in 0..glyph_scale {
                    let x = pad + gx * glyph_scale + dx;
                    let y = pad + gy * glyph_scale + dy;
                    let i = ((y * SIZE + x) * 4) as usize;
                    buf[i..i + 4].copy_from_slice(&[18, 18, 18, 255]);
                }
            }
        }
    }
    (SIZE, SIZE, buf)
}
