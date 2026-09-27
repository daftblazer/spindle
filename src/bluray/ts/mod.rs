// SPDX-License-Identifier: GPL-3.0-or-later

//! Transport stream handling: demuxing ffmpeg output and muxing BDAV clips.

pub mod audio;
pub mod demux;
pub mod mux;
pub mod psi;

use crate::bluray::{EsInfo, EsKind};
use anyhow::{bail, Context, Result};
use audio::Role;
use demux::{Demuxer, Pes};
use mux::{MuxStats, Muxer};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Remux an ffmpeg-produced transport stream into a BDAV clip.
///
/// `streams` describes the output streams; the first video and the audio
/// streams of the input are mapped in order onto the output's video and
/// audio entries. Further input audio streams are the AC-3 cores of the
/// TrueHD outputs, in order. `extra` holds additional PES packets (e.g. IG
/// segments) as `(output stream index, pes)`.
pub fn remux(
    input: &Path,
    output: &Path,
    streams: &[EsInfo],
    extra: Vec<(usize, Pes)>,
    mut progress: impl FnMut(u64) -> bool,
) -> Result<MuxStats> {
    let mut demux = Demuxer::open(input)?;
    demux.read_headers()?;

    let video_out = streams.iter().position(|s| matches!(s.kind, EsKind::Video(_)));
    let role = |i: usize| match streams[i].kind {
        EsKind::Audio { codec, .. } => Role::for_codec(codec),
        _ => Role::AsIs,
    };
    let audio: Vec<usize> = streams.iter().enumerate().filter(|(_, s)| matches!(s.kind, EsKind::Audio { .. })).map(|(i, _)| i).collect();
    let mut audio_out = audio.iter().map(|&i| (i, role(i))).chain(audio.iter().filter(|&&i| role(i) == Role::TrueHd).map(|&i| (i, Role::TrueHdCore)));

    let mut map: Vec<Option<(usize, Role)>> = Vec::new();
    let mut have_video = false;
    for s in &demux.streams {
        let out = match s.stream_type {
            0x1b if !have_video => {
                have_video = true;
                video_out.map(|v| (v, Role::AsIs))
            }
            0x80..=0x86 | 0x06 | 0x03 | 0x04 | 0x0f | 0x87 => audio_out.next(),
            _ => None,
        };
        map.push(out);
    }
    if !have_video {
        bail!("{} has no H.264 video stream", input.display());
    }

    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut muxer = Muxer::new(BufWriter::with_capacity(1 << 20, file), streams.to_vec());
    for (s, pes) in extra {
        muxer.push(s, pes);
    }
    // Audio split into core and extension comes out as two packets.
    let mut pending: std::collections::VecDeque<(usize, Pes)> = Default::default();
    muxer.run(
        || {
            loop {
                if let Some(p) = pending.pop_front() {
                    return Ok(Some(p));
                }
                let Some(pes) = demux.next_pes()? else { return Ok(None) };
                if let Some((out, role)) = map[pes.stream] {
                    pending.extend(audio::convert(pes, role).into_iter().map(|p| (out, p)));
                }
            }
        },
        &mut progress,
    )
}

/// PTS of the first video access unit in a transport stream.
pub fn first_video_pts(input: &Path) -> Result<u64> {
    let mut demux = Demuxer::open(input)?;
    demux.read_headers()?;
    let mut min: Option<u64> = None;
    let mut seen = 0;
    while let Some(pes) = demux.next_pes()? {
        if demux.streams[pes.stream].stream_type == 0x1b {
            if let Some(pts) = pes.pts {
                min = Some(min.map_or(pts, |m| m.min(pts)));
            }
            seen += 1;
            // B-frames may precede in PTS order; a few frames suffice.
            if seen > 16 {
                break;
            }
        }
    }
    min.with_context(|| format!("{} has no video", input.display()))
}
