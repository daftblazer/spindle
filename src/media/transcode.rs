// SPDX-License-Identifier: GPL-3.0-or-later

//! ffmpeg argument construction for BD-compliant H.264/AC-3 streams.

use super::picture::{self, VideoOptions};
use super::probe::MediaInfo;
use crate::bluray::{AudioCodec, VideoFormat};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct EncodeSettings {
    pub video: VideoFormat,
    pub video_bitrate: u32,
    pub audio: AudioCodec,
    /// AC-3 bitrate in kbit/s; `None` suits each track's channels.
    pub audio_bitrate: Option<u32>,
    pub quality: Quality,
    pub tune: Tune,
}

/// What the video is, so x264 spends its bits where that kind shows flaws.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub enum Tune {
    #[default]
    None,
    /// Live action.
    Film,
    /// Flat colours and sharp lines (cartoons, anime).
    Animation,
    /// Keeps film grain.
    Grain,
}

impl Tune {
    pub const ALL: [Tune; 4] = [Tune::None, Tune::Film, Tune::Animation, Tune::Grain];

    fn x264(self) -> Option<&'static str> {
        match self {
            Tune::None => None,
            Tune::Film => Some("film"),
            Tune::Animation => Some("animation"),
            Tune::Grain => Some("grain"),
        }
    }
}

/// Speed against quality for x264.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub enum Quality {
    /// Quick drafts.
    Fast,
    #[default]
    Balanced,
    /// Slower preset and two passes: the best picture for the bitrate, and
    /// sizes that match the estimate closely.
    Best,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Fast, Quality::Balanced, Quality::Best];

    /// The x264 preset.
    pub fn preset(self) -> &'static str {
        match self {
            Quality::Fast => "veryfast",
            Quality::Balanced => "medium",
            Quality::Best => "slow",
        }
    }
}

/// One-pass encoding, or one of the two passes (sharing a log file).
#[derive(Debug, Clone)]
pub enum Pass {
    Only,
    /// Analyse the video only; nothing is written.
    First(PathBuf),
    Second(PathBuf),
}

impl EncodeSettings {
    /// The encode settings of a disc.
    pub fn for_disc(d: &crate::model::DiscSettings) -> Self {
        EncodeSettings {
            video: d.video,
            video_bitrate: d.video_bitrate,
            audio: d.audio,
            audio_bitrate: d.audio_bitrate,
            quality: d.quality,
            tune: d.tune,
        }
    }

}

/// A title's video: the file, what's in it, and how to prepare it.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub path: &'a Path,
    pub info: &'a MediaInfo,
    pub picture: &'a VideoOptions,
    /// Subtitles drawn into the picture.
    pub burn: Option<&'a crate::subtitles::BurnIn>,
}

/// Colour description of the disc format: (ffmpeg primaries, transfer,
/// matrix) names.
fn colour(v: VideoFormat) -> (&'static str, &'static str, &'static str) {
    match v {
        VideoFormat::I480_2997 | VideoFormat::I480_2997_4x3 => ("smpte170m", "smpte170m", "smpte170m"),
        VideoFormat::I576_25 | VideoFormat::I576_25_4x3 => ("bt470bg", "smpte170m", "bt470bg"),
        _ => ("bt709", "bt709", "bt709"),
    }
}

/// H.264 VUI codes (tables E-3, E-4 and E-5) of ffmpeg's colour names.
fn primaries_code(name: &str) -> Option<u8> {
    Some(match name {
        "bt709" => 1,
        "bt470m" => 4,
        "bt470bg" => 5,
        "smpte170m" => 6,
        "smpte240m" => 7,
        "film" => 8,
        "bt2020" => 9,
        _ => return None,
    })
}

fn transfer_code(name: &str) -> Option<u8> {
    Some(match name {
        "bt709" => 1,
        "gamma22" => 4,
        "gamma28" => 5,
        "smpte170m" => 6,
        "smpte240m" => 7,
        "iec61966-2-1" => 13,
        "bt2020-10" => 14,
        "bt2020-12" => 15,
        "smpte2084" => 16,
        "arib-std-b67" => 18,
        _ => return None,
    })
}

fn matrix_code(name: &str) -> Option<u8> {
    Some(match name {
        "bt709" => 1,
        "fcc" => 4,
        "bt470bg" => 5,
        "smpte170m" => 6,
        "smpte240m" => 7,
        "bt2020nc" => 9,
        "bt2020c" => 10,
        _ => return None,
    })
}

