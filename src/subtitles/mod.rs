// SPDX-License-Identifier: GPL-3.0-or-later

//! Subtitle conversion to Blu-ray PGS.
//!
//! Every source ends up as a list of [`SubImage`]s (timed RGBA bitmaps in
//! disc coordinates) which [`pgs::encode`] turns into PG display sets:
//! * text (SRT, ASS/SSA, WebVTT, MP4 text) is rendered with libass, keeping
//!   ASS styling and positioning;
//! * PGS (embedded or `.sup`) is decoded and re-encoded, rescaled when the
//!   disc resolution differs from the source.

pub mod ass;
pub mod pgs;

use crate::bluray::ig::Bitmap;
use crate::bluray::VideoFormat;
use crate::media::ffmpeg;
use crate::media::picture::{self, Area};
use crate::media::probe::MediaInfo;
use crate::model::{SubtitleKind, SubtitleSource, SubtitleStyle, SubtitleTrack};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// A subtitle bitmap shown from `start` to `end` (seconds on the video's
/// timeline, 0 = first frame) at (`x`, `y`) on the disc picture.
#[derive(Debug, Clone)]
pub struct SubImage {
    pub start: f64,
    pub end: f64,
    pub x: u32,
    pub y: u32,
    pub bitmap: Bitmap,
    pub forced: bool,
}

/// Where the video picture is on the disc frame when nothing is changed
/// (kept video, or a whole picture fitted in).
pub fn default_area(info: &MediaInfo, disc: VideoFormat) -> Area {
    picture::plan(info, &Default::default(), disc).area
}

/// Squeeze images rendered with square pixels onto the disc's pixels.
fn squeeze(img: &mut SubImage, k: f64) {
    if (k - 1.0).abs() < 1e-3 {
        return;
    }
    let w = ((img.bitmap.width as f64 * k).round() as u16).max(1);
    img.bitmap = resize(&img.bitmap, w, img.bitmap.height);
    img.x = (img.x as f64 * k).round() as u32;
}

