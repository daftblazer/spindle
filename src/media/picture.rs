// SPDX-License-Identifier: GPL-3.0-or-later

//! Fitting a title's picture into the disc frame: deinterlacing or inverse
//! telecine, cropping, aspect ratio, scaling, HDR tone mapping and colour
//! conversion, as an ffmpeg filter chain.

use super::probe::MediaInfo;
use crate::bluray::VideoFormat;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Deinterlace {
    /// Deinterlace interlaced video, unless the disc can keep it as it is.
    #[default]
    Auto,
    Off,
    /// Always deinterlace (for video not marked as interlaced).
    On,
    /// Restore film frames from telecined 29.97 video (NTSC film and
    /// anime DVDs), giving 23.976.
    InverseTelecine,
}

impl Deinterlace {
    pub const ALL: [Deinterlace; 4] = [Deinterlace::Auto, Deinterlace::Off, Deinterlace::On, Deinterlace::InverseTelecine];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AspectRatio {
    /// As the file says.
    #[default]
    Auto,
    /// 4:3
    Standard,
    /// 16:9
    Wide,
    /// 1.85:1
    Flat,
    /// 2.39:1
    Scope,
}

impl AspectRatio {
    pub const ALL: [AspectRatio; 5] = [AspectRatio::Auto, AspectRatio::Standard, AspectRatio::Wide, AspectRatio::Flat, AspectRatio::Scope];

    fn value(self) -> Option<f64> {
        match self {
            AspectRatio::Auto => None,
            AspectRatio::Standard => Some(4.0 / 3.0),
            AspectRatio::Wide => Some(16.0 / 9.0),
            AspectRatio::Flat => Some(1.85),
            AspectRatio::Scope => Some(2.39),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Fit {
    /// Show the whole picture, with black bars where it's narrower or wider
    /// than 16:9 (letterbox / pillarbox).
    #[default]
    Whole,
    /// Fill the screen, cutting off the edges that don't fit.
    Fill,
    /// Fill the screen by stretching the picture.
    Stretch,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Whole, Fit::Fill, Fit::Stretch];
}

/// How a title's video is prepared for the disc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VideoOptions {
    #[serde(default)]
    pub deinterlace: Deinterlace,
    /// Pixels to remove from the source: left, top, right, bottom.
    #[serde(default)]
    pub crop: [u32; 4],
    #[serde(default)]
    pub aspect: AspectRatio,
    #[serde(default)]
    pub fit: Fit,
}

/// Where the picture lands on the disc frame, in disc pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Disc pixels per square pixel horizontally (below 1 for the wide
    /// pixels of SD formats).
    pub squeeze: f64,
}

/// The filters for a title, and how the result is coded.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Filter chain, ending in 8-bit 4:2:0.
    pub filters: String,
    /// Kept interlaced: `Some(true)` top field first, `Some(false)` bottom
    /// field first. `None` for progressive pictures.
    pub interlaced: Option<bool>,
    pub area: Area,
    /// Film frames (23.976) turned into 29.97 interlaced video by 3:2
    /// pulldown (telecine).
    pub pulldown: bool,
    /// HDR video tone mapped to SDR.
    pub tone_mapped: bool,
    /// Deinterlaced or inverse telecined.
    pub deinterlaced: bool,
}

fn even(v: f64) -> u32 {
    ((v / 2.0).round() * 2.0).max(2.0) as u32
}

fn rate(fps: (u32, u32)) -> f64 {
    fps.0 as f64 / fps.1.max(1) as f64
}

/// Colour matrix of the disc format, for ffmpeg's scaler.
pub fn matrix(disc: VideoFormat) -> &'static str {
    if disc.is_sd() {
        "bt601"
    } else {
        "bt709"
    }
}

