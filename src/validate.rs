// SPDX-License-Identifier: GPL-3.0-or-later

//! Checks a project before building: problems that stop the build
//! (errors) and ones that make a poor disc (warnings), plus disc size
//! estimates.

use crate::bluray::AudioCodec;
use crate::model::{language_name, Action, AudioSource, EndAction, FirstPlay, Id, Project, SubtitleSource, MAX_AUDIO_TRACKS, SAFE_AREA};
use gettextrs::gettext;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

/// What an issue is about, so the UI can take the user there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Menu(Id),
    Item { menu: Id, item: Id },
    Title(Id),
    Settings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
    pub target: Option<Target>,
}

/// Recordable Blu-ray media, by usable size in bytes.
pub const DISCS: [(&str, f64); 3] = [("BD-25", 25.0e9), ("BD-50", 50.0e9), ("BD-100", 100.0e9)];

/// Headroom kept free for file system, navigation files and estimate error.
const MARGIN: f64 = 0.95;

/// Smallest disc the project fits on.
pub fn disc_for(bytes: f64) -> Option<(&'static str, f64)> {
    DISCS.iter().copied().find(|(_, cap)| bytes <= cap * MARGIN)
}

/// Bitrate on disc in kbit/s of an audio stream encoded to the disc's
/// format with `channels` channels.
fn audio_kbps(p: &Project, channels: u8) -> f64 {
    let ch = (channels as f64).max(2.0);
    match p.disc.audio {
        // 48 kHz; 16-bit samples, stored in 24-bit words for more than two channels.
        AudioCodec::Lpcm => 48.0 * if ch > 2.0 { 24.0 } else { 16.0 } * ch,
        AudioCodec::Lpcm24 => 48.0 * 24.0 * ch,
        _ => p.disc.audio_bitrate as f64,
    }
}

/// Bitrate on disc in kbit/s of a copied stream, from the source's when
/// known (plus the AC-3 core of TrueHD).
fn copied_kbps(codec: AudioCodec, source: Option<u64>) -> f64 {
    let typical = match codec {
        AudioCodec::Ac3 => 640.0,
        AudioCodec::Dts => 1509.0,
        AudioCodec::DtsHdHra => 3000.0,
        AudioCodec::DtsHdMa | AudioCodec::TrueHd => 4000.0,
        AudioCodec::Eac3 => 1536.0,
        _ => 2304.0,
    };
    let core = if codec == AudioCodec::TrueHd { 640.0 } else { 0.0 };
    source.map_or(typical, |b| b as f64 / 1000.0) + core
}

/// Transport overhead (packet headers, PSI, subtitles) in kbit/s.
const OVERHEAD_KBPS: f64 = 400.0;

/// Size of each title on disc in bytes: `(re-encoded seconds, fixed bytes)`.
/// Fixed bytes cover kept video, audio and overhead; re-encoded seconds are
/// multiplied by the video bitrate.
fn size_parts(p: &Project) -> (f64, f64) {
    let (mut secs, mut fixed) = (0.0, 0.0);
    for t in &p.titles {
        let Some(a) = p.asset(t.asset) else { continue };
        let d = a.info.duration;
        let own = a.info.audio();
        let (mut audio, mut extra_audio) = (0.0, 0.0);
        let set = crate::media::transcode::EncodeSettings::for_disc(&p.disc);
        for (input, _) in crate::build::title_audio(p, t, &set, 0.0).unwrap_or_default() {
            let source = if input.file.is_some() { None } else { own.iter().find(|s| s.index == input.index).and_then(|s| s.bit_rate) };
            let kbps = match input.copy {
                Some(codec) => copied_kbps(codec, source),
                None => audio_kbps(p, input.output_channels()),
            };
            if input.file.is_some() {
                extra_audio += kbps;
            } else {
                audio += kbps;
            }
        }
        match std::fs::metadata(&a.path) {
            // Kept video (with its own audio) is about the size of the source file.
            Ok(m) if t.keep_video => fixed += m.len() as f64 * 1.03 + d * extra_audio * 1000.0 / 8.0,
            _ => {
                secs += d;
                fixed += d * (audio + extra_audio + OVERHEAD_KBPS) * 1000.0 / 8.0;
            }
        }
    }
    // Menus: short loops at a low bitrate.
    for m in &p.menus {
        fixed += m.duration.max(1.0) * 8000.0 * 1000.0 / 8.0;
    }
    (secs, fixed)
}

