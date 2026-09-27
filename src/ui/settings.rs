// SPDX-License-Identifier: GPL-3.0-or-later

//! Disc settings dialog.

use super::rows;
use crate::bluray::{AudioCodec, VideoFormat};
use crate::document::{Change, Document};
use crate::model::FirstPlay;
use adw::prelude::*;
use gtk::glib;
use gettextrs::gettext;
use std::rc::Rc;

const AUDIO_BITRATES: [u32; 5] = [192, 256, 384, 448, 640];

pub fn present(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::PreferencesDialog::builder().title(gettext("Disc Settings")).build();
    let page = adw::PreferencesPage::new();
    let d = doc.project().disc.clone();
    let first_play = doc.project().first_play;

    let g = rows::group(&gettext("Disc"));
    g.add(&rows::entry(doc, &gettext("Name"), &d.name, Change::Content, |p, v| p.disc.name = v));
    let starts = [gettext("Show the First Menu"), gettext("Play the First Title")];
    g.add(&rows::combo(
        doc,
        &gettext("On Insert"),
        &starts,
        if first_play == FirstPlay::FirstMenu { 0 } else { 1 },
        Change::Content,
        |p, i| p.first_play = if i == 0 { FirstPlay::FirstMenu } else { FirstPlay::FirstTitle },
    ));
    page.add(&g);

    let g = rows::group(&gettext("Video"));
    g.set_description(Some(&gettext("All titles and menus are converted to this Blu-ray video format")));
    let formats: Vec<String> = VideoFormat::ALL.iter().map(|f| f.label().to_string()).collect();
    let sel = VideoFormat::ALL.iter().position(|f| *f == d.video).unwrap_or(0);
    g.add(&rows::combo(doc, &gettext("Format"), &formats, sel, Change::Content, |p, i| {
        p.disc.video = VideoFormat::ALL[i.min(VideoFormat::ALL.len() - 1)];
    }));
    let br = rows::spin(doc, &gettext("Average Bitrate"), d.video_bitrate as f64 / 1000.0, 4.0, 35.0, 1.0, 0, Change::Content, |p, v| {
        p.disc.video_bitrate = (v * 1000.0) as u32;
    });
    br.set_subtitle(&gettext("Mbit/s — about 18 fits two hours on a BD-25"));
    g.add(&br);
    page.add(&g);

    let g = rows::group(&gettext("Audio"));
    let codecs = [gettext("Dolby Digital (AC-3)"), gettext("Uncompressed PCM")];
    g.add(&rows::combo(
        doc,
        &gettext("Codec"),
        &codecs,
        if d.audio == AudioCodec::Ac3 { 0 } else { 1 },
        Change::Content,
        |p, i| p.disc.audio = if i == 0 { AudioCodec::Ac3 } else { AudioCodec::Lpcm },
    ));
    let labels: Vec<String> = AUDIO_BITRATES.iter().map(|b| format!("{b} kbit/s")).collect();
    let sel = AUDIO_BITRATES.iter().position(|b| *b == d.audio_bitrate).unwrap_or(3);
    g.add(&rows::combo(doc, &gettext("AC-3 Bitrate"), &labels, sel, Change::Content, |p, i| {
        p.disc.audio_bitrate = AUDIO_BITRATES[i.min(AUDIO_BITRATES.len() - 1)];
    }));
    page.add(&g);

    subtitle_group(doc, &page);
    languages_group(doc, &page);

    let g = rows::group(&gettext("Capacity"));
    let size_row = adw::ActionRow::builder().title(gettext("Estimated Size")).build();
    let bar = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).valign(gtk::Align::Center).width_request(140).build();
    bar.add_offset_value("full", 1.0);
    size_row.add_suffix(&bar);
    g.add(&size_row);
    page.add(&g);
    let update = {
        let (size_row, bar) = (size_row.downgrade(), bar.downgrade());
        let doc = Rc::downgrade(doc);
        move || {
            let (Some(row), Some(bar), Some(doc)) = (size_row.upgrade(), bar.upgrade(), doc.upgrade()) else { return };
            let bytes = crate::validate::estimated_bytes(&doc.project());
            let (disc, cap) = crate::validate::disc_for(bytes).unwrap_or(crate::validate::DISCS[2]);
            row.set_subtitle(&format!("{:.1} GB · {disc}", bytes / 1e9));
            bar.set_value((bytes / cap).min(1.0));
        }
    };
    update();
    doc.connect(move |c| {
        if c == Change::Content {
            update();
        }
    });

    dialog.add(&page);
    dialog.present(Some(parent));
}

