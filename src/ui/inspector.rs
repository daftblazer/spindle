// SPDX-License-Identifier: GPL-3.0-or-later

//! Property inspector for the selected menu, menu item or title.

use super::rows::{self, format_time, group};
use crate::document::{Change, Document, Node};
use crate::model::*;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Inspector {
    doc: Rc<Document>,
    container: gtk::Box,
    title: adw::WindowTitle,
    geometry: RefCell<Option<(Id, Id, [adw::SpinRow; 4])>>,
    rebuild_queued: Cell<bool>,
    /// The main text row of the current page (label/text/name).
    primary: RefCell<Option<gtk::Widget>>,
}

/// A row of icon buttons for alignment actions.
fn align_bar(distribute: bool) -> gtk::Box {
    let bar = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).margin_top(6).margin_bottom(6).build();
    let group = |items: &[(&str, &str, String)]| {
        let linked = gtk::Box::builder().css_classes(["linked"]).build();
        for (icon, action, tip) in items {
            let b = gtk::Button::builder()
                .icon_name(*icon)
                .action_name(*action)
                .tooltip_text(tip.as_str())
                .build();
            b.update_property(&[gtk::accessible::Property::Label(tip)]);
            linked.append(&b);
        }
        linked
    };
    bar.append(&group(&[
        ("spindle-align-left-symbolic", "win.align-left", gettext("Align Left")),
        ("spindle-align-hcenter-symbolic", "win.align-hcenter", gettext("Center Horizontally")),
        ("spindle-align-right-symbolic", "win.align-right", gettext("Align Right")),
    ]));
    bar.append(&group(&[
        ("spindle-align-top-symbolic", "win.align-top", gettext("Align Top")),
        ("spindle-align-vcenter-symbolic", "win.align-vcenter", gettext("Center Vertically")),
        ("spindle-align-bottom-symbolic", "win.align-bottom", gettext("Align Bottom")),
    ]));
    if distribute {
        bar.append(&group(&[
            ("spindle-distribute-h-symbolic", "win.distribute-h", gettext("Distribute Horizontally")),
            ("spindle-distribute-v-symbolic", "win.distribute-v", gettext("Distribute Vertically")),
        ]));
    }
    bar
}

/// Small "open file" button placed after a combo row.
fn pick_button(tooltip: &str, action: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(tooltip)
        .action_name(action)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build()
}

fn delete_button(label: &str, action: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let b = gtk::Button::builder()
        .label(label)
        .action_name(action)
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    g.add(&b);
    g
}

/// (id, label) pairs for assets of the given kinds.
fn assets_of(p: &Project, kinds: &[AssetKind]) -> Vec<(Id, String)> {
    p.assets.iter().filter(|a| kinds.contains(&a.kind)).map(|a| (a.id, a.name())).collect()
}

/// Combo labels with a leading "none" entry and the index of `current`.
fn optional_choice(none: &str, options: &[(Id, String)], current: Option<Id>) -> (Vec<String>, usize) {
    let mut labels = vec![none.to_string()];
    labels.extend(options.iter().map(|(_, l)| l.clone()));
    let sel = current.and_then(|c| options.iter().position(|(id, _)| *id == c)).map_or(0, |i| i + 1);
    (labels, sel)
}

fn pick(options: &[(Id, String)], i: usize) -> Option<Id> {
    i.checked_sub(1).and_then(|i| options.get(i)).map(|(id, _)| *id)
}

impl Inspector {
    pub fn new(doc: Rc<Document>, container: gtk::Box, title: adw::WindowTitle) -> Rc<Self> {
        let insp = Rc::new(Inspector {
            doc: doc.clone(),
            container,
            title,
            geometry: RefCell::new(None),
            rebuild_queued: Cell::new(false),
            primary: RefCell::new(None),
        });
        let weak = Rc::downgrade(&insp);
        doc.connect(move |c| {
            let Some(i) = weak.upgrade() else { return };
            match c {
                Change::Structure | Change::Selection => i.queue_rebuild(),
                Change::Content => i.refresh_geometry(),
                Change::File => {}
            }
        });
        insp.rebuild();
        insp
    }

    fn queue_rebuild(self: &Rc<Self>) {
        if self.rebuild_queued.replace(true) {
            return;
        }
        let this = self.clone();
        glib::idle_add_local_once(move || {
            this.rebuild_queued.set(false);
            this.rebuild();
        });
    }

