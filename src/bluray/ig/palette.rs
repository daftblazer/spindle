// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared-palette quantization for IG bitmaps.

use super::Bitmap;
use anyhow::{anyhow, Result};
use imagequant::RGBA;

/// Bitmaps mapped onto one palette. Index 0 is always fully transparent.
pub struct Quantized {
    /// Straight-alpha RGBA palette entries.
    pub palette: Vec<[u8; 4]>,
    /// Per-bitmap index buffers, in input order.
    pub indexes: Vec<Vec<u8>>,
}

impl Quantized {
    /// Palette as (Y, Cr, Cb, T) using BT.709 limited range.
    pub fn palette_ycrcb(&self) -> Vec<[u8; 4]> {
        self.palette
            .iter()
            .map(|&[r, g, b, a]| {
                let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
                let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let cb = (b - y) / 1.8556;
                let cr = (r - y) / 1.5748;
                let q = |v: f32| v.round().clamp(0.0, 255.0) as u8;
                [q(16.0 + 219.0 * y), q(128.0 + 224.0 * cr), q(128.0 + 224.0 * cb), a]
            })
            .collect()
    }
}

fn pixels(b: &Bitmap) -> Vec<RGBA> {
    b.rgba.as_chunks::<4>().0.iter().map(|p| RGBA::new(p[0], p[1], p[2], p[3])).collect()
}

pub fn quantize(bitmaps: &[&Bitmap]) -> Result<Quantized> {
    let err = |e: imagequant::Error| anyhow!("palette quantization failed: {e}");
    let mut attr = imagequant::new();
    attr.set_max_colors(255).map_err(err)?;
    attr.set_quality(0, 100).map_err(err)?;
    attr.set_speed(3).map_err(err)?;

    let mut hist = imagequant::Histogram::new(&attr);
    for b in bitmaps {
        let mut img = attr
            .new_image(pixels(b), b.width as usize, b.height as usize, 0.0)
            .map_err(err)?;
        hist.add_image(&attr, &mut img).map_err(err)?;
    }
    let mut res = hist.quantize(&attr).map_err(err)?;
    res.set_dithering_level(0.5).map_err(err)?;

    // The palette may be refined by the first remap; retry once so every
    // bitmap is mapped with the same final palette.
    for _ in 0..2 {
        let mut remapped = Vec::with_capacity(bitmaps.len());
        for b in bitmaps {
            let mut img = attr
                .new_image(pixels(b), b.width as usize, b.height as usize, 0.0)
                .map_err(err)?;
            remapped.push(res.remapped(&mut img).map_err(err)?);
        }
        let Some((pal, _)) = remapped.first() else {
            return Ok(Quantized { palette: vec![[0, 0, 0, 0]], indexes: vec![] });
        };
        if remapped.iter().any(|(p, _)| p != pal) {
            continue;
        }
        let pal = pal.clone();
        let mut palette: Vec<[u8; 4]> = vec![[0, 0, 0, 0]];
        palette.extend(pal.iter().map(|c| [c.r, c.g, c.b, c.a]));
        let indexes = remapped
            .into_iter()
            .map(|(_, idx)| idx.into_iter().map(|i| if pal[i as usize].a == 0 { 0 } else { i + 1 }).collect())
            .collect();
        return Ok(Quantized { palette, indexes });
    }
    Err(anyhow!("palette quantization did not converge"))
}
