// SPDX-License-Identifier: GPL-3.0-or-later

//! ffmpeg argument construction for BD-compliant H.264/AC-3 streams.

use super::hwenc::{self, VideoEncoder};
use super::probe::MediaInfo;
use crate::bluray::{AudioCodec, VideoFormat};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct EncodeSettings {
    pub video: VideoFormat,
    pub video_bitrate: u32,
    pub audio: AudioCodec,
    pub audio_bitrate: u32,
    pub encoder: VideoEncoder,
}

impl EncodeSettings {
    /// The encoder actually used: hardware encoders can't make the
    /// fake-interlaced streams of 1080i formats, so those use x264.
    pub fn effective_encoder(&self) -> VideoEncoder {
        if self.video.fake_interlaced() {
            VideoEncoder::Software
        } else {
            self.encoder
        }
    }

    /// The same settings with x264 (for menus: short, and quality matters).
    fn software(&self) -> Self {
        EncodeSettings { encoder: VideoEncoder::Software, ..*self }
    }
}

/// Options that go before the inputs (hardware device).
fn device_args(set: &EncodeSettings) -> Vec<String> {
    match set.effective_encoder() {
        VideoEncoder::Vaapi => match hwenc::render_node() {
            Some(node) => vec![s("-vaapi_device"), path(&node)],
            None => vec![],
        },
        _ => vec![],
    }
}

/// Scaling to the disc format, plus the upload to the GPU for VA-API.
fn video_filter(set: &EncodeSettings) -> String {
    match set.effective_encoder() {
        VideoEncoder::Vaapi => format!("{},format=nv12,hwupload", scale_filter(set.video)),
        _ => scale_filter(set.video),
    }
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
    // Blu-ray limits for hardware encoders: High@4.1, closed GOPs of at
    // most a second, BT.709 and access unit delimiters.
    let common = |a: &mut Vec<String>| {
        a.extend([
            s("-b:v"),
            format!("{bitrate}k"),
            s("-maxrate"),
            s("38000k"),
            s("-bufsize"),
            s("30000k"),
            s("-g"),
            s(keyint),
            s("-color_primaries"),
            s("bt709"),
            s("-color_trc"),
            s("bt709"),
            s("-colorspace"),
            s("bt709"),
            // Delimiters and BT.709 colour in the stream headers.
            s("-bsf:v"),
            s("h264_metadata=aud=insert:video_full_range_flag=0:colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1"),
        ]);
    };
    match set.effective_encoder() {
        VideoEncoder::Vaapi => {
            let mut a = vec![s("-c:v"), s("h264_vaapi"), s("-profile:v"), s("high"), s("-level"), s("41"), s("-rc_mode"), s("VBR")];
            a.extend([s("-bf"), s("2"), s("-slices"), s("4")]);
            common(&mut a);
            return a;
        }
        VideoEncoder::Nvenc => {
            let mut a = vec![s("-c:v"), s("h264_nvenc"), s("-preset"), s("p6"), s("-tune"), s("hq"), s("-profile:v"), s("high")];
            a.extend([s("-level"), s("4.1"), s("-rc"), s("vbr"), s("-pix_fmt"), s("yuv420p")]);
            a.extend([s("-bf"), s("3"), s("-b_ref_mode"), s("disabled"), s("-forced-idr"), s("1"), s("-strict_gop"), s("1")]);
            common(&mut a);
            return a;
        }
        VideoEncoder::Software => {}
    }
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

/// An audio stream of a title's output.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioInput {
    /// Separate file, or `None` for the title's video file.
    pub file: Option<PathBuf>,
    /// Audio stream index within its file.
    pub index: usize,
    /// Delay in seconds (negative starts the file earlier).
    pub offset: f64,
    /// Source channel count.
    pub channels: u8,
    /// Copy the stream (Blu-ray compatible AC-3) instead of encoding it.
    pub copy: bool,
}

