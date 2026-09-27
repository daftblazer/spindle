// SPDX-License-Identifier: GPL-3.0-or-later

//! Finished title encodes, kept so a rebuild (e.g. after a menu change)
//! doesn't encode the videos again. Entries are keyed by everything that
//! affects the encode, and the oldest are removed past a size limit.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bump when the encode arguments change in a way the key doesn't cover.
const VERSION: u32 = 1;
/// Largest total size before old entries are removed.
const LIMIT: u64 = 64 * 1024 * 1024 * 1024;

pub fn dir() -> PathBuf {
    crate::media::cache_dir().join("encodes")
}

/// 64-bit FNV-1a, stable across runs and builds.
pub fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The ffmpeg in use (its first version line), so an upgrade re-encodes.
fn ffmpeg_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        std::process::Command::new(crate::media::ffmpeg_bin())
            .arg("-version")
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string))
            .unwrap_or_default()
    })
}

/// Identity of a file: path, size and modification time.
pub fn file_id(path: &Path) -> String {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let mtime = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    format!("{}|{size}|{mtime}", path.display())
}

/// Cache file for an encode described by `parts` (the ffmpeg arguments
/// minus paths, the files involved, …).
pub fn entry(parts: &[String]) -> PathBuf {
    let mut key = format!("v{VERSION}|{}", ffmpeg_version());
    for p in parts {
        key.push('|');
        key.push_str(p);
    }
    dir().join(format!("{:016x}.ts", fnv(key.as_bytes())))
}

/// Mark an entry as just used (for the eviction order).
pub fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Total size of the cache in bytes.
pub fn size() -> u64 {
    entries().iter().map(|(_, s, _)| s).sum()
}

fn entries() -> Vec<(PathBuf, u64, SystemTime)> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return vec![] };
    rd.filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(UNIX_EPOCH)))
        })
        .collect()
}

/// Remove the least recently used entries until the cache fits the limit.
pub fn trim() {
    let mut all = entries();
    let mut total: u64 = all.iter().map(|(_, s, _)| s).sum();
    all.sort_by_key(|(_, _, t)| *t);
    for (path, s, _) in all {
        if total <= LIMIT {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= s;
        }
    }
}

pub fn clear() {
    let _ = std::fs::remove_dir_all(dir());
}

/// Measured encoding speeds, for time estimates: seconds of video encoded
/// per second, by encoder, quality and picture height.
fn speeds_file() -> PathBuf {
    crate::media::cache_dir().join("speeds.json")
}

fn speeds() -> std::collections::HashMap<String, f64> {
    std::fs::read(speeds_file()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

/// Remember that `secs` of video took `wall` seconds to encode with the
/// settings summarised by `key`.
pub fn record_speed(key: &str, secs: f64, wall: f64) {
    if secs < 20.0 || wall < 1.0 {
        return;
    }
    let mut all = speeds();
    let speed = secs / wall;
    // Lean towards the latest build, which reflects the computer as it is.
    let v = all.get(key).map_or(speed, |old| old * 0.3 + speed * 0.7);
    all.insert(key.to_string(), v);
    if let Ok(data) = serde_json::to_vec(&all) {
        let _ = std::fs::write(speeds_file(), data);
    }
}

pub fn speed(key: &str) -> Option<f64> {
    speeds().get(key).copied().filter(|v| *v > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_depend_on_every_part() {
        let a = entry(&["x".into(), "18000".into()]);
        assert_eq!(a, entry(&["x".into(), "18000".into()]));
        assert_ne!(a, entry(&["x".into(), "20000".into()]));
        assert_ne!(entry(&["ab".into(), "c".into()]), entry(&["a".into(), "bc".into()]));
    }
}
