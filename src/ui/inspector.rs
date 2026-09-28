// SPDX-License-Identifier: GPL-3.0-or-later

//! Property inspector for the selected menu, menu item or title.

use super::rows::{self, format_time, group};
use crate::document::{Change, Document, Node};
use crate::bluray::VideoFormat;
use crate::media::picture::VideoOptions;
use crate::media::probe::MediaInfo;
use crate::model::*;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

pub struct Inspector {
    doc: Rc<Document>,
    container: gtk::Box,
    title: adw::WindowTitle,
    geometry: RefCell<Option<(Id, Id, [adw::SpinRow; 4])>>,
    rebuild_queued: Cell<bool>,
    /// The main text row of the current page (label/text/name).
    primary: RefCell<Option<gtk::Widget>>,
    /// What the page shows, to keep its scroll position when it's rebuilt
    /// for the same thing.
    shown: Cell<Option<(Node, Option<Id>)>>,
}

/// A row that copies this title's track choices to every other title.
fn apply_to_all_row(doc: &Rc<Document>, id: Id, apply: fn(&mut Project, Id) -> usize) -> adw::ButtonRow {
    let row = adw::ButtonRow::builder().title(gettext("Apply to All Titles")).start_icon_name("edit-copy-symbolic").build();
    row.set_tooltip_text(Some(&gettext("Tracks are matched by their stream number, or by language for separate files")));
    let doc = doc.clone();
    row.connect_activated(move |row| {
        let overlay = row.ancestor(adw::ToastOverlay::static_type()).and_downcast::<adw::ToastOverlay>();
        let mut n = 0;
        doc.edit(Change::Structure, |p| n = apply(p, id));
        let Some(overlay) = overlay else { return };
        let toast = if n == 0 {
            adw::Toast::new(&gettext("The other titles already match"))
        } else {
            let t = adw::Toast::new(&ngettext("Applied to {} title", "Applied to {} titles", n as u32).replace("{}", &n.to_string()));
            t.set_button_label(Some(&gettext("_Undo")));
            t.set_action_name(Some("win.undo"));
            t
        };
        overlay.add_toast(toast);
    });
    row
}

