// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal libass bindings and a renderer turning an ASS script into timed
//! RGBA images.

use super::SubImage;
use crate::bluray::ig::Bitmap;
use crate::model::{Rgba, SubtitleStyle};
use anyhow::{bail, Result};
use std::ffi::{c_char, c_int, c_longlong, c_void, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
struct AssImage {
    w: c_int,
    h: c_int,
    stride: c_int,
    bitmap: *const u8,
    color: u32,
    dst_x: c_int,
    dst_y: c_int,
    next: *const AssImage,
    kind: c_int,
}

#[repr(C)]
struct AssEvent {
    start: c_longlong,
    duration: c_longlong,
    read_order: c_int,
    layer: c_int,
    style: c_int,
    name: *const c_char,
    margin_l: c_int,
    margin_r: c_int,
    margin_v: c_int,
    effect: *const c_char,
    text: *const c_char,
    render_priv: *mut c_void,
}

/// Leading fields of `ASS_Track` (stable across libass releases).
#[repr(C)]
struct AssTrackHead {
    n_styles: c_int,
    max_styles: c_int,
    n_events: c_int,
    max_events: c_int,
    styles: *mut c_void,
    events: *const AssEvent,
}

type Library = c_void;
/// `void (*)(int level, const char *fmt, va_list args, void *data)`; the
/// va_list is passed as a pointer on the supported ABIs and ignored here.
type MessageCb = unsafe extern "C" fn(c_int, *const c_char, *mut c_void, *mut c_void);

unsafe extern "C" fn on_message(level: c_int, fmt: *const c_char, _args: *mut c_void, _data: *mut c_void) {
    // Only errors are interesting; the format arguments are not expanded.
    if level <= 1 && !fmt.is_null() {
        log::warn!("libass: {}", unsafe { std::ffi::CStr::from_ptr(fmt) }.to_string_lossy().trim());
    }
}
type Renderer = c_void;
type Track = c_void;

extern "C" {
    fn ass_library_init() -> *mut Library;
    fn ass_library_done(lib: *mut Library);
    fn ass_set_extract_fonts(lib: *mut Library, extract: c_int);
    fn ass_set_message_cb(lib: *mut Library, cb: Option<MessageCb>, data: *mut c_void);
    fn ass_set_style_overrides(lib: *mut Library, list: *const *const c_char);
    fn ass_renderer_init(lib: *mut Library) -> *mut Renderer;
    fn ass_renderer_done(r: *mut Renderer);
    fn ass_set_frame_size(r: *mut Renderer, w: c_int, h: c_int);
    fn ass_set_storage_size(r: *mut Renderer, w: c_int, h: c_int);
    fn ass_set_fonts(r: *mut Renderer, default_font: *const c_char, default_family: *const c_char, dfp: c_int, config: *const c_char, update: c_int);
    fn ass_read_file(lib: *mut Library, fname: *const c_char, codepage: *const c_char) -> *mut Track;
    fn ass_process_force_style(track: *mut Track);
    fn ass_free_track(track: *mut Track);
    fn ass_render_frame(r: *mut Renderer, track: *mut Track, now: c_longlong, detect_change: *mut c_int) -> *const AssImage;
}

/// ASS colour "&HAABBGGRR" (alpha 00 = opaque).
fn ass_color(c: Rgba) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    format!("&H{:02X}{:02X}{:02X}{:02X}", 255 - b(c.a), b(c.b), b(c.g), b(c.r))
}

/// Style overrides for the "Default" style that ffmpeg gives converted
/// SRT/WebVTT subtitles (PlayResY 288).
pub fn style_overrides(style: &SubtitleStyle) -> Vec<String> {
    let k = 288.0 / 1080.0;
    vec![
        format!("Default.FontName={}", style.font),
        format!("Default.FontSize={:.2}", style.size * k),
        format!("Default.Bold={}", if style.bold { -1 } else { 0 }),
        format!("Default.PrimaryColour={}", ass_color(style.color)),
        format!("Default.OutlineColour={}", ass_color(style.outline_color)),
        format!("Default.BackColour={}", ass_color(Rgba::new(0.0, 0.0, 0.0, 0.6))),
        format!("Default.Outline={:.2}", style.outline * k),
        format!("Default.Shadow={:.2}", style.shadow * k),
        format!("Default.MarginV={}", (style.margin * k).round() as i32),
        "Default.BorderStyle=1".into(),
        "Default.Alignment=2".into(),
    ]
}

pub struct RenderOptions<'a> {
    /// Size of the video picture on the disc and its top-left offset.
    pub frame: (u32, u32),
    pub offset: (u32, u32),
    /// Source video size (for correct aspect of `\\pos` etc.).
    pub storage: (u32, u32),
    /// Style overrides applied to the Default style, if any.
    pub style: Option<&'a SubtitleStyle>,
    /// Only render this (start, end) range in ms on the script's clock.
    pub window: Option<(i64, i64)>,
}

