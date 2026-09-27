// SPDX-License-Identifier: GPL-3.0-or-later

//! Blu-ray (BDMV / HDMV) structure writers.

pub mod bytes;
pub mod ig;
pub mod layout;
pub mod nav;
pub mod ts;
pub mod udf;

use serde::{Deserialize, Serialize};

/// 90 kHz clock ticks per second.
pub const CLOCK_90K: u64 = 90_000;

/// Disc video formats allowed by the BD-ROM spec for H.264 primary video.
/// All are 16:9; the SD ones use wide (anamorphic) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VideoFormat {
    #[default]
    P1080_23976,
    P1080_24,
    /// 1080i50: interlaced video as it is, progressive 25p carried as
    /// fields (segmented frames).
    I1080_25,
    /// 1080i59.94, the same way for 29.97p.
    I1080_2997,
    P720_23976,
    P720_24,
    P720_50,
    P720_5994,
    /// 720×576 interlaced (PAL DVD resolution).
    I576_25,
    /// 720×480 interlaced (NTSC DVD resolution).
    I480_2997,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 10] = [
        VideoFormat::P1080_23976,
        VideoFormat::P1080_24,
        VideoFormat::I1080_25,
        VideoFormat::I1080_2997,
        VideoFormat::P720_23976,
        VideoFormat::P720_24,
        VideoFormat::P720_50,
        VideoFormat::P720_5994,
        VideoFormat::I576_25,
        VideoFormat::I480_2997,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::P1080_23976 => "1080p 23.976",
            VideoFormat::P1080_24 => "1080p 24",
            VideoFormat::I1080_25 => "1080 25 (PAL)",
            VideoFormat::I1080_2997 => "1080 29.97 (NTSC)",
            VideoFormat::P720_23976 => "720p 23.976",
            VideoFormat::P720_24 => "720p 24",
            VideoFormat::P720_50 => "720p 50",
            VideoFormat::P720_5994 => "720p 59.94",
            VideoFormat::I576_25 => "SD 576i 25 (PAL)",
            VideoFormat::I480_2997 => "SD 480i 29.97 (NTSC)",
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            VideoFormat::P720_23976 | VideoFormat::P720_24 | VideoFormat::P720_50 | VideoFormat::P720_5994 => (1280, 720),
            VideoFormat::I576_25 => (720, 576),
            VideoFormat::I480_2997 => (720, 480),
            _ => (1920, 1080),
        }
    }

    /// Frame rate as a rational (num, den); interlaced formats have two
    /// fields per frame.
    pub fn fps(self) -> (u32, u32) {
        match self {
            VideoFormat::P1080_23976 | VideoFormat::P720_23976 => (24000, 1001),
            VideoFormat::P1080_24 | VideoFormat::P720_24 => (24, 1),
            VideoFormat::I1080_25 | VideoFormat::I576_25 => (25, 1),
            VideoFormat::I1080_2997 | VideoFormat::I480_2997 => (30000, 1001),
            VideoFormat::P720_50 => (50, 1),
            VideoFormat::P720_5994 => (60000, 1001),
        }
    }

    /// Coded as fields (1080i and SD).
    pub fn interlaced(self) -> bool {
        matches!(self, VideoFormat::I1080_25 | VideoFormat::I1080_2997 | VideoFormat::I576_25 | VideoFormat::I480_2997)
    }

    pub fn is_sd(self) -> bool {
        matches!(self, VideoFormat::I576_25 | VideoFormat::I480_2997)
    }

    /// Pixel aspect ratio for a 16:9 picture.
    pub fn sar(self) -> (u32, u32) {
        match self {
            VideoFormat::I480_2997 => (40, 33),
            VideoFormat::I576_25 => (16, 11),
            _ => (1, 1),
        }
    }

    /// Frame duration in 90 kHz ticks (rounded).
    pub fn frame_ticks(self) -> u64 {
        let (n, d) = self.fps();
        (CLOCK_90K * d as u64 + n as u64 / 2) / n as u64
    }

    /// `video_format` code used in index/mpls/clpi.
    pub fn format_code(self) -> u8 {
        match self {
            VideoFormat::I480_2997 => 1,
            VideoFormat::I576_25 => 2,
            VideoFormat::I1080_25 | VideoFormat::I1080_2997 => 4,
            VideoFormat::P720_23976 | VideoFormat::P720_24 | VideoFormat::P720_50 | VideoFormat::P720_5994 => 5,
            _ => 6,
        }
    }

    /// `frame_rate` code used in index/mpls/clpi.
    pub fn rate_code(self) -> u8 {
        match self.fps() {
            (24000, 1001) => 1,
            (24, 1) => 2,
            (25, 1) => 3,
            (30000, 1001) => 4,
            (50, 1) => 6,
            _ => 7,
        }
    }

    /// Frame rate code used in IG/PG video descriptors.
    pub fn ig_rate_code(self) -> u8 {
        self.rate_code()
    }
}

/// Audio codec for primary audio streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioCodec {
    #[default]
    Ac3,
    Lpcm,
}

impl AudioCodec {
    pub fn coding_type(self) -> u8 {
        match self {
            AudioCodec::Lpcm => 0x80,
            AudioCodec::Ac3 => 0x81,
        }
    }
}

/// Elementary stream description shared by clip info, playlists and PMTs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EsKind {
    Video(VideoFormat),
    Audio { codec: AudioCodec, channels: u8, lang: String },
    Ig { lang: String },
    /// Presentation graphics (subtitles).
    Pg { lang: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsInfo {
    pub pid: u16,
    pub kind: EsKind,
}

impl EsInfo {
    pub fn coding_type(&self) -> u8 {
        match &self.kind {
            EsKind::Video(_) => 0x1b,
            EsKind::Audio { codec, .. } => codec.coding_type(),
            EsKind::Ig { .. } => 0x91,
            EsKind::Pg { .. } => 0x90,
        }
    }

    /// Audio presentation type code: 1 = mono, 3 = stereo, 6 = multichannel.
    pub fn audio_format_code(channels: u8) -> u8 {
        match channels {
            1 => 1,
            2 => 3,
            _ => 6,
        }
    }
}

pub const PID_PMT: u16 = 0x0100;
pub const PID_PCR: u16 = 0x1001;
pub const PID_VIDEO: u16 = 0x1011;
pub const PID_AUDIO_FIRST: u16 = 0x1100;
pub const PID_IG_FIRST: u16 = 0x1400;
pub const PID_PG_FIRST: u16 = 0x1200;

/// Language code padded/truncated to the 3 bytes the format requires.
pub fn lang3(lang: &str) -> String {
    let mut s: String = lang.chars().filter(|c| c.is_ascii_alphabetic()).take(3).collect();
    if s.len() != 3 {
        s = "und".into();
    }
    s.to_ascii_lowercase()
}
