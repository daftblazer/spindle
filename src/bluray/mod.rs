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
    /// The SD formats with a 4:3 picture, for 4:3 televisions.
    I576_25_4x3,
    I480_2997_4x3,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 12] = [
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
        VideoFormat::I576_25_4x3,
        VideoFormat::I480_2997_4x3,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::P1080_23976 => "1080p 23.976",
            VideoFormat::P1080_24 => "1080p 24",
            VideoFormat::I1080_25 => "1080i 25 (PAL)",
            VideoFormat::I1080_2997 => "1080i 29.97 (NTSC)",
            VideoFormat::P720_23976 => "720p 23.976",
            VideoFormat::P720_24 => "720p 24",
            VideoFormat::P720_50 => "720p 50",
            VideoFormat::P720_5994 => "720p 59.94",
            VideoFormat::I576_25 => "576i PAL 16:9",
            VideoFormat::I480_2997 => "480i NTSC 16:9",
            VideoFormat::I576_25_4x3 => "576i PAL 4:3",
            VideoFormat::I480_2997_4x3 => "480i NTSC 4:3",
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            VideoFormat::P720_23976 | VideoFormat::P720_24 | VideoFormat::P720_50 | VideoFormat::P720_5994 => (1280, 720),
            VideoFormat::I576_25 | VideoFormat::I576_25_4x3 => (720, 576),
            VideoFormat::I480_2997 | VideoFormat::I480_2997_4x3 => (720, 480),
            _ => (1920, 1080),
        }
    }

    /// Frame rate as a rational (num, den); interlaced formats have two
    /// fields per frame.
    pub fn fps(self) -> (u32, u32) {
        match self {
            VideoFormat::P1080_23976 | VideoFormat::P720_23976 => (24000, 1001),
            VideoFormat::P1080_24 | VideoFormat::P720_24 => (24, 1),
            VideoFormat::I1080_25 | VideoFormat::I576_25 | VideoFormat::I576_25_4x3 => (25, 1),
            VideoFormat::I1080_2997 | VideoFormat::I480_2997 | VideoFormat::I480_2997_4x3 => (30000, 1001),
            VideoFormat::P720_50 => (50, 1),
            VideoFormat::P720_5994 => (60000, 1001),
        }
    }

    /// Coded as fields (1080i and SD).
    pub fn interlaced(self) -> bool {
        matches!(self, VideoFormat::I1080_25 | VideoFormat::I1080_2997) || self.is_sd()
    }

    pub fn is_sd(self) -> bool {
        matches!(self, VideoFormat::I576_25 | VideoFormat::I480_2997 | VideoFormat::I576_25_4x3 | VideoFormat::I480_2997_4x3)
    }

    /// A 4:3 picture (only SD can be).
    pub fn is_4x3(self) -> bool {
        matches!(self, VideoFormat::I576_25_4x3 | VideoFormat::I480_2997_4x3)
    }

    /// NTSC resolution (480 lines).
    fn is_480(self) -> bool {
        matches!(self, VideoFormat::I480_2997 | VideoFormat::I480_2997_4x3)
    }

    /// Pixel aspect ratio of the picture (ITU-R BT.601 for SD).
    pub fn sar(self) -> (u32, u32) {
        match self {
            VideoFormat::I480_2997 => (40, 33),
            VideoFormat::I576_25 => (16, 11),
            VideoFormat::I480_2997_4x3 => (10, 11),
            VideoFormat::I576_25_4x3 => (12, 11),
            _ => (1, 1),
        }
    }

    /// `aspect_ratio` code of clip info: 2 = 4:3, 3 = 16:9.
    pub fn aspect_code(self) -> u8 {
        if self.is_4x3() {
            2
        } else {
            3
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
            _ if self.is_sd() && self.is_480() => 1,
            _ if self.is_sd() => 2,
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
    /// 16-bit LPCM.
    Lpcm,
    Lpcm24,
    /// The codecs below are only copied from sources, never encoded.
    Dts,
    DtsHdHra,
    DtsHdMa,
    /// Dolby TrueHD, with an AC-3 core for players without TrueHD.
    TrueHd,
    /// Dolby Digital Plus (an AC-3 core plus E-AC-3 extension frames).
    Eac3,
}

impl AudioCodec {
    /// The formats audio can be encoded to.
    pub const ENCODE: [AudioCodec; 3] = [AudioCodec::Ac3, AudioCodec::Lpcm, AudioCodec::Lpcm24];

    pub fn coding_type(self) -> u8 {
        match self {
            AudioCodec::Lpcm | AudioCodec::Lpcm24 => 0x80,
            AudioCodec::Ac3 => 0x81,
            AudioCodec::Dts => 0x82,
            AudioCodec::TrueHd => 0x83,
            AudioCodec::Eac3 => 0x84,
            AudioCodec::DtsHdHra => 0x85,
            AudioCodec::DtsHdMa => 0x86,
        }
    }

    pub fn is_lpcm(self) -> bool {
        matches!(self, AudioCodec::Lpcm | AudioCodec::Lpcm24)
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Ac3 => "Dolby Digital (AC-3)",
            AudioCodec::Lpcm => "LPCM 16-bit",
            AudioCodec::Lpcm24 => "LPCM 24-bit",
            AudioCodec::Dts => "DTS",
            AudioCodec::DtsHdHra => "DTS-HD High Resolution",
            AudioCodec::DtsHdMa => "DTS-HD Master Audio",
            AudioCodec::TrueHd => "Dolby TrueHD",
            AudioCodec::Eac3 => "Dolby Digital Plus",
        }
    }

    /// `sampling_frequency` code of stream attributes. HD codecs above
    /// 48 kHz carry a 48 kHz core.
    pub fn rate_code(self, rate: u32) -> u8 {
        let core = matches!(self, AudioCodec::DtsHdHra | AudioCodec::DtsHdMa | AudioCodec::TrueHd);
        match rate {
            96_000 if core => 14,
            96_000 => 4,
            192_000 if core => 12,
            192_000 => 5,
            _ => 1,
        }
    }
}

/// Elementary stream description shared by clip info, playlists and PMTs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EsKind {
    Video(VideoFormat),
    Audio { codec: AudioCodec, channels: u8, rate: u32, lang: String },
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

    /// (presentation type, sampling frequency) byte of audio attributes.
    pub fn audio_attributes(&self) -> u8 {
        match &self.kind {
            EsKind::Audio { codec, channels, rate, .. } => (Self::audio_format_code(*channels) << 4) | codec.rate_code(*rate),
            _ => 0,
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