pub fn estimated_bytes(p: &Project) -> f64 {
    let (secs, fixed) = size_parts(p);
    fixed + secs * p.disc.video_bitrate as f64 * 1000.0 / 8.0
}

/// Video bitrate (kbit/s) that makes the project fill `capacity` bytes,
/// or `None` when nothing is re-encoded.
pub fn fit_bitrate(p: &Project, capacity: f64) -> Option<u32> {
    let (secs, fixed) = size_parts(p);
    if secs <= 0.0 {
        return None;
    }
    let kbps = (capacity * MARGIN - fixed) * 8.0 / 1000.0 / secs;
    Some(kbps.floor().max(0.0) as u32)
}

pub const MIN_VIDEO_KBPS: u32 = 4000;
pub const MAX_VIDEO_KBPS: u32 = 35000;

/// Menus and titles a viewer can get to from First Play.
fn reachable(p: &Project) -> (HashSet<Id>, HashSet<Id>) {
    let (mut menus, mut titles) = (HashSet::new(), HashSet::new());
    let mut todo_m: Vec<Id> = Vec::new();
    let mut todo_t: Vec<Id> = Vec::new();
    // The remote's Top Menu key always leads to the first menu.
    if let Some(m) = p.first_menu() {
        todo_m.push(m.id);
    }
    if p.first_play == FirstPlay::FirstTitle || p.first_menu().is_none() {
        todo_t.extend(p.titles.first().map(|t| t.id));
    }
    loop {
        if let Some(id) = todo_m.pop() {
            if !menus.insert(id) {
                continue;
            }
            let Some(m) = p.menu(id) else { continue };
            let timeout = m.timeout.map(|t| t.action);
            for action in m.buttons().filter_map(|i| i.button()).map(|b| &b.action).chain(timeout.as_ref()) {
                match action {
                    Action::ShowMenu(m) => todo_m.push(*m),
                    Action::PlayTitle { title, .. } => todo_t.push(*title),
                    Action::PlayAll => todo_t.extend(p.titles.iter().map(|t| t.id)),
                    Action::SetLanguage { menu, .. } => todo_m.extend(*menu),
                    Action::PlayChapter(_) | Action::None => {}
                }
            }
        } else if let Some(id) = todo_t.pop() {
            if !titles.insert(id) {
                continue;
            }
            let Some(i) = p.titles.iter().position(|t| t.id == id) else { continue };
            let t = &p.titles[i];
            // Its pop-up menu opens during playback.
            todo_m.extend(p.title_popup(t).map(|m| m.id));
            let next = p.titles.get(i + 1).map(|t| t.id);
            match t.end_action {
                EndAction::PlayNextTitle if next.is_some() => todo_t.extend(next),
                EndAction::Loop => {}
                _ => match t.return_menu.or(p.first_menu().map(|m| m.id)) {
                    Some(m) => todo_m.push(m),
                    None => todo_t.extend(next),
                },
            }
        } else {
            break;
        }
    }
    (menus, titles)
}

fn is_language_code(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_lowercase())
}

