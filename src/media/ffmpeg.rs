// SPDX-License-Identifier: GPL-3.0-or-later

//! Running ffmpeg with progress reporting and cancellation.

use anyhow::{bail, Context, Result};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Run ffmpeg with `args`. `progress` receives the output position in
/// seconds. Returns an error with the tail of ffmpeg's log on failure.
pub fn run(args: &[String], cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<()> {
    log::debug!("ffmpeg {}", args.join(" "));
    let mut child = Command::new(super::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-y", "-nostats", "-progress", "pipe:1"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run ffmpeg (is FFmpeg installed?)")?;

    // Keep the last lines of stderr for error reporting.
    let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let stderr = child.stderr.take().unwrap();
    let tail2 = tail.clone();
    let err_thread = std::thread::spawn(move || {
        let mut r = BufReader::new(stderr);
        let mut buf = Vec::new();
        while r.read_until(b'\n', &mut buf).unwrap_or(0) > 0 {
            let line = String::from_utf8_lossy(&buf).trim_end().to_string();
            buf.clear();
            let mut t = tail2.lock().unwrap();
            t.push_back(line);
            if t.len() > 20 {
                t.pop_front();
            }
        }
    });

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled");
        }
        line.clear();
        if stdout.read_line(&mut line)? == 0 {
            break;
        }
        if let Some(v) = line.trim().strip_prefix("out_time_us=") {
            if let Ok(us) = v.parse::<i64>() {
                progress(us.max(0) as f64 / 1e6);
            }
        }
    }
    // Drain anything left so the child can exit.
    let mut rest = Vec::new();
    let _ = stdout.read_to_end(&mut rest);
    let status = child.wait()?;
    let _ = err_thread.join();
    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    if !status.success() {
        let t = tail.lock().unwrap();
        bail!("ffmpeg failed:\n{}", t.iter().cloned().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}
