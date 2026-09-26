// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal MPEG-TS / M2TS demuxer that yields whole PES packets.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// A complete PES packet (header included) from the input stream.
#[derive(Debug, Clone)]
pub struct Pes {
    /// Index into [`Demuxer::streams`].
    pub stream: usize,
    pub data: Vec<u8>,
    pub pts: Option<u64>,
    pub dts: Option<u64>,
    /// Starts a random access point (IDR / keyframe).
    pub random_access: bool,
}

impl Pes {
    pub fn decode_ts(&self) -> u64 {
        self.dts.or(self.pts).unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub struct InputStream {
    pub stream_type: u8,
}

pub struct Demuxer {
    reader: BufReader<File>,
    packet_size: usize,
    pmt_pid: Option<u16>,
    pub streams: Vec<InputStream>,
    pid_to_stream: HashMap<u16, usize>,
    partial: HashMap<usize, Pes>,
    ready: std::collections::VecDeque<Pes>,
    eof: bool,
}

pub fn parse_timestamp(b: &[u8]) -> u64 {
    (((b[0] as u64 >> 1) & 0x07) << 30)
        | ((b[1] as u64) << 22)
        | (((b[2] as u64) >> 1) << 15)
        | ((b[3] as u64) << 7)
        | ((b[4] as u64) >> 1)
}

/// Parse PTS/DTS out of a PES header.
pub fn pes_timestamps(data: &[u8]) -> (Option<u64>, Option<u64>) {
    if data.len() < 9 || data[0..3] != [0, 0, 1] {
        return (None, None);
    }
    let flags = data[7] >> 6;
    let pts = (flags & 2 != 0 && data.len() >= 14).then(|| parse_timestamp(&data[9..14]));
    let dts = (flags == 3 && data.len() >= 19).then(|| parse_timestamp(&data[14..19]));
    (pts, dts)
}

/// Offset of the PES payload (after the header).
pub fn pes_payload_offset(data: &[u8]) -> usize {
    if data.len() < 9 {
        return data.len();
    }
    (9 + data[8] as usize).min(data.len())
}

/// Does this H.264 access unit contain an IDR slice or a recovery point SEI?
pub fn h264_is_keyframe(payload: &[u8]) -> bool {
    let mut i = 0;
    let limit = payload.len().min(64 * 1024);
    while i + 3 < limit {
        if payload[i] == 0 && payload[i + 1] == 0 && payload[i + 2] == 1 {
            let nal = payload[i + 3] & 0x1F;
            match nal {
                5 => return true,
                // SEI: recovery point (payload type 6)
                6 if payload.get(i + 4) == Some(&6) => return true,
                1 => return false,
                _ => {}
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

impl Demuxer {
    pub fn open(path: &Path) -> Result<Self> {
        let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut probe = [0u8; 192 * 3];
        f.read_exact(&mut probe).context("input too short to be a transport stream")?;
        let packet_size = if probe[0] == 0x47 && probe[188] == 0x47 && probe[376] == 0x47 {
            188
        } else if probe[4] == 0x47 && probe[196] == 0x47 && probe[388] == 0x47 {
            192
        } else {
            bail!("{} is not an MPEG transport stream", path.display());
        };
        let f = File::open(path)?;
        Ok(Demuxer {
            reader: BufReader::with_capacity(1 << 20, f),
            packet_size,
            pmt_pid: None,
            streams: Vec::new(),
            pid_to_stream: HashMap::new(),
            partial: HashMap::new(),
            ready: Default::default(),
            eof: false,
        })
    }

    /// Read packets until the PMT has been seen, so [`Self::streams`] is populated.
    pub fn read_headers(&mut self) -> Result<()> {
        while self.streams.is_empty() {
            if !self.read_packet()? {
                bail!("no PMT found in transport stream");
            }
        }
        Ok(())
    }

    /// Next complete PES packet, or None at end of stream.
    pub fn next_pes(&mut self) -> Result<Option<Pes>> {
        loop {
            if let Some(p) = self.ready.pop_front() {
                return Ok(Some(p));
            }
            if self.eof {
                return Ok(None);
            }
            if !self.read_packet()? {
                self.eof = true;
                // flush partial PES packets in stream order
                let mut rest: Vec<Pes> = self.partial.drain().map(|(_, p)| p).collect();
                rest.sort_by_key(|p| p.decode_ts());
                self.ready.extend(rest);
            }
        }
    }

    fn read_packet(&mut self) -> Result<bool> {
        let mut buf = [0u8; 192];
        let pkt = &mut buf[..self.packet_size];
        match self.reader.read_exact(pkt) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(e) => return Err(e.into()),
        }
        let p = if self.packet_size == 192 { &buf[4..192] } else { &buf[..188] };
        if p[0] != 0x47 {
            bail!("lost transport stream sync");
        }
        let pusi = p[1] & 0x40 != 0;
        let pid = ((p[1] as u16 & 0x1F) << 8) | p[2] as u16;
        let afc = (p[3] >> 4) & 3;
        let mut off = 4;
        let mut random_access = false;
        if afc & 2 != 0 {
            let af_len = p[4] as usize;
            if af_len > 0 && p[5] & 0x40 != 0 {
                random_access = true;
            }
            off = 5 + af_len;
        }
        if afc & 1 == 0 || off >= 188 {
            return Ok(true);
        }
        let payload = &p[off..188];

        if pid == 0 {
            if pusi {
                self.parse_pat(payload);
            }
        } else if Some(pid) == self.pmt_pid {
            if pusi && self.streams.is_empty() {
                self.parse_pmt(payload);
            }
        } else if let Some(&stream) = self.pid_to_stream.get(&pid) {
            if pusi {
                if let Some(done) = self.partial.remove(&stream) {
                    self.finish(done);
                }
                self.partial.insert(
                    stream,
                    Pes { stream, data: payload.to_vec(), pts: None, dts: None, random_access },
                );
            } else if let Some(cur) = self.partial.get_mut(&stream) {
                cur.data.extend_from_slice(payload);
            }
        }
        Ok(true)
    }

    fn finish(&mut self, mut pes: Pes) {
        let (pts, dts) = pes_timestamps(&pes.data);
        pes.pts = pts;
        pes.dts = dts;
        if self.streams[pes.stream].stream_type == 0x1b {
            let off = pes_payload_offset(&pes.data);
            pes.random_access = pes.random_access || h264_is_keyframe(&pes.data[off..]);
        }
        // Trim to declared PES length when bounded.
        let declared = ((pes.data[4] as usize) << 8) | pes.data[5] as usize;
        if declared != 0 && declared + 6 < pes.data.len() {
            pes.data.truncate(declared + 6);
        }
        self.ready.push_back(pes);
    }

    fn section(payload: &[u8]) -> Option<&[u8]> {
        let ptr = *payload.first()? as usize;
        let s = payload.get(1 + ptr..)?;
        if s.len() < 3 {
            return None;
        }
        let len = (((s[1] as usize) & 0x0F) << 8) | s[2] as usize;
        s.get(..3 + len)
    }

    fn parse_pat(&mut self, payload: &[u8]) {
        let Some(s) = Self::section(payload) else { return };
        let end = s.len().saturating_sub(4);
        let mut i = 8;
        while i + 4 <= end {
            let program = ((s[i] as u16) << 8) | s[i + 1] as u16;
            let pid = ((s[i + 2] as u16 & 0x1F) << 8) | s[i + 3] as u16;
            if program != 0 {
                self.pmt_pid = Some(pid);
                return;
            }
            i += 4;
        }
    }

    fn parse_pmt(&mut self, payload: &[u8]) {
        let Some(s) = Self::section(payload) else { return };
        if s.len() < 12 {
            return;
        }
        let end = s.len() - 4;
        let prog_info_len = (((s[10] as usize) & 0x0F) << 8) | s[11] as usize;
        let mut i = 12 + prog_info_len;
        while i + 5 <= end {
            let stream_type = s[i];
            let pid = ((s[i + 1] as u16 & 0x1F) << 8) | s[i + 2] as u16;
            let es_len = (((s[i + 3] as usize) & 0x0F) << 8) | s[i + 4] as usize;
            let idx = self.streams.len();
            self.streams.push(InputStream { stream_type });
            self.pid_to_stream.insert(pid, idx);
            i += 5 + es_len;
        }
    }
}
