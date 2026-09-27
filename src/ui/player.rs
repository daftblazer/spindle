// SPDX-License-Identifier: GPL-3.0-or-later

//! Video player with controls laid out like GNOME's video player: transport
//! buttons in the centre, title/volume, a progress bar and times at the
//! bottom. Controls hide while playing when the pointer is idle.

use super::rows::format_time;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

type SkipHandler = Rc<dyn Fn(f64)>;
type StepHandler = Rc<dyn Fn(i32)>;
type SeekHandler = Rc<dyn Fn(f64)>;
type Handler = Rc<dyn Fn()>;

pub struct Player {
    pub widget: gtk::Overlay,
    picture: gtk::Picture,
    media: RefCell<Option<gtk::MediaFile>>,
    controls: gtk::Revealer,
    play: gtk::Button,
    play_icon: gtk::Image,
    scale: gtk::Scale,
    time_now: gtk::Label,
    time_total: gtk::Label,
    title: gtk::Label,
    volume: gtk::Scale,
    volume_icon: gtk::Image,
    /// The user is dragging the progress bar.
    scrubbing: Cell<bool>,
    /// Pointer is over the bottom bar or transport.
    over_controls: Cell<bool>,
    hide_timer: RefCell<Option<glib::SourceId>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    on_skip: RefCell<Option<SkipHandler>>,
    on_chapter: RefCell<Option<StepHandler>>,
    on_seek: RefCell<Option<SeekHandler>>,
    on_error: RefCell<Option<Handler>>,
}

/// Elapsed/total time without tenths.
fn clock(secs: f64) -> String {
    let t = format_time(secs);
    t.split('.').next().unwrap_or_default().to_string()
}

fn icon_button(icon: &str, size: i32, tip: &str) -> gtk::Button {
    let img = gtk::Image::builder().icon_name(icon).pixel_size(size).build();
    let b = gtk::Button::builder().child(&img).tooltip_text(tip).css_classes(["flat", "circular"]).valign(gtk::Align::Center).build();
    b.update_property(&[gtk::accessible::Property::Label(tip)]);
    b
}

impl Player {
    pub fn new() -> Rc<Self> {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .build();
        let widget = gtk::Overlay::builder().child(&picture).css_classes(["player"]).build();

        // Centre: chapter, 10 s, play/pause, 10 s, chapter.
        let center = gtk::Box::builder()
            .spacing(18)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["player-center"])
            .build();
        let prev_chapter = icon_button("media-skip-backward-symbolic", 24, &gettext("Previous Chapter (Page Up)"));
        let back = icon_button("media-seek-backward-symbolic", 28, &gettext("Back 10 Seconds (J)"));
        let play_icon = gtk::Image::builder().icon_name("media-playback-start-symbolic").pixel_size(48).build();
        let play = gtk::Button::builder().child(&play_icon).css_classes(["flat", "circular", "play"]).build();
        play.update_property(&[gtk::accessible::Property::Label(&gettext("Play"))]);
        let forward = icon_button("media-seek-forward-symbolic", 28, &gettext("Forward 10 Seconds (L)"));
        let next_chapter = icon_button("media-skip-forward-symbolic", 24, &gettext("Next Chapter (Page Down)"));
        for b in [&prev_chapter, &back, &play, &forward, &next_chapter] {
            center.append(b);
        }

