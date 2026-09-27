// SPDX-License-Identifier: GPL-3.0-or-later

//! Disc build pipeline: project → BDMV folder.
//!
//! Disc structure produced:
//! * Movie object 0: first play – jumps to the first menu or title.
//! * Movie objects 1..=M: one per menu, playing the menu playlist (still
//!   menus hold the last frame forever; motion menus loop).
//! * Movie objects M+1..=M+T: one per title. A chapter to start from is
//!   passed in GPR0 by the menu button that jumps to the title.
//! * Playlists/clips are numbered 1..=M for menus and M+1..=M+T for titles.

use crate::bluray::ig;
use crate::bluray::layout::{self, clip_name};
use crate::bluray::nav::clpi::ClipInfo;
use crate::bluray::nav::hdmv::{Cmp, Command, Operand};
use crate::bluray::nav::index::{Index, ObjectRef, PlaybackType};
use crate::bluray::nav::mobj::MovieObject;
use crate::bluray::nav::mpls::{Mark, PlayItem, Playlist, StillMode};
use crate::bluray::ts::{self, demux::Pes};
use crate::bluray::{EsInfo, EsKind, PID_AUDIO_FIRST, PID_IG_FIRST, PID_PG_FIRST, PID_VIDEO};
use crate::bluray::AudioCodec;
use crate::media::compat;
use crate::subtitles;
use crate::media::transcode::{self, EncodeSettings};
use crate::media::ffmpeg;
use crate::model::*;
use crate::render::{self, ButtonState, ImageCache};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Seconds a still menu clip lasts before the player holds its last frame.
const STILL_MENU_SECONDS: f64 = 1.0;
/// GPR carrying the chapter to start a title at.
const GPR_CHAPTER: u16 = 0;
const GPR_TMP: u16 = 1;
/// 1 while "Play All" is running.
const GPR_PLAY_ALL: u16 = 2;
/// Index of the menu "Play All" was started from.
const GPR_PLAY_ALL_MENU: u16 = 3;
/// Chosen language preset (1-based; 0 when the disc has none).
const GPR_LANGUAGE: u16 = 4;

#[derive(Debug, Clone)]
pub enum BuildEvent {
    Stage(String),
    Progress(f64),
    Log(String),
    Finished(Result<PathBuf, String>),
}

pub struct Builder<'a> {
    project: &'a Project,
    out: PathBuf,
    work: PathBuf,
    cancel: &'a AtomicBool,
    emit: &'a dyn Fn(BuildEvent),
    settings: EncodeSettings,
    total_work: f64,
    done_work: f64,
}

/// Audio of a title: the ffmpeg inputs and the disc stream of each.
/// AC-3 that is already Blu-ray compatible is copied rather than
/// re-encoded (for kept video always, else when the disc uses AC-3), except
/// in clips starting at `clip_start` > 0: a copied stream cut there keeps
/// audio from before it.
pub fn title_audio(p: &Project, t: &Title, set: &EncodeSettings, clip_start: f64) -> Result<Vec<(transcode::AudioInput, EsInfo)>> {
    let asset = p.asset(t.asset).context("title without video")?;
    let own = asset.info.audio();
    let mut out = Vec::new();
    for track in t.disc_audio() {
        let input = match track.source {
            AudioSource::Embedded { index } => {
                let Some(stream) = own.iter().find(|s| s.index == index) else { continue };
                let copy = stream.is_bluray_ac3() && (t.keep_video || set.audio == AudioCodec::Ac3) && clip_start <= 0.0;
                transcode::AudioInput { file: None, index, offset: 0.0, channels: stream.channels, copy }
            }
            AudioSource::External { asset, offset } => {
                let a = p.asset(asset).with_context(|| format!("audio track “{}” of “{}” has no file", track.name, t.name))?;
                let stream = a.info.audio().into_iter().next().with_context(|| format!("{} has no audio", a.path.display()))?;
                transcode::AudioInput { file: Some(a.path.clone()), index: stream.index, offset, channels: stream.channels, copy: false }
            }
        };
        let es = EsInfo {
            pid: PID_AUDIO_FIRST + out.len() as u16,
            kind: EsKind::Audio {
                codec: if input.copy { AudioCodec::Ac3 } else { set.audio },
                channels: input.output_channels(),
                lang: track.lang.clone(),
            },
        };
        out.push((input, es));
    }
    Ok(out)
}

