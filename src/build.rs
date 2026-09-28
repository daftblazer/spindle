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
use crate::encode_cache;
use crate::subtitles;
use crate::media::transcode::{self, EncodeSettings};
use crate::media::ffmpeg;
use crate::media::probe::AudioStream;
use crate::model::*;
use crate::render::{self, ButtonState, ImageCache};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

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
/// First of one flag per menu: its intro has played.
const GPR_INTRO_FIRST: u16 = 16;

#[derive(Debug, Clone)]
pub enum BuildEvent {
    Stage(String),
    Progress(f64),
    Log(String),
    /// A step of the build, listed before it starts. Steps are shown by
    /// `group`, then in the order they were added.
    AddTask { key: String, name: String, group: TaskGroup },
    Task { key: String, state: TaskState, detail: String, fraction: f64 },
    Finished(Result<PathBuf, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskGroup {
    Menus,
    Titles,
    Disc,
    Image,
    Burn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Waiting,
    Running,
    Done,
    Failed,
}

/// Work done on each step, for the overall progress. Work is measured in
/// seconds of media to encode (a pass over it), plus 10% for remuxing.
struct Tracker<'a> {
    emit: &'a (dyn Fn(BuildEvent) + Sync),
    /// (key, total, done, state)
    tasks: Mutex<Vec<(String, f64, f64, TaskState)>>,
}

impl Tracker<'_> {
    fn add(&self, key: &str, name: &str, group: TaskGroup, work: f64) {
        self.tasks.lock().unwrap().push((key.into(), work.max(1e-3), 0.0, TaskState::Waiting));
        (self.emit)(BuildEvent::AddTask { key: key.into(), name: name.into(), group });
    }

    /// Set a step's work done so far (at most its total) and describe it.
    fn update(&self, key: &str, state: TaskState, done: f64, detail: &str) {
        let (fraction, overall) = {
            let mut tasks = self.tasks.lock().unwrap();
            let mut fraction = 0.0;
            if let Some(t) = tasks.iter_mut().find(|t| t.0 == key) {
                t.2 = if state == TaskState::Done { t.1 } else { done.clamp(0.0, t.1) };
                t.3 = state;
                fraction = t.2 / t.1;
            }
            let (total, done) = tasks.iter().fold((0.0, 0.0), |(a, b), t| (a + t.1, b + t.2));
            (fraction, done / total.max(1e-3))
        };
        (self.emit)(BuildEvent::Task { key: key.into(), state, detail: detail.into(), fraction });
        (self.emit)(BuildEvent::Progress(overall.clamp(0.0, 1.0)));
    }
}

/// How many titles are encoded at once. x264 already uses every core for
/// one 1080p stream up to about eight; more cores are shared out.
pub fn parallel_jobs(set: &EncodeSettings) -> usize {
    if let Some(n) = std::env::var("SPINDLE_JOBS").ok().and_then(|n| n.parse::<usize>().ok()) {
        return n.clamp(1, 16);
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    if set.effective_encoder().is_hardware() {
        2
    } else {
        (cores / 8).clamp(1, 3)
    }
}

/// A title or menu intro, encoded by one of the workers.
enum Job {
    Title(usize),
    Intro(usize),
}

fn title_key(i: usize) -> String {
    format!("title-{i}")
}

fn intro_key(i: usize) -> String {
    format!("intro-{i}")
}

/// What building a title involves.
struct TitlePlan<'p> {
    asset: &'p Asset,
    /// Chapter starts after the first, in seconds.
    chapters: Vec<f64>,
    duration: f64,
    audio: Vec<(transcode::AudioInput, EsInfo)>,
    burn: Option<subtitles::BurnIn>,
    /// The encode command and its cache entry; `None` keeps the video.
    encode: Option<(Vec<String>, PathBuf)>,
}

impl TitlePlan<'_> {
    /// Work units of the title (see [`Tracker`]).
    fn work(&self, two_pass: bool) -> f64 {
        let passes = match &self.encode {
            None => 1.0,
            Some((_, cached)) if cached.exists() => 0.0,
            Some(_) if two_pass => 2.0,
            Some(_) => 1.0,
        };
        self.duration * (passes + 0.1)
    }
}

pub struct Builder<'a> {
    project: &'a Project,
    /// Menus with their own clip and movie object (pop-up menus are part
    /// of the titles instead).
    menus: Vec<&'a Menu>,
    out: PathBuf,
    work: PathBuf,
    cancel: &'a AtomicBool,
    /// Set when the build is cancelled or a step fails, to stop the others.
    stop: AtomicBool,
    emit: &'a (dyn Fn(BuildEvent) + Sync),
    settings: EncodeSettings,
    tracker: Tracker<'a>,
    /// Cache entries being encoded; titles with the same encode wait for
    /// the first and reuse it.
    encoding: (Mutex<std::collections::HashSet<PathBuf>>, std::sync::Condvar),
}

/// Audio of a title: the ffmpeg inputs and the disc stream of each.
/// Streams already valid on Blu-ray are copied rather than re-encoded:
/// DTS, DTS-HD, TrueHD and Dolby Digital Plus always, AC-3 for kept video
/// or when the disc uses AC-3. Not when the track's channels or loudness
/// change, or in clips starting at `clip_start` > 0 (a copied stream cut
/// there keeps audio from before it). Loudness uses earlier measurements
/// (see [`measure_loudness`]).
pub fn title_audio(p: &Project, t: &Title, set: &EncodeSettings, clip_start: f64) -> Result<Vec<(transcode::AudioInput, EsInfo)>> {
    let asset = p.asset(t.asset).context("title without video")?;
    let mut out = Vec::new();
    for (track, file, stream, offset) in audio_sources(p, t)? {
        let external = file != asset.path;
        let copy = stream.bluray_codec().filter(|c| {
            let wanted = match c {
                AudioCodec::Ac3 => t.keep_video || set.audio == AudioCodec::Ac3,
                _ => true,
            };
            wanted
                && !track.reencode
                && track.layout == ChannelLayout::Original
                && !p.disc.normalize_loudness
                && clip_start <= 0.0
                && offset <= 0.0
        });
        let loudness = if p.disc.normalize_loudness && copy.is_none() {
            crate::media::loudness::cached(&file, stream.index).map(|m| crate::media::loudness::filter(&m))
        } else {
            None
        };
        let input = transcode::AudioInput {
            file: external.then(|| file.clone()),
            index: stream.index,
            offset,
            channels: stream.channels,
            copy,
            layout: track.layout.channels(),
            loudness,
        };
        let es = EsInfo {
            pid: PID_AUDIO_FIRST + out.len() as u16,
            kind: EsKind::Audio {
                codec: copy.unwrap_or(set.audio),
                channels: input.output_channels(),
                rate: if copy.is_some() { stream.sample_rate } else { 48_000 },
                lang: track.lang.clone(),
            },
        };
        out.push((input, es));
    }
    Ok(out)
}