struct Handles {
    lib: *mut Library,
    renderer: *mut Renderer,
    track: *mut Track,
}

impl Drop for Handles {
    fn drop(&mut self) {
        unsafe {
            if !self.track.is_null() {
                ass_free_track(self.track);
            }
            if !self.renderer.is_null() {
                ass_renderer_done(self.renderer);
            }
            if !self.lib.is_null() {
                ass_library_done(self.lib);
            }
        }
    }
}

/// Composite the libass image list into one RGBA bitmap cropped to the
/// painted area. Returns (x, y, bitmap) in frame coordinates.
fn composite(mut img: *const AssImage, frame: (u32, u32)) -> Option<(u32, u32, Bitmap)> {
    // Bounding box first.
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut p = img;
    while !p.is_null() {
        let i = unsafe { &*p };
        if i.w > 0 && i.h > 0 && (i.color & 0xFF) != 0xFF {
            x0 = x0.min(i.dst_x);
            y0 = y0.min(i.dst_y);
            x1 = x1.max(i.dst_x + i.w);
            y1 = y1.max(i.dst_y + i.h);
        }
        p = i.next;
    }
    let (fw, fh) = (frame.0 as i32, frame.1 as i32);
    x0 = x0.max(0);
    y0 = y0.max(0);
    x1 = x1.min(fw);
    y1 = y1.min(fh);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    // Premultiplied float accumulation, "over" operator in list order.
    let mut acc = vec![[0f32; 4]; w * h];
    while !img.is_null() {
        let i = unsafe { &*img };
        let r = ((i.color >> 24) & 0xFF) as f32 / 255.0;
        let g = ((i.color >> 16) & 0xFF) as f32 / 255.0;
        let b = ((i.color >> 8) & 0xFF) as f32 / 255.0;
        let a = (255 - (i.color & 0xFF)) as f32 / 255.0;
        for yy in 0..i.h {
            let fy = i.dst_y + yy;
            if fy < y0 || fy >= y1 {
                continue;
            }
            let row = unsafe { std::slice::from_raw_parts(i.bitmap.add((yy * i.stride) as usize), i.w as usize) };
            for (xx, &cov) in row.iter().enumerate() {
                let fx = i.dst_x + xx as i32;
                if cov == 0 || fx < x0 || fx >= x1 {
                    continue;
                }
                let alpha = a * cov as f32 / 255.0;
                let px = &mut acc[(fy - y0) as usize * w + (fx - x0) as usize];
                px[0] = r * alpha + px[0] * (1.0 - alpha);
                px[1] = g * alpha + px[1] * (1.0 - alpha);
                px[2] = b * alpha + px[2] * (1.0 - alpha);
                px[3] = alpha + px[3] * (1.0 - alpha);
            }
        }
        img = i.next;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in acc {
        let a = px[3];
        let un = |c: f32| if a > 0.0 { (c / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        rgba.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), (a * 255.0).round() as u8]);
    }
    Some((x0 as u32, y0 as u32, Bitmap { width: w as u16, height: h as u16, rgba }))
}