        // Bottom: title + volume, progress bar, times.
        let bottom = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .valign(gtk::Align::End)
            .css_classes(["player-bottom"])
            .build();
        let top_row = gtk::Box::builder().spacing(6).build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();
        top_row.append(&title);
        let volume_icon = gtk::Image::from_icon_name("audio-volume-high-symbolic");
        let volume = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, 1.0, 0.05);
        volume.set_inverted(true);
        volume.set_height_request(120);
        volume.set_value(1.0);
        let volume_button = gtk::MenuButton::builder()
            .child(&volume_icon)
            .popover(&gtk::Popover::builder().child(&volume).build())
            .tooltip_text(gettext("Volume"))
            .css_classes(["flat", "circular"])
            .direction(gtk::ArrowType::Up)
            .build();
        top_row.append(&volume_button);
        bottom.append(&top_row);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        scale.set_hexpand(true);
        scale.update_property(&[gtk::accessible::Property::Label(&gettext("Position"))]);
        bottom.append(&scale);
        let times = gtk::Box::builder().build();
        let time_now = gtk::Label::builder().label("0:00").xalign(0.0).hexpand(true).css_classes(["numeric", "caption-heading"]).build();
        let time_total = gtk::Label::builder().label("0:00").xalign(1.0).css_classes(["numeric", "caption-heading"]).build();
        times.append(&time_now);
        times.append(&time_total);
        bottom.append(&times);

        let layer = gtk::Overlay::builder().child(&gtk::Box::builder().can_target(false).build()).build();
        layer.add_overlay(&center);
        layer.add_overlay(&bottom);
        let controls = gtk::Revealer::builder()
            .child(&layer)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(250)
            .reveal_child(true)
            .build();
        widget.add_overlay(&controls);

        let player = Rc::new(Player {
            widget,
            picture,
            media: RefCell::new(None),
            controls,
            play,
            play_icon,
            scale,
            time_now,
            time_total,
            title,
            volume,
            volume_icon,
            scrubbing: Cell::new(false),
            over_controls: Cell::new(false),
            hide_timer: RefCell::new(None),
            handlers: RefCell::new(Vec::new()),
            on_skip: RefCell::new(None),
            on_chapter: RefCell::new(None),
            on_seek: RefCell::new(None),
            on_error: RefCell::new(None),
        });

        // Buttons
        let weak = Rc::downgrade(&player);
        player.play.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.toggle();
            }
        });
        for (button, delta) in [(&back, -10.0), (&forward, 10.0)] {
            let weak = Rc::downgrade(&player);
            button.connect_clicked(move |_| {
                let Some(p) = weak.upgrade() else { return };
                let f = p.on_skip.borrow().clone();
                if let Some(f) = f {
                    f(delta);
                }
            });
        }
        for (button, dir) in [(&prev_chapter, -1), (&next_chapter, 1)] {
            let weak = Rc::downgrade(&player);
            button.connect_clicked(move |_| {
                let Some(p) = weak.upgrade() else { return };
                let f = p.on_chapter.borrow().clone();
                if let Some(f) = f {
                    f(dir);
                }
            });
        }

        // Progress bar: only user changes seek.
        let weak = Rc::downgrade(&player);
        player.scale.connect_change_value(move |_, _, value| {
            if let Some(p) = weak.upgrade() {
                p.time_now.set_label(&clock(value));
                let f = p.on_seek.borrow().clone();
                if let Some(f) = f {
                    f(value);
                }
            }
            glib::Propagation::Proceed
        });
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&player);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(p) = weak.upgrade() {
                p.scrubbing.set(true);
            }
        });
        let weak = Rc::downgrade(&player);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(p) = weak.upgrade() {
                p.scrubbing.set(false);
            }
        });
        player.scale.add_controller(drag);

        // Volume
        let weak = Rc::downgrade(&player);
        player.volume.connect_value_changed(move |s| {
            let Some(p) = weak.upgrade() else { return };
            let v = s.value();
            if let Some(m) = p.media.borrow().as_ref() {
                m.set_volume(v);
                m.set_muted(v <= 0.001);
            }
            p.volume_icon.set_icon_name(Some(match v {
                v if v <= 0.001 => "audio-volume-muted-symbolic",
                v if v < 0.34 => "audio-volume-low-symbolic",
                v if v < 0.67 => "audio-volume-medium-symbolic",
                _ => "audio-volume-high-symbolic",
            }));
        });

        // Click the picture: focus + play/pause.
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&player);
        click.connect_released(move |g, n, _, _| {
            let Some(p) = weak.upgrade() else { return };
            p.picture.grab_focus();
            if n == 1 {
                p.toggle();
            }
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        player.picture.add_controller(click);

        // Auto-hide.
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&player);
        motion.connect_motion(move |_, _, _| {
            if let Some(p) = weak.upgrade() {
                p.show_controls();
            }
        });
        let weak = Rc::downgrade(&player);
        motion.connect_leave(move |_| {
            if let Some(p) = weak.upgrade() {
                if p.is_playing() {
                    p.set_controls_visible(false);
                }
            }
        });
        player.widget.add_controller(motion);
        for area in [&center, &bottom] {
            let m = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(&player);
            m.connect_enter(move |_, _, _| {
                if let Some(p) = weak.upgrade() {
                    p.over_controls.set(true);
                }
            });
            let weak = Rc::downgrade(&player);
            m.connect_leave(move |_| {
                if let Some(p) = weak.upgrade() {
                    p.over_controls.set(false);
                }
            });
            area.add_controller(m);
        }
        player
    }

    pub fn stream(&self) -> Option<gtk::MediaStream> {
        self.media.borrow().as_ref().map(|m| m.clone().upcast())
    }

    pub fn is_playing(&self) -> bool {
        self.media.borrow().as_ref().is_some_and(|m| m.is_playing())
    }

    pub fn toggle(&self) {
        if let Some(m) = self.media.borrow().as_ref() {
            m.set_playing(!m.is_playing());
        }
    }

    pub fn pause(&self) {
        if let Some(m) = self.media.borrow().as_ref() {
            m.pause();
        }
    }

    pub fn connect_skip(&self, f: impl Fn(f64) + 'static) {
        *self.on_skip.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_chapter(&self, f: impl Fn(i32) + 'static) {
        *self.on_chapter.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_seek(&self, f: impl Fn(f64) + 'static) {
        *self.on_seek.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_error(&self, f: impl Fn() + 'static) {
        *self.on_error.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_controls_visible(&self, visible: bool) {
        self.controls.set_reveal_child(visible);
        self.widget.set_cursor_from_name(if visible { None } else { Some("none") });
    }

    /// Show the controls; hide them again after a while if playing.
    pub fn show_controls(self: &Rc<Self>) {
        self.set_controls_visible(true);
        if let Some(id) = self.hide_timer.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
            let Some(p) = weak.upgrade() else { return };
            p.hide_timer.borrow_mut().take();
            if p.is_playing() && !p.over_controls.get() && !p.scrubbing.get() {
                p.set_controls_visible(false);
            }
        });
        *self.hide_timer.borrow_mut() = Some(id);
    }

    fn update_time(&self, m: &gtk::MediaFile) {
        let secs = m.timestamp() as f64 / 1e6;
        if !self.scrubbing.get() {
            self.scale.set_value(secs);
        }
        self.time_now.set_label(&clock(secs));
    }

    /// Load a file (or clear with `None`).
    pub fn set_title(&self, title: &str) {
        self.title.set_label(title);
    }

    pub fn set_file(self: &Rc<Self>, path: Option<&Path>) {
        if let Some(old) = self.media.borrow_mut().take() {
            old.pause();
            for h in self.handlers.borrow_mut().drain(..) {
                old.disconnect(h);
            }
        }
        self.scale.set_value(0.0);
        self.time_now.set_label("0:00");
        self.time_total.set_label("0:00");
        self.play_icon.set_icon_name(Some("media-playback-start-symbolic"));
        let Some(path) = path else {
            self.picture.set_paintable(gtk::gdk::Paintable::NONE);
            return;
        };
        let media = gtk::MediaFile::for_file(&gio::File::for_path(path));
        media.set_volume(self.volume.value());
        self.picture.set_paintable(Some(&media));
        let mut handlers = Vec::new();
        let weak = Rc::downgrade(self);
        handlers.push(media.connect_timestamp_notify(move |m| {
            if let Some(p) = weak.upgrade() {
                p.update_time(m);
            }
        }));
        let weak = Rc::downgrade(self);
        handlers.push(media.connect_duration_notify(move |m| {
            let Some(p) = weak.upgrade() else { return };
            let d = m.duration() as f64 / 1e6;
            p.scale.set_range(0.0, d.max(0.1));
            p.time_total.set_label(&clock(d));
        }));
        let weak = Rc::downgrade(self);
        handlers.push(media.connect_playing_notify(move |m| {
            let Some(p) = weak.upgrade() else { return };
            let playing = m.is_playing();
            p.play_icon.set_icon_name(Some(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" }));
            p.play.update_property(&[gtk::accessible::Property::Label(&if playing { gettext("Pause") } else { gettext("Play") })]);
            p.play.set_tooltip_text(Some(&if playing { gettext("Pause (Space)") } else { gettext("Play (Space)") }));
            if playing {
                p.show_controls();
            } else {
                p.set_controls_visible(true);
            }
        }));
        let weak = Rc::downgrade(self);
        handlers.push(media.connect_ended_notify(move |m| {
            if let Some(p) = weak.upgrade() {
                if m.is_ended() {
                    p.set_controls_visible(true);
                }
            }
        }));
        let weak = Rc::downgrade(self);
        handlers.push(media.connect_error_notify(move |m| {
            let Some(p) = weak.upgrade() else { return };
            if m.error().is_some() {
                let f = p.on_error.borrow().clone();
                if let Some(f) = f {
                    f();
                }
            }
        }));
        *self.handlers.borrow_mut() = handlers;
        *self.media.borrow_mut() = Some(media);
        self.set_controls_visible(true);
    }
}
