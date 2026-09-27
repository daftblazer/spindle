// SPDX-License-Identifier: GPL-3.0-or-later

//! Checks whether a video can go on a Blu-ray without re-encoding.

use crate::bluray::VideoFormat;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Pass,
    /// Works on most players but isn't strictly within the specification.
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Check {
    pub name: String,
    pub level: Level,
    pub detail: String,
}

/// Result of checking a video file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub checks: Vec<Check>,
    /// Blu-ray format of the stream, when it is one.
    pub format: Option<VideoFormat>,
    /// The first audio stream is AC-3 that can be copied too.
    pub copy_audio: bool,
}

impl Report {
    pub fn compatible(&self) -> bool {
        self.format.is_some() && self.checks.iter().all(|c| c.level != Level::Fail)
    }

    pub fn has_warnings(&self) -> bool {
        self.checks.iter().any(|c| c.level == Level::Warn)
    }

    /// One-line summary for the UI.
    pub fn summary(&self) -> String {
        if let Some(fail) = self.checks.iter().find(|c| c.level == Level::Fail) {
            return format!("{}: {}", fail.name, fail.detail);
        }
        let fmt = self.format.map(|f| f.label()).unwrap_or("?");
        if self.has_warnings() {
            format!("Compatible with warnings · {fmt}")
        } else {
            format!("Compatible · {fmt}")
        }
    }
}

#[derive(Deserialize, Default)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    level: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    field_order: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    bit_rate: Option<String>,
    channels: Option<u32>,
    sample_rate: Option<String>,
    #[serde(default)]
    disposition: std::collections::HashMap<String, i32>,
}

