// SPDX-License-Identifier: GPL-3.0-or-later

//! Blu-ray packaging of copied audio. Blu-ray carries audio in PES packets
//! with an extended stream id, and splits HD formats into a core that every
//! player decodes and an extension: DTS-HD into its DTS core and extension
//! substream, Dolby Digital Plus into AC-3 and E-AC-3 frames, and TrueHD
//! next to an AC-3 core of its own.

use super::demux::{pes_payload_offset, Pes};
use crate::bluray::AudioCodec;
use crate::media::probe::ac3_frame_bytes;

/// `stream_id_extension` values.
const EXT_CORE: u8 = 0x71;
const EXT_HD: u8 = 0x72;
const EXT_TRUEHD_AC3: u8 = 0x76;

/// What an input audio stream becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Written as ffmpeg made it (encoded AC-3 and LPCM, copied AC-3).
    AsIs,
    Dts,
    DtsHd,
    Eac3,
    TrueHd,
    /// The AC-3 core that goes with a TrueHD stream.
    TrueHdCore,
}

impl Role {
    pub fn for_codec(codec: AudioCodec) -> Role {
        match codec {
            AudioCodec::Dts => Role::Dts,
            AudioCodec::DtsHdHra | AudioCodec::DtsHdMa => Role::DtsHd,
            AudioCodec::Eac3 => Role::Eac3,
            AudioCodec::TrueHd => Role::TrueHd,
            _ => Role::AsIs,
        }
    }
}

fn push_ts(v: &mut Vec<u8>, prefix: u8, ts: u64) {
    let ts = ts & 0x1_FFFF_FFFF;
    v.push((prefix << 4) | (((ts >> 30) as u8 & 7) << 1) | 1);
    v.push((ts >> 22) as u8);
    v.push((((ts >> 15) as u8) << 1) | 1);
    v.push((ts >> 7) as u8);
    v.push(((ts as u8) << 1) | 1);
}

/// A PES packet with stream id 0xFD and extension `ext`.
fn extended(from: &Pes, payload: &[u8], ext: u8) -> Pes {
    let with_dts = from.dts.is_some_and(|d| Some(d) != from.pts);
    let ts_len = match (from.pts, with_dts) {
        (Some(_), true) => 10,
        (Some(_), false) => 5,
        _ => 0,
    };
    let header_len = ts_len + 3;
    let mut v = Vec::with_capacity(payload.len() + 9 + header_len);
    v.extend_from_slice(&[0, 0, 1, 0xFD]);
    v.extend_from_slice(&((3 + header_len + payload.len()).min(0xFFFF) as u16).to_be_bytes());
    v.push(0x81);
    let flags = match (from.pts, with_dts) {
        (Some(_), true) => 0xC0,
        (Some(_), false) => 0x80,
        _ => 0,
    };
    v.push(flags | 0x01); // PES extension present
    v.push(header_len as u8);
    if let Some(pts) = from.pts {
        push_ts(&mut v, if with_dts { 3 } else { 2 }, pts);
        if let (true, Some(dts)) = (with_dts, from.dts) {
            push_ts(&mut v, 1, dts);
        }
    }
    // PES_extension_flag_2, then its one byte: the stream id extension.
    v.extend_from_slice(&[0x01, 0x81, ext]);
    v.extend_from_slice(payload);
    Pes { stream: from.stream, data: v, pts: from.pts, dts: from.dts, random_access: from.random_access }
}

/// Split DTS-HD frames into their DTS cores and extension substreams.
fn split_dts_hd(payload: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut core, mut ext) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i + 8 <= payload.len() {
        if payload[i..i + 4] == [0x7F, 0xFE, 0x80, 0x01] {
            // FSIZE: 14 bits from bit 46 of the core frame header.
            let fsize = (((payload[i + 5] as usize & 0x03) << 12) | ((payload[i + 6] as usize) << 4) | (payload[i + 7] as usize >> 4)) + 1;
            let end = (i + fsize.max(8)).min(payload.len());
            core.extend_from_slice(&payload[i..end]);
            i = end;
        } else if payload[i..i + 4] == [0x64, 0x58, 0x20, 0x25] && i + 10 <= payload.len() {
            // Extension substream: size in its header (12 or 16 bits by the
            // header type bit).
            let b = &payload[i + 4..];
            let blownup = b[1] & 0x20 != 0;
            let size = if blownup {
                ((((b[2] as usize) & 0x01) << 19) | ((b[3] as usize) << 11) | ((b[4] as usize) << 3) | (b[5] as usize >> 5)) + 1
            } else {
                ((((b[2] as usize) & 0x1F) << 11) | ((b[3] as usize) << 3) | (b[4] as usize >> 5)) + 1
            };
            let end = (i + size.max(8)).min(payload.len());
            ext.extend_from_slice(&payload[i..end]);
            i = end;
        } else {
            // Unknown data stays with whichever part came last.
            if ext.is_empty() { &mut core } else { &mut ext }.push(payload[i]);
            i += 1;
        }
    }
    let rest = &payload[i..];
    if ext.is_empty() { &mut core } else { &mut ext }.extend_from_slice(rest);
    (core, ext)
}

