// SPDX-License-Identifier: GPL-3.0-or-later

//! Loudness normalisation (EBU R128): each track is measured once, then
//! adjusted to the target with ffmpeg's `loudnorm` in linear mode, which
//! changes the level without squashing the dynamics.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Integrated loudness to aim for (LUFS).
pub const TARGET: f64 = -23.0;
/// Highest true peak allowed (dBTP).
pub const TRUE_PEAK: f64 = -1.0;

/// What `loudnorm` measured on a track.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Measured {
    pub integrated: f64,
    pub true_peak: f64,
    pub range: f64,
    pub threshold: f64,
    pub offset: f64,
}

fn cache_file() -> PathBuf {
    super::cache_dir().join("loudness.json")
}

fn key(file: &Path, index: usize) -> String {
    format!("{}#{index}", crate::encode_cache::file_id(file))
}

fn load() -> HashMap<String, Measured> {
    std::fs::read(cache_file()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

/// The measurement of audio stream `index` of `file`, if it has been made.
pub fn cached(file: &Path, index: usize) -> Option<Measured> {
    load().get(&key(file, index)).copied()
}

/// Measure audio stream `index` of `file` (decoding all of it), or return
/// the earlier measurement.
pub fn measure(file: &Path, index: usize, cancel: &AtomicBool) -> Result<Measured> {
    if let Some(m) = cached(file, index) {
        return Ok(m);
    }
    let args: Vec<String> = [
        "-i".to_string(),
        file.to_string_lossy().into_owned(),
        "-map".into(),
        format!("0:a:{index}"),
        "-af".into(),
        format!("loudnorm=I={TARGET}:TP={TRUE_PEAK}:LRA=20:print_format=json"),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]
    .into();
    let log = super::ffmpeg::run_log(&args, cancel).context("could not measure the loudness")?;
    let m = parse(&log).context("ffmpeg didn't report the loudness")?;
    let mut all = load();
    all.insert(key(file, index), m);
    if let Ok(data) = serde_json::to_vec(&all) {
        let _ = std::fs::write(cache_file(), data);
    }
    Ok(m)
}

/// The JSON block `loudnorm` prints at the end.
fn parse(log: &str) -> Option<Measured> {
    let start = log.rfind('{')?;
    let end = start + log[start..].find('}')?;
    let v: HashMap<String, String> = serde_json::from_str(&log[start..=end]).ok()?;
    let num = |k: &str| v.get(k)?.trim().parse::<f64>().ok().filter(|x| x.is_finite());
    Some(Measured {
        integrated: num("input_i")?,
        true_peak: num("input_tp")?,
        range: num("input_lra")?,
        threshold: num("input_thresh")?,
        offset: num("target_offset").unwrap_or(0.0),
    })
}

/// Filter that brings a track with measurement `m` to the target.
pub fn filter(m: &Measured) -> String {
    format!(
        "loudnorm=I={TARGET}:TP={TRUE_PEAK}:LRA=20:measured_I={:.2}:measured_TP={:.2}:measured_LRA={:.2}:measured_thresh={:.2}:offset={:.2}:linear=true:print_format=none",
        m.integrated, m.true_peak, m.range, m.threshold, m.offset
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loudnorm_report() {
        let log = r#"[Parsed_loudnorm_0 @ 0x55] 
{
	"input_i" : "-18.42",
	"input_tp" : "-0.51",
	"input_lra" : "7.90",
	"input_thresh" : "-28.61",
	"output_i" : "-23.03",
	"output_tp" : "-5.02",
	"output_lra" : "7.50",
	"output_thresh" : "-33.19",
	"normalization_type" : "dynamic",
	"target_offset" : "0.03"
}"#;
        let m = parse(log).unwrap();
        assert_eq!(m.integrated, -18.42);
        assert_eq!(m.offset, 0.03);
        assert!(filter(&m).contains("measured_I=-18.42"));
        // Silence reports -inf, which can't be adjusted.
        assert!(parse(r#"{"input_i" : "-inf", "input_tp" : "-inf", "input_lra" : "0.00", "input_thresh" : "-inf"}"#).is_none());
    }
}
