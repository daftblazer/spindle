// SPDX-License-Identifier: GPL-3.0-or-later

//! ffmpeg argument construction for BD-compliant H.264/AC-3 streams.

use super::probe::MediaInfo;
use crate::bluray::{AudioCodec, VideoFormat};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct EncodeSettings {
    pub video: VideoFormat,
    pub video_bitrate: u32,
    pub audio: AudioCodec,
    pub audio_bitrate: u32,
}

fn s(v: impl ToString) -> String {
    v.to_string()
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Output channel count for a source with `channels` channels.
pub fn output_channels(channels: u8) -> u8 {
    if channels >= 6 {
        6
    } else if channels == 1 {
        1
    } else {
        2
    }
}

fn scale_filter(v: VideoFormat) -> String {
    let (w, h) = v.size();
    let (n, d) = v.fps();
    format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black,setsar=1,fps={n}/{d},format=yuv420p"
    )
}

fn video_args(set: &EncodeSettings, bitrate: u32) -> Vec<String> {
    let (n, d) = set.video.fps();
    // GOP length at most one second.
    let keyint = n / d;
    let mut x264 = format!(
        "bluray-compat=1:keyint={keyint}:min-keyint=1:open-gop=0:slices=4:aud=1:nal-hrd=vbr:\
         b-pyramid=strict:bframes=3:colorprim=bt709:transfer=bt709:colormatrix=bt709"
    );
    if set.video.fake_interlaced() {
        x264.push_str(":fake-interlaced=1:pic-struct=1");
    }
    vec![
        s("-c:v"),
        s("libx264"),
        s("-preset"),
        s("medium"),
        s("-profile:v"),
        s("high"),
        s("-level:v"),
        s("4.1"),
        s("-pix_fmt"),
        s("yuv420p"),
        s("-b:v"),
        format!("{bitrate}k"),
        s("-maxrate"),
        s("38000k"),
        s("-bufsize"),
        s("30000k"),
        s("-x264-params"),
        x264,
    ]
}

fn audio_args(set: &EncodeSettings, channels: u8) -> Vec<String> {
    let mut a = match set.audio {
        AudioCodec::Ac3 => vec![s("-c:a"), s("ac3"), s("-b:a"), format!("{}k", set.audio_bitrate)],
        AudioCodec::Lpcm => vec![s("-c:a"), s("pcm_bluray"), s("-sample_fmt"), s("s16")],
    };
    a.extend([s("-ar"), s("48000"), s("-ac"), s(channels)]);
    a
}

fn mux_args(output: &Path, ts_offset: f64) -> Vec<String> {
    vec![
        s("-f"),
        s("mpegts"),
        s("-mpegts_m2ts_mode"),
        s("1"),
        s("-pes_payload_size"),
        s("0"),
        s("-output_ts_offset"),
        format!("{ts_offset:.3}"),
        path(output),
    ]
}

/// Transcode a title. `keyframes` are forced IDR positions (chapter starts).
/// `range` limits the encode to (start, duration) seconds, for previews.
pub fn title_args(
    input: &Path,
    info: &MediaInfo,
    set: &EncodeSettings,
    keyframes: &[f64],
    range: Option<(f64, f64)>,
    output: &Path,
) -> Vec<String> {
    let mut a = Vec::new();
    if let Some((start, _)) = range {
        a.extend([s("-ss"), format!("{start:.3}")]);
    }
    a.extend([s("-i"), path(input), s("-map"), s("0:v:0")]);
    if let Some((_, duration)) = range {
        a.extend([s("-t"), format!("{duration:.3}")]);
    }
    if info.has_audio() {
        a.extend([s("-map"), s("0:a:0")]);
    }
    a.extend([s("-sn"), s("-dn"), s("-map_chapters"), s("-1"), s("-vf"), scale_filter(set.video)]);
    a.extend(video_args(set, set.video_bitrate));
    if !keyframes.is_empty() {
        let list: Vec<String> = keyframes.iter().map(|t| format!("{t:.3}")).collect();
        a.extend([s("-force_key_frames"), list.join(",")]);
    }
    if info.has_audio() {
        a.extend(audio_args(set, output_channels(info.audio_channels)));
    }
    a.extend(mux_args(output, 1.0));
    a
}

