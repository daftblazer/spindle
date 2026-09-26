// SPDX-License-Identifier: GPL-3.0-or-later

//! `BDMV/CLIPINF/xxxxx.clpi` writer.

use crate::bluray::bytes::BitWriter;
use crate::bluray::{lang3, EsInfo, EsKind, PID_PCR, PID_PMT, PID_VIDEO};

/// Random access point in the clip's video stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpEntry {
    /// PTS in 90 kHz ticks.
    pub pts: u64,
    /// Source packet number of the packet starting the I picture.
    pub spn: u32,
    /// Size class of the I picture (1..=7).
    pub i_end_offset: u8,
}

#[derive(Debug, Clone)]
pub struct ClipInfo {
    /// TS recording rate in bytes per second.
    pub ts_recording_rate: u32,
    pub num_source_packets: u32,
    /// Presentation start/end in 45 kHz ticks.
    pub presentation_start: u32,
    pub presentation_end: u32,
    pub streams: Vec<EsInfo>,
    pub ep_map: Vec<EpEntry>,
}

/// I_end_position_offset class for an I picture occupying `packets` source packets.
pub fn i_end_offset_class(packets: u32) -> u8 {
    let bytes = packets as u64 * 192;
    match bytes {
        0..=131_071 => 1,
        131_072..=262_143 => 2,
        262_144..=393_215 => 3,
        393_216..=589_823 => 4,
        589_824..=917_503 => 5,
        917_504..=1_310_719 => 6,
        _ => 7,
    }
}

fn write_coding_info(w: &mut BitWriter, s: &EsInfo) {
    let len = w.begin_len8();
    w.u8(s.coding_type());
    match &s.kind {
        EsKind::Video(v) => {
            w.bits(4, v.format_code() as u64)
                .bits(4, v.rate_code() as u64)
                .bits(4, 3 /* 16:9 */)
                .zeros(2)
                .flag(false /* cc */)
                .zeros(17);
        }
        EsKind::Audio { channels, lang, .. } => {
            w.bits(4, EsInfo::audio_format_code(*channels) as u64).bits(4, 1 /* 48 kHz */);
            w.ascii(&lang3(lang), 3);
        }
        EsKind::Ig { lang } | EsKind::Pg { lang } => {
            w.ascii(&lang3(lang), 3);
            w.u8(0);
        }
    }
    // ISRC + reserved; the stream coding info block is 21 bytes long.
    let written = w.len() - len - 1;
    w.bytes(&vec![0u8; 21 - written]);
    w.end_len8(len);
}

/// (ref_to_EP_fine_id, PTS_EP_coarse, SPN_EP_coarse)
type CoarseEntry = (u32, u16, u32);
/// (I_end_position_offset, PTS_EP_fine, SPN_EP_fine)
type FineEntry = (u8, u16, u32);

/// Split the EP map into coarse and fine tables as described by the
/// BD-ROM spec (and read by libbluray's `clpi_lookup_spn`).
fn split_ep_map(ep: &[EpEntry]) -> (Vec<CoarseEntry>, Vec<FineEntry>) {
    let mut coarse = Vec::new();
    let mut fine = Vec::new();
    let mut last: Option<(u64, u32)> = None;
    for (i, e) in ep.iter().enumerate() {
        let pts45 = (e.pts >> 1) & 0xFFFF_FFFF;
        let coarse_pts = ((pts45 >> 18) & 0x3FFF) as u16;
        let fine_pts = ((pts45 >> 8) & 0x7FF) as u16;
        let key = (pts45 >> 19, e.spn >> 17);
        if last != Some(key) {
            coarse.push((i as u32, coarse_pts, e.spn));
            last = Some(key);
        }
        fine.push((e.i_end_offset, fine_pts, e.spn & 0x1FFFF));
    }
    (coarse, fine)
}

impl ClipInfo {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ascii("HDMV", 4).ascii("0200", 4);
        let addrs = w.len();
        w.u32(0).u32(0).u32(0).u32(0).u32(0);
        w.pad_to(40);