/// Size and position of the picture in the disc frame, and the scaled
/// size before cropping to the frame (for `Fit::Fill`).
fn geometry(aspect: f64, fit: Fit, disc: VideoFormat) -> ((u32, u32), Area) {
    let (w, h) = disc.size();
    let (sn, sd) = disc.sar();
    // Display width of the frame in square pixels, and disc pixels per
    // square pixel.
    let display_w = w as f64 * sn as f64 / sd as f64;
    let squeeze = w as f64 / display_w;
    let frame_aspect = display_w / h as f64;
    let (dw, dh) = match fit {
        Fit::Stretch => (display_w, h as f64),
        Fit::Whole if aspect >= frame_aspect => (display_w, display_w / aspect),
        Fit::Whole => (h as f64 * aspect, h as f64),
        Fit::Fill if aspect >= frame_aspect => (h as f64 * aspect, h as f64),
        Fit::Fill => (display_w, display_w / aspect),
    };
    let scaled = if fit == Fit::Stretch { (w, h) } else { (even(dw * squeeze), even(dh)) };
    let (vw, vh) = (scaled.0.min(w), scaled.1.min(h));
    let area = Area { x: (w - vw) / 2, y: (h - vh) / 2, w: vw, h: vh, squeeze };
    (scaled, area)
}

/// Filters that turn video with `info` into the picture of `disc`.
pub fn plan(info: &MediaInfo, opts: &VideoOptions, disc: VideoFormat) -> Plan {
    let (w, h) = disc.size();
    let src = (info.width.max(2), info.height.max(2));
    let [cl, ct, cr, cb] = opts.crop;
    let cropped = (src.0.saturating_sub(cl + cr).max(2), src.1.saturating_sub(ct + cb).max(2));
    let has_crop = cl + ct + cr + cb > 0;
    let (sn, sd) = info.sar.unwrap_or((1, 1));
    let aspect = opts.aspect.value().unwrap_or(cropped.0 as f64 * sn as f64 / sd.max(1) as f64 / cropped.1 as f64);
    let (scaled, area) = geometry(aspect, opts.fit, disc);

    let disc_rate = rate(disc.fps());
    let src_rate = info.fps.map_or(disc_rate, rate);
    let interlaced_src = info.interlaced() == Some(true);
    // The disc can carry interlaced video as it is when nothing moves or
    // scales it and the field rate matches.
    let untouched = !has_crop && scaled == (w, h) && src == (w, h);
    let keep = disc.interlaced() && (src_rate - disc_rate).abs() < 0.01 && untouched && !info.is_hdr();
    let mut f: Vec<String> = Vec::new();
    let mut interlaced = None;
    let mut deinterlaced = false;
    // Double the frame rate when the disc runs at the field rate (1080i50
    // on a 720p50 disc).
    let bwdif = |f: &mut Vec<String>| {
        let mode = if !disc.interlaced() && (disc_rate - 2.0 * src_rate).abs() < 0.1 { "send_field" } else { "send_frame" };
        f.push(format!("bwdif=mode={mode}:parity=auto:deint=all"));
    };
    match opts.deinterlace {
        Deinterlace::Off => {}
        Deinterlace::On => {
            bwdif(&mut f);
            deinterlaced = true;
        }
        Deinterlace::InverseTelecine => {
            f.push("fieldmatch=order=auto:combmatch=full,yadif=deint=interlaced,decimate".into());
            deinterlaced = true;
        }
        Deinterlace::Auto if interlaced_src && keep => interlaced = Some(!info.bottom_field_first()),
        Deinterlace::Auto if interlaced_src => {
            bwdif(&mut f);
            deinterlaced = true;
        }
        Deinterlace::Auto => {}
    }
    if has_crop {
        f.push(format!("crop={}:{}:{cl}:{ct}", cropped.0, cropped.1));
    }
    let tone_mapped = info.is_hdr();
    if tone_mapped {
        // To linear light, BT.2020 → BT.709 primaries, tone map, then back
        // to BT.709 video.
        f.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,\
             zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
                .into(),
        );
    }
    if interlaced.is_none() || !untouched {
        let flags = if interlaced.is_some() { ":interl=1" } else { "" };
        f.push(format!(
            "scale={}:{}:flags=lanczos{flags}:out_color_matrix={}:out_range=tv",
            scaled.0,
            scaled.1,
            matrix(disc)
        ));
        if opts.fit == Fit::Fill {
            f.push(format!("crop={}:{}", area.w, area.h));
        }
        if (area.w, area.h) != (w, h) {
            f.push(format!("pad={w}:{h}:{}:{}:black", area.x, area.y));
        }
    }
    let (n, d) = disc.sar();
    f.push(format!("setsar={n}/{d}"));
    // Film on a 29.97 interlaced disc: whole frames spread over fields
    // (3:2 pulldown), rather than repeated frames.
    let film = opts.deinterlace == Deinterlace::InverseTelecine || (!deinterlaced && (src_rate - 24000.0 / 1001.0).abs() < 0.01);
    let pulldown = interlaced.is_none() && disc.interlaced() && disc.fps() == (30000, 1001) && film;
    if pulldown {
        f.push("fps=24000/1001,telecine=first_field=top:pattern=23".into());
        interlaced = Some(true);
    } else if interlaced.is_none() {
        let (n, d) = disc.fps();
        f.push(format!("fps={n}/{d}"));
    }
    f.push("format=yuv420p".into());
    Plan { filters: f.join(","), interlaced, area, pulldown, tone_mapped, deinterlaced }
}