/// Validate a project before building. Returns human-readable problems.
/// Problems that stop the build.
pub fn check(project: &Project) -> Vec<String> {
    crate::validate::check(project)
        .into_iter()
        .filter(|i| i.severity == crate::validate::Severity::Error)
        .map(|i| i.message)
        .collect()
}

fn object_for_menu(i: usize) -> u32 {
    1 + i as u32
}

impl<'a> Builder<'a> {
    pub fn new(project: &'a Project, out: &Path, cancel: &'a AtomicBool, emit: &'a dyn Fn(BuildEvent)) -> Self {
        let settings = EncodeSettings {
            video: project.disc.video,
            video_bitrate: project.disc.video_bitrate,
            audio: project.disc.audio,
            audio_bitrate: project.disc.audio_bitrate,
        };
        Builder {
            project,
            out: out.to_path_buf(),
            work: out.join(".spindle-work"),
            cancel,
            emit,
            settings,
            total_work: 1.0,
            done_work: 0.0,
        }
    }

    fn object_for_title(&self, i: usize) -> u32 {
        1 + self.project.menus.len() as u32 + i as u32
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        Ok(())
    }

    fn stage(&self, s: String) {
        (self.emit)(BuildEvent::Log(s.clone()));
        (self.emit)(BuildEvent::Stage(s));
    }

    fn progress(&self, extra: f64) {
        (self.emit)(BuildEvent::Progress(((self.done_work + extra) / self.total_work).clamp(0.0, 1.0)));
    }

    fn menu_duration(&self, m: &Menu) -> f64 {
        if m.is_motion() {
            m.duration.max(1.0)
        } else {
            STILL_MENU_SECONDS
        }
    }

    pub fn run(mut self) -> Result<PathBuf> {
        let problems = check(self.project);
        if !problems.is_empty() {
            bail!("{}", problems.join("\n"));
        }
        let p = self.project;
        // Work units: seconds of media to encode, plus 10% for remuxing.
        self.total_work = p
            .titles
            .iter()
            .filter_map(|t| p.asset(t.asset))
            .map(|a| a.info.duration.max(1.0))
            .chain(p.menus.iter().map(|m| self.menu_duration(m)))
            .sum::<f64>()
            * 1.1;

        std::fs::create_dir_all(&self.work)
            .with_context(|| format!("creating {}", self.work.display()))?;
        if layout::bdmv_dir(&self.out).exists() {
            std::fs::remove_dir_all(layout::bdmv_dir(&self.out))?;
        }
        layout::create_dirs(&self.out)?;

        let mut playlists = Vec::new();
        let mut clips = Vec::new();

        for (i, m) in p.menus.iter().enumerate() {
            self.check_cancel()?;
            let n = 1 + i as u32;
            let (pl, ci) = self.build_menu(m, n)?;
            playlists.push((n, pl));
            clips.push((n, ci));
        }
        for (i, t) in p.titles.iter().enumerate() {
            self.check_cancel()?;
            let n = 1 + (p.menus.len() + i) as u32;
            let (pl, ci) = self.build_title(t, n)?;
            playlists.push((n, pl));
            clips.push((n, ci));
        }

        self.stage("Writing disc navigation".into());
        let (index, objects) = self.navigation();
        layout::write_database(&self.out, &index, &objects, &playlists, &clips)?;
        let _ = std::fs::remove_dir_all(&self.work);
        (self.emit)(BuildEvent::Progress(1.0));
        Ok(self.out.clone())
    }

