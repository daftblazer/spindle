// SPDX-License-Identifier: GPL-3.0-or-later

//! Project document model (serialized as `.spindle` JSON).

mod audio;
mod language;
mod menu;
mod subtitle;
mod undo;

pub use audio::*;
pub use language::*;
pub use menu::*;
pub use subtitle::*;
pub use undo::UndoStack;

use crate::bluray::{AudioCodec, VideoFormat};
use crate::media::probe::MediaInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub type Id = Uuid;

pub fn new_id() -> Id {
    Uuid::new_v4()
}

pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscSettings {
    pub name: String,
    pub video: VideoFormat,
    pub audio: AudioCodec,
    /// Average video bitrate in kbit/s.
    pub video_bitrate: u32,
    /// AC-3 bitrate in kbit/s.
    pub audio_bitrate: u32,
    #[serde(default)]
    pub subtitle_style: SubtitleStyle,
    /// Choices for a Setup menu.
    #[serde(default)]
    pub languages: Vec<LanguagePreset>,
    /// Preset used until the viewer picks one (first when unset).
    #[serde(default)]
    pub default_language: Option<Id>,
    /// Pop-up menu of titles that don't choose their own.
    #[serde(default)]
    pub popup_menu: Option<Id>,
    /// Menu opened by the remote's Top Menu key (the first when unset).
    #[serde(default)]
    pub top_menu: Option<Id>,
    /// Video encoder; hardware ones are for test discs.
    #[serde(default)]
    pub encoder: crate::media::hwenc::VideoEncoder,
    #[serde(default)]
    pub quality: crate::media::transcode::Quality,
    /// Bring every encoded audio track to the same loudness (EBU R128).
    #[serde(default)]
    pub normalize_loudness: bool,
}

