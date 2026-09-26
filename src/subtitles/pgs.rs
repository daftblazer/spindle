// SPDX-License-Identifier: GPL-3.0-or-later

//! Presentation Graphics (PGS) streams: encoding timed bitmaps into display
//! sets, and decoding `.sup` files / embedded PGS back into bitmaps.

use super::SubImage;
use crate::bluray::bytes::BitWriter;
use crate::bluray::ig::{self, palette, rle, Bitmap};
use crate::bluray::ts::demux::Pes;
use crate::bluray::ts::mux::private_pes;
use crate::bluray::VideoFormat;
use anyhow::{bail, Result};
use std::collections::HashMap;

const SEG_PDS: u8 = 0x14;
const SEG_ODS: u8 = 0x15;
const SEG_PCS: u8 = 0x16;
const SEG_WDS: u8 = 0x17;
const SEG_END: u8 = ig::SEG_END;

/// 90 kHz ticks to decode `pixels` of object data at the PG decoding rate.
fn decode_ticks(pixels: u64) -> u64 {
    const RD: u64 = 128_000_000;
    (pixels * 8 * 90_000).div_ceil(RD)
}

/// 90 kHz ticks to draw (or clear) a window of `pixels` in the PG plane.
fn window_ticks(pixels: u64) -> u64 {
    const RC: u64 = 256_000_000;
    (pixels * 8 * 90_000).div_ceil(RC).max(1)
}

struct Window {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

fn pcs(video: VideoFormat, number: u16, epoch_start: bool, object: Option<(&Window, bool)>) -> Vec<u8> {
    let (w, h) = video.size();
    let mut b = BitWriter::new();
    b.u16(w as u16).u16(h as u16).bits(4, video.ig_rate_code() as u64).zeros(4);
    b.u16(number).bits(2, if epoch_start { 2 } else { 0 }).zeros(6);
    b.flag(false).zeros(7); // palette_update_flag
    b.u8(0); // palette id
    match object {
        Some((win, forced)) => {
            b.u8(1);
            b.u16(0).u8(0).flag(false).flag(forced).zeros(6).u16(win.x).u16(win.y);
        }
        None => {
            b.u8(0);
        }
    }
    ig::segment(SEG_PCS, &b.into_bytes())
}

fn wds(win: &Window) -> Vec<u8> {
    let mut b = BitWriter::new();
    b.u8(1).u8(0).u16(win.x).u16(win.y).u16(win.w).u16(win.h);
    ig::segment(SEG_WDS, &b.into_bytes())
}

fn pes(seg: Vec<u8>, pts: u64, dts: u64) -> Pes {
    Pes { stream: 0, data: private_pes(&seg, pts, dts), pts: Some(pts), dts: Some(dts), random_access: false }
}

/// Encode images into PG display sets. `to_pts` maps a subtitle time in
/// seconds to a 90 kHz presentation timestamp on the clip's clock.
pub fn encode(images: &[SubImage], video: VideoFormat, to_pts: impl Fn(f64) -> Option<u64>) -> Result<Vec<Pes>> {
    let (vw, vh) = video.size();
    let mut out = Vec::new();
    let mut number: u16 = 0;
    let mut last_dts: u64 = 0;
    let mut shown: Option<(Window, u64)> = None; // current window and when it ends

    let clear = |out: &mut Vec<Pes>, number: &mut u16, last_dts: &mut u64, win: &Window, at: u64| {
        let wt = window_ticks(win.w as u64 * win.h as u64);
        let t = at.saturating_sub(wt).max(*last_dts);
        let present = t + wt;
        out.push(pes(pcs(video, *number, false, None), present, t));
        out.push(pes(wds(win), t, t));
        out.push(pes(ig::segment(SEG_END, &[]), t, t));
        *number = number.wrapping_add(1);
        *last_dts = t;
    };

    for img in images {
        let (Some(start), Some(end)) = (to_pts(img.start), to_pts(img.end)) else { continue };
        if end <= start || img.bitmap.width == 0 || img.bitmap.height == 0 {
            continue;
        }
        // Keep the object inside the video frame.
        let x = img.x.min(vw - 1) as u16;
        let y = img.y.min(vh - 1) as u16;
        let w = (img.bitmap.width as u32).min(vw - x as u32) as u16;
        let h = (img.bitmap.height as u32).min(vh - y as u32) as u16;
        let bitmap = if (w, h) != (img.bitmap.width, img.bitmap.height) { crop(&img.bitmap, w, h) } else { img.bitmap.clone() };
        let win = Window { x, y, w, h };

        // End the previous subtitle unless this one replaces it directly.
        if let Some((prev, prev_end)) = shown.take() {
            if prev_end < start {
                clear(&mut out, &mut number, &mut last_dts, &prev, prev_end);
            }
        }

        let q = palette::quantize(&[&bitmap])?;
        let rle_data = rle::encode(&q.indexes[0], w as usize, h as usize);
        let decode = decode_ticks(w as u64 * h as u64);
        let wt = window_ticks(w as u64 * h as u64);
        let t0 = start.saturating_sub(decode + wt + 900).max(last_dts);
        let present = start.max(t0 + decode + wt);

        out.push(pes(pcs(video, number, true, Some((&win, img.forced))), present, t0));
        out.push(pes(wds(&win), present - wt, t0));
        out.push(pes(ig::pds(&q), t0, t0));
        let frags = ig::ods(0, w, h, &rle_data);
        let n = frags.len();
        for (k, f) in frags.into_iter().enumerate() {
            let pts = if k == n - 1 { t0 + decode } else { t0 };
            out.push(pes(f, pts, t0));
        }
        out.push(pes(ig::segment(SEG_END, &[]), t0 + decode, t0 + decode));
        number = number.wrapping_add(1);
        last_dts = t0 + decode;

        shown = Some((win, end.max(present + 1)));
    }
    if let Some((win, end)) = shown {
        clear(&mut out, &mut number, &mut last_dts, &win, end);
    }
    Ok(out)
}

fn crop(b: &Bitmap, w: u16, h: u16) -> Bitmap {
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let row = y * b.width as usize * 4;
        rgba.extend_from_slice(&b.rgba[row..row + w as usize * 4]);
    }
    Bitmap { width: w, height: h, rgba }
}