/// Each disc audio track of `t` with its file, stream and delay.
fn audio_sources<'p>(p: &'p Project, t: &'p Title) -> Result<Vec<(&'p AudioTrack, PathBuf, AudioStream, f64)>> {
    let asset = p.asset(t.asset).context("title without video")?;
    let own = asset.info.audio();
    let mut out = Vec::new();
    for track in t.disc_audio() {
        match track.source {
            AudioSource::Embedded { index } => {
                let Some(stream) = own.iter().find(|s| s.index == index) else { continue };
                out.push((track, asset.path.clone(), stream.clone(), 0.0));
            }
            AudioSource::External { asset, offset } => {
                let a = p.asset(asset).with_context(|| format!("audio track “{}” of “{}” has no file", track.name, t.name))?;
                let stream = a.info.audio().into_iter().next().with_context(|| format!("{} has no audio", a.path.display()))?;
                out.push((track, a.path.clone(), stream, offset));
            }
        }
    }
    Ok(out)
}

/// Measure the loudness of the tracks of `t` that will be adjusted.
pub fn measure_loudness(p: &Project, t: &Title, set: &EncodeSettings, cancel: &AtomicBool) -> Result<()> {
    if !p.disc.normalize_loudness {
        return Ok(());
    }
    let copied: Vec<bool> = title_audio(p, t, set, 0.0)?.iter().map(|(i, _)| i.copy.is_some()).collect();
    for ((track, file, stream, _), copied) in audio_sources(p, t)?.into_iter().zip(copied) {
        if !copied {
            crate::media::loudness::measure(&file, stream.index, cancel).with_context(|| format!("audio track “{}” of “{}”", track.name, t.name))?;
        }
    }
    Ok(())
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

/// Stream selection for `preset` in `t`, if it changes anything.
fn language_streams(t: &Title, preset: &LanguagePreset) -> Option<Command> {
    let audio: Vec<Id> = t.disc_audio().map(|a| a.id).collect();
    let subs: Vec<Id> = t.disc_subtitles().map(|s| s.id).collect();
    let r = t.resolve_language(preset);
    let a = r.audio.and_then(|id| audio.iter().position(|x| *x == id)).map(|i| i as u16 + 1);
    let pg = r.subtitle.and_then(|id| subs.iter().position(|x| *x == id)).map(|i| i as u16 + 1);
    (a.is_some() || !subs.is_empty()).then_some(Command::SetStream { audio: a, pg, display: pg.is_some() })
}

/// Pop-up menus close after this long without input.
const POPUP_TIMEOUT_SECONDS: u32 = 30;

fn seconds_to_ticks(s: f64) -> u32 {
    (s.clamp(0.0, 10.0) * 90_000.0) as u32
}

fn object_for_menu(i: usize) -> u32 {
    1 + i as u32
}

impl<'a> Builder<'a> {
    pub fn new(project: &'a Project, out: &Path, cancel: &'a AtomicBool, emit: &'a (dyn Fn(BuildEvent) + Sync)) -> Self {
        let settings = EncodeSettings::for_disc(&project.disc);
        Builder {
            project,
            menus: project.disc_menus().collect(),
            out: out.to_path_buf(),
            work: out.join(".spindle-work"),
            cancel,
            stop: AtomicBool::new(false),
            emit,
            settings,
            tracker: Tracker { emit, tasks: Mutex::new(Vec::new()) },
            encoding: Default::default(),
        }
    }

    /// Position of a disc menu (not a pop-up) among the menu clips.
    fn menu_index(&self, id: Id) -> Option<usize> {
        self.menus.iter().position(|m| m.id == id)
    }

    /// Playlist of the intro of disc menu `i`, if it has one. Intros come
    /// after the menus and titles.
    fn intro_playlist(&self, i: usize) -> Option<u32> {
        self.menus[i].intro?;
        let k = self.menus[..i].iter().filter(|m| m.intro.is_some()).count();
        Some(1 + (self.menus.len() + self.project.titles.len() + k) as u32)
    }

    /// Titles are encoded in two passes (Best quality with x264).
    fn two_pass(&self) -> bool {
        self.settings.quality == transcode::Quality::Best && !self.settings.effective_encoder().is_hardware()
    }

    fn object_for_title(&self, i: usize) -> u32 {
        1 + self.menus.len() as u32 + i as u32
    }

    fn check_cancel(&self) -> Result<()> {
        if self.stop.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        Ok(())
    }

    fn log(&self, s: String) {
        (self.emit)(BuildEvent::Log(s));
    }

    /// Log a step and show it as the headline (for the command line and
    /// older consumers; the build dialog shows the steps themselves).
    fn stage(&self, s: String) {
        (self.emit)(BuildEvent::Log(s.clone()));
        (self.emit)(BuildEvent::Stage(s));
    }

    fn menu_duration(&self, m: &Menu) -> f64 {
        if m.is_motion() {
            m.duration.max(1.0)
        } else {
            STILL_MENU_SECONDS
        }
    }

    pub fn run(self) -> Result<PathBuf> {
        let problems = check(self.project);
        if !problems.is_empty() {
            bail!("{}", problems.join("\n"));
        }
        let encoder = self.settings.effective_encoder();
        if encoder.is_hardware() {
            if !crate::media::hwenc::available().contains(&encoder) {
                bail!("The {} video encoder doesn't work on this computer. Choose Software in Disc Settings.", encoder.label());
            }
            self.log(format!("Using {} video encoding: for test discs only, the quality is lower than Software", encoder.label()));
        }
        let p = self.project;

        // The steps, with their share of the work.
        if !self.menus.is_empty() {
            let work: f64 = self.menus.iter().map(|m| self.menu_duration(m) * 1.1).sum();
            self.tracker.add("menus", "Menus", TaskGroup::Menus, work);
        }
        let mut jobs = Vec::new();
        for (i, t) in p.titles.iter().enumerate() {
            let work = self.title_plan(t, self.title_clip(i)).map_or(1.0, |plan| plan.work(self.two_pass()));
            self.tracker.add(&title_key(i), &t.name, TaskGroup::Titles, work);
            jobs.push(Job::Title(i));
        }
        for (i, m) in self.menus.iter().enumerate() {
            if let (Some(_), Some(a)) = (self.intro_playlist(i), m.intro.and_then(|a| p.asset(a))) {
                let name = format!("Intro of “{}”", m.name);
                self.tracker.add(&intro_key(i), &name, TaskGroup::Titles, a.info.duration.max(1.0) * 1.1);
                jobs.push(Job::Intro(i));
            }
        }
        self.tracker.add("nav", "Disc Navigation", TaskGroup::Disc, 0.5);

        std::fs::create_dir_all(&self.work).with_context(|| format!("creating {}", self.work.display()))?;
        if layout::bdmv_dir(&self.out).exists() {
            std::fs::remove_dir_all(layout::bdmv_dir(&self.out))?;
        }
        layout::create_dirs(&self.out)?;

        let finished = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            // Pass a cancel from the user on to every step.
            scope.spawn(|| {
                while !finished.load(Ordering::Relaxed) {
                    if self.cancel.load(Ordering::Relaxed) {
                        self.stop.store(true, Ordering::Relaxed);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
            let r = self.run_steps(&jobs);
            finished.store(true, Ordering::Relaxed);
            r
        });
        let (mut playlists, mut clips) = match result {
            Ok(v) => v,
            Err(e) => {
                // Steps that were under way stopped with the failure.
                let running: Vec<String> = self.tracker.tasks.lock().unwrap().iter().filter(|t| t.3 == TaskState::Running).map(|t| t.0.clone()).collect();
                for key in running {
                    (self.emit)(BuildEvent::Task { key, state: TaskState::Waiting, detail: "Stopped".into(), fraction: 0.0 });
                }
                return Err(e);
            }
        };
        playlists.sort_by_key(|(n, _)| *n);
        clips.sort_by_key(|(n, _)| *n);

        self.tracker.update("nav", TaskState::Running, 0.0, "Writing");
        self.stage("Writing disc navigation".into());
        let (index, objects) = self.navigation();
        layout::write_database(&self.out, &index, &objects, &playlists, &clips)?;
        self.tracker.update("nav", TaskState::Done, 0.0, &format!("{} playlists", playlists.len()));
        let _ = std::fs::remove_dir_all(&self.work);
        encode_cache::trim();
        (self.emit)(BuildEvent::Progress(1.0));
        Ok(self.out.clone())
    }

    /// Menus, then the titles and intros (several at once when the
    /// computer has the cores for it).
    #[allow(clippy::type_complexity)]
    fn run_steps(&self, jobs: &[Job]) -> Result<(Vec<(u32, Playlist)>, Vec<(u32, ClipInfo)>)> {
        let mut playlists = Vec::new();
        let mut clips = Vec::new();
        if !self.menus.is_empty() {
            let mut done = 0.0;
            for (i, m) in self.menus.iter().enumerate() {
                self.check_cancel()?;
                let n = 1 + i as u32;
                let (pl, ci) = self.build_menu(m, n, done).inspect_err(|_| self.fail("menus"))?;
                done += self.menu_duration(m) * 1.1;
                playlists.push((n, pl));
                clips.push((n, ci));
            }
            let count = self.menus.len();
            self.tracker.update("menus", TaskState::Done, 0.0, &if count == 1 { "1 menu".to_string() } else { format!("{count} menus") });
        }

        let next = AtomicUsize::new(0);
        let results = Mutex::new(Vec::new());
        let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
        let workers = parallel_jobs(&self.settings).min(jobs.len());
        if workers > 1 {
            self.log(format!("Encoding {workers} titles at a time"));
        }
        // Seconds of video encoded (not reused or copied), for the speed.
        let encoded = Mutex::new(0.0);
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    if self.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
                    let (key, res) = match *job {
                        Job::Title(i) => (title_key(i), self.build_title(&self.project.titles[i], i)),
                        Job::Intro(i) => (intro_key(i), self.build_intro(i)),
                    };
                    match res {
                        Ok((r, secs)) => {
                            *encoded.lock().unwrap() += secs;
                            results.lock().unwrap().push(r);
                        }
                        Err(e) => {
                            // The first failure stops the others.
                            if !self.stop.swap(true, Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed) {
                                self.fail(&key);
                                error.lock().unwrap().get_or_insert(e);
                            }
                            break;
                        }
                    }
                });
            }
        });
        if let Some(e) = error.into_inner().unwrap() {
            return Err(e);
        }
        self.check_cancel()?;
        encode_cache::record_speed(&speed_key(&self.settings), encoded.into_inner().unwrap(), started.elapsed().as_secs_f64());
        for (n, pl, ci) in results.into_inner().unwrap() {
            playlists.push((n, pl));
            clips.push((n, ci));
        }
        Ok((playlists, clips))
    }

    fn fail(&self, key: &str) {
        if !self.cancel.load(Ordering::Relaxed) {
            self.tracker.update(key, TaskState::Failed, 0.0, "Failed");
        }
    }

    fn navigation(&self) -> (Index, Vec<MovieObject>) {
        let p = self.project;
        let m_count = self.menus.len();
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

        for (i, m) in self.menus.iter().enumerate() {
            let pl = Operand::Imm(1 + i as u32);
            let mut commands = Vec::new();
            // Intro: once per disc session (a flag per menu) or every time.
            if let Some(intro) = self.intro_playlist(i) {
                if m.intro_every_time {
                    commands.push(Command::PlayPl(Operand::Imm(intro)));
                } else {
                    let flag = GPR_INTRO_FIRST + i as u16;
                    let after = commands.len() as u32 + 4;
                    commands.extend([
                        Command::Compare(Cmp::Ne, Operand::Gpr(flag), Operand::Imm(0)),
                        Command::Goto(after),
                        Command::Move(flag, Operand::Imm(1)),
                        Command::PlayPl(Operand::Imm(intro)),
                    ]);
                }
            }
            let timeout = m.timeout.filter(|t| t.action != Action::None && t.seconds > 0);
            match timeout {
                // A still menu holds for the timeout, then acts.
                Some(t) if !m.is_motion() => {
                    commands.push(Command::PlayPl(pl));
                    commands.extend(self.button_commands(t.action, i));
                }
                // A motion menu counts its loops.
                Some(t) => {
                    let loops = (t.seconds as f64 / self.menu_duration(m)).ceil().max(1.0) as u32;
                    commands.push(Command::Move(GPR_TMP, Operand::Imm(0)));
                    let top = commands.len() as u32;
                    commands.extend([
                        Command::PlayPl(pl),
                        Command::Add(GPR_TMP, Operand::Imm(1)),
                        Command::Compare(Cmp::Lt, Operand::Gpr(GPR_TMP), Operand::Imm(loops)),
                        Command::Goto(top),
                    ]);
                    commands.extend(self.button_commands(t.action, i));
                }
                None => {}
            }
            // Loop the menu (also after a timeout that did nothing).
            let top = commands.len() as u32;
            commands.extend([Command::PlayPl(pl), Command::Goto(top)]);
            objects.push(MovieObject { menu_call_mask: true, commands, ..Default::default() });
        }

        for (i, t) in p.titles.iter().enumerate() {
            let pl = Operand::Imm(1 + (m_count + i) as u32);
            let menu_obj = t
                .return_menu
                .and_then(|id| self.menu_index(id))
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
            for (k, preset) in p.disc.languages.iter().enumerate() {
                if let Some(c) = language_streams(t, preset) {
                    commands.push(Command::Compare(Cmp::Eq, Operand::Gpr(GPR_LANGUAGE), Operand::Imm(k as u32 + 1)));
                    commands.push(c);
                }
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

        let top_menu = p.disc.top_menu.and_then(|id| self.menu_index(id)).map(object_for_menu).or(first_menu);
        let top = match top_menu {
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

    /// HDMV commands for a button on pop-up page of title `title` (index),
    /// whose pages are `pages`; `marks` is the number of playlist marks.
    fn popup_commands(&self, action: Action, title: usize, pages: &[Id], marks: usize) -> Vec<Command> {
        let p = self.project;
        let t = &p.titles[title];
        let page_of = |id: Id| pages.iter().position(|x| *x == id);
        match action {
            // Chapters of the playing title: jump within the playlist.
            Action::PlayTitle { title: id, chapter } if id == t.id => {
                vec![Command::LinkMark(Operand::Imm((chapter as usize).min(marks.saturating_sub(1)) as u32))]
            }
            Action::PlayChapter(chapter) => vec![Command::LinkMark(Operand::Imm((chapter as usize).min(marks.saturating_sub(1)) as u32))],
            Action::ShowMenu(id) if page_of(id).is_some() => {
                vec![Command::SetButtonPage { button: None, page: page_of(id).map(|i| i as u8) }]
            }
            Action::SetLanguage { preset, menu } => {
                let Some(k) = p.disc.languages.iter().position(|l| l.id == preset) else { return vec![] };
                let mut c = vec![Command::Move(GPR_LANGUAGE, Operand::Imm(k as u32 + 1))];
                c.extend(language_streams(t, &p.disc.languages[k]));
                match menu {
                    Some(m) if page_of(m).is_some() => c.push(Command::SetButtonPage { button: None, page: page_of(m).map(|i| i as u8) }),
                    Some(m) => c.extend(self.button_commands(Action::ShowMenu(m), 0)),
                    None => {}
                }
                c
            }
            // Everything else leaves the title as a menu button would.
            other => self.button_commands(other, 0),
        }
    }

    /// Interactive graphics of a title's pop-up menu, if it has one.
    fn popup_menu(&self, t: &Title, title: usize, marks: usize) -> Option<ig::Menu> {
        let p = self.project;
        let popup = p.title_popup(t)?;
        let pages = p.popup_pages(popup);
        let page_ids: Vec<Id> = pages.iter().map(|m| m.id).collect();
        let images = ImageCache::new_sync();
        let mut ig_pages = Vec::new();
        for (pi, m) in pages.iter().enumerate() {
            let frame = render::DiscFrame::for_menu(self.settings.video, m.shape);
            // Chapter buttons for chapters this title doesn't have are left out.
            let mut m = (*m).clone();
            m.items.retain(|i| !matches!(i.button().map(|b| b.action), Some(Action::PlayChapter(c)) if c as usize >= marks));
            let m = &m;
            let base = (pi as u16) << 8;
            let items: Vec<&MenuItem> = m.buttons().take(254).collect();
            let nav = m.navigation();
            let id_of = |id: Id| base + items.iter().position(|i| i.id == id).unwrap_or(0) as u16;
            let mut buttons = Vec::new();
            // Everything that isn't a button, as one button nothing leads to.
            if let Some((x, y, bmp)) = render::render_static_bitmap(p, m, &images, &frame) {
                let id = base + 0xFF;
                buttons.push(ig::Button {
                    id,
                    numeric: 0xFFFF,
                    x,
                    y,
                    up: id,
                    down: id,
                    left: id,
                    right: id,
                    normal: bmp.clone(),
                    selected: bmp.clone(),
                    activated: bmp,
                    commands: vec![],
                    auto_action: false,
                });
            }
            for (bi, item) in items.iter().enumerate() {
                let b = item.button().unwrap();
                let (x, y, _, _) = render::button_geometry(item, &frame);
                buttons.push(ig::Button {
                    id: base + bi as u16,
                    numeric: bi as u16 + 1,
                    x,
                    y,
                    up: id_of(nav[bi][0]),
                    down: id_of(nav[bi][1]),
                    left: id_of(nav[bi][2]),
                    right: id_of(nav[bi][3]),
                    normal: render::render_button_bitmap(p, &images, item, b, ButtonState::Normal, &frame),
                    selected: render::render_button_bitmap(p, &images, item, b, ButtonState::Selected, &frame),
                    activated: render::render_button_bitmap(p, &images, item, b, ButtonState::Activated, &frame),
                    commands: self.popup_commands(b.action, title, &page_ids, marks),
                    auto_action: false,
                });
            }
            let default = m.default_button.and_then(|d| items.iter().position(|i| i.id == d)).unwrap_or(0);
            let default_button = if items.is_empty() { ig::NO_BUTTON } else { base + default as u16 };
            ig_pages.push(ig::Page { buttons, default_button, fade_in: seconds_to_ticks(m.fade_in), fade_out: seconds_to_ticks(m.fade_out) });
        }
        Some(ig::Menu { video: self.settings.video, pages: ig_pages, popup: true, user_timeout: POPUP_TIMEOUT_SECONDS * 90_000 })
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
            // Only meaningful in pop-up menus.
            Action::PlayChapter(_) => vec![],
            Action::ShowMenu(menu) => match self.menu_index(menu) {
                Some(i) => vec![Command::JumpObject(object_for_menu(i))],
                None => vec![],
            },
            Action::SetLanguage { preset, menu } => match p.disc.languages.iter().position(|l| l.id == preset) {
                Some(k) => {
                    let mut c = vec![Command::Move(GPR_LANGUAGE, Operand::Imm(k as u32 + 1))];
                    if let Some(i) = menu.and_then(|m| self.menu_index(m)) {
                        c.push(Command::JumpObject(object_for_menu(i)));
                    }
                    c
                }
                None => vec![],
            },
        }
    }

    /// Run ffmpeg for step `key`, whose work so far is `base`; `detail`
    /// describes what it does.
    fn encode(&self, key: &str, args: Vec<String>, duration: f64, base: f64, detail: &str) -> Result<()> {
        self.tracker.update(key, TaskState::Running, base, detail);
        ffmpeg::run_status(&args, &self.stop, |s| {
            let speed = s.fps.map(|f| format!(" · {f:.0} fps")).unwrap_or_default();
            self.tracker.update(key, TaskState::Running, base + s.time.min(duration), &format!("{detail}{speed}"));
        })
    }

    /// Mux `input` into clip `n` for step `key`; `work` is the step's work
    /// so far and the clip's duration.
    fn remux(&self, key: &str, input: &Path, n: u32, streams: &[EsInfo], extra: Vec<(usize, Pes)>, work: (f64, f64)) -> Result<ts::mux::MuxStats> {
        let (base, duration) = work;
        let out = layout::stream_path(&self.out, n);
        let mut first: Option<u64> = None;
        self.tracker.update(key, TaskState::Running, base, "Multiplexing");
        let mut last = std::time::Instant::now();
        let stats = ts::remux(input, &out, streams, extra, |t90| {
            // Limit the updates; this is called for every packet.
            if last.elapsed().as_millis() >= 250 {
                last = std::time::Instant::now();
                let f = *first.get_or_insert(t90);
                let secs = (t90.saturating_sub(f)) as f64 / 90_000.0;
                self.tracker.update(key, TaskState::Running, base + secs.min(duration) * 0.1, "Multiplexing");
            }
            !self.stop.load(Ordering::Relaxed)
        })?;
        // Encodes in the cache stay for the next build.
        if !input.starts_with(encode_cache::dir()) {
            let _ = std::fs::remove_file(input);
        }
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
                rate: 48_000,
                lang: lang.into(),
            },
        }
    }

    /// Clip number of title `i`.
    fn title_clip(&self, i: usize) -> u32 {
        1 + (self.menus.len() + i) as u32
    }

    /// What building title `t` as clip `n` involves.
    fn title_plan<'p>(&'p self, t: &Title, n: u32) -> Result<TitlePlan<'p>> {
        let p = self.project;
        let asset = p.asset(t.asset).context("title without video")?;
        let mut chapters: Vec<f64> = t.chapters.iter().copied().filter(|&c| c > 0.0 && c < asset.info.duration).collect();
        chapters.sort_by(f64::total_cmp);
        chapters.dedup();
        let audio = title_audio(p, t, &self.settings, 0.0)?;
        let burn = t.burned_subtitle().map(|s| subtitles::burn_in(s, &asset.path, &asset.info, &p.disc.subtitle_style)).transpose()?;
        let encode = if t.keep_video {
            None
        } else {
            let inputs: Vec<transcode::AudioInput> = audio.iter().map(|(i, _)| i.clone()).collect();
            let pass = if self.two_pass() { transcode::Pass::Second(self.work.join(format!("pass-{}", clip_name(n)))) } else { transcode::Pass::Only };
            let src = transcode::Source { path: &asset.path, info: &asset.info, picture: &t.video, burn: burn.as_ref() };
            let args = transcode::title_args(&src, &self.settings, &chapters, &inputs, None, &pass, Path::new("out.ts"));
            // Reuse an identical earlier encode: the key is the whole command
            // (without its output or pass log) and the files it reads.
            let mut key: Vec<String> = args[..args.len() - 1]
                .iter()
                .enumerate()
                .filter(|(i, _)| *i == 0 || args[i - 1] != "-passlogfile")
                .map(|(_, a)| a.clone())
                .collect();
            key.push(encode_cache::file_id(&asset.path));
            key.extend(inputs.iter().filter_map(|i| i.file.as_deref()).map(encode_cache::file_id));
            key.extend(burn.as_ref().map(|b| encode_cache::file_id(&b.source)));
            Some((args, encode_cache::entry(&key)))
        };
        Ok(TitlePlan { asset, chapters, duration: asset.info.duration.max(1.0), audio, burn, encode })
    }

    /// Take on encoding cache entry `cached`: false when it already exists
    /// (after waiting for another title making the same encode).
    fn claim(&self, cached: &Path) -> Result<bool> {
        let (lock, cond) = &self.encoding;
        let mut busy = lock.lock().unwrap();
        while busy.contains(cached) {
            self.check_cancel()?;
            busy = cond.wait_timeout(busy, std::time::Duration::from_millis(200)).unwrap().0;
        }
        if cached.exists() {
            return Ok(false);
        }
        busy.insert(cached.to_path_buf());
        Ok(true)
    }

    fn release(&self, cached: &Path) {
        let (lock, cond) = &self.encoding;
        lock.lock().unwrap().remove(cached);
        cond.notify_all();
    }

    /// Encode title `t` into cache entry `cached`; returns the work done.
    fn encode_title(&self, t: &Title, key: &str, plan: &TitlePlan, args: &[String], cached: &Path, n: u32) -> Result<f64> {
        let (asset, duration) = (plan.asset, plan.duration);
        let inputs: Vec<transcode::AudioInput> = plan.audio.iter().map(|(i, _)| i.clone()).collect();
        let two_pass = self.two_pass();
        let log = self.work.join(format!("pass-{}", clip_name(n)));
        let mut work = 0.0;
        if two_pass {
            self.stage(format!("Analysing title “{}” (pass 1 of 2)", t.name));
            let src = transcode::Source { path: &asset.path, info: &asset.info, picture: &t.video, burn: plan.burn.as_ref() };
            let first = transcode::title_args(&src, &self.settings, &plan.chapters, &inputs, None, &transcode::Pass::First(log), &self.work.join("null.ts"));
            self.encode(key, first, duration, 0.0, "Analysing · pass 1 of 2")?;
            work += duration;
        }
        let detail = if self.settings.effective_encoder().is_hardware() {
            "Encoding (hardware, for testing)"
        } else if two_pass {
            "Encoding · pass 2 of 2"
        } else {
            "Encoding"
        };
        self.stage(format!("Encoding title “{}”", t.name));
        std::fs::create_dir_all(encode_cache::dir())?;
        let part = cached.with_extension("part");
        let mut args = args.to_vec();
        *args.last_mut().expect("output") = part.to_string_lossy().into_owned();
        let res = self.encode(key, args, duration, work, detail);
        if res.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        res?;
        std::fs::rename(&part, cached).with_context(|| format!("saving the encode of “{}”", t.name))?;
        Ok(work + duration)
    }

    /// Build title `i`; also returns the seconds of video it encoded.
    fn build_title(&self, t: &Title, i: usize) -> Result<((u32, Playlist, ClipInfo), f64)> {
        let p = self.project;
        let n = self.title_clip(i);
        let key = title_key(i);
        if p.disc.normalize_loudness {
            self.tracker.update(&key, TaskState::Running, 0.0, "Measuring loudness");
            measure_loudness(p, t, &self.settings, &self.stop)?;
        }
        let plan = self.title_plan(t, n)?;
        let TitlePlan { asset, chapters, duration, audio, .. } = &plan;
        let (asset, duration) = (*asset, *duration);
        let inputs: Vec<transcode::AudioInput> = audio.iter().map(|(i, _)| i.clone()).collect();
        let mut work = 0.0;
        let mut encoded = 0.0;
        // Either copy the original video or encode it in the disc format.
        let (tmp, vformat) = match &plan.encode {
            None => {
                self.tracker.update(&key, TaskState::Running, 0.0, "Checking the video");
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
                let tmp = self.work.join(format!("title-{}.ts", clip_name(n)));
                let src = transcode::Source { path: &asset.path, info: &asset.info, picture: &t.video, burn: None };
                let args = transcode::passthrough_args(&src, &self.settings, &inputs, None, &tmp);
                self.encode(&key, args, duration, 0.0, "Copying the original video")?;
                work += duration;
                (tmp, format)
            }
            Some((args, cached)) => {
                if self.claim(cached)? {
                    let res = self.encode_title(t, &key, &plan, args, cached, n);
                    self.release(cached);
                    work += res?;
                    encoded = duration;
                } else {
                    self.stage(format!("Reusing the earlier encode of “{}”", t.name));
                    self.tracker.update(&key, TaskState::Running, 0.0, "Reusing the earlier encode");
                    encode_cache::touch(cached);
                }
                (cached.clone(), self.settings.video)
            }
        };

        let mut streams = vec![EsInfo { pid: PID_VIDEO, kind: EsKind::Video(vformat) }];
        streams.extend(audio.iter().map(|(_, es)| es.clone()));

        // Subtitles → PG streams, timed against the encoded video.
        let mut extra = Vec::new();
        let tracks: Vec<&SubtitleTrack> = t.disc_subtitles().collect();
        if !tracks.is_empty() {
            let first_pts = ts::first_video_pts(&tmp)?;
            let area = if t.keep_video {
                subtitles::default_area(&asset.info, vformat)
            } else {
                crate::media::picture::plan(&asset.info, &t.video, vformat).area
            };
            for (i, track) in tracks.iter().enumerate() {
                self.check_cancel()?;
                self.tracker.update(&key, TaskState::Running, work, &format!("Converting subtitles “{}”", track.name));
                self.stage(format!("Converting subtitles “{}” ({}) for “{}”", track.name, track.lang, t.name));
                let images = subtitles::prepare(
                    track,
                    &asset.path,
                    &asset.info,
                    &area,
                    &p.disc.subtitle_style,
                    &self.work.join(format!("subtitles-{}", clip_name(n))),
                    None,
                    &self.stop,
                )
                .with_context(|| format!("subtitle track “{}” of “{}”", track.name, t.name))?;
                let pes = subtitles::pgs::encode(&images, vformat, |secs| {
                    (secs >= 0.0).then(|| first_pts + (secs * 90_000.0).round() as u64)
                })?;
                self.log(format!("  “{}”: {} subtitle images in “{}”", t.name, images.len(), track.name));
                let index = streams.len();
                streams.push(EsInfo { pid: PID_PG_FIRST + i as u16, kind: EsKind::Pg { lang: track.lang.clone() } });
                extra.extend(pes.into_iter().map(|p| (index, p)));
            }
        }

        // Pop-up menu: one display set at the start of the clip. (Players
        // treat each repeat as a new menu, resetting it mid-use.)
        if let Some(menu) = self.popup_menu(t, i, 1 + chapters.len()) {
            self.tracker.update(&key, TaskState::Running, work, "Adding the pop-up menu");
            let first_pts = ts::first_video_pts(&tmp)?;
            let ig_index = streams.len();
            streams.push(EsInfo { pid: PID_IG_FIRST, kind: EsKind::Ig { lang: "und".into() } });
            extra.extend(ig::encode(&menu, first_pts)?.into_iter().map(|p| (ig_index, p)));
        }
        self.stage(format!("Multiplexing title “{}”", t.name));
        let stats = self.remux(&key, &tmp, n, &streams, extra, (work, duration)).with_context(|| format!("multiplexing “{}”", t.name))?;
        let (ci, in_t, out_t) = self.clip_info(&stats, streams.clone())?;
        let size = std::fs::metadata(layout::stream_path(&self.out, n)).map_or(0, |m| m.len());
        self.tracker.update(&key, TaskState::Done, 0.0, &format!("{:.2} GB", size as f64 / 1e9));

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
        Ok(((n, pl, ci), encoded))
    }

    /// The intro video of disc menu `i` as its own clip (no buttons), and
    /// the seconds of video encoded.
    fn build_intro(&self, i: usize) -> Result<((u32, Playlist, ClipInfo), f64)> {
        let m = self.menus[i];
        let n = self.intro_playlist(i).context("menu without intro")?;
        let asset = m.intro.and_then(|a| self.project.asset(a)).context("intro video is missing")?;
        let key = intro_key(i);
        self.stage(format!("Encoding the intro of menu “{}”", m.name));
        let tmp = self.work.join(format!("intro-{}.ts", clip_name(n)));
        let duration = asset.info.duration.max(1.0);
        let audio: Vec<transcode::AudioInput> = asset
            .info
            .audio()
            .first()
            .map(|s| transcode::AudioInput::encode(None, s.index, 0.0, s.channels))
            .into_iter()
            .collect();
        let opts = crate::media::picture::VideoOptions::default();
        let src = transcode::Source { path: &asset.path, info: &asset.info, picture: &opts, burn: None };
        let args = transcode::title_args(&src, &self.settings, &[], &audio, None, &transcode::Pass::Only, &tmp);
        self.encode(&key, args, duration, 0.0, "Encoding")?;
        let mut streams = vec![EsInfo { pid: PID_VIDEO, kind: EsKind::Video(self.settings.video) }];
        if asset.info.has_audio() {
            streams.push(self.audio_es(&asset.info, "und"));
        }
        let stats = self.remux(&key, &tmp, n, &streams, Vec::new(), (duration, duration))?;
        let (ci, in_t, out_t) = self.clip_info(&stats, streams.clone())?;
        self.tracker.update(&key, TaskState::Done, 0.0, "Done");
        let pl = Playlist {
            items: vec![PlayItem { clip_id: clip_name(n), in_time: in_t, out_time: out_t, still: StillMode::None, streams }],
            marks: vec![Mark { play_item: 0, time: in_t }],
        };
        Ok(((n, pl, ci), duration))
    }

    /// Disc menu clip `n`; `done` is the menus' work before it.
    fn build_menu(&self, m: &Menu, n: u32, done: f64) -> Result<(Playlist, ClipInfo)> {
        let p = self.project;
        let frame = render::DiscFrame::for_menu(self.settings.video, m.shape);
        self.tracker.update("menus", TaskState::Running, done, &format!("Drawing “{}”", m.name));
        self.stage(format!("Rendering menu “{}”", m.name));
        let images = ImageCache::new_sync();
        let still = self.work.join(format!("menu-{}.png", clip_name(n)));
        let motion = m.background.video.and_then(|id| p.asset(id));
        render::render_static_png(p, m, &images, &frame, motion.is_none(), &still)?;

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
            let (x, y, _, _) = render::button_geometry(item, &frame);
            buttons.push(ig::Button {
                id: base + bi as u16,
                numeric: bi as u16 + 1,
                x,
                y,
                up: id_of(nav[bi][0]),
                down: id_of(nav[bi][1]),
                left: id_of(nav[bi][2]),
                right: id_of(nav[bi][3]),
                normal: render::render_button_bitmap(p, &images, item, b, ButtonState::Normal, &frame),
                selected: render::render_button_bitmap(p, &images, item, b, ButtonState::Selected, &frame),
                activated: render::render_button_bitmap(p, &images, item, b, ButtonState::Activated, &frame),
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
            motion.map(|a| (a.path.as_path(), &a.info, m.background.video_start)),
            audio.map(|a| (a.path.as_path(), &a.info)),
            duration,
            &self.settings,
            &tmp,
        );
        self.encode("menus", args, duration, done, &format!("Encoding “{}”", m.name))?;

        self.stage(format!("Multiplexing menu “{}”", m.name));
        let mut streams = vec![EsInfo { pid: PID_VIDEO, kind: EsKind::Video(self.settings.video) }];
        if let Some(a) = audio {
            streams.push(self.audio_es(&a.info, "und"));
        }
        let mut extra = Vec::new();
        if !buttons.is_empty() {
            let first_pts = ts::first_video_pts(&tmp)?;
            let mut ig_menu = ig::Menu::single(self.settings.video, buttons, default_button);
            ig_menu.pages[0].fade_in = seconds_to_ticks(m.fade_in);
            ig_menu.pages[0].fade_out = seconds_to_ticks(m.fade_out);
            let ig_index = streams.len();
            streams.push(EsInfo { pid: PID_IG_FIRST, kind: EsKind::Ig { lang: "und".into() } });
            extra = ig::encode(&ig_menu, first_pts)?.into_iter().map(|p| (ig_index, p)).collect();
        }
        let stats = self.remux("menus", &tmp, n, &streams, extra, (done + duration, duration))?;
        let (ci, in_t, out_t) = self.clip_info(&stats, streams.clone())?;
        let still_mode = match m.timeout.filter(|t| t.action != Action::None && t.seconds > 0) {
            _ if m.is_motion() => StillMode::None,
            Some(t) => StillMode::Time(t.seconds.min(u16::MAX as u32) as u16),
            None => StillMode::Infinite,
        };
        let pl = Playlist {
            items: vec![PlayItem { clip_id: clip_name(n), in_time: in_t, out_time: out_t, still: still_mode, streams }],
            marks: vec![Mark { play_item: 0, time: in_t }],
        };
        Ok((pl, ci))
    }
}

