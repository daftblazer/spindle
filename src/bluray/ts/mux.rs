// SPDX-License-Identifier: GPL-3.0-or-later

//! BDAV (.m2ts) muxer.
//!
//! Takes whole PES packets with timestamps (from ffmpeg output via
//! [`super::demux`] and from the IG encoder) and schedules them into 192-byte
//! source packets with arrival timestamps, PCR and PSI, collecting the EP map
//! needed by the clip info file.

use super::demux::Pes;
use super::psi::{self, PID_NULL, PID_PAT, PID_SIT};
use crate::bluray::nav::clpi::{i_end_offset_class, EpEntry};
use crate::bluray::{EsInfo, EsKind, PID_PCR, PID_PMT};
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::io::Write;

/// 27 MHz system clock.
const SYSTEM_CLOCK: u64 = 27_000_000;
/// Maximum BD-ROM transport rate.
pub const MUX_RATE: u64 = 48_000_000;
const PCR_INTERVAL: u64 = SYSTEM_CLOCK * 30 / 1000;
const PSI_INTERVAL: u64 = SYSTEM_CLOCK / 10;
/// How far ahead of the mux clock to read from the source (90 kHz).
const LOOKAHEAD_90K: u64 = 3 * 90_000;

/// Packets per aligned unit (6144 bytes).
const ALIGNED_UNIT: u32 = 32;

#[derive(Debug, Clone, Default)]
pub struct MuxStats {
    pub num_packets: u32,
    pub ep_map: Vec<EpEntry>,
    /// Earliest video PTS and latest video PTS (90 kHz).
    pub first_video_pts: Option<u64>,
    pub last_video_pts: Option<u64>,
}

struct Item {
    data: Vec<u8>,
    pts: Option<u64>,
    dts: u64,
    random_access: bool,
    sent: usize,
}

struct OutStream {
    info: EsInfo,
    /// Maximum time a PES may be sent ahead of its DTS (27 MHz).
    lead: u64,
    queue: VecDeque<Item>,
}

/// Build a PES packet for private stream 1 (used for IG segments).
pub fn private_pes(payload: &[u8], pts: u64, dts: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 19);
    let with_dts = dts != pts;
    let hdr_data_len = if with_dts { 10 } else { 5 };
    let pes_len = 3 + hdr_data_len + payload.len();
    v.extend_from_slice(&[0, 0, 1, 0xBD]);
    v.extend_from_slice(&(pes_len.min(0xFFFF) as u16).to_be_bytes());
    v.push(0x81); // '10' + original
    v.push(if with_dts { 0xC0 } else { 0x80 });
    v.push(hdr_data_len as u8);
    push_ts(&mut v, if with_dts { 3 } else { 2 }, pts);
    if with_dts {
        push_ts(&mut v, 1, dts);
    }
    v.extend_from_slice(payload);
    v
}

fn push_ts(v: &mut Vec<u8>, prefix: u8, ts: u64) {
    let ts = ts & 0x1_FFFF_FFFF;
    v.push((prefix << 4) | (((ts >> 30) as u8 & 7) << 1) | 1);
    v.push((ts >> 22) as u8);
    v.push((((ts >> 15) as u8) << 1) | 1);
    v.push((ts >> 7) as u8);
    v.push(((ts as u8) << 1) | 1);
}

pub struct Muxer<W: Write> {
    out: W,
    streams: Vec<OutStream>,
    now: u64,
    pkt_ticks: u64,
    cc: HashMap<u16, u8>,
    spn: u32,
    last_pcr: Option<u64>,
    last_psi: Option<u64>,
    /// Decode time of the last PES read from the source (streams preloaded
    /// with [`Self::push`], like subtitles, don't count as lookahead).
    source_dts: Option<u64>,
    pat: Vec<u8>,
    pmt: Vec<u8>,
    sit: Vec<u8>,
    stats: MuxStats,
}