/// Write PES packets as a `.sup` file (for inspection with other tools).
#[allow(dead_code)]
pub fn to_sup(packets: &[Pes]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in packets {
        let off = crate::bluray::ts::demux::pes_payload_offset(&p.data);
        out.extend_from_slice(b"PG");
        out.extend_from_slice(&(p.pts.unwrap_or(0) as u32).to_be_bytes());
        out.extend_from_slice(&(p.dts.unwrap_or(0) as u32).to_be_bytes());
        out.extend_from_slice(&p.data[off..]);
    }
    out
}

// ---------------------------------------------------------------- decoding

fn ycrcb_to_rgba(e: [u8; 4]) -> [u8; 4] {
    let (y, cr, cb) = (e[0] as f32 - 16.0, e[1] as f32 - 128.0, e[2] as f32 - 128.0);
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [c(1.164 * y + 1.793 * cr), c(1.164 * y - 0.213 * cb - 0.533 * cr), c(1.164 * y + 2.112 * cb), e[3]]
}

fn decode_rle(mut d: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h);
    let mut line = Vec::with_capacity(w);
    while !d.is_empty() && out.len() < w * h {
        let (color, len);
        if d[0] != 0 {
            (color, len, d) = (d[0], 1usize, &d[1..]);
        } else if d.len() < 2 {
            break;
        } else {
            let f = d[1];
            let get = |i: usize| d.get(i).copied().unwrap_or(0) as usize;
            match f >> 6 {
                0 => (color, len, d) = (0, (f & 0x3F) as usize, &d[2..]),
                1 => (color, len, d) = (0, (((f & 0x3F) as usize) << 8) | get(2), &d[3.min(d.len())..]),
                2 => (color, len, d) = (get(2) as u8, (f & 0x3F) as usize, &d[3.min(d.len())..]),
                _ => (color, len, d) = (get(3) as u8, (((f & 0x3F) as usize) << 8) | get(2), &d[4.min(d.len())..]),
            }
        }
        if len == 0 {
            line.resize(w, 0);
            out.append(&mut line);
        } else {
            line.extend(std::iter::repeat_n(color, len));
        }
    }
    line.truncate(w);
    out.append(&mut line);
    out.resize(w * h, 0);
    out
}