    fn navigation(&self) -> (Index, Vec<MovieObject>) {
        let p = self.project;
        let m_count = p.menus.len();
        let first_menu = (m_count > 0).then(|| object_for_menu(0));

        let mut objects = Vec::new();
        // 0: first play
        let first = match (p.first_play, first_menu) {
            (FirstPlay::FirstMenu, Some(o)) => vec![Command::JumpObject(o)],
            _ => vec![Command::JumpTitle(1)],
        };
        let default_language = p.default_language().and_then(|d| p.disc.languages.iter().position(|l| l.id == d.id));
        let first = [
            Command::Move(GPR_CHAPTER, Operand::Imm(0)),
            Command::Move(GPR_PLAY_ALL, Operand::Imm(0)),
            Command::Move(GPR_LANGUAGE, Operand::Imm(default_language.map_or(0, |i| i as u32 + 1))),
        ]
        .into_iter()
        .chain(first)
        .collect();
        objects.push(MovieObject { commands: first, ..Default::default() });

        for i in 0..m_count {
            let pl = 1 + i as u32;
            objects.push(MovieObject {
                menu_call_mask: true,
                commands: vec![Command::PlayPl(Operand::Imm(pl)), Command::Goto(0)],
                ..Default::default()
            });
        }

        for (i, t) in p.titles.iter().enumerate() {
            let pl = Operand::Imm(1 + (m_count + i) as u32);
            let menu_obj = t
                .return_menu
                .and_then(|id| p.menus.iter().position(|m| m.id == id))
                .map(object_for_menu)
                .or(first_menu);
            // Stream selection: the title's default subtitles, then the
            // viewer's language preset.
            let mut commands = Vec::new();
            let subs: Vec<&SubtitleTrack> = t.disc_subtitles().collect();
            if !subs.is_empty() {
                let default = t.default_subtitle.and_then(|d| subs.iter().position(|s| s.id == d));
                commands.push(match default {
                    Some(i) => Command::SetPgStream { number: i as u16 + 1, display: true },
                    None => Command::SetPgStream { number: 1, display: false },
                });
            }
            let audio: Vec<Id> = t.disc_audio().map(|a| a.id).collect();
            for (k, preset) in p.disc.languages.iter().enumerate() {
                let r = t.resolve_language(preset);
                let a = r.audio.and_then(|id| audio.iter().position(|x| *x == id)).map(|i| i as u16 + 1);
                let pg = r.subtitle.and_then(|id| subs.iter().position(|s| s.id == id)).map(|i| i as u16 + 1);
                if a.is_none() && subs.is_empty() {
                    continue;
                }
                commands.push(Command::Compare(Cmp::Eq, Operand::Gpr(GPR_LANGUAGE), Operand::Imm(k as u32 + 1)));
                commands.push(Command::SetStream { audio: a, pg, display: pg.is_some() });
            }
            let end = match t.end_action {
                // Loop past the stream selection so a viewer's choice sticks.
                EndAction::Loop => Command::Goto(commands.len() as u32),
                EndAction::PlayNextTitle if i + 1 < p.titles.len() => Command::JumpTitle(i as u32 + 2),
                _ => match menu_obj {
                    Some(o) => Command::JumpObject(o),
                    None if i + 1 < p.titles.len() => Command::JumpTitle(i as u32 + 2),
                    None => Command::Break,
                },
            };
            commands.extend([
                Command::Move(GPR_TMP, Operand::Gpr(GPR_CHAPTER)),
                Command::Move(GPR_CHAPTER, Operand::Imm(0)),
                Command::Compare(Cmp::Eq, Operand::Gpr(GPR_TMP), Operand::Imm(0)),
                Command::PlayPl(pl),
                Command::Compare(Cmp::Ne, Operand::Gpr(GPR_TMP), Operand::Imm(0)),
                Command::PlayPlAtMark(pl, Operand::Gpr(GPR_TMP)),
            ]);
            // Play All: continue with the next title, or after the last one
            // return to the menu it was started from.
            let play_all_at = commands.len() as u32 + 3;
            commands.push(Command::Compare(Cmp::Eq, Operand::Gpr(GPR_PLAY_ALL), Operand::Imm(1)));
            commands.push(Command::Goto(play_all_at));
            commands.push(end);
            if i + 1 < p.titles.len() {
                commands.push(Command::JumpTitle(i as u32 + 2));
            } else {
                commands.push(Command::Move(GPR_PLAY_ALL, Operand::Imm(0)));
                for m in 0..m_count {
                    commands.push(Command::Compare(Cmp::Eq, Operand::Gpr(GPR_PLAY_ALL_MENU), Operand::Imm(m as u32)));
                    commands.push(Command::JumpObject(object_for_menu(m)));
                }
                commands.push(match first_menu {
                    Some(o) => Command::JumpObject(o),
                    None => Command::Break,
                });
            }
            objects.push(MovieObject { resume_intention: true, commands, ..Default::default() });
        }

        let top = match first_menu {
            Some(o) => ObjectRef { object_id: o as u16, playback_type: PlaybackType::Interactive },
            None => ObjectRef { object_id: 0xFFFF, playback_type: PlaybackType::Interactive },
        };
        let titles = (0..p.titles.len())
            .map(|i| ObjectRef { object_id: self.object_for_title(i) as u16, playback_type: PlaybackType::Movie })
            .collect();
        let index = Index {
            first_play: ObjectRef { object_id: 0, playback_type: PlaybackType::Interactive },
            top_menu: top,
            titles,
            video: p.disc.video,
        };
        (index, objects)
    }

