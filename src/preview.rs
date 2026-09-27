// SPDX-License-Identifier: GPL-3.0-or-later

//! Preview encodes: a short clip of a title encoded exactly like the disc
//! (same video/audio settings, same subtitle conversion and muxer), saved as
//! an MKV with the subtitle track marked default so any player shows it.

use crate::bluray::ts;
use crate::bluray::{EsInfo, EsKind, PID_PG_FIRST, PID_VIDEO};
use crate::build::BuildEvent;
use crate::media::ffmpeg;
use crate::media::transcode::{self, EncodeSettings};
use crate::model::{Id, Project};
use crate::subtitles;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub title: Id,
    /// Start in seconds on the title's video.
    pub start: f64,
    pub duration: f64,
    pub subtitle: Option<Id>,
}

pub fn preview_dir() -> PathBuf {
    let d = crate::media::cache_dir().join("previews");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn file_safe(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Encode the preview; returns the MKV path.
pub fn encode(project: &Project, req: &PreviewRequest, cancel: &AtomicBool, emit: &dyn Fn(BuildEvent)) -> Result<PathBuf> {
    let t = project.title(req.title).context("title not found")?;
    let asset = project.asset(t.asset).context("title has no video")?;
    let info = &asset.info;
    let mut start = req.start.clamp(0.0, (info.duration - 1.0).max(0.0));
    // Kept (not re-encoded) video can only start on a keyframe.
    let keep = if t.keep_video {
        let report = crate::media::compat::analyze(&asset.path)?;
        let Some(format) = report.format.filter(|_| report.compatible()) else {
            bail!("the original video can't be kept ({})", report.summary());
        };
        let key = crate::media::compat::keyframe_before(&asset.path, start + info.start_time)?;
        start = (key - info.start_time).max(0.0);
        Some(format)
    } else {
        None
    };
    let duration = req.duration.min(info.duration - start).max(1.0);
    let settings = EncodeSettings {
        video: project.disc.video,
        video_bitrate: project.disc.video_bitrate,
        audio: project.disc.audio,
        audio_bitrate: project.disc.audio_bitrate,
        encoder: project.disc.encoder,
        quality: project.disc.quality,
    };

    let work = preview_dir().join(format!("work-{}", crate::model::new_id()));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<PathBuf> {
        let progress = |f: f64| emit(BuildEvent::Progress(f.clamp(0.0, 1.0)));

        let tmp = work.join("preview.ts");
        let audio = crate::build::title_audio(project, t, &settings, start)?;
        let inputs: Vec<transcode::AudioInput> = audio.iter().map(|(i, _)| i.clone()).collect();
        let vformat = match keep {
            Some(format) => {
                emit(BuildEvent::Stage(format!("Copying {} seconds of “{}” (original video)", duration.round(), t.name)));
                let args = transcode::passthrough_args(&asset.path, info, &settings, &inputs, Some((start, duration)), &tmp);
                ffmpeg::run(&args, cancel, |secs| progress(secs / duration * 0.8))?;
                format
            }
            None => {
                emit(BuildEvent::Stage(format!("Encoding {} seconds of “{}”", duration.round(), t.name)));
                let args = transcode::title_args(&asset.path, info, &settings, &[], &inputs, Some((start, duration)), &transcode::Pass::Only, &tmp);
                ffmpeg::run(&args, cancel, |secs| progress(secs / duration * 0.8))?;
                settings.video
            }
        };

        let mut streams = vec![EsInfo { pid: PID_VIDEO, kind: EsKind::Video(vformat) }];
        streams.extend(audio.into_iter().map(|(_, es)| es));
        let mut extra = Vec::new();
        let mut sub_lang = None;
        if let Some(track) = req.subtitle.and_then(|id| t.subtitles.iter().find(|s| s.id == id)) {
            emit(BuildEvent::Stage(format!("Converting subtitles “{}”", track.name)));
            let images = subtitles::prepare(
                track,
                &asset.path,
                info,
                vformat,
                &project.disc.subtitle_style,
                &work,
                Some((start, start + duration)),
                cancel,
            )?;
            let first_pts = ts::first_video_pts(&tmp)?;
            // Shift onto the clip's timeline.
            let shifted: Vec<subtitles::SubImage> = images
                .into_iter()
                .filter(|i| i.end > start && i.start < start + duration)
                .map(|mut i| {
                    i.start = (i.start - start).max(0.0);
                    i.end = (i.end - start).min(duration);
                    i
                })
                .collect();
            emit(BuildEvent::Log(format!("{} subtitle images in range", shifted.len())));
            let pes = subtitles::pgs::encode(&shifted, vformat, |secs| {
                (secs >= 0.0).then(|| first_pts + (secs * 90_000.0).round() as u64)
            })?;
            let index = streams.len();
            streams.push(EsInfo { pid: PID_PG_FIRST, kind: EsKind::Pg { lang: track.lang.clone() } });
            extra.extend(pes.into_iter().map(|p| (index, p)));
            sub_lang = Some(track.lang.clone());
        }
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }

        emit(BuildEvent::Stage("Multiplexing".into()));
        let m2ts = work.join("preview.m2ts");
        ts::remux(&tmp, &m2ts, &streams, extra, |_| !cancel.load(Ordering::Relaxed))?;
        progress(0.9);

        let out = preview_dir().join(format!("{} - {}.mkv", file_safe(&t.name), crate::ui::rows::format_time(start).replace(':', ".")));
        let mut args: Vec<String> = ["-i", &m2ts.to_string_lossy(), "-map", "0", "-c", "copy", "-disposition:v:0", "default"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let audio_langs: Vec<String> = streams
            .iter()
            .filter_map(|s| match &s.kind {
                EsKind::Audio { lang, .. } => Some(lang.clone()),
                _ => None,
            })
            .collect();
        for (n, lang) in audio_langs.iter().enumerate() {
            args.extend([format!("-disposition:a:{n}"), (if n == 0 { "default" } else { "0" }).to_string()]);
            args.extend([format!("-metadata:s:a:{n}"), format!("language={lang}")]);
        }
        if let Some(lang) = sub_lang {
            args.extend(["-disposition:s:0", "default", "-metadata:s:s:0", &format!("language={lang}")].iter().map(|s| s.to_string()));
        }
        args.push(out.to_string_lossy().into_owned());
        ffmpeg::run(&args, cancel, |_| {}).context("could not package the preview")?;
        progress(1.0);
        Ok(out)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}