/// Key of the encoding speeds remembered for `set`.
fn speed_key(set: &EncodeSettings) -> String {
    format!("{:?}/{:?}/{}", set.effective_encoder(), set.quality, set.video.size().1)
}

/// A guess at the encoding speed (seconds of video per second) before
/// Spindle has measured one on this computer.
fn default_speed(set: &EncodeSettings) -> f64 {
    let (n, d) = set.video.fps();
    let fps_rate = n as f64 / d as f64;
    let pixels = {
        let (w, h) = set.video.size();
        (w * h) as f64 / (1920.0 * 1080.0)
    };
    let fps = if set.effective_encoder().is_hardware() {
        250.0
    } else {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get()) as f64;
        let per_core = match set.quality {
            transcode::Quality::Fast => 6.0,
            transcode::Quality::Balanced => 1.8,
            // Slower preset, plus the first pass.
            transcode::Quality::Best => 0.9 / 1.3,
        };
        cores.min(32.0) * per_core
    };
    fps / pixels.max(0.1) / fps_rate
}

/// How long building `project` should take, in seconds, and whether that
/// is based on builds measured on this computer.
pub fn estimate_seconds(project: &Project) -> (f64, bool) {
    let cancel = AtomicBool::new(false);
    let emit = |_: BuildEvent| {};
    let b = Builder::new(project, Path::new("/"), &cancel, &emit);
    let (mut encode, mut other) = (0.0, 0.0);
    for (i, t) in project.titles.iter().enumerate() {
        let Ok(plan) = b.title_plan(t, b.title_clip(i)) else { continue };
        match &plan.encode {
            // Copying and muxing run at disk speed.
            None => other += plan.duration / 40.0,
            Some((_, cached)) if cached.exists() => other += plan.duration / 100.0,
            Some(_) => encode += plan.duration,
        }
    }
    for m in &b.menus {
        other += 3.0 + b.menu_duration(m) / 5.0;
        encode += m.intro.and_then(|a| project.asset(a)).map_or(0.0, |a| a.info.duration);
    }
    let measured = encode_cache::speed(&speed_key(&b.settings));
    let speed = measured.unwrap_or_else(|| default_speed(&b.settings));
    (encode / speed + other, measured.is_some())
}

