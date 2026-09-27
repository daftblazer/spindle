// SPDX-License-Identifier: GPL-3.0-or-later

//! Running ffmpeg with progress reporting and cancellation.

use anyhow::{bail, Context, Result};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Progress of a running ffmpeg.
#[derive(Debug, Clone, Copy, Default)]
pub struct Status {
    /// Output position in seconds.
    pub time: f64,
    /// Frames encoded per second, once known.
    pub fps: Option<f64>,
}

/// Run ffmpeg with `args`. `progress` receives the output position in
/// seconds. Returns an error with the tail of ffmpeg's log on failure.
pub fn run(args: &[String], cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<()> {
    run_status(args, cancel, |s| progress(s.time))
}

/// Run ffmpeg and return the end of its log (for filters that report
/// there, like loudness measurements).
pub fn run_log(args: &[String], cancel: &AtomicBool) -> Result<String> {
    let mut all = vec!["-v".to_string(), "info".to_string()];
    all.extend(args.iter().cloned());
    let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
    run_inner(&all, cancel, &mut |_| {}, &tail, 60)?;
    let t = tail.lock().unwrap();
    Ok(t.iter().cloned().collect::<Vec<_>>().join("\n"))
}

/// [`run`], with the encoding speed as well.
pub fn run_status(args: &[String], cancel: &AtomicBool, mut progress: impl FnMut(Status)) -> Result<()> {
    let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
    run_inner(args, cancel, &mut progress, &tail, 20)
}

/// Run ffmpeg keeping the last `keep` lines of its log in `tail`.
fn run_inner(args: &[String], cancel: &AtomicBool, progress: &mut dyn FnMut(Status), tail: &Arc<Mutex<VecDeque<String>>>, keep: usize) -> Result<()> {
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
            if t.len() > keep {
                t.pop_front();
            }
        }
    });

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let mut status = Status::default();
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
        // Each report ends with "progress=…".
        let l = line.trim();
        if let Some(v) = l.strip_prefix("out_time_us=") {
            if let Ok(us) = v.parse::<i64>() {
                status.time = us.max(0) as f64 / 1e6;
            }
        } else if let Some(v) = l.strip_prefix("fps=") {
            status.fps = v.parse::<f64>().ok().filter(|f| *f > 0.0);
        } else if l.starts_with("progress=") {
            progress(status);
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