impl<W: Write> Muxer<W> {
    pub fn new(out: W, infos: Vec<EsInfo>) -> Self {
        let streams = infos
            .iter()
            .map(|info| {
                let lead = match info.kind {
                    EsKind::Video(_) => SYSTEM_CLOCK * 4 / 10,
                    EsKind::Audio { .. } => SYSTEM_CLOCK * 15 / 100,
                    EsKind::Ig { .. } => SYSTEM_CLOCK * 2,
                    EsKind::Pg { .. } => SYSTEM_CLOCK,
                };
                OutStream { info: info.clone(), lead, queue: VecDeque::new() }
            })
            .collect();
        Muxer {
            out,
            streams,
            now: 0,
            pkt_ticks: 188 * 8 * SYSTEM_CLOCK / MUX_RATE,
            cc: HashMap::new(),
            spn: 0,
            last_pcr: None,
            last_psi: None,
            source_dts: None,
            pat: psi::pat(PID_PMT),
            pmt: psi::pmt(&infos),
            sit: psi::sit(MUX_RATE),
            stats: MuxStats::default(),
        }
    }

    /// Queue a PES packet for output stream `stream` (index into the
    /// `infos` passed to [`Self::new`]).
    pub fn push(&mut self, stream: usize, pes: Pes) {
        let dts = pes.decode_ts();
        self.streams[stream].queue.push_back(Item {
            data: pes.data,
            pts: pes.pts,
            dts,
            random_access: pes.random_access,
            sent: 0,
        });
    }


    /// Run the mux loop. `source` supplies PES packets in (roughly) decode
    /// order as `(output stream index, pes)`; `progress` receives the current
    /// mux time in 90 kHz ticks and returns false to cancel.
    pub fn run(
        mut self,
        mut source: impl FnMut() -> Result<Option<(usize, Pes)>>,
        mut progress: impl FnMut(u64) -> bool,
    ) -> Result<MuxStats> {
        let mut source_done = false;
        let mut fill = |m: &mut Self| -> Result<()> {
            while !source_done {
                let horizon = m.now / 300 + LOOKAHEAD_90K;
                if m.source_dts.is_some_and(|t| t > horizon) {
                    break;
                }
                match source()? {
                    Some((s, pes)) => {
                        m.source_dts = Some(m.source_dts.map_or(pes.decode_ts(), |t| t.max(pes.decode_ts())));
                        m.push(s, pes)
                    }
                    None => source_done = true,
                }
            }
            Ok(())
        };

        fill(&mut self)?;
        let start = self
            .streams
            .iter()
            .filter_map(|s| s.queue.front().map(|i| (i.dts * 300).saturating_sub(s.lead)))
            .min()
            .unwrap_or(0);
        self.now = start.saturating_sub(SYSTEM_CLOCK / 20);

        let mut since_progress = 0u32;
        loop {
            fill(&mut self)?;
            if self.streams.iter().all(|s| s.queue.is_empty()) {
                break;
            }
            since_progress += 1;
            if since_progress >= 4096 {
                since_progress = 0;
                if !progress(self.now / 300) {
                    anyhow::bail!("cancelled");
                }
            }

            if self.last_psi.is_none_or(|t| self.now >= t + PSI_INTERVAL) {
                self.write_psi()?;
            }
            if self.last_pcr.is_none_or(|t| self.now >= t + PCR_INTERVAL) {
                self.write_pcr()?;
            }

            // Pick the eligible stream with the earliest deadline.
            let mut best: Option<(usize, u64)> = None;
            let mut next_eligible = u64::MAX;
            for (i, s) in self.streams.iter().enumerate() {
                if let Some(head) = s.queue.front() {
                    let earliest = (head.dts * 300).saturating_sub(s.lead);
                    if earliest <= self.now {
                        if best.is_none_or(|(_, d)| head.dts < d) {
                            best = Some((i, head.dts));
                        }
                    } else {
                        next_eligible = next_eligible.min(earliest);
                    }
                }
            }

            match best {
                Some((i, _)) => self.write_pes_packet(i)?,
                None => {
                    // Idle until the next PES may be sent, keeping PCR flowing.
                    let pcr_due = self.last_pcr.unwrap_or(self.now) + PCR_INTERVAL;
                    self.now = self.now.max(next_eligible.min(pcr_due));
                }
            }
        }

        // Pad to a whole aligned unit.
        while !self.spn.is_multiple_of(ALIGNED_UNIT) {
            let mut payload = [0xFFu8; 184];
            payload[0] = 0xFF;
            self.write_ts(PID_NULL, false, None, false, &payload)?;
        }
        self.out.flush()?;
        self.stats.num_packets = self.spn;
        Ok(self.stats)
    }

