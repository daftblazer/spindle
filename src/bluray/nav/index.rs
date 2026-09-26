// SPDX-License-Identifier: GPL-3.0-or-later

//! `BDMV/index.bdmv` writer.

use crate::bluray::bytes::BitWriter;
use crate::bluray::VideoFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackType {
    Movie = 0,
    Interactive = 1,
}

/// Reference from index.bdmv to an HDMV movie object.
#[derive(Debug, Clone, Copy)]
pub struct ObjectRef {
    pub object_id: u16,
    pub playback_type: PlaybackType,
}

#[derive(Debug, Clone)]
pub struct Index {
    pub first_play: ObjectRef,
    pub top_menu: ObjectRef,
    pub titles: Vec<ObjectRef>,
    pub video: VideoFormat,
}

fn write_hdmv_obj(w: &mut BitWriter, r: ObjectRef) {
    w.bits(2, r.playback_type as u64).zeros(14).u16(r.object_id).zeros(32);
}

impl Index {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ascii("INDX", 4).ascii("0200", 4);
        let indexes_start_pos = w.len();
        w.u32(0); // indexes_start (patched)
        w.u32(0); // extension_data_start
        w.pad_to(40);

        // AppInfoBDMV
        let len = w.begin_len32();
        w.zeros(1)
            .flag(false) // initial_output_mode_preference (2D)
            .flag(false) // SS_content_exist_flag
            .zeros(1)
            .bits(4, 0) // initial_dynamic_range_type
            .bits(4, self.video.format_code() as u64)
            .bits(4, self.video.rate_code() as u64)
            .bytes(&[0u8; 32]); // user data
        w.end_len32(len);

        let start = w.len();
        w.patch_u32(indexes_start_pos, start as u32);

        let len = w.begin_len32();
        // First playback
        w.bits(2, 1).zeros(30);
        write_hdmv_obj(&mut w, self.first_play);
        // Top menu
        w.bits(2, 1).zeros(30);
        write_hdmv_obj(&mut w, self.top_menu);

        w.u16(self.titles.len() as u16);
        for t in &self.titles {
            w.bits(2, 1) // object type HDMV
                .bits(2, 0) // access type: permitted
                .zeros(28);
            write_hdmv_obj(&mut w, *t);
        }
        w.end_len32(len);
        w.into_bytes()
    }
}