/// Filters for a picture already drawn at the disc's size (menus).
pub fn frame_filters(disc: VideoFormat) -> String {
    let (w, h) = disc.size();
    let (n, d) = disc.fps();
    let (sn, sd) = disc.sar();
    format!("scale={w}:{h}:flags=lanczos:out_color_matrix={}:out_range=tv,setsar={sn}/{sd},fps={n}/{d},format=yuv420p", matrix(disc))
}

/// Parse the last "crop=w:h:x:y" that ffmpeg's cropdetect printed.
fn last_crop(log: &str) -> Option<(u32, u32, u32, u32)> {
    let c = log.rsplit("crop=").next().filter(|_| log.contains("crop="))?;
    let v: Vec<u32> = c.split(|ch: char| !ch.is_ascii_digit()).filter(|x| !x.is_empty()).take(4).filter_map(|x| x.parse().ok()).collect();
    (v.len() == 4).then(|| (v[0], v[1], v[2], v[3]))
}

/// Find black bars around the picture by sampling frames through the
/// video; returns the crop (left, top, right, bottom) that keeps everything
/// seen in any sample.
pub fn detect_crop(path: &std::path::Path, info: &MediaInfo) -> anyhow::Result<[u32; 4]> {
    let (w, h) = (info.width, info.height);
    anyhow::ensure!(w > 0 && h > 0, "the video size is unknown");
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for k in 1..=7 {
        let at = info.duration * k as f64 / 8.0;
        let out = std::process::Command::new(super::ffmpeg_bin())
            .args(["-hide_banner", "-nostdin", "-ss", &format!("{at:.2}"), "-i"])
            .arg(path)
            .args(["-frames:v", "15", "-an", "-sn", "-vf", "cropdetect=limit=24:round=2:reset=0", "-f", "null", "-"])
            .output()?;
        let Some((cw, ch, cx, cy)) = last_crop(&String::from_utf8_lossy(&out.stderr)) else { continue };
        // A sample that is all black says nothing.
        if cw < w / 4 || ch < h / 4 {
            continue;
        }
        let b = bounds.get_or_insert((cx, cy, cx + cw, cy + ch));
        *b = (b.0.min(cx), b.1.min(cy), b.2.max(cx + cw), b.3.max(cy + ch));
    }
    let (x0, y0, x1, y1) = bounds.ok_or_else(|| anyhow::anyhow!("no picture found to measure"))?;
    // Bars of a few pixels are usually noise at the edges.
    let bar = |v: u32| if v >= 4 { v } else { 0 };
    Ok([bar(x0), bar(y0), bar(w.saturating_sub(x1)), bar(h.saturating_sub(y1))])
}