/// Convenience wrapper used by the UI and the command line.
/// Whether `out` names a disc image rather than a folder.
pub fn is_image(out: &Path) -> bool {
    out.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso"))
}

/// Build the disc into a folder, or into a UDF image when `out` ends in
/// ".iso" (built in a temporary folder next to it, which is then removed).
pub fn build(project: &Project, out: &Path, cancel: &AtomicBool, emit: &(dyn Fn(BuildEvent) + Sync)) -> Result<PathBuf> {
    if !is_image(out) {
        let res = Builder::new(project, out, cancel, emit).run();
        // A failed or cancelled build leaves no work files behind.
        if res.is_err() {
            let _ = std::fs::remove_dir_all(out.join(".spindle-work"));
        }
        return res;
    }
    let folder = out.with_extension("spindle-tmp");
    let scaled = |ev: BuildEvent| match ev {
        BuildEvent::Progress(p) => emit(BuildEvent::Progress(p * 0.9)),
        ev => emit(ev),
    };
    let task = |state, detail: &str, fraction| emit(BuildEvent::Task { key: "image".into(), state, detail: detail.into(), fraction });
    emit(BuildEvent::AddTask { key: "image".into(), name: "Disc Image".into(), group: TaskGroup::Image });
    let result = (|| -> Result<()> {
        Builder::new(project, &folder, cancel, &scaled).run()?;
        emit(BuildEvent::Stage("Writing disc image".into()));
        task(TaskState::Running, "Writing", 0.0);
        crate::bluray::udf::write_image(&folder, out, &project.disc.name, |f| {
            emit(BuildEvent::Progress(0.9 + f * 0.1));
            task(TaskState::Running, "Writing", f);
            !cancel.load(Ordering::Relaxed)
        })?;
        let size = std::fs::metadata(out).map_or(0, |m| m.len());
        task(TaskState::Done, &format!("{:.2} GB", size as f64 / 1e9), 1.0);
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&folder);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result.map(|_| out.to_path_buf())
}
