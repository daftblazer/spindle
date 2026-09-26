// SPDX-License-Identifier: GPL-3.0-or-later

//! Property rows bound to the document: each row reads an initial value
//! and applies edits through a closure on the project.

use crate::document::{Change, Document};
use crate::model::{Project, Rgba};
use adw::prelude::*;
use gtk::{gdk, pango};
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    /// Set while rows are refreshed programmatically.
    static UPDATING: Cell<bool> = const { Cell::new(false) };
}

pub fn entry(doc: &Rc<Document>, title: &str, value: &str, change: Change, apply: impl Fn(&mut Project, String) + 'static) -> adw::EntryRow {
    let row = adw::EntryRow::builder().title(title).text(value).build();
    // One undo step per focus session rather than per keystroke.
    let focus = gtk::EventControllerFocus::new();
    let d = doc.clone();
    focus.connect_enter(move |_| d.checkpoint());
    row.add_controller(focus);
    let d = doc.clone();
    row.connect_changed(move |r| {
        let text = r.text().to_string();
        d.edit_silent(change, |p| apply(p, text));
    });
    row
}

#[allow(clippy::too_many_arguments)]
pub fn spin(
    doc: &Rc<Document>,
    title: &str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    digits: u32,
    change: Change,
    apply: impl Fn(&mut Project, f64) + 'static,
) -> adw::SpinRow {
    let adj = gtk::Adjustment::new(value, min, max, step, step * 10.0, 0.0);
    let row = adw::SpinRow::builder().title(title).adjustment(&adj).digits(digits).build();
    let d = doc.clone();
    row.connect_value_notify(move |r| {
        if UPDATING.get() {
            return;
        }
        let v = r.value();
        d.edit(change, |p| apply(p, v));
    });
    row
}

/// Update a spin row without triggering its edit handler.
pub fn set_spin_quietly(row: &adw::SpinRow, v: f64) {
    if (row.value() - v).abs() < 1e-6 {
        return;
    }
    UPDATING.set(true);
    row.set_value(v);
    UPDATING.set(false);
}

pub fn combo(
    doc: &Rc<Document>,
    title: &str,
    labels: &[String],
    selected: usize,
    change: Change,
    apply: impl Fn(&mut Project, usize) + 'static,
) -> adw::ComboRow {
    let strs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let model = gtk::StringList::new(&strs);
    let row = adw::ComboRow::builder().title(title).model(&model).selected(selected as u32).build();
    let d = doc.clone();
    row.connect_selected_notify(move |r| {
        let i = r.selected() as usize;
        d.edit(change, |p| apply(p, i));
    });
    row
}

pub fn switch(doc: &Rc<Document>, title: &str, value: bool, change: Change, apply: impl Fn(&mut Project, bool) + 'static) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).active(value).build();
    let d = doc.clone();
    row.connect_active_notify(move |r| {
        let v = r.is_active();
        d.edit(change, |p| apply(p, v));
    });
    row
}

pub fn to_gdk(c: Rgba) -> gdk::RGBA {
    gdk::RGBA::new(c.r, c.g, c.b, c.a)
}

pub fn from_gdk(c: &gdk::RGBA) -> Rgba {
    Rgba::new(c.red(), c.green(), c.blue(), c.alpha())
}

pub fn color(doc: &Rc<Document>, title: &str, value: Rgba, change: Change, apply: impl Fn(&mut Project, Rgba) + 'static) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let dialog = gtk::ColorDialog::builder().with_alpha(true).build();
    let button = gtk::ColorDialogButton::builder().dialog(&dialog).rgba(&to_gdk(value)).valign(gtk::Align::Center).build();
    let d = doc.clone();
    button.connect_rgba_notify(move |b| {
        let c = from_gdk(&b.rgba());
        d.edit(change, |p| apply(p, c));
    });
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    row
}

pub fn font(doc: &Rc<Document>, title: &str, value: &str, change: Change, apply: impl Fn(&mut Project, String) + 'static) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let dialog = gtk::FontDialog::new();
    let button = gtk::FontDialogButton::builder()
        .dialog(&dialog)
        .font_desc(&pango::FontDescription::from_string(value))
        .use_font(false)
        .use_size(true)
        .valign(gtk::Align::Center)
        .build();
    let d = doc.clone();
    button.connect_font_desc_notify(move |b| {
        if let Some(fd) = b.font_desc() {
            let s = fd.to_string();
            d.edit(change, |p| apply(p, s));
        }
    });
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    row
}

/// A row showing a video frame with a "Choose…" button opening the frame
/// picker. `apply` receives the chosen time.
pub fn frame(
    doc: &Rc<Document>,
    title: &str,
    asset: &crate::model::Asset,
    time: f64,
    marks: Vec<f64>,
    change: Change,
    apply: impl Fn(&mut Project, f64) + 'static,
) -> adw::ActionRow {
    use gtk::{gio, glib};
    let row = adw::ActionRow::builder().title(title).subtitle(format_time(time)).activatable(true).build();
    let pic = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(80)
        .height_request(45)
        .valign(gtk::Align::Center)
        .css_classes(["thumb-small"])
        .build();
    let clamp = adw::Clamp::builder().maximum_size(80).child(&pic).valign(gtk::Align::Center).build();
    pic.set_overflow(gtk::Overflow::Hidden);
    row.add_prefix(&clamp);
    let path = asset.path.clone();
    {
        let (pic, path) = (pic.clone(), path.clone());
        glib::spawn_future_local(async move {
            let file = gio::spawn_blocking(move || crate::media::thumbnail::frame(&path, time, 240).ok()).await.ok().flatten();
            if let Some(tex) = file.and_then(|f| gdk::Texture::from_filename(f).ok()) {
                pic.set_paintable(Some(&tex));
            }
        });
    }
    let button = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(gettextrs::gettext("Choose Frame…"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&button);
    let (duration, fps) = (asset.info.duration, asset.info.fps);
    let heading = title.to_string();
    let apply = Rc::new(apply);
    let open = {
        let doc = doc.clone();
        let row = row.clone();
        move || {
            let doc = doc.clone();
            let apply = apply.clone();
            super::frame_picker::present(&row, &heading, path.clone(), duration, fps, time, &marks, move |t| {
                doc.edit(change, |p| apply(p, t));
            });
        }
    };
    let open = Rc::new(open);
    let o = open.clone();
    button.connect_clicked(move |_| o());
    row.connect_activated(move |_| open());
    row
}

pub fn group(title: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).build()
}

pub fn format_time(secs: f64) -> String {
    let s = secs.max(0.0);
    let h = (s / 3600.0) as u64;
    let m = ((s % 3600.0) / 60.0) as u64;
    let sec = s % 60.0;
    if h > 0 {
        format!("{h}:{m:02}:{sec:04.1}")
    } else {
        format!("{m}:{sec:04.1}")
    }
}
