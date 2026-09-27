/* window.rs
 *
 * Copyright 2026 daftblazer
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use crate::document::{Change, Document, Node};
use crate::media::{portal, probe};
use crate::model::*;
use crate::render::ImageCache;
use crate::ui;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use std::cell::{OnceCell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/daftblazer/Spindle/window.ui")]
    pub struct SpindleWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub sidebar_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub inspector_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub inspector_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub center_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub canvas_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub title_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub media_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub highlight_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub safe_area_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub grid_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub inspector_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub missing_banner: TemplateChild<adw::Banner>,

        pub state: OnceCell<Rc<super::State>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpindleWindow {
        const NAME: &'static str = "SpindleWindow";
        type Type = super::SpindleWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SpindleWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }
    impl WidgetImpl for SpindleWindow {}
    impl WindowImpl for SpindleWindow {
        fn close_request(&self) -> glib::Propagation {
            let obj = self.obj();
            obj.save_window_state();
            let state = obj.state();
            if state.doc.is_dirty() {
                let win = obj.clone();
                obj.confirm_discard(move || {
                    crate::recovery::remove(win.state().session);
                    win.destroy()
                });
                return glib::Propagation::Stop;
            }
            crate::recovery::remove(state.session);
            glib::Propagation::Proceed
        }
    }
    impl ApplicationWindowImpl for SpindleWindow {}
    impl AdwApplicationWindowImpl for SpindleWindow {}
}

glib::wrapper! {
    pub struct SpindleWindow(ObjectSubclass<imp::SpindleWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

/// Editor components owned by the window.
pub struct State {
    pub doc: Rc<Document>,
    pub canvas: ui::canvas::MenuCanvas,
    images: Rc<ImageCache>,
    /// Copied menu items (Ctrl+C / Ctrl+V), shared between menus.
    clipboard: RefCell<Vec<MenuItem>>,
    settings: Option<gio::Settings>,
    _sidebar: Rc<ui::sidebar::Sidebar>,
    inspector: Rc<ui::inspector::Inspector>,
    _media: Rc<ui::media_bin::MediaBin>,
    title_view: Rc<ui::title_view::TitleView>,
    /// Names this window's crash-recovery copy.
    session: Id,
    /// Hash of the last recovery copy written.
    recovery_hash: std::cell::Cell<u64>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("State")
    }
}

const VIDEO_EXT: &[&str] = &["mp4", "mkv", "mov", "avi", "m4v", "webm", "mpg", "mpeg", "ts", "m2ts", "mts", "wmv", "flv", "vob", "ogv"];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "svg"];
const AUDIO_EXT: &[&str] = &["mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac", "ac3", "wma"];

/// Evenly distributed button rectangles for `n` items inside the lower
/// part of the safe area.
pub fn grid_layout(n: usize, with_thumbnails: bool) -> Vec<Rect> {
    if n == 0 {
        return vec![];
    }
    let cols = if with_thumbnails { (n as f64).sqrt().ceil().clamp(1.0, 4.0) as usize } else { 1 };
    let rows = n.div_ceil(cols);
    let (x0, y0, w, h) = (DESIGN_WIDTH * 0.08, 260.0, DESIGN_WIDTH * 0.84, DESIGN_HEIGHT - 260.0 - DESIGN_HEIGHT * 0.1);
    let gap = 30.0;
    let cw = (w - gap * (cols as f64 - 1.0)) / cols as f64;
    let ch = ((h - gap * (rows as f64 - 1.0)) / rows as f64).min(if with_thumbnails { cw * 0.72 } else { 90.0 });
    let total_h = ch * rows as f64 + gap * (rows as f64 - 1.0);
    let top = y0 + (h - total_h) / 2.0;
    (0..n)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            let in_row = if r == rows - 1 { n - r * cols } else { cols };
            let row_w = cw * in_row as f64 + gap * (in_row as f64 - 1.0);
            let left = x0 + (w - row_w) / 2.0;
            if with_thumbnails {
                Rect::new(left + c as f64 * (cw + gap), top + r as f64 * (ch + gap), cw, ch)
            } else {
                Rect::new(DESIGN_WIDTH / 2.0 - 300.0, top + r as f64 * (ch + gap), 600.0, ch)
            }
        })
        .collect()
}

impl SpindleWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder().property("application", application).build()
    }

    pub fn state(&self) -> Rc<State> {
        self.imp().state.get().expect("window state").clone()
    }

    fn doc(&self) -> Rc<Document> {
        self.state().doc.clone()
    }

    fn setup(&self) {
        let imp = self.imp();
        let doc = Document::new();

        let win = self.downgrade();
        let images = ImageCache::new_async(move || {
            if let Some(w) = win.upgrade() {
                w.state().canvas.queue_draw();
            }
        });
        let canvas = ui::canvas::MenuCanvas::new(doc.clone(), images.clone());
        imp.canvas_box.append(canvas.widget());

        let sidebar = ui::sidebar::Sidebar::new(doc.clone(), imp.sidebar_box.get(), images.clone());
        let inspector = ui::inspector::Inspector::new(doc.clone(), imp.inspector_box.get(), imp.inspector_title.get());
        let win = self.downgrade();
        let media = ui::media_bin::MediaBin::new(doc.clone(), &imp.media_box, move |files| {
            if let Some(w) = win.upgrade() {
                w.import_files(files);
            }
        });
        let title_view = ui::title_view::TitleView::new(doc.clone(), &imp.title_box);

        let c = canvas.clone();
        imp.safe_area_button.connect_toggled(move |b| c.set_show_safe_area(b.is_active()));
        let c = canvas.clone();
        imp.grid_button.connect_toggled(move |b| c.set_grid(b.is_active()));

        let win = self.downgrade();
        canvas.connect_message(move |msg| {
            if let Some(w) = win.upgrade() {
                let toast = adw::Toast::builder().title(msg).timeout(2).build();
                w.imp().toast_overlay.add_toast(toast);
            }
        });
        let win = self.downgrade();
        canvas.connect_files_dropped(move |files, x, y| {
            if let Some(w) = win.upgrade() {
                let w2 = w.clone();
                w.import_files_then(files, move |ids| {
                    w2.state().canvas.place_assets(&ids, x, y);
                });
            }
        });

        let state = Rc::new(State {
            doc: doc.clone(),
            canvas,
            images,
            clipboard: RefCell::new(Vec::new()),
            settings: crate::app_settings(),
            _sidebar: sidebar,
            inspector,
            _media: media,
            title_view,
            session: crate::model::new_id(),
            recovery_hash: std::cell::Cell::new(0),
        });
        imp.state.set(state).unwrap();
        self.setup_recovery();

        let win = self.downgrade();
        doc.connect(move |c| {
            if let Some(w) = win.upgrade() {
                w.on_change(c);
            }
        });

        self.setup_actions();
        self.restore_window_state();
        self.on_change(Change::Structure);
        #[cfg(debug_assertions)]
        self.debug_screenshot();
    }

    /// Development aid: `SPINDLE_SCREENSHOT=out.png` renders the window to a
    /// PNG after startup (`SPINDLE_SCREENSHOT_ITEM=n` selects the n-th item of
    /// the current menu first) and quits.
    #[cfg(debug_assertions)]
    fn debug_screenshot(&self) {
        let Ok(path) = std::env::var("SPINDLE_SCREENSHOT") else { return };
        if let Some((w, h)) = std::env::var("SPINDLE_SCREENSHOT_SIZE").ok().and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        }) {
            self.set_default_size(w, h);
        }
        // Apply the requested state once the project from the command line
        // is loaded, before the window is first laid out.
        let win = self.clone();
        glib::idle_add_local_once(move || {
            let doc = win.doc();
            if let Some(n) = std::env::var("SPINDLE_SCREENSHOT_MENU").ok().and_then(|n| n.parse::<usize>().ok()) {
                let m = doc.project().menus.get(n).map(|m| m.id);
                if let Some(m) = m {
                    doc.select(Node::Menu(m), None);
                }
            }
            if let Ok(list) = std::env::var("SPINDLE_SCREENSHOT_ITEM") {
                let ids: Vec<Id> = list
                    .split(',')
                    .filter_map(|n| n.parse::<usize>().ok())
                    .filter_map(|n| doc.current_menu().and_then(|m| doc.project().menu(m).and_then(|m| m.items.get(n).map(|i| i.id))))
                    .collect();
                doc.set_selection(ids);
            }
            if std::env::var("SPINDLE_SCREENSHOT_GRID").is_ok() {
                win.imp().grid_button.set_active(true);
            }
            if let Some(z) = std::env::var("SPINDLE_SCREENSHOT_ZOOM").ok().and_then(|z| z.parse::<f64>().ok()) {
                let c = win.state().canvas.clone();
                // After the first layout, so the zoom centres on the page.
                glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || c.zoom_by(z, None));
            }
            if let Ok(keys) = std::env::var("SPINDLE_SCREENSHOT_KEYS") {
                win.state().canvas.debug_keys(&keys);
            }
            if std::env::var("SPINDLE_SCREENSHOT_OSD").is_ok() {
                win.state().title_view.debug_show_osd();
            }
            if let Ok(keys) = std::env::var("SPINDLE_SCREENSHOT_TITLE_KEYS") {
                let tv = win.state().title_view.clone();
                // Give the player time to load before pressing keys.
                glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || tv.debug_keys(&keys));
            }
            if let Some(node) = std::env::var("SPINDLE_SCREENSHOT_TITLE").ok().and_then(|n| n.parse::<usize>().ok()) {
                let t = doc.project().titles.get(node).map(|t| t.id);
                if let Some(t) = t {
                    doc.select(Node::Title(t), None);
                }
            }
            if let Ok(action) = std::env::var("SPINDLE_SCREENSHOT_ACTION") {
                let _ = WidgetExt::activate_action(&win, &action, None);
            }
        });
        let win = self.clone();
        {
            let win2 = win.clone();
            glib::timeout_add_seconds_local_once(5, move || {
                let paintable = gtk::WidgetPaintable::new(Some(&win2));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, win2.width() as f64, win2.height() as f64);
                if let (Some(node), Some(renderer)) = (snapshot.to_node(), win2.renderer()) {
                    let tex = renderer.render_texture(&node, None);
                    if let Err(e) = tex.save_to_png(&path) {
                        log::error!("screenshot failed: {e}");
                    }
                }
                if let Some(app) = win2.application() {
                    app.quit();
                }
            });
        }
    }

    fn restore_window_state(&self) {
        let Some(settings) = self.state().settings.clone() else { return };
        settings.bind("show-safe-area", &*self.imp().safe_area_button, "active").build();
        settings.bind("snap-to-grid", &*self.imp().grid_button, "active").build();
        let (w, h) = (settings.int("window-width"), settings.int("window-height"));
        if w > 0 && h > 0 {
            self.set_default_size(w, h);
        }
        if settings.boolean("window-maximized") {
            self.maximize();
        }
    }

    pub fn save_window_state(&self) {
        let Some(settings) = self.state().settings.clone() else { return };
        if !self.is_maximized() {
            let (w, h) = self.default_size();
            let _ = settings.set_int("window-width", w);
            let _ = settings.set_int("window-height", h);
        }
        let _ = settings.set_boolean("window-maximized", self.is_maximized());
    }

    fn on_change(&self, c: Change) {
        let imp = self.imp();
        let state = self.state();
        let doc = &state.doc;
        if matches!(c, Change::Selection | Change::Structure) && state.canvas.is_preview() {
            if doc.current_menu().is_some() {
                state.canvas.reset_preview_button();
            } else if let Some(a) = self.lookup_action("remote-preview") {
                a.change_state(&false.to_variant());
            }
        }
        if c == Change::Structure {
            self.update_missing_banner();
        }
        if matches!(c, Change::Structure | Change::Selection) {
            let page = match doc.node() {
                Node::Menu(_) => "menu",
                Node::Title(_) => "title",
                Node::None => "empty",
            };
            imp.center_stack.set_visible_child_name(page);
        }
        state.canvas.queue_draw();
        let dirty = if doc.is_dirty() { "• " } else { "" };
        imp.window_title.set_title(&format!("{dirty}{}", doc.title()));
        let sub = doc
            .path()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| {
                let home = glib::home_dir();
                match d.strip_prefix(&home) {
                    Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
                    Ok(rest) => format!("~/{}", rest.display()),
                    Err(_) => d.display().to_string(),
                }
            })
            .unwrap_or_default();
        imp.window_title.set_subtitle(&sub);
        self.update_actions();
    }

    fn action_enabled(&self, name: &str, on: bool) {
        if let Some(a) = self.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(on);
        }
    }

    fn update_actions(&self) {
        let doc = self.doc();
        let n_selected = doc.selection().len();
        let has_item = n_selected > 0;
        let in_menu = doc.current_menu().is_some();
        self.action_enabled("undo", doc.can_undo());
        self.action_enabled("redo", doc.can_redo());
        for a in [
            "delete-item", "duplicate-item", "raise-item", "lower-item", "forward-item", "backward-item", "lock-item", "hide-item", "copy", "cut", "align-left", "align-hcenter",
            "align-right", "align-top", "align-vcenter", "align-bottom",
        ] {
            self.action_enabled(a, has_item);
        }
        let single_button = n_selected == 1
            && doc.current_menu().zip(doc.item()).is_some_and(|(m, i)| {
                doc.project().menu(m).and_then(|m| m.item(i)).is_some_and(|i| i.button().is_some())
            });
        self.action_enabled("default-button", single_button);
        self.action_enabled("edit-label", n_selected == 1);
        self.action_enabled("distribute-h", n_selected > 2);
        self.action_enabled("group-items", n_selected > 1);
        let grouped = doc
            .current_menu()
            .and_then(|m| doc.project().menu(m).map(|m| m.items.iter().any(|i| i.group.is_some() && doc.is_selected(i.id))))
            .unwrap_or(false);
        self.action_enabled("ungroup-items", grouped);
        self.action_enabled("distribute-v", n_selected > 2);
        let editing = in_menu && !self.state().canvas.is_preview();
        let single_visual = n_selected == 1
            && doc.current_menu().zip(doc.item()).is_some_and(|(m, i)| {
                doc.project().menu(m).and_then(|m| m.item(i)).is_some_and(|i| matches!(i.kind, ItemKind::Text(_) | ItemKind::Image(_)))
            });
        self.action_enabled("replace-with-image", single_visual);
        self.action_enabled("menu-background-image", in_menu);
        self.action_enabled("menu-music", in_menu);
        for a in ["add-button", "add-text", "add-image", "add-button-grid", "select-all"] {
            self.action_enabled(a, editing);
        }
        self.action_enabled("paste", in_menu && !self.state().clipboard.borrow().is_empty());
        self.action_enabled("add-chapter-menu", matches!(doc.node(), Node::Title(_)));
        self.action_enabled("choose-thumbnail", matches!(doc.node(), Node::Title(_)));
        self.action_enabled("add-subtitle", matches!(doc.node(), Node::Title(_)));
        self.action_enabled("add-audio", matches!(doc.node(), Node::Title(_)));
        self.action_enabled("preview-title", matches!(doc.node(), Node::Title(_)));
        self.action_enabled("check-video", matches!(doc.node(), Node::Title(_)));
        self.action_enabled("delete-node", doc.node() != Node::None);
        self.action_enabled("duplicate-menu", matches!(doc.node(), Node::Menu(_)));
    }

    fn toast(&self, msg: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(msg));
    }

    fn setup_actions(&self) {
        let add = |name: &str, f: fn(&SpindleWindow)| {
            let action = gio::SimpleAction::new(name, None);
            let win = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(w) = win.upgrade() {
                    f(&w);
                }
            });
            self.add_action(&action);
        };
        add("new", |w| {
            let w2 = w.clone();
            w.confirm_discard(move || w2.doc().replace(Project::default(), None));
        });
        add("open", |w| {
            let w2 = w.clone();
            w.confirm_discard(move || w2.open_dialog());
        });
        add("save", |w| w.save(false));
        add("save-as", |w| w.save(true));
        add("undo", |w| w.doc().undo());
        add("redo", |w| w.doc().redo());
        add("import", |w| w.import_dialog());
        add("build", |w| ui::build_dialog::present(&w.doc(), w));
        add("disc-settings", |w| ui::settings::present(&w.doc(), w));
        add("burn-image", |w| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&gettext("Disc Images")));
            filter.add_suffix("iso");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let fd = gtk::FileDialog::builder().title(gettext("Choose a Disc Image to Burn")).filters(&filters).modal(true).build();
            if let Some(dir) = glib::user_special_dir(glib::UserDirectory::Videos) {
                fd.set_initial_folder(Some(&gio::File::for_path(dir)));
            }
            let win = w.clone();
            fd.open(Some(w), gio::Cancellable::NONE, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path()) {
                    ui::burn::present_image(&win, crate::media::portal::real_path(&path));
                }
            });
        });
        add("templates", |w| {
            let state = w.state();
            let win = w.downgrade();
            let picker = w.downgrade();
            let import: ui::templates::ImportImage = Rc::new(move |done| {
                if let Some(w) = picker.upgrade() {
                    w.pick_image(done);
                }
            });
            ui::templates::present(&state.doc, &state.images, w, import, move || {
                if let Some(w) = win.upgrade() {
                    w.toast(&gettext("Template applied"));
                }
            });
        });
        add("add-menu", |w| {
            let doc = w.doc();
            let n = doc.project().menus.len() + 1;
            let id = doc.edit(Change::Structure, |p| {
                let m = Menu::new(&format!("{} {n}", gettext("Menu")));
                let id = m.id;
                p.menus.push(m);
                id
            });
            doc.select(Node::Menu(id), None);
        });
        add("add-popup-menu", |w| {
            let doc = w.doc();
            let n = doc.project().popup_menus().count() + 1;
            let id = doc.edit(Change::Structure, |p| {
                let m = Menu::new_popup(&format!("{} {n}", gettext("Pop-up Menu")));
                let id = m.id;
                p.menus.push(m);
                // The first pop-up menu applies to every title.
                if p.disc.popup_menu.is_none() {
                    p.disc.popup_menu = Some(id);
                }
                id
            });
            doc.select(Node::Menu(id), None);
        });
        add("add-chapter-menu", |w| w.add_chapter_menu());
        add("choose-thumbnail", |w| w.state().title_view.choose_thumbnail(w));
        add("add-subtitle", |w| w.add_subtitle_dialog());
        add("add-audio", |w| {
            let Node::Title(title) = w.doc().node() else { return };
            let win = w.clone();
            w.pick_media(&gettext("Add Audio Track"), &gettext("Audio"), &["audio/*", "video/*"], glib::UserDirectory::Music, move |asset| {
                let doc = win.doc();
                let track = doc.project().asset(asset).and_then(external_audio_track);
                let Some(track) = track else {
                    win.toast(&gettext("That file has no sound"));
                    return;
                };
                doc.edit(Change::Structure, |p| {
                    if let Some(t) = p.title_mut(title) {
                        t.audio.push(track);
                    }
                });
            });
        });
        add("locate-media", |w| w.locate_media());
        add("preview-title", |w| w.state().title_view.preview(w));
        add("check-video", |w| {
            if let Node::Title(t) = w.doc().node() {
                ui::compat_dialog::check(&w.doc(), t, w);
            }
        });
        add("add-button", |w| {
            w.add_item(|p| {
                let action = p.titles.first().map_or(Action::None, |t| Action::PlayTitle { title: t.id, chapter: 0 });
                MenuItem::new_button(&gettext("Play"), action, Rect::new(710.0, 800.0, 500.0, 90.0))
            })
        });
        add("add-text", |w| w.add_item(|_| MenuItem::new_text(&gettext("Title"), Rect::new(260.0, 90.0, 1400.0, 130.0))));
        add("add-image", |w| {
            let win = w.clone();
            w.pick_image(move |asset| {
                let area = Rect::new(DESIGN_WIDTH / 2.0 - 400.0, DESIGN_HEIGHT / 2.0 - 225.0, 800.0, 450.0);
                let rect = area.fit(win.image_aspect(asset), Align::Center);
                win.add_item(|_| MenuItem::new_image(asset, rect));
            });
        });
        add("replace-with-image", |w| w.replace_with_image());
        add("menu-music", |w| {
            let Some(menu) = w.doc().current_menu() else { return };
            let win = w.clone();
            w.pick_audio(move |asset| {
                let doc = win.doc();
                let length = doc.project().asset(asset).map(|a| a.info.duration).filter(|d| *d > 1.0);
                if doc.project().asset(asset).is_some_and(|a| !a.info.has_audio()) {
                    win.toast(&gettext("That file has no sound"));
                    return;
                }
                doc.edit(Change::Structure, |p| {
                    if let Some(m) = p.menu_mut(menu) {
                        m.audio = Some(asset);
                        // Loop the whole song (within reason).
                        if let Some(d) = length {
                            m.duration = d.clamp(5.0, 600.0).floor();
                        }
                    }
                });
            });
        });
        add("menu-background-image", |w| {
            let Some(menu) = w.doc().current_menu() else { return };
            let win = w.clone();
            w.pick_image(move |asset| {
                win.doc().edit(Change::Structure, |p| {
                    if let Some(m) = p.menu_mut(menu) {
                        m.background.image = Some(asset);
                        m.background.video = None;
                    }
                });
            });
        });
        add("add-button-grid", |w| w.add_title_buttons());
        add("delete-item", |w| w.edit_selection(|m, ids| m.items.retain(|i| !ids.contains(&i.id))));
        add("duplicate-item", |w| {
            let items = w.selected_items();
            w.paste_items(items, 30.0);
        });
        add("copy", |w| {
            let items = w.selected_items();
            *w.state().clipboard.borrow_mut() = items;
            w.update_actions();
        });
        add("cut", |w| {
            let items = w.selected_items();
            *w.state().clipboard.borrow_mut() = items;
            w.edit_selection(|m, ids| m.items.retain(|i| !ids.contains(&i.id)));
        });
        add("paste", |w| {
            let items = w.state().clipboard.borrow().clone();
            // Offset when pasting onto the menu the items came from.
            let doc = w.doc();
            let same_menu = doc.current_menu().is_some_and(|m| {
                doc.project().menu(m).is_some_and(|m| items.iter().any(|i| m.items.iter().any(|x| x.rect == i.rect)))
            });
            w.paste_items(items, if same_menu { 30.0 } else { 0.0 });
        });
        add("select-all", |w| {
            let doc = w.doc();
            let ids = doc
                .current_menu()
                .and_then(|m| doc.project().menu(m).map(|m| m.items.iter().map(|i| i.id).collect::<Vec<_>>()))
                .unwrap_or_default();
            doc.set_selection(ids);
        });
        add("edit-label", |w| {
            w.imp().inspector_button.set_active(true);
            w.state().inspector.focus_primary();
        });
        add("raise-item", |w| {
            w.edit_selection(|m, ids| {
                let (sel, rest): (Vec<MenuItem>, Vec<MenuItem>) = m.items.drain(..).partition(|i| ids.contains(&i.id));
                m.items = rest.into_iter().chain(sel).collect();
            })
        });
        add("lower-item", |w| {
            w.edit_selection(|m, ids| {
                let (sel, rest): (Vec<MenuItem>, Vec<MenuItem>) = m.items.drain(..).partition(|i| ids.contains(&i.id));
                m.items = sel.into_iter().chain(rest).collect();
            })
        });
        // One step up/down the stacking order, keeping the selection's order.
        add("forward-item", |w| {
            w.edit_selection(|m, ids| {
                for i in (0..m.items.len().saturating_sub(1)).rev() {
                    if ids.contains(&m.items[i].id) && !ids.contains(&m.items[i + 1].id) {
                        m.items.swap(i, i + 1);
                    }
                }
            })
        });
        add("backward-item", |w| {
            w.edit_selection(|m, ids| {
                for i in 1..m.items.len() {
                    if ids.contains(&m.items[i].id) && !ids.contains(&m.items[i - 1].id) {
                        m.items.swap(i, i - 1);
                    }
                }
            })
        });
        add("group-items", |w| {
            let group = new_id();
            w.edit_selection(|m, ids| m.items.iter_mut().filter(|i| ids.contains(&i.id)).for_each(|i| i.group = Some(group)));
        });
        add("ungroup-items", |w| {
            w.edit_selection(|m, ids| {
                let all = m.with_groups(ids);
                m.items.iter_mut().filter(|i| all.contains(&i.id)).for_each(|i| i.group = None);
            })
        });
        add("lock-item", |w| {
            w.edit_selection(|m, ids| m.items.iter_mut().filter(|i| ids.contains(&i.id)).for_each(|i| i.locked = true));
            w.doc().select_item(None);
        });
        add("hide-item", |w| w.edit_selection(|m, ids| m.items.iter_mut().filter(|i| ids.contains(&i.id)).for_each(|i| i.hidden = true)));
        add("unlock-all", |w| {
            let Some(menu) = w.doc().current_menu() else { return };
            w.doc().edit(Change::Structure, |p| {
                if let Some(m) = p.menu_mut(menu) {
                    for i in &mut m.items {
                        i.locked = false;
                        i.hidden = false;
                    }
                }
            });
        });
        add("default-button", |w| w.edit_item(|m, id| m.default_button = Some(id)));
        for (name, op) in [
            ("align-left", AlignOp::Left),
            ("align-hcenter", AlignOp::HCenter),
            ("align-right", AlignOp::Right),
            ("align-top", AlignOp::Top),
            ("align-vcenter", AlignOp::VCenter),
            ("align-bottom", AlignOp::Bottom),
            ("distribute-h", AlignOp::DistributeH),
            ("distribute-v", AlignOp::DistributeV),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let win = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(w) = win.upgrade() {
                    w.edit_selection_content(|m, ids| m.align_items(ids, op));
                }
            });
            self.add_action(&action);
        }
        add("delete-node", |w| {
            let doc = w.doc();
            match doc.node() {
                Node::Menu(id) => doc.edit(Change::Structure, |p| p.remove_menu(id)),
                Node::Title(id) => doc.edit(Change::Structure, |p| p.remove_title(id)),
                Node::None => {}
            }
        });
        add("duplicate-menu", |w| {
            let doc = w.doc();
            let Node::Menu(id) = doc.node() else { return };
            if let Some(copy) = doc.edit(Change::Structure, |p| p.duplicate_menu(id)) {
                doc.select(Node::Menu(copy), None);
            }
        });
        add("move-node-up", |w| w.move_node(-1));
        add("move-node-down", |w| w.move_node(1));

        let preview = gio::SimpleAction::new_stateful("remote-preview", None, &false.to_variant());
        let win = self.downgrade();
        preview.connect_change_state(move |a, value| {
            let Some(w) = win.upgrade() else { return };
            let on = value.and_then(|v| v.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            w.state().canvas.set_preview(on);
            w.update_actions();
        });
        preview.connect_activate(|a, _| {
            let on = a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            a.change_state(&(!on).to_variant());
        });
        self.add_action(&preview);

        let add_s = |name: &str, f: fn(&SpindleWindow, Id)| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let win = self.downgrade();
            action.connect_activate(move |_, param| {
                let id = param.and_then(|p| p.get::<String>()).and_then(|s| s.parse::<Id>().ok());
                if let (Some(w), Some(id)) = (win.upgrade(), id) {
                    f(&w, id);
                }
            });
            self.add_action(&action);
        };
        add_s("asset-add-title", |w, id| {
            let doc = w.doc();
            if let Some(t) = doc.edit(Change::Structure, |p| p.ensure_title_for(id)) {
                doc.select(Node::Title(t), None);
            }
        });
        add_s("asset-background", |w, id| {
            let doc = w.doc();
            let Some(menu) = doc.current_menu().or_else(|| doc.project().menus.first().map(|m| m.id)) else {
                w.toast(&gettext("Create a menu first"));
                return;
            };
            doc.edit(Change::Structure, |p| {
                let kind = p.asset(id).map(|a| a.kind);
                if let Some(m) = p.menu_mut(menu) {
                    match kind {
                        Some(AssetKind::Image) => {
                            m.background.image = Some(id);
                            m.background.video = None;
                        }
                        Some(AssetKind::Video) => m.background.video = Some(id),
                        Some(AssetKind::Audio) => m.audio = Some(id),
                        None => {}
                    }
                }
            });
            doc.select(Node::Menu(menu), None);
        });
        add_s("asset-remove", |w, id| w.doc().edit(Change::Structure, |p| p.remove_asset(id)));
    }

    fn move_node(&self, delta: isize) {
        let doc = self.doc();
        let node = doc.node();
        doc.edit(Change::Structure, |p| {
            fn shift<T>(v: &mut [T], pos: Option<usize>, delta: isize) {
                if let Some(i) = pos {
                    let j = i as isize + delta;
                    if j >= 0 && (j as usize) < v.len() {
                        v.swap(i, j as usize);
                    }
                }
            }
            match node {
                Node::Menu(id) => {
                    let pos = p.menus.iter().position(|m| m.id == id);
                    shift(&mut p.menus, pos, delta);
                }
                Node::Title(id) => {
                    let pos = p.titles.iter().position(|t| t.id == id);
                    shift(&mut p.titles, pos, delta);
                }
                Node::None => {}
            }
        });
    }

    fn add_item(&self, make: impl FnOnce(&Project) -> MenuItem) {
        let doc = self.doc();
        let Some(menu) = doc.current_menu() else { return };
        let id = doc.edit(Change::Structure, |p| {
            let item = make(p);
            let id = item.id;
            if let Some(m) = p.menu_mut(menu) {
                m.items.push(item);
            }
            id
        });
        doc.select_item(Some(id));
    }

    /// Clones of the selected items, in stacking order.
    fn selected_items(&self) -> Vec<MenuItem> {
        let doc = self.doc();
        let sel = doc.selection();
        doc.current_menu()
            .and_then(|m| doc.project().menu(m).map(|m| m.items.iter().filter(|i| sel.contains(&i.id)).cloned().collect()))
            .unwrap_or_default()
    }

    /// Insert copies of `items` into the current menu, offset by `offset`,
    /// and select them. Links to titles/menus are kept.
    fn paste_items(&self, items: Vec<MenuItem>, offset: f64) {
        let doc = self.doc();
        let Some(menu) = doc.current_menu() else { return };
        if items.is_empty() {
            return;
        }
        let ids = doc.edit(Change::Structure, |p| {
            let Some(m) = p.menu_mut(menu) else { return vec![] };
            let mut ids = Vec::new();
            // Copies of a group form a new group.
            let mut groups: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
            for mut it in items {
                it.id = new_id();
                it.group = it.group.map(|g| *groups.entry(g).or_insert_with(new_id));
                it.rect = Rect::new(it.rect.x + offset, it.rect.y + offset, it.rect.w, it.rect.h).clamped();
                if let Some(b) = it.button_mut() {
                    b.nav = NavOverride::default();
                }
                ids.push(it.id);
                m.items.push(it);
            }
            ids
        });
        doc.set_selection(ids);
    }

    fn edit_selection(&self, f: impl FnOnce(&mut Menu, &[Id])) {
        self.edit_selection_with(Change::Structure, f);
    }

    fn edit_selection_content(&self, f: impl FnOnce(&mut Menu, &[Id])) {
        self.edit_selection_with(Change::Content, f);
    }

    fn edit_selection_with(&self, change: Change, f: impl FnOnce(&mut Menu, &[Id])) {
        let doc = self.doc();
        let sel = doc.selection();
        let Some(menu) = doc.current_menu() else { return };
        if sel.is_empty() {
            return;
        }
        doc.edit(change, |p| {
            if let Some(m) = p.menu_mut(menu) {
                f(m, &sel);
            }
        });
    }

    fn edit_item(&self, f: impl FnOnce(&mut Menu, Id)) {
        let doc = self.doc();
        let (Some(menu), Some(item)) = (doc.current_menu(), doc.item()) else { return };
        doc.edit(Change::Structure, |p| {
            if let Some(m) = p.menu_mut(menu) {
                f(m, item);
            }
        });
    }

    /// Add a thumbnail button for every title to the current menu.
    fn add_title_buttons(&self) {
        let doc = self.doc();
        let Some(menu) = doc.current_menu() else { return };
        if doc.project().titles.is_empty() {
            self.toast(&gettext("Import some videos first"));
            return;
        }
        doc.edit(Change::Structure, |p| {
            let titles: Vec<(Id, Id, String, f64)> =
                p.titles.iter().map(|t| (t.id, t.asset, t.name.clone(), p.title_poster(t.id))).collect();
            let rects = grid_layout(titles.len(), true);
            let Some(m) = p.menu_mut(menu) else { return };
            for ((tid, asset, name, poster), rect) in titles.into_iter().zip(rects) {
                let mut item = MenuItem::new_button(&name, Action::PlayTitle { title: tid, chapter: 0 }, rect);
                if let Some(b) = item.button_mut() {
                    b.thumbnail = Some(asset);
                    b.thumbnail_time = poster;
                    b.text.font = "Cantarell Bold 30".into();
                }
                m.items.push(item);
            }
        });
    }

    /// Create a chapter selection menu for the selected title.
    fn add_chapter_menu(&self) {
        let doc = self.doc();
        let Node::Title(tid) = doc.node() else { return };
        let menu_id = doc.edit(Change::Structure, |p| {
            let back_to = p.first_menu().map(|m| m.id);
            let pages = crate::templates::chapter_menus(p, tid, back_to);
            let first = pages.first().map(|m| m.id);
            p.menus.extend(pages);
            first
        });
        if let Some(id) = menu_id {
            doc.select(Node::Menu(id), None);
        }
    }

    // ---------------------------------------------------------------- files

    /// Autosave a recovery copy while there are unsaved changes, and offer
    /// to restore one left by a run that didn't close normally.
    fn setup_recovery(&self) {
        let win = self.downgrade();
        glib::timeout_add_seconds_local(30, move || {
            let Some(w) = win.upgrade() else { return glib::ControlFlow::Break };
            w.autosave();
            glib::ControlFlow::Continue
        });
        // Saving (or opening another project) makes the copy unnecessary.
        let win = self.downgrade();
        self.doc().connect(move |c| {
            if matches!(c, Change::File | Change::Structure) {
                if let Some(w) = win.upgrade().filter(|w| !w.doc().is_dirty()) {
                    crate::recovery::remove(w.state().session);
                    w.state().recovery_hash.set(0);
                }
            }
        });
        // Only the first window of a run looks for copies left behind (the
        // app is single-instance, so any copy found belongs to no one).
        static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let screenshot = std::env::var("SPINDLE_SCREENSHOT").is_ok() && std::env::var("SPINDLE_SCREENSHOT_RECOVERY").is_err();
        if CHECKED.swap(true, std::sync::atomic::Ordering::Relaxed) || screenshot {
            return;
        }
        let win = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(w) = win.upgrade() {
                w.offer_recovery();
            }
        });
    }

    fn autosave(&self) {
        let state = self.state();
        let doc = &state.doc;
        if !doc.is_dirty() {
            return;
        }
        let project = doc.project().clone();
        let json = serde_json::to_vec(&project).unwrap_or_default();
        let hash = json.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
        if hash == state.recovery_hash.get() {
            return;
        }
        match crate::recovery::write(state.session, &project, doc.path().as_deref()) {
            Ok(()) => state.recovery_hash.set(hash),
            Err(e) => log::warn!("autosave failed: {e:#}"),
        }
    }

    fn offer_recovery(&self) {
        let found = crate::recovery::leftovers();
        let Some(newest) = found.first().cloned() else { return };
        // Older copies are superseded by the newest one.
        for old in &found[1..] {
            old.discard();
        }
        let ago = newest.saved.elapsed().map(|d| d.as_secs() / 60).unwrap_or(0);
        let when = if ago < 1 {
            gettext("less than a minute ago")
        } else if ago < 120 {
            gettextrs::ngettext("{} minute ago", "{} minutes ago", ago as u32).replace("{}", &ago.to_string())
        } else {
            gettextrs::ngettext("{} hour ago", "{} hours ago", (ago / 60) as u32).replace("{}", &(ago / 60).to_string())
        };
        let name = if newest.name.is_empty() { gettext("a project") } else { format!("“{}”", newest.name) };
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Recover Unsaved Changes?"))
            .body(
                gettext("Spindle didn't close normally while {name} had unsaved changes. A copy was saved {when}.")
                    .replace("{name}", &name)
                    .replace("{when}", &when),
            )
            // Dismissing keeps the copy, to be offered again next time.
            .close_response("keep")
            .default_response("recover")
            .build();
        dialog.add_responses(&[("discard", &gettext("_Discard")), ("recover", &gettext("_Recover"))]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("recover", adw::ResponseAppearance::Suggested);
        let win = self.clone();
        dialog.connect_response(None, move |_, r| {
            if r == "recover" {
                match newest.load() {
                    Ok(project) => {
                        let doc = win.doc();
                        doc.replace(project, newest.original.clone());
                        // Still unsaved: the user decides where it goes.
                        doc.mark_dirty();
                        win.autosave();
                    }
                    Err(e) => {
                        win.toast(&format!("{}: {e}", gettext("Could not recover the project")));
                        return;
                    }
                }
            }
            if r != "keep" {
                newest.discard();
            }
        });
        dialog.present(Some(self));
    }

    /// Ask to save unsaved changes, then run `then`.
    pub fn confirm_discard(&self, then: impl Fn() + 'static) {
        let doc = self.doc();
        if !doc.is_dirty() {
            then();
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Save Changes?"))
            .body(gettext("The disc project has unsaved changes. Changes which are not saved will be permanently lost."))
            .close_response("cancel")
            .default_response("save")
            .build();
        dialog.add_responses(&[("cancel", &gettext("_Cancel")), ("discard", &gettext("_Discard")), ("save", &gettext("_Save"))]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        let win = self.clone();
        let then = Rc::new(then);
        dialog.choose(Some(self), gio::Cancellable::NONE, move |r| match r.as_str() {
            "discard" => then(),
            "save" => {
                let then = then.clone();
                win.save_then(false, move || then());
            }
            _ => {}
        });
    }

    fn open_dialog(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&gettext("Spindle Projects")));
        filter.add_pattern("*.spindle");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let fd = gtk::FileDialog::builder().title(gettext("Open Project")).filters(&filters).modal(true).build();
        let win = self.clone();
        fd.open(Some(self), gio::Cancellable::NONE, move |res| {
            if let Ok(f) = res {
                if let Some(p) = f.path() {
                    win.open_path(&p);
                }
            }
        });
    }

    pub fn open_path(&self, path: &std::path::Path) {
        let path = portal::real_path(path);
        match Project::load(&path) {
            Ok(mut p) => {
                // Older projects may hold document portal links; use real paths.
                for (id, media) in p.media_paths() {
                    if portal::is_portal_path(&media) {
                        let real = portal::real_path(&media);
                        if real != media {
                            p.relink(id, real);
                        }
                    }
                }
                self.doc().replace(p, Some(path.to_path_buf()));
            }
            Err(e) => self.toast(&format!("{}: {e}", gettext("Could not open project"))),
        }
    }

    /// Show or hide the banner about media files that can't be found.
    fn update_missing_banner(&self) {
        let missing = self.doc().project().missing_media().len() as u32;
        let banner = &self.imp().missing_banner;
        if missing > 0 {
            banner.set_title(&ngettext("{} media file can't be found", "{} media files can't be found", missing).replace("{}", &missing.to_string()));
        }
        banner.set_revealed(missing > 0);
    }

    /// Ask for a folder and relink missing files found in it by name.
    fn locate_media(&self) {
        let missing = self.doc().project().missing_media();
        if missing.is_empty() {
            return;
        }
        let first = missing[0].1.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fd = gtk::FileDialog::builder()
            .title(gettext("Choose the Folder Containing “{}”").replace("{}", &first))
            .modal(true)
            .build();
        let win = self.clone();
        fd.select_folder(Some(self), gio::Cancellable::NONE, move |res| {
            let Some(dir) = res.ok().and_then(|f| f.path()).map(|p| portal::real_path(&p)) else { return };
            let win2 = win.clone();
            glib::spawn_future_local(async move {
                let names: Vec<String> =
                    missing.iter().filter_map(|(_, p)| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                let found = gio::spawn_blocking(move || portal::find_by_name(&dir, &names, 4)).await.unwrap_or_default();
                let doc = win2.doc();
                let mut relinked = 0;
                doc.edit(Change::Structure, |p| {
                    for (id, old) in &missing {
                        let name = old.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        if let Some((_, new)) = found.iter().find(|(n, _)| *n == name) {
                            p.relink(*id, new.clone());
                            relinked += 1;
                        }
                    }
                });
                let total = missing.len();
                win2.toast(&if relinked == total {
                    ngettext("Found {} file", "Found all {} files", relinked as u32).replace("{}", &relinked.to_string())
                } else {
                    gettext("Found {} of {} files").replacen("{}", &relinked.to_string(), 1).replacen("{}", &total.to_string(), 1)
                });
            });
        });
    }

    fn save(&self, ask: bool) {
        self.save_then(ask, || {});
    }

    fn save_then(&self, ask: bool, then: impl Fn() + 'static) {
        let doc = self.doc();
        let write = {
            let win = self.clone();
            move |path: PathBuf| {
                let doc = win.doc();
                let path = if path.extension().is_none() { path.with_extension("spindle") } else { path };
                let res = doc.project().save(&path);
                match res {
                    Ok(()) => {
                        doc.set_saved(path);
                        win.toast(&gettext("Project saved"));
                        true
                    }
                    Err(e) => {
                        win.toast(&format!("{}: {e}", gettext("Could not save")));
                        false
                    }
                }
            }
        };
        match doc.path() {
            Some(p) if !ask => {
                if write(p) {
                    then();
                }
            }
            _ => {
                let fd = gtk::FileDialog::builder()
                    .title(gettext("Save Project"))
                    .initial_name(format!("{}.spindle", doc.project().disc.name))
                    .modal(true)
                    .build();
                fd.save(Some(self), gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        if write(p) {
                            then();
                        }
                    }
                });
            }
        }
    }

    fn import_dialog(&self) {
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        let all = gtk::FileFilter::new();
        all.set_name(Some(&gettext("Video, Image and Audio Files")));
        for m in ["video/*", "image/*", "audio/*"] {
            all.add_mime_type(m);
        }
        filters.append(&all);
        let fd = gtk::FileDialog::builder().title(gettext("Import Media")).filters(&filters).modal(true).build();
        if let Some(v) = glib::user_special_dir(glib::UserDirectory::Videos) {
            fd.set_initial_folder(Some(&gio::File::for_path(v)));
        }
        let win = self.clone();
        fd.open_multiple(Some(self), gio::Cancellable::NONE, move |res| {
            if let Ok(list) = res {
                let files: Vec<gio::File> = list.iter::<gio::File>().filter_map(Result::ok).collect();
                win.import_files(files);
            }
        });
    }

    /// Let the user pick an image file, import it, and call `done` with
    /// its asset id.
    pub fn pick_image(&self, done: impl FnOnce(Id) + 'static) {
        self.pick_media(&gettext("Choose an Image"), &gettext("Images"), &["image/*"], glib::UserDirectory::Pictures, done);
    }

    /// Pick an audio file (or a video, for its soundtrack) and import it.
    pub fn pick_audio(&self, done: impl FnOnce(Id) + 'static) {
        self.pick_media(&gettext("Choose Menu Music"), &gettext("Audio"), &["audio/*", "video/*"], glib::UserDirectory::Music, done);
    }

    /// Pick one file matching `mimes`, import it as an asset and call
    /// `done` with its id.
    fn pick_media(&self, title: &str, filter_name: &str, mimes: &[&str], start: glib::UserDirectory, done: impl FnOnce(Id) + 'static) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(filter_name));
        for m in mimes {
            filter.add_mime_type(m);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let fd = gtk::FileDialog::builder().title(title).filters(&filters).modal(true).build();
        if let Some(p) = glib::user_special_dir(start) {
            fd.set_initial_folder(Some(&gio::File::for_path(p)));
        }
        let win = self.clone();
        fd.open(Some(self), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                win.import_files_then(vec![file], move |ids| {
                    if let Some(id) = ids.first() {
                        done(*id);
                    }
                });
            }
        });
    }

    fn image_aspect(&self, asset: Id) -> f64 {
        let doc = self.doc();
        let p = doc.project();
        p.asset(asset).filter(|a| a.info.height > 0).map_or(1.0, |a| a.info.width as f64 / a.info.height as f64)
    }

    /// Replace the selected text (or image) item with a picked image that
    /// fills the same area, e.g. a show logo instead of its name.
    fn replace_with_image(&self) {
        let doc = self.doc();
        let (Some(menu), Some(item)) = (doc.current_menu(), doc.item()) else { return };
        let win = self.clone();
        self.pick_image(move |asset| {
            let aspect = win.image_aspect(asset);
            let doc = win.doc();
            doc.edit(Change::Structure, |p| {
                if let Some(it) = p.menu_mut(menu).and_then(|m| m.item_mut(item)) {
                    let align = match &it.kind {
                        ItemKind::Text(t) => t.style.align,
                        _ => Align::Center,
                    };
                    it.rect = it.rect.fit(aspect, align);
                    it.kind = ItemKind::Image(ImageItem::new(asset));
                }
            });
            doc.select_item(Some(item));
        });
    }

    fn add_subtitle_dialog(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&gettext("Subtitles (SRT, ASS, SSA, WebVTT, PGS)")));
        for ext in SUBTITLE_EXTENSIONS {
            filter.add_suffix(ext);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let doc = self.doc();
        let start = match doc.node() {
            Node::Title(t) => doc.project().title(t).and_then(|t| doc.project().asset(t.asset).and_then(|a| a.path.parent().map(|p| p.to_path_buf()))),
            _ => None,
        };
        let fd = gtk::FileDialog::builder().title(gettext("Add Subtitle Files")).filters(&filters).modal(true).build();
        if let Some(dir) = start {
            fd.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let win = self.clone();
        fd.open_multiple(Some(self), gio::Cancellable::NONE, move |res| {
            if let Ok(list) = res {
                let paths: Vec<PathBuf> = list
                    .iter::<gio::File>()
                    .filter_map(Result::ok)
                    .filter_map(|f| f.path())
                    .map(|p| portal::real_path(&p))
                    .collect();
                win.attach_subtitles(paths);
            }
        });
    }

    /// Attach subtitle files to titles: to the title whose video name they
    /// match, else to the selected title.
    fn attach_subtitles(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let doc = self.doc();
        let selected = match doc.node() {
            Node::Title(t) => Some(t),
            _ => None,
        };
        let mut attached = 0;
        let mut orphans = 0;
        doc.edit(Change::Structure, |p| {
            for path in paths {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let matching = p.titles.iter().find_map(|t| {
                    let a = p.asset(t.asset)?;
                    let stem = a.path.file_stem()?.to_string_lossy().to_string();
                    (name.starts_with(&format!("{stem}.")) && !stem.is_empty()).then_some((t.id, a.path.clone()))
                });
                let target = matching.or_else(|| selected.and_then(|s| p.title(s).and_then(|t| p.asset(t.asset)).map(|a| (s, a.path.clone()))));
                let Some((tid, video)) = target else {
                    orphans += 1;
                    continue;
                };
                let Some(t) = p.title_mut(tid) else { continue };
                if t.subtitles.iter().any(|s| s.source == SubtitleSource::External { path: path.clone() }) {
                    continue;
                }
                t.subtitles.push(external_track(&path, Some(&video)));
                attached += 1;
            }
        });
        if orphans > 0 {
            self.toast(&gettext("Select a title to add subtitles to"));
        } else if attached > 0 {
            self.toast(&ngettext("Added {} subtitle track", "Added {} subtitle tracks", attached).replace("{}", &attached.to_string()));
        }
    }

    pub fn import_files(&self, files: Vec<gio::File>) {
        self.import_files_then(files, |_| {});
    }

    /// Import files as assets, then call `done` with the ids of the assets
    /// (newly added or already present).
    pub fn import_files_then(&self, files: Vec<gio::File>, done: impl FnOnce(Vec<Id>) + 'static) {
        // Store real paths, not Flatpak document portal links.
        let all: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).map(|p| portal::real_path(&p)).collect();
        let is_sub = |p: &PathBuf| {
            p.extension().is_some_and(|e| SUBTITLE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        };
        let (subs, paths): (Vec<PathBuf>, Vec<PathBuf>) = all.into_iter().partition(is_sub);
        if paths.is_empty() {
            self.attach_subtitles(subs);
            return;
        }
        let win = self.downgrade();
        glib::spawn_future_local(async move {
            let results = gio::spawn_blocking(move || {
                paths.into_iter().map(|p| {
                    let r = probe::probe(&p);
                    (p, r)
                }).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let Some(w) = win.upgrade() else { return };
            let doc = w.doc();
            let mut errors = Vec::new();
            let mut first_title = None;
            let mut ids = Vec::new();
            let mut added = 0;
            doc.edit(Change::Structure, |p| {
                for (path, res) in results {
                    let info = match res {
                        Ok(i) => i,
                        Err(e) => {
                            errors.push(format!("{e:#}"));
                            continue;
                        }
                    };
                    if let Some(a) = p.assets.iter().find(|a| a.path == path) {
                        ids.push(a.id);
                        continue;
                    }
                    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    let kind = if IMAGE_EXT.contains(&ext.as_str()) || (info.has_video() && info.duration < 0.2) {
                        AssetKind::Image
                    } else if info.has_video() || VIDEO_EXT.contains(&ext.as_str()) {
                        AssetKind::Video
                    } else if info.has_audio() || AUDIO_EXT.contains(&ext.as_str()) {
                        AssetKind::Audio
                    } else {
                        errors.push(format!("{}: {}", path.display(), gettext("unsupported file")));
                        continue;
                    };
                    let id = new_id();
                    p.assets.push(Asset { id, path, kind, info });
                    ids.push(id);
                    added += 1;
                    if kind == AssetKind::Video {
                        let t = p.ensure_title_for(id);
                        first_title = first_title.or(t);
                    }
                }
            });
            if let Some(e) = errors.first() {
                w.toast(e);
            } else if added > 0 {
                let msg = ngettext("Imported {} file", "Imported {} files", added as u32).replace("{}", &added.to_string());
                let empty = doc.project().menus.iter().all(|m| m.items.is_empty());
                let toast = adw::Toast::new(&msg);
                if empty && !doc.project().titles.is_empty() {
                    toast.set_button_label(Some(&gettext("Use a _Template")));
                    toast.set_action_name(Some("win.templates"));
                    toast.set_timeout(8);
                }
                w.imp().toast_overlay.add_toast(toast);
            }
            if doc.node() == Node::None {
                if let Some(t) = first_title {
                    doc.select(Node::Title(t), None);
                }
            }
            // Subtitle files dropped with their videos attach to the new titles.
            w.attach_subtitles(subs);
            done(ids);
        });
    }
}
