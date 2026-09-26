// SPDX-License-Identifier: GPL-3.0-or-later

//! Media bin: imported assets shown as draggable thumbnail cards.

use super::rows::format_time;
use crate::document::{Change, Document};
use crate::media::thumbnail;
use crate::model::*;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

pub struct MediaBin {
    doc: Rc<Document>,
    flow: gtk::FlowBox,
    count: gtk::Label,
    empty: gtk::Box,
    textures: RefCell<HashMap<PathBuf, gdk::Texture>>,
}

impl MediaBin {
    pub fn new(doc: Rc<Document>, container: &gtk::Box, on_files: impl Fn(Vec<gio::File>) + 'static) -> Rc<Self> {
        let header = gtk::Box::builder()
            .spacing(8)
            .margin_start(12)
            .margin_end(8)
            .margin_top(6)
            .build();
        header.append(&gtk::Label::builder().label(gettext("Media")).css_classes(["heading"]).build());
        let count = gtk::Label::builder().css_classes(["dim-label", "caption"]).hexpand(true).xalign(0.0).build();
        header.append(&count);
        let import = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(gettext("Import Media"))
            .action_name("win.import")
            .css_classes(["flat"])
            .build();
        header.append(&import);
        container.append(&header);

        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .halign(gtk::Align::Start)
            .min_children_per_line(2)
            .max_children_per_line(40)
            .column_spacing(10)
            .row_spacing(10)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(12)
            .valign(gtk::Align::Start)
            .build();
        let empty = gtk::Box::builder()
            .spacing(18)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .css_classes(["media-empty"])
            .build();
        empty.append(&gtk::Image::builder().icon_name("folder-videos-symbolic").pixel_size(48).css_classes(["dim-label"]).build());
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).valign(gtk::Align::Center).build();
        text.append(&gtk::Label::builder().label(gettext("No Media Yet")).xalign(0.0).css_classes(["title-4"]).build());
        text.append(
            &gtk::Label::builder()
                .label(gettext("Drop video, image or audio files here, or import them"))
                .xalign(0.0)
                .wrap(true)
                .css_classes(["dim-label"])
                .build(),
        );
        empty.append(&text);
        empty.append(
            &gtk::Button::builder()
                .label(gettext("_Import…"))
                .use_underline(true)
                .action_name("win.import")
                .valign(gtk::Align::Center)
                .css_classes(["pill", "suggested-action"])
                .build(),
        );
        let stack = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        stack.append(&empty);
        stack.append(&flow);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&stack)
            .build();
        container.append(&scroller);

        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_drop(move |_, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            on_files(list.files());
            true
        });
        container.add_controller(drop);

        let bin = Rc::new(MediaBin { doc: doc.clone(), flow, count, empty, textures: RefCell::new(HashMap::new()) });
        let weak = Rc::downgrade(&bin);
        doc.connect(move |c| {
            if c == Change::Structure {
                if let Some(b) = weak.upgrade() {
                    b.rebuild();
                }
            }
        });
        bin.rebuild();
        bin
    }

    fn rebuild(self: &Rc<Self>) {
        self.flow.remove_all();
        let p = self.doc.project();
        let n = p.assets.len();
        self.count.set_label(&if n == 0 { String::new() } else { format!("{n}") });
        self.empty.set_visible(n == 0);
        for a in &p.assets {
            let card = self.card(a);
            self.flow.append(&card);
        }
    }

    fn card(self: &Rc<Self>, a: &Asset) -> gtk::Widget {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .width_request(160)
            .height_request(90)
            .css_classes(["media-thumb"])
            .build();
        let icon = match a.kind {
            AssetKind::Video => "video-x-generic-symbolic",
            AssetKind::Image => "image-x-generic-symbolic",
            AssetKind::Audio => "audio-x-generic-symbolic",
        };
        let placeholder = gtk::Image::builder().icon_name(icon).pixel_size(32).css_classes(["dim-label"]).build();
        let overlay = gtk::Overlay::builder().child(&picture).build();
        overlay.add_overlay(&placeholder);
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.add_css_class("media-thumb-frame");

        if a.kind != AssetKind::Audio {
            self.load_thumbnail(a, &picture, &placeholder);
        }

        let name = gtk::Label::builder()
            .label(a.name())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(18)
            .css_classes(["caption-heading"])
            .build();
        let detail = match a.kind {
            AssetKind::Video => format!("{} · {}p", format_time(a.info.duration), a.info.height),
            AssetKind::Image => format!("{}×{}", a.info.width, a.info.height),
            AssetKind::Audio => format_time(a.info.duration),
        };
        let used = {
            let p = self.doc.project();
            p.titles.iter().any(|t| t.asset == a.id)
        };
        let detail = if used { format!("{detail} · {}", gettext("Title")) } else { detail };
        let info = gtk::Label::builder().label(detail).xalign(0.0).css_classes(["caption", "dim-label"]).build();

        let v = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .width_request(160)
            .hexpand(false)
            .css_classes(["media-card"])
            .build();
        v.append(&overlay);
        v.append(&name);
        v.append(&info);
        v.set_tooltip_text(Some(&a.path.to_string_lossy()));

        let id = a.id;
        let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_string().to_value())));
        let pic = picture.clone();
        drag.connect_drag_begin(move |src, _| {
            if let Some(paintable) = pic.paintable() {
                src.set_icon(Some(&paintable), 40, 24);
            }
        });
        v.add_controller(drag);

        // Double-click: open as title
        let dbl = gtk::GestureClick::new();
        let doc = self.doc.clone();
        let kind = a.kind;
        dbl.connect_pressed(move |_, n, _, _| {
            if n == 2 && kind == AssetKind::Video {
                let t = doc.edit(Change::Structure, |p| p.ensure_title_for(id));
                if let Some(t) = t {
                    doc.select(crate::document::Node::Title(t), None);
                }
            }
        });
        v.add_controller(dbl);

        let ctx = gtk::GestureClick::new();
        ctx.set_button(gdk::BUTTON_SECONDARY);
        let vv = v.clone();
        ctx.connect_pressed(move |_, _, x, y| {
            let menu = gio::Menu::new();
            let target = id.to_string().to_variant();
            let add = |label: &str, action: &str, m: &gio::Menu| {
                let item = gio::MenuItem::new(Some(label), None);
                item.set_action_and_target_value(Some(action), Some(&target));
                m.append_item(&item);
            };
            let s1 = gio::Menu::new();
            match kind {
                AssetKind::Video => {
                    add(&gettext("Add as _Title"), "win.asset-add-title", &s1);
                    add(&gettext("Use as Menu _Background"), "win.asset-background", &s1);
                }
                AssetKind::Image => add(&gettext("Use as Menu _Background"), "win.asset-background", &s1),
                AssetKind::Audio => add(&gettext("Use as Menu _Music"), "win.asset-background", &s1),
            }
            menu.append_section(None, &s1);
            let s2 = gio::Menu::new();
            add(&gettext("_Remove from Project"), "win.asset-remove", &s2);
            menu.append_section(None, &s2);
            let pop = gtk::PopoverMenu::from_model(Some(&menu));
            pop.set_parent(&vv);
            pop.set_has_arrow(false);
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.connect_closed(|p| {
                let p = p.clone();
                glib::idle_add_local_once(move || p.unparent());
            });
            pop.popup();
        });
        v.add_controller(ctx);
        // Cap the card width; pictures would otherwise request their
        // texture's natural size.
        adw::Clamp::builder().maximum_size(172).tightening_threshold(172).child(&v).build().upcast()
    }

    fn load_thumbnail(self: &Rc<Self>, a: &Asset, picture: &gtk::Picture, placeholder: &gtk::Image) {
        if let Some(t) = self.textures.borrow().get(&a.path) {
            picture.set_paintable(Some(t));
            placeholder.set_visible(false);
            return;
        }
        let path = a.path.clone();
        let kind = a.kind;
        let time = (a.info.duration * 0.1).min(10.0);
        let weak = Rc::downgrade(self);
        let (picture, placeholder) = (picture.clone(), placeholder.clone());
        glib::spawn_future_local(async move {
            let p2 = path.clone();
            let file = gio::spawn_blocking(move || match kind {
                AssetKind::Image => Some(p2),
                _ => thumbnail::frame(&p2, time, 320).ok(),
            })
            .await
            .ok()
            .flatten();
            let Some(file) = file else { return };
            let Ok(tex) = gdk::Texture::from_filename(&file) else { return };
            if let Some(bin) = weak.upgrade() {
                bin.textures.borrow_mut().insert(path, tex.clone());
            }
            picture.set_paintable(Some(&tex));
            placeholder.set_visible(false);
        });
    }
}