    fn refresh_geometry(&self) {
        let g = self.geometry.borrow();
        let Some((menu, item, rows)) = g.as_ref() else { return };
        let p = self.doc.project();
        let Some(i) = p.menu(*menu).and_then(|m| m.item(*item)) else { return };
        let r = i.rect;
        for (row, v) in rows.iter().zip([r.x, r.y, r.w, r.h]) {
            rows::set_spin_quietly(row, v);
        }
    }

    /// Focus the main text field (double-click / Enter on the canvas).
    pub fn focus_primary(&self) {
        if let Some(w) = self.primary.borrow().as_ref() {
            w.grab_focus();
        }
    }

    pub fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.container.first_child() {
            self.container.remove(&c);
        }
        *self.geometry.borrow_mut() = None;
        *self.primary.borrow_mut() = None;
        let page = adw::PreferencesPage::new();
        page.set_vexpand(true);
        let n_selected = self.doc.selection().len();
        if n_selected > 1 {
            self.multi_page(&page, n_selected);
            self.title.set_title(&gettext("Selection"));
            self.title.set_subtitle(&ngettext("{} item", "{} items", n_selected as u32).replace("{}", &n_selected.to_string()));
            self.container.append(&page);
            return;
        }
        let (title, subtitle) = match (self.doc.node(), self.doc.item()) {
            (Node::Menu(m), None) => {
                self.menu_page(&page, m);
                (gettext("Menu"), self.doc.project().menu(m).map(|m| m.name.clone()).unwrap_or_default())
            }
            (Node::Menu(m), Some(item)) => {
                let kind = self.doc.project().menu(m).and_then(|mm| mm.item(item)).map(|i| i.kind_name()).unwrap_or("");
                self.item_page(&page, m, item);
                (gettext(kind), String::new())
            }
            (Node::Title(t), _) => {
                self.title_page(&page, t);
                (gettext("Title"), self.doc.project().title(t).map(|t| t.name.clone()).unwrap_or_default())
            }
            (Node::None, _) => {
                let status = adw::StatusPage::builder()
                    .icon_name("document-properties-symbolic")
                    .title(gettext("Nothing Selected"))
                    .build();
                status.add_css_class("compact");
                self.container.append(&status);
                self.title.set_title(&gettext("Properties"));
                self.title.set_subtitle("");
                return;
            }
        };
        self.title.set_title(&title);
        self.title.set_subtitle(&subtitle);
        self.container.append(&page);
    }

    fn multi_page(&self, page: &adw::PreferencesPage, n: usize) {
        let g = group(&gettext("Arrange"));
        g.set_description(Some(&gettext("Aligns the selected items to each other")));
        g.add(&align_bar(n > 2));
        page.add(&g);
        let g = adw::PreferencesGroup::new();
        let actions = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
        for (label, action) in [(gettext("_Duplicate"), "win.duplicate-item"), (gettext("Bring to _Front"), "win.raise-item")] {
            actions.append(&gtk::Button::builder().label(label).use_underline(true).action_name(action).css_classes(["pill"]).build());
        }
        g.add(&actions);
        page.add(&g);
        page.add(&delete_button(&gettext("Delete Items"), "win.delete-item"));
    }

    fn menu_page(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(m) = p.menu(id) else { return };

        let g = group(&gettext("Menu"));
        let name = rows::entry(doc, &gettext("Name"), &m.name, Change::Content, move |p, v| {
            if let Some(m) = p.menu_mut(id) {
                m.name = v;
            }
        });
        *self.primary.borrow_mut() = Some(name.clone().upcast());
        g.add(&name);
        let buttons: Vec<(Id, String)> = m
            .buttons()
            .map(|i| (i.id, i.button().map(|b| b.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| gettext("Button"))))
            .collect();
        let (labels, sel) = optional_choice(&gettext("Automatic"), &buttons, m.default_button);
        let row = rows::combo(doc, &gettext("Default Button"), &labels, sel, Change::Content, move |p, i| {
            if let Some(m) = p.menu_mut(id) {
                m.default_button = pick(&buttons, i);
            }
        });
        row.set_tooltip_text(Some(&gettext("Automatic remembers the last selected button")));
        g.add(&row);
        page.add(&g);

        let g = group(&gettext("Background"));
        g.add(&rows::color(doc, &gettext("Color"), m.background.color, Change::Content, move |p, c| {
            if let Some(m) = p.menu_mut(id) {
                m.background.color = c;
            }
        }));
        g.add(&rows::switch(doc, &gettext("Gradient"), m.background.gradient.is_some(), Change::Structure, move |p, v| {
            if let Some(m) = p.menu_mut(id) {
                let c = m.background.color;
                m.background.gradient = v.then(|| Rgba::new(c.r * 0.4, c.g * 0.4, c.b * 0.4, 1.0));
            }
        }));
        if let Some(bottom) = m.background.gradient {
            g.add(&rows::color(doc, &gettext("Bottom Color"), bottom, Change::Content, move |p, c| {
                if let Some(m) = p.menu_mut(id) {
                    m.background.gradient = Some(c);
                }
            }));
        }
        let images = assets_of(&p, &[AssetKind::Image]);
        let (labels, sel) = optional_choice(&gettext("None"), &images, m.background.image);
        let image_row = rows::combo(doc, &gettext("Image"), &labels, sel, Change::Content, move |p, i| {
            if let Some(m) = p.menu_mut(id) {
                m.background.image = pick(&images, i);
            }
        });
        image_row.set_subtitle(&gettext("Fills the screen behind everything"));
        image_row.add_suffix(&pick_button(&gettext("Choose Image File…"), "win.menu-background-image"));
        g.add(&image_row);
        let videos = assets_of(&p, &[AssetKind::Video]);
        let (labels, sel) = optional_choice(&gettext("None"), &videos, m.background.video);
        let row = rows::combo(doc, &gettext("Motion Video"), &labels, sel, Change::Content, move |p, i| {
            if let Some(m) = p.menu_mut(id) {
                m.background.video = pick(&videos, i);
            }
        });
        row.set_subtitle(&gettext("Loops behind the menu"));
        g.add(&row);
        page.add(&g);

        let g = group(&gettext("Sound"));
        let audio: Vec<(Id, String)> = p
            .assets
            .iter()
            .filter(|a| a.info.has_audio())
            .map(|a| (a.id, a.name()))
            .collect();
        let (labels, sel) = optional_choice(&gettext("Silent"), &audio, m.audio);
        let music_row = rows::combo(doc, &gettext("Music"), &labels, sel, Change::Structure, move |p, i| {
            if let Some(m) = p.menu_mut(id) {
                m.audio = pick(&audio, i);
            }
        });
        music_row.set_subtitle(&gettext("Loops while the menu is shown"));
        music_row.add_suffix(&pick_button(&gettext("Choose Music File…"), "win.menu-music"));
        g.add(&music_row);
        let row = rows::spin(doc, &gettext("Loop Length"), m.duration, 1.0, 600.0, 1.0, 0, Change::Content, move |p, v| {
            if let Some(m) = p.menu_mut(id) {
                m.duration = v;
            }
        });
        row.set_subtitle(&gettext("Seconds, for motion menus and music"));
        g.add(&row);
        page.add(&g);

        page.add(&delete_button(&gettext("Delete Menu"), "win.delete-node"));
    }

    fn geometry_group(&self, page: &adw::PreferencesPage, menu: Id, item: Id, r: Rect) {
        let doc = &self.doc;
        let g = group(&gettext("Position"));
        let mk = |label: &str, v: f64, max: f64, idx: usize| {
            rows::spin(doc, label, v, 0.0, max, 1.0, 0, Change::Content, move |p, v| {
                if let Some(i) = p.menu_mut(menu).and_then(|m| m.item_mut(item)) {
                    let mut r = i.rect;
                    match idx {
                        0 => r.x = v,
                        1 => r.y = v,
                        2 => r.w = v,
                        _ => r.h = v,
                    }
                    i.rect = r.clamped();
                }
            })
        };
        let rows = [
            mk("X", r.x, DESIGN_WIDTH, 0),
            mk("Y", r.y, DESIGN_HEIGHT, 1),
            mk(&gettext("Width"), r.w, DESIGN_WIDTH, 2),
            mk(&gettext("Height"), r.h, DESIGN_HEIGHT, 3),
        ];
        for row in &rows {
            g.add(row);
        }
        let hint = gtk::Label::builder()
            .label(gettext("Align to the safe area"))
            .css_classes(["dim-label", "caption"])
            .margin_top(12)
            .build();
        g.add(&hint);
        g.add(&align_bar(false));
        page.add(&g);
        *self.geometry.borrow_mut() = Some((menu, item, rows));
    }

    fn text_style_rows(&self, g: &adw::PreferencesGroup, style: &TextStyle, menu: Id, item: Id, with_color: bool) {
        let doc = &self.doc;
        fn style_mut(p: &mut Project, menu: Id, item: Id) -> Option<&mut TextStyle> {
            match &mut p.menu_mut(menu)?.item_mut(item)?.kind {
                ItemKind::Button(b) => Some(&mut b.text),
                ItemKind::Text(t) => Some(&mut t.style),
                _ => None,
            }
        }
        g.add(&rows::font(doc, &gettext("Font"), &style.font, Change::Content, move |p, f| {
            if let Some(s) = style_mut(p, menu, item) {
                s.font = f;
            }
        }));
        if with_color {
            g.add(&rows::color(doc, &gettext("Color"), style.color, Change::Content, move |p, c| {
                if let Some(s) = style_mut(p, menu, item) {
                    s.color = c;
                }
            }));
        }
        let aligns = [gettext("Left"), gettext("Center"), gettext("Right")];
        let sel = match style.align {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
        };
        g.add(&rows::combo(doc, &gettext("Alignment"), &aligns, sel, Change::Content, move |p, i| {
            if let Some(s) = style_mut(p, menu, item) {
                s.align = [Align::Left, Align::Center, Align::Right][i.min(2)];
            }
        }));
        g.add(&rows::switch(doc, &gettext("Shadow"), style.shadow, Change::Content, move |p, v| {
            if let Some(s) = style_mut(p, menu, item) {
                s.shadow = v;
            }
        }));
    }

    fn item_page(&self, page: &adw::PreferencesPage, menu: Id, item: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(m) = p.menu(menu) else { return };
        let Some(it) = m.item(item) else { return };

        fn button_mut(p: &mut Project, menu: Id, item: Id) -> Option<&mut ButtonItem> {
            p.menu_mut(menu)?.item_mut(item)?.button_mut()
        }

        match &it.kind {
            ItemKind::Button(b) => {
                let g = group(&gettext("Button"));
                let label = rows::entry(doc, &gettext("Label"), &b.label, Change::Content, move |p, v| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.label = v;
                    }
                });
                *self.primary.borrow_mut() = Some(label.clone().upcast());
                g.add(&label);

                // Action: none / play title / show menu
                let mut targets: Vec<(Action, String)> =
                    vec![(Action::None, gettext("Do Nothing")), (Action::PlayAll, gettext("Play All Titles"))];
                for t in &p.titles {
                    targets.push((Action::PlayTitle { title: t.id, chapter: 0 }, format!("{} {}", gettext("Play"), t.name)));
                }
                for mm in p.menus.iter().filter(|x| x.id != menu) {
                    targets.push((Action::ShowMenu(mm.id), format!("{} {}", gettext("Show"), mm.name)));
                }
                let sel = targets
                    .iter()
                    .position(|(a, _)| match (a, b.action) {
                        (Action::PlayTitle { title: x, .. }, Action::PlayTitle { title: y, .. }) => *x == y,
                        (a, b) => *a == b,
                    })
                    .unwrap_or(0);
                let labels: Vec<String> = targets.iter().map(|(_, l)| l.clone()).collect();
                let actions: Vec<Action> = targets.iter().map(|(a, _)| *a).collect();
                g.add(&rows::combo(doc, &gettext("Action"), &labels, sel, Change::Structure, move |p, i| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.action = actions.get(i).copied().unwrap_or(Action::None);
                    }
                }));
                if let Action::PlayTitle { title, chapter } = b.action {
                    let n_chapters = p.title(title).map_or(1, |t| t.chapters.len() + 1);
                    let row = rows::spin(doc, &gettext("Start at Chapter"), chapter as f64 + 1.0, 1.0, n_chapters as f64, 1.0, 0, Change::Content, move |p, v| {
                        if let Some(b) = button_mut(p, menu, item) {
                            if let Action::PlayTitle { title, .. } = b.action {
                                b.action = Action::PlayTitle { title, chapter: (v as u32).saturating_sub(1) };
                            }
                        }
                    });
                    g.add(&row);
                }
                page.add(&g);

                let g = group(&gettext("Appearance"));
                self.text_style_rows(&g, &b.text, menu, item, true);
                let highlights = [gettext("Frame"), gettext("Text Color"), gettext("Underline")];
                let sel = match b.highlight {
                    Highlight::Frame => 0,
                    Highlight::Text => 1,
                    Highlight::Underline => 2,
                };
                g.add(&rows::combo(doc, &gettext("Highlight Style"), &highlights, sel, Change::Content, move |p, i| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.highlight = [Highlight::Frame, Highlight::Text, Highlight::Underline][i.min(2)];
                    }
                }));
                g.add(&rows::color(doc, &gettext("Selected Color"), b.selected_color, Change::Content, move |p, c| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.selected_color = c;
                    }
                }));
                g.add(&rows::color(doc, &gettext("Activated Color"), b.activated_color, Change::Content, move |p, c| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.activated_color = c;
                    }
                }));
                let fill_row = rows::switch(doc, &gettext("Background Fill"), b.fill.is_some(), Change::Structure, move |p, v| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.fill = v.then_some(Rgba::new(0.0, 0.0, 0.0, 0.5));
                    }
                });
                g.add(&fill_row);
                if let Some(fill) = b.fill {
                    g.add(&rows::color(doc, &gettext("Fill Color"), fill, Change::Content, move |p, c| {
                        if let Some(b) = button_mut(p, menu, item) {
                            b.fill = Some(c);
                        }
                    }));
                }
                page.add(&g);

                let g = group(&gettext("Thumbnail"));
                let videos = assets_of(&p, &[AssetKind::Video]);
                let (labels, sel) = optional_choice(&gettext("None"), &videos, b.thumbnail);
                g.add(&rows::combo(doc, &gettext("Video"), &labels, sel, Change::Structure, move |p, i| {
                    let asset = pick(&videos, i);
                    // Start from the title's chosen thumbnail for that video.
                    let poster = asset.and_then(|a| p.titles.iter().find(|t| t.asset == a).map(|t| t.id)).map(|t| p.title_poster(t));
                    if let Some(b) = button_mut(p, menu, item) {
                        b.thumbnail = asset;
                        if let Some(t) = poster {
                            b.thumbnail_time = t;
                        }
                    }
                }));
                if let Some(a) = b.thumbnail.and_then(|a| p.asset(a)) {
                    let marks = p.titles.iter().find(|t| t.asset == a.id).map(|t| t.chapters.clone()).unwrap_or_default();
                    g.add(&rows::frame(doc, &gettext("Frame"), a, b.thumbnail_time, marks, Change::Structure, move |p, t| {
                        if let Some(b) = button_mut(p, menu, item) {
                            b.thumbnail_time = t;
                        }
                    }));
                }
                page.add(&g);

                let g = group(&gettext("Remote Navigation"));
                g.set_description(Some(&gettext("Which button is selected when pressing the arrow keys")));
                let others: Vec<(Id, String)> = m
                    .buttons()
                    .filter(|o| o.id != item)
                    .map(|o| (o.id, o.button().map(|b| b.label.clone()).unwrap_or_default()))
                    .collect();
                let dirs = [gettext("Up"), gettext("Down"), gettext("Left"), gettext("Right")];
                let current = [b.nav.up, b.nav.down, b.nav.left, b.nav.right];
                for (d, (label, cur)) in dirs.iter().zip(current).enumerate() {
                    let (labels, sel) = optional_choice(&gettext("Automatic"), &others, cur);
                    let others = others.clone();
                    g.add(&rows::combo(doc, label, &labels, sel, Change::Content, move |p, i| {
                        if let Some(b) = button_mut(p, menu, item) {
                            let v = pick(&others, i);
                            match d {
                                0 => b.nav.up = v,
                                1 => b.nav.down = v,
                                2 => b.nav.left = v,
                                _ => b.nav.right = v,
                            }
                        }
                    }));
                }
                page.add(&g);
            }
            ItemKind::Text(t) => {
                let g = group(&gettext("Text"));
                let text_row = rows::entry(doc, &gettext("Text"), &t.text, Change::Content, move |p, v| {
                    if let Some(ItemKind::Text(t)) = p.menu_mut(menu).and_then(|m| m.item_mut(item)).map(|i| &mut i.kind) {
                        t.text = v;
                    }
                });
                *self.primary.borrow_mut() = Some(text_row.clone().upcast());
                g.add(&text_row);
                let replace = adw::ButtonRow::builder()
                    .title(gettext("Replace with Image…"))
                    .start_icon_name("image-x-generic-symbolic")
                    .action_name("win.replace-with-image")
                    .build();
                replace.set_tooltip_text(Some(&gettext("Use a picture, such as a transparent PNG logo, instead of this text")));
                g.add(&replace);
                self.text_style_rows(&g, &t.style, menu, item, true);
                page.add(&g);
            }
            ItemKind::Image(img) => {
                let g = group(&gettext("Image"));
                let images = assets_of(&p, &[AssetKind::Image, AssetKind::Video]);
                let sel = images.iter().position(|(id, _)| *id == img.asset).unwrap_or(0);
                let labels: Vec<String> = images.iter().map(|(_, l)| l.clone()).collect();
                g.add(&rows::combo(doc, &gettext("Source"), &labels, sel, Change::Structure, move |p, i| {
                    if let (Some((id, _)), Some(ItemKind::Image(img))) =
                        (images.get(i), p.menu_mut(menu).and_then(|m| m.item_mut(item)).map(|i| &mut i.kind))
                    {
                        img.asset = *id;
                    }
                }));
                g.add(
                    &adw::ButtonRow::builder()
                        .title(gettext("Choose Another Image…"))
                        .start_icon_name("document-open-symbolic")
                        .action_name("win.replace-with-image")
                        .build(),
                );
                if let Some(a) = p.asset(img.asset).filter(|a| a.kind == AssetKind::Video) {
                    g.add(&rows::frame(doc, &gettext("Frame"), a, img.time, vec![], Change::Structure, move |p, t| {
                        if let Some(ItemKind::Image(img)) = p.menu_mut(menu).and_then(|m| m.item_mut(item)).map(|i| &mut i.kind) {
                            img.time = t;
                        }
                    }));
                }
                page.add(&g);
            }
            ItemKind::Shape(sh) => {
                fn shape_mut(p: &mut Project, menu: Id, item: Id) -> Option<&mut ShapeItem> {
                    match &mut p.menu_mut(menu)?.item_mut(item)?.kind {
                        ItemKind::Shape(s) => Some(s),
                        _ => None,
                    }
                }
                let g = group(&gettext("Shape"));
                g.add(&rows::color(doc, &gettext("Fill"), sh.fill, Change::Content, move |p, c| {
                    if let Some(s) = shape_mut(p, menu, item) {
                        s.fill = c;
                    }
                }));
                g.add(&rows::spin(doc, &gettext("Corner Radius"), sh.radius, 0.0, 200.0, 2.0, 0, Change::Content, move |p, v| {
                    if let Some(s) = shape_mut(p, menu, item) {
                        s.radius = v;
                    }
                }));
                page.add(&g);
            }
        }
        self.geometry_group(page, menu, item, it.rect);
        page.add(&delete_button(&gettext("Delete"), "win.delete-item"));
    }

    fn subtitles_group(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(t) = p.title(id) else { return };
        let g = group(&gettext("Subtitles"));
        g.set_description(Some(&gettext("Converted to Blu-ray subtitles. Text subtitles use the style in Disc Settings; ASS keeps its own styling.")));

        fn track_mut(p: &mut Project, title: Id, track: Id) -> Option<&mut SubtitleTrack> {
            p.title_mut(title)?.subtitles.iter_mut().find(|s| s.id == track)
        }

        for track in &t.subtitles {
            let tid = track.id;
            let supported = track.kind() != SubtitleKind::Unsupported;
            let origin = match &track.source {
                SubtitleSource::Embedded { .. } => gettext("in video"),
                SubtitleSource::External { path } => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            };
            let mut subtitle = format!("{} · {} · {}", language_name(&track.lang), codec_label(&track.codec), origin);
            if !supported {
                subtitle = format!("{} · {}", codec_label(&track.codec), gettext("not supported yet"));
            } else if track.forced {
                subtitle.push_str(&format!(" · {}", gettext("forced")));
            }
            let row = adw::ExpanderRow::builder()
                .title(glib::markup_escape_text(&track.name).as_str())
                .subtitle(glib::markup_escape_text(&subtitle).as_str())
                .show_enable_switch(true)
                .enable_expansion(track.enabled && supported)
                .sensitive(supported)
                .build();
            let d = doc.clone();
            row.connect_enable_expansion_notify(move |r| {
                let on = r.enables_expansion();
                if d.project().title(id).and_then(|t| t.subtitles.iter().find(|s| s.id == tid)).is_some_and(|s| s.enabled == on) {
                    return;
                }
                d.edit(Change::Structure, |p| {
                    if let Some(s) = track_mut(p, id, tid) {
                        s.enabled = on;
                    }
                    if let Some(t) = p.title_mut(id) {
                        if !on && t.default_subtitle == Some(tid) {
                            t.default_subtitle = None;
                        }
                    }
                });
            });
            let lang = rows::entry(doc, &gettext("Language Code"), &track.lang, Change::Content, move |p, v| {
                if let Some(s) = track_mut(p, id, tid) {
                    s.lang = normalize_lang(&v);
                }
            });
            lang.set_tooltip_text(Some(&gettext("ISO 639 code such as eng, fra or jpn")));
            row.add_row(&lang);
            row.add_row(&rows::entry(doc, &gettext("Name"), &track.name, Change::Content, move |p, v| {
                if let Some(s) = track_mut(p, id, tid) {
                    s.name = v;
                }
            }));
            let forced = rows::switch(doc, &gettext("Forced"), track.forced, Change::Structure, move |p, v| {
                if let Some(s) = track_mut(p, id, tid) {
                    s.forced = v;
                }
            });
            forced.set_subtitle(&gettext("Shown even when subtitles are off (foreign dialogue, signs)"));
            row.add_row(&forced);
            if matches!(track.source, SubtitleSource::External { .. }) {
                let remove = adw::ButtonRow::builder().title(gettext("Remove Subtitle File")).css_classes(["destructive-action"]).build();
                let d = doc.clone();
                remove.connect_activated(move |_| {
                    d.edit(Change::Structure, |p| {
                        if let Some(t) = p.title_mut(id) {
                            t.subtitles.retain(|s| s.id != tid);
                            if t.default_subtitle == Some(tid) {
                                t.default_subtitle = None;
                            }
                        }
                    });
                });
                row.add_row(&remove);
            }
            g.add(&row);
        }

        let on_disc: Vec<(Id, String)> = t.disc_subtitles().map(|s| (s.id, format!("{} ({})", s.name, language_name(&s.lang)))).collect();
        if !on_disc.is_empty() {
            let (labels, sel) = optional_choice(&gettext("Off"), &on_disc, t.default_subtitle);
            let row = rows::combo(doc, &gettext("Shown by Default"), &labels, sel, Change::Structure, move |p, i| {
                if let Some(t) = p.title_mut(id) {
                    t.default_subtitle = pick(&on_disc, i);
                }
            });
            row.set_subtitle(&gettext("Viewers can switch with the remote"));
            g.add(&row);
        } else if t.subtitles.is_empty() {
            let empty = adw::ActionRow::builder().title(gettext("No subtitles")).css_classes(["dim-label"]).build();
            g.add(&empty);
        }

        let add = adw::ButtonRow::builder().title(gettext("Add Subtitle File…")).start_icon_name("list-add-symbolic").build();
        add.set_action_name(Some("win.add-subtitle"));
        g.add(&add);
        page.add(&g);
    }

    fn title_page(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(t) = p.title(id) else { return };

        let g = group(&gettext("Title"));
        g.add(&rows::entry(doc, &gettext("Name"), &t.name, Change::Content, move |p, v| {
            if let Some(t) = p.title_mut(id) {
                t.name = v;
            }
        }));
        g.add(&rows::entry(doc, &gettext("Audio Language"), &t.audio_lang, Change::Content, move |p, v| {
            if let Some(t) = p.title_mut(id) {
                t.audio_lang = v;
            }
        }));
        if let Some(a) = p.asset(t.asset) {
            let info = &a.info;
            let row = adw::ActionRow::builder()
                .title(a.name())
                .subtitle(format!(
                    "{} · {}×{} · {}",
                    format_time(info.duration),
                    info.width,
                    info.height,
                    info.video_codec.clone().unwrap_or_default()
                ))
                .subtitle_selectable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("video-x-generic-symbolic"));
            g.add(&row);
        }
        page.add(&g);

        if let Some(a) = p.asset(t.asset) {
            let g = group(&gettext("Thumbnail"));
            g.set_description(Some(&gettext("Used for this title's buttons and stills in every menu")));
            let row = rows::frame(doc, &gettext("Frame"), a, p.title_poster(id), t.chapters.clone(), Change::Structure, move |p, time| {
                p.set_title_poster(id, time);
            });
            if t.poster.is_none() {
                row.set_subtitle(&gettext("Automatic"));
            } else {
                let reset = gtk::Button::builder()
                    .icon_name("edit-undo-symbolic")
                    .tooltip_text(gettext("Use Automatic Thumbnail"))
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                let d = doc.clone();
                reset.connect_clicked(move |_| {
                    d.edit(Change::Structure, |p| {
                        if let Some(t) = p.title_mut(id) {
                            t.poster = None;
                        }
                        let auto = p.title_poster(id);
                        p.set_title_poster(id, auto);
                        if let Some(t) = p.title_mut(id) {
                            t.poster = None;
                        }
                    });
                });
                row.add_suffix(&reset);
            }
            g.add(&row);
            page.add(&g);
        }

        self.subtitles_group(page, id);

        let g = group(&gettext("When Finished"));
        let ends = [gettext("Return to Menu"), gettext("Play Next Title"), gettext("Loop")];
        let sel = match t.end_action {
            EndAction::ReturnToMenu => 0,
            EndAction::PlayNextTitle => 1,
            EndAction::Loop => 2,
        };
        g.add(&rows::combo(doc, &gettext("Action"), &ends, sel, Change::Content, move |p, i| {
            if let Some(t) = p.title_mut(id) {
                t.end_action = [EndAction::ReturnToMenu, EndAction::PlayNextTitle, EndAction::Loop][i.min(2)];
            }
        }));
        let menus: Vec<(Id, String)> = p.menus.iter().map(|m| (m.id, m.name.clone())).collect();
        let (labels, sel) = optional_choice(&gettext("First Menu"), &menus, t.return_menu);
        g.add(&rows::combo(doc, &gettext("Menu"), &labels, sel, Change::Content, move |p, i| {
            if let Some(t) = p.title_mut(id) {
                t.return_menu = pick(&menus, i);
            }
        }));
        page.add(&g);

        let g = group(&gettext("Chapters"));
        g.set_description(Some(&gettext("Add chapters at the playhead in the title view, or generate them at a fixed interval")));
        let interval = adw::SpinRow::with_range(1.0, 60.0, 1.0);
        interval.set_title(&gettext("Interval"));
        interval.set_subtitle(&gettext("Minutes between chapters"));
        interval.set_value(5.0);
        g.add(&interval);
        let apply = adw::ButtonRow::builder().title(gettext("Generate Chapters")).start_icon_name("view-list-bullet-symbolic").build();
        let d = doc.clone();
        let iv = interval.clone();
        let duration = p.asset(t.asset).map_or(0.0, |a| a.info.duration);
        apply.connect_activated(move |_| {
            let secs = iv.value() * 60.0;
            d.edit(Change::Structure, |p| {
                if let Some(t) = p.title_mut(id) {
                    t.chapters = auto_chapters(duration, secs);
                }
            });
        });
        g.add(&apply);
        let first = adw::ActionRow::builder().title(format!("{} 1", gettext("Chapter"))).subtitle("0:00.0").build();
        g.add(&first);
        for (i, c) in t.chapters.iter().enumerate() {
            let row = adw::ActionRow::builder()
                .title(format!("{} {}", gettext("Chapter"), i + 2))
                .subtitle(format_time(*c))
                .build();
            let del = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(gettext("Remove Chapter"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let d = doc.clone();
            let time = *c;
            del.connect_clicked(move |_| {
                d.edit(Change::Structure, |p| {
                    if let Some(t) = p.title_mut(id) {
                        t.chapters.retain(|x| *x != time);
                    }
                });
            });
            row.add_suffix(&del);
            g.add(&row);
        }
        page.add(&g);

        page.add(&delete_button(&gettext("Delete Title"), "win.delete-node"));
    }
}