/// A short description of the source picture, e.g. "1920×1080 · 23.976
/// fps · interlaced · HDR".
pub fn describe(info: &MediaInfo) -> String {
    let mut parts = vec![format!("{}×{}", info.width, info.height)];
    if let Some((n, d)) = info.fps {
        let r = n as f64 / d.max(1) as f64;
        let r = format!("{r:.3}");
        parts.push(format!("{} fps", r.trim_end_matches('0').trim_end_matches('.')));
    }
    match info.interlaced() {
        Some(true) => parts.push(gettextrs::gettext("interlaced")),
        Some(false) => parts.push(gettextrs::gettext("progressive")),
        None => {}
    }
    if info.bit_depth() > 8 {
        parts.push(format!("{}-bit", info.bit_depth()));
    }
    if info.is_hdr() {
        parts.push(if info.color_transfer.as_deref() == Some("arib-std-b67") { "HLG".into() } else { "HDR10".into() });
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(w: u32, h: u32, fps: (u32, u32)) -> MediaInfo {
        MediaInfo { width: w, height: h, fps: Some(fps), video_codec: Some("h264".into()), ..Default::default() }
    }

    #[test]
    fn letterbox_and_pillarbox() {
        let p = plan(&info(1920, 800, (24000, 1001)), &VideoOptions::default(), VideoFormat::P1080_23976);
        assert_eq!(p.area, Area { x: 0, y: 140, w: 1920, h: 800, squeeze: 1.0 });
        assert!(p.filters.contains("scale=1920:800:"));
        assert!(p.filters.contains("pad=1920:1080:0:140:black"));
        // A 4:3 DVD (wide pixels 8:9) pillarboxed.
        let mut dvd = info(720, 480, (24000, 1001));
        dvd.sar = Some((8, 9));
        let p = plan(&dvd, &VideoOptions::default(), VideoFormat::P1080_23976);
        assert_eq!((p.area.w, p.area.h, p.area.x), (1440, 1080, 240));
        // Forced 16:9 fills the frame.
        let wide = VideoOptions { aspect: AspectRatio::Wide, ..Default::default() };
        assert_eq!(plan(&dvd, &wide, VideoFormat::P1080_23976).area.w, 1920);
    }

    #[test]
    fn fill_crops_the_edges() {
        let opts = VideoOptions { fit: Fit::Fill, ..Default::default() };
        let p = plan(&info(1440, 1080, (25, 1)), &opts, VideoFormat::P1080_24);
        assert!(p.filters.contains("scale=1920:1440:"), "{}", p.filters);
        assert!(p.filters.contains("crop=1920:1080"));
        assert!(!p.filters.contains("pad="));
    }

    #[test]
    fn crop_and_sd_squeeze() {
        let opts = VideoOptions { crop: [0, 138, 0, 138], ..Default::default() };
        let p = plan(&info(1920, 1080, (24000, 1001)), &opts, VideoFormat::P1080_23976);
        assert!(p.filters.starts_with("crop=1920:804:0:138"), "{}", p.filters);
        assert_eq!(p.area.h, 804);
        // 16:9 HD on an NTSC SD disc: wide pixels, 704 of the 720 wide
        // (the rest is the nominal overscan).
        let p = plan(&info(1920, 1080, (30000, 1001)), &VideoOptions::default(), VideoFormat::I480_2997);
        assert_eq!((p.area.w, p.area.h, p.area.x), (704, 480, 8));
        assert!(p.filters.contains("setsar=40/33"));
        assert!(p.filters.contains("out_color_matrix=bt601"));
        assert!(p.area.squeeze < 1.0);
    }

    #[test]
    fn standard_sd() {
        // A 4:3 DVD on a 4:3 NTSC disc fills the active picture.
        let mut dvd = info(708, 480, (24000, 1001));
        dvd.sar = Some((160, 177));
        let p = plan(&dvd, &VideoOptions::default(), VideoFormat::I480_2997_4x3);
        assert_eq!((p.area.w, p.area.h), (704, 480));
        assert!(p.filters.contains("setsar=10/11"));
        assert!(p.pulldown);
        // 16:9 video is letterboxed on it.
        let p = plan(&info(1920, 1080, (25, 1)), &VideoOptions::default(), VideoFormat::I576_25_4x3);
        assert_eq!((p.area.w, p.area.h), (720, 442));
    }

    #[test]
    fn interlaced_sources() {
        let mut i = info(1920, 1080, (25, 1));
        i.field_order = Some("tt".into());
        // Same format: kept as fields.
        let p = plan(&i, &VideoOptions::default(), VideoFormat::I1080_25);
        assert_eq!(p.interlaced, Some(true));
        assert!(!p.filters.contains("bwdif") && !p.filters.contains("fps="));
        // A 720p50 disc gets one frame per field.
        let p = plan(&i, &VideoOptions::default(), VideoFormat::P720_50);
        assert_eq!(p.interlaced, None);
        assert!(p.filters.contains("bwdif=mode=send_field"));
        // A 1080p24 disc: deinterlaced frame by frame.
        let p = plan(&i, &VideoOptions::default(), VideoFormat::P1080_24);
        assert!(p.filters.contains("bwdif=mode=send_frame"));
        // Turned off: left alone.
        let off = VideoOptions { deinterlace: Deinterlace::Off, ..Default::default() };
        assert!(!plan(&i, &off, VideoFormat::P1080_24).filters.contains("bwdif"));
        let ivtc = VideoOptions { deinterlace: Deinterlace::InverseTelecine, ..Default::default() };
        assert!(plan(&info(720, 480, (30000, 1001)), &ivtc, VideoFormat::P1080_23976).filters.starts_with("fieldmatch"));
        // Film on a 29.97 interlaced disc: pulldown, not repeated frames.
        let p = plan(&info(720, 480, (30000, 1001)), &ivtc, VideoFormat::I480_2997);
        assert!(p.pulldown && p.filters.contains("telecine=first_field=top"));
        assert_eq!(p.interlaced, Some(true));
        let p = plan(&info(1920, 1080, (24000, 1001)), &VideoOptions::default(), VideoFormat::I1080_2997);
        assert!(p.pulldown);
        assert!(!plan(&info(1920, 1080, (24000, 1001)), &VideoOptions::default(), VideoFormat::I1080_25).pulldown);
    }

    #[test]
    fn parses_cropdetect() {
        let log = "[Parsed_cropdetect_0 @ 0x1] x1:0 x2:1919 y1:140 y2:939 w:1920 h:800 x:0 y:140 pts:1 t:0.04 limit:0.09 crop=1920:800:0:140\n\
                   [Parsed_cropdetect_0 @ 0x1] x1:0 x2:1919 y1:138 y2:941 w:1920 h:804 x:0 y:138 pts:2 t:0.08 limit:0.09 crop=1920:804:0:138\n";
        assert_eq!(last_crop(log), Some((1920, 804, 0, 138)));
        assert_eq!(last_crop("nothing"), None);
    }

    #[test]
    fn hdr_is_tone_mapped() {
        let mut i = info(3840, 2160, (24000, 1001));
        i.color_transfer = Some("smpte2084".into());
        i.pix_fmt = Some("yuv420p10le".into());
        let p = plan(&i, &VideoOptions::default(), VideoFormat::P1080_23976);
        assert!(p.tone_mapped);
        assert!(p.filters.contains("tonemap=tonemap=hable"));
        assert!(p.filters.ends_with("format=yuv420p"));
    }
}