impl AudioInput {
    /// Channels on the disc.
    pub fn output_channels(&self) -> u8 {
        if self.copy {
            self.channels.max(1)
        } else {
            output_channels(self.channels)
        }
    }
}

/// How the video of a title is written.
enum VideoMode<'a> {
    Encode { keyframes: &'a [f64] },
    Copy,
}

fn title_common(
    input: &Path,
    info: &MediaInfo,
    set: &EncodeSettings,
    video: VideoMode,
    audio: &[AudioInput],
    range: Option<(f64, f64)>,
    output: &Path,
) -> Vec<String> {
    let start = range.map_or(0.0, |(start, _)| start);
    let mut a = Vec::new();
    if matches!(video, VideoMode::Encode { .. }) {
        a.extend(device_args(set));
    }
    if start > 0.0 {
        a.extend([s("-ss"), format!("{start:.3}")]);
    }
    a.extend([s("-i"), path(input)]);
    // Separate audio files, positioned against the video's timeline.
    let mut delays = Vec::new();
    let mut file_inputs = Vec::new();
    for au in audio {
        let Some(file) = &au.file else {
            delays.push(0.0);
            file_inputs.push(None);
            continue;
        };
        let lead = au.offset - start;
        if lead < 0.0 {
            a.extend([s("-ss"), format!("{:.3}", -lead)]);
        }
        a.extend([s("-i"), path(file)]);
        delays.push(lead.max(0.0));
        file_inputs.push(Some(file_inputs.iter().flatten().count() + 1));
    }

    a.extend([s("-map"), s("0:v:0")]);
    for (au, input) in audio.iter().zip(&file_inputs) {
        a.extend([s("-map"), format!("{}:a:{}", input.unwrap_or(0), au.index)]);
    }
    // Stop with the video even when a separate audio file runs longer.
    let duration = range.map(|(_, d)| d).or((file_inputs.iter().any(Option::is_some) && info.duration > 0.0).then_some(info.duration - start));
    if let Some(d) = duration {
        a.extend([s("-t"), format!("{d:.3}")]);
    }
    a.extend([s("-sn"), s("-dn"), s("-map_chapters"), s("-1")]);
    match video {
        VideoMode::Encode { keyframes } => {
            a.extend([s("-vf"), video_filter(set)]);
            a.extend(video_args(set, set.video_bitrate));
            if !keyframes.is_empty() {
                let list: Vec<String> = keyframes.iter().map(|t| format!("{t:.3}")).collect();
                a.extend([s("-force_key_frames"), list.join(",")]);
            }
        }
        VideoMode::Copy => a.extend([s("-c:v"), s("copy"), s("-bsf:v"), s("h264_metadata=aud=insert")]),
    }
    for (n, (au, delay)) in audio.iter().zip(&delays).enumerate() {
        if au.copy {
            a.extend([format!("-c:a:{n}"), s("copy")]);
            continue;
        }
        match set.audio {
            AudioCodec::Ac3 => a.extend([format!("-c:a:{n}"), s("ac3"), format!("-b:a:{n}"), format!("{}k", set.audio_bitrate)]),
            AudioCodec::Lpcm => a.extend([format!("-c:a:{n}"), s("pcm_bluray"), format!("-sample_fmt:a:{n}"), s("s16")]),
        }
        a.extend([format!("-ar:a:{n}"), s("48000"), format!("-ac:a:{n}"), s(au.output_channels())]);
        if *delay > 0.0 {
            a.extend([format!("-filter:a:{n}"), format!("adelay={:.0}:all=1", delay * 1000.0)]);
        }
    }
    a.extend(mux_args(output, 1.0));
    a
}

