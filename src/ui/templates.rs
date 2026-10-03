// SPDX-License-Identifier: GPL-3.0-or-later

//! Template chooser: pick a category and a style, a color palette and the
//! title lines, preview every generated menu and apply it to the project.

use crate::document::{Change, Document, Node};
use crate::model::{Menu, Project};
use crate::render::{self, ImageCache};
use crate::templates::{self, custom, Category, Options, Style, THEMES};
use std::path::PathBuf;
use std::sync::Arc;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{cairo, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct State {
    doc: Rc<Document>,
    images: Rc<ImageCache>,
    category: Cell<Category>,
    style: Cell<Style>,
    theme: Cell<usize>,
    /// The palette was picked by hand (else it follows the style).
    theme_chosen: Cell<bool>,
    title: RefCell<String>,
    season: RefCell<String>,
    disc: RefCell<String>,
    logo: Cell<Option<crate::model::Id>>,
    styles: gtk::FlowBox,
    style_cards: RefCell<Vec<(Style, gtk::Picture)>>,
    theme_buttons: Vec<gtk::ToggleButton>,
    pages: gtk::FlowBox,
    summary: gtk::Label,
    /// Set while updating widgets from code.
    syncing: Cell<bool>,
    /// Showing the saved templates rather than a category.
    custom_mode: Cell<bool>,
    customs: RefCell<Vec<(PathBuf, Arc<custom::CustomTemplate>)>>,
    custom_index: Cell<usize>,
    custom_cards: RefCell<Vec<(Arc<custom::CustomTemplate>, gtk::Picture)>>,
    /// Palette choice (not for saved templates, which have their own).
    palette_box: gtk::Box,
    custom_actions: gtk::Box,
    apply_button: gtk::Button,
}

impl State {
    fn options(&self, style: Style) -> Options {
        Options {
            style,
            theme: if self.theme_chosen.get() { self.theme.get() } else { style.default_theme() },
            title: self.title.borrow().clone(),
            season: self.season.borrow().clone(),
            disc: self.disc.borrow().clone(),
            logo: self.logo.get(),
            custom: if self.custom_mode.get() { self.selected_custom() } else { None },
        }
    }

    fn selected_custom(&self) -> Option<Arc<custom::CustomTemplate>> {
        self.customs.borrow().get(self.custom_index.get()).map(|(_, t)| t.clone())
    }

    fn generate(&self, style: Style) -> Project {
        self.generate_with(&self.options(style))
    }

    fn generate_with(&self, opts: &Options) -> Project {
        let mut p = self.doc.project().clone();
        templates::apply(&mut p, opts);
        p
    }

    fn reload_customs(&self) {
        *self.customs.borrow_mut() = custom::library().into_iter().map(|(p, t)| (p, Arc::new(t))).collect();
        let n = self.customs.borrow().len();
        self.custom_index.set(self.custom_index.get().min(n.saturating_sub(1)));
    }

    /// Cards for the saved templates.
    fn fill_customs(self: &Rc<Self>) {
        self.styles.remove_all();
        let mut cards = Vec::new();
        for (_, t) in self.customs.borrow().iter() {
            let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).can_shrink(true).height_request(126).css_classes(["template-page"]).build();
            let v = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_start(6).margin_end(6).margin_top(6).margin_bottom(6).build();
            v.append(&pic);
            v.append(&gtk::Label::builder().label(&t.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build());
            let mut about = t.description.clone();
            if t.has_standard() {
                about = if about.is_empty() { gettext("16:9 and 4:3") } else { format!("{about} · {}", gettext("16:9 and 4:3")) };
            }
            if !t.author.is_empty() {
                about = if about.is_empty() { gettext("By {}").replace("{}", &t.author) } else { format!("{about} · {}", gettext("By {}").replace("{}", &t.author)) };
            }
            v.append(&gtk::Label::builder().label(&about).xalign(0.0).wrap(true).lines(3).css_classes(["caption", "dim-label"]).build());
            self.styles.append(&v);
            cards.push((t.clone(), pic));
        }
        *self.custom_cards.borrow_mut() = cards;
        self.syncing.set(true);
        if let Some(child) = self.styles.child_at_index(self.custom_index.get() as i32) {
            self.styles.select_child(&child);
        }
        self.syncing.set(false);
    }

    /// Switch between the built-in categories and the saved templates.
    fn show_mode(self: &Rc<Self>) {
        let custom = self.custom_mode.get();
        self.palette_box.set_visible(!custom);
        self.custom_actions.set_visible(custom);
        if custom {
            self.reload_customs();
            self.fill_customs();
        } else {
            self.fill_styles();
        }
        self.apply_button.set_sensitive(!custom || !self.customs.borrow().is_empty());
        self.refresh();
    }

    /// Style cards for the current category.
    fn fill_styles(self: &Rc<Self>) {
        self.styles.remove_all();
        let mut cards = Vec::new();
        for style in self.category.get().styles() {
            let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).can_shrink(true).height_request(126).css_classes(["template-page"]).build();
            let v = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_start(6).margin_end(6).margin_top(6).margin_bottom(6).build();
            v.append(&pic);
            v.append(&gtk::Label::builder().label(style.name()).xalign(0.0).css_classes(["heading"]).build());
            v.append(&gtk::Label::builder().label(style.description()).xalign(0.0).wrap(true).lines(3).css_classes(["caption", "dim-label"]).build());
            self.styles.append(&v);
            cards.push((style, pic));
        }
        *self.style_cards.borrow_mut() = cards;
        let index = self.category.get().styles().iter().position(|s| *s == self.style.get()).unwrap_or(0);
        self.syncing.set(true);
        if let Some(child) = self.styles.child_at_index(index as i32) {
            self.styles.select_child(&child);
        }
        self.syncing.set(false);
    }

    /// Show the palette in use on the palette buttons.
    fn sync_theme_buttons(&self) {
        let theme = self.options(self.style.get()).theme;
        self.syncing.set(true);
        if let Some(b) = self.theme_buttons.get(theme) {
            b.set_active(true);
        }
        self.syncing.set(false);
    }

    fn refresh(&self) {
        if self.custom_mode.get() {
            for (t, pic) in self.custom_cards.borrow().iter() {
                let p = self.generate_with(&Options { custom: Some(t.clone()), ..self.options(Style::Classic) });
                if let Some(m) = p.menus.first() {
                    pic.set_paintable(render::menu_thumbnail(&p, m, &self.images, 480).as_ref());
                }
            }
            if self.customs.borrow().is_empty() {
                self.pages.remove_all();
                self.summary.set_label(&gettext("No saved templates yet. Save your menus with Save Menus as Template, or import a template file."));
                return;
            }
        } else {
            for (style, pic) in self.style_cards.borrow().iter() {
                let p = self.generate(*style);
                if let Some(m) = p.menus.first() {
                    pic.set_paintable(render::menu_thumbnail(&p, m, &self.images, 480).as_ref());
                }
            }
        }
        // All menus of the selected style
        self.pages.remove_all();
        let p = self.generate(self.style.get());
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

/// Round gradient swatch for a palette.
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
    let dialog = adw::Dialog::builder().title(gettext("Menu Templates")).content_width(980).content_height(760).build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
    let cancel = gtk::Button::with_mnemonic(&gettext("_Cancel"));
    let apply = gtk::Button::builder().label(gettext("_Apply")).use_underline(true).css_classes(["suggested-action"]).build();
    header.pack_start(&cancel);
    header.pack_end(&apply);

    // Categories in the header.
    let categories = adw::ToggleGroup::new();
    for c in Category::ALL {
        categories.add(adw::Toggle::builder().label(c.name()).build());
    }
    categories.add(adw::Toggle::builder().label(gettext("My Templates")).build());
    header.set_title_widget(Some(&categories));
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
    let heading = |t: &str| gtk::Label::builder().label(t).xalign(0.0).css_classes(["heading"]).build();

    // Styles of the category
    body.append(&heading(&gettext("Style")));
    let styles = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .homogeneous(true)
        .min_children_per_line(3)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(12)
        .css_classes(["template-layouts"])
        .build();
    body.append(&styles);

    // Saved templates: bring in, share, delete.
    let custom_actions = gtk::Box::builder().spacing(12).visible(false).build();
    let import_template = gtk::Button::builder().label(gettext("_Import…")).use_underline(true).css_classes(["pill"]).build();
    let export_template = gtk::Button::builder().label(gettext("_Export…")).use_underline(true).css_classes(["pill"]).build();
    let remove_template = gtk::Button::builder().label(gettext("_Remove")).use_underline(true).css_classes(["pill", "destructive-action"]).build();
    custom_actions.append(&import_template);
    custom_actions.append(&export_template);
    custom_actions.append(&remove_template);
    body.append(&custom_actions);

    // Palettes
    let palette_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).build();
    palette_box.append(&heading(&gettext("Colors")));
    let palettes = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(6)
        .column_spacing(6)
        .row_spacing(6)
        .css_classes(["theme-chips"])
        .build();
    let mut first: Option<gtk::ToggleButton> = None;
    let mut theme_buttons = Vec::new();
    for (i, t) in THEMES.iter().enumerate() {
        let content = gtk::Box::builder().spacing(8).build();
        content.append(&swatch(i));
        content.append(&gtk::Label::new(Some(&gettext(t.name))));
        let b = gtk::ToggleButton::builder().child(&content).css_classes(["flat"]).build();
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        palettes.append(&b);
        theme_buttons.push(b);
    }
    palette_box.append(&palettes);
    body.append(&palette_box);

    // Title lines
    let group = adw::PreferencesGroup::builder().title(gettext("Title")).build();
    let title_row = adw::EntryRow::builder().title(gettext("Name on the Main Menu")).text(doc.project().disc.name.clone()).build();
    group.add(&title_row);
    let season_row = adw::EntryRow::builder().title(gettext("Season (optional, e.g. “Season 2”)")).build();
    group.add(&season_row);
    let disc_row = adw::EntryRow::builder().title(gettext("Disc (optional, e.g. “Disc 1”)")).build();
    group.add(&disc_row);
    let logo_row = adw::ComboRow::builder()
        .title(gettext("Title Image"))
        .subtitle(gettext("A logo, e.g. a transparent PNG, shown instead of the name"))
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

    // A movie for a single title, a show for several.
    let category = if doc.project().titles.len() == 1 { Category::Movie } else { Category::TvShow };
    let state = Rc::new(State {
        doc: doc.clone(),
        images: images.clone(),
        category: Cell::new(category),
        style: Cell::new(category.styles()[0]),
        theme: Cell::new(0),
        theme_chosen: Cell::new(false),
        title: RefCell::new(doc.project().disc.name.clone()),
        season: RefCell::new(String::new()),
        disc: RefCell::new(String::new()),
        logo: Cell::new(None),
        styles: styles.clone(),
        style_cards: RefCell::new(Vec::new()),
        theme_buttons: theme_buttons.clone(),
        pages,
        summary,
        syncing: Cell::new(false),
        custom_mode: Cell::new(false),
        customs: RefCell::default(),
        custom_index: Cell::new(0),
        custom_cards: RefCell::default(),
        palette_box,
        custom_actions,
        apply_button: apply.clone(),
    });
    categories.set_active(Category::ALL.iter().position(|c| *c == category).unwrap_or(0) as u32);

    let s = state.clone();
    categories.connect_active_notify(move |g| {
        let Some(c) = Category::ALL.get(g.active() as usize).copied() else {
            s.custom_mode.set(true);
            s.show_mode();
            return;
        };
        if c == s.category.get() && !s.custom_mode.get() {
            return;
        }
        s.custom_mode.set(false);
        s.category.set(c);
        s.style.set(c.styles()[0]);
        s.show_mode();
        s.sync_theme_buttons();
    });
    let s = state.clone();
    styles.connect_selected_children_changed(move |fb| {
        if s.syncing.get() {
            return;
        }
        if s.custom_mode.get() {
            if let Some(i) = fb.selected_children().first().map(|c| c.index()) {
                s.custom_index.set(i.max(0) as usize);
                s.refresh();
            }
            return;
        }
        let list = s.category.get().styles();
        if let Some(i) = fb.selected_children().first().map(|c| c.index()) {
            s.style.set(list[(i as usize).min(list.len() - 1)]);
            s.sync_theme_buttons();
            s.refresh();
        }
    });
    for (i, b) in theme_buttons.iter().enumerate() {
        let s = state.clone();
        b.connect_toggled(move |b| {
            if b.is_active() && !s.syncing.get() {
                s.theme.set(i);
                s.theme_chosen.set(true);
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
    // Redrawing every preview takes a while, so wait for a pause in typing.
    let typing: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    for (row, field) in [(&title_row, 0), (&season_row, 1), (&disc_row, 2)] {
        let (s, typing) = (state.clone(), typing.clone());
        row.connect_changed(move |r| {
            let text = r.text().to_string();
            match field {
                0 => *s.title.borrow_mut() = text,
                1 => *s.season.borrow_mut() = text,
                _ => *s.disc.borrow_mut() = text,
            }
            if let Some(id) = typing.take() {
                id.remove();
            }
            let (weak, done) = (Rc::downgrade(&s), typing.clone());
            *typing.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                done.take();
                if let Some(s) = weak.upgrade() {
                    s.refresh();
                }
            }));
        });
    }
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
    state.fill_styles();
    state.sync_theme_buttons();
    state.refresh();

    {
        let (s, d) = (state.clone(), dialog.clone());
        import_template.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&gettext("Spindle Templates")));
            filter.add_pattern(&format!("*.{}", custom::EXTENSION));
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let fd = gtk::FileDialog::builder().title(gettext("Import Template")).filters(&filters).modal(true).build();
            let s = s.clone();
            let root = d.root().and_downcast::<gtk::Window>();
            fd.open(root.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                let result = custom::load(&crate::media::portal::real_path(&path)).and_then(|t| custom::install(&t).map(|_| t.name));
                match result {
                    Ok(name) => {
                        s.reload_customs();
                        let i = s.customs.borrow().iter().position(|(_, t)| t.name == name).unwrap_or(0);
                        s.custom_index.set(i);
                        s.show_mode();
                    }
                    Err(e) => s.summary.set_label(&e.to_string()),
                }
            });
        });
    }
    {
        let (s, d) = (state.clone(), dialog.clone());
        let summary = state.summary.clone();
        export_template.connect_clicked(move |_| {
            let Some(t) = s.selected_custom() else { return };
            let summary = summary.clone();
            super::save_template::export_file(&d, (*t).clone(), Rc::new(move |msg| summary.set_label(&msg)));
        });
    }
    {
        let (s, d) = (state.clone(), dialog.clone());
        remove_template.connect_clicked(move |_| {
            let Some((path, t)) = s.customs.borrow().get(s.custom_index.get()).cloned() else { return };
            let ask = adw::AlertDialog::builder()
                .heading(gettext("Remove “{}”?").replace("{}", &t.name))
                .body(gettext("The template is removed from Spindle. Projects made with it keep their menus."))
                .build();
            ask.add_responses(&[("cancel", &gettext("_Cancel")), ("remove", &gettext("_Remove"))]);
            ask.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            let s = s.clone();
            ask.connect_response(None, move |_, r| {
                if r == "remove" {
                    let _ = std::fs::remove_file(&path);
                    s.show_mode();
                }
            });
            ask.present(Some(&d));
        });
    }
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let d = dialog.clone();
    let s = state.clone();
    apply.connect_clicked(move |_| {
        let opts = s.options(s.style.get());
        let main = s.doc.edit(Change::Structure, |p| templates::apply(p, &opts));
        s.doc.select(Node::Menu(main), None);
        d.close();
        on_applied();
    });
    #[cfg(debug_assertions)]
    if std::env::var("SPINDLE_SCREENSHOT_MY_TEMPLATES").is_ok() {
        categories.set_active(Category::ALL.len() as u32);
    }
    dialog.present(Some(parent));
}