impl Default for DiscSettings {
    fn default() -> Self {
        DiscSettings {
            name: "SPINDLE".into(),
            video: VideoFormat::default(),
            audio: AudioCodec::default(),
            video_bitrate: 18_000,
            audio_bitrate: 448,
            subtitle_style: SubtitleStyle::default(),
            languages: Vec::new(),
            default_language: None,
            popup_menu: None,
            top_menu: None,
            encoder: Default::default(),
            quality: Default::default(),
            normalize_loudness: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AssetKind {
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Asset {
    pub id: Id,
    pub path: PathBuf,
    pub kind: AssetKind,
    pub info: MediaInfo,
}

impl Asset {
    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum EndAction {
    /// Return to the menu the title is linked from (first menu by default).
    #[default]
    ReturnToMenu,
    PlayNextTitle,
    Loop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Title {
    pub id: Id,
    pub name: String,
    pub asset: Id,
    /// Chapter start times in seconds. The first chapter always starts at 0
    /// and is implicit.
    pub chapters: Vec<f64>,
    pub end_action: EndAction,
    /// Menu to return to; `None` means the first menu.
    pub return_menu: Option<Id>,
    /// Language of the only audio track, from projects made before titles
    /// had several; moved into `audio` on load.
    #[serde(default, skip_serializing)]
    pub audio_lang: String,
    /// Audio tracks in disc order; the first one plays by default.
    #[serde(default)]
    pub audio: Vec<AudioTrack>,
    /// Tracks chosen by hand for language presets.
    #[serde(default)]
    pub language_tracks: Vec<LanguageTracks>,
    /// Pop-up menu shown during this title.
    #[serde(default)]
    pub popup: PopupChoice,
    /// Frame (seconds) used as this title's thumbnail; `None` picks one
    /// automatically.
    #[serde(default)]
    pub poster: Option<f64>,
    #[serde(default)]
    pub subtitles: Vec<SubtitleTrack>,
    /// Subtitle track shown when the title starts (viewers can still switch).
    #[serde(default)]
    pub default_subtitle: Option<Id>,
    /// Put the original video on the disc instead of re-encoding it.
    #[serde(default)]
    pub keep_video: bool,
    /// Last compatibility check of the video for `keep_video`.
    #[serde(default)]
    pub video_check: Option<crate::media::compat::Report>,
    /// Deinterlacing, cropping and fitting of the picture.
    #[serde(default)]
    pub video: crate::media::picture::VideoOptions,
}

impl Title {
    /// Audio tracks that go on the disc, in stream order.
    pub fn disc_audio(&self) -> impl Iterator<Item = &AudioTrack> {
        self.audio.iter().filter(|t| t.enabled).take(MAX_AUDIO_TRACKS)
    }

    /// Tracks that go on the disc as subtitle streams, in stream order.
    pub fn disc_subtitles(&self) -> impl Iterator<Item = &SubtitleTrack> {
        self.subtitles.iter().filter(|t| t.enabled && !self.is_burned(t) && t.kind() != SubtitleKind::Unsupported).take(32)
    }

    /// The track drawn into the picture, if any (not for kept video,
    /// which isn't re-encoded).
    pub fn burned_subtitle(&self) -> Option<&SubtitleTrack> {
        self.subtitles.iter().find(|t| self.is_burned(t))
    }

    fn is_burned(&self, t: &SubtitleTrack) -> bool {
        t.enabled && t.burn_in && t.can_burn_in() && !self.keep_video && self.subtitles.iter().find(|s| s.enabled && s.burn_in && s.can_burn_in()).is_some_and(|s| s.id == t.id)
    }
}

/// Match the tracks of one title to another's: embedded tracks by stream
/// number, separate files by `same`. Returns, for each track of `from`, the
/// index of its match in `to`.
fn match_tracks<T>(from: &[T], to: &[T], embedded: impl Fn(&T) -> Option<usize>, same: impl Fn(&T, &T) -> bool) -> Vec<Option<usize>> {
    let mut used = vec![false; to.len()];
    from.iter()
        .map(|f| {
            let found = match embedded(f) {
                Some(i) => to.iter().position(|t| embedded(t) == Some(i)),
                None => to.iter().enumerate().position(|(k, t)| !used[k] && embedded(t).is_none() && same(f, t)),
            };
            if let Some(k) = found.filter(|k| !used[*k]) {
                used[k] = true;
                Some(k)
            } else {
                None
            }
        })
        .collect()
}

/// Reorder `to` so matched tracks follow `from`'s order (the others stay
/// after them), returning the new order as indices.
fn follow_order(matches: &[Option<usize>], len: usize) -> Vec<usize> {
    let matched: Vec<usize> = matches.iter().flatten().copied().collect();
    let rest = (0..len).filter(|k| !matched.contains(k));
    matched.iter().copied().chain(rest).collect()
}

impl Project {
    /// Give every other title the audio track choices of title `from`
    /// (on/off, order, language, name, channels, re-encoding). Returns how
    /// many titles changed.
    pub fn apply_audio_to_all(&mut self, from: Id) -> usize {
        let Some(src) = self.title(from).map(|t| t.audio.clone()) else { return 0 };
        let embedded = |a: &AudioTrack| match a.source {
            AudioSource::Embedded { index } => Some(index),
            AudioSource::External { .. } => None,
        };
        let mut changed = 0;
        for t in self.titles.iter_mut().filter(|t| t.id != from) {
            let matches = match_tracks(&src, &t.audio, embedded, |a, b| a.lang == b.lang);
            if matches.iter().all(Option::is_none) {
                continue;
            }
            let before = t.audio.clone();
            for (s, m) in src.iter().zip(&matches) {
                if let Some(a) = m.and_then(|k| t.audio.get_mut(k)) {
                    a.enabled = s.enabled;
                    a.lang = s.lang.clone();
                    a.name = s.name.clone();
                    a.layout = s.layout;
                    a.reencode = s.reencode;
                }
            }
            let order = follow_order(&matches, t.audio.len());
            t.audio = order.into_iter().map(|k| t.audio[k].clone()).collect();
            if t.audio != before {
                changed += 1;
            }
        }
        changed
    }

    /// Give every other title the subtitle choices of title `from` (on/off,
    /// order, language, name, forced, burn-in and the default track).
    pub fn apply_subtitles_to_all(&mut self, from: Id) -> usize {
        let Some((src, default)) = self.title(from).map(|t| (t.subtitles.clone(), t.default_subtitle)) else { return 0 };
        let embedded = |s: &SubtitleTrack| match s.source {
            SubtitleSource::Embedded { index } => Some(index),
            SubtitleSource::External { .. } => None,
        };
        let mut changed = 0;
        for t in self.titles.iter_mut().filter(|t| t.id != from) {
            let matches = match_tracks(&src, &t.subtitles, embedded, |a, b| a.lang == b.lang && a.codec == b.codec && a.forced == b.forced);
            if matches.iter().all(Option::is_none) {
                continue;
            }
            let before = (t.subtitles.clone(), t.default_subtitle);
            let mut new_default = None;
            for (s, m) in src.iter().zip(&matches) {
                if let Some(sub) = m.and_then(|k| t.subtitles.get_mut(k)) {
                    sub.enabled = s.enabled;
                    sub.lang = s.lang.clone();
                    sub.name = s.name.clone();
                    sub.forced = s.forced;
                    sub.burn_in = s.burn_in;
                    if default == Some(s.id) {
                        new_default = Some(sub.id);
                    }
                }
            }
            t.default_subtitle = new_default;
            let order = follow_order(&matches, t.subtitles.len());
            t.subtitles = order.into_iter().map(|k| t.subtitles[k].clone()).collect();
            if (t.subtitles.clone(), t.default_subtitle) != before {
                changed += 1;
            }
        }
        changed
    }
}

/// Which pop-up menu a title shows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PopupChoice {
    /// The disc's default pop-up menu (Disc Settings).
    #[default]
    Default,
    None,
    Menu(Id),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FirstPlay {
    FirstMenu,
    FirstTitle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub version: u32,
    pub disc: DiscSettings,
    pub assets: Vec<Asset>,
    pub titles: Vec<Title>,
    pub menus: Vec<Menu>,
    pub first_play: FirstPlay,
    /// Media paths relative to the project file, written on save so a
    /// project folder can be moved or copied to another machine.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub relative_paths: std::collections::BTreeMap<Id, PathBuf>,
}

/// `path` relative to `base`, when they share more than the root.
fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let (p, b): (Vec<_>, Vec<_>) = (path.components().collect(), base.components().collect());
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common < 2 || !path.is_absolute() || p[..common].iter().any(|c| !matches!(c, Component::RootDir | Component::Normal(_))) {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    Some(out)
}

impl Default for Project {
    fn default() -> Self {
        Project {
            version: PROJECT_VERSION,
            disc: DiscSettings::default(),
            assets: Vec::new(),
            titles: Vec::new(),
            menus: vec![Menu::new("Main Menu")],
            first_play: FirstPlay::FirstMenu,
            relative_paths: Default::default(),
        }
    }
}

impl Project {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let mut p: Project =
            serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display()))?;
        p.migrate();
        let reprobe = p.assets.iter().any(|a| a.kind != AssetKind::Image && a.info.probe_version < crate::media::probe::PROBE_VERSION);
        // Resolve asset paths relative to the project file.
        if let Some(dir) = path.parent() {
            for a in &mut p.assets {
                if a.path.is_relative() {
                    a.path = dir.join(&a.path);
                }
            }
            // The project was moved: find media at the same place relative to it.
            let rel = std::mem::take(&mut p.relative_paths);
            for (id, old) in p.media_paths() {
                if old.exists() {
                    continue;
                }
                if let Some(r) = rel.get(&id) {
                    let moved = dir.join(r);
                    if moved.exists() {
                        p.relink(id, moved.canonicalize().unwrap_or(moved));
                    }
                }
            }
        }
        if reprobe {
            p.reprobe();
        }
        Ok(p)
    }

    /// Probe media again when it was probed by an older version (which
    /// didn't record everything the build uses now).
    fn reprobe(&mut self) {
        let old: Vec<(usize, PathBuf)> = self
            .assets
            .iter()
            .enumerate()
            .filter(|(_, a)| a.kind != AssetKind::Image && a.info.probe_version < crate::media::probe::PROBE_VERSION && a.path.exists())
            .map(|(i, a)| (i, a.path.clone()))
            .collect();
        let mut found: Vec<(usize, MediaInfo)> = Vec::new();
        for batch in old.chunks(8) {
            std::thread::scope(|s| {
                let handles: Vec<_> = batch.iter().map(|(i, path)| s.spawn(move || crate::media::probe::probe(path).ok().map(|info| (*i, info)))).collect();
                found.extend(handles.into_iter().filter_map(|h| h.join().ok().flatten()));
            });
        }
        for (i, info) in found {
            self.assets[i].info = info;
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut out = self.clone();
        let dir = path.parent().map(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()));
        out.relative_paths = dir
            .map(|dir| self.media_paths().into_iter().filter_map(|(id, p)| Some((id, relative_to(&p, &dir)?))).collect())
            .unwrap_or_default();
        let data = serde_json::to_vec_pretty(&out)?;
        let tmp = path.with_extension("spindle~");
        std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Bring projects saved by older versions up to date.
    fn migrate(&mut self) {
        let infos: std::collections::HashMap<Id, MediaInfo> = self.assets.iter().map(|a| (a.id, a.info.clone())).collect();
        for t in &mut self.titles {
            let lang = std::mem::take(&mut t.audio_lang);
            if !t.audio.is_empty() {
                continue;
            }
            // Only the first stream was used before.
            if let Some(mut track) = infos.get(&t.asset).and_then(|i| embedded_tracks_audio(i).into_iter().find(|a| a.source == AudioSource::Embedded { index: 0 })) {
                if !lang.is_empty() {
                    track.lang = normalize_lang(&lang);
                }
                t.audio.push(track);
            }
        }
    }

    /// Every file the project refers to: (asset or subtitle id, path).
    pub fn media_paths(&self) -> Vec<(Id, PathBuf)> {
        let mut out: Vec<(Id, PathBuf)> = self.assets.iter().map(|a| (a.id, a.path.clone())).collect();
        for t in &self.titles {
            for s in &t.subtitles {
                if let SubtitleSource::External { path } = &s.source {
                    out.push((s.id, path.clone()));
                }
            }
        }
        out
    }

    /// Change the file of an asset or external subtitle track.
    pub fn relink(&mut self, id: Id, path: PathBuf) {
        if let Some(a) = self.assets.iter_mut().find(|a| a.id == id) {
            a.path = path;
            return;
        }
        for t in &mut self.titles {
            for s in &mut t.subtitles {
                if s.id == id {
                    s.source = SubtitleSource::External { path };
                    return;
                }
            }
        }
    }

    /// Paths that don't exist on this system.
    pub fn missing_media(&self) -> Vec<(Id, PathBuf)> {
        self.media_paths().into_iter().filter(|(_, p)| !p.exists()).collect()
    }

    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    pub fn title(&self, id: Id) -> Option<&Title> {
        self.titles.iter().find(|t| t.id == id)
    }

    pub fn title_mut(&mut self, id: Id) -> Option<&mut Title> {
        self.titles.iter_mut().find(|t| t.id == id)
    }

    /// Menus that are screens of their own (not pop-ups), in disc order.
    pub fn disc_menus(&self) -> impl Iterator<Item = &Menu> {
        self.menus.iter().filter(|m| !m.popup)
    }

    /// The top menu: first non-pop-up menu.
    pub fn first_menu(&self) -> Option<&Menu> {
        self.disc_menus().next()
    }

    pub fn popup_menus(&self) -> impl Iterator<Item = &Menu> {
        self.menus.iter().filter(|m| m.popup)
    }

    /// Pop-up menu shown during `title`.
    pub fn title_popup(&self, title: &Title) -> Option<&Menu> {
        let id = match title.popup {
            PopupChoice::Default => self.disc.popup_menu?,
            PopupChoice::None => return None,
            PopupChoice::Menu(id) => id,
        };
        self.menu(id).filter(|m| m.popup)
    }

    /// Pop-up pages reachable from `menu` (itself first).
    pub fn popup_pages<'a>(&'a self, menu: &'a Menu) -> Vec<&'a Menu> {
        let mut pages = vec![menu];
        let mut i = 0;
        while i < pages.len() {
            for b in pages[i].buttons().filter_map(|b| b.button()) {
                if let Action::ShowMenu(id) | Action::SetLanguage { menu: Some(id), .. } = b.action {
                    if let Some(m) = self.menu(id).filter(|m| m.popup) {
                        if !pages.iter().any(|p| p.id == m.id) {
                            pages.push(m);
                        }
                    }
                }
            }
            i += 1;
        }
        pages
    }

    pub fn menu(&self, id: Id) -> Option<&Menu> {
        self.menus.iter().find(|m| m.id == id)
    }

    pub fn menu_mut(&mut self, id: Id) -> Option<&mut Menu> {
        self.menus.iter_mut().find(|m| m.id == id)
    }

    /// Add a title for a video asset (or return the existing one).
    pub fn ensure_title_for(&mut self, asset: Id) -> Option<Id> {
        if let Some(t) = self.titles.iter().find(|t| t.asset == asset) {
            return Some(t.id);
        }
        let a = self.asset(asset)?;
        if a.kind != AssetKind::Video {
            return None;
        }
        let name = a.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let t = Title {
            id: new_id(),
            name,
            asset,
            chapters: auto_chapters(a.info.duration, 300.0),
            end_action: EndAction::default(),
            return_menu: None,
            audio_lang: String::new(),
            audio: embedded_tracks_audio(&a.info),
            language_tracks: Vec::new(),
            popup: PopupChoice::default(),
            poster: None,
            subtitles: {
                let mut subs = embedded_tracks(&a.info);
                subs.extend(sidecar_files(&a.path).iter().map(|f| external_track(f, Some(&a.path))));
                subs
            },
            default_subtitle: None,
            keep_video: false,
            video_check: None,
            video: Default::default(),
        };
        let id = t.id;
        self.titles.push(t);
        Some(id)
    }

    /// Thumbnail frame of a title: the chosen one, else 10% in (at most 10 s,
    /// skipping black leaders of short clips).
    pub fn title_poster(&self, title: Id) -> f64 {
        let Some(t) = self.title(title) else { return 0.0 };
        let d = self.asset(t.asset).map_or(0.0, |a| a.info.duration);
        match t.poster {
            Some(p) => p.clamp(0.0, (d - 0.1).max(0.0)),
            None => (d * 0.1).min(10.0).min((d - 0.5).max(0.0)),
        }
    }

    /// Choose a title's thumbnail frame and use it everywhere the title is
    /// shown: buttons playing it from the start and still images of it.
    pub fn set_title_poster(&mut self, title: Id, time: f64) {
        let Some(t) = self.title_mut(title) else { return };
        t.poster = Some(time);
        let asset = t.asset;
        for m in &mut self.menus {
            for it in &mut m.items {
                match &mut it.kind {
                    ItemKind::Button(b) if b.thumbnail == Some(asset) && b.action == (Action::PlayTitle { title, chapter: 0 }) => {
                        b.thumbnail_time = time;
                    }
                    ItemKind::Image(img) if img.asset == asset => img.time = time,
                    _ => {}
                }
            }
        }
    }

    /// Remove an asset along with titles using it and references to them.
    pub fn remove_asset(&mut self, asset: Id) {
        let titles: Vec<Id> = self.titles.iter().filter(|t| t.asset == asset).map(|t| t.id).collect();
        for t in titles {
            self.remove_title(t);
        }
        self.assets.retain(|a| a.id != asset);
        for m in &mut self.menus {
            m.forget_asset(asset);
        }
        for t in &mut self.titles {
            t.audio.retain(|a| !matches!(a.source, AudioSource::External { asset: x, .. } if x == asset));
        }
    }

    pub fn remove_title(&mut self, title: Id) {
        self.titles.retain(|t| t.id != title);
        for m in &mut self.menus {
            m.forget_target(title);
        }
    }

    /// Copy a menu (with new ids) right after it; returns the copy's id.
    pub fn duplicate_menu(&mut self, menu: Id) -> Option<Id> {
        let i = self.menus.iter().position(|m| m.id == menu)?;
        let mut copy = self.menus[i].clone();
        copy.id = new_id();
        copy.name = format!("{} {}", copy.name, gettextrs::gettext("(Copy)"));
        let map: std::collections::HashMap<Id, Id> = copy.items.iter().map(|it| (it.id, new_id())).collect();
        let remap = |id: &mut Option<Id>| {
            if let Some(x) = id.as_mut() {
                if let Some(n) = map.get(x) {
                    *x = *n;
                }
            }
        };
        for it in &mut copy.items {
            it.id = map[&it.id];
            if let Some(b) = it.button_mut() {
                remap(&mut b.nav.up);
                remap(&mut b.nav.down);
                remap(&mut b.nav.left);
                remap(&mut b.nav.right);
                // Links to the menu itself stay within the copy.
                if b.action == Action::ShowMenu(menu) {
                    b.action = Action::ShowMenu(copy.id);
                }
            }
        }
        remap(&mut copy.default_button);
        let id = copy.id;
        self.menus.insert(i + 1, copy);
        Some(id)
    }

    /// Give buttons the look (text, colors, highlight, fill) of `from`.
    /// `scope` limits it to one menu.
    pub fn apply_button_look(&mut self, from: &ButtonItem, scope: Option<Id>) {
        for m in self.menus.iter_mut().filter(|m| scope.is_none_or(|s| s == m.id)) {
            for it in &mut m.items {
                if let Some(b) = it.button_mut() {
                    b.text = from.text.clone();
                    b.selected_color = from.selected_color;
                    b.activated_color = from.activated_color;
                    b.highlight = from.highlight;
                    b.highlight_text = from.highlight_text;
                    b.fill = from.fill;
                }
            }
        }
    }

    pub fn remove_menu(&mut self, menu: Id) {
        self.menus.retain(|m| m.id != menu);
        for m in &mut self.menus {
            m.forget_target(menu);
        }
        for t in &mut self.titles {
            if t.return_menu == Some(menu) {
                t.return_menu = None;
            }
            if t.popup == PopupChoice::Menu(menu) {
                t.popup = PopupChoice::Default;
            }
        }
        if self.disc.popup_menu == Some(menu) {
            self.disc.popup_menu = None;
        }
        if self.disc.top_menu == Some(menu) {
            self.disc.top_menu = None;
        }
    }
}

/// Evenly spaced chapter points (excluding 0) every `interval` seconds.
pub fn auto_chapters(duration: f64, interval: f64) -> Vec<f64> {
    let mut v = Vec::new();
    let mut t = interval;
    while t < duration - interval / 4.0 {
        v.push(t);
        t += interval;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_roundtrip() {
        let mut p = Project::default();
        let m = &mut p.menus[0];
        m.items.push(MenuItem::new_button("Play", Action::None, Rect::new(100.0, 100.0, 300.0, 60.0)));
        m.items.push(MenuItem::new_text("Hello", Rect::new(100.0, 20.0, 600.0, 60.0)));
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn duplicate_menu_remaps_items() {
        let mut p = Project::default();
        let menu = p.menus[0].id;
        let a = MenuItem::new_button("A", Action::ShowMenu(menu), Rect::new(100.0, 100.0, 300.0, 60.0));
        let mut b = MenuItem::new_button("B", Action::None, Rect::new(100.0, 200.0, 300.0, 60.0));
        b.button_mut().unwrap().nav.up = Some(a.id);
        p.menus[0].default_button = Some(b.id);
        p.menus[0].items.extend([a, b]);
        let copy = p.duplicate_menu(menu).unwrap();
        let (orig, dup) = (p.menu(menu).unwrap(), p.menu(copy).unwrap());
        assert_eq!(p.menus[1].id, copy);
        assert!(dup.items.iter().all(|i| orig.item(i.id).is_none()));
        assert_eq!(dup.default_button, Some(dup.items[1].id));
        assert_eq!(dup.items[1].button().unwrap().nav.up, Some(dup.items[0].id));
        assert_eq!(dup.items[0].button().unwrap().action, Action::ShowMenu(copy));

        let mut look = dup.items[0].button().unwrap().clone();
        look.highlight = Highlight::Fill;
        p.apply_button_look(&look, Some(menu));
        assert!(p.menu(menu).unwrap().buttons().all(|b| b.button().unwrap().highlight == Highlight::Fill));
        assert!(p.menu(copy).unwrap().buttons().all(|b| b.button().unwrap().highlight != Highlight::Fill));
    }

    #[test]
    fn track_choices_apply_to_all_titles() {
        use crate::media::probe::{AudioStream, SubtitleStream};
        let mut p = Project::default();
        let mut titles = Vec::new();
        for k in 0..3 {
            let asset = new_id();
            let audio = |i: usize, lang: &str| AudioStream { index: i, codec: "flac".into(), channels: 2, sample_rate: 48000, lang: Some(lang.into()), ..Default::default() };
            let sub = |i: usize, lang: &str| SubtitleStream { index: i, codec: "ass".into(), lang: Some(lang.into()), ..Default::default() };
            let info = MediaInfo {
                duration: 10.0,
                video_codec: Some("h264".into()),
                audio_codec: Some("flac".into()),
                audio_streams: vec![audio(0, "jpn"), audio(1, "eng")],
                // The last title has one subtitle stream fewer.
                subtitles: if k < 2 { vec![sub(0, "eng"), sub(1, "eng")] } else { vec![sub(0, "eng")] },
                ..Default::default()
            };
            p.assets.push(Asset { id: asset, path: format!("/tmp/ep{k}.mkv").into(), kind: AssetKind::Video, info });
            titles.push(p.ensure_title_for(asset).unwrap());
        }
        {
            let t = p.title_mut(titles[0]).unwrap();
            t.audio.swap(0, 1);
            t.audio[1].enabled = false;
            t.audio[0].layout = ChannelLayout::Surround;
            t.subtitles[0].enabled = false;
            t.subtitles[1].burn_in = true;
            t.default_subtitle = Some(t.subtitles[1].id);
        }
        assert_eq!(p.apply_audio_to_all(titles[0]), 2);
        assert_eq!(p.apply_subtitles_to_all(titles[0]), 2);
        let t1 = p.title(titles[1]).unwrap();
        assert_eq!(t1.audio[0].source, AudioSource::Embedded { index: 1 });
        assert_eq!((t1.audio[0].layout, t1.audio[1].enabled), (ChannelLayout::Surround, false));
        assert!(!t1.subtitles[0].enabled && t1.subtitles[1].burn_in);
        assert_eq!(t1.default_subtitle, Some(t1.subtitles[1].id));
        // A missing stream is skipped; the rest still apply.
        let t2 = p.title(titles[2]).unwrap();
        assert!(!t2.subtitles[0].enabled);
        assert_eq!(t2.default_subtitle, None);
        // Nothing left to change.
        assert_eq!(p.apply_audio_to_all(titles[0]), 0);
    }

    #[test]
    fn old_audio_language_becomes_track() {
        let mut p = Project::default();
        let asset = new_id();
        let info = MediaInfo { duration: 10.0, video_codec: Some("h264".into()), audio_codec: Some("aac".into()), audio_channels: 2, ..Default::default() };
        p.assets.push(Asset { id: asset, path: "/tmp/x.mkv".into(), kind: AssetKind::Video, info });
        let t = p.ensure_title_for(asset).unwrap();
        let mut json: serde_json::Value = serde_json::to_value(&p).unwrap();
        let title = &mut json["titles"][0];
        title.as_object_mut().unwrap().remove("audio");
        title["audio_lang"] = "fra".into();
        let path = std::env::temp_dir().join(format!("spindle-old-{}.spindle", new_id()));
        std::fs::write(&path, json.to_string()).unwrap();
        let back = Project::load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let audio = &back.title(t).unwrap().audio;
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].lang, "fra");
        assert_eq!(audio[0].source, AudioSource::Embedded { index: 0 });
    }

    #[test]
    fn moved_project_finds_media() {
        let root = std::env::temp_dir().join(format!("spindle-move-{}", new_id()));
        let old = root.join("old");
        std::fs::create_dir_all(old.join("media")).unwrap();
        std::fs::write(old.join("media/ep.mkv"), b"x").unwrap();
        let mut p = Project::default();
        let id = new_id();
        p.assets.push(Asset { id, path: old.join("media/ep.mkv"), kind: AssetKind::Video, info: MediaInfo::default() });
        p.save(&old.join("show.spindle")).unwrap();
        let new = root.join("new");
        std::fs::rename(&old, &new).unwrap();
        let back = Project::load(&new.join("show.spindle")).unwrap();
        assert_eq!(back.assets[0].path, new.canonicalize().unwrap().join("media/ep.mkv"));
        assert!(back.relative_paths.is_empty());
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(relative_to(Path::new("/a/b/c.mkv"), Path::new("/a/d")), Some(PathBuf::from("../b/c.mkv")));
        assert_eq!(relative_to(Path::new("/x/c.mkv"), Path::new("/a/d")), None);
    }

    #[test]
    fn poster_propagates_to_buttons() {
        let mut p = Project::default();
        let asset = new_id();
        p.assets.push(Asset {
            id: asset,
            path: "/tmp/x.mkv".into(),
            kind: AssetKind::Video,
            info: MediaInfo { duration: 100.0, video_codec: Some("h264".into()), ..Default::default() },
        });
        let t = p.ensure_title_for(asset).unwrap();
        assert_eq!(p.title_poster(t), 10.0);
        let mut b = MenuItem::new_button("x", Action::PlayTitle { title: t, chapter: 0 }, Rect::new(0.0, 0.0, 10.0, 10.0));
        b.button_mut().unwrap().thumbnail = Some(asset);
        let mut chapter = b.clone();
        chapter.button_mut().unwrap().action = Action::PlayTitle { title: t, chapter: 2 };
        p.menus[0].items.extend([b, chapter]);
        p.set_title_poster(t, 42.0);
        assert_eq!(p.title_poster(t), 42.0);
        assert_eq!(p.menus[0].items[0].button().unwrap().thumbnail_time, 42.0);
        // Chapter buttons keep their own frame.
        assert_ne!(p.menus[0].items[1].button().unwrap().thumbnail_time, 42.0);
    }

    #[test]
    fn chapters_are_spaced() {
        assert_eq!(auto_chapters(1000.0, 300.0), vec![300.0, 600.0, 900.0]);
        assert!(auto_chapters(100.0, 300.0).is_empty());
    }
}