/// `h264_metadata` options that write the colour description into a
/// copied stream. Remuxes often don't carry one in the stream (MKV may
/// keep it in the container, which the disc loses), and players then
/// guess, some wrongly (UHD players with Dolby Vision or HDR conversion
/// on show washed out or tinted colours). What the source says is kept;
/// the rest is the disc format's standard.
fn colour_metadata(info: &MediaInfo, v: VideoFormat) -> String {
    let (prim, trc, matrix) = colour(v);
    let pick = |src: &Option<String>, code: fn(&str) -> Option<u8>, default: &str| {
        src.as_deref().and_then(code).or_else(|| code(default)).unwrap_or(2)
    };
    let range = if info.color_range.as_deref() == Some("pc") { 1 } else { 0 };
    format!(
        "colour_primaries={}:transfer_characteristics={}:matrix_coefficients={}:video_full_range_flag={range}",
        pick(&info.color_primaries, primaries_code, prim),
        pick(&info.color_transfer, transfer_code, trc),
        pick(&info.color_space, matrix_code, matrix),
    )
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

/// `interlaced`: the picture is coded as fields (top field first when
/// true); otherwise progressive frames are carried as fields on
/// interlaced disc formats (segmented frames).
fn video_args(set: &EncodeSettings, bitrate: u32, interlaced: Option<bool>) -> Vec<String> {
    let (n, d) = set.video.fps();
    // GOP length at most one second.
    let keyint = n / d;
    let (prim, trc, matrix) = colour(set.video);
    let mut x264 = format!(
        "bluray-compat=1:keyint={keyint}:min-keyint=1:open-gop=0:slices=4:aud=1:nal-hrd=vbr:\
         b-pyramid=strict:bframes=3:colorprim={prim}:transfer={trc}:colormatrix={matrix}"
    );
    match interlaced {
        Some(true) => x264.push_str(":tff=1"),
        Some(false) => x264.push_str(":bff=1"),
        // Progressive frames carried as fields (segmented frames).
        None if set.video.interlaced() => x264.push_str(":fake-interlaced=1:pic-struct=1"),
        None => {}
    }
    let mut a = vec![s("-c:v"), s("libx264"), s("-preset"), s(set.quality.preset())];
    if let Some(tune) = set.tune.x264() {
        a.extend([s("-tune"), s(tune)]);
    }
    a.extend([s("-profile:v"), s("high"), s("-level:v"), s("4.1"), s("-pix_fmt"), s("yuv420p")]);
    a.extend([s("-b:v"), format!("{bitrate}k")]);
    a.extend([
        s("-maxrate"),
        s("38000k"),
        s("-bufsize"),
        s("30000k"),
        s("-x264-params"),
        x264,
    ]);
    a
}

/// AC-3 bitrate in kbit/s for a track with `channels` channels on the
/// disc: the chosen one, or what suits the channels.
pub fn ac3_bitrate(chosen: Option<u32>, channels: u8) -> u32 {
    chosen.unwrap_or(match channels {
        0 | 1 => 128,
        2..=4 => 256,
        _ => 640,
    })
}

/// Encoder options for output audio stream `n` in the disc's format.
fn encode_audio(set: &EncodeSettings, n: usize, channels: u8) -> Vec<String> {
    let mut a = match set.audio {
        AudioCodec::Lpcm => vec![format!("-c:a:{n}"), s("pcm_bluray"), format!("-sample_fmt:a:{n}"), s("s16")],
        AudioCodec::Lpcm24 => vec![format!("-c:a:{n}"), s("pcm_bluray"), format!("-sample_fmt:a:{n}"), s("s32")],
        _ => vec![format!("-c:a:{n}"), s("ac3"), format!("-b:a:{n}"), format!("{}k", ac3_bitrate(set.audio_bitrate, channels))],
    };
    a.extend([format!("-ar:a:{n}"), s("48000"), format!("-ac:a:{n}"), s(channels)]);
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
    /// Copied as it is, in this Blu-ray codec, instead of encoded.
    pub copy: Option<AudioCodec>,
    /// Channels wanted on the disc, when not the source's.
    pub layout: Option<u8>,
    /// Loudness adjustment filter.
    pub loudness: Option<String>,
}

impl AudioInput {
    /// A stream encoded as it comes.
    pub fn encode(file: Option<PathBuf>, index: usize, offset: f64, channels: u8) -> Self {
        AudioInput { file, index, offset, channels, copy: None, layout: None, loudness: None }
    }

    /// Channels on the disc.
    pub fn output_channels(&self) -> u8 {
        match self.copy {
            Some(_) => self.channels.max(1),
            None => self.layout.unwrap_or_else(|| output_channels(self.channels)),
        }
    }

    /// Copied TrueHD, which carries an AC-3 core for other players.
    pub fn needs_core(&self) -> bool {
        self.copy == Some(AudioCodec::TrueHd)
    }
}

/// How the video of a title is written.
enum VideoMode<'a> {
    Encode { keyframes: &'a [f64] },
    Copy,
}

#[allow(clippy::too_many_arguments)]
fn title_common(
    src: &Source,
    set: &EncodeSettings,
    video: VideoMode,
    audio: &[AudioInput],
    range: Option<(f64, f64)>,
    pass: &Pass,
    output: &Path,
) -> Vec<String> {
    let (input, info) = (src.path, src.info);
    let start = range.map_or(0.0, |(start, _)| start);
    let mut a = Vec::new();
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
            let burn = src.burn.map(|b| b.filter(start));
            let plan = picture::plan_with(info, src.picture, set.video, burn.as_deref());
            a.extend([s("-vf"), plan.filters.clone()]);
            a.extend(video_args(set, set.video_bitrate, plan.interlaced));
            if !keyframes.is_empty() {
                let list: Vec<String> = keyframes.iter().map(|t| format!("{t:.3}")).collect();
                a.extend([s("-force_key_frames"), list.join(",")]);
            }
            match pass {
                Pass::Only => {}
                Pass::First(log) => a.extend([s("-pass"), s("1"), s("-passlogfile"), path(log)]),
                Pass::Second(log) => a.extend([s("-pass"), s("2"), s("-passlogfile"), path(log)]),
            }
        }
        VideoMode::Copy => {
            let bsf = format!("h264_metadata=aud=insert:{}", colour_metadata(info, set.video));
            a.extend([s("-c:v"), s("copy"), s("-bsf:v"), bsf]);
        }
    }
    for (n, (au, delay)) in audio.iter().zip(&delays).enumerate() {
        if au.copy.is_some() {
            a.extend([format!("-c:a:{n}"), s("copy")]);
            continue;
        }
        a.extend(encode_audio(set, n, au.output_channels()));
        let mut filters = Vec::new();
        if *delay > 0.0 {
            filters.push(format!("adelay={:.0}:all=1", delay * 1000.0));
        }
        // Stereo (or mono) spread over 5.1.
        if au.layout == Some(6) && au.channels < 6 {
            filters.push(s("surround=chl_out=5.1"));
        }
        filters.extend(au.loudness.clone());
        if !filters.is_empty() {
            a.extend([format!("-filter:a:{n}"), filters.join(",")]);
        }
    }
    // AC-3 cores of TrueHD streams, as extra streams after the others
    // (the remuxer puts each with its TrueHD).
    let cores = audio.iter().zip(&file_inputs).filter(|(au, _)| au.needs_core());
    for (n, (au, input)) in (audio.len()..).zip(cores) {
        a.extend([s("-map"), format!("{}:a:{}", input.unwrap_or(0), au.index)]);
        a.extend([format!("-c:a:{n}"), s("ac3"), format!("-b:a:{n}"), s("640k"), format!("-ar:a:{n}"), s("48000")]);
        a.extend([format!("-ac:a:{n}"), s(au.channels.min(6))]);
    }
    if matches!(pass, Pass::First(_)) {
        // The first pass only looks at the video.
        a.extend([s("-an"), s("-f"), s("null"), s("-")]);
        return a;
    }
    a.extend(mux_args(output, 1.0));
    a
}

