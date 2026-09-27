// SPDX-License-Identifier: GPL-3.0-or-later

//! Disc structure sidebar: menus and titles.

use super::rows::format_time;
use crate::document::{Change, Document, Node};
use crate::media::thumbnail;
use crate::model::*;
use crate::render::{self, ImageCache};
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

const THUMB_W: i32 = 64;
const THUMB_H: i32 = 36;

struct Row {
    node: Node,
    row: gtk::ListBoxRow,
    title: gtk::Label,
    subtitle: gtk::Label,
    preview: gtk::Picture,
    badge: gtk::Label,
}

pub struct Sidebar {
    doc: Rc<Document>,
    images: Rc<ImageCache>,
    menus: gtk::ListBox,
    titles: gtk::ListBox,
    rows: RefCell<Vec<Row>>,
    syncing: Cell<bool>,
    preview_queued: Cell<bool>,
    title_thumbs: RefCell<HashMap<(PathBuf, i64), gdk::Texture>>,
}

fn section_header(title: &str, action: Option<(&str, &str)>) -> gtk::Box {
    let b = gtk::Box::builder()
        .spacing(6)
        .margin_start(18)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(6)
        .build();
    let l = gtk::Label::builder().label(title).xalign(0.0).hexpand(true).css_classes(["heading", "dim-label"]).build();
    b.append(&l);
    if let Some((icon, act)) = action {
        let btn = gtk::Button::builder()
            .icon_name(icon)
            .action_name(act)
            .css_classes(["flat", "circular"])
            .valign(gtk::Align::Center)
            .build();
        b.append(&btn);
    }
    b
}