/// Language presets for a Setup menu.
fn languages_group(doc: &Rc<Document>, page: &adw::PreferencesPage) {
    use crate::model::{language_name, LanguagePreset, SubtitleMode};
    let g = rows::group(&gettext("Languages"));
    g.set_description(Some(&gettext(
        "Choices for a Setup menu. Each sets the audio and subtitles of every title; the default applies until the viewer picks another.",
    )));
    page.add(&g);
    let rows_in: Rc<std::cell::RefCell<Vec<gtk::Widget>>> = Rc::default();
    let expanded: Rc<std::cell::RefCell<Vec<crate::model::Id>>> = Rc::default();

    let rebuild: Rc<dyn Fn()> = {
        let (doc, g) = (Rc::downgrade(doc), g.downgrade());
        let (rows_in, expanded) = (rows_in.clone(), expanded.clone());
        Rc::new(move || {
            let (Some(doc), Some(g)) = (doc.upgrade(), g.upgrade()) else { return };
            for w in rows_in.borrow_mut().drain(..) {
                g.remove(&w);
            }
            let add = |w: gtk::Widget| {
                g.add(&w);
                rows_in.borrow_mut().push(w);
            };
            let p = doc.project();
            // Languages to offer: those in the titles first, then common ones.
            let mut codes: Vec<String> = Vec::new();
            for t in &p.titles {
                for l in t.disc_audio().map(|a| a.lang.clone()).chain(t.disc_subtitles().map(|s| s.lang.clone())) {
                    if l != "und" && !codes.contains(&l) {
                        codes.push(l);
                    }
                }
            }
            for l in ["eng", "jpn", "fra", "deu", "spa", "ita", "por", "kor", "zho", "rus"] {
                if !codes.iter().any(|c| c == l) {
                    codes.push(l.into());
                }
            }
            let default = p.default_language().map(|l| l.id);
            for preset in &p.disc.languages {
                let pid = preset.id;
                let mut codes = codes.clone();
                for l in [&preset.audio, &preset.subtitle_lang] {
                    if !codes.contains(l) {
                        codes.push(l.clone());
                    }
                }
                let names: Vec<String> = codes.iter().map(|c| language_name(c)).collect();
                let modes = [gettext("Off"), gettext("Signs & Songs"), gettext("Full Subtitles")];
                let mode_of = |m: SubtitleMode| match m {
                    SubtitleMode::Off => 0,
                    SubtitleMode::SignsSongs => 1,
                    SubtitleMode::Full => 2,
                };
                let summary = match preset.subtitles {
                    SubtitleMode::Off => format!("{} {}", language_name(&preset.audio), gettext("audio · no subtitles")),
                    m => format!("{} {} · {} {}", language_name(&preset.audio), gettext("audio"), language_name(&preset.subtitle_lang), modes[mode_of(m)].to_lowercase()),
                };
                let mut title = glib::markup_escape_text(&preset.name).to_string();
                if default == Some(pid) {
                    title = format!("{title} · {}", gettext("Default"));
                }
                let row = adw::ExpanderRow::builder().title(title.as_str()).subtitle(glib::markup_escape_text(&summary).as_str()).build();
                row.set_expanded(expanded.borrow().contains(&pid));
                let exp = expanded.clone();
                row.connect_expanded_notify(move |r| {
                    let mut e = exp.borrow_mut();
                    e.retain(|x| *x != pid);
                    if r.is_expanded() {
                        e.push(pid);
                    }
                });
                fn preset_mut(p: &mut crate::model::Project, id: crate::model::Id) -> Option<&mut LanguagePreset> {
                    p.disc.languages.iter_mut().find(|l| l.id == id)
                }
                let name = rows::entry(&doc, &gettext("Name"), &preset.name, Change::Content, move |p, v| {
                    if let Some(l) = preset_mut(p, pid) {
                        l.name = v;
                    }
                });
                name.set_tooltip_text(Some(&gettext("The label of this choice's button")));
                row.add_row(&name);
                let c = codes.clone();
                row.add_row(&rows::combo(&doc, &gettext("Audio"), &names, codes.iter().position(|x| *x == preset.audio).unwrap_or(0), Change::Structure, move |p, i| {
                    if let (Some(l), Some(code)) = (preset_mut(p, pid), c.get(i)) {
                        l.audio = code.clone();
                    }
                }));
                row.add_row(&rows::combo(&doc, &gettext("Subtitles"), &modes, mode_of(preset.subtitles), Change::Structure, move |p, i| {
                    if let Some(l) = preset_mut(p, pid) {
                        l.subtitles = [SubtitleMode::Off, SubtitleMode::SignsSongs, SubtitleMode::Full][i.min(2)];
                    }
                }));
                if preset.subtitles != SubtitleMode::Off {
                    let c = codes.clone();
                    row.add_row(&rows::combo(&doc, &gettext("Subtitle Language"), &names, codes.iter().position(|x| *x == preset.subtitle_lang).unwrap_or(0), Change::Structure, move |p, i| {
                        if let (Some(l), Some(code)) = (preset_mut(p, pid), c.get(i)) {
                            l.subtitle_lang = code.clone();
                        }
                    }));
                }
                if default != Some(pid) {
                    let make_default = adw::ButtonRow::builder().title(gettext("Make Default")).build();
                    let d = doc.clone();
                    make_default.connect_activated(move |_| d.edit(Change::Structure, |p| p.disc.default_language = Some(pid)));
                    row.add_row(&make_default);
                }
                let remove = adw::ButtonRow::builder().title(gettext("Remove Language")).css_classes(["destructive-action"]).build();
                let d = doc.clone();
                remove.connect_activated(move |_| d.edit(Change::Structure, |p| p.remove_language(pid)));
                row.add_row(&remove);
                add(row.upcast());
            }

            if p.disc.languages.is_empty() {
                let suggested = p.suggest_languages();
                if !suggested.is_empty() {
                    let names: Vec<String> = suggested.iter().map(|l| l.name.clone()).collect();
                    let b = adw::ButtonRow::builder()
                        .title(gettext("Add from Tracks: {}").replace("{}", &names.join(", ")))
                        .start_icon_name("list-add-symbolic")
                        .build();
                    let d = doc.clone();
                    b.connect_activated(move |_| d.edit(Change::Structure, |p| p.disc.languages = suggested.clone()));
                    add(b.upcast());
                }
            }
            let b = adw::ButtonRow::builder().title(gettext("Add Language")).start_icon_name("list-add-symbolic").build();
            let d = doc.clone();
            b.connect_activated(move |_| {
                d.edit(Change::Structure, |p| {
                    let lang = p.titles.iter().flat_map(|t| t.disc_audio()).map(|a| a.lang.clone()).find(|l| l != "und").unwrap_or_else(|| "eng".into());
                    p.disc.languages.push(LanguagePreset {
                        id: crate::model::new_id(),
                        name: language_name(&lang),
                        audio: lang.clone(),
                        subtitles: SubtitleMode::Off,
                        subtitle_lang: lang,
                    });
                })
            });
            add(b.upcast());
        })
    };
    rebuild();
    let weak = Rc::downgrade(&rebuild);
    doc.connect(move |c| {
        if c == Change::Structure {
            if let Some(r) = weak.upgrade() {
                glib::idle_add_local_once(move || r());
            }
        }
    });
    // Keep the rebuild alive as long as the group.
    g.connect_destroy(move |_| {
        let _keep = &rebuild;
    });
}

