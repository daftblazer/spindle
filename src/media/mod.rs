// SPDX-License-Identifier: GPL-3.0-or-later

//! ffmpeg/ffprobe integration.

pub mod ffmpeg;
pub mod portal;
pub mod probe;
pub mod thumbnail;
pub mod transcode;

use std::path::PathBuf;

pub fn ffmpeg_bin() -> String {
    std::env::var("SPINDLE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

pub fn ffprobe_bin() -> String {
    std::env::var("SPINDLE_FFPROBE").unwrap_or_else(|_| "ffprobe".into())
}

pub fn cache_dir() -> PathBuf {
    let dir = gtk::glib::user_cache_dir().join("spindle");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