/// The scrolled window inside a page.
fn scroller(w: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    if let Some(s) = w.downcast_ref::<gtk::ScrolledWindow>() {
        return Some(s.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(s) = scroller(&c) {
            return Some(s);
        }
        child = c.next_sibling();
    }
    None
}

/// Scroll `page` to `pos` once it has been laid out tall enough.
fn restore_scroll(page: &adw::PreferencesPage, pos: f64) {
    let Some(adj) = scroller(page.upcast_ref()).map(|s| s.vadjustment()) else { return };
    let handler: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
    let h = handler.clone();
    let id = adj.connect_changed(move |adj| {
        if adj.upper() - adj.page_size() >= pos || adj.upper() > 0.0 {
            adj.set_value(pos);
            if adj.upper() - adj.page_size() >= pos {
                if let Some(id) = h.take() {
                    adj.disconnect(id);
                }
            }
        }
    });
    handler.set(Some(id));
    // Stop adjusting once the page has settled.
    let adj2 = adj.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
        if let Some(id) = handler.take() {
            adj2.disconnect(id);
        }
    });
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
            shown: Cell::new(None),
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
        let now = (self.doc.node(), self.doc.item());
        let scroll = match self.shown.replace(Some(now)) {
            Some(before) if before == now => self.container.first_child().and_then(|c| scroller(&c)).map(|s| s.vadjustment().value()),
            _ => None,
        };
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
        if let Some(pos) = scroll.filter(|p| *p > 0.0) {
            restore_scroll(&page, pos);
        }
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
        let shapes = [gettext("16:9 Widescreen"), gettext("4:3 Standard")];
        let shape = rows::combo(doc, &gettext("Shape"), &shapes, usize::from(m.shape == MenuShape::Standard), Change::Structure, move |p, i| {
            if let Some(m) = p.menu_mut(id) {
                m.shape = if i == 1 { MenuShape::Standard } else { MenuShape::Wide };
            }
        });
        shape.set_subtitle(&match (m.shape, p.disc.video.is_4x3()) {
            (MenuShape::Wide, true) => gettext("Shown letterboxed on this 4:3 disc"),
            (MenuShape::Wide, false) => gettext("For widescreen TVs"),
            (MenuShape::Standard, true) => gettext("Designed in the middle of the canvas, which fills 4:3 TVs"),
            (MenuShape::Standard, false) => gettext("On this 16:9 disc the whole canvas is shown, sides included"),
        });
        shape.set_subtitle_lines(2);
        g.add(&shape);
        page.add(&g);

        if m.popup {
            let g = group(&gettext("Pop-up"));
            g.set_description(Some(&gettext(
                "Shown over the playing title with the remote's Pop-up key. Only the menu's items appear; the video stays visible behind them.",
            )));
            let all = rows::switch(doc, &gettext("Use for All Titles"), p.disc.popup_menu == Some(id), Change::Structure, move |p, v| {
                if v {
                    p.disc.popup_menu = Some(id);
                } else if p.disc.popup_menu == Some(id) {
                    p.disc.popup_menu = None;
                }
            });
            all.set_subtitle(&gettext("Titles can choose a different pop-up menu in their settings"));
            g.add(&all);
            self.fade_rows(&g, id, m.fade_in, m.fade_out);
            page.add(&g);
            page.add(&delete_button(&gettext("Delete Pop-up Menu"), "win.delete-node"));
            return;
        }

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
        if let Some(a) = m.background.video.and_then(|v| p.asset(v)) {
            let start = rows::frame(doc, &gettext("Loop Starts At"), a, m.background.video_start, vec![], Change::Structure, move |p, t| {
                if let Some(m) = p.menu_mut(id) {
                    m.background.video_start = t;
                }
            });
            start.set_subtitle(&format!(
                "{} · {}",
                format_time(m.background.video_start),
                gettext("the loop lasts the menu's Loop Length")
            ));
            g.add(&start);
        }
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

        self.timing_group(page, id);
        page.add(&delete_button(&gettext("Delete Menu"), "win.delete-node"));
    }

    fn fade_rows(&self, g: &adw::PreferencesGroup, id: Id, fade_in: f64, fade_out: f64) {
        let doc = &self.doc;
        let row = rows::spin(doc, &gettext("Fade In"), fade_in, 0.0, 3.0, 0.1, 1, Change::Content, move |p, v| {
            if let Some(m) = p.menu_mut(id) {
                m.fade_in = v;
            }
        });
        row.set_subtitle(&gettext("Seconds for the buttons to appear"));
        g.add(&row);
        let row = rows::spin(doc, &gettext("Fade Out"), fade_out, 0.0, 3.0, 0.1, 1, Change::Content, move |p, v| {
            if let Some(m) = p.menu_mut(id) {
                m.fade_out = v;
            }
        });
        row.set_subtitle(&gettext("When switching to another page of a pop-up"));
        g.add(&row);
    }

    /// Intro video and timeout of a menu.
    fn timing_group(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(m) = p.menu(id) else { return };
        let g = group(&gettext("Timing"));
        self.fade_rows(&g, id, m.fade_in, m.fade_out);
        let videos = assets_of(&p, &[AssetKind::Video]);
        let (labels, sel) = optional_choice(&gettext("None"), &videos, m.intro);
        let intro = rows::combo(doc, &gettext("Intro Video"), &labels, sel, Change::Structure, move |p, i| {
            if let Some(m) = p.menu_mut(id) {
                m.intro = pick(&videos, i);
            }
        });
        intro.set_subtitle(&gettext("Plays before the menu appears"));
        g.add(&intro);
        if m.intro.is_some() {
            let every = rows::switch(doc, &gettext("Play Intro Every Time"), m.intro_every_time, Change::Content, move |p, v| {
                if let Some(m) = p.menu_mut(id) {
                    m.intro_every_time = v;
                }
            });
            every.set_subtitle(&gettext("Otherwise only the first time the menu is shown"));
            g.add(&every);
        }
        let timeout = rows::switch(doc, &gettext("Timeout"), m.timeout.is_some(), Change::Structure, move |p, v| {
            let first = p.titles.first().map(|t| Action::PlayTitle { title: t.id, chapter: 0 }).unwrap_or(Action::PlayAll);
            if let Some(m) = p.menu_mut(id) {
                m.timeout = v.then_some(MenuTimeout { seconds: 60, action: first });
            }
        });
        timeout.set_subtitle(&gettext("Do something by itself after a while on this menu"));
        g.add(&timeout);
        if let Some(t) = m.timeout {
            g.add(&rows::spin(doc, &gettext("Seconds"), t.seconds as f64, 5.0, 3600.0, 5.0, 0, Change::Content, move |p, v| {
                if let Some(t) = p.menu_mut(id).and_then(|m| m.timeout.as_mut()) {
                    t.seconds = v as u32;
                }
            }));
            let mut targets: Vec<(Action, String)> = vec![(Action::PlayAll, gettext("Play All Titles"))];
            for t in &p.titles {
                targets.push((Action::PlayTitle { title: t.id, chapter: 0 }, format!("{} {}", gettext("Play"), t.name)));
            }
            for mm in p.disc_menus().filter(|x| x.id != id) {
                targets.push((Action::ShowMenu(mm.id), format!("{} {}", gettext("Show"), mm.name)));
            }
            let sel = targets.iter().position(|(a, _)| *a == t.action).unwrap_or(0);
            let labels: Vec<String> = targets.iter().map(|(_, l)| l.clone()).collect();
            g.add(&rows::combo(doc, &gettext("Then"), &labels, sel, Change::Content, move |p, i| {
                if let (Some(t), Some((a, _))) = (p.menu_mut(id).and_then(|m| m.timeout.as_mut()), targets.get(i)) {
                    t.action = *a;
                }
            }));
        }
        page.add(&g);
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
        let outline = rows::spin(doc, &gettext("Outline"), style.outline, 0.0, 20.0, 0.5, 1, Change::Structure, move |p, v| {
            if let Some(s) = style_mut(p, menu, item) {
                s.outline = v;
            }
        });
        outline.set_subtitle(&gettext("Width in pixels"));
        g.add(&outline);
        if style.outline > 0.0 {
            g.add(&rows::color(doc, &gettext("Outline Color"), style.outline_color, Change::Content, move |p, c| {
                if let Some(s) = style_mut(p, menu, item) {
                    s.outline_color = c;
                }
            }));
        }
        g.add(&rows::spin(doc, &gettext("Letter Spacing"), style.letter_spacing, -10.0, 60.0, 0.5, 1, Change::Content, move |p, v| {
            if let Some(s) = style_mut(p, menu, item) {
                s.letter_spacing = v;
            }
        }));
        g.add(&rows::spin(doc, &gettext("Line Spacing"), style.line_spacing, 0.6, 3.0, 0.05, 2, Change::Content, move |p, v| {
            if let Some(s) = style_mut(p, menu, item) {
                s.line_spacing = v;
            }
        }));
        let glow = rows::spin(doc, &gettext("Glow"), style.glow, 0.0, 40.0, 1.0, 0, Change::Structure, move |p, v| {
            if let Some(s) = style_mut(p, menu, item) {
                s.glow = v;
            }
        });
        glow.set_subtitle(&gettext("Size in pixels"));
        g.add(&glow);
        if style.glow > 0.0 {
            g.add(&rows::color(doc, &gettext("Glow Color"), style.glow_color, Change::Content, move |p, c| {
                if let Some(s) = style_mut(p, menu, item) {
                    s.glow_color = c;
                }
            }));
        }
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
                let in_popup = p.menu(menu).is_some_and(|m| m.popup);
                let mut targets: Vec<(Action, String)> =
                    vec![(Action::None, gettext("Do Nothing")), (Action::PlayAll, gettext("Play All Titles"))];
                if in_popup {
                    targets.push((Action::PlayChapter(0), gettext("Go to Chapter")));
                }
                for t in &p.titles {
                    targets.push((Action::PlayTitle { title: t.id, chapter: 0 }, format!("{} {}", gettext("Play"), t.name)));
                }
                // Pop-ups can only be opened from pop-ups.
                for mm in p.menus.iter().filter(|x| x.id != menu && (in_popup || !x.popup)) {
                    targets.push((Action::ShowMenu(mm.id), format!("{} {}", gettext("Show"), mm.name)));
                }
                // Language choices return to the first menu by default.
                let home = p.first_menu().map(|m| m.id).filter(|m| *m != menu);
                for l in &p.disc.languages {
                    targets.push((Action::SetLanguage { preset: l.id, menu: home }, gettext("Set Language: {}").replace("{}", &l.name)));
                }
                let sel = targets
                    .iter()
                    .position(|(a, _)| match (a, b.action) {
                        (Action::PlayTitle { title: x, .. }, Action::PlayTitle { title: y, .. }) => *x == y,
                        (Action::SetLanguage { preset: x, .. }, Action::SetLanguage { preset: y, .. }) => *x == y,
                        (Action::PlayChapter(_), Action::PlayChapter(_)) => true,
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
                if let Action::PlayChapter(chapter) = b.action {
                    let max = p.titles.iter().map(|t| t.chapters.len() + 1).max().unwrap_or(1).max(chapter as usize + 1);
                    let row = rows::spin(doc, &gettext("Chapter"), chapter as f64 + 1.0, 1.0, max.max(99) as f64, 1.0, 0, Change::Content, move |p, v| {
                        if let Some(b) = button_mut(p, menu, item) {
                            b.action = Action::PlayChapter((v as u32).saturating_sub(1));
                        }
                    });
                    row.set_subtitle(&gettext("Of the playing title; hidden in titles with fewer chapters"));
                    g.add(&row);
                }
                if let Action::SetLanguage { menu: then, .. } = b.action {
                    let menus: Vec<(Id, String)> =
                        p.menus.iter().filter(|m| m.id != menu && (in_popup || !m.popup)).map(|m| (m.id, m.name.clone())).collect();
                    let (labels, sel) = optional_choice(&gettext("Stay on This Menu"), &menus, then);
                    let row = rows::combo(doc, &gettext("Then Show"), &labels, sel, Change::Structure, move |p, i| {
                        if let Some(b) = button_mut(p, menu, item) {
                            if let Action::SetLanguage { preset, .. } = b.action {
                                b.action = Action::SetLanguage { preset, menu: pick(&menus, i) };
                            }
                        }
                    });
                    g.add(&row);
                }
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
                const STYLES: [Highlight; 5] = [Highlight::Frame, Highlight::Text, Highlight::Underline, Highlight::Fill, Highlight::Arrow];
                let highlights = [gettext("Frame"), gettext("Text Color"), gettext("Underline"), gettext("Filled"), gettext("Arrow")];
                let sel = STYLES.iter().position(|h| *h == b.highlight).unwrap_or(0);
                g.add(&rows::combo(doc, &gettext("Highlight Style"), &highlights, sel, Change::Content, move |p, i| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.highlight = STYLES[i.min(STYLES.len() - 1)];
                    }
                }));
                let ht = rows::switch(doc, &gettext("Own Highlighted Text Color"), b.highlight_text.is_some(), Change::Structure, move |p, v| {
                    if let Some(b) = button_mut(p, menu, item) {
                        b.highlight_text = v.then_some(Rgba::WHITE);
                    }
                });
                ht.set_subtitle(&gettext("Otherwise the label takes the highlight color (or keeps its color when filled)"));
                g.add(&ht);
                if let Some(c) = b.highlight_text {
                    g.add(&rows::color(doc, &gettext("Highlighted Text Color"), c, Change::Content, move |p, c| {
                        if let Some(b) = button_mut(p, menu, item) {
                            b.highlight_text = Some(c);
                        }
                    }));
                }
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
                // Copy this look to other buttons (for consistent templates).
                let look = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).margin_top(12).build();
                for (label, whole_disc) in [(gettext("Apply to Menu"), false), (gettext("Apply to All Menus"), true)] {
                    let btn = gtk::Button::builder().label(label.as_str()).css_classes(["pill", "small"]).build();
                    btn.set_tooltip_text(Some(&if whole_disc {
                        gettext("Give every button on the disc this font, colors and highlight")
                    } else {
                        gettext("Give every button on this menu this font, colors and highlight")
                    }));
                    let d = doc.clone();
                    btn.connect_clicked(move |_| {
                        let from = d.project().menu(menu).and_then(|m| m.item(item)).and_then(|i| i.button().cloned());
                        if let Some(from) = from {
                            d.edit(Change::Content, |p| p.apply_button_look(&from, (!whole_disc).then_some(menu)));
                        }
                    });
                    look.append(&btn);
                }
                g.add(&look);
                page.add(&g);

                let g = group(&gettext("Pictures"));
                g.set_description(Some(&gettext("Artwork for the button in each state, e.g. exported from an image editor. Import pictures into Media first.")));
                let pictures = assets_of(&p, &[AssetKind::Image]);
                type Slot = fn(&mut StateImages) -> &mut Option<Id>;
                let slots: [(String, Option<Id>, Slot); 3] = [
                    (gettext("Normal"), b.images.normal, |s| &mut s.normal),
                    (gettext("Selected"), b.images.selected, |s| &mut s.selected),
                    (gettext("Activated"), b.images.activated, |s| &mut s.activated),
                ];
                for (title, current, slot) in slots {
                    let pictures = pictures.clone();
                    let (labels, sel) = optional_choice(&gettext("None"), &pictures, current);
                    g.add(&rows::combo(doc, &title, &labels, sel, Change::Content, move |p, i| {
                        if let Some(b) = button_mut(p, menu, item) {
                            *slot(&mut b.images) = pick(&pictures, i);
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
                g.set_description(Some(&gettext("Which button is selected when pressing the arrow keys. On the canvas, Alt+drag from this button to another to set one.")));
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
                fn image_mut(p: &mut Project, menu: Id, item: Id) -> Option<&mut ImageItem> {
                    match &mut p.menu_mut(menu)?.item_mut(item)?.kind {
                        ItemKind::Image(i) => Some(i),
                        _ => None,
                    }
                }
                let op = rows::spin(doc, &gettext("Opacity"), img.opacity * 100.0, 0.0, 100.0, 5.0, 0, Change::Content, move |p, v| {
                    if let Some(i) = image_mut(p, menu, item) {
                        i.opacity = v / 100.0;
                    }
                });
                op.set_subtitle(&gettext("Percent"));
                g.add(&op);
                g.add(&rows::spin(doc, &gettext("Corner Radius"), img.radius, 0.0, 400.0, 2.0, 0, Change::Content, move |p, v| {
                    if let Some(i) = image_mut(p, menu, item) {
                        i.radius = v;
                    }
                }));
                g.add(&rows::switch(doc, &gettext("Drop Shadow"), img.shadow, Change::Content, move |p, v| {
                    if let Some(i) = image_mut(p, menu, item) {
                        i.shadow = v;
                    }
                }));
                for (k, label) in [gettext("Crop Left"), gettext("Crop Top"), gettext("Crop Right"), gettext("Crop Bottom")].into_iter().enumerate() {
                    let row = rows::spin(doc, &label, img.crop[k] * 100.0, 0.0, 45.0, 1.0, 0, Change::Content, move |p, v| {
                        if let Some(i) = image_mut(p, menu, item) {
                            i.crop[k] = v / 100.0;
                        }
                    });
                    row.set_subtitle(&gettext("Percent"));
                    g.add(&row);
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
                g.add(&rows::switch(doc, &gettext("Gradient"), sh.gradient.is_some(), Change::Structure, move |p, v| {
                    if let Some(s) = shape_mut(p, menu, item) {
                        let c = s.fill;
                        s.gradient = v.then(|| Rgba::new(c.r * 0.4, c.g * 0.4, c.b * 0.4, c.a));
                    }
                }));
                if let Some(end) = sh.gradient {
                    g.add(&rows::color(doc, &if sh.horizontal { gettext("Right Color") } else { gettext("Bottom Color") }, end, Change::Content, move |p, c| {
                        if let Some(s) = shape_mut(p, menu, item) {
                            s.gradient = Some(c);
                        }
                    }));
                    g.add(&rows::switch(doc, &gettext("Left to Right"), sh.horizontal, Change::Structure, move |p, v| {
                        if let Some(s) = shape_mut(p, menu, item) {
                            s.horizontal = v;
                        }
                    }));
                }
                g.add(&rows::switch(doc, &gettext("Ellipse"), sh.ellipse, Change::Structure, move |p, v| {
                    if let Some(s) = shape_mut(p, menu, item) {
                        s.ellipse = v;
                    }
                }));
                if !sh.ellipse {
                    g.add(&rows::spin(doc, &gettext("Corner Radius"), sh.radius, 0.0, 540.0, 2.0, 0, Change::Content, move |p, v| {
                        if let Some(s) = shape_mut(p, menu, item) {
                            s.radius = v;
                        }
                    }));
                }
                g.add(&rows::spin(doc, &gettext("Border"), sh.stroke, 0.0, 40.0, 0.5, 1, Change::Structure, move |p, v| {
                    if let Some(s) = shape_mut(p, menu, item) {
                        s.stroke = v;
                    }
                }));
                if sh.stroke > 0.0 {
                    g.add(&rows::color(doc, &gettext("Border Color"), sh.stroke_color, Change::Content, move |p, c| {
                        if let Some(s) = shape_mut(p, menu, item) {
                            s.stroke_color = c;
                        }
                    }));
                }
                page.add(&g);
            }
        }
        self.geometry_group(page, menu, item, it.rect);
        let g = group(&gettext("Layer"));
        let hidden = rows::switch(doc, &gettext("Hidden"), it.hidden, Change::Structure, move |p, v| {
            if let Some(i) = p.menu_mut(menu).and_then(|m| m.item_mut(item)) {
                i.hidden = v;
            }
        });
        hidden.set_subtitle(&gettext("Left out of the menu, but kept for later"));
        g.add(&hidden);
        let locked = rows::switch(doc, &gettext("Locked"), it.locked, Change::Structure, move |p, v| {
            if let Some(i) = p.menu_mut(menu).and_then(|m| m.item_mut(item)) {
                i.locked = v;
            }
        });
        locked.set_subtitle(&gettext("Can't be clicked or moved on the canvas"));
        g.add(&locked);
        let order = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).margin_top(6).build();
        for (icon, tip, action) in [
            ("go-bottom-symbolic", gettext("Send to Back"), "win.lower-item"),
            ("go-down-symbolic", gettext("Send Backward"), "win.backward-item"),
            ("go-up-symbolic", gettext("Bring Forward"), "win.forward-item"),
            ("go-top-symbolic", gettext("Bring to Front"), "win.raise-item"),
        ] {
            let b = gtk::Button::builder().icon_name(icon).tooltip_text(tip.as_str()).action_name(action).css_classes(["flat"]).build();
            b.update_property(&[gtk::accessible::Property::Label(&tip)]);
            order.append(&b);
        }
        g.add(&order);
        page.add(&g);
        page.add(&delete_button(&gettext("Delete"), "win.delete-item"));
    }

    /// Keep the original video (no re-encoding) when it's compatible.
    fn video_group(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(t) = p.title(id) else { return };
        let g = group(&gettext("Video"));
        let compatible = t.video_check.as_ref().is_some_and(|r| r.compatible());
        let keep = rows::switch(doc, &gettext("Keep Original Video"), t.keep_video, Change::Structure, move |p, v| {
            if let Some(t) = p.title_mut(id) {
                t.keep_video = v;
            }
        });
        keep.set_sensitive(compatible || t.keep_video);
        let status = match &t.video_check {
            None => gettext("Check compatibility first. Otherwise the video is re-encoded using Disc Settings."),
            Some(r) if r.compatible() && t.keep_video => {
                format!("{} · {}", r.summary(), gettext("no re-encoding, Disc Settings video options don't apply"))
            }
            Some(r) => r.summary(),
        };
        keep.set_subtitle(&status);
        g.add(&keep);
        let check = adw::ButtonRow::builder()
            .title(if t.video_check.is_some() { gettext("Check Again…") } else { gettext("Check Compatibility…") })
            .start_icon_name("object-select-symbolic")
            .action_name("win.check-video")
            .build();
        g.add(&check);
        page.add(&g);
        if let Some(a) = p.asset(t.asset) {
            self.picture_group(page, id, &a.info, a.path.clone(), t.video, t.keep_video, p.disc.video);
        }
    }

    /// How the picture is prepared: deinterlacing, aspect ratio, fit, crop.
    #[allow(clippy::too_many_arguments)]
    fn picture_group(&self, page: &adw::PreferencesPage, id: Id, info: &MediaInfo, path: PathBuf, opts: VideoOptions, keep: bool, disc: VideoFormat) {
        use crate::media::picture::{self, AspectRatio, Deinterlace, Fit};
        let doc = &self.doc;
        let g = group(&gettext("Picture"));
        g.set_description(Some(&picture::describe(info)));
        g.set_sensitive(!keep);
        let edit = |f: fn(&mut VideoOptions, usize)| {
            move |p: &mut Project, i: usize| {
                if let Some(t) = p.title_mut(id) {
                    f(&mut t.video, i);
                }
            }
        };
        let plan = picture::plan(info, &opts, disc);

        let labels = [gettext("Auto"), gettext("Off"), gettext("Always"), gettext("Film")];
        let sel = Deinterlace::ALL.iter().position(|d| *d == opts.deinterlace).unwrap_or(0);
        let row = rows::combo(doc, &gettext("Deinterlace"), &labels, sel, Change::Content, edit(|v, i| v.deinterlace = Deinterlace::ALL[i.min(3)]));
        row.set_subtitle(&match opts.deinterlace {
            Deinterlace::InverseTelecine => gettext("Inverse telecine: restores film frames in 29.97 video (NTSC DVDs)"),
            _ if plan.interlaced.is_some() => gettext("Kept interlaced, as the disc format allows"),
            _ if plan.deinterlaced => gettext("The video is deinterlaced"),
            Deinterlace::Auto if info.interlaced().is_none() => gettext("The file doesn't say whether it's interlaced; treated as progressive"),
            _ => gettext("Nothing to do: the video is progressive"),
        });
        row.set_subtitle_lines(2);
        g.add(&row);

        let auto = format!("{} {:.2}", gettext("Auto"), info.display_aspect());
        let labels = [auto, "4:3".into(), "16:9".into(), "1.85:1".into(), "2.39:1".into()];
        let sel = AspectRatio::ALL.iter().position(|a| *a == opts.aspect).unwrap_or(0);
        g.add(&rows::combo(doc, &gettext("Aspect Ratio"), &labels, sel, Change::Content, edit(|v, i| v.aspect = AspectRatio::ALL[i.min(4)])));

        let labels = [gettext("Whole"), gettext("Fill"), gettext("Stretch")];
        let sel = Fit::ALL.iter().position(|f| *f == opts.fit).unwrap_or(0);
        let row = rows::combo(doc, &gettext("Fit"), &labels, sel, Change::Content, edit(|v, i| v.fit = Fit::ALL[i.min(2)]));
        row.set_subtitle_lines(2);
        row.set_subtitle(&match opts.fit {
            Fit::Whole => gettext("The whole picture, with black bars if it isn't 16:9"),
            Fit::Fill => gettext("Fills the screen, cutting off edges that don't fit"),
            Fit::Stretch => gettext("Fills the screen by stretching the picture"),
        });
        g.add(&row);

        // Crop, in source pixels.
        let [l, t, r, b] = opts.crop;
        let crop = adw::ExpanderRow::builder().title(gettext("Crop")).build();
        crop.set_subtitle(&if l + t + r + b == 0 {
            gettext("None")
        } else {
            gettext("{l} left, {t} top, {r} right, {b} bottom")
                .replace("{l}", &l.to_string())
                .replace("{t}", &t.to_string())
                .replace("{r}", &r.to_string())
                .replace("{b}", &b.to_string())
        });
        let sides = [gettext("Left"), gettext("Top"), gettext("Right"), gettext("Bottom")];
        for (k, name) in sides.iter().enumerate() {
            let max = if k % 2 == 0 { info.width / 2 } else { info.height / 2 };
            let row = rows::spin(doc, name, opts.crop[k] as f64, 0.0, max.max(2) as f64, 2.0, 0, Change::Content, move |p, v| {
                if let Some(t) = p.title_mut(id) {
                    t.video.crop[k] = (v.round() as u32) & !1;
                }
            });
            crop.add_row(&row);
        }
        let detect = adw::ButtonRow::builder().title(gettext("Detect Black Bars")).start_icon_name("edit-find-symbolic").build();
        {
            let (doc, info) = (doc.clone(), info.clone());
            detect.connect_activated(move |row| {
                row.set_sensitive(false);
                row.set_title(&gettext("Looking for Black Bars…"));
                let (tx, rx) = async_channel::bounded(1);
                let (path, info) = (path.clone(), info.clone());
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(picture::detect_crop(&path, &info).map_err(|e| e.to_string()));
                });
                let (doc, row) = (doc.clone(), row.clone());
                glib::spawn_future_local(async move {
                    match rx.recv().await {
                        Ok(Ok(c)) => {
                            doc.edit(Change::Content, |p| {
                                if let Some(t) = p.title_mut(id) {
                                    t.video.crop = c;
                                }
                            });
                            if c == [0; 4] {
                                row.set_title(&gettext("No Black Bars Found"));
                            }
                        }
                        Ok(Err(e)) => row.set_title(&e),
                        Err(_) => {}
                    }
                    row.set_sensitive(true);
                });
            });
        }
        crop.add_row(&detect);
        g.add(&crop);

        if plan.tone_mapped {
            let hdr = adw::ActionRow::builder()
                .title(gettext("HDR Video"))
                .subtitle(gettext("Blu-ray is standard range (SDR): the picture is tone mapped, so bright highlights and colours look less intense than the HDR original."))
                .subtitle_lines(4)
                .build();
            hdr.add_prefix(&gtk::Image::builder().icon_name("dialog-warning-symbolic").css_classes(["warning"]).build());
            g.add(&hdr);
        }
        page.add(&g);
    }

    fn audio_group(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(t) = p.title(id) else { return };
        let g = group(&gettext("Audio"));
        g.set_description(Some(&gettext("The first track plays by default; viewers switch with the remote. Audio Blu-ray players decode (AC-3, DTS, DTS-HD, TrueHD) is copied without re-encoding.")));

        // What each track becomes on the disc.
        let set = crate::media::transcode::EncodeSettings::for_disc(&p.disc);
        let on_disc: std::collections::HashMap<Id, (crate::media::transcode::AudioInput, crate::bluray::EsInfo)> =
            t.disc_audio().map(|a| a.id).zip(crate::build::title_audio(&p, t, &set, 0.0).unwrap_or_default()).collect();

        fn track_mut(p: &mut Project, title: Id, track: Id) -> Option<&mut AudioTrack> {
            p.title_mut(title)?.audio.iter_mut().find(|a| a.id == track)
        }

        let own = p.asset(t.asset).map(|a| a.info.audio()).unwrap_or_default();
        let first_enabled = t.audio.iter().find(|a| a.enabled).map(|a| a.id);
        for (i, track) in t.audio.iter().enumerate() {
            let tid = track.id;
            let (stream, origin) = match track.source {
                AudioSource::Embedded { index } => (own.iter().find(|s| s.index == index).cloned(), gettext("in video")),
                AudioSource::External { asset, .. } => {
                    let a = p.asset(asset);
                    (a.and_then(|a| a.info.audio().into_iter().next()), a.map(|a| a.name()).unwrap_or_default())
                }
            };
            let mut subtitle = format!("{} · {} · {}", language_name(&track.lang), stream.as_ref().map(describe).unwrap_or_default(), origin);
            if first_enabled == Some(tid) {
                subtitle.push_str(&format!(" · {}", gettext("default")));
            }
            let result = on_disc.get(&tid).map(|(input, es)| {
                let codec = match es.kind {
                    crate::bluray::EsKind::Audio { codec, .. } => codec.label(),
                    _ => "",
                };
                let layout = match input.output_channels() {
                    1 => gettext("mono"),
                    2 => gettext("stereo"),
                    6 => "5.1".to_string(),
                    8 => "7.1".to_string(),
                    n => format!("{n} ch"),
                };
                if input.copy.is_some() {
                    gettext("On the disc as it is: {}").replace("{}", &format!("{codec} {layout}"))
                } else {
                    gettext("Encoded as {}").replace("{}", &format!("{codec} {layout}"))
                }
            });
            let row = adw::ExpanderRow::builder()
                .title(glib::markup_escape_text(&track.name).as_str())
                .subtitle(glib::markup_escape_text(&subtitle).as_str())
                .show_enable_switch(true)
                .enable_expansion(track.enabled)
                .build();
            if i > 0 {
                let up = gtk::Button::builder()
                    .icon_name("go-up-symbolic")
                    .tooltip_text(gettext("Move Up"))
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                up.update_property(&[gtk::accessible::Property::Label(&gettext("Move Up"))]);
                let d = doc.clone();
                up.connect_clicked(move |_| {
                    d.edit(Change::Structure, |p| {
                        if let Some(t) = p.title_mut(id) {
                            if let Some(i) = t.audio.iter().position(|a| a.id == tid).filter(|i| *i > 0) {
                                t.audio.swap(i, i - 1);
                            }
                        }
                    });
                });
                row.add_suffix(&up);
            }
            let d = doc.clone();
            row.connect_enable_expansion_notify(move |r| {
                let on = r.enables_expansion();
                if d.project().title(id).and_then(|t| t.audio.iter().find(|a| a.id == tid)).is_some_and(|a| a.enabled == on) {
                    return;
                }
                d.edit(Change::Structure, |p| {
                    if let Some(a) = track_mut(p, id, tid) {
                        a.enabled = on;
                    }
                });
            });
            let lang = rows::entry(doc, &gettext("Language Code"), &track.lang, Change::Content, move |p, v| {
                if let Some(a) = track_mut(p, id, tid) {
                    a.lang = normalize_lang(&v);
                }
            });
            lang.set_tooltip_text(Some(&gettext("ISO 639 code such as eng, fra or jpn")));
            if let Some(result) = &result {
                let info = adw::ActionRow::builder().title(gettext("On the Disc")).subtitle(result).build();
                info.add_css_class("property");
                row.add_row(&info);
            }
            row.add_row(&lang);
            row.add_row(&rows::entry(doc, &gettext("Name"), &track.name, Change::Content, move |p, v| {
                if let Some(a) = track_mut(p, id, tid) {
                    a.name = v;
                }
            }));
            let labels = [gettext("Original"), gettext("Mono"), gettext("Stereo"), gettext("5.1 Surround")];
            let sel = ChannelLayout::ALL.iter().position(|l| *l == track.layout).unwrap_or(0);
            let channels = rows::combo(doc, &gettext("Channels"), &labels, sel, Change::Content, move |p, i| {
                if let Some(a) = track_mut(p, id, tid) {
                    a.layout = ChannelLayout::ALL[i.min(3)];
                }
            });
            channels.set_subtitle(&match track.layout {
                ChannelLayout::Original => gettext("7.1 is mixed down to 5.1 when encoding"),
                ChannelLayout::Surround if stream.as_ref().is_some_and(|s| s.channels < 6) => gettext("Spreads the sound over the surround speakers"),
                _ => gettext("Mixed to this layout"),
            });
            row.add_row(&channels);
            if stream.as_ref().and_then(|s| s.bluray_codec()).is_some() {
                let keep = rows::switch(doc, &gettext("Keep Original Audio"), !track.reencode, Change::Content, move |p, v| {
                    if let Some(a) = track_mut(p, id, tid) {
                        a.reencode = !v;
                    }
                });
                keep.set_subtitle(&gettext("Blu-ray players decode this format, so it can go on the disc as it is"));
                keep.set_subtitle_lines(2);
                row.add_row(&keep);
            }
            if let AudioSource::External { offset, .. } = track.source {
                let delay = rows::spin(doc, &gettext("Delay"), offset, -600.0, 600.0, 0.1, 2, Change::Content, move |p, v| {
                    if let Some(a) = track_mut(p, id, tid) {
                        if let AudioSource::External { offset, .. } = &mut a.source {
                            *offset = v;
                        }
                    }
                });
                delay.set_subtitle(&gettext("Seconds; negative starts the audio file earlier"));
                row.add_row(&delay);
                let remove = adw::ButtonRow::builder().title(gettext("Remove Audio Track")).css_classes(["destructive-action"]).build();
                let d = doc.clone();
                remove.connect_activated(move |_| {
                    d.edit(Change::Structure, |p| {
                        if let Some(t) = p.title_mut(id) {
                            t.audio.retain(|a| a.id != tid);
                        }
                    });
                });
                row.add_row(&remove);
            }
            g.add(&row);
        }
        if t.audio.is_empty() {
            g.add(&adw::ActionRow::builder().title(gettext("No audio")).css_classes(["dim-label"]).build());
        }
        let add = adw::ButtonRow::builder().title(gettext("Add Audio File…")).start_icon_name("list-add-symbolic").build();
        add.set_action_name(Some("win.add-audio"));
        g.add(&add);
        if p.titles.len() > 1 && !t.audio.is_empty() {
            g.add(&apply_to_all_row(doc, id, Project::apply_audio_to_all));
        }
        page.add(&g);
    }

    /// Tracks each language preset picks in this title.
    fn languages_group(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(t) = p.title(id) else { return };
        if p.disc.languages.is_empty() {
            return;
        }
        let g = group(&gettext("Language Setup"));
        g.set_description(Some(&gettext("What each language choice from Disc Settings plays in this title")));
        let audio: Vec<(Id, String)> = t.disc_audio().map(|a| (a.id, format!("{} ({})", a.name, language_name(&a.lang)))).collect();
        let subs: Vec<(Id, String)> = t.disc_subtitles().map(|s| (s.id, format!("{} ({})", s.name, language_name(&s.lang)))).collect();
        let name_of = |list: &[(Id, String)], id: Option<Id>, none: String| id.and_then(|id| list.iter().find(|(x, _)| *x == id)).map_or(none, |(_, n)| n.clone());

        fn picks_mut(p: &mut Project, title: Id, preset: Id) -> Option<&mut LanguageTracks> {
            let t = p.title_mut(title)?;
            if !t.language_tracks.iter().any(|l| l.preset == preset) {
                t.language_tracks.push(LanguageTracks { preset, audio: TrackPick::Auto, subtitle: TrackPick::Auto });
            }
            t.language_tracks.iter_mut().find(|l| l.preset == preset)
        }

        for preset in &p.disc.languages {
            let pid = preset.id;
            let r = t.resolve_language(preset);
            let current = t.language_tracks.iter().find(|l| l.preset == pid).cloned();
            let (a_name, s_name) = (name_of(&audio, r.audio, gettext("Unchanged")), name_of(&subs, r.subtitle, gettext("Off")));
            let row = adw::ExpanderRow::builder()
                .title(glib::markup_escape_text(&preset.name).as_str())
                .subtitle(glib::markup_escape_text(&format!("{a_name} · {s_name}")).as_str())
                .build();

            // Combo entries: Automatic, then (for subtitles) Off, then tracks.
            let choice_row = |title: String, list: &[(Id, String)], with_off: bool, pick: TrackPick, auto: String, set: fn(&mut LanguageTracks, TrackPick)| {
                let mut labels = vec![gettext("Automatic ({})").replace("{}", &auto)];
                let mut values = vec![TrackPick::Auto];
                if with_off {
                    labels.push(gettext("Off"));
                    values.push(TrackPick::Off);
                }
                labels.extend(list.iter().map(|(_, n)| n.clone()));
                values.extend(list.iter().map(|(id, _)| TrackPick::Track(*id)));
                let sel = values.iter().position(|v| *v == pick).unwrap_or(0);
                rows::combo(doc, &title, &labels, sel, Change::Structure, move |p, i| {
                    if let Some(l) = picks_mut(p, id, pid) {
                        set(l, values.get(i).copied().unwrap_or_default());
                    }
                })
            };
            let auto_preset = LanguagePreset { id: new_id(), ..preset.clone() };
            let auto = t.resolve_language(&auto_preset);
            row.add_row(&choice_row(
                gettext("Audio"),
                &audio,
                false,
                current.as_ref().map_or(TrackPick::Auto, |c| c.audio),
                name_of(&audio, auto.audio, gettext("unchanged")),
                |l, v| l.audio = v,
            ));
            row.add_row(&choice_row(
                gettext("Subtitles"),
                &subs,
                true,
                current.as_ref().map_or(TrackPick::Auto, |c| c.subtitle),
                name_of(&subs, auto.subtitle, gettext("off")),
                |l, v| l.subtitle = v,
            ));
            g.add(&row);
        }
        page.add(&g);
    }

    fn subtitles_group(&self, page: &adw::PreferencesPage, id: Id) {
        let doc = &self.doc;
        let p = doc.project();
        let Some(t) = p.title(id) else { return };
        let g = group(&gettext("Subtitles"));
        g.set_description(Some(&gettext("Converted to Blu-ray subtitles. Text subtitles use the style in Disc Settings; ASS keeps its own styling. A text track can instead be burned into the picture.")));

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
            let burned = t.burned_subtitle().is_some_and(|b| b.id == tid);
            if !supported {
                subtitle = format!("{} · {}", codec_label(&track.codec), gettext("not supported yet"));
            } else if burned {
                subtitle.push_str(&format!(" · {}", gettext("burned in")));
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
            forced.set_visible(!burned);
            row.add_row(&forced);
            if track.can_burn_in() {
                let burn = rows::switch(doc, &gettext("Burn Into the Picture"), track.burn_in, Change::Structure, move |p, v| {
                    if let Some(t) = p.title_mut(id) {
                        // Only one track can be burned in.
                        for s in &mut t.subtitles {
                            s.burn_in = v && s.id == tid;
                        }
                        if v && t.default_subtitle == Some(tid) {
                            t.default_subtitle = None;
                        }
                    }
                });
                burn.set_subtitle(&if t.keep_video {
                    gettext("Needs the video re-encoded: turn off Keep Original Video")
                } else {
                    gettext("Always shown and can't be turned off; every ASS style and effect looks exactly as made")
                });
                burn.set_subtitle_lines(3);
                row.add_row(&burn);
            }
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
            row.set_subtitle(&if p.disc.languages.is_empty() {
                gettext("Viewers can switch with the remote")
            } else {
                gettext("Only when the disc's language setting doesn't choose")
            });
            g.add(&row);
        } else if t.subtitles.is_empty() {
            let empty = adw::ActionRow::builder().title(gettext("No subtitles")).css_classes(["dim-label"]).build();
            g.add(&empty);
        }

        let add = adw::ButtonRow::builder().title(gettext("Add Subtitle File…")).start_icon_name("list-add-symbolic").build();
        add.set_action_name(Some("win.add-subtitle"));
        g.add(&add);
        if p.titles.len() > 1 && !t.subtitles.is_empty() {
            g.add(&apply_to_all_row(doc, id, Project::apply_subtitles_to_all));
        }
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

        self.video_group(page, id);
        self.audio_group(page, id);
        self.subtitles_group(page, id);
        self.languages_group(page, id);

        // Pop-up menu during playback.
        let popups: Vec<(Id, String)> = p.popup_menus().map(|m| (m.id, m.name.clone())).collect();
        if !popups.is_empty() {
            let g = group(&gettext("Pop-up Menu"));
            let default = p.disc.popup_menu.and_then(|d| p.menu(d)).map_or_else(|| gettext("none"), |m| m.name.clone());
            let mut labels = vec![gettext("Disc Default ({})").replace("{}", &default), gettext("None")];
            labels.extend(popups.iter().map(|(_, n)| n.clone()));
            let sel = match t.popup {
                PopupChoice::Default => 0,
                PopupChoice::None => 1,
                PopupChoice::Menu(m) => popups.iter().position(|(id, _)| *id == m).map_or(0, |i| i + 2),
            };
            let row = rows::combo(doc, &gettext("Menu"), &labels, sel, Change::Structure, move |p, i| {
                if let Some(t) = p.title_mut(id) {
                    t.popup = match i {
                        0 => PopupChoice::Default,
                        1 => PopupChoice::None,
                        i => popups.get(i - 2).map_or(PopupChoice::Default, |(m, _)| PopupChoice::Menu(*m)),
                    };
                }
            });
            row.set_subtitle(&gettext("Opens with the remote's Pop-up key"));
            g.add(&row);
            page.add(&g);
        }

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
        let menus: Vec<(Id, String)> = p.disc_menus().map(|m| (m.id, m.name.clone())).collect();
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