/// A decoded PGS image in the stream's own canvas.
pub struct Decoded {
    pub canvas: (u16, u16),
    pub image: SubImage,
}

struct PcsObject {
    object: u16,
    x: u16,
    y: u16,
    crop: Option<(u16, u16, u16, u16)>,
    forced: bool,
}

/// Parse a `.sup` file into timed images (seconds on the file's clock).
pub fn decode_sup(data: &[u8]) -> Result<Vec<Decoded>> {
    let mut palettes: HashMap<u8, [[u8; 4]; 256]> = HashMap::new();
    let mut objects: HashMap<u16, (u16, u16, Vec<u8>)> = HashMap::new();
    let mut pending_ods: HashMap<u16, (u16, u16, Vec<u8>)> = HashMap::new();
    let mut canvas = (1920u16, 1080u16);
    let mut current: Option<(f64, u8, Vec<PcsObject>)> = None;
    let mut out: Vec<Decoded> = Vec::new();

    let mut i = 0;
    while i + 13 <= data.len() {
        if &data[i..i + 2] != b"PG" {
            bail!("not a PGS (.sup) stream at offset {i}");
        }
        let pts = u32::from_be_bytes(data[i + 2..i + 6].try_into().unwrap()) as f64 / 90_000.0;
        let kind = data[i + 10];
        let len = u16::from_be_bytes([data[i + 11], data[i + 12]]) as usize;
        let body = data.get(i + 13..i + 13 + len).unwrap_or(&[]);
        i += 13 + len;
        match kind {
            SEG_PCS if body.len() >= 11 => {
                canvas = (u16::from_be_bytes([body[0], body[1]]), u16::from_be_bytes([body[2], body[3]]));
                let epoch_start = body[7] >> 6 == 2;
                if epoch_start {
                    objects.clear();
                }
                let palette_id = body[9];
                let n = body[10] as usize;
                let mut objs = Vec::new();
                let mut p = 11;
                for _ in 0..n {
                    if p + 8 > body.len() {
                        break;
                    }
                    let object = u16::from_be_bytes([body[p], body[p + 1]]);
                    let cropped = body[p + 3] & 0x80 != 0;
                    let forced = body[p + 3] & 0x40 != 0;
                    let x = u16::from_be_bytes([body[p + 4], body[p + 5]]);
                    let y = u16::from_be_bytes([body[p + 6], body[p + 7]]);
                    p += 8;
                    let crop = if cropped && p + 8 <= body.len() {
                        let v = |o: usize| u16::from_be_bytes([body[p + o], body[p + o + 1]]);
                        let c = (v(0), v(2), v(4), v(6));
                        p += 8;
                        Some(c)
                    } else {
                        None
                    };
                    objs.push(PcsObject { object, x, y, crop, forced });
                }
                // A new composition ends the previous one.
                if let Some(last) = out.last_mut() {
                    if last.image.end <= last.image.start {
                        last.image.end = pts;
                    }
                }
                current = Some((pts, palette_id, objs));
            }
            SEG_PDS if body.len() >= 2 => {
                let pal = palettes.entry(body[0]).or_insert([[16, 128, 128, 0]; 256]);
                for e in body[2..].as_chunks::<5>().0 {
                    pal[e[0] as usize] = [e[1], e[2], e[3], e[4]];
                }
            }
            SEG_ODS if body.len() >= 4 => {
                let id = u16::from_be_bytes([body[0], body[1]]);
                let first = body[3] & 0x80 != 0;
                if first && body.len() >= 11 {
                    let w = u16::from_be_bytes([body[7], body[8]]);
                    let h = u16::from_be_bytes([body[9], body[10]]);
                    pending_ods.insert(id, (w, h, body[11..].to_vec()));
                } else if let Some(o) = pending_ods.get_mut(&id) {
                    o.2.extend_from_slice(&body[4..]);
                }
                if body[3] & 0x40 != 0 {
                    if let Some(o) = pending_ods.remove(&id) {
                        objects.insert(id, o);
                    }
                }
            }
            SEG_END => {
                let Some((start, palette_id, objs)) = current.take() else { continue };
                if objs.is_empty() {
                    continue;
                }
                let pal = palettes.get(&palette_id).copied().unwrap_or([[16, 128, 128, 0]; 256]);
                // Compose all objects into one bitmap.
                let mut rects = Vec::new();
                for o in &objs {
                    if let Some((w, h, _)) = objects.get(&o.object) {
                        let (cw, ch) = o.crop.map_or((*w, *h), |c| (c.2, c.3));
                        rects.push((o.x, o.y, cw, ch));
                    }
                }
                if rects.is_empty() {
                    continue;
                }
                let x0 = rects.iter().map(|r| r.0).min().unwrap();
                let y0 = rects.iter().map(|r| r.1).min().unwrap();
                let x1 = rects.iter().map(|r| r.0 + r.2).max().unwrap();
                let y1 = rects.iter().map(|r| r.1 + r.3).max().unwrap();
                let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
                let mut rgba = vec![0u8; bw * bh * 4];
                for o in &objs {
                    let Some((w, h, data)) = objects.get(&o.object) else { continue };
                    let idx = decode_rle(data, *w as usize, *h as usize);
                    let (cx, cy, cw, ch) = o.crop.unwrap_or((0, 0, *w, *h));
                    for yy in 0..ch as usize {
                        for xx in 0..cw as usize {
                            let (sx, sy) = (cx as usize + xx, cy as usize + yy);
                            if sx >= *w as usize || sy >= *h as usize {
                                continue;
                            }
                            let c = ycrcb_to_rgba(pal[idx[sy * *w as usize + sx] as usize]);
                            let dx = (o.x - x0) as usize + xx;
                            let dy = (o.y - y0) as usize + yy;
                            rgba[(dy * bw + dx) * 4..(dy * bw + dx) * 4 + 4].copy_from_slice(&c);
                        }
                    }
                }
                let forced = objs.iter().any(|o| o.forced);
                out.push(Decoded {
                    canvas,
                    image: SubImage {
                        start,
                        end: start,
                        x: x0 as u32,
                        y: y0 as u32,
                        bitmap: Bitmap { width: bw as u16, height: bh as u16, rgba },
                        forced,
                    },
                });
            }
            _ => {}
        }
    }
    // An unterminated last subtitle shows for a few seconds.
    if let Some(last) = out.last_mut() {
        if last.image.end <= last.image.start {
            last.image.end = last.image.start + 4.0;
        }
    }
    out.retain(|d| d.image.end > d.image.start);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_like(w: u16, h: u16) -> Bitmap {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let on = (x / 6 + y / 6) % 2 == 0;
                rgba.extend_from_slice(if on { &[255, 255, 255, 255] } else { &[0, 0, 0, 0] });
            }
        }
        Bitmap { width: w, height: h, rgba }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let imgs = vec![
            SubImage { start: 1.0, end: 2.0, x: 600, y: 900, bitmap: text_like(300, 60), forced: false },
            SubImage { start: 2.0, end: 3.5, x: 500, y: 950, bitmap: text_like(420, 48), forced: true },
            SubImage { start: 10.0, end: 12.0, x: 100, y: 100, bitmap: text_like(64, 64), forced: false },
        ];
        let base = 90_000 * 5;
        let pes = encode(&imgs, VideoFormat::P1080_23976, |t| Some(base + (t * 90_000.0) as u64)).unwrap();
        // DTS never goes backwards and never exceeds PTS.
        let mut last = 0;
        for p in &pes {
            assert!(p.dts.unwrap() >= last);
            assert!(p.dts.unwrap() <= p.pts.unwrap());
            last = p.dts.unwrap();
        }
        let back = decode_sup(&to_sup(&pes)).unwrap();
        assert_eq!(back.len(), 3);
        for (d, orig) in back.iter().zip(&imgs) {
            let start = d.image.start - base as f64 / 90_000.0;
            let end = d.image.end - base as f64 / 90_000.0;
            assert!((start - orig.start).abs() < 0.05, "start {start} vs {}", orig.start);
            assert!((end - orig.end).abs() < 0.05, "end {end} vs {}", orig.end);
            assert_eq!((d.image.x, d.image.y), (orig.x, orig.y));
            assert_eq!((d.image.bitmap.width, d.image.bitmap.height), (orig.bitmap.width, orig.bitmap.height));
            assert_eq!(d.image.forced, orig.forced);
            // Opaque pixels survive (white stays white).
            let px = &d.image.bitmap.rgba[0..4];
            assert!(px[3] == 255 && px[0] > 240, "{px:?}");
        }
    }
}