impl Sidebar {
    pub fn new(doc: Rc<Document>, container: gtk::Box, images: Rc<ImageCache>) -> Rc<Self> {
        let menus = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let titles = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let s = Rc::new(Sidebar {
            doc: doc.clone(),
            images: images.clone(),
            menus: menus.clone(),
            titles: titles.clone(),
            rows: RefCell::new(Vec::new()),
            syncing: Cell::new(false),
            preview_queued: Cell::new(false),
            title_thumbs: RefCell::new(HashMap::new()),
        });
        let weak = Rc::downgrade(&s);
        images.connect_ready(move || {
            if let Some(s) = weak.upgrade() {
                s.queue_previews();
            }
        });

        let mh = section_header(&gettext("Menus"), Some(("list-add-symbolic", "win.add-menu")));
        mh.last_child().unwrap().set_tooltip_text(Some(&gettext("New Menu")));
        container.append(&mh);
        container.append(&menus);
        let th = section_header(&gettext("Titles"), Some(("list-add-symbolic", "win.import")));
        th.last_child().unwrap().set_tooltip_text(Some(&gettext("Import Media")));
        container.append(&th);
        container.append(&titles);

        for list in [&menus, &titles] {
            let weak = Rc::downgrade(&s);
            list.connect_row_selected(move |list, row| {
                let Some(s) = weak.upgrade() else { return };
                if s.syncing.get() {
                    return;
                }
                let Some(row) = row else { return };
                let node = s.rows.borrow().iter().find(|r| &r.row == row).map(|r| r.node);
                if let Some(node) = node {
                    // Only one list has a selection at a time.
                    let other = if list == &s.menus { &s.titles } else { &s.menus };
                    s.syncing.set(true);
                    other.unselect_all();
                    s.syncing.set(false);
                    s.doc.select(node, None);
                }
            });
        }

        // Dropping a video onto the titles list creates a title.
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        let d = doc.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Some(id) = value.get::<String>().ok().and_then(|s| s.parse::<Id>().ok()) else { return false };
            let title = d.edit(Change::Structure, |p| p.ensure_title_for(id));
            if let Some(t) = title {
                d.select(Node::Title(t), None);
            }
            title.is_some()
        });
        titles.add_controller(drop);

        // Delete removes the selected menu/title.
        for list in [&menus, &titles] {
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(|c, key, _, _| {
                if key == gdk::Key::Delete {
                    if let Some(w) = c.widget() {
                        let _ = w.activate_action("win.delete-node", None);
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            list.add_controller(keys);
        }

        let weak = Rc::downgrade(&s);
        doc.connect(move |c| {
            let Some(s) = weak.upgrade() else { return };
            match c {
                Change::Structure => s.rebuild(),
                Change::Content => {
                    s.refresh_labels();
                    s.queue_previews();
                }
                Change::Selection => s.sync_selection(),
                Change::File => {}
            }
        });
        s.rebuild();
        s
    }

    fn make_row(self: &Rc<Self>, node: Node, title: &str, subtitle: &str) -> gtk::ListBoxRow {
        let b = gtk::Box::builder().spacing(10).margin_top(4).margin_bottom(4).build();
        let preview = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .width_request(THUMB_W)
            .height_request(THUMB_H)
            .build();
        let frame = gtk::Overlay::builder().child(&preview).valign(gtk::Align::Center).css_classes(["sidebar-thumb"]).build();
        frame.set_overflow(gtk::Overflow::Hidden);
        let icon = if matches!(node, Node::Menu(_)) { "view-grid-symbolic" } else { "video-x-generic-symbolic" };
        let placeholder = gtk::Image::builder().icon_name(icon).css_classes(["dim-label"]).build();
        frame.add_overlay(&placeholder);
        preview.connect_paintable_notify(move |p| placeholder.set_visible(p.paintable().is_none()));
        b.append(&adw::Clamp::builder().maximum_size(THUMB_W).child(&frame).build());

        let v = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).hexpand(true).build();
        let t = gtk::Label::builder().label(title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let st = gtk::Label::builder()
            .label(subtitle)
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .visible(!subtitle.is_empty())
            .build();
        v.append(&t);
        v.append(&st);
        b.append(&v);
        let badge = gtk::Label::builder()
            .label(gettext("Start"))
            .css_classes(["start-badge", "caption-heading"])
            .valign(gtk::Align::Center)
            .tooltip_text(gettext("Plays when the disc is inserted"))
            .visible(false)
            .build();
        b.append(&badge);
        let row = gtk::ListBoxRow::builder().child(&b).build();

        // Context menu
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let doc = self.doc.clone();
        let r = row.clone();
        click.connect_pressed(move |_, _, x, y| {
            doc.select(node, None);
            let menu = gio::Menu::new();
            if matches!(node, Node::Title(_)) {
                menu.append(Some(&gettext("_Preview…")), Some("win.preview-title"));
                menu.append(Some(&gettext("Choose _Thumbnail…")), Some("win.choose-thumbnail"));
                menu.append(Some(&gettext("Create _Chapter Menu")), Some("win.add-chapter-menu"));
            }
            menu.append(Some(&gettext("Move _Up")), Some("win.move-node-up"));
            menu.append(Some(&gettext("Move _Down")), Some("win.move-node-down"));
            menu.append(Some(&gettext("_Delete")), Some("win.delete-node"));
            let pop = gtk::PopoverMenu::from_model(Some(&menu));
            pop.set_parent(&r);
            pop.set_has_arrow(false);
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.connect_closed(|p| {
                let p = p.clone();
                glib::idle_add_local_once(move || p.unparent());
            });
            pop.popup();
        });
        row.add_controller(click);
        self.rows.borrow_mut().push(Row { node, row: row.clone(), title: t, subtitle: st, preview, badge });
        row
    }

    fn labels(p: &Project, node: Node) -> (String, String) {
        match node {
            Node::Menu(id) => p
                .menu(id)
                .map(|m| {
                    let n = m.buttons().count() as u32;
                    (m.name.clone(), ngettext("{} button", "{} buttons", n).replace("{}", &n.to_string()))
                })
                .unwrap_or_default(),
            Node::Title(id) => p
                .title(id)
                .map(|t| {
                    let dur = p.asset(t.asset).map_or(0.0, |a| a.info.duration);
                    let n = t.chapters.len() as u32 + 1;
                    let chapters = ngettext("{} chapter", "{} chapters", n).replace("{}", &n.to_string());
                    let kept = if t.keep_video { format!(" · {}", gettext("original video")) } else { String::new() };
                    (t.name.clone(), format!("{} · {chapters}{kept}", format_time(dur)))
                })
                .unwrap_or_default(),
            Node::None => Default::default(),
        }
    }

    fn rebuild(self: &Rc<Self>) {
        self.syncing.set(true);
        self.rows.borrow_mut().clear();
        self.menus.remove_all();
        self.titles.remove_all();
        let p = self.doc.project();
        for m in &p.menus {
            let node = Node::Menu(m.id);
            let (t, s) = Self::labels(&p, node);
            let row = self.make_row(node, &t, &s);
            self.menus.append(&row);
        }
        for t in &p.titles {
            let node = Node::Title(t.id);
            let (tl, s) = Self::labels(&p, node);
            let row = self.make_row(node, &tl, &s);
            self.titles.append(&row);
            if let Some(a) = p.asset(t.asset) {
                self.load_title_thumb(a.path.clone(), p.title_poster(t.id));
            }
        }
        if p.titles.is_empty() {
            let hint = gtk::Label::builder()
                .label(gettext("Drop videos here"))
                .css_classes(["dim-label", "caption"])
                .margin_top(12)
                .margin_bottom(12)
                .build();
            let row = gtk::ListBoxRow::builder().child(&hint).selectable(false).activatable(false).build();
            self.titles.append(&row);
        }
        drop(p);
        self.syncing.set(false);
        self.sync_selection();
        self.refresh_labels();
        self.render_previews();
    }

    fn queue_previews(self: &Rc<Self>) {
        if self.preview_queued.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            this.preview_queued.set(false);
            this.render_previews();
        });
    }

    fn render_previews(&self) {
        let p = self.doc.project();
        let scale = self.menus.scale_factor().max(1);
        for r in self.rows.borrow().iter() {
            match r.node {
                Node::Menu(id) => {
                    if let Some(m) = p.menu(id) {
                        let tex = render::menu_thumbnail(&p, m, &self.images, THUMB_W * scale);
                        r.preview.set_paintable(tex.as_ref());
                    }
                }
                Node::Title(id) => {
                    let key = p.title(id).and_then(|t| p.asset(t.asset)).map(|a| (a.path.clone(), (p.title_poster(id) * 1000.0) as i64));
                    if let Some(tex) = key.and_then(|k| self.title_thumbs.borrow().get(&k).cloned()) {
                        r.preview.set_paintable(Some(&tex));
                    }
                }
                Node::None => {}
            }
        }
    }

    fn load_title_thumb(self: &Rc<Self>, path: PathBuf, time: f64) {
        let key = (path.clone(), (time * 1000.0) as i64);
        if self.title_thumbs.borrow().contains_key(&key) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let p2 = path.clone();
            let file = gio::spawn_blocking(move || thumbnail::frame(&p2, time, 320).ok()).await.ok().flatten();
            let Some(tex) = file.and_then(|f| gdk::Texture::from_filename(f).ok()) else { return };
            if let Some(s) = weak.upgrade() {
                s.title_thumbs.borrow_mut().insert(key, tex);
                s.render_previews();
            }
        });
    }

    fn refresh_labels(&self) {
        let p = self.doc.project();
        let start = match p.first_play {
            FirstPlay::FirstMenu if !p.menus.is_empty() => p.menus.first().map(|m| Node::Menu(m.id)),
            _ => p.titles.first().map(|t| Node::Title(t.id)),
        };
        for r in self.rows.borrow().iter() {
            let (tl, sl) = Self::labels(&p, r.node);
            if r.title.label() != tl {
                r.title.set_label(&tl);
            }
            if r.subtitle.label() != sl {
                r.subtitle.set_label(&sl);
            }
            r.badge.set_visible(Some(r.node) == start);
        }
    }

    fn sync_selection(&self) {
        self.syncing.set(true);
        let node = self.doc.node();
        self.menus.unselect_all();
        self.titles.unselect_all();
        for r in self.rows.borrow().iter() {
            if r.node == node {
                if let Some(list) = r.row.parent().and_downcast::<gtk::ListBox>() {
                    list.select_row(Some(&r.row));
                }
            }
        }
        self.syncing.set(false);
    }
}
