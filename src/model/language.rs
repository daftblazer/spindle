// SPDX-License-Identifier: GPL-3.0-or-later

//! Language presets for a disc's Setup menu: each picks an audio language
//! and a subtitle style, applied to every title with that title's own
//! matching tracks.

use super::{new_id, AudioTrack, Id, Project, SubtitleTrack, Title};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SubtitleMode {
    #[default]
    Off,
    /// Only signs and songs (for viewers who understand the dialogue).
    SignsSongs,
    /// Full dialogue subtitles.
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LanguagePreset {
    pub id: Id,
    pub name: String,
    /// Audio language code ("eng").
    pub audio: String,
    pub subtitles: SubtitleMode,
    /// Subtitle language code.
    pub subtitle_lang: String,
}

/// A title's own choice for a preset, overriding the automatic match.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TrackPick {
    #[default]
    Auto,
    Off,
    Track(Id),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LanguageTracks {
    pub preset: Id,
    pub audio: TrackPick,
    pub subtitle: TrackPick,
}

/// Streams a preset selects in a title: audio track (None keeps the
/// current one) and subtitle track (None turns subtitles off).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub audio: Option<Id>,
    pub subtitle: Option<Id>,
}

/// Whether a subtitle track only covers signs and songs.
pub fn is_signs_track(t: &SubtitleTrack) -> bool {
    let n = t.name.to_lowercase();
    t.forced || ["sign", "song", "forced"].iter().any(|k| n.contains(k))
}

fn auto_audio<'a>(title: &'a Title, preset: &LanguagePreset) -> Option<&'a AudioTrack> {
    title.disc_audio().find(|a| a.lang == preset.audio)
}

fn auto_subtitle<'a>(title: &'a Title, preset: &LanguagePreset) -> Option<&'a SubtitleTrack> {
    let lang_ok = |s: &&SubtitleTrack| preset.subtitle_lang.is_empty() || preset.subtitle_lang == "und" || s.lang == preset.subtitle_lang;
    match preset.subtitles {
        SubtitleMode::Off => None,
        SubtitleMode::SignsSongs => title.disc_subtitles().filter(lang_ok).find(|s| is_signs_track(s)),
        SubtitleMode::Full => {
            let mut full = title.disc_subtitles().filter(lang_ok).filter(|s| !is_signs_track(s));
            // Prefer tracks named as dialogue/full over e.g. "SDH".
            let all: Vec<&SubtitleTrack> = full.by_ref().collect();
            all.iter()
                .find(|s| {
                    let n = s.name.to_lowercase();
                    n.contains("dialog") || n.contains("full")
                })
                .or(all.first())
                .copied()
        }
    }
}

impl Title {
    /// The tracks `preset` selects in this title.
    pub fn resolve_language(&self, preset: &LanguagePreset) -> Resolved {
        let pick = self.language_tracks.iter().find(|l| l.preset == preset.id).cloned().unwrap_or(LanguageTracks {
            preset: preset.id,
            audio: TrackPick::Auto,
            subtitle: TrackPick::Auto,
        });
        let audio = match pick.audio {
            TrackPick::Track(id) if self.disc_audio().any(|a| a.id == id) => Some(id),
            TrackPick::Off => None,
            _ => auto_audio(self, preset).map(|a| a.id),
        };
        let subtitle = match pick.subtitle {
            TrackPick::Track(id) if self.disc_subtitles().any(|s| s.id == id) => Some(id),
            TrackPick::Off => None,
            _ => auto_subtitle(self, preset).map(|s| s.id),
        };
        Resolved { audio, subtitle }
    }
}

impl Project {
    pub fn language(&self, id: Id) -> Option<&LanguagePreset> {
        self.disc.languages.iter().find(|l| l.id == id)
    }

    /// Preset in effect until the viewer chooses one.
    pub fn default_language(&self) -> Option<&LanguagePreset> {
        self.disc.default_language.and_then(|id| self.language(id)).or(self.disc.languages.first())
    }

    /// Remove a preset and the buttons' and titles' references to it.
    pub fn remove_language(&mut self, id: Id) {
        self.disc.languages.retain(|l| l.id != id);
        if self.disc.default_language == Some(id) {
            self.disc.default_language = None;
        }
        for t in &mut self.titles {
            t.language_tracks.retain(|l| l.preset != id);
        }
        for m in &mut self.menus {
            m.forget_target(id);
        }
    }