    /// HDMV commands for a button on menu number `menu_index`.
    fn button_commands(&self, action: Action, menu_index: usize) -> Vec<Command> {
        let p = self.project;
        match action {
            Action::None => vec![],
            Action::PlayTitle { title, chapter } => match p.titles.iter().position(|t| t.id == title) {
                Some(i) => vec![
                    Command::Move(GPR_PLAY_ALL, Operand::Imm(0)),
                    Command::Move(GPR_CHAPTER, Operand::Imm(chapter)),
                    Command::JumpTitle(i as u32 + 1),
                ],
                None => vec![],
            },
            Action::PlayAll if !p.titles.is_empty() => vec![
                Command::Move(GPR_PLAY_ALL, Operand::Imm(1)),
                Command::Move(GPR_PLAY_ALL_MENU, Operand::Imm(menu_index as u32)),
                Command::Move(GPR_CHAPTER, Operand::Imm(0)),
                Command::JumpTitle(1),
            ],
            Action::PlayAll => vec![],
            Action::ShowMenu(menu) => match p.menus.iter().position(|m| m.id == menu) {
                Some(i) => vec![Command::JumpObject(object_for_menu(i))],
                None => vec![],
            },
            Action::SetLanguage { preset, menu } => match p.disc.languages.iter().position(|l| l.id == preset) {
                Some(k) => {
                    let mut c = vec![Command::Move(GPR_LANGUAGE, Operand::Imm(k as u32 + 1))];
                    if let Some(i) = menu.and_then(|m| p.menus.iter().position(|x| x.id == m)) {
                        c.push(Command::JumpObject(object_for_menu(i)));
                    }
                    c
                }
                None => vec![],
            },
        }
    }

    fn encode(&mut self, args: Vec<String>, duration: f64) -> Result<()> {
        let emit = self.emit;
        let (done, total) = (self.done_work, self.total_work);
        ffmpeg::run(&args, self.cancel, |t| {
            emit(BuildEvent::Progress(((done + t.min(duration)) / total).clamp(0.0, 1.0)));
        })?;
        self.done_work += duration;
        Ok(())
    }

    fn remux(&mut self, input: &Path, n: u32, streams: &[EsInfo], extra: Vec<(usize, Pes)>, duration: f64) -> Result<ts::mux::MuxStats> {
        let out = layout::stream_path(&self.out, n);
        let emit = self.emit;
        let cancel = self.cancel;
        let (done, total) = (self.done_work, self.total_work);
        let mut first: Option<u64> = None;
        let stats = ts::remux(input, &out, streams, extra, |t90| {
            let f = *first.get_or_insert(t90);
            let secs = (t90.saturating_sub(f)) as f64 / 90_000.0;
            emit(BuildEvent::Progress(((done + secs.min(duration) * 0.1) / total).clamp(0.0, 1.0)));
            !cancel.load(Ordering::Relaxed)
        })?;
        self.done_work += duration * 0.1;
        self.progress(0.0);
        let _ = std::fs::remove_file(input);
        Ok(stats)
    }

    fn clip_info(&self, stats: &ts::mux::MuxStats, streams: Vec<EsInfo>) -> Result<(ClipInfo, u32, u32)> {
        let first = stats.first_video_pts.context("clip has no video")?;
        let video = streams
            .iter()
            .find_map(|s| match s.kind {
                EsKind::Video(v) => Some(v),
                _ => None,
            })
            .unwrap_or(self.settings.video);
        let last = stats.last_video_pts.unwrap_or(first) + video.frame_ticks();
        let (in_t, out_t) = ((first / 2) as u32, (last / 2) as u32);
        let ci = ClipInfo {
            ts_recording_rate: (ts::mux::MUX_RATE / 8) as u32,
            num_source_packets: stats.num_packets,
            presentation_start: in_t,
            presentation_end: out_t,
            streams,
            ep_map: stats.ep_map.clone(),
        };
        Ok((ci, in_t, out_t))
    }

    fn audio_es(&self, info: &crate::media::probe::MediaInfo, lang: &str) -> EsInfo {
        EsInfo {
            pid: PID_AUDIO_FIRST,
            kind: EsKind::Audio {
                codec: self.settings.audio,
                channels: transcode::output_channels(info.audio_channels),
                lang: lang.into(),
            },
        }
    }

