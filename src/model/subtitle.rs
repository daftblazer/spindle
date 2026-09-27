// SPDX-License-Identifier: GPL-3.0-or-later

//! Subtitle tracks of titles and the text subtitle style.

use super::{new_id, Id, Rgba};
use crate::media::probe::MediaInfo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubtitleSource {
    /// The N-th subtitle stream of the title's video file.
    Embedded { index: usize },
    /// A separate subtitle file.
    External { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleTrack {
    pub id: Id,
    pub source: SubtitleSource,
    /// ffmpeg codec name (subrip, ass, hdmv_pgs_subtitle, …).
    pub codec: String,
    /// Language code (ISO 639-2, e.g. "eng").
    pub lang: String,
    /// Description shown in the editor.
    pub name: String,
    /// Include on the disc.
    pub enabled: bool,
    pub forced: bool,
    /// Drawn into the picture instead of being a subtitle track (always
    /// shown; keeps every ASS effect exactly).
    #[serde(default)]
    pub burn_in: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleKind {
    /// Rendered with libass (SRT, ASS/SSA, WebVTT, MP4 text).
    Text,
    /// Presentation graphics, decoded and re-encoded.
    Pgs,
    Unsupported,
}

pub fn codec_kind(codec: &str) -> SubtitleKind {
    match codec {
        "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "vtt" | "mov_text" | "text" => SubtitleKind::Text,
        "hdmv_pgs_subtitle" | "sup" | "pgs" => SubtitleKind::Pgs,
        _ => SubtitleKind::Unsupported,
    }
}

/// Human readable format name.
pub fn codec_label(codec: &str) -> &str {
    match codec {
        "subrip" | "srt" => "SubRip",
        "ass" => "ASS",
        "ssa" => "SSA",
        "webvtt" | "vtt" => "WebVTT",
        "mov_text" => "MP4 Text",
        "hdmv_pgs_subtitle" | "sup" | "pgs" => "PGS",
        "dvd_subtitle" => "DVD (VobSub)",
        "dvb_subtitle" => "DVB",
        other => other,
    }
}

impl SubtitleTrack {
    pub fn kind(&self) -> SubtitleKind {
        codec_kind(&self.codec)
    }

    pub fn is_ass(&self) -> bool {
        matches!(self.codec.as_str(), "ass" | "ssa")
    }

    /// Text subtitles can be burned into the picture.
    pub fn can_burn_in(&self) -> bool {
        self.kind() == SubtitleKind::Text
    }
}

/// Two-letter to three-letter language codes for common languages
/// (sidecar files often use "movie.en.srt").
fn lang_3(code: &str) -> Option<&'static str> {
    const MAP: &[(&str, &str)] = &[
        ("en", "eng"), ("fr", "fra"), ("de", "deu"), ("es", "spa"), ("it", "ita"), ("pt", "por"),
        ("nl", "nld"), ("sv", "swe"), ("no", "nor"), ("nb", "nob"), ("da", "dan"), ("fi", "fin"),
        ("pl", "pol"), ("cs", "ces"), ("hu", "hun"), ("ro", "ron"), ("ru", "rus"), ("uk", "ukr"),
        ("el", "ell"), ("tr", "tur"), ("ar", "ara"), ("he", "heb"), ("hi", "hin"), ("ja", "jpn"),
        ("ko", "kor"), ("zh", "zho"), ("th", "tha"), ("vi", "vie"), ("id", "ind"), ("ms", "msa"),
    ];
    MAP.iter().find(|(two, _)| *two == code).map(|(_, three)| *three)
}

/// English name of a language code, for display.
pub fn language_name(code: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("eng", "English"), ("fra", "French"), ("fre", "French"), ("deu", "German"), ("ger", "German"),
        ("spa", "Spanish"), ("ita", "Italian"), ("por", "Portuguese"), ("nld", "Dutch"), ("dut", "Dutch"),
        ("swe", "Swedish"), ("nor", "Norwegian"), ("nob", "Norwegian"), ("dan", "Danish"), ("fin", "Finnish"),
        ("pol", "Polish"), ("ces", "Czech"), ("cze", "Czech"), ("hun", "Hungarian"), ("ron", "Romanian"),
        ("rum", "Romanian"), ("rus", "Russian"), ("ukr", "Ukrainian"), ("ell", "Greek"), ("gre", "Greek"),
        ("tur", "Turkish"), ("ara", "Arabic"), ("heb", "Hebrew"), ("hin", "Hindi"), ("jpn", "Japanese"),
        ("kor", "Korean"), ("zho", "Chinese"), ("chi", "Chinese"), ("tha", "Thai"), ("vie", "Vietnamese"),
        ("ind", "Indonesian"), ("msa", "Malay"), ("und", "Unknown language"),
    ];
    NAMES.iter().find(|(c, _)| *c == code).map_or_else(|| code.to_string(), |(_, n)| n.to_string())
}

/// Normalise a language tag to a three-letter code ("und" if unknown).
pub fn normalize_lang(tag: &str) -> String {
    let t = tag.trim().to_ascii_lowercase();
    let t = t.split(['-', '_']).next().unwrap_or("");
    if t.len() == 3 && t.chars().all(|c| c.is_ascii_alphabetic()) {
        return t.to_string();
    }
    lang_3(t).map(str::to_string).unwrap_or_else(|| "und".into())
}

pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sup"];

/// Build a track for an external subtitle file.
pub fn external_track(path: &Path, video: Option<&Path>) -> SubtitleTrack {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let codec = match ext.as_str() {
        "srt" => "subrip",
        "vtt" => "webvtt",
        "sup" => "hdmv_pgs_subtitle",
        other => other,
    }
    .to_string();
    // "movie.en.forced.srt" → language "en", forced
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let video_stem = video.and_then(|v| v.file_stem()).map(|s| s.to_string_lossy().into_owned());
    let tags: Vec<String> = match &video_stem {
        Some(vs) if stem.starts_with(vs.as_str()) => stem[vs.len()..].split('.').filter(|t| !t.is_empty()).map(str::to_lowercase).collect(),
        _ => stem.rsplit('.').take(2).map(str::to_lowercase).collect(),
    };
    let lang = tags.iter().map(|t| normalize_lang(t)).find(|l| l != "und").unwrap_or_else(|| "und".into());
    let forced = tags.iter().any(|t| t == "forced");
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    SubtitleTrack {
        id: new_id(),
        source: SubtitleSource::External { path: path.to_path_buf() },
        codec,
        lang,
        name,
        enabled: true,
        forced,
        burn_in: false,
    }
}

/// Tracks for the subtitle streams embedded in a video.
pub fn embedded_tracks(info: &MediaInfo) -> Vec<SubtitleTrack> {
    info.subtitles
        .iter()
        .map(|s| {
            let lang = s.lang.as_deref().map(normalize_lang).unwrap_or_else(|| "und".into());
            let mut name = s.title.clone().unwrap_or_default();
            if name.is_empty() {
                name = format!("{} {}", codec_label(&s.codec), s.index + 1);
            }
            SubtitleTrack {
                id: new_id(),
                source: SubtitleSource::Embedded { index: s.index },
                codec: s.codec.clone(),
                lang,
                name,
                enabled: codec_kind(&s.codec) != SubtitleKind::Unsupported,
                forced: s.forced,
                burn_in: false,
            }
        })
        .collect()
}

/// Subtitle files next to a video that belong to it ("Movie.srt",
/// "Movie.en.srt", "Movie.eng.forced.ass").
pub fn sidecar_files(video: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem()) else { return vec![] };
    let stem = stem.to_string_lossy().to_string();
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let ext_ok = p
                .extension()
                .map(|e| SUBTITLE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
                .unwrap_or(false);
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            ext_ok && (name == stem || name.starts_with(&format!("{stem}.")))
        })
        .collect();
    out.sort();
    out
}

