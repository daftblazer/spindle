// SPDX-License-Identifier: GPL-3.0-or-later

//! Dialog for choosing a frame of a video (thumbnails, stills). Frames are
//! extracted with ffmpeg, so this works even when GStreamer cannot play
//! the file.

use super::rows::format_time;
use crate::media::thumbnail;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

const FILMSTRIP: usize = 8;

struct Picker {
    path: PathBuf,
    duration: f64,
    frame_step: f64,
    picture: gtk::Picture,
    spinner: adw::Spinner,
    time_label: gtk::Label,
    scale: gtk::Scale,
    /// Increments with every request so stale frames are dropped.
    generation: Cell<u64>,
    queued: Cell<bool>,
}

impl Picker {
    fn time(&self) -> f64 {
        self.scale.value()
    }

    fn set_time(&self, t: f64) {
        self.scale.set_value(t.clamp(0.0, self.max_time()));
    }

    /// Current time snapped to a frame boundary.
    fn snapped(&self) -> f64 {
        ((self.time() / self.frame_step).round() * self.frame_step * 1000.0).round() / 1000.0
    }

    fn max_time(&self) -> f64 {
        (self.duration - 0.1).max(0.0)
    }

    /// Load the frame at the current time after a short debounce.
    fn queue_frame(self: &Rc<Self>) {
        self.time_label.set_label(&format!("{} / {}", format_time(self.time()), format_time(self.duration)));
        if self.queued.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            this.queued.set(false);
            this.load_frame();
        });
    }

    fn load_frame(self: &Rc<Self>) {
        let gen = self.generation.get() + 1;
        self.generation.set(gen);
        let t = self.snapped();
        let path = self.path.clone();
        self.spinner.set_visible(true);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let file = gio::spawn_blocking(move || thumbnail::frame(&path, t, 960).ok()).await.ok().flatten();
            if this.generation.get() != gen {
                return;
            }
            this.spinner.set_visible(false);
            if let Some(tex) = file.and_then(|f| gdk::Texture::from_filename(f).ok()) {
                this.picture.set_paintable(Some(&tex));
            }
        });
    }
}

/// Show the picker. `marks` are highlighted positions (chapter starts);
/// `on_pick` receives the chosen time in seconds.
#[allow(clippy::too_many_arguments)]
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    path: PathBuf,
    duration: f64,
    fps: Option<(u32, u32)>,
    initial: f64,
    marks: &[f64],
    on_pick: impl Fn(f64) + 'static,
) {
    let dialog = adw::Dialog::builder().title(heading).content_width(860).build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
    let cancel = gtk::Button::with_mnemonic(&gettext("_Cancel"));
    let choose = gtk::Button::builder().label(gettext("_Use This Frame")).use_underline(true).css_classes(["suggested-action"]).build();
    header.pack_start(&cancel);
    header.pack_end(&choose);
    toolbar.add_top_bar(&header);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(12)
        .margin_bottom(24)
        .build();

    let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).can_shrink(true).height_request(400).build();
    let spinner = adw::Spinner::builder().width_request(32).height_request(32).halign(gtk::Align::End).valign(gtk::Align::Start).margin_top(12).margin_end(12).build();
    let frame = gtk::Overlay::builder().child(&picture).css_classes(["frame-preview"]).build();
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.add_overlay(&spinner);
    body.append(&frame);

    let time_label = gtk::Label::builder().css_classes(["numeric", "title-4"]).build();
    body.append(&time_label);

    let max = (duration - 0.1).max(0.1);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, max, 0.04);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.update_property(&[gtk::accessible::Property::Label(&gettext("Position"))]);
    for m in marks.iter().filter(|m| **m > 0.0 && **m < max) {
        scale.add_mark(*m, gtk::PositionType::Bottom, None);
    }
    body.append(&scale);

    let frame_step = fps.map_or(0.04, |(n, d)| d as f64 / n as f64);
    let steps = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
    let step_buttons: Vec<(gtk::Button, f64)> = [
        ("−10 s", -10.0, gettext("Back 10 seconds")),
        ("−1 s", -1.0, gettext("Back 1 second")),
        ("‹", -frame_step, gettext("Previous frame")),
        ("›", frame_step, gettext("Next frame")),
        ("+1 s", 1.0, gettext("Forward 1 second")),
        ("+10 s", 10.0, gettext("Forward 10 seconds")),
    ]
    .into_iter()
    .map(|(label, delta, tip)| {
        let b = gtk::Button::builder().label(label).tooltip_text(tip).css_classes(["flat", "numeric"]).build();
        steps.append(&b);
        (b, delta)
    })
    .collect();
    body.append(&steps);

    // Filmstrip across the whole video for quick jumps.
    let strip = gtk::Box::builder().spacing(6).homogeneous(true).margin_top(6).build();
    body.append(&strip);

    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));

    let picker = Rc::new(Picker {
        path: path.clone(),
        duration,
        frame_step,
        picture,
        spinner,
        time_label,
        scale: scale.clone(),
        generation: Cell::new(0),
        queued: Cell::new(false),
    });
    picker.set_time(initial);
    picker.queue_frame();

    let p = picker.clone();
    scale.connect_value_changed(move |_| p.queue_frame());
    for (b, delta) in step_buttons {
        let p = picker.clone();
        b.connect_clicked(move |_| p.set_time(p.time() + delta));
    }

    for i in 0..FILMSTRIP {
        let t = duration * (i as f64 + 0.5) / FILMSTRIP as f64;
        let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).height_request(54).build();
        let b = gtk::Button::builder()
            .child(&pic)
            .tooltip_text(format_time(t))
            .css_classes(["flat", "filmstrip-frame"])
            .build();
        let p = picker.clone();
        b.connect_clicked(move |_| p.set_time(t));
        strip.append(&b);
        let path = path.clone();
        glib::spawn_future_local(async move {
            let file = gio::spawn_blocking(move || thumbnail::frame(&path, t, 240).ok()).await.ok().flatten();
            if let Some(tex) = file.and_then(|f| gdk::Texture::from_filename(f).ok()) {
                pic.set_paintable(Some(&tex));
            }
        });
    }

    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let d = dialog.clone();
    let p = picker.clone();
    choose.connect_clicked(move |_| {
        on_pick(p.snapped());
        d.close();
    });
    dialog.present(Some(parent));
}