        // ClipInfo
        let len = w.begin_len32();
        w.u16(0);
        w.u8(1); // clip_stream_type: AV stream
        w.u8(1); // application_type: main TS for main path of movie
        w.zeros(31).flag(false); // is_ATC_delta
        w.u32(self.ts_recording_rate);
        w.u32(self.num_source_packets);
        w.bytes(&[0; 128]);
        let tlen = w.begin_len16();
        w.u8(0x80).ascii("HDMV", 4).bytes(&[0; 25]);
        w.end_len16(tlen);
        w.end_len32(len);

        // SequenceInfo
        let pos = w.len();
        w.patch_u32(addrs, pos as u32);
        let len = w.begin_len32();
        w.u8(0).u8(1); // one ATC sequence
        w.u32(0); // SPN_ATC_start
        w.u8(1).u8(0); // one STC sequence, offset_STC_id
        w.u16(PID_PCR).u32(0).u32(self.presentation_start).u32(self.presentation_end);
        w.end_len32(len);

        // ProgramInfo
        let pos = w.len();
        w.patch_u32(addrs + 4, pos as u32);
        let len = w.begin_len32();
        w.u8(0).u8(1); // one program
        w.u32(0).u16(PID_PMT).u8(self.streams.len() as u8).u8(0);
        for s in &self.streams {
            w.u16(s.pid);
            write_coding_info(&mut w, s);
        }
        w.end_len32(len);

        // CPI (EP_map)
        let pos = w.len();
        w.patch_u32(addrs + 8, pos as u32);
        let len = w.begin_len32();
        if !self.ep_map.is_empty() {
            w.zeros(12).bits(4, 1); // CPI_type = EP_map
            let ep_map_start = w.len();
            let (coarse, fine) = split_ep_map(&self.ep_map);
            w.u8(0).u8(1); // one stream PID
            w.u16(PID_VIDEO)
                .zeros(10)
                .bits(4, 1) // EP_stream_type: video
                .u16(coarse.len() as u16)
                .bits(18, fine.len() as u64);
            let start_addr_at = w.len();
            w.u32(0);

            let stream_start = w.len();
            w.patch_u32(start_addr_at, (stream_start - ep_map_start) as u32);
            let fine_start_at = w.len();
            w.u32(0);
            for (fine_id, pts, spn) in &coarse {
                w.bits(18, *fine_id as u64).bits(14, *pts as u64).u32(*spn);
            }
            let fine_start = w.len();
            w.patch_u32(fine_start_at, (fine_start - stream_start) as u32);
            for (i_end, pts, spn) in &fine {
                w.flag(false).bits(3, *i_end as u64).bits(11, *pts as u64).bits(17, *spn as u64);
            }
        }
        w.end_len32(len);

        // ClipMark (empty)
        let pos = w.len();
        w.patch_u32(addrs + 12, pos as u32);
        w.u32(0);

        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ep_split_reconstructs_pts_and_spn() {
        let ep: Vec<EpEntry> = (0..500u64)
            .map(|i| EpEntry { pts: 180_000 + i * 90_090, spn: (i * 3000) as u32, i_end_offset: 1 })
            .collect();
        let (coarse, fine) = split_ep_map(&ep);
        for (ci, &(ref_id, cpts, cspn)) in coarse.iter().enumerate() {
            let end = coarse.get(ci + 1).map_or(fine.len() as u32, |c| c.0);
            for fi in ref_id..end {
                let (_, fpts, fspn) = fine[fi as usize];
                let pts45 = ((cpts as u64 & !1) << 18) + ((fpts as u64) << 8);
                let spn = (cspn & !0x1FFFF) + fspn;
                let e = ep[fi as usize];
                assert_eq!(pts45, ((e.pts >> 1) & 0xFFFF_FFFF) & !0xFF);
                assert_eq!(spn, e.spn);
            }
        }
    }
}