/// Text subtitle style with a live preview.
fn subtitle_group(doc: &Rc<Document>, page: &adw::PreferencesPage) {
    use crate::model::SubtitleStyle;
    let st = doc.project().disc.subtitle_style.clone();
    let preview_group = rows::group(&gettext("Subtitle Style"));
    preview_group.set_description(Some(&gettext("For SRT, WebVTT and other plain text subtitles")));
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .height_request(290)
        .css_classes(["frame-preview"])
        .build();
    picture.set_overflow(gtk::Overflow::Hidden);
    preview_group.add(&picture);
    page.add(&preview_group);
    let g = adw::PreferencesGroup::new();

    fn edit(p: &mut crate::model::Project) -> &mut SubtitleStyle {
        &mut p.disc.subtitle_style
    }
    let font_row = rows::font(doc, &gettext("Font"), &format!("{} {}", st.font, st.size as i32), Change::Content, |p, f| {
        let fd = gtk::pango::FontDescription::from_string(&f);
        if let Some(family) = fd.family() {
            edit(p).font = family.to_string();
        }
    });
    g.add(&font_row);
    g.add(&rows::spin(doc, &gettext("Size"), st.size, 20.0, 120.0, 2.0, 0, Change::Content, |p, v| edit(p).size = v));
    g.add(&rows::switch(doc, &gettext("Bold"), st.bold, Change::Content, |p, v| edit(p).bold = v));
    g.add(&rows::color(doc, &gettext("Text Color"), st.color, Change::Content, |p, c| edit(p).color = c));
    g.add(&rows::color(doc, &gettext("Outline Color"), st.outline_color, Change::Content, |p, c| edit(p).outline_color = c));
    g.add(&rows::spin(doc, &gettext("Outline"), st.outline, 0.0, 12.0, 0.5, 1, Change::Content, |p, v| edit(p).outline = v));
    g.add(&rows::spin(doc, &gettext("Shadow"), st.shadow, 0.0, 10.0, 0.5, 1, Change::Content, |p, v| edit(p).shadow = v));
    g.add(&rows::spin(doc, &gettext("Distance From Bottom"), st.margin, 0.0, 400.0, 5.0, 0, Change::Content, |p, v| edit(p).margin = v));
    let restyle = rows::switch(doc, &gettext("Also Restyle ASS Subtitles"), st.restyle_ass, Change::Content, |p, v| edit(p).restyle_ass = v);
    restyle.set_subtitle(&gettext("Normally ASS/SSA subtitles keep their own fonts, colors and positions"));
    g.add(&restyle);
    page.add(&g);

    // Background: a frame of the first title, if any.
    let background = {
        let p = doc.project();
        p.titles.first().and_then(|t| p.asset(t.asset).map(|a| (a.path.clone(), p.title_poster(t.id))))
    };
    let render = {
        let picture = picture.downgrade();
        let doc = Rc::downgrade(doc);
        let background = background.clone();
        move || {
            let (Some(picture), Some(doc)) = (picture.upgrade(), doc.upgrade()) else { return };
            let style = doc.project().disc.subtitle_style.clone();
            let (w, h) = (960u32, 540u32);
            let Ok(mut surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w as i32, h as i32) else { return };
            {
                let Ok(cr) = gtk::cairo::Context::new(&surface) else { return };
                let bg = background
                    .as_ref()
                    .and_then(|(path, t)| crate::media::thumbnail::frame(path, *t, 960).ok())
                    .and_then(|f| crate::render::load_image(&f));
                match bg {
                    Some(img) => {
                        let s = (w as f64 / img.width() as f64).max(h as f64 / img.height() as f64);
                        cr.scale(s, s);
                        cr.set_source_surface(&img, 0.0, 0.0).ok();
                        cr.paint().ok();
                        cr.identity_matrix();
                    }
                    None => {
                        let g = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h as f64);
                        g.add_color_stop_rgb(0.0, 0.35, 0.45, 0.6);
                        g.add_color_stop_rgb(1.0, 0.1, 0.12, 0.18);
                        cr.set_source(&g).ok();
                        cr.paint().ok();
                    }
                }
                let sample = gettext("The quick brown fox\njumps over the lazy dog.");
                if let Ok(Some(img)) = crate::subtitles::ass::preview(&style, &sample, w, h) {
                    let b = &img.bitmap;
                    let mut data = Vec::with_capacity(b.rgba.len());
                    for px in b.rgba.as_chunks::<4>().0 {
                        let a = px[3] as u32;
                        let pm = |c: u8| (c as u32 * a / 255) as u8;
                        data.extend_from_slice(&[pm(px[2]), pm(px[1]), pm(px[0]), px[3]]);
                    }
                    if let Ok(src) = gtk::cairo::ImageSurface::create_for_data(
                        data,
                        gtk::cairo::Format::ARgb32,
                        b.width as i32,
                        b.height as i32,
                        b.width as i32 * 4,
                    ) {
                        cr.set_source_surface(&src, img.x as f64, img.y as f64).ok();
                        cr.paint().ok();
                    }
                }
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let Ok(data) = surface.data() else { return };
            let bytes = gtk::glib::Bytes::from(&data[..]);
            let tex = gtk::gdk::MemoryTexture::new(w as i32, h as i32, gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, stride);
            picture.set_paintable(Some(&tex));
        }
    };
    let render = Rc::new(render);
    render();
    let queued = Rc::new(std::cell::Cell::new(false));
    let r = render.clone();
    doc.connect(move |c| {
        if c != Change::Content || queued.replace(true) {
            return;
        }
        let (r, queued) = (r.clone(), queued.clone());
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            queued.set(false);
            r();
        });
    });
}
