// SPDX-License-Identifier: GPL-3.0-or-later

//! Project document model (serialized as `.spindle` JSON).

mod menu;
mod subtitle;
mod undo;

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
    pub audio_lang: String,
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
}

impl Title {
    /// Tracks that go on the disc, in stream order.
    pub fn disc_subtitles(&self) -> impl Iterator<Item = &SubtitleTrack> {
        self.subtitles.iter().filter(|t| t.enabled && t.kind() != SubtitleKind::Unsupported).take(32)
    }
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
        }
    }
}

impl Project {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let mut p: Project =
            serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display()))?;
        // Resolve asset paths relative to the project file.
        if let Some(dir) = path.parent() {
            for a in &mut p.assets {
                if a.path.is_relative() {
                    a.path = dir.join(&a.path);
                }
            }
        }
        Ok(p)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("spindle~");
        std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
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
            audio_lang: a.info.audio_lang.clone().unwrap_or_else(|| "eng".into()),
            poster: None,
            subtitles: {
                let mut subs = embedded_tracks(&a.info);
                subs.extend(sidecar_files(&a.path).iter().map(|f| external_track(f, Some(&a.path))));
                subs
            },
            default_subtitle: None,
            keep_video: false,
            video_check: None,
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
    }

    pub fn remove_title(&mut self, title: Id) {
        self.titles.retain(|t| t.id != title);
        for m in &mut self.menus {
            m.forget_target(title);
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
