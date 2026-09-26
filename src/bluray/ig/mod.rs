// SPDX-License-Identifier: GPL-3.0-or-later

//! HDMV Interactive Graphics (IG) stream encoder.
//!
//! Produces one epoch-start display set: PDS (palette), ODS (button bitmaps),
//! ICS (the interactive composition with pages/buttons/commands) and END.

pub(crate) mod palette;
pub(crate) mod rle;

use crate::bluray::bytes::BitWriter;
use crate::bluray::nav::hdmv::Command;
use crate::bluray::ts::demux::Pes;
use crate::bluray::ts::mux::private_pes;
use crate::bluray::VideoFormat;
use anyhow::{bail, Result};

pub use palette::Quantized;

const SEG_PDS: u8 = 0x14;
const SEG_ODS: u8 = 0x15;
const SEG_ICS: u8 = 0x18;
pub(crate) const SEG_END: u8 = 0x80;

/// Keep segments (plus PES header) within the 16-bit PES length.
const MAX_SEGMENT_PAYLOAD: usize = 65_000;

pub const NO_BUTTON: u16 = 0xFFFF;

/// Straight-alpha RGBA bitmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Button {
    pub id: u16,
    /// Value selecting the button with the remote's number keys.
    pub numeric: u16,
    pub x: u16,
    pub y: u16,
    pub up: u16,
    pub down: u16,
    pub left: u16,
    pub right: u16,
    pub normal: Bitmap,
    pub selected: Bitmap,
    pub activated: Bitmap,
    pub commands: Vec<Command>,
    pub auto_action: bool,
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub video: VideoFormat,
    pub buttons: Vec<Button>,
    /// Button selected when the menu appears; [`NO_BUTTON`] keeps the
    /// player's last selection (PSR10) or falls back to the first button.
    pub default_button: u16,
}

pub(crate) fn segment(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 3);
    v.push(kind);
    v.extend_from_slice(&(body.len() as u16).to_be_bytes());
    v.extend_from_slice(body);
    v
}

pub(crate) fn pds(q: &Quantized) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u8(0).u8(0); // palette id, version
    for (i, e) in q.palette_ycrcb().iter().enumerate() {
        w.u8(i as u8).u8(e[0]).u8(e[1]).u8(e[2]).u8(e[3]);
    }
    segment(SEG_PDS, &w.into_bytes())
}

/// ODS segments for one object, fragmented as needed.
pub(crate) fn ods(id: u16, width: u16, height: u16, rle: &[u8]) -> Vec<Vec<u8>> {
    let mut data = Vec::with_capacity(rle.len() + 4);
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(rle);

    let first_room = MAX_SEGMENT_PAYLOAD - 7;
    let mut chunks: Vec<&[u8]> = Vec::new();
    let (head, mut rest) = data.split_at(data.len().min(first_room));
    chunks.push(head);
    while !rest.is_empty() {
        let (c, r) = rest.split_at(rest.len().min(MAX_SEGMENT_PAYLOAD - 4));
        chunks.push(c);
        rest = r;
    }

    let n = chunks.len();
    chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut w = BitWriter::new();
            w.u16(id).u8(0).flag(i == 0).flag(i == n - 1).zeros(6);
            if i == 0 {
                w.u24(data.len() as u32);
            }
            w.bytes(c);
            segment(SEG_ODS, &w.into_bytes())
        })
        .collect()
}

fn ics(menu: &Menu, object_ids: &[[u16; 3]]) -> Result<Vec<u8>> {
    let (width, height) = menu.video.size();

    let mut comp = BitWriter::new();
    comp.flag(false) // stream_model: multiplexed
        .flag(false) // ui_model: always on
        .zeros(6);
    comp.zeros(7).bits(33, 0); // composition_time_out_pts
    comp.zeros(7).bits(33, 0); // selection_time_out_pts
    comp.u24(0); // user_time_out_duration
    comp.u8(1); // one page

    // page 0
    comp.u8(0).u8(0);
    comp.bytes(&[0; 8]); // UO mask
    comp.u8(0).u8(0); // in effects: no windows, no effects
    comp.u8(0).u8(0); // out effects
    comp.u8(0); // animation frame rate code
    comp.u16(menu.default_button).u16(NO_BUTTON);
    comp.u8(0); // palette id
    comp.u8(menu.buttons.len() as u8); // one BOG per button
    for (b, objs) in menu.buttons.iter().zip(object_ids) {
        comp.u16(b.id).u8(1);
        comp.u16(b.id)
            .u16(b.numeric)
            .flag(b.auto_action)
            .zeros(7)
            .u16(b.x)
            .u16(b.y)
            .u16(b.up)
            .u16(b.down)
            .u16(b.left)
            .u16(b.right);
        comp.u16(objs[0]).u16(objs[0]).flag(false).zeros(7);
        comp.u8(0xFF).u16(objs[1]).u16(objs[1]).flag(false).zeros(7);
        comp.u8(0xFF).u16(objs[2]).u16(objs[2]);
        comp.u16(b.commands.len() as u16);
        for c in &b.commands {
            c.write(&mut comp);
        }
    }
    let comp = comp.into_bytes();

    let mut w = BitWriter::new();
    w.u16(width as u16).u16(height as u16).bits(4, menu.video.ig_rate_code() as u64).zeros(4);
    w.u16(0).bits(2, 2 /* epoch start */).zeros(6);
    w.flag(true).flag(true).zeros(6); // first & last in sequence
    w.u24(comp.len() as u32);
    w.bytes(&comp);
    let body = w.into_bytes();
    if body.len() > MAX_SEGMENT_PAYLOAD {
        bail!("menu has too many buttons/commands for a single composition segment");
    }
    Ok(segment(SEG_ICS, &body))
}

