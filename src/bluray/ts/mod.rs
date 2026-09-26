// SPDX-License-Identifier: GPL-3.0-or-later

//! Transport stream handling: demuxing ffmpeg output and muxing BDAV clips.

pub mod demux;
pub mod mux;
pub mod psi;

use crate::bluray::{EsInfo, EsKind};
use anyhow::{bail, Context, Result};
use demux::{Demuxer, Pes};
use mux::{MuxStats, Muxer};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Remux an ffmpeg-produced transport stream into a BDAV clip.
///
/// `streams` describes the output streams; the first video and the audio
/// streams of the input are mapped in order onto the output's video and
/// audio entries. `extra` holds additional PES packets (e.g. IG segments) as
/// `(output stream index, pes)`.
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
    let mut audio_out = streams
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s.kind, EsKind::Audio { .. }))
        .map(|(i, _)| i);

    let mut map: Vec<Option<usize>> = Vec::new();
    let mut have_video = false;
    for s in &demux.streams {
        let out = match s.stream_type {
            0x1b if !have_video => {
                have_video = true;
                video_out
            }
            0x80 | 0x81 | 0x06 | 0x03 | 0x04 | 0x0f => audio_out.next(),
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
    muxer.run(
        || {
            while let Some(pes) = demux.next_pes()? {
                if let Some(out) = map[pes.stream] {
                    return Ok(Some((out, pes)));
                }
            }
            Ok(None)
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
