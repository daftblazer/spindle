// SPDX-License-Identifier: GPL-3.0-or-later

//! Template chooser: pick a layout and theme, preview every generated menu
//! and apply it to the project.

use crate::document::{Change, Document, Node};
use crate::model::{Menu, Project};
use crate::render::{self, ImageCache};
use crate::templates::{self, Layout, Options, THEMES};
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{cairo, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct State {
    doc: Rc<Document>,
    images: Rc<ImageCache>,
    layout: Cell<Layout>,
    theme: Cell<usize>,
    title: RefCell<String>,
    logo: Cell<Option<crate::model::Id>>,
    layout_pictures: Vec<(Layout, gtk::Picture)>,
    pages: gtk::FlowBox,
    summary: gtk::Label,
}

impl State {
    fn options(&self, layout: Layout) -> Options {
        Options { layout, theme: self.theme.get(), title: self.title.borrow().clone(), logo: self.logo.get() }
    }

    fn generate(&self, layout: Layout) -> Project {
        let mut p = self.doc.project().clone();
        templates::apply(&mut p, &self.options(layout));
        p
    }

    fn refresh(&self) {
        for (layout, pic) in &self.layout_pictures {
            let p = self.generate(*layout);
            if let Some(m) = p.menus.first() {
                pic.set_paintable(render::menu_thumbnail(&p, m, &self.images, 480).as_ref());
            }
        }
        // All menus of the selected layout
        self.pages.remove_all();
        let p = self.generate(self.layout.get());
        for m in &p.menus {
            self.pages.append(&page_card(&p, m, &self.images));
        }
        let n = p.menus.len() as u32;
        self.summary.set_label(&ngettext("Creates {} menu", "Creates {} menus", n).replace("{}", &n.to_string()));
    }
}

fn page_card(p: &Project, m: &Menu, images: &ImageCache) -> gtk::Widget {
    let pic = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .width_request(192)
        .height_request(108)
        .css_classes(["template-page"])
        .build();
    pic.set_paintable(render::menu_thumbnail(p, m, images, 384).as_ref());
    let v = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    v.append(&pic);
    v.append(&gtk::Label::builder().label(&m.name).css_classes(["caption"]).ellipsize(gtk::pango::EllipsizeMode::End).build());
    adw::Clamp::builder().maximum_size(192).child(&v).build().upcast()
}

/// Round gradient swatch for a theme.
fn swatch(theme: usize) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(28).content_height(28).valign(gtk::Align::Center).build();
    area.set_draw_func(move |_, cr, w, h| {
        let t = &THEMES[theme];
        let (w, h) = (w as f64, h as f64);
        let g = cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
        g.add_color_stop_rgb(0.0, t.top.r as f64, t.top.g as f64, t.top.b as f64);
        g.add_color_stop_rgb(1.0, t.bottom.r as f64, t.bottom.g as f64, t.bottom.b as f64);
        cr.arc(w / 2.0, h / 2.0, w.min(h) / 2.0 - 1.0, 0.0, std::f64::consts::TAU);
        cr.set_source(&g).ok();
        cr.fill_preserve().ok();
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.4);
        cr.set_line_width(1.0);
        cr.stroke().ok();
        // accent dot
        cr.arc(w * 0.72, h * 0.72, w * 0.16, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(t.selected.r as f64, t.selected.g as f64, t.selected.b as f64);
        cr.fill().ok();
    });
    area
}

/// Asks the window to pick and import an image, then calls back with it.
pub type ImportImage = Rc<dyn Fn(Box<dyn FnOnce(crate::model::Id)>)>;

/// (id, name) of the project's image assets.
fn image_assets(doc: &Document) -> Vec<(crate::model::Id, String)> {
    doc.project().assets.iter().filter(|a| a.kind == crate::model::AssetKind::Image).map(|a| (a.id, a.name())).collect()
}

