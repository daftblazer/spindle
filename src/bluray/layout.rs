// SPDX-License-Identifier: GPL-3.0-or-later

//! BDMV directory tree layout.

use super::nav::clpi::ClipInfo;
use super::nav::index::Index;
use super::nav::mobj::{movie_objects_to_bytes, MovieObject};
use super::nav::mpls::Playlist;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn clip_name(n: u32) -> String {
    format!("{n:05}")
}

pub fn bdmv_dir(root: &Path) -> PathBuf {
    root.join("BDMV")
}

pub fn stream_path(root: &Path, clip: u32) -> PathBuf {
    bdmv_dir(root).join("STREAM").join(format!("{}.m2ts", clip_name(clip)))
}

/// Create the empty directory skeleton.
pub fn create_dirs(root: &Path) -> Result<()> {
    let bdmv = bdmv_dir(root);
    for d in [
        "PLAYLIST",
        "CLIPINF",
        "STREAM",
        "AUXDATA",
        "META",
        "JAR",
        "BDJO",
        "BACKUP/PLAYLIST",
        "BACKUP/CLIPINF",
        "BACKUP/BDJO",
    ] {
        fs::create_dir_all(bdmv.join(d)).with_context(|| format!("creating {}", bdmv.join(d).display()))?;
    }
    fs::create_dir_all(root.join("CERTIFICATE/BACKUP"))?;
    Ok(())
}

fn write_with_backup(root: &Path, rel: &str, data: &[u8]) -> Result<()> {
    let bdmv = bdmv_dir(root);
    for p in [bdmv.join(rel), bdmv.join("BACKUP").join(rel)] {
        fs::write(&p, data).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(())
}

/// Write the database files. Stream files must already be in `BDMV/STREAM`.
pub fn write_database(
    root: &Path,
    index: &Index,
    objects: &[MovieObject],
    playlists: &[(u32, Playlist)],
    clips: &[(u32, ClipInfo)],
) -> Result<()> {
    create_dirs(root)?;
    write_with_backup(root, "index.bdmv", &index.to_bytes())?;
    write_with_backup(root, "MovieObject.bdmv", &movie_objects_to_bytes(objects))?;
    for (n, pl) in playlists {
        write_with_backup(root, &format!("PLAYLIST/{}.mpls", clip_name(*n)), &pl.to_bytes())?;
    }
    for (n, ci) in clips {
        write_with_backup(root, &format!("CLIPINF/{}.clpi", clip_name(*n)), &ci.to_bytes())?;
    }
    Ok(())
}