/// Decode duration of `pixels` at the IG decoding rate, in 90 kHz ticks.
fn decode_ticks(pixels: u64) -> u64 {
    const RD: u64 = 64_000_000; // bits per second
    (pixels * 8 * 90_000).div_ceil(RD)
}

/// Encode `menu` as a display set whose composition becomes valid at (or
/// shortly after) `present_pts`. Returns the PES packets in decode order.
pub fn encode(menu: &Menu, present_pts: u64) -> Result<Vec<Pes>> {
    if menu.buttons.len() > 255 {
        bail!("a menu page can have at most 255 buttons");
    }
    let bitmaps: Vec<&Bitmap> =
        menu.buttons.iter().flat_map(|b| [&b.normal, &b.selected, &b.activated]).collect();
    let q = palette::quantize(&bitmaps)?;

    let mut object_ids = Vec::new();
    let mut objects: Vec<(u16, &Bitmap, Vec<u8>)> = Vec::new();
    for (bi, _) in menu.buttons.iter().enumerate() {
        let mut ids = [0u16; 3];
        for si in 0..3 {
            let idx = bi * 3 + si;
            // Reuse identical bitmaps of the same button (e.g. activated == selected).
            ids[si] = match (0..si).find(|&p| bitmaps[bi * 3 + p] == bitmaps[idx]) {
                Some(p) => ids[p],
                None => {
                    let oid = objects.len() as u16;
                    let bmp = bitmaps[idx];
                    let rle = rle::encode(&q.indexes[idx], bmp.width as usize, bmp.height as usize);
                    objects.push((oid, bmp, rle));
                    oid
                }
            };
        }
        object_ids.push(ids);
    }

    let mut segments: Vec<(Vec<u8>, u64)> = Vec::new(); // (segment, decode duration)
    segments.push((pds(&q), 0));
    for (id, bmp, rle) in &objects {
        let frags = ods(*id, bmp.width, bmp.height, rle);
        let dur = decode_ticks(bmp.width as u64 * bmp.height as u64);
        let n = frags.len();
        for (i, f) in frags.into_iter().enumerate() {
            segments.push((f, if i == n - 1 { dur } else { 0 }));
        }
    }

    let total_decode: u64 = segments.iter().map(|s| s.1).sum();
    let (w, h) = menu.video.size();
    let plane_clear = decode_ticks(w as u64 * h as u64 / 2);
    let margin = 9_000; // 100 ms
    let start = present_pts.saturating_sub(total_decode + plane_clear + margin).max(margin);
    let present = present_pts.max(start + total_decode + plane_clear);

    let mut out = Vec::new();
    let pes = |seg: Vec<u8>, pts: u64, dts: u64| Pes {
        stream: 0,
        data: private_pes(&seg, pts, dts),
        pts: Some(pts),
        dts: Some(dts),
        random_access: false,
    };
    out.push(pes(ics(menu, &object_ids)?, present, start));
    let mut t = start;
    for (seg, dur) in segments {
        let dts = t;
        t += dur;
        out.push(pes(seg, t, dts));
    }
    out.push(pes(segment(SEG_END, &[]), t, t));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bluray::nav::hdmv::Command;

    fn solid(w: u16, h: u16, rgba: [u8; 4]) -> Bitmap {
        Bitmap { width: w, height: h, rgba: rgba.repeat(w as usize * h as usize) }
    }

    #[test]
    fn encodes_display_set() {
        let b = Button {
            id: 0,
            numeric: 1,
            x: 100,
            y: 100,
            up: 0,
            down: 0,
            left: 0,
            right: 0,
            normal: solid(200, 50, [255, 255, 255, 255]),
            selected: solid(200, 50, [255, 200, 0, 255]),
            activated: solid(200, 50, [255, 200, 0, 255]),
            commands: vec![Command::JumpTitle(1)],
            auto_action: false,
        };
        let menu = Menu { video: VideoFormat::P1080_23976, buttons: vec![b], default_button: 0 };
        let pes = encode(&menu, 90_000 * 2).unwrap();
        // ICS, PDS, 2 ODS (activated reuses selected), END
        assert_eq!(pes.len(), 5);
        assert_eq!(pes[0].data[9 + 10], SEG_ICS);
        assert!(pes.iter().all(|p| p.dts.unwrap() <= p.pts.unwrap()));
    }
}
