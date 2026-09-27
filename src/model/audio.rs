// SPDX-License-Identifier: GPL-3.0-or-later

//! Audio tracks of titles.

use super::{new_id, normalize_lang, Id};
use crate::media::probe::{AudioStream, MediaInfo};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum AudioSource {
    /// The N-th audio stream of the title's video file.
    Embedded { index: usize },
    /// The first audio stream of another asset (dub, commentary), delayed
    /// by `offset` seconds (negative to start earlier).
    External { asset: Id, offset: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioTrack {
    pub id: Id,
    pub source: AudioSource,
    /// Language code (ISO 639-2, e.g. "eng").
    pub lang: String,
    /// Description shown in the editor.
    pub name: String,
    /// Include on the disc.
    pub enabled: bool,
    /// Channels on the disc.
    #[serde(default)]
    pub layout: ChannelLayout,
    /// Encode it even when the original is valid on Blu-ray (to change its
    /// channels or loudness, or to save space).
    #[serde(default)]
    pub reencode: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ChannelLayout {
    /// As the source: mono, stereo, or 5.1 (7.1 is mixed down to 5.1).
    #[default]
    Original,
    Mono,
    Stereo,
    /// 5.1, spreading stereo sources over the surround channels.
    Surround,
}

impl ChannelLayout {
    pub const ALL: [ChannelLayout; 4] = [ChannelLayout::Original, ChannelLayout::Mono, ChannelLayout::Stereo, ChannelLayout::Surround];

    /// Channel count asked for, if not the source's.
    pub fn channels(self) -> Option<u8> {
        match self {
            ChannelLayout::Original => None,
            ChannelLayout::Mono => Some(1),
            ChannelLayout::Stereo => Some(2),
            ChannelLayout::Surround => Some(6),
        }
    }
}

/// Blu-ray allows 32 primary audio streams; players cope better with few.
pub const MAX_AUDIO_TRACKS: usize = 8;

/// Short description of a stream, e.g. "AC-3 5.1".
pub fn describe(stream: &AudioStream) -> String {
    let profile = stream.profile.as_deref().unwrap_or("");
    let codec = match stream.codec.as_str() {
        "ac3" => "AC-3",
        "eac3" => "E-AC-3",
        "dts" if profile.starts_with("DTS-HD MA") => "DTS-HD MA",
        "dts" if profile.starts_with("DTS-HD HRA") => "DTS-HD HRA",
        "dts" => "DTS",
        "truehd" => "TrueHD",
        "aac" => "AAC",
        "mp3" => "MP3",
        "opus" => "Opus",
        "vorbis" => "Vorbis",
        "flac" => "FLAC",
        c if c.starts_with("pcm") => "PCM",
        c => c,
    };
    let layout = match stream.channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        6 => "5.1".to_string(),
        8 => "7.1".to_string(),
        n => format!("{n} ch"),
    };
    format!("{codec} {layout}")
}

/// Tracks for the audio streams of a video; the first (or the one marked
/// default) is enabled.
pub fn embedded_tracks_audio(info: &MediaInfo) -> Vec<AudioTrack> {
    let streams = info.audio();
    let default = streams.iter().position(|s| s.default).unwrap_or(0);
    let mut tracks: Vec<AudioTrack> = streams
        .iter()
        .map(|s| AudioTrack {
            id: new_id(),
            source: AudioSource::Embedded { index: s.index },
            lang: s.lang.as_deref().map(normalize_lang).unwrap_or_else(|| "und".into()),
            name: s.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| describe(s)),
            enabled: true,
            layout: ChannelLayout::Original,
            reencode: false,
        })
        .collect();
    // The default stream goes first so it plays first on the disc.
    if default > 0 && default < tracks.len() {
        let t = tracks.remove(default);
        tracks.insert(0, t);
    }
    tracks
}

/// Track for a separate audio file ("Movie.fr.ac3", "Commentary.flac").
pub fn external_audio_track(asset: &super::Asset) -> Option<AudioTrack> {
    let stream = asset.info.audio().into_iter().next()?;
    let stem = asset.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let lang = stream
        .lang
        .as_deref()
        .map(normalize_lang)
        .filter(|l| l != "und")
        .or_else(|| stem.rsplit(['.', '_', '-', ' ']).take(2).map(normalize_lang).find(|l| l != "und"))
        .unwrap_or_else(|| "und".into());
    Some(AudioTrack {
        id: new_id(),
        source: AudioSource::External { asset: asset.id, offset: 0.0 },
        lang,
        name: stream.title.clone().filter(|t| !t.is_empty()).unwrap_or(stem),
        enabled: true,
        layout: ChannelLayout::Original,
        reencode: false,
    })
}