/// Look of text subtitles (SRT, WebVTT, …; ASS keeps its own styling
/// unless `restyle_ass` is set). Sizes are pixels at 1080 lines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleStyle {
    pub font: String,
    pub size: f64,
    pub bold: bool,
    pub color: Rgba,
    pub outline_color: Rgba,
    pub outline: f64,
    pub shadow: f64,
    /// Distance of the text from the bottom edge.
    pub margin: f64,
    pub restyle_ass: bool,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        SubtitleStyle {
            font: "Cantarell".into(),
            size: 54.0,
            bold: true,
            color: Rgba::WHITE,
            outline_color: Rgba::BLACK,
            outline: 3.0,
            shadow: 1.5,
            margin: 60.0,
            restyle_ass: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert_eq!(normalize_lang("en"), "eng");
        assert_eq!(normalize_lang("fre"), "fre");
        assert_eq!(normalize_lang("pt-BR"), "por");
        assert_eq!(normalize_lang("??"), "und");
    }

    #[test]
    fn sidecar_names() {
        let t = external_track(Path::new("/v/Movie.en.forced.srt"), Some(Path::new("/v/Movie.mkv")));
        assert_eq!(t.lang, "eng");
        assert!(t.forced);
        assert_eq!(t.codec, "subrip");
        let t = external_track(Path::new("/v/Movie.ass"), Some(Path::new("/v/Movie.mkv")));
        assert_eq!(t.lang, "und");
        assert!(t.is_ass());
        let t = external_track(Path::new("/other/Show.S01E01.de.sup"), None);
        assert_eq!(t.lang, "deu");
        assert_eq!(t.kind(), SubtitleKind::Pgs);
    }
}
