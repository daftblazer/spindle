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
pub struct Page {
    /// Drawn in order; a static decoration layer can be a button that no
    /// other button navigates to.
    pub buttons: Vec<Button>,
    /// Button selected when the page appears; [`NO_BUTTON`] keeps the
    /// player's last selection (PSR10) or falls back to the first button.
    pub default_button: u16,
    /// Fade the buttons in when the page appears / out when it goes (90 kHz
    /// ticks, 0 = no effect).
    pub fade_in: u32,
    pub fade_out: u32,
}

/// Most steps of a fade effect (each with its own palette).
const FADE_STEPS: usize = 5;
/// Palettes an IG stream may use (ids 0–7): the pages' palettes and the
/// fades' share them.
const MAX_PALETTES: usize = 8;

/// The palettes of a page: its own, and those of its fade steps.
#[derive(Debug, Clone, Default)]
struct PagePalettes {
    id: u8,
    /// Palette ids of the fade steps, faintest first (empty: no fade).
    fade: Vec<u8>,
}

/// Palette ids for `pages` pages, of which those in `fading` fade: one
/// palette per page when they fit (else one for all), and as many fade steps
/// as the rest of the eight allow (none when fewer than two).
fn plan_palettes(pages: usize, fading: &[bool]) -> Vec<PagePalettes> {
    let own = pages <= MAX_PALETTES;
    let used = if own { pages } else { 1 };
    let faded: Vec<usize> = (0..pages).filter(|&p| fading.get(p).copied().unwrap_or(false)).collect();
    // With a shared palette, the fade steps are shared too.
    let groups = if own { faded.len() } else { usize::from(!faded.is_empty()) };
    let steps = (MAX_PALETTES - used).checked_div(groups).unwrap_or(0).min(FADE_STEPS);
    let steps = if steps >= 2 { steps } else { 0 };
    (0..pages)
        .map(|p| {
            let id = if own { p as u8 } else { 0 };
            let group = if own { faded.iter().position(|&f| f == p) } else { faded.contains(&p).then_some(0) };
            let fade = match group {
                Some(g) if steps > 0 => (0..steps).map(|k| (used + g * steps + k) as u8).collect(),
                _ => vec![],
            };
            PagePalettes { id, fade }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub video: VideoFormat,
    pub pages: Vec<Page>,
    /// Pop-up menu shown over the movie with the remote's Pop-up key,
    /// rather than an always-on menu.
    pub popup: bool,
    /// Hide the pop-up after this long without input (90 kHz ticks, 0 = never).
    pub user_timeout: u32,
}

impl Menu {
    /// A single always-on page.
    pub fn single(video: VideoFormat, buttons: Vec<Button>, default_button: u16) -> Self {
        Menu { video, pages: vec![Page { buttons, default_button, fade_in: 0, fade_out: 0 }], popup: false, user_timeout: 0 }
    }
}


pub(crate) fn segment(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 3);
    v.push(kind);
    v.extend_from_slice(&(body.len() as u16).to_be_bytes());
    v.extend_from_slice(body);
    v
}

pub(crate) fn pds(q: &Quantized) -> Vec<u8> {
    pds_with_id(q, 0)
}

fn pds_with_id(q: &Quantized, id: u8) -> Vec<u8> {
    pds_scaled(q, id, 1.0)
}

/// A palette with every entry's opacity scaled by `alpha` (for fades).
fn pds_scaled(q: &Quantized, id: u8, alpha: f32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u8(id).u8(0); // palette id, version
    for (i, e) in q.palette_ycrcb().iter().enumerate() {
        w.u8(i as u8).u8(e[0]).u8(e[1]).u8(e[2]).u8((e[3] as f32 * alpha).round() as u8);
    }
    segment(SEG_PDS, &w.into_bytes())
}

/// An in or out effect sequence: the page's buttons (normal state) drawn
/// with palettes of rising (or falling) opacity.
fn effect_sequence(w: &mut BitWriter, page: &Page, ids: &[[u16; 3]], fade: &[u8], ticks: u32, fade_in: bool) {
    let usable = ticks > 0 && !page.buttons.is_empty() && !fade.is_empty();
    if !usable {
        w.u8(0).u8(0); // no windows, no effects
        return;
    }
    let x0 = page.buttons.iter().map(|b| b.x).min().unwrap_or(0);
    let y0 = page.buttons.iter().map(|b| b.y).min().unwrap_or(0);
    let x1 = page.buttons.iter().map(|b| b.x + b.normal.width).max().unwrap_or(0);
    let y1 = page.buttons.iter().map(|b| b.y + b.normal.height).max().unwrap_or(0);
    w.u8(1).u8(0).u16(x0).u16(y0).u16(x1 - x0).u16(y1 - y0);
    let steps = fade.len();
    w.u8(steps as u8);
    for k in 0..steps {
        let step = if fade_in { k } else { steps - 1 - k };
        w.u24(ticks / steps as u32).u8(fade[step]);
        w.u8(page.buttons.len() as u8);
        for (b, objs) in page.buttons.iter().zip(ids) {
            w.u16(objs[0]).u8(0).u8(0).u16(b.x).u16(b.y);
        }
    }
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

fn ics(menu: &Menu, object_ids: &[Vec<[u16; 3]>], palettes: &[PagePalettes]) -> Result<Vec<u8>> {
    let (width, height) = menu.video.size();

    let mut comp = BitWriter::new();
    comp.flag(false) // stream_model: multiplexed
        .flag(menu.popup) // ui_model
        .zeros(6);
    comp.zeros(7).bits(33, 0); // composition_time_out_pts
    comp.zeros(7).bits(33, 0); // selection_time_out_pts
    comp.u24(menu.user_timeout);
    comp.u8(menu.pages.len() as u8);

    for (pi, ((page, ids), pal)) in menu.pages.iter().zip(object_ids).zip(palettes).enumerate() {
        comp.u8(pi as u8).u8(0);
        comp.bytes(&[0; 8]); // UO mask
        effect_sequence(&mut comp, page, ids, &pal.fade, page.fade_in, true);
        effect_sequence(&mut comp, page, ids, &pal.fade, page.fade_out, false);
        comp.u8(0); // animation frame rate code
        comp.u16(page.default_button).u16(NO_BUTTON);
        comp.u8(pal.id); // palette id
        comp.u8(page.buttons.len() as u8); // one BOG per button
        for (b, objs) in page.buttons.iter().zip(ids) {
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

/// Encoded segments of a menu, ready to be timed.
pub struct DisplaySet {
    /// (segment, decode duration in 90 kHz ticks), excluding the ICS.
    segments: Vec<(Vec<u8>, u64)>,
    object_ids: Vec<Vec<[u16; 3]>>,
    palettes: Vec<PagePalettes>,
    decode: u64,
    plane_clear: u64,
}

/// Quantize and compress `menu`'s bitmaps (one palette per page).
pub fn prepare(menu: &Menu) -> Result<DisplaySet> {
    if menu.pages.is_empty() || menu.pages.len() > 255 {
        bail!("a menu needs 1–255 pages");
    }
    let mut segments: Vec<(Vec<u8>, u64)> = Vec::new();
    let mut object_ids = Vec::new();
    let mut objects: Vec<(u16, &Bitmap, Vec<u8>)> = Vec::new();
    if let Some(page) = menu.pages.iter().find(|p| p.buttons.len() > 255) {
        bail!("a menu page can have at most 255 buttons ({})", page.buttons.len());
    }
    let fading: Vec<bool> = menu.pages.iter().map(|p| p.fade_in > 0 || p.fade_out > 0).collect();
    let palettes = plan_palettes(menu.pages.len(), &fading);
    let page_bitmaps: Vec<Vec<&Bitmap>> =
        menu.pages.iter().map(|page| page.buttons.iter().flat_map(|b| [&b.normal, &b.selected, &b.activated]).collect()).collect();
    // One palette per page, or one for all when there are more pages than
    // palettes allowed.
    let shared = menu.pages.len() > MAX_PALETTES;
    let mut quantized: Vec<Option<palette::Quantized>> = Vec::new();
    if shared {
        let all: Vec<&Bitmap> = page_bitmaps.iter().flatten().copied().collect();
        let q = palette::quantize(&all)?;
        let mut rest = q.indexes.clone().into_iter();
        for bitmaps in &page_bitmaps {
            let indexes: Vec<Vec<u8>> = rest.by_ref().take(bitmaps.len()).collect();
            quantized.push(Some(palette::Quantized { palette: q.palette.clone(), indexes }));
        }
    } else {
        for bitmaps in &page_bitmaps {
            quantized.push(if bitmaps.is_empty() { None } else { Some(palette::quantize(bitmaps)?) });
        }
    }
    let mut written = std::collections::HashSet::new();
    for (pi, (page, bitmaps)) in menu.pages.iter().zip(&page_bitmaps).enumerate() {
        let Some(q) = &quantized[pi] else {
            object_ids.push(Vec::new());
            continue;
        };
        let pal = &palettes[pi];
        // Each palette once (pages sharing one write it once).
        if written.insert(pal.id) {
            segments.push((pds_with_id(q, pal.id), 0));
            for (k, id) in pal.fade.iter().enumerate() {
                if written.insert(*id) {
                    segments.push((pds_scaled(q, *id, (k + 1) as f32 / (pal.fade.len() + 1) as f32), 0));
                }
            }
        }
        let mut page_ids = Vec::new();
        for bi in 0..page.buttons.len() {
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
            page_ids.push(ids);
        }
        object_ids.push(page_ids);
    }
    for (id, bmp, rle) in &objects {
        let frags = ods(*id, bmp.width, bmp.height, rle);
        let dur = decode_ticks(bmp.width as u64 * bmp.height as u64);
        let n = frags.len();
        for (i, f) in frags.into_iter().enumerate() {
            segments.push((f, if i == n - 1 { dur } else { 0 }));
        }
    }
    let decode = segments.iter().map(|s| s.1).sum();
    let (w, h) = menu.video.size();
    Ok(DisplaySet { segments, object_ids, palettes, decode, plane_clear: decode_ticks(w as u64 * h as u64 / 2) })
}

impl DisplaySet {
    /// PES packets (in decode order) of a display set valid at (or
    /// shortly after) `present_pts`.
    pub fn packets(&self, menu: &Menu, present_pts: u64) -> Result<Vec<Pes>> {
        let margin = 9_000; // 100 ms
        let start = present_pts.saturating_sub(self.decode + self.plane_clear + margin).max(margin);
        let present = present_pts.max(start + self.decode + self.plane_clear);
        let pes = |seg: &[u8], pts: u64, dts: u64| Pes {
            stream: 0,
            data: private_pes(seg, pts, dts),
            pts: Some(pts),
            dts: Some(dts),
            random_access: false,
        };
        let mut out = vec![pes(&ics(menu, &self.object_ids, &self.palettes)?, present, start)];
        let mut t = start;
        for (seg, dur) in &self.segments {
            let dts = t;
            t += dur;
            out.push(pes(seg, t, dts));
        }
        out.push(pes(&segment(SEG_END, &[]), t, t));
        Ok(out)
    }
}

/// Encode `menu` as a display set whose composition becomes valid at (or
/// shortly after) `present_pts`. Returns the PES packets in decode order.
pub fn encode(menu: &Menu, present_pts: u64) -> Result<Vec<Pes>> {
    prepare(menu)?.packets(menu, present_pts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_stay_within_eight() {
        // A single page with a fade: its palette and five fade steps.
        let p = plan_palettes(1, &[true]);
        assert_eq!((p[0].id, p[0].fade.clone()), (0, vec![1, 2, 3, 4, 5]));
        // Three pop-up pages that fade: 3 own palettes, 5 left, one step each is too few.
        let p = plan_palettes(3, &[true, true, true]);
        assert!(p.iter().all(|pp| pp.fade.is_empty()));
        assert_eq!(p.iter().map(|pp| pp.id).collect::<Vec<_>>(), vec![0, 1, 2]);
        // Two pages, only the first fading: two palettes, then steps 2–6.
        let p = plan_palettes(2, &[true, false]);
        assert_eq!(p[0].fade, vec![2, 3, 4, 5, 6]);
        assert!(p[1].fade.is_empty());
        // More pages than palettes: one shared palette, and shared fade steps.
        let p = plan_palettes(12, &[true; 12]);
        assert!(p.iter().all(|pp| pp.id == 0 && pp.fade == vec![1, 2, 3, 4, 5]));
        for pages in 1..20 {
            for pp in plan_palettes(pages, &vec![true; pages]) {
                assert!(pp.id < 8 && pp.fade.iter().all(|f| *f < 8), "{pages} pages");
            }
        }
    }
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
        let menu = Menu::single(VideoFormat::P1080_23976, vec![b], 0);
        let pes = encode(&menu, 90_000 * 2).unwrap();
        // ICS, PDS, 2 ODS (activated reuses selected), END
        assert_eq!(pes.len(), 5);
        assert_eq!(pes[0].data[9 + 10], SEG_ICS);
        assert!(pes.iter().all(|p| p.dts.unwrap() <= p.pts.unwrap()));
    }
}