    fn build_title(&mut self, t: &Title, n: u32) -> Result<(Playlist, ClipInfo)> {
        let p = self.project;
        let asset = p.asset(t.asset).context("title without video")?;
        let tmp = self.work.join(format!("title-{}.ts", clip_name(n)));
        let mut chapters: Vec<f64> = t.chapters.iter().copied().filter(|&c| c > 0.0 && c < asset.info.duration).collect();
        chapters.sort_by(f64::total_cmp);
        chapters.dedup();
        let duration = asset.info.duration.max(1.0);

        let audio = title_audio(p, t, &self.settings, 0.0)?;
        let inputs: Vec<transcode::AudioInput> = audio.iter().map(|(i, _)| i.clone()).collect();
        // Either copy the original video or encode it in the disc format.
        let vformat = if t.keep_video {
            self.stage(format!("Checking the video of “{}”", t.name));
            let report = compat::analyze(&asset.path)?;
            let Some(format) = report.format.filter(|_| report.compatible()) else {
                bail!(
                    "“{}” can't keep its original video ({}). Turn off “Keep Original Video” to re-encode it.",
                    t.name,
                    report.summary()
                );
            };
            self.stage(format!("Copying the video of “{}”", t.name));
            let args = transcode::passthrough_args(&asset.path, &asset.info, &self.settings, &inputs, None, &tmp);
            self.encode(args, duration)?;
            format
        } else {
            self.stage(format!("Encoding title “{}”", t.name));
            let args = transcode::title_args(&asset.path, &asset.info, &self.settings, &chapters, &inputs, None, &tmp);
            self.encode(args, duration)?;
            self.settings.video
        };

        self.stage(format!("Multiplexing title “{}”", t.name));
        let mut streams = vec![EsInfo { pid: PID_VIDEO, kind: EsKind::Video(vformat) }];
        streams.extend(audio.into_iter().map(|(_, es)| es));

        // Subtitles → PG streams, timed against the encoded video.
        let mut extra = Vec::new();
        let tracks: Vec<&SubtitleTrack> = t.disc_subtitles().collect();
        if !tracks.is_empty() {
            let first_pts = ts::first_video_pts(&tmp)?;
            for (i, track) in tracks.iter().enumerate() {
                self.check_cancel()?;
                self.stage(format!("Converting subtitles “{}” ({}) for “{}”", track.name, track.lang, t.name));
                let images = subtitles::prepare(
                    track,
                    &asset.path,
                    &asset.info,
                    vformat,
                    &p.disc.subtitle_style,
                    &self.work.join("subtitles"),
                    None,
                    self.cancel,
                )
                .with_context(|| format!("subtitle track “{}” of “{}”", track.name, t.name))?;
                let pes = subtitles::pgs::encode(&images, vformat, |secs| {
                    (secs >= 0.0).then(|| first_pts + (secs * 90_000.0).round() as u64)
                })?;
                (self.emit)(BuildEvent::Log(format!("  {} subtitle images", images.len())));
                let index = streams.len();
                streams.push(EsInfo { pid: PID_PG_FIRST + i as u16, kind: EsKind::Pg { lang: track.lang.clone() } });
                extra.extend(pes.into_iter().map(|p| (index, p)));
            }
            self.stage(format!("Multiplexing title “{}”", t.name));
        }
        let stats = self.remux(&tmp, n, &streams, extra, duration)?;
        let (ci, in_t, out_t) = self.clip_info(&stats, streams.clone())?;

        let mut marks = vec![Mark { play_item: 0, time: in_t }];
        for c in chapters {
            let time = in_t + (c * 45_000.0) as u32;
            if time < out_t {
                marks.push(Mark { play_item: 0, time });
            }
        }
        let pl = Playlist {
            items: vec![PlayItem {
                clip_id: clip_name(n),
                in_time: in_t,
                out_time: out_t,
                still: StillMode::None,
                streams,
            }],
            marks,
        };
        Ok((pl, ci))
    }