pub fn present(
    doc: &Rc<Document>,
    images: &Rc<ImageCache>,
    parent: &impl IsA<gtk::Widget>,
    import_image: ImportImage,
    on_applied: impl Fn() + 'static,
) {
    let dialog = adw::Dialog::builder().title(gettext("Menu Templates")).content_width(900).content_height(720).build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
    let cancel = gtk::Button::with_mnemonic(&gettext("_Cancel"));
    let apply = gtk::Button::builder().label(gettext("_Apply")).use_underline(true).css_classes(["suggested-action"]).build();
    header.pack_start(&cancel);
    header.pack_end(&apply);
    toolbar.add_top_bar(&header);

    let has_content = doc.project().menus.iter().any(|m| !m.items.is_empty());
    let banner = adw::Banner::builder()
        .title(gettext("Applying a template replaces your current menus. You can undo it."))
        .revealed(has_content)
        .build();
    toolbar.add_top_bar(&banner);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_start(24)
        .margin_end(24)
        .margin_top(18)
        .margin_bottom(24)
        .build();

    // Layout cards
    let heading = |t: &str| gtk::Label::builder().label(t).xalign(0.0).css_classes(["heading"]).build();
    body.append(&heading(&gettext("Layout")));
    let layouts = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .column_spacing(12)
        .row_spacing(12)
        .css_classes(["template-layouts"])
        .build();
    let mut layout_pictures = Vec::new();
    for layout in Layout::ALL {
        let pic = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .height_request(112)
            .css_classes(["template-page"])
            .build();
        let v = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_start(6).margin_end(6).margin_top(6).margin_bottom(6).build();
        v.append(&pic);
        v.append(&gtk::Label::builder().label(layout.name()).xalign(0.0).css_classes(["heading"]).build());
        v.append(
            &gtk::Label::builder()
                .label(layout.description())
                .xalign(0.0)
                .wrap(true)
                .lines(2)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        layouts.append(&v);
        layout_pictures.push((layout, pic));
    }
    body.append(&layouts);

    // Themes
    body.append(&heading(&gettext("Theme")));
    let themes = gtk::Box::builder().spacing(8).css_classes(["theme-chips"]).build();
    let mut first: Option<gtk::ToggleButton> = None;
    let mut theme_buttons = Vec::new();
    for (i, t) in THEMES.iter().enumerate() {
        let content = gtk::Box::builder().spacing(8).build();
        content.append(&swatch(i));
        content.append(&gtk::Label::new(Some(&gettext(t.name))));
        let b = gtk::ToggleButton::builder().child(&content).active(i == 0).css_classes(["flat"]).build();
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        themes.append(&b);
        theme_buttons.push(b);
    }
    body.append(&themes);

    // Title
    let group = adw::PreferencesGroup::new();
    let title_row = adw::EntryRow::builder().title(gettext("Title on the Main Menu")).text(doc.project().disc.name.clone()).build();
    group.add(&title_row);
    let logo_row = adw::ComboRow::builder()
        .title(gettext("Title Image"))
        .subtitle(gettext("A logo, e.g. a transparent PNG, shown instead of the title text"))
        .build();
    let logo_choices: Rc<RefCell<Vec<crate::model::Id>>> = Rc::new(RefCell::new(Vec::new()));
    let fill_logos = {
        let (row, choices, doc) = (logo_row.clone(), logo_choices.clone(), doc.clone());
        move |select: Option<crate::model::Id>| {
            let assets = image_assets(&doc);
            let mut labels = vec![gettext("None (use text)")];
            labels.extend(assets.iter().map(|(_, n)| n.clone()));
            let strs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            *choices.borrow_mut() = assets.iter().map(|(id, _)| *id).collect();
            row.set_model(Some(&gtk::StringList::new(&strs)));
            let idx = select.and_then(|s| assets.iter().position(|(id, _)| *id == s)).map_or(0, |i| i + 1);
            row.set_selected(idx as u32);
        }
    };
    fill_logos(None);
    let import = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(gettext("Import an Image…"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    logo_row.add_suffix(&import);
    group.add(&logo_row);
    body.append(&group);

    // Generated pages
    let summary = gtk::Label::builder().xalign(0.0).css_classes(["heading"]).build();
    body.append(&summary);
    let pages = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .max_children_per_line(8)
        .column_spacing(12)
        .row_spacing(12)
        .halign(gtk::Align::Start)
        .build();
    body.append(&pages);

    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&body).build();
    toolbar.set_content(Some(&scroller));
    dialog.set_child(Some(&toolbar));

    let state = Rc::new(State {
        doc: doc.clone(),
        images: images.clone(),
        layout: Cell::new(Layout::Show),
        theme: Cell::new(0),
        title: RefCell::new(doc.project().disc.name.clone()),
        logo: Cell::new(None),
        layout_pictures,
        pages,
        summary,
    });

    if let Some(child) = layouts.child_at_index(0) {
        layouts.select_child(&child);
    }
    let s = state.clone();
    layouts.connect_selected_children_changed(move |fb| {
        if let Some(i) = fb.selected_children().first().map(|c| c.index()) {
            s.layout.set(Layout::ALL[(i as usize).min(Layout::ALL.len() - 1)]);
            s.refresh();
        }
    });
    for (i, b) in theme_buttons.iter().enumerate() {
        let s = state.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                s.theme.set(i);
                s.refresh();
            }
        });
    }
    let (s, choices) = (state.clone(), logo_choices.clone());
    logo_row.connect_selected_notify(move |r| {
        let pick = (r.selected() as usize).checked_sub(1).and_then(|i| choices.borrow().get(i).copied());
        if s.logo.get() != pick {
            s.logo.set(pick);
            s.refresh();
        }
    });
    let fill_logos = Rc::new(fill_logos);
    import.connect_clicked(move |_| {
        let fill = fill_logos.clone();
        import_image(Box::new(move |id| fill(Some(id))));
    });
    let s = state.clone();
    title_row.connect_changed(move |r| {
        *s.title.borrow_mut() = r.text().to_string();
        s.refresh();
    });
    // Thumbnails arrive asynchronously.
    let weak = Rc::downgrade(&state);
    let pending = Rc::new(Cell::new(false));
    images.connect_ready(move || {
        let Some(s) = weak.upgrade() else { return };
        if pending.replace(true) {
            return;
        }
        let pending = pending.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            pending.set(false);
            s.refresh();
        });
    });
    state.refresh();

    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let d = dialog.clone();
    let s = state.clone();
    apply.connect_clicked(move |_| {
        let opts = s.options(s.layout.get());
        let main = s.doc.edit(Change::Structure, |p| templates::apply(p, &opts));
        s.doc.select(Node::Menu(main), None);
        d.close();
        on_applied();
    });
    dialog.present(Some(parent));
}
