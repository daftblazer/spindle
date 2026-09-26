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
    pub fn has_video(&self) -> bool {
        self.video_codec.is_some()
    }

    pub fn has_audio(&self) -> bool {
        self.audio_codec.is_some()
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
    duration: Option<String>,
    start_time: Option<String>,
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
    parse(&out.stdout)
}

fn parse(json: &[u8]) -> Result<MediaInfo> {
    let p: Probe = serde_json::from_slice(json).context("unexpected ffprobe output")?;
    let mut info = MediaInfo::default();
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
            Some("audio") if info.audio_codec.is_none() => {
                info.audio_codec = s.codec_name.clone();
                info.audio_channels = s.channels.unwrap_or(2);
                info.audio_lang = s.tags.get("language").filter(|l| l.len() == 3).cloned();
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
             "avg_frame_rate":"24000/1001","r_frame_rate":"24000/1001"},
            {"codec_type":"audio","codec_name":"aac","channels":6,"tags":{"language":"eng"}},
            {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"fre","title":"Francais"},
             "disposition":{"default":0,"forced":1}},
            {"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle"}
          ],
          "format": {"duration":"123.456"}
        }"#;
        let i = parse(json).unwrap();
        assert_eq!(i.width, 1920);
        assert_eq!(i.fps, Some((24000, 1001)));
        assert_eq!(i.audio_channels, 6);
        assert_eq!(i.audio_lang.as_deref(), Some("eng"));
        assert!((i.duration - 123.456).abs() < 1e-9);
        assert_eq!(i.subtitles.len(), 2);
        assert_eq!(i.subtitles[0].lang.as_deref(), Some("fre"));
        assert!(i.subtitles[0].forced);
        assert_eq!(i.subtitles[1].index, 1);
    }
}