    fn build_menu(&mut self, m: &Menu, n: u32) -> Result<(Playlist, ClipInfo)> {
        let p = self.project;
        let (w, h) = self.settings.video.size();
        self.stage(format!("Rendering menu “{}”", m.name));
        let images = ImageCache::new_sync();
        let still = self.work.join(format!("menu-{}.png", clip_name(n)));
        let motion = m.background.video.and_then(|id| p.asset(id));
        render::render_static_png(p, m, &images, w, h, motion.is_none(), &still)?;

        // IG buttons. Button ids are unique across the disc so that the
        // player's remembered selection (PSR10) only applies to the menu it
        // came from.
        let items: Vec<&MenuItem> = m.buttons().collect();
        let nav = m.navigation();
        let base = ((n - 1) as u16) * 256;
        let id_of = |id: Id| base + items.iter().position(|i| i.id == id).unwrap_or(0) as u16;
        let mut buttons = Vec::new();
        for (bi, item) in items.iter().enumerate() {
            let b = item.button().unwrap();
            let (x, y, _, _) = render::button_geometry(item, w, h);
            buttons.push(ig::Button {
                id: base + bi as u16,
                numeric: bi as u16 + 1,
                x,
                y,
                up: id_of(nav[bi][0]),
                down: id_of(nav[bi][1]),
                left: id_of(nav[bi][2]),
                right: id_of(nav[bi][3]),
                normal: render::render_button_bitmap(item, b, ButtonState::Normal, w, h),
                selected: render::render_button_bitmap(item, b, ButtonState::Selected, w, h),
                activated: render::render_button_bitmap(item, b, ButtonState::Activated, w, h),
                commands: self.button_commands(b.action, (n - 1) as usize),
                auto_action: false,
            });
        }
        let default_button = m
            .default_button
            .and_then(|d| items.iter().position(|i| i.id == d))
            .map_or(ig::NO_BUTTON, |i| base + i as u16);

        self.stage(format!("Encoding menu “{}”", m.name));
        let duration = self.menu_duration(m);
        let audio = m.audio.and_then(|id| p.asset(id)).filter(|a| a.info.has_audio());
        let tmp = self.work.join(format!("menu-{}.ts", clip_name(n)));
        let args = transcode::menu_args(
            &still,
            motion.map(|a| a.path.as_path()),
            audio.map(|a| (a.path.as_path(), &a.info)),
            duration,
            &self.settings,
            &tmp,
        );
        self.encode(args, duration)?;

        self.stage(format!("Multiplexing menu “{}”", m.name));
        let mut streams = vec![EsInfo { pid: PID_VIDEO, kind: EsKind::Video(self.settings.video) }];
        if let Some(a) = audio {
            streams.push(self.audio_es(&a.info, "und"));
        }
        let mut extra = Vec::new();
        if !buttons.is_empty() {
            let first_pts = ts::first_video_pts(&tmp)?;
            let ig_menu = ig::Menu { video: self.settings.video, buttons, default_button };
            let ig_index = streams.len();
            streams.push(EsInfo { pid: PID_IG_FIRST, kind: EsKind::Ig { lang: "und".into() } });
            extra = ig::encode(&ig_menu, first_pts)?.into_iter().map(|p| (ig_index, p)).collect();
        }
        let stats = self.remux(&tmp, n, &streams, extra, duration)?;
        let (ci, in_t, out_t) = self.clip_info(&stats, streams.clone())?;
        let still_mode = if m.is_motion() { StillMode::None } else { StillMode::Infinite };
        let pl = Playlist {
            items: vec![PlayItem { clip_id: clip_name(n), in_time: in_t, out_time: out_t, still: still_mode, streams }],
            marks: vec![Mark { play_item: 0, time: in_t }],
        };
        Ok((pl, ci))
    }
}

/// Convenience wrapper used by the UI and the command line.
/// Whether `out` names a disc image rather than a folder.
pub fn is_image(out: &Path) -> bool {
    out.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso"))
}

/// Build the disc into a folder, or into a UDF image when `out` ends in
/// ".iso" (built in a temporary folder next to it, which is then removed).
pub fn build(project: &Project, out: &Path, cancel: &AtomicBool, emit: &dyn Fn(BuildEvent)) -> Result<PathBuf> {
    if !is_image(out) {
        return Builder::new(project, out, cancel, emit).run();
    }
    let folder = out.with_extension("spindle-tmp");
    let scaled = |ev: BuildEvent| match ev {
        BuildEvent::Progress(p) => emit(BuildEvent::Progress(p * 0.9)),
        ev => emit(ev),
    };
    let result = (|| -> Result<()> {
        Builder::new(project, &folder, cancel, &scaled).run()?;
        emit(BuildEvent::Stage("Writing disc image".into()));
        crate::bluray::udf::write_image(&folder, out, &project.disc.name, |f| {
            emit(BuildEvent::Progress(0.9 + f * 0.1));
            !cancel.load(Ordering::Relaxed)
        })
    })();
    let _ = std::fs::remove_dir_all(&folder);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result.map(|_| out.to_path_buf())
}