    fn write_psi(&mut self) -> Result<()> {
        for (pid, section) in [(PID_PAT, self.pat.clone()), (PID_PMT, self.pmt.clone()), (PID_SIT, self.sit.clone())] {
            let mut payload = vec![0u8]; // pointer_field
            payload.extend_from_slice(&section);
            payload.resize(184, 0xFF);
            self.write_ts(pid, true, None, false, &payload)?;
        }
        self.last_psi = Some(self.now);
        Ok(())
    }

    fn write_pcr(&mut self) -> Result<()> {
        let pcr = self.now;
        self.write_ts(PID_PCR, false, Some(pcr), false, &[])?;
        self.last_pcr = Some(pcr);
        Ok(())
    }

    fn write_pes_packet(&mut self, stream: usize) -> Result<()> {
        let pid = self.streams[stream].info.pid;
        let is_video = matches!(self.streams[stream].info.kind, EsKind::Video(_));
        let head = self.streams[stream].queue.front().unwrap();
        let first = head.sent == 0;
        let random_access = first && head.random_access;
        let max_payload = if random_access { 182 } else { 184 };
        let n = (head.data.len() - head.sent).min(max_payload);
        let chunk = head.data[head.sent..head.sent + n].to_vec();

        if first && is_video {
            if let Some(pts) = head.pts {
                self.stats.first_video_pts = Some(self.stats.first_video_pts.map_or(pts, |p| p.min(pts)));
                self.stats.last_video_pts = Some(self.stats.last_video_pts.map_or(pts, |p| p.max(pts)));
                if head.random_access {
                    let packets = head.data.len().div_ceil(184) as u32;
                    self.stats.ep_map.push(EpEntry {
                        pts,
                        spn: self.spn,
                        i_end_offset: i_end_offset_class(packets),
                    });
                }
            }
        }

        self.write_ts(pid, first, None, random_access, &chunk)?;
        let head = self.streams[stream].queue.front_mut().unwrap();
        head.sent += n;
        if head.sent >= head.data.len() {
            self.streams[stream].queue.pop_front();
        }
        Ok(())
    }

    /// Write one source packet. `payload` may be shorter than 184 bytes, in
    /// which case adaptation field stuffing is added.
    fn write_ts(&mut self, pid: u16, pusi: bool, pcr: Option<u64>, random_access: bool, payload: &[u8]) -> Result<()> {
        let mut pkt = [0xFFu8; 192];
        let ats = (self.now & 0x3FFF_FFFF) as u32;
        pkt[0..4].copy_from_slice(&ats.to_be_bytes()); // copy_permission_indicator = 0

        let has_payload = !payload.is_empty();
        let need_af = pcr.is_some() || random_access || payload.len() < 184;
        let afc = match (need_af, has_payload) {
            (true, true) => 3,
            (true, false) => 2,
            (false, _) => 1,
        };
        let cc = self.cc.entry(pid).or_insert(0);
        let this_cc = *cc;
        if has_payload {
            *cc = (*cc + 1) & 0x0F;
        }

        let t = &mut pkt[4..];
        t[0] = 0x47;
        t[1] = ((pusi as u8) << 6) | ((pid >> 8) as u8 & 0x1F);
        t[2] = pid as u8;
        t[3] = (afc << 4) | this_cc;
        let mut off = 4;
        if need_af {
            let af_len = 184 - payload.len() - 1;
            t[4] = af_len as u8;
            if af_len > 0 {
                let mut flags = 0u8;
                if random_access {
                    flags |= 0x40;
                }
                let mut p = 6;
                if let Some(pcr) = pcr {
                    flags |= 0x10;
                    let base = (pcr / 300) & 0x1_FFFF_FFFF;
                    let ext = pcr % 300;
                    t[6] = (base >> 25) as u8;
                    t[7] = (base >> 17) as u8;
                    t[8] = (base >> 9) as u8;
                    t[9] = (base >> 1) as u8;
                    t[10] = (((base & 1) as u8) << 7) | 0x7E | ((ext >> 8) as u8 & 1);
                    t[11] = ext as u8;
                    p = 12;
                }
                t[5] = flags;
                debug_assert!(p <= 5 + af_len);
            }
            off = 5 + af_len;
        }
        t[off..off + payload.len()].copy_from_slice(payload);
        self.out.write_all(&pkt)?;
        self.spn += 1;
        self.now += self.pkt_ticks;
        Ok(())
    }
}
