// SPDX-License-Identifier: GPL-3.0-or-later

//! A button that plays a menu's music the way the disc will: the chosen
//! part of the file, over and over, until it's pressed again.

use crate::document::Document;
use crate::model::Id;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// File and (start, length) in seconds of what `menu` plays.
fn music(doc: &Document, menu: Id) -> Option<(PathBuf, f64, f64)> {
    let p = doc.project();
    let m = p.menu(menu)?;
    let (start, length) = p.menu_music(m)?;
    Some((p.asset(m.audio?)?.path.clone(), start, length))
}

fn show(button: &gtk::Button, playing: bool) {
    button.set_icon_name(if playing { "media-playback-stop-symbolic" } else { "media-playback-start-symbolic" });
    button.set_tooltip_text(Some(&if playing { gettext("Stop") } else { gettext("Preview the Loop") }));
}

pub fn button(doc: &Rc<Document>, menu: Id) -> gtk::Button {
    let button = gtk::Button::builder().valign(gtk::Align::Center).css_classes(["flat"]).build();
    show(&button, false);
    let playing: Rc<RefCell<Option<gtk::MediaFile>>> = Rc::default();
    let stop = {
        let playing = playing.clone();
        move |b: &gtk::Button| {
            if let Some(m) = playing.borrow_mut().take() {
                m.pause();
                show(b, false);
            }
        }
    };
    // The inspector is rebuilt when something else gets selected.
    button.connect_unrealize(stop.clone());
    let doc = doc.clone();
    button.connect_clicked(move |b| {
        if playing.borrow().is_some() {
            stop(b);
            return;
        }
        let Some((path, start, _)) = music(&doc, menu) else { return };
        let media = gtk::MediaFile::for_file(&gio::File::for_path(path));
        // For a section that runs to the end of the file.
        media.set_loop(true);
        let to_start = move |m: &gtk::MediaFile, start: f64| {
            if m.is_seekable() {
                m.seek((start * 1e6) as i64);
            }
        };
        // Start at the section rather than with a moment of the beginning.
        media.connect_prepared_notify(move |m| {
            if m.is_prepared() {
                if start > 0.0 {
                    to_start(m, start);
                }
                m.play();
            }
        });
        // Follows the section as it's changed in the inspector.
        let d = doc.clone();
        media.connect_timestamp_notify(move |m| {
            let Some((_, start, length)) = music(&d, menu) else { return };
            let at = m.timestamp() as f64 / 1e6;
            #[cfg(debug_assertions)]
            if std::env::var("SPINDLE_SCREENSHOT_MUSIC").is_ok() {
                eprintln!("music at {at:.1}");
            }
            if !m.is_seeking() && (at >= start + length || at < start - 0.5) {
                to_start(m, start);
            }
        });
        let (weak, p) = (b.downgrade(), Rc::downgrade(&playing));
        media.connect_error_notify(move |m| {
            if m.error().is_some() {
                if let (Some(b), Some(p)) = (weak.upgrade(), p.upgrade()) {
                    p.borrow_mut().take();
                    show(&b, false);
                }
            }
        });
        *playing.borrow_mut() = Some(media);
        show(b, true);
    });
    // Development aid: start playing by itself and print the position.
    #[cfg(debug_assertions)]
    if std::env::var("SPINDLE_SCREENSHOT_MUSIC").is_ok() {
        let b = button.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || b.emit_clicked());
    }
    button
}