#[derive(Deserialize, Default)]
struct Format {
    bit_rate: Option<String>,
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

fn rate(s: &Option<String>) -> Option<(u32, u32)> {
    let (n, d) = s.as_deref()?.split_once('/')?;
    let (n, d) = (n.parse().ok()?, d.parse().ok()?);
    (n > 0 && d > 0).then_some((n, d))
}

/// Map size, rate and scan type onto a Blu-ray format.
fn bd_format(w: u32, h: u32, fps: (u32, u32), interlaced: bool) -> Option<VideoFormat> {
    let r = fps.0 as f64 / fps.1 as f64;
    let near = |x: f64| (r - x).abs() < 0.01;
    match (w, h, interlaced) {
        (1920, 1080, false) if near(24000.0 / 1001.0) => Some(VideoFormat::P1080_23976),
        (1920, 1080, false) if near(24.0) => Some(VideoFormat::P1080_24),
        // Interlaced streams report the frame rate (25 / 29.97).
        (1920, 1080, true) if near(25.0) => Some(VideoFormat::I1080_25),
        (1920, 1080, true) if near(30000.0 / 1001.0) => Some(VideoFormat::I1080_2997),
        (1280, 720, false) if near(24000.0 / 1001.0) => Some(VideoFormat::P720_23976),
        (1280, 720, false) if near(24.0) => Some(VideoFormat::P720_24),
        (1280, 720, false) if near(50.0) => Some(VideoFormat::P720_50),
        (1280, 720, false) if near(60000.0 / 1001.0) => Some(VideoFormat::P720_5994),
        (720, 576, true) if near(25.0) => Some(VideoFormat::I576_25),
        (720, 480, true) if near(30000.0 / 1001.0) => Some(VideoFormat::I480_2997),
        _ => None,
    }
}

fn describe_rate(fps: (u32, u32)) -> String {
    let r = fps.0 as f64 / fps.1 as f64;
    if (r - r.round()).abs() < 0.001 {
        format!("{r:.0}")
    } else {
        format!("{r:.3}")
    }
}

/// Longest gap between keyframes (seconds) in the first five minutes.
fn max_keyframe_gap(path: &Path) -> Result<Option<f64>> {
    let out = Command::new(super::ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-show_entries", "frame=pts_time,best_effort_timestamp_time", "-of", "csv=p=0", "-read_intervals", "%+300"])
        .arg(path)
        .output()
        .context("failed to run ffprobe")?;
    let mut times: Vec<f64> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split(',').find_map(|v| v.trim().parse::<f64>().ok()))
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    if times.len() < 2 {
        return Ok(None);
    }
    Ok(times.windows(2).map(|w| w[1] - w[0]).fold(None, |m: Option<f64>, g| Some(m.map_or(g, |m| m.max(g)))))
}

/// Latest keyframe at or before `t` seconds (on the file's clock).
pub fn keyframe_before(path: &Path, t: f64) -> Result<f64> {
    let from = (t - 30.0).max(0.0);
    let out = Command::new(super::ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-show_entries", "frame=pts_time,best_effort_timestamp_time", "-of", "csv=p=0", "-read_intervals"])
        .arg(format!("{from:.3}%{:.3}", t + 0.05))
        .arg(path)
        .output()
        .context("failed to run ffprobe")?;
    let best = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split(',').find_map(|v| v.trim().parse::<f64>().ok()))
        .filter(|k| *k <= t + 0.001)
        .fold(None, |m: Option<f64>, k| Some(m.map_or(k, |m| m.max(k))));
    Ok(best.unwrap_or(0.0))
}

pub fn analyze(path: &Path) -> Result<Report> {
    let out = Command::new(super::ffprobe_bin())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .context("failed to run ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe could not read {}", path.display());
    }
    let probe: Probe = serde_json::from_slice(&out.stdout).context("unexpected ffprobe output")?;
    let video = probe
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.get("attached_pic") != Some(&1))
        .context("no video stream")?;
    let mut checks = Vec::new();
    let mut add = |name: &str, level: Level, detail: String| checks.push(Check { name: name.into(), level, detail });

    // Codec
    let codec = video.codec_name.clone().unwrap_or_default();
    match codec.as_str() {
        "h264" => add("Codec", Level::Pass, "H.264 / AVC".into()),
        "hevc" => add("Codec", Level::Fail, "HEVC is only allowed on Ultra HD Blu-ray".into()),
        other => add("Codec", Level::Fail, format!("{other} can't be kept; Blu-ray video must be H.264 here")),
    }

    // Profile, level, pixel format
    let profile = video.profile.clone().unwrap_or_default();
    if codec == "h264" {
        let ok = matches!(profile.as_str(), "High" | "Main" | "Constrained Baseline");
        add(
            "Profile",
            if ok { Level::Pass } else { Level::Fail },
            if ok { profile.clone() } else { format!("{profile} isn't allowed (needs Main or High)") },
        );
        let level = video.level.unwrap_or(0);
        let lvl = format!("{}.{}", level / 10, level % 10);
        add(
            "Level",
            if level > 0 && level <= 41 { Level::Pass } else { Level::Fail },
            if level <= 41 { lvl } else { format!("{lvl} is above the 4.1 limit") },
        );
    }
    let pix = video.pix_fmt.clone().unwrap_or_default();
    let pix_ok = matches!(pix.as_str(), "yuv420p" | "yuvj420p");
    add(
        "Color Format",
        if pix_ok { Level::Pass } else { Level::Fail },
        if pix_ok { "8-bit 4:2:0".into() } else { format!("{pix} (needs 8-bit 4:2:0)") },
    );

    // Resolution, frame rate, scan
    let (w, h) = (video.width.unwrap_or(0), video.height.unwrap_or(0));
    let fps = rate(&video.r_frame_rate).or_else(|| rate(&video.avg_frame_rate)).unwrap_or((0, 1));
    let interlaced = matches!(video.field_order.as_deref(), Some("tt" | "bb" | "tb" | "bt"));
    let format = bd_format(w, h, fps, interlaced);
    let scan = if interlaced { "i" } else { "p" };
    match format {
        Some(f) => add("Resolution", Level::Pass, f.label().to_string()),
        None => add(
            "Resolution",
            Level::Fail,
            format!(
                "{w}×{h}{scan} at {} fps isn't a Blu-ray format (1080p 23.976/24, 1080i 25/29.97, 720p 50/59.94)",
                describe_rate(fps)
            ),
        ),
    }

    // Bitrate
    let bitrate = video
        .bit_rate
        .as_deref()
        .or(probe.format.as_ref().and_then(|f| f.bit_rate.as_deref()))
        .and_then(|b| b.parse::<f64>().ok())
        .map(|b| b / 1e6);
    match bitrate {
        Some(b) if b <= 35.0 => add("Bitrate", Level::Pass, format!("{b:.1} Mbit/s")),
        Some(b) if b <= 40.0 => add("Bitrate", Level::Warn, format!("{b:.1} Mbit/s is close to the 40 Mbit/s limit")),
        Some(b) => add("Bitrate", Level::Fail, format!("{b:.1} Mbit/s is above the 40 Mbit/s limit")),
        None => add("Bitrate", Level::Warn, "Unknown".into()),
    }

    // Keyframe spacing (GOP length)
    if codec == "h264" {
        match max_keyframe_gap(path)? {
            Some(g) if g <= 1.05 => add("Keyframes", Level::Pass, format!("Every {g:.1} s or less")),
            Some(g) => add(
                "Keyframes",
                Level::Warn,
                format!("Up to {g:.1} s apart (Blu-ray expects 1 s); chapters and seeking may be less precise on some players"),
            ),
            None => add(
                "Keyframes",
                Level::Warn,
                "Too far apart to measure; chapters and seeking may be imprecise on some players".into(),
            ),
        }
    }

    // Audio: AC-3 at 48 kHz can be copied as well.
    let audio = probe.streams.iter().find(|s| s.codec_type.as_deref() == Some("audio"));
    let copy_audio = audio.is_some_and(|a| {
        a.codec_name.as_deref() == Some("ac3")
            && a.sample_rate.as_deref() == Some("48000")
            && a.channels.unwrap_or(0) <= 6
            && a.bit_rate.as_deref().and_then(|b| b.parse::<u64>().ok()).is_none_or(|b| b <= 640_000)
    });
    match audio {
        None => {}
        Some(_) if copy_audio => add("Audio", Level::Pass, "AC-3, kept as is".into()),
        Some(a) => add(
            "Audio",
            Level::Pass,
            format!("{} will be converted (fast)", a.codec_name.clone().unwrap_or_default().to_uppercase()),
        ),
    }

    let format = format.filter(|_| codec == "h264");
    Ok(Report { checks, format, copy_audio })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bd_format(1920, 1080, (24000, 1001), false), Some(VideoFormat::P1080_23976));
        assert_eq!(bd_format(1920, 1080, (25, 1), true), Some(VideoFormat::I1080_25));
        // 1080p25 progressive isn't a Blu-ray format.
        assert_eq!(bd_format(1920, 1080, (25, 1), false), None);
        assert_eq!(bd_format(1280, 720, (60000, 1001), false), Some(VideoFormat::P720_5994));
        assert_eq!(bd_format(1280, 720, (30, 1), false), None);
    }
}