/// Bilinear resize of a straight-alpha RGBA bitmap (premultiplied internally).
fn resize(b: &Bitmap, w: u16, h: u16) -> Bitmap {
    let (sw, sh) = (b.width as usize, b.height as usize);
    let px = |x: usize, y: usize| {
        let i = (y.min(sh - 1) * sw + x.min(sw - 1)) * 4;
        let a = b.rgba[i + 3] as f32 / 255.0;
        [b.rgba[i] as f32 * a, b.rgba[i + 1] as f32 * a, b.rgba[i + 2] as f32 * a, a]
    };
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let fy = ((y as f32 + 0.5) * sh as f32 / h as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy as usize, fy.fract());
        for x in 0..w as usize {
            let fx = ((x as f32 + 0.5) * sw as f32 / w as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx as usize, fx.fract());
            let (a, b2, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
            let mut o = [0f32; 4];
            for k in 0..4 {
                o[k] = (a[k] * (1.0 - tx) + b2[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
            }
            let al = o[3];
            let un = |v: f32| if al > 0.0 { (v / al).round().clamp(0.0, 255.0) as u8 } else { 0 };
            rgba.extend_from_slice(&[un(o[0]), un(o[1]), un(o[2]), (al * 255.0).round() as u8]);
        }
    }
    Bitmap { width: w, height: h, rgba }
}

fn extract(video: &Path, index: usize, codec: &str, out: &Path, cancel: &AtomicBool) -> Result<()> {
    let copy = matches!(codec, "ass" | "ssa" | "hdmv_pgs_subtitle");
    let args: Vec<String> = [
        "-copyts",
        "-i",
        &video.to_string_lossy(),
        "-map",
        &format!("0:s:{index}"),
        "-c:s",
        if copy { "copy" } else { "ass" },
        &out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    ffmpeg::run(&args, cancel, |_| {}).context("could not extract the subtitle stream")
}

fn convert_text(input: &Path, out: &Path, cancel: &AtomicBool) -> Result<()> {
    let args: Vec<String> = ["-i", &input.to_string_lossy(), "-c:s", "ass", &out.to_string_lossy()].iter().map(|s| s.to_string()).collect();
    ffmpeg::run(&args, cancel, |_| {}).context("could not read the subtitle file")
}

/// Produce the images of one track for a title whose video has `info` and
/// is shown in `area` of the disc picture. `work` is a scratch directory.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    track: &SubtitleTrack,
    video_path: &Path,
    info: &MediaInfo,
    area: &Area,
    style: &SubtitleStyle,
    work: &Path,
    window: Option<(f64, f64)>,
    cancel: &AtomicBool,
) -> Result<Vec<SubImage>> {
    std::fs::create_dir_all(work)?;
    let stem = work.join(format!("sub-{}", track.id));
    // Embedded streams keep the container clock; external files start at 0.
    let (source_file, base) = match &track.source {
        SubtitleSource::Embedded { index } => {
            let ext = match track.kind() {
                SubtitleKind::Pgs => "sup",
                _ => "ass",
            };
            let f = stem.with_extension(ext);
            extract(video_path, *index, &track.codec, &f, cancel)?;
            (f, info.start_time)
        }
        SubtitleSource::External { path } => {
            if !path.exists() {
                bail!("subtitle file {} is missing", path.display());
            }
            match track.kind() {
                SubtitleKind::Text if !track.is_ass() => {
                    let f = stem.with_extension("ass");
                    convert_text(path, &f, cancel)?;
                    (f, 0.0)
                }
                _ => (path.clone(), 0.0),
            }
        }
    };

    let video_size = (info.width.max(1), info.height.max(1));
    let mut images = match track.kind() {
        SubtitleKind::Text => {
            let restyle = !track.is_ass() || style.restyle_ass;
            // Rendered with square pixels, then squeezed onto wide ones.
            let square = (area.w as f64 / area.squeeze).round() as u32;
            let opts = ass::RenderOptions {
                frame: (square, area.h),
                offset: (0, 0),
                storage: video_size,
                style: restyle.then_some(style),
                // (start, end) on the video timeline → script clock in ms
                window: window.map(|(a, b)| (((a + base) * 1000.0) as i64, ((b + base) * 1000.0) as i64)),
            };
            let mut images = ass::render(&source_file, &opts, cancel)?;
            for img in &mut images {
                squeeze(img, area.squeeze);
                img.x += area.x;
                img.y += area.y;
            }
            images
        }
        SubtitleKind::Pgs => {
            let data = std::fs::read(&source_file).with_context(|| format!("reading {}", source_file.display()))?;
            pgs::decode_sup(&data)?
                .into_iter()
                .map(|d| {
                    // PGS canvas → the picture's area of the disc.
                    let (sx, sy) = (area.w as f64 / d.canvas.0.max(1) as f64, area.h as f64 / d.canvas.1.max(1) as f64);
                    let mut img = d.image;
                    if (sx - 1.0).abs() > 1e-3 || (sy - 1.0).abs() > 1e-3 {
                        let w = ((img.bitmap.width as f64 * sx).round() as u16).max(1);
                        let h = ((img.bitmap.height as f64 * sy).round() as u16).max(1);
                        img.bitmap = resize(&img.bitmap, w, h);
                    }
                    img.x = (img.x as f64 * sx).round() as u32 + area.x;
                    img.y = (img.y as f64 * sy).round() as u32 + area.y;
                    img
                })
                .collect()
        }
        SubtitleKind::Unsupported => bail!("{} subtitles are not supported", crate::model::codec_label(&track.codec)),
    };
    for img in &mut images {
        img.start -= base;
        img.end -= base;
        img.forced |= track.forced;
    }
    images.retain(|i| i.end > 0.0 && i.start < info.duration.max(1.0));
    for img in &mut images {
        img.start = img.start.max(0.0);
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_area_letterboxes() {
        let info = |w, h| MediaInfo { width: w, height: h, ..Default::default() };
        let a = default_area(&info(640, 480), VideoFormat::P1080_23976);
        assert_eq!((a.w, a.h, a.x, a.y), (1440, 1080, 240, 0));
        let a = default_area(&info(1920, 800), VideoFormat::P1080_23976);
        assert_eq!((a.x, a.y), (0, 140));
    }
}