/// Transcode a title. `keyframes` are forced IDR positions (chapter starts).
/// `range` limits the encode to (start, duration) seconds, for previews.
#[allow(clippy::too_many_arguments)]
pub fn title_args(
    src: &Source,
    set: &EncodeSettings,
    keyframes: &[f64],
    audio: &[AudioInput],
    range: Option<(f64, f64)>,
    pass: &Pass,
    output: &Path,
) -> Vec<String> {
    title_common(src, set, VideoMode::Encode { keyframes }, audio, range, pass, output)
}

/// Copy a compatible H.264 stream without re-encoding, adding the access
/// unit delimiters Blu-ray requires.
pub fn passthrough_args(src: &Source, set: &EncodeSettings, audio: &[AudioInput], range: Option<(f64, f64)>, output: &Path) -> Vec<String> {
    title_common(src, set, VideoMode::Copy, audio, range, &Pass::Only, output)
}

/// Encode a menu background clip.
///
/// * `still` – rendered PNG containing the whole static layer (background
///   color/image, texts, images, thumbnails).
/// * `motion` – optional video looped underneath `still` (which then must
///   be transparent where the video should show).
/// * `audio` – optional audio played from a start time for the menu
///   duration.
pub fn menu_args(
    still: &Path,
    motion: Option<(&Path, &MediaInfo, f64)>,
    audio: Option<(&Path, &MediaInfo, f64)>,
    duration: f64,
    set: &EncodeSettings,
    output: &Path,
) -> Vec<String> {
    let (n, d) = set.video.fps();
    let (w, h) = set.video.size();
    let mut a = Vec::new();
    let mut next_input = 0;
    let video_filter;
    if let Some((m, info, start)) = motion {
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
            picture::plan(info, &VideoOptions::default(), set.video).filters
        );
    } else {
        a.extend([s("-loop"), s("1"), s("-framerate"), format!("{n}/{d}"), s("-i"), path(still)]);
        next_input += 1;
        video_filter = format!("[0:v]{}[v]", picture::frame_filters(set.video));
    }
    if let Some((p, _, start)) = audio {
        if start > 0.0 {
            a.extend([s("-ss"), format!("{start:.3}")]);
        }
        a.extend([s("-stream_loop"), s("-1"), s("-i"), path(p)]);
    }
    a.extend([s("-filter_complex"), video_filter, s("-map"), s("[v]")]);
    if audio.is_some() {
        a.extend([s("-map"), format!("{next_input}:a:0")]);
    }
    a.extend([s("-t"), format!("{duration:.3}")]);
    let bitrate = if motion.is_some() { set.video_bitrate } else { set.video_bitrate.min(15_000) };
    a.extend(video_args(set, bitrate, None));
    if let Some((_, info, _)) = audio {
        a.extend(encode_audio(set, 0, output_channels(info.audio_channels)));
    }
    // Leave room before the first frame for the IG stream to be decoded.
    a.extend(mux_args(output, 2.0));
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(video: VideoFormat) -> EncodeSettings {
        EncodeSettings { video, video_bitrate: 18_000, audio: AudioCodec::Ac3, audio_bitrate: None, quality: Quality::Balanced, tune: Tune::None }
    }

    fn arg_after<'a>(a: &'a [String], key: &str) -> Option<&'a str> {
        a.iter().position(|x| x == key).and_then(|i| a.get(i + 1)).map(|s| s.as_str())
    }

    #[test]
    fn audio_options() {
        let info = MediaInfo { duration: 60.0, video_codec: Some("h264".into()), ..Default::default() };
        let opts = VideoOptions::default();
        let src = Source { path: Path::new("/tmp/in.mkv"), info: &info, picture: &opts, burn: None };
        let set = EncodeSettings { audio: AudioCodec::Lpcm24, ..settings(VideoFormat::P1080_23976) };
        let upmix = AudioInput { layout: Some(6), loudness: Some("loudnorm=I=-23".into()), ..AudioInput::encode(None, 0, 0.0, 2) };
        let truehd = AudioInput { copy: Some(AudioCodec::TrueHd), ..AudioInput::encode(None, 1, 0.0, 8) };
        let a = title_args(&src, &set, &[], &[upmix, truehd], None, &Pass::Only, Path::new("/tmp/o.ts"));
        assert_eq!(arg_after(&a, "-sample_fmt:a:0"), Some("s32"));
        assert_eq!(arg_after(&a, "-ac:a:0"), Some("6"));
        assert_eq!(arg_after(&a, "-filter:a:0"), Some("surround=chl_out=5.1,loudnorm=I=-23"));
        assert_eq!(arg_after(&a, "-c:a:1"), Some("copy"));
        assert_eq!(arg_after(&a, "-b:a:2"), Some("640k"));
        // The TrueHD's AC-3 core comes last, from the same source stream.
        assert_eq!(arg_after(&a, "-c:a:2"), Some("ac3"));
        assert_eq!(arg_after(&a, "-ac:a:2"), Some("6"));
        assert_eq!(a.iter().filter(|x| *x == "0:a:1").count(), 2);
    }

    #[test]
    fn automatic_ac3_bitrate() {
        let info = MediaInfo { duration: 60.0, video_codec: Some("h264".into()), ..Default::default() };
        let opts = VideoOptions::default();
        let src = Source { path: Path::new("in.mkv"), info: &info, picture: &opts, burn: None };
        let audio = [AudioInput::encode(None, 0, 0.0, 6), AudioInput::encode(None, 1, 0.0, 2), AudioInput::encode(None, 2, 0.0, 1)];
        let a = title_args(&src, &settings(VideoFormat::P1080_23976), &[], &audio, None, &Pass::Only, Path::new("out.ts"));
        assert_eq!(arg_after(&a, "-b:a:0"), Some("640k"));
        assert_eq!(arg_after(&a, "-b:a:1"), Some("256k"));
        assert_eq!(arg_after(&a, "-b:a:2"), Some("128k"));
        // Stereo spread over 5.1 is encoded as 5.1.
        let upmix = AudioInput { layout: Some(6), ..AudioInput::encode(None, 0, 0.0, 2) };
        let a = title_args(&src, &settings(VideoFormat::P1080_23976), &[], &[upmix], None, &Pass::Only, Path::new("out.ts"));
        assert_eq!(arg_after(&a, "-b:a:0"), Some("640k"));
        // A chosen bitrate applies to every track.
        let set = EncodeSettings { audio_bitrate: Some(448), ..settings(VideoFormat::P1080_23976) };
        let a = title_args(&src, &set, &[], &audio, None, &Pass::Only, Path::new("out.ts"));
        assert_eq!(arg_after(&a, "-b:a:2"), Some("448k"));
    }

    #[test]
    fn tune() {
        let (info, opts) = (MediaInfo::default(), VideoOptions::default());
        let src = Source { path: Path::new("in.mkv"), info: &info, picture: &opts, burn: None };
        let set = settings(VideoFormat::P1080_23976);
        let args = title_args(&src, &set, &[], &[], None, &Pass::Only, Path::new("out.ts"));
        assert_eq!(arg_after(&args, "-tune"), None);
        let args = title_args(&src, &EncodeSettings { tune: Tune::Animation, ..set }, &[], &[], None, &Pass::Only, Path::new("out.ts"));
        assert_eq!(arg_after(&args, "-tune"), Some("animation"));
    }

    #[test]
    fn copied_video_gets_colour_tags() {
        let opts = VideoOptions::default();
        let copy = |info: &MediaInfo, v| {
            let src = Source { path: Path::new("in.mkv"), info, picture: &opts, burn: None };
            let a = passthrough_args(&src, &settings(v), &[], None, Path::new("out.ts"));
            arg_after(&a, "-bsf:v").unwrap().to_string()
        };
        // Untagged HD gets BT.709, limited range.
        let untagged = MediaInfo { video_codec: Some("h264".into()), ..Default::default() };
        assert_eq!(
            copy(&untagged, VideoFormat::P1080_23976),
            "h264_metadata=aud=insert:colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1:video_full_range_flag=0"
        );
        // Untagged SD gets its own standard.
        assert!(copy(&untagged, VideoFormat::I576_25).contains("colour_primaries=5:transfer_characteristics=6:matrix_coefficients=5"));
        // What the source says is kept.
        let tagged = MediaInfo { color_primaries: Some("smpte170m".into()), color_space: Some("smpte170m".into()), ..untagged };
        assert!(copy(&tagged, VideoFormat::P1080_23976).contains("colour_primaries=6:transfer_characteristics=1:matrix_coefficients=6"));
    }

    #[test]
    fn two_pass() {
        let info = MediaInfo { duration: 60.0, video_codec: Some("h264".into()), audio_codec: Some("aac".into()), ..Default::default() };
        let set = EncodeSettings { quality: Quality::Best, ..settings(VideoFormat::P1080_23976) };
        let log = Path::new("/tmp/pass");
        let audio = [AudioInput::encode(None, 0, 0.0, 2)];
        let opts = VideoOptions::default();
        let src = Source { path: Path::new("/tmp/in.mkv"), info: &info, picture: &opts, burn: None };
        let first = title_args(&src, &set, &[], &audio, None, &Pass::First(log.into()), Path::new("/tmp/o.ts"));
        assert_eq!(arg_after(&first, "-pass"), Some("1"));
        assert_eq!(arg_after(&first, "-preset"), Some("slow"));
        assert!(first.ends_with(&["-an".into(), "-f".into(), "null".into(), "-".into()]));
        let second = title_args(&src, &set, &[], &audio, None, &Pass::Second(log.into()), Path::new("/tmp/o.ts"));
        assert_eq!(arg_after(&second, "-pass"), Some("2"));
        assert_eq!(second.last().map(String::as_str), Some("/tmp/o.ts"));
    }
}