    /// Presets for the languages found in the titles: each audio language
    /// with its own signs & songs subtitles (or none), and other languages
    /// with full subtitles in the main subtitle language. English first.
    pub fn suggest_languages(&self) -> Vec<LanguagePreset> {
        let count = |langs: &mut Vec<(String, usize)>, l: &str| {
            if l == "und" || l.is_empty() {
                return;
            }
            match langs.iter_mut().find(|(x, _)| x == l) {
                Some((_, n)) => *n += 1,
                None => langs.push((l.to_string(), 1)),
            }
        };
        let (mut audio, mut subs) = (Vec::new(), Vec::new());
        for t in &self.titles {
            for a in t.disc_audio() {
                count(&mut audio, &a.lang);
            }
            for s in t.disc_subtitles().filter(|s| !is_signs_track(s)) {
                count(&mut subs, &s.lang);
            }
        }
        let english_first = |v: &mut Vec<(String, usize)>| v.sort_by_key(|(l, n)| (l != "eng", std::cmp::Reverse(*n)));
        english_first(&mut audio);
        english_first(&mut subs);
        let main_sub = subs.first().map(|(l, _)| l.clone());
        let has_signs = |lang: &str| self.titles.iter().any(|t| t.disc_subtitles().any(|s| s.lang == lang && is_signs_track(s)));
        let name = super::language_name;

        let mut out = Vec::new();
        for (lang, _) in &audio {
            let (subtitles, subtitle_lang) = match &main_sub {
                Some(s) if s != lang => (SubtitleMode::Full, s.clone()),
                _ if has_signs(lang) => (SubtitleMode::SignsSongs, lang.clone()),
                _ => (SubtitleMode::Off, lang.clone()),
            };
            let label = if subtitles == SubtitleMode::Full {
                gettextrs::gettext("{audio} · {subtitles} Subtitles").replace("{audio}", &name(lang)).replace("{subtitles}", &name(&subtitle_lang))
            } else {
                name(lang)
            };
            out.push(LanguagePreset { id: new_id(), name: label, audio: lang.clone(), subtitles, subtitle_lang });
        }
        // A single language with subtitles: offer them on and off.
        if out.len() == 1 {
            if let Some(s) = &main_sub {
                let lang = out[0].audio.clone();
                out.push(LanguagePreset {
                    id: new_id(),
                    name: gettextrs::gettext("{} · Subtitles").replace("{}", &name(&lang)),
                    audio: lang,
                    subtitles: SubtitleMode::Full,
                    subtitle_lang: s.clone(),
                });
            }
        }
        if out.len() < 2 {
            out.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::probe::MediaInfo;
    use crate::model::{Asset, AssetKind, AudioSource, SubtitleSource};

    fn sub(name: &str, lang: &str, forced: bool) -> SubtitleTrack {
        SubtitleTrack {
            id: new_id(),
            source: SubtitleSource::Embedded { index: 0 },
            codec: "ass".into(),
            lang: lang.into(),
            name: name.into(),
            enabled: true,
            forced,
        }
    }

    fn audio(lang: &str) -> AudioTrack {
        AudioTrack { id: new_id(), source: AudioSource::Embedded { index: 0 }, lang: lang.into(), name: lang.into(), enabled: true, layout: Default::default(), reencode: false }
    }

    fn anime() -> Project {
        let mut p = Project::default();
        let asset = new_id();
        p.assets.push(Asset { id: asset, path: "/tmp/a.mkv".into(), kind: AssetKind::Video, info: MediaInfo { duration: 60.0, ..Default::default() } });
        let t = p.ensure_title_for(asset).unwrap();
        let t = p.title_mut(t).unwrap();
        t.audio = vec![audio("jpn"), audio("eng")];
        t.subtitles = vec![sub("Signs@Reza", "eng", false), sub("Dialogue@Reza", "eng", false)];
        p
    }

    #[test]
    fn suggests_dub_and_sub() {
        let p = anime();
        let s = p.suggest_languages();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].audio.as_str(), s[0].subtitles), ("eng", SubtitleMode::SignsSongs));
        assert_eq!((s[1].audio.as_str(), s[1].subtitles, s[1].subtitle_lang.as_str()), ("jpn", SubtitleMode::Full, "eng"));
        assert_eq!(s[1].name, "Japanese · English Subtitles");
    }

    #[test]
    fn resolves_tracks_per_title() {
        let mut p = anime();
        let s = p.suggest_languages();
        let t = &p.titles[0];
        let en = t.resolve_language(&s[0]);
        assert_eq!(en.audio, Some(t.audio[1].id));
        assert_eq!(en.subtitle, Some(t.subtitles[0].id));
        let ja = t.resolve_language(&s[1]);
        assert_eq!(ja.audio, Some(t.audio[0].id));
        assert_eq!(ja.subtitle, Some(t.subtitles[1].id));
        // An override wins.
        let off = LanguageTracks { preset: s[1].id, audio: TrackPick::Auto, subtitle: TrackPick::Off };
        p.titles[0].language_tracks.push(off);
        assert_eq!(p.titles[0].resolve_language(&s[1]).subtitle, None);
    }
}