/// Transcode a title. `keyframes` are forced IDR positions (chapter starts).
/// `range` limits the encode to (start, duration) seconds, for previews.
pub fn title_args(
    input: &Path,
    info: &MediaInfo,
    set: &EncodeSettings,
    keyframes: &[f64],
    audio: &[AudioInput],
    range: Option<(f64, f64)>,
    output: &Path,
) -> Vec<String> {
    title_common(input, info, set, VideoMode::Encode { keyframes }, audio, range, output)
}

/// Copy a compatible H.264 stream without re-encoding, adding the access
/// unit delimiters Blu-ray requires.
pub fn passthrough_args(
    input: &Path,
    info: &MediaInfo,
    set: &EncodeSettings,
    audio: &[AudioInput],
    range: Option<(f64, f64)>,
    output: &Path,
) -> Vec<String> {
    title_common(input, info, set, VideoMode::Copy, audio, range, output)
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
    motion: Option<(&Path, f64)>,
    audio: Option<(&Path, &MediaInfo)>,
    duration: f64,
    set: &EncodeSettings,
    output: &Path,
) -> Vec<String> {
    let set = &set.software();
    let (n, d) = set.video.fps();
    let (w, h) = set.video.size();
    let mut a = Vec::new();
    let mut next_input = 0;
    let video_filter;
    if let Some((m, start)) = motion {
        // Start into the video; if it runs out before the loop ends, it
        // continues from its beginning.
        if start > 0.0 {
            a.extend([s("-ss"), format!("{start:.3}")]);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(video: VideoFormat, encoder: VideoEncoder) -> EncodeSettings {
        EncodeSettings { video, video_bitrate: 18_000, audio: AudioCodec::Ac3, audio_bitrate: 448, encoder }
    }

    fn arg_after<'a>(a: &'a [String], key: &str) -> Option<&'a str> {
        a.iter().position(|x| x == key).and_then(|i| a.get(i + 1)).map(|s| s.as_str())
    }

    #[test]
    fn hardware_encoders_for_titles_only() {
        let info = MediaInfo { duration: 60.0, video_codec: Some("h264".into()), ..Default::default() };
        let out = Path::new("/tmp/out.ts");
        let vaapi = settings(VideoFormat::P1080_23976, VideoEncoder::Vaapi);
        let a = title_args(Path::new("/tmp/in.mkv"), &info, &vaapi, &[10.0], &[], None, out);
        assert_eq!(arg_after(&a, "-c:v"), Some("h264_vaapi"));
        assert_eq!(arg_after(&a, "-g"), Some("23"));
        assert!(arg_after(&a, "-vf").unwrap().ends_with("hwupload"));
        assert!(arg_after(&a, "-bsf:v").unwrap().contains("aud=insert"));
        assert_eq!(arg_after(&a, "-force_key_frames"), Some("10.000"));
        if hwenc::render_node().is_some() {
            // The device comes before the input.
            let dev = a.iter().position(|x| x == "-vaapi_device").unwrap();
            assert!(dev < a.iter().position(|x| x == "-i").unwrap());
        }

        let nvenc = settings(VideoFormat::P720_5994, VideoEncoder::Nvenc);
        let a = title_args(Path::new("/tmp/in.mkv"), &info, &nvenc, &[], &[], None, out);
        assert_eq!(arg_after(&a, "-c:v"), Some("h264_nvenc"));
        assert_eq!(arg_after(&a, "-g"), Some("59"));
        assert!(!a.iter().any(|x| x == "-vaapi_device"));

        // 1080i can't be made in hardware; menus are always x264.
        let interlaced = settings(VideoFormat::I1080_25, VideoEncoder::Vaapi);
        let a = title_args(Path::new("/tmp/in.mkv"), &info, &interlaced, &[], &[], None, out);
        assert_eq!(arg_after(&a, "-c:v"), Some("libx264"));
        let a = menu_args(Path::new("/tmp/still.png"), None, None, 1.0, &vaapi, out);
        assert_eq!(arg_after(&a, "-c:v"), Some("libx264"));
        assert!(!a.iter().any(|x| x == "-vaapi_device"));
    }
}
