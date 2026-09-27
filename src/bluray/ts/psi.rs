// SPDX-License-Identifier: GPL-3.0-or-later

//! PSI tables (PAT, PMT, SIT) for BDAV transport streams.

use crate::bluray::bytes::BitWriter;
use crate::bluray::{EsInfo, EsKind, PID_PCR};

pub const PID_PAT: u16 = 0x0000;
pub const PID_SIT: u16 = 0x001F;
pub const PID_NULL: u16 = 0x1FFF;

/// MPEG-2 CRC32 (polynomial 0x04C11DB7, no reflection).
pub fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

fn finish_section(mut w: BitWriter) -> Vec<u8> {
    let mut s = w.clone().into_bytes();
    let crc = crc32_mpeg2(&s);
    w.u32(crc);
    s = w.into_bytes();
    s
}

/// Build a long-form section: `table_id`, `id_ext`, followed by `body` and CRC.
fn long_section(table_id: u8, id_ext: u16, body: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    let section_length = 5 + body.len() + 4;
    w.u8(table_id)
        .flag(true) // section_syntax_indicator
        .flag(false)
        .bits(2, 3)
        .bits(12, section_length as u64)
        .u16(id_ext)
        .bits(2, 3)
        .bits(5, 0) // version
        .flag(true) // current_next
        .u8(0)
        .u8(0)
        .bytes(body);
    finish_section(w)
}

pub fn pat(pmt_pid: u16) -> Vec<u8> {
    let mut b = BitWriter::new();
    b.u16(1).bits(3, 7).bits(13, pmt_pid as u64);
    long_section(0x00, 0x0000 /* transport_stream_id */, &b.into_bytes())
}

fn registration(w: &mut BitWriter, extra: &[u8]) {
    w.u8(0x05).u8(4 + extra.len() as u8).ascii("HDMV", 4).bytes(extra);
}

/// MPEG-2 stream_type for the PMT.
fn stream_type(es: &EsInfo) -> u8 {
    es.coding_type()
}

pub fn pmt(streams: &[EsInfo]) -> Vec<u8> {
    let mut b = BitWriter::new();
    b.bits(3, 7).bits(13, PID_PCR as u64);

    let mut prog = BitWriter::new();
    registration(&mut prog, &[]);
    // Copy control descriptor: no restrictions.
    prog.bytes(&[0x88, 0x04, 0x0F, 0xFF, 0xFC, 0xFC]);
    let prog = prog.into_bytes();
    b.bits(4, 0xF).bits(12, prog.len() as u64).bytes(&prog);

    for es in streams {
        let mut d = BitWriter::new();
        let st = stream_type(es);
        match &es.kind {
            EsKind::Video(v) => registration(&mut d, &[0xFF, st, (v.format_code() << 4) | v.rate_code(), 0x3F]),
            EsKind::Audio { .. } => registration(&mut d, &[0xFF, st, es.audio_attributes(), 0x3F]),
            EsKind::Ig { .. } | EsKind::Pg { .. } => {}
        };
        let d = d.into_bytes();
        b.u8(st).bits(3, 7).bits(13, es.pid as u64).bits(4, 0xF).bits(12, d.len() as u64).bytes(&d);
    }
    long_section(0x02, 1 /* program_number */, &b.into_bytes())
}

/// Selection Information Table carrying the partial TS descriptor.
pub fn sit(peak_rate_bps: u64) -> Vec<u8> {
    let mut desc = BitWriter::new();
    let peak = (peak_rate_bps / 400).min((1 << 22) - 1);
    desc.u8(0x63).u8(8);
    desc.bits(2, 3).bits(22, peak).bits(2, 3).bits(22, 0x3F_FFFF).bits(2, 3).bits(14, 0x3FFF);
    let desc = desc.into_bytes();

    let mut b = BitWriter::new();
    b.bits(4, 0xF).bits(12, desc.len() as u64).bytes(&desc);
    // one service entry: service_id 1, running_status 4 (running), no descriptors
    b.u16(1).flag(true).bits(3, 4).bits(12, 0);
    let body = b.into_bytes();

    let mut w = BitWriter::new();
    let section_length = 5 + body.len() + 4;
    w.u8(0x7F)
        .flag(true)
        .flag(true)
        .bits(2, 3)
        .bits(12, section_length as u64)
        .u16(0xFFFF)
        .bits(2, 3)
        .bits(5, 0)
        .flag(true)
        .u8(0)
        .u8(0)
        .bytes(&body);
    finish_section(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_known_pat() {
        // PAT with program 1 -> PID 0x100, as emitted by many muxers.
        let s = pat(0x100);
        assert_eq!(&s[..s.len() - 4], &[0x00, 0xB0, 0x0D, 0x00, 0x00, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE1, 0x00]);
        assert_eq!(crc32_mpeg2(&s), 0, "CRC over section including CRC must be 0");
    }
}
