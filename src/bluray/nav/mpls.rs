// SPDX-License-Identifier: GPL-3.0-or-later

//! `BDMV/PLAYLIST/xxxxx.mpls` writer.

use crate::bluray::bytes::BitWriter;
use crate::bluray::{lang3, EsInfo, EsKind};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillMode {
    None,
    /// Still for the given number of seconds.
    Time(u16),
    Infinite,
}

#[derive(Debug, Clone)]
pub struct PlayItem {
    /// 5-digit clip name, e.g. "00001".
    pub clip_id: String,
    /// In/out time in 45 kHz ticks.
    pub in_time: u32,
    pub out_time: u32,
    pub still: StillMode,
    pub streams: Vec<EsInfo>,
}

#[derive(Debug, Clone, Copy)]
pub struct Mark {
    pub play_item: u16,
    /// Absolute clip time in 45 kHz ticks.
    pub time: u32,
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub items: Vec<PlayItem>,
    pub marks: Vec<Mark>,
}

fn write_stn(w: &mut BitWriter, streams: &[EsInfo]) {
    let count = |f: fn(&EsKind) -> bool| streams.iter().filter(|s| f(&s.kind)).count() as u8;
    let n_video = count(|k| matches!(k, EsKind::Video(_)));
    let n_audio = count(|k| matches!(k, EsKind::Audio { .. }));
    let n_ig = count(|k| matches!(k, EsKind::Ig { .. }));
    let n_pg = count(|k| matches!(k, EsKind::Pg { .. }));

    let len = w.begin_len16();
    w.u16(0); // reserved
    w.u8(n_video).u8(n_audio).u8(n_pg).u8(n_ig);
    w.u8(0).u8(0).u8(0); // secondary audio, secondary video, PiP PG
    w.bytes(&[0; 5]);

    // Streams must be listed grouped by type in this order.
    let order: [fn(&EsKind) -> bool; 4] = [
        |k| matches!(k, EsKind::Video(_)),
        |k| matches!(k, EsKind::Audio { .. }),
        |k| matches!(k, EsKind::Pg { .. }),
        |k| matches!(k, EsKind::Ig { .. }),
    ];
    for f in order {
        for s in streams.iter().filter(|s| f(&s.kind)) {
            // stream_entry: type 1 = stream in main path clip
            let l = w.begin_len8();
            w.u8(1).u16(s.pid).bytes(&[0; 6]);
            w.end_len8(l);

            // stream_attributes
            let l = w.begin_len8();
            w.u8(s.coding_type());
            match &s.kind {
                EsKind::Video(v) => {
                    w.bits(4, v.format_code() as u64).bits(4, v.rate_code() as u64);
                    w.bytes(&[0; 3]);
                }
                EsKind::Audio { lang, .. } => {
                    w.u8(s.audio_attributes());
                    w.ascii(&lang3(lang), 3);
                }
                EsKind::Ig { lang } | EsKind::Pg { lang } => {
                    w.ascii(&lang3(lang), 3);
                    w.u8(0);
                }
            }
            w.end_len8(l);
        }
    }
    w.end_len16(len);
}

fn write_play_item(w: &mut BitWriter, pi: &PlayItem) {
    let len = w.begin_len16();
    w.ascii(&pi.clip_id, 5).ascii("M2TS", 4);
    w.zeros(11).flag(false /* multi angle */).bits(4, 1 /* connection condition */);
    w.u8(0); // ref_to_STC_id
    w.u32(pi.in_time).u32(pi.out_time);
    w.bytes(&[0; 8]); // UO mask
    w.flag(false).zeros(7); // PlayItem_random_access_flag
    match pi.still {
        StillMode::None => w.u8(0).u16(0),
        StillMode::Time(t) => w.u8(1).u16(t),
        StillMode::Infinite => w.u8(2).u16(0),
    };
    write_stn(w, &pi.streams);
    w.end_len16(len);
}

impl Playlist {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ascii("MPLS", 4).ascii("0200", 4);
        let list_pos_at = w.len();
        w.u32(0).u32(0).u32(0); // playlist, mark, extension start addresses
        w.pad_to(40);

        // AppInfoPlayList
        let len = w.begin_len32();
        w.u8(0).u8(1 /* sequential */).u16(0);
        w.bytes(&[0; 8]); // UO mask
        w.flag(false /* random access */).flag(true /* audio mix */).flag(false).zeros(13);
        w.end_len32(len);

        let list_pos = w.len();
        w.patch_u32(list_pos_at, list_pos as u32);
        let len = w.begin_len32();
        w.u16(0).u16(self.items.len() as u16).u16(0 /* subpaths */);
        for pi in &self.items {
            write_play_item(&mut w, pi);
        }
        w.end_len32(len);

        let mark_pos = w.len();
        w.patch_u32(list_pos_at + 4, mark_pos as u32);
        let len = w.begin_len32();
        w.u16(self.marks.len() as u16);
        for m in &self.marks {
            w.u8(0).u8(1 /* entry mark */).u16(m.play_item).u32(m.time).u16(0xffff).u32(0);
        }
        w.end_len32(len);
        w.into_bytes()
    }
}
