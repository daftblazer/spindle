// SPDX-License-Identifier: GPL-3.0-or-later

//! Media inspection via `ffprobe`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaInfo {
    /// Duration in seconds (0 for still images).
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// Frame rate as (num, den).
    pub fps: Option<(u32, u32)>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: u8,
    pub audio_lang: Option<String>,
    /// Start time of the video stream in seconds (timestamps of embedded
    /// subtitles are relative to the same clock).
    #[serde(default)]
    pub start_time: f64,
    #[serde(default)]
    pub subtitles: Vec<SubtitleStream>,
    #[serde(default)]
    pub audio_streams: Vec<AudioStream>,
    /// Version of the probe that filled this in (older ones lack fields).
    #[serde(default)]
    pub probe_version: u32,
    /// Sample (pixel) aspect ratio of the video, when not square.
    #[serde(default)]
    pub sar: Option<(u32, u32)>,
    /// "progressive", "tt", "bb", "tb" or "bt" (interlaced), when known.
    #[serde(default)]
    pub field_order: Option<String>,
    /// Transfer characteristics, e.g. "smpte2084" (HDR10) or "arib-std-b67" (HLG).
    #[serde(default)]
    pub color_transfer: Option<String>,
    #[serde(default)]
    pub color_space: Option<String>,
    #[serde(default)]
    pub pix_fmt: Option<String>,
}

/// Current [`MediaInfo::probe_version`].
pub const PROBE_VERSION: u32 = 2;

/// An audio stream inside a media file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AudioStream {
    /// Index among the file's audio streams (ffmpeg `0:a:N`).
    pub index: usize,
    pub codec: String,
    pub channels: u8,
    pub sample_rate: u32,
    /// Bits per second, when known.
    pub bit_rate: Option<u64>,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub default: bool,
}

impl AudioStream {
    /// AC-3 that is valid on a Blu-ray as it is.
    pub fn is_bluray_ac3(&self) -> bool {
        self.codec == "ac3" && self.sample_rate == 48000 && self.channels <= 6 && self.bit_rate.is_none_or(|b| b <= 640_000)
    }
}

/// A subtitle stream inside a media file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SubtitleStream {
    /// Index among the file's subtitle streams (ffmpeg `0:s:N`).
    pub index: usize,
    pub codec: String,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub forced: bool,
    pub default: bool,
}

impl MediaInfo {
    /// Whether the video is interlaced (`None` when the file doesn't say).
    pub fn interlaced(&self) -> Option<bool> {
        match self.field_order.as_deref()? {
            "progressive" => Some(false),
            "tt" | "bb" | "tb" | "bt" => Some(true),
            _ => None,
        }
    }

    /// Bottom field first.
    pub fn bottom_field_first(&self) -> bool {
        matches!(self.field_order.as_deref(), Some("bb" | "bt"))
    }

