// SPDX-License-Identifier: GPL-3.0-or-later

//! `BDMV/MovieObject.bdmv` writer.

use super::hdmv::Command;
use crate::bluray::bytes::BitWriter;

#[derive(Debug, Clone, Default)]
pub struct MovieObject {
    pub resume_intention: bool,
    pub menu_call_mask: bool,
    pub title_search_mask: bool,
    pub commands: Vec<Command>,
}

pub fn movie_objects_to_bytes(objects: &[MovieObject]) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ascii("MOBJ", 4).ascii("0200", 4);
    w.u32(0); // extension_data_start
    w.pad_to(40);

    let len = w.begin_len32();
    w.u32(0); // reserved
    w.u16(objects.len() as u16);
    for o in objects {
        w.flag(o.resume_intention)
            .flag(o.menu_call_mask)
            .flag(o.title_search_mask)
            .zeros(13)
            .u16(o.commands.len() as u16);
        for c in &o.commands {
            c.write(&mut w);
        }
    }
    w.end_len32(len);
    w.into_bytes()
}
