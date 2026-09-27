// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware H.264 encoders (VA-API, NVENC). Faster than x264 but lower
//! quality at Blu-ray bitrates, so meant for test discs.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VideoEncoder {
    /// x264: the best quality and full Blu-ray compliance.
    #[default]
    Software,
    /// AMD and Intel GPUs.
    Vaapi,
    /// NVIDIA GPUs.
    Nvenc,
}

impl VideoEncoder {
    pub const ALL: [VideoEncoder; 3] = [VideoEncoder::Software, VideoEncoder::Vaapi, VideoEncoder::Nvenc];

    pub fn is_hardware(self) -> bool {
        self != VideoEncoder::Software
    }

    pub fn label(self) -> String {
        use gettextrs::gettext;
        match self {
            VideoEncoder::Software => gettext("Software (x264)"),
            VideoEncoder::Vaapi => gettext("Hardware · AMD/Intel (VA-API)"),
            VideoEncoder::Nvenc => gettext("Hardware · NVIDIA (NVENC)"),
        }
    }
}

/// The first GPU render node, for VA-API.
pub fn render_node() -> Option<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("renderD")))
        .collect();
    nodes.sort();
    nodes.into_iter().next()
}

/// Try a tiny encode to see whether `enc` works on this computer.
fn works(enc: VideoEncoder) -> bool {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-v".into(), "error".into()];
    match enc {
        VideoEncoder::Software => return true,
        VideoEncoder::Vaapi => {
            let Some(node) = render_node() else { return false };
            args.extend(["-vaapi_device".into(), node.to_string_lossy().into_owned()]);
        }
        VideoEncoder::Nvenc => {}
    }
    args.extend(["-f", "lavfi", "-i", "color=black:s=640x360:r=24", "-frames:v", "5"].map(String::from));
    match enc {
        VideoEncoder::Vaapi => args.extend(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi"].map(String::from)),
        _ => args.extend(["-pix_fmt", "yuv420p", "-c:v", "h264_nvenc"].map(String::from)),
    }
    args.extend(["-f", "null", "-"].map(String::from));
    Command::new(super::ffmpeg_bin())
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

static AVAILABLE: OnceLock<Vec<VideoEncoder>> = OnceLock::new();

/// Encoders that work here (tested once, then remembered). Blocks for a
/// moment the first time.
pub fn available() -> &'static [VideoEncoder] {
    AVAILABLE.get_or_init(|| VideoEncoder::ALL.into_iter().filter(|e| works(*e)).collect())
}

/// The result of [`available`] if it has been worked out already.
pub fn known_available() -> Option<&'static [VideoEncoder]> {
    AVAILABLE.get().map(|v| v.as_slice())
}