/// Render every change of an ASS script into timed images. Times are in
/// seconds on the script's clock.
pub fn render(path: &Path, opts: &RenderOptions, cancel: &AtomicBool) -> Result<Vec<SubImage>> {
    let cpath = CString::new(path.to_string_lossy().as_bytes())?;
    let overrides: Vec<CString> = opts
        .style
        .map(style_overrides)
        .unwrap_or_default()
        .into_iter()
        .map(|s| CString::new(s).unwrap())
        .collect();
    let mut override_ptrs: Vec<*const c_char> = overrides.iter().map(|s| s.as_ptr()).collect();
    override_ptrs.push(std::ptr::null());

    let h = unsafe {
        let lib = ass_library_init();
        if lib.is_null() {
            bail!("could not initialise libass");
        }
        let mut h = Handles { lib, renderer: std::ptr::null_mut(), track: std::ptr::null_mut() };
        ass_set_message_cb(lib, Some(on_message), std::ptr::null_mut());
        ass_set_extract_fonts(lib, 1);
        if opts.style.is_some() {
            ass_set_style_overrides(lib, override_ptrs.as_ptr());
        }
        h.renderer = ass_renderer_init(lib);
        if h.renderer.is_null() {
            bail!("could not initialise the libass renderer");
        }
        ass_set_frame_size(h.renderer, opts.frame.0 as c_int, opts.frame.1 as c_int);
        ass_set_storage_size(h.renderer, opts.storage.0 as c_int, opts.storage.1 as c_int);
        let family = CString::new("Cantarell").unwrap();
        // 1 = ASS_FONTPROVIDER_AUTODETECT (fontconfig on Linux)
        ass_set_fonts(h.renderer, std::ptr::null(), family.as_ptr(), 1, std::ptr::null(), 1);
        h.track = ass_read_file(lib, cpath.as_ptr(), std::ptr::null());
        if h.track.is_null() {
            bail!("could not read subtitles from {}", path.display());
        }
        if opts.style.is_some() {
            ass_process_force_style(h.track);
        }
        h
    };

    // Event boundaries in ms.
    let head = unsafe { &*(h.track as *const AssTrackHead) };
    let events = if head.n_events > 0 && !head.events.is_null() {
        unsafe { std::slice::from_raw_parts(head.events, head.n_events as usize) }
    } else {
        &[]
    };
    let mut bounds: Vec<i64> = events.iter().flat_map(|e| [e.start, e.start + e.duration]).collect();
    bounds.sort_unstable();
    bounds.dedup();

    let mut out: Vec<SubImage> = Vec::new();
    let mut current: Option<SubImage> = None;
    let mut last_hash: Option<u64> = None;
    let close = |cur: &mut Option<SubImage>, out: &mut Vec<SubImage>, at: f64| {
        if let Some(mut c) = cur.take() {
            c.end = at;
            if c.end > c.start {
                out.push(c);
            }
        }
    };
    const STEP_MS: i64 = 200;
    for win in bounds.windows(2) {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if let Some((from, to)) = opts.window {
            if win[1] <= from || win[0] >= to {
                continue;
            }
        }
        let (a, b) = (win[0], win[1]);
        let mut t = a;
        let mut first = true;
        while t < b {
            let mut change: c_int = 0;
            let img = unsafe { ass_render_frame(h.renderer, h.track, t, &mut change) };
            if first || change != 0 {
                let secs = t as f64 / 1000.0;
                let frame = composite(img, opts.frame);
                let hash = frame.as_ref().map(|(x, y, bmp)| {
                    use std::hash::{Hash, Hasher};
                    let mut hs = std::collections::hash_map::DefaultHasher::new();
                    (x, y, bmp.width, bmp.height, &bmp.rgba).hash(&mut hs);
                    hs.finish()
                });
                if hash != last_hash {
                    close(&mut current, &mut out, secs);
                    current = frame.map(|(x, y, bitmap)| SubImage {
                        start: secs,
                        end: secs,
                        x: x + opts.offset.0,
                        y: y + opts.offset.1,
                        bitmap,
                        forced: false,
                    });
                    last_hash = hash;
                }
            }
            first = false;
            t += STEP_MS;
        }
    }
    if let Some(&end) = bounds.last() {
        close(&mut current, &mut out, end as f64 / 1000.0);
    }
    Ok(out)
}

/// Render one sample line in `style` for previews (`width`×`height` frame).
pub fn preview(style: &SubtitleStyle, text: &str, width: u32, height: u32) -> Result<Option<SubImage>> {
    let dir = std::env::temp_dir().join(format!("spindle-preview-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("preview.ass");
    // Same header ffmpeg writes for converted SRT, so overrides match.
    std::fs::write(
        &file,
        format!(
            "[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\nScaledBorderAndShadow: yes\n\n[V4+ Styles]\n\
             Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
             Style: Default,Arial,16,&Hffffff,&Hffffff,&H0,&H0,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,0\n\n[Events]\n\
             Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{}\n",
            text.replace('\n', "\\N")
        ),
    )?;
    let opts = RenderOptions { frame: (width, height), offset: (0, 0), storage: (width, height), style: Some(style), window: None };
    let imgs = render(&file, &opts, &AtomicBool::new(false))?;
    let _ = std::fs::remove_file(&file);
    Ok(imgs.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_srt_style_script() {
        let dir = std::env::temp_dir().join(format!("spindle-ass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.ass");
        std::fs::write(
            &file,
            "[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\n\n[V4+ Styles]\n\
             Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
             Style: Default,Arial,16,&Hffffff,&Hffffff,&H0,&H0,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,0\n\n[Events]\n\
             Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello world\n\
             Dialogue: 0,0:00:02.50,0:00:04.00,Default,,0,0,0,,Second line\n",
        )
        .unwrap();
        let style = SubtitleStyle::default();
        let opts = RenderOptions { frame: (1920, 1080), offset: (0, 0), storage: (1920, 1080), style: Some(&style), window: None };
        let imgs = render(&file, &opts, &AtomicBool::new(false)).unwrap();
        assert_eq!(imgs.len(), 2, "{:?}", imgs.iter().map(|i| (i.start, i.end)).collect::<Vec<_>>());
        assert_eq!((imgs[0].start, imgs[0].end), (1.0, 2.5));
        assert_eq!((imgs[1].start, imgs[1].end), (2.5, 4.0));
        // Bottom-centred text.
        let i = &imgs[0];
        assert!(i.y > 900 && i.y < 1080, "y = {}", i.y);
        assert!(i.x > 500 && i.x + (i.bitmap.width as u32) < 1420);
        assert!(i.bitmap.rgba.chunks(4).any(|p| p[3] == 255 && p[0] == 255));
        let _ = std::fs::remove_dir_all(dir);
    }
}
