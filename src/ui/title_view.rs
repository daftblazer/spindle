// SPDX-License-Identifier: GPL-3.0-or-later

//! Title page: preview the source video and place chapter points.

use super::rows::format_time;
use crate::document::{Change, Document, Node};
use crate::model::Id;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

pub struct TitleView {
    doc: Rc<Document>,
    video: gtk::Video,
    stack: gtk::Stack,
    poster: gtk::Picture,
    add: gtk::Button,
    chips: gtk::Box,
    current: Cell<Option<Id>>,
    current_path: RefCell<Option<PathBuf>>,
}

impl TitleView {
    pub fn new(doc: Rc<Document>, container: &gtk::Box) -> Rc<Self> {
        let video = gtk::Video::builder().vexpand(true).hexpand(true).autoplay(false).build();
        video.add_css_class("title-video");
        // Shown when the system cannot play the file (missing GStreamer codecs).
        let poster = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).vexpand(true).build();
        let note = gtk::Label::builder()
            .label(gettext("Preview unavailable: no GStreamer decoder for this video. Chapters can still be generated from the inspector."))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .margin_top(12)
            .margin_bottom(12)
            .build();
        let fallback = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["title-video"]).build();
        fallback.append(&poster);
        fallback.append(&note);
        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&video, Some("video"));
        stack.add_named(&fallback, Some("poster"));
        container.append(&stack);

        // Toolbar: compact buttons, never stretched by the chapter strip.
        let bar = gtk::Box::builder()
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .css_classes(["toolbar"])
            .build();
        let tool = |icon: &str, label: String, tip: String| {
            let b = gtk::Button::builder().valign(gtk::Align::Center).tooltip_text(tip).css_classes(["flat"]).build();
            b.set_child(Some(&adw::ButtonContent::builder().icon_name(icon).label(label).use_underline(true).build()));
            b
        };
        let add = tool(
            "bookmark-new-symbolic",
            gettext("Add _Chapter"),
            gettext("Add a chapter at the playhead"),
        );
        bar.append(&add);
        let thumb = tool(
            "image-x-generic-symbolic",
            gettext("_Thumbnail…"),
            gettext("Choose the frame used for this title in menus"),
        );
        bar.append(&thumb);
        bar.append(&gtk::Box::builder().hexpand(true).build());
        let preview = tool(
            "media-playback-start-symbolic",
            gettext("_Preview…"),
            gettext("Encode a short clip like the disc and play it (Ctrl+P)"),
        );
        preview.set_action_name(Some("win.preview-title"));
        preview.remove_css_class("flat");
        bar.append(&preview);
        container.append(&bar);

        // Chapter strip: one row, scrolls sideways when there are many.
        let strip = gtk::Box::builder().spacing(8).margin_start(12).margin_end(12).margin_bottom(8).build();
        strip.append(&gtk::Label::builder().label(gettext("Chapters")).css_classes(["caption-heading", "dim-label"]).valign(gtk::Align::Center).build());
        let chips = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).build();
        let chip_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .child(&chips)
            .build();
        // Mouse wheel scrolls the strip sideways.
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let adj = chip_scroller.hadjustment();
        wheel.connect_scroll(move |_, _, dy| {
            adj.set_value(adj.value() + dy * 40.0);
            glib::Propagation::Stop
        });
        chip_scroller.add_controller(wheel);
        strip.append(&chip_scroller);
        container.append(&strip);

        let tv = Rc::new(TitleView {
            doc: doc.clone(),
            video,
            stack,
            poster,
            add: add.clone(),
            chips,
            current: Cell::new(None),
            current_path: RefCell::new(None),
        });

        let weak = Rc::downgrade(&tv);
        add.connect_clicked(move |_| {
            if let Some(tv) = weak.upgrade() {
                tv.add_chapter();
            }
        });
        let weak = Rc::downgrade(&tv);
        thumb.connect_clicked(move |b| {
            if let Some(tv) = weak.upgrade() {
                tv.choose_thumbnail(b);
            }
        });

        let weak = Rc::downgrade(&tv);
        doc.connect(move |c| {
            let Some(tv) = weak.upgrade() else { return };
            if matches!(c, Change::Structure | Change::Selection) {
                tv.refresh();
            }
        });
        tv.refresh();
        tv
    }

    fn position(&self) -> Option<f64> {
        let stream = self.video.media_stream()?;
        Some(stream.timestamp() as f64 / 1e6)
    }

    fn add_chapter(&self) {
        let (Some(id), Some(pos)) = (self.current.get(), self.position()) else { return };
        if pos <= 0.5 {
            return;
        }
        self.doc.edit(Change::Structure, |p| {
            if let Some(t) = p.title_mut(id) {
                if !t.chapters.iter().any(|c| (c - pos).abs() < 0.5) {
                    t.chapters.push(pos);
                    t.chapters.sort_by(f64::total_cmp);
                }
            }
        });
    }

    /// Open the preview encode dialog, starting at the playhead.
    pub fn preview(&self, parent: &impl IsA<gtk::Widget>) {
        let Some(id) = self.current.get() else { return };
        let playing = self.stack.visible_child_name().as_deref() == Some("video");
        let start = self.position().filter(|t| *t > 0.05 && playing).unwrap_or(0.0);
        if let Some(s) = self.video.media_stream() {
            s.pause();
        }
        super::preview_dialog::present(&self.doc, id, start, parent);
    }

    pub fn choose_thumbnail(&self, parent: &impl IsA<gtk::Widget>) {
        let Some(id) = self.current.get() else { return };
        let (asset, poster, chapters) = {
            let p = self.doc.project();
            let Some(t) = p.title(id) else { return };
            let Some(a) = p.asset(t.asset) else { return };
            (a.clone(), p.title_poster(id), t.chapters.clone())
        };
        // Start at the playhead when the preview has been moved.
        let start = self.position().filter(|t| *t > 0.05 && self.stack.visible_child_name().as_deref() == Some("video")).unwrap_or(poster);
        if let Some(s) = self.video.media_stream() {
            s.pause();
        }
        let doc = self.doc.clone();
        let name = self.doc.project().title(id).map(|t| t.name.clone()).unwrap_or_default();
        super::frame_picker::present(
            parent,
            &format!("{} — {}", gettext("Thumbnail"), name),
            asset.path.clone(),
            asset.info.duration,
            asset.info.fps,
            start,
            &chapters,
            move |t| doc.edit(Change::Structure, |p| p.set_title_poster(id, t)),
        );
    }

    fn seek(&self, secs: f64) {
        if let Some(s) = self.video.media_stream() {
            s.seek((secs * 1e6) as i64);
        }
    }

    fn show_poster(self: &Rc<Self>) {
        self.stack.set_visible_child_name("poster");
        self.add.set_sensitive(false);
        let Some(path) = self.current_path.borrow().clone() else { return };
        let poster = self.poster.clone();
        glib::spawn_future_local(async move {
            let frame = gio::spawn_blocking(move || crate::media::thumbnail::frame(&path, 1.0, 960).ok()).await.ok().flatten();
            if let Some(tex) = frame.and_then(|f| gtk::gdk::Texture::from_filename(f).ok()) {
                poster.set_paintable(Some(&tex));
            }
        });
    }

    fn refresh(self: &Rc<Self>) {
        let Node::Title(id) = self.doc.node() else {
            if let Some(s) = self.video.media_stream() {
                s.pause();
            }
            self.current.set(None);
            return;
        };
        let (path, chapters) = {
            let p = self.doc.project();
            let Some(t) = p.title(id) else { return };
            (p.asset(t.asset).map(|a| a.path.clone()), t.chapters.clone())
        };
        self.current.set(Some(id));
        if *self.current_path.borrow() != path {
            self.stack.set_visible_child_name("video");
            self.add.set_sensitive(true);
            self.video.set_file(path.as_ref().map(gio::File::for_path).as_ref());
            if let Some(stream) = self.video.media_stream() {
                let weak = Rc::downgrade(self);
                stream.connect_error_notify(move |s| {
                    if s.error().is_some() {
                        if let Some(tv) = weak.upgrade() {
                            tv.show_poster();
                        }
                    }
                });
            }
            *self.current_path.borrow_mut() = path;
        }

        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        for (i, c) in std::iter::once(0.0).chain(chapters.iter().copied()).enumerate() {
            let b = gtk::Button::builder()
                .label(format!("{} · {}", i + 1, format_time(c)))
                .css_classes(["pill", "small"])
                .tooltip_text(gettext("Jump to chapter"))
                .build();
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(tv) = weak.upgrade() {
                    tv.seek(c);
                }
            });
            self.chips.append(&b);
        }
    }
}