/// Split Dolby Digital Plus into AC-3 frames and E-AC-3 frames.
fn split_eac3(payload: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut core, mut ext) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i + 6 <= payload.len() && payload[i] == 0x0B && payload[i + 1] == 0x77 {
        let bsid = payload[i + 5] >> 3;
        let size = if bsid <= 10 {
            ac3_frame_bytes(payload[i + 4] >> 6, payload[i + 4] & 0x3F)
        } else {
            (((payload[i + 2] as usize & 0x07) << 8 | payload[i + 3] as usize) + 1) * 2
        };
        let end = (i + size.max(6)).min(payload.len());
        if bsid <= 8 { &mut core } else { &mut ext }.extend_from_slice(&payload[i..end]);
        i = end;
    }
    ext.extend_from_slice(&payload[i..]);
    (core, ext)
}

/// Repackage an audio PES from ffmpeg for Blu-ray.
pub fn convert(pes: Pes, role: Role) -> Vec<Pes> {
    if role == Role::AsIs {
        return vec![pes];
    }
    let payload = pes.data[pes_payload_offset(&pes.data)..].to_vec();
    let pair = |core: Vec<u8>, ext: Vec<u8>| {
        let mut out = Vec::new();
        if !core.is_empty() {
            out.push(extended(&pes, &core, EXT_CORE));
        }
        if !ext.is_empty() {
            out.push(extended(&pes, &ext, EXT_HD));
        }
        out
    };
    match role {
        Role::AsIs => unreachable!(),
        Role::Dts => vec![extended(&pes, &payload, EXT_CORE)],
        Role::DtsHd => {
            let (core, ext) = split_dts_hd(&payload);
            pair(core, ext)
        }
        Role::Eac3 => {
            let (core, ext) = split_eac3(&payload);
            pair(core, ext)
        }
        Role::TrueHd => vec![extended(&pes, &payload, EXT_HD)],
        Role::TrueHdCore => vec![extended(&pes, &payload, EXT_TRUEHD_AC3)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bluray::ts::demux::pes_timestamps;

    fn pes(payload: &[u8], pts: u64) -> Pes {
        let data = crate::bluray::ts::mux::private_pes(payload, pts, pts);
        Pes { stream: 1, data, pts: Some(pts), dts: None, random_access: false }
    }

    #[test]
    fn extended_header() {
        let out = convert(pes(&[1, 2, 3], 90_000), Role::TrueHdCore);
        let d = &out[0].data;
        assert_eq!(&d[..4], &[0, 0, 1, 0xFD]);
        assert_eq!(pes_timestamps(d), (Some(90_000), None));
        assert_eq!(pes_payload_offset(d), d.len() - 3);
        assert_eq!(d[d.len() - 4], EXT_TRUEHD_AC3);
        assert_eq!(u16::from_be_bytes([d[4], d[5]]) as usize, d.len() - 6);
    }

    #[test]
    fn splits_dts_hd() {
        // A core frame of 16 bytes (FSIZE 15), then a 12-byte substream.
        let mut core = vec![0x7F, 0xFE, 0x80, 0x01, 0, 0, 0, 0];
        core[6] = 15 >> 4;
        core[7] = (15 & 0x0F) << 4;
        core.resize(16, 0xAA);
        let mut ext = vec![0x64, 0x58, 0x20, 0x25, 0, 0, 0, 0, 0];
        // Size 12 → 11 in 12 bits at bits 12..: b[2] low 5 bits, b[3], b[4] top 3.
        ext[4 + 3] = (11 >> 3) as u8;
        ext[4 + 4] = ((11 & 7) << 5) as u8;
        ext.resize(12, 0xBB);
        let frame = [core.clone(), ext.clone()].concat();
        let out = convert(pes(&frame, 1000), Role::DtsHd);
        assert_eq!(out.len(), 2);
        assert_eq!(&out[0].data[pes_payload_offset(&out[0].data)..], &core[..]);
        assert_eq!(&out[1].data[pes_payload_offset(&out[1].data)..], &ext[..]);
        assert_eq!(out[1].pts, Some(1000));
    }

    #[test]
    fn splits_dolby_digital_plus() {
        let mut ac3 = vec![0x0B, 0x77, 0, 0, 0x1E, 8 << 3];
        ac3.resize(ac3_frame_bytes(0, 0x1E), 1);
        let mut eac3 = vec![0x0B, 0x77, 0x00, 0x03, 0, 16 << 3];
        eac3.resize(8, 2);
        let out = convert(pes(&[ac3.clone(), eac3.clone()].concat(), 0), Role::Eac3);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].data[pes_payload_offset(&out[0].data)..].len(), ac3.len());
        assert_eq!(&out[1].data[pes_payload_offset(&out[1].data)..], &eac3[..]);
    }
}