/// Copy a compatible H.264 stream without re-encoding, adding the access
/// unit delimiters Blu-ray requires. AC-3 audio is copied when possible,
/// other audio is encoded per `set`.
pub fn passthrough_args(
    input: &Path,
    info: &MediaInfo,
    set: &EncodeSettings,
    copy_audio: bool,
    range: Option<(f64, f64)>,
    output: &Path,
) -> Vec<String> {
    let mut a = Vec::new();
    if let Some((start, _)) = range {
        a.extend([s("-ss"), format!("{start:.3}")]);
    }
    a.extend([s("-i"), path(input), s("-map"), s("0:v:0")]);
    if info.has_audio() {
        a.extend([s("-map"), s("0:a:0")]);
    }
    if let Some((_, duration)) = range {
        a.extend([s("-t"), format!("{duration:.3}")]);
    }
    a.extend([s("-sn"), s("-dn"), s("-map_chapters"), s("-1")]);
    a.extend([s("-c:v"), s("copy"), s("-bsf:v"), s("h264_metadata=aud=insert")]);
    if info.has_audio() {
        if copy_audio {
            a.extend([s("-c:a"), s("copy")]);
        } else {
            a.extend(audio_args(set, output_channels(info.audio_channels)));
        }
    }
    a.extend(mux_args(output, 1.0));
    a
}

/// Encode a menu background clip.
///
/// * `still` – rendered PNG containing the whole static layer (background
///   color/image, texts, images, thumbnails).
/// * `motion` – optional video looped underneath `still` (which then must
///   be transparent where the video should show).
/// * `audio` – optional audio looped for the menu duration.
pub fn menu_args(
    still: &Path,
    motion: Option<&Path>,
    audio: Option<(&Path, &MediaInfo)>,
    duration: f64,
    set: &EncodeSettings,
    output: &Path,
) -> Vec<String> {
    let (n, d) = set.video.fps();
    let (w, h) = set.video.size();
    let mut a = Vec::new();
    let mut next_input = 0;
    let video_filter;
    if let Some(m) = motion {
        a.extend([s("-stream_loop"), s("-1"), s("-i"), path(m)]);
        a.extend([s("-loop"), s("1"), s("-framerate"), format!("{n}/{d}"), s("-i"), path(still)]);
        next_input = 2;
        video_filter = format!(
            "[0:v]{}[bg];[1:v]scale={w}:{h},format=rgba[fg];[bg][fg]overlay=format=auto,format=yuv420p[v]",
            scale_filter(set.video)
        );
    } else {
        a.extend([s("-loop"), s("1"), s("-framerate"), format!("{n}/{d}"), s("-i"), path(still)]);
        next_input += 1;
        video_filter = format!("[0:v]{}[v]", scale_filter(set.video));
    }
    if let Some((p, _)) = audio {
        a.extend([s("-stream_loop"), s("-1"), s("-i"), path(p)]);
    }
    a.extend([s("-filter_complex"), video_filter, s("-map"), s("[v]")]);
    if audio.is_some() {
        a.extend([s("-map"), format!("{next_input}:a:0")]);
    }
    a.extend([s("-t"), format!("{duration:.3}")]);
    let bitrate = if motion.is_some() { set.video_bitrate } else { set.video_bitrate.min(15_000) };
    a.extend(video_args(set, bitrate));
    if let Some((_, info)) = audio {
        a.extend(audio_args(set, output_channels(info.audio_channels)));
    }
    // Leave room before the first frame for the IG stream to be decoded.
    a.extend(mux_args(output, 2.0));
    a
}
