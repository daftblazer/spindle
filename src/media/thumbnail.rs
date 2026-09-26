// SPDX-License-Identifier: GPL-3.0-or-later

//! Frame extraction for asset thumbnails and button images.

use anyhow::{bail, Context, Result};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Extract a frame at `time` seconds scaled to `width` pixels wide, cached
/// on disk. Returns the PNG path.
pub fn frame(path: &Path, time: f64, width: u32) -> Result<PathBuf> {
    let mut h = DefaultHasher::new();
    "rgb24".hash(&mut h);
    path.hash(&mut h);
    ((time * 1000.0) as i64).hash(&mut h);
    width.hash(&mut h);
    if let Ok(m) = std::fs::metadata(path) {
        m.len().hash(&mut h);
        m.modified().ok().hash(&mut h);
    }
    let out = super::cache_dir().join(format!("thumb-{:016x}.png", h.finish()));
    if out.exists() {
        return Ok(out);
    }

    let tmp = out.with_extension("tmp.png");
    let status = Command::new(super::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{time:.3}"))
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-vf"])
        // 8-bit RGB even for 10/12-bit sources (smaller, loads everywhere).
        .arg(format!("scale={width}:-2,format=rgb24"))
        .arg(&tmp)
        .status()
        .context("failed to run ffmpeg")?;
    if !status.success() || !tmp.exists() {
        // The seek may have gone past the end; retry at the start.
        if time > 0.0 {
            return frame(path, 0.0, width);
        }
        bail!("could not extract a frame from {}", path.display());
    }
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}