pub fn check(p: &Project) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut error = |message: String, target: Option<Target>| out.push(Issue { severity: Severity::Error, message, target });
    let mut issues_w = Vec::new();
    let mut warn = |message: String, target: Option<Target>| issues_w.push(Issue { severity: Severity::Warning, message, target });

    // --- things the build can't do
    if p.titles.is_empty() {
        error(gettext("Add at least one title (drag a video into the project)."), None);
    }
    for t in &p.titles {
        match p.asset(t.asset) {
            None => error(gettext("Title “{}” has no video.").replace("{}", &t.name), Some(Target::Title(t.id))),
            Some(a) if !a.path.exists() => error(
                gettext("The video of “{}” is missing: {}").replacen("{}", &t.name, 1).replacen("{}", &a.path.display().to_string(), 1),
                Some(Target::Title(t.id)),
            ),
            Some(_) if t.keep_video && t.video_check.as_ref().is_some_and(|r| !r.compatible()) => error(
                gettext("“{}” is set to keep its original video, but the video isn't Blu-ray compatible.").replace("{}", &t.name),
                Some(Target::Title(t.id)),
            ),
            _ => {}
        }
        for a in t.disc_audio() {
            if let AudioSource::External { asset, .. } = a.source {
                match p.asset(asset) {
                    Some(f) if f.path.exists() => {}
                    f => error(
                        gettext("The audio file of track “{}” of “{}” is missing: {}")
                            .replacen("{}", &a.name, 1)
                            .replacen("{}", &t.name, 1)
                            .replacen("{}", &f.map(|f| f.path.display().to_string()).unwrap_or_default(), 1),
                        Some(Target::Title(t.id)),
                    ),
                }
            }
        }
        for s in &t.subtitles {
            if let SubtitleSource::External { path } = &s.source {
                if s.enabled && !path.exists() {
                    error(
                        gettext("A subtitle file of “{}” is missing: {}").replacen("{}", &t.name, 1).replacen("{}", &path.display().to_string(), 1),
                        Some(Target::Title(t.id)),
                    );
                }
            }
        }
    }
    for m in &p.menus {
        for (id, what) in [
            (m.background.image, gettext("background image")),
            (m.background.video, gettext("background video")),
            (m.audio, gettext("music")),
            (m.intro, gettext("intro video")),
        ] {
            if let Some(a) = id.and_then(|id| p.asset(id)) {
                if !a.path.exists() {
                    error(
                        gettext("The {what} of menu “{menu}” is missing: {path}")
                            .replace("{what}", &what)
                            .replace("{menu}", &m.name)
                            .replace("{path}", &a.path.display().to_string()),
                        Some(Target::Menu(m.id)),
                    );
                }
            }
        }
        for item in &m.items {
            if let crate::model::ItemKind::Image(img) = &item.kind {
                if p.asset(img.asset).is_some_and(|a| !a.path.exists()) {
                    error(
                        gettext("An image on menu “{}” is missing.").replace("{}", &m.name),
                        Some(Target::Item { menu: m.id, item: item.id }),
                    );
                }
            }
        }
    }
    if p.menus.len() > 255 {
        error(gettext("A disc can have at most 255 menus."), None);
    }
    let bytes = estimated_bytes(p);
    if disc_for(bytes).is_none() {
        error(
            gettext("The disc is about {} GB, more than fits on a BD-100. Lower the video bitrate or remove titles.")
                .replace("{}", &format!("{:.1}", bytes / 1e9)),
            Some(Target::Settings),
        );
    }

    // --- menus and navigation
    let (menus, titles) = reachable(p);
    for m in &p.menus {
        let n = m.buttons().count();
        if n > 255 {
            error(gettext("Menu “{}” has more than 255 buttons.").replace("{}", &m.name), Some(Target::Menu(m.id)));
        }
        if n == 0 && menus.contains(&m.id) {
            warn(gettext("Menu “{}” has no buttons, so viewers can't leave it.").replace("{}", &m.name), Some(Target::Menu(m.id)));
        }
        if !menus.contains(&m.id) {
            let msg = if m.popup { gettext("Pop-up menu “{}” isn't used by any title.") } else { gettext("No button leads to menu “{}”.") };
            warn(msg.replace("{}", &m.name), Some(Target::Menu(m.id)));
        }
        let buttons: Vec<_> = m.buttons().collect();
        for (i, item) in buttons.iter().enumerate() {
            let Some(b) = item.button() else { continue };
            let target = Some(Target::Item { menu: m.id, item: item.id });
            let name = if b.label.trim().is_empty() { gettext("Unnamed") } else { b.label.clone() };
            match &b.action {
                Action::None => warn(gettext("Button “{}” on “{}” does nothing.").replacen("{}", &name, 1).replacen("{}", &m.name, 1), target),
                Action::ShowMenu(id) if p.menu(*id).is_none() => {
                    error(gettext("Button “{}” on “{}” links to a deleted menu.").replacen("{}", &name, 1).replacen("{}", &m.name, 1), target)
                }
                Action::ShowMenu(id) if !m.popup && p.menu(*id).is_some_and(|x| x.popup) => error(
                    gettext("Button “{}” on “{}” opens a pop-up menu, which only appears during titles.").replacen("{}", &name, 1).replacen("{}", &m.name, 1),
                    target,
                ),
                Action::PlayChapter(_) if !m.popup => error(
                    gettext("Button “{}” on “{}” goes to a chapter, which only works in pop-up menus.").replacen("{}", &name, 1).replacen("{}", &m.name, 1),
                    target,
                ),
                Action::SetLanguage { preset, .. } if p.language(*preset).is_none() => {
                    error(gettext("Button “{}” on “{}” sets a deleted language.").replacen("{}", &name, 1).replacen("{}", &m.name, 1), target)
                }
                Action::PlayTitle { title, chapter } => match p.title(*title) {
                    None => error(
                        gettext("Button “{}” on “{}” links to a deleted title.").replacen("{}", &name, 1).replacen("{}", &m.name, 1),
                        target,
                    ),
                    Some(t) if *chapter as usize > t.chapters.len() => warn(
                        gettext("Button “{}” on “{}” plays a chapter that no longer exists; it will start at the beginning.")
                            .replacen("{}", &name, 1)
                            .replacen("{}", &m.name, 1),
                        target,
                    ),
                    _ => {}
                },
                _ => {}
            }
            let r = item.rect;
            let s = SAFE_AREA;
            if r.x < s.x - 0.5 || r.y < s.y - 0.5 || r.x + r.w > s.x + s.w + 0.5 || r.y + r.h > s.y + s.h + 0.5 {
                warn(
                    gettext("Button “{}” on “{}” is outside the safe area and may be cut off on some TVs.")
                        .replacen("{}", &name, 1)
                        .replacen("{}", &m.name, 1),
                    target,
                );
            }
            for other in &buttons[i + 1..] {
                let o = other.rect;
                let overlap = r.x < o.x + o.w && o.x < r.x + r.w && r.y < o.y + o.h && o.y < r.y + r.h;
                if overlap {
                    warn(
                        gettext("Buttons “{}” and “{}” on “{}” overlap; highlights will look wrong.")
                            .replacen("{}", &name, 1)
                            .replacen("{}", &other.button().map(|b| b.label.clone()).unwrap_or_default(), 1)
                            .replacen("{}", &m.name, 1),
                        target,
                    );
                }
            }
        }
    }

    // --- titles
    for t in &p.titles {
        let target = Some(Target::Title(t.id));
        if !titles.contains(&t.id) {
            warn(gettext("Title “{}” can't be reached from any menu.").replace("{}", &t.name), target);
        }
        for a in t.disc_audio() {
            if !is_language_code(&a.lang) {
                warn(
                    gettext("Audio track “{}” of “{}” has language “{}”; use a three-letter code such as eng.")
                        .replacen("{}", &a.name, 1)
                        .replacen("{}", &t.name, 1)
                        .replacen("{}", &a.lang, 1),
                    target,
                );
            }
        }
        if t.audio.iter().filter(|a| a.enabled).count() > MAX_AUDIO_TRACKS {
            warn(
                gettext("“{}” has more than {} audio tracks; only the first {} go on the disc.")
                    .replacen("{}", &t.name, 1)
                    .replace("{}", &MAX_AUDIO_TRACKS.to_string()),
                target,
            );
        }
        for l in &p.disc.languages {
            if t.resolve_language(l).audio.is_none() && t.disc_audio().next().is_some() {
                warn(
                    gettext("“{}” has no {} audio for the language choice “{}”; its audio stays as it is.")
                        .replacen("{}", &t.name, 1)
                        .replacen("{}", &language_name(&l.audio), 1)
                        .replacen("{}", &l.name, 1),
                    target,
                );
            }
        }
        if t.keep_video && t.video_check.is_none() {
            warn(gettext("“{}” keeps its original video but hasn't been checked for compatibility.").replace("{}", &t.name), target);
        }
        if !t.keep_video && p.asset(t.asset).is_some_and(|a| a.info.is_hdr()) {
            warn(
                gettext("“{}” is HDR video. Blu-ray is standard range, so it will be tone mapped: highlights and colours will look less intense.").replace("{}", &t.name),
                target,
            );
        }
    }

    // --- encoder
    let encoder = p.disc.encoder;
    if encoder.is_hardware() {
        if crate::media::hwenc::known_available().is_some_and(|a| !a.contains(&encoder)) {
            error(
                gettext("The {} video encoder doesn't work on this computer. Choose Software in Disc Settings.").replace("{}", &encoder.label()),
                Some(Target::Settings),
            );
        } else if p.disc.video.interlaced() {
            warn(gettext("The 1080i and SD formats can't be encoded in hardware; this disc will use Software encoding."), Some(Target::Settings));
        } else {
            warn(
                gettext("Hardware video encoding is on. It's faster, but the picture is noticeably worse than Software: use it for test discs only."),
                Some(Target::Settings),
            );
        }
    }

    // --- size
    if let Some((disc, _)) = disc_for(bytes) {
        if disc == "BD-100" {
            warn(
                gettext("The disc is about {} GB and needs a BD-100 (BD-XL), which many burners and players don't support.")
                    .replace("{}", &format!("{:.1}", bytes / 1e9)),
                Some(Target::Settings),
            );
        }
    }

    out.extend(issues_w);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::probe::MediaInfo;
    use crate::model::{new_id, Asset, AssetKind, MenuItem, Rect};

    fn project() -> (Project, Id) {
        let mut p = Project::default();
        let asset = new_id();
        p.assets.push(Asset {
            id: asset,
            path: std::env::current_exe().unwrap(),
            kind: AssetKind::Video,
            info: MediaInfo { duration: 3600.0, video_codec: Some("h264".into()), ..Default::default() },
        });
        let t = p.ensure_title_for(asset).unwrap();
        (p, t)
    }

    #[test]
    fn unlinked_title_and_empty_menu() {
        let (p, _) = project();
        let issues = check(&p);
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
        assert!(issues.iter().any(|i| i.message.contains("can't be reached")));
        assert!(issues.iter().any(|i| i.message.contains("no buttons")));
    }

    #[test]
    fn linked_title_is_clean() {
        let (mut p, t) = project();
        p.menus[0].items.push(MenuItem::new_button("Play", Action::PlayTitle { title: t, chapter: 0 }, Rect::new(200.0, 200.0, 300.0, 60.0)));
        p.menus.push(crate::model::Menu::new("Orphan"));
        let issues = check(&p);
        let msgs: Vec<_> = issues.iter().map(|i| i.message.as_str()).collect();
        assert_eq!(msgs, ["No button leads to menu “Orphan”."]);
    }

    #[test]
    fn fit_bitrate_fills_disc() {
        let (p, _) = project();
        let kbps = fit_bitrate(&p, 25.0e9).unwrap();
        let mut q = p.clone();
        q.disc.video_bitrate = kbps;
        let bytes = estimated_bytes(&q);
        assert!(bytes <= 25.0e9 * MARGIN && bytes > 25.0e9 * MARGIN * 0.99, "{bytes}");
    }
}