    /// HDR video (PQ or HLG), which needs tone mapping for Blu-ray.
    pub fn is_hdr(&self) -> bool {
        matches!(self.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67"))
    }

    /// Bits per colour sample.
    pub fn bit_depth(&self) -> u32 {
        let f = self.pix_fmt.as_deref().unwrap_or("");
        if f.contains("p10") {
            10
        } else if f.contains("p12") {
            12
        } else if f.contains("p16") {
            16
        } else {
            8
        }
    }

    /// Width / height as shown (with the pixel aspect ratio).
    pub fn display_aspect(&self) -> f64 {
        let (n, d) = self.sar.unwrap_or((1, 1));
        self.width.max(1) as f64 * n as f64 / d.max(1) as f64 / self.height.max(1) as f64
    }

    pub fn has_video(&self) -> bool {
        self.video_codec.is_some()
    }

    pub fn has_audio(&self) -> bool {
        self.audio_codec.is_some()
    }

    /// Audio streams; files probed by older versions only list the first.
    pub fn audio(&self) -> Vec<AudioStream> {
        if !self.audio_streams.is_empty() || !self.has_audio() {
            return self.audio_streams.clone();
        }
        vec![AudioStream {
            index: 0,
            codec: self.audio_codec.clone().unwrap_or_default(),
            channels: self.audio_channels,
            lang: self.audio_lang.clone(),
            ..Default::default()
        }]
    }
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    channels: Option<u8>,
    sample_rate: Option<String>,
    bit_rate: Option<String>,
    duration: Option<String>,
    start_time: Option<String>,
    sample_aspect_ratio: Option<String>,
    field_order: Option<String>,
    color_transfer: Option<String>,
    color_space: Option<String>,
    pix_fmt: Option<String>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    disposition: std::collections::HashMap<String, i32>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

fn parse_rate(s: &str) -> Option<(u32, u32)> {
    let (n, d) = s.split_once('/')?;
    let (n, d): (u32, u32) = (n.parse().ok()?, d.parse().ok()?);
    (n > 0 && d > 0).then_some((n, d))
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let out = Command::new(super::ffprobe_bin())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .context("failed to run ffprobe (is FFmpeg installed?)")?;
    if !out.status.success() {
        bail!("ffprobe could not read {}: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    let mut info = parse(&out.stdout)?;
    // Containers disagree on what their field order flags mean; a decoded
    // frame says which field comes first.
    if info.interlaced() == Some(true) {
        if let Some(tff) = first_frame_tff(path) {
            info.field_order = Some(if tff { "tt" } else { "bb" }.into());
        }
    }
    Ok(info)
}

/// Whether the first interlaced frame shows its top field first.
fn first_frame_tff(path: &Path) -> Option<bool> {
    let out = Command::new(super::ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0", "-read_intervals", "%+#5", "-show_entries", "frame=interlaced_frame,top_field_first", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let v: Vec<&str> = l.split(',').map(str::trim).collect();
        (v.first() == Some(&"1")).then(|| v.get(1) == Some(&"1"))
    })
}

fn parse(json: &[u8]) -> Result<MediaInfo> {
    let p: Probe = serde_json::from_slice(json).context("unexpected ffprobe output")?;
    let mut info = MediaInfo { probe_version: PROBE_VERSION, ..Default::default() };
    let secs = |s: &Option<String>| s.as_deref().and_then(|d| d.parse::<f64>().ok());
    info.duration = p.format.as_ref().and_then(|f| secs(&f.duration)).unwrap_or(0.0);

    for s in &p.streams {
        match s.codec_type.as_deref() {
            Some("video") if info.video_codec.is_none() && s.disposition.get("attached_pic") != Some(&1) => {
                info.video_codec = s.codec_name.clone();
                info.width = s.width.unwrap_or(0);
                info.height = s.height.unwrap_or(0);
                info.fps = s
                    .avg_frame_rate
                    .as_deref()
                    .and_then(parse_rate)
                    .or_else(|| s.r_frame_rate.as_deref().and_then(parse_rate));
                if info.duration == 0.0 {
                    info.duration = secs(&s.duration).unwrap_or(0.0);
                }
                info.start_time = secs(&s.start_time).unwrap_or(0.0);
                info.sar = s
                    .sample_aspect_ratio
                    .as_deref()
                    .and_then(|r| r.split_once(':'))
                    .and_then(|(n, d)| Some((n.parse::<u32>().ok()?, d.parse::<u32>().ok()?)))
                    .filter(|&(n, d)| n > 0 && d > 0 && n != d);
                let known = |v: &Option<String>| v.clone().filter(|v| v != "unknown");
                info.field_order = known(&s.field_order);
                info.color_transfer = known(&s.color_transfer);
                info.color_space = known(&s.color_space);
                info.pix_fmt = known(&s.pix_fmt);
            }
            Some("subtitle") => {
                let tag = |k: &str| s.tags.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
                info.subtitles.push(SubtitleStream {
                    index: info.subtitles.len(),
                    codec: s.codec_name.clone().unwrap_or_default(),
                    lang: tag("language").filter(|l| l != "und"),
                    title: tag("title"),
                    forced: s.disposition.get("forced") == Some(&1),
                    default: s.disposition.get("default") == Some(&1),
                });
            }
            Some("audio") => {
                let tag = |k: &str| s.tags.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
                let stream = AudioStream {
                    index: info.audio_streams.len(),
                    codec: s.codec_name.clone().unwrap_or_default(),
                    channels: s.channels.unwrap_or(2),
                    sample_rate: s.sample_rate.as_deref().and_then(|r| r.parse().ok()).unwrap_or(0),
                    bit_rate: s.bit_rate.as_deref().and_then(|r| r.parse().ok()),
                    lang: tag("language").filter(|l| l.len() == 3 && l != "und"),
                    title: tag("title"),
                    default: s.disposition.get("default") == Some(&1),
                };
                if info.audio_codec.is_none() {
                    info.audio_codec = Some(stream.codec.clone());
                    info.audio_channels = stream.channels;
                    info.audio_lang = stream.lang.clone();
                }
                info.audio_streams.push(stream);
            }
            _ => {}
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{
          "streams": [
            {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
             "avg_frame_rate":"24000/1001","r_frame_rate":"24000/1001","sample_aspect_ratio":"1:1",
             "field_order":"progressive","color_transfer":"smpte2084","pix_fmt":"yuv420p10le"},
            {"codec_type":"audio","codec_name":"aac","channels":6,"tags":{"language":"eng"}},
            {"codec_type":"audio","codec_name":"ac3","channels":2,"sample_rate":"48000","bit_rate":"192000",
             "tags":{"language":"jpn","title":"Commentary"}},
            {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"fre","title":"Francais"},
             "disposition":{"default":0,"forced":1}},
            {"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle"}
          ],
          "format": {"duration":"123.456"}
        }"#;
        let i = parse(json).unwrap();
        assert_eq!(i.width, 1920);
        assert_eq!(i.fps, Some((24000, 1001)));
        assert_eq!(i.sar, None);
        assert_eq!(i.interlaced(), Some(false));
        assert!(i.is_hdr());
        assert_eq!(i.bit_depth(), 10);
        assert!((i.display_aspect() - 16.0 / 9.0).abs() < 1e-9);
        assert_eq!(i.audio_channels, 6);
        assert_eq!(i.audio_lang.as_deref(), Some("eng"));
        assert_eq!(i.audio_streams.len(), 2);
        assert_eq!(i.audio_streams[1].index, 1);
        assert_eq!(i.audio_streams[1].title.as_deref(), Some("Commentary"));
        assert!(i.audio_streams[1].is_bluray_ac3());
        assert!(!i.audio_streams[0].is_bluray_ac3());
        assert!((i.duration - 123.456).abs() < 1e-9);
        assert_eq!(i.subtitles.len(), 2);
        assert_eq!(i.subtitles[0].lang.as_deref(), Some("fre"));
        assert!(i.subtitles[0].forced);
        assert_eq!(i.subtitles[1].index, 1);
    }
}
