// SPDX-License-Identifier: GPL-3.0-or-later

//! The menu editing canvas: WYSIWYG view of a menu where items can be
//! selected (click, shift/ctrl-click, rubber band), dragged, resized and
//! dropped in from the media bin or the file manager.

use crate::document::{Change, Document, Node};
use crate::model::*;
use crate::render::{self, ButtonState, ImageCache};
use gettextrs::{gettext, ngettext};
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const HANDLE: f64 = 9.0;
const SNAP: f64 = 10.0;
/// Grid spacing in design pixels.
const GRID: f64 = 20.0;
const MAX_ZOOM: f64 = 8.0;

#[derive(Clone)]
enum Drag {
    /// Moving the selection; start rects of all moved items.
    Move { starts: Vec<(Id, Rect)>, moved: bool },
    /// Resizing one item with handle 0..8 (clockwise from top-left).
    Resize { item: Id, handle: u8, start: Rect, moved: bool },
    /// Rubber band selection in design coordinates.
    Marquee { x0: f64, y0: f64, x1: f64, y1: f64, base: Vec<Id> },
    /// Alt+drag from a button to another: sets where an arrow key leads.
    Link { from: Id, x0: f64, y0: f64, x1: f64, y1: f64 },
}

#[derive(Clone, Copy)]
enum Guide {
    V(f64),
    H(f64),
}

type FileDropHandler = Rc<dyn Fn(Vec<gio::File>, f64, f64)>;
type MessageHandler = Rc<dyn Fn(String)>;

struct Inner {
    doc: Rc<Document>,
    images: Rc<ImageCache>,
    area: gtk::DrawingArea,
    drag: RefCell<Option<Drag>>,
    hover: Cell<Option<Id>>,
    guides: RefCell<Vec<Guide>>,
    /// Pointer position while something is dragged over the canvas.
    drop_hover: Cell<Option<(f64, f64)>>,
    on_files: RefCell<Option<FileDropHandler>>,
    show_safe_area: Cell<bool>,
    /// Remote-control preview: arrow keys move a highlight like a player.
    preview: Cell<bool>,
    /// Button highlighted in preview mode, and whether it is activated.
    preview_button: Cell<Option<Id>>,
    preview_activated: Cell<bool>,
    on_message: RefCell<Option<MessageHandler>>,
    /// Zoom relative to fitting the whole menu (1.0), and the pan offset in
    /// widget pixels.
    zoom: Cell<f64>,
    pan: Cell<(f64, f64)>,
    /// Snap to a grid (and draw it).
    grid: Cell<bool>,
}

#[derive(Clone)]
pub struct MenuCanvas {
    inner: Rc<Inner>,
}

struct View {
    scale: f64,
    ox: f64,
    oy: f64,
}

impl View {
    fn to_design(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.ox) / self.scale, (y - self.oy) / self.scale)
    }
}

fn handle_points(r: Rect) -> [(f64, f64); 8] {
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (xm, ym) = r.center();
    [(x0, y0), (xm, y0), (x1, y0), (x1, ym), (x1, y1), (xm, y1), (x0, y1), (x0, ym)]
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn accent(cr: &cairo::Context, alpha: f64) {
    cr.set_source_rgba(0.21, 0.52, 0.89, alpha);
}

impl MenuCanvas {
    pub fn new(doc: Rc<Document>, images: Rc<ImageCache>) -> Self {
        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .can_focus(true)
            .build();
        area.add_css_class("menu-canvas");
        area.update_property(&[gtk::accessible::Property::Label(&gettext("Menu canvas"))]);
        let inner = Rc::new(Inner {
            doc,
            images,
            area: area.clone(),
            drag: RefCell::new(None),
            hover: Cell::new(None),
            guides: RefCell::new(Vec::new()),
            drop_hover: Cell::new(None),
            on_files: RefCell::new(None),
            show_safe_area: Cell::new(true),
            preview: Cell::new(false),
            preview_button: Cell::new(None),
            preview_activated: Cell::new(false),
            on_message: RefCell::new(None),
            zoom: Cell::new(1.0),
            pan: Cell::new((0.0, 0.0)),
            grid: Cell::new(false),
        });
        let canvas = MenuCanvas { inner };
        canvas.setup();
        canvas
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.inner.area
    }

    pub fn queue_draw(&self) {
        self.inner.area.queue_draw();
    }

    pub fn set_show_safe_area(&self, v: bool) {
        self.inner.show_safe_area.set(v);
        self.queue_draw();
    }

    pub fn set_grid(&self, v: bool) {
        self.inner.grid.set(v);
        self.queue_draw();
    }

    /// Zoom by `factor` keeping the widget point (x, y) in place (the
    /// centre when None).
    pub fn zoom_by(&self, factor: f64, at: Option<(f64, f64)>) {
        let a = &self.inner.area;
        let (x, y) = at.unwrap_or((a.width() as f64 / 2.0, a.height() as f64 / 2.0));
        let before = self.view();
        let d = before.to_design(x, y);
        let zoom = (self.inner.zoom.get() * factor).clamp(1.0, MAX_ZOOM);
        self.inner.zoom.set(zoom);
        if zoom <= 1.0 {
            self.inner.pan.set((0.0, 0.0));
        } else {
            let after = self.view();
            let (px, py) = self.inner.pan.get();
            self.inner.pan.set((px + x - (after.ox + d.0 * after.scale), py + y - (after.oy + d.1 * after.scale)));
        }
        self.queue_draw();
    }

    pub fn zoom_fit(&self) {
        self.inner.zoom.set(1.0);
        self.inner.pan.set((0.0, 0.0));
        self.queue_draw();
    }

    fn pan_by(&self, dx: f64, dy: f64) {
        if self.inner.zoom.get() <= 1.0 {
            return;
        }
        let (px, py) = self.inner.pan.get();
        self.inner.pan.set((px + dx, py + dy));
        self.queue_draw();
    }

    /// Enter or leave remote-control preview mode.
    pub fn set_preview(&self, v: bool) {
        self.inner.preview.set(v);
        self.inner.preview_activated.set(false);
        self.inner.preview_button.set(None);
        if v {
            self.inner.doc.select_item(None);
            self.reset_preview_button();
            self.inner.area.grab_focus();
        }
        self.queue_draw();
    }

    pub fn is_preview(&self) -> bool {
        self.inner.preview.get()
    }

    /// Development aid: feed key presses as if typed on the canvas.
    #[cfg(debug_assertions)]
    pub fn debug_keys(&self, names: &str) {
        for name in names.split(',') {
            if let Some(key) = gdk::Key::from_name(name.trim()) {
                self.key(key, gdk::ModifierType::empty());
            }
        }
    }

    /// Called with a description of what activating a button would do.
    pub fn connect_message(&self, f: impl Fn(String) + 'static) {
        *self.inner.on_message.borrow_mut() = Some(Rc::new(f));
    }

    /// Highlight the menu's default (or first) button.
    pub fn reset_preview_button(&self) {
        let b = self
            .with_menu(|m| m.default_button.filter(|d| m.item(*d).is_some()).or_else(|| m.buttons().next().map(|i| i.id)))
            .flatten();
        self.inner.preview_button.set(b);
        self.queue_draw();
    }

    fn preview_key(&self, key: gdk::Key) -> glib::Propagation {
        let dir = match key {
            gdk::Key::Up | gdk::Key::KP_Up => Some(0),
            gdk::Key::Down | gdk::Key::KP_Down => Some(1),
            gdk::Key::Left | gdk::Key::KP_Left => Some(2),
            gdk::Key::Right | gdk::Key::KP_Right => Some(3),
            _ => None,
        };
        let current = self.inner.preview_button.get();
        if let Some(d) = dir {
            let next = self
                .with_menu(|m| {
                    let nav = m.navigation();
                    let idx = m.buttons().position(|b| Some(b.id) == current)?;
                    Some(nav[idx][d])
                })
                .flatten();
            if next.is_some() {
                self.inner.preview_button.set(next);
                self.inner.preview_activated.set(false);
                self.queue_draw();
            }
            return glib::Propagation::Stop;
        }
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                self.preview_activate();
                glib::Propagation::Stop
            }
            gdk::Key::Escape => {
                let _ = self.inner.area.activate_action("win.remote-preview", None);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn preview_activate(&self) {
        let Some(button) = self.inner.preview_button.get() else { return };
        let action = self.with_menu(|m| m.item(button).and_then(|i| i.button()).map(|b| b.action)).flatten();
        self.inner.preview_activated.set(true);
        self.queue_draw();
        let msg = {
            let p = self.inner.doc.project();
            match action {
                Some(Action::PlayTitle { title, chapter }) => {
                    let name = p.title(title).map(|t| t.name.clone()).unwrap_or_default();
                    Some(if chapter > 0 {
                        gettext("Would play “{}” from chapter {}").replacen("{}", &name, 1).replacen("{}", &(chapter + 1).to_string(), 1)
                    } else {
                        gettext("Would play “{}”").replace("{}", &name)
                    })
                }
                Some(Action::ShowMenu(m)) => {
                    drop(p);
                    // Follow the link, like the player would.
                    let this = self.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                        this.inner.doc.select(Node::Menu(m), None);
                        this.inner.preview_activated.set(false);
                        this.reset_preview_button();
                    });
                    None
                }
                Some(Action::SetLanguage { preset, menu }) => {
                    let name = p.language(preset).map(|l| l.name.clone()).unwrap_or_default();
                    drop(p);
                    if let Some(m) = menu {
                        let this = self.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                            this.inner.doc.select(Node::Menu(m), None);
                            this.inner.preview_activated.set(false);
                            this.reset_preview_button();
                        });
                    }
                    Some(gettext("Language set to “{}”").replace("{}", &name))
                }
                Some(Action::PlayChapter(c)) => Some(gettext("Would go to chapter {} of the playing title").replace("{}", &(c + 1).to_string())),
                Some(Action::PlayAll) => Some(
                    ngettext("Would play all {} title", "Would play all {} titles", p.titles.len() as u32)
                        .replace("{}", &p.titles.len().to_string()),
                ),
                _ => Some(gettext("This button has no action")),
            }
        };
        if let (Some(msg), Some(f)) = (msg, self.inner.on_message.borrow().clone()) {
            f(msg);
        }
        let this = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            this.inner.preview_activated.set(false);
            this.queue_draw();
        });
    }

    /// Called with files dropped from outside and the design-space drop point.
    pub fn connect_files_dropped(&self, f: impl Fn(Vec<gio::File>, f64, f64) + 'static) {
        *self.inner.on_files.borrow_mut() = Some(Rc::new(f));
    }

    fn view(&self) -> View {
        let a = &self.inner.area;
        let (w, h) = (a.width() as f64, a.height() as f64);
        let margin = 28.0;
        let fit = ((w - 2.0 * margin) / DESIGN_WIDTH).min((h - 2.0 * margin) / DESIGN_HEIGHT).max(0.05);
        let scale = fit * self.inner.zoom.get();
        let (px, py) = self.inner.pan.get();
        View {
            scale,
            ox: ((w - DESIGN_WIDTH * scale) / 2.0 + px).round(),
            oy: ((h - DESIGN_HEIGHT * scale) / 2.0 + py).round(),
        }
    }

    fn with_menu<R>(&self, f: impl FnOnce(&Menu) -> R) -> Option<R> {
        let id = self.inner.doc.current_menu()?;
        let p = self.inner.doc.project();
        p.menu(id).map(f)
    }

    fn hit(&self, dx: f64, dy: f64) -> Option<Id> {
        self.with_menu(|m| m.items.iter().rev().find(|i| !i.locked && i.rect.contains(dx, dy)).map(|i| i.id)).flatten()
    }

    fn hit_handle(&self, x: f64, y: f64) -> Option<(Id, u8)> {
        let doc = &self.inner.doc;
        if doc.selection().len() != 1 {
            return None;
        }
        let v = self.view();
        let sel = doc.item()?;
        let rect = self.with_menu(|m| m.item(sel).map(|i| i.rect)).flatten()?;
        for (i, (hx, hy)) in handle_points(rect).iter().enumerate() {
            let (sx, sy) = (v.ox + hx * v.scale, v.oy + hy * v.scale);
            if (x - sx).abs() <= HANDLE && (y - sy).abs() <= HANDLE {
                return Some((sel, i as u8));
            }
        }
        None
    }

    fn setup(&self) {
        let area = self.inner.area.clone();
        let this = self.clone();
        area.set_draw_func(move |_, cr, w, h| this.draw(cr, w as f64, h as f64));

        // Selection + move/resize/marquee
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let this = self.clone();
        drag.connect_drag_begin(move |g, x, y| this.drag_begin(g, x, y));
        let this = self.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let (sx, sy) = g.start_point().unwrap_or_default();
            this.drag_update(sx, sy, dx, dy)
        });
        let this = self.clone();
        drag.connect_drag_end(move |_, _, _| this.drag_end());
        area.add_controller(drag);

        // Ctrl+scroll zooms; scrolling pans while zoomed in.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let this = self.clone();
        scroll.connect_scroll(move |c, dx, dy| {
            let state = c.current_event_state();
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                let at = c.current_event().and_then(|e| e.position());
                this.zoom_by(if dy < 0.0 { 1.15 } else { 1.0 / 1.15 }, at);
                return glib::Propagation::Stop;
            }
            if this.inner.zoom.get() > 1.0 {
                let (dx, dy) = if state.contains(gdk::ModifierType::SHIFT_MASK) { (dy, dx) } else { (dx, dy) };
                this.pan_by(-dx * 40.0, -dy * 40.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        area.add_controller(scroll);
        // Middle-button drag pans.
        let pan = gtk::GestureDrag::new();
        pan.set_button(gdk::BUTTON_MIDDLE);
        let last: Rc<Cell<(f64, f64)>> = Rc::default();
        let l = last.clone();
        pan.connect_drag_begin(move |_, _, _| l.set((0.0, 0.0)));
        let this = self.clone();
        pan.connect_drag_update(move |_, dx, dy| {
            let (lx, ly) = last.get();
            this.pan_by(dx - lx, dy - ly);
            last.set((dx, dy));
        });
        area.add_controller(pan);

        // Double click edits the label
        let dbl = gtk::GestureClick::new();
        dbl.set_button(gdk::BUTTON_PRIMARY);
        let this = self.clone();
        dbl.connect_pressed(move |_, n, x, y| {
            if n == 2 && !this.inner.preview.get() {
                let (dx, dy) = this.view().to_design(x, y);
                if this.hit(dx, dy).is_some() {
                    let _ = this.inner.area.activate_action("win.edit-label", None);
                }
            }
        });
        area.add_controller(dbl);

        // Hover + cursor
        let motion = gtk::EventControllerMotion::new();
        let this = self.clone();
        motion.connect_motion(move |_, x, y| this.motion(x, y));
        let this = self.clone();
        motion.connect_leave(move |_| {
            this.inner.hover.set(None);
            this.queue_draw();
        });
        area.add_controller(motion);

        // Context menu
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let this = self.clone();
        click.connect_pressed(move |_, _, x, y| this.context_menu(x, y));
        area.add_controller(click);

        // Keyboard
        let keys = gtk::EventControllerKey::new();
        let this = self.clone();
        keys.connect_key_pressed(move |_, key, _, state| this.key(key, state));
        area.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let this = self.clone();
        focus.connect_enter(move |_| this.queue_draw());
        let this = self.clone();
        focus.connect_leave(move |_| this.queue_draw());
        area.add_controller(focus);

        // Drops: asset ids from the media bin, or files from outside.
        let drop = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        drop.set_types(&[glib::Type::STRING, gdk::FileList::static_type()]);
        let this = self.clone();
        drop.connect_enter(move |_, x, y| {
            this.inner.drop_hover.set(Some((x, y)));
            this.queue_draw();
            gdk::DragAction::COPY
        });
        let this = self.clone();
        drop.connect_motion(move |_, x, y| {
            this.inner.drop_hover.set(Some((x, y)));
            this.queue_draw();
            gdk::DragAction::COPY
        });
        let this = self.clone();
        drop.connect_leave(move |_| {
            this.inner.drop_hover.set(None);
            this.queue_draw();
        });
        let this = self.clone();
        drop.connect_drop(move |_, value, x, y| {
            this.inner.drop_hover.set(None);
            this.queue_draw();
            if let Ok(files) = value.get::<gdk::FileList>() {
                let (dx, dy) = this.view().to_design(x, y);
                let handler = this.inner.on_files.borrow().clone();
                if let Some(h) = handler {
                    h(files.files(), dx, dy);
                    return true;
                }
                return false;
            }
            let Some(id) = value.get::<String>().ok().and_then(|s| s.parse::<Id>().ok()) else { return false };
            let (dx, dy) = this.view().to_design(x, y);
            this.place_assets(&[id], dx, dy)
        });
        area.add_controller(drop);
    }

    fn drag_begin(&self, g: &gtk::GestureDrag, x: f64, y: f64) {
        self.inner.area.grab_focus();
        let doc = &self.inner.doc;
        if doc.current_menu().is_none() {
            return;
        }
        let v = self.view();
        let (dx, dy) = v.to_design(x, y);
        if self.inner.preview.get() {
            g.set_state(gtk::EventSequenceState::Denied);
            let hit = self.with_menu(|m| m.items.iter().rev().find(|i| i.button().is_some() && i.rect.contains(dx, dy)).map(|i| i.id)).flatten();
            if let Some(b) = hit {
                if self.inner.preview_button.get() == Some(b) {
                    self.preview_activate();
                } else {
                    self.inner.preview_button.set(Some(b));
                    self.queue_draw();
                }
            }
            return;
        }
        let state = g.current_event_state();
        let extend = state.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK);

        if state.contains(gdk::ModifierType::ALT_MASK) {
            let button = self.with_menu(|m| m.buttons().rev().find(|i| i.rect.contains(dx, dy)).map(|i| i.id)).flatten();
            if let Some(from) = button {
                doc.select_item(Some(from));
                *self.inner.drag.borrow_mut() = Some(Drag::Link { from, x0: dx, y0: dy, x1: dx, y1: dy });
                return;
            }
        }

        if let Some((item, handle)) = self.hit_handle(x, y) {
            let start = self.with_menu(|m| m.item(item).map(|i| i.rect)).flatten().unwrap();
            *self.inner.drag.borrow_mut() = Some(Drag::Resize { item, handle, start, moved: false });
            return;
        }
        match self.hit(dx, dy) {
            Some(item) if extend => {
                doc.toggle_item(item);
                g.set_state(gtk::EventSequenceState::Denied);
            }
            Some(item) => {
                if !doc.is_selected(item) {
                    // Grouped items come along, with the clicked one first.
                    let mut group = self.with_menu(|m| m.with_groups(&[item])).unwrap_or_default();
                    group.retain(|i| *i != item);
                    group.insert(0, item);
                    doc.set_selection(group);
                } else if doc.item() != Some(item) {
                    // Make the grabbed item primary, keep the group.
                    let mut sel = doc.selection();
                    sel.retain(|i| *i != item);
                    sel.insert(0, item);
                    doc.set_selection(sel);
                }
                let sel = doc.selection();
                let starts = self
                    .with_menu(|m| m.items.iter().filter(|i| sel.contains(&i.id)).map(|i| (i.id, i.rect)).collect())
                    .unwrap_or_default();
                *self.inner.drag.borrow_mut() = Some(Drag::Move { starts, moved: false });
            }
            None => {
                let base = if extend { doc.selection() } else { vec![] };
                if !extend {
                    doc.select_item(None);
                }
                *self.inner.drag.borrow_mut() = Some(Drag::Marquee { x0: dx, y0: dy, x1: dx, y1: dy, base });
            }
        }
    }

    fn snap_targets(&self, skip: &[Id]) -> (Vec<f64>, Vec<f64>) {
        let mut xs = vec![0.0, DESIGN_WIDTH / 2.0, DESIGN_WIDTH, SAFE_AREA.x, SAFE_AREA.x + SAFE_AREA.w];
        let mut ys = vec![0.0, DESIGN_HEIGHT / 2.0, DESIGN_HEIGHT, SAFE_AREA.y, SAFE_AREA.y + SAFE_AREA.h];
        self.with_menu(|m| {
            for i in m.items.iter().filter(|i| !skip.contains(&i.id)) {
                let r = i.rect;
                xs.extend([r.x, r.x + r.w / 2.0, r.x + r.w]);
                ys.extend([r.y, r.y + r.h / 2.0, r.y + r.h]);
            }
        });
        (xs, ys)
    }

    /// Snap the best of `edges` to a target; returns (correction, target).
    fn snap(edges: &[f64], targets: &[f64], threshold: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        for e in edges {
            for t in targets {
                let d = t - e;
                if d.abs() <= threshold && best.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                    best = Some((d, *t));
                }
            }
        }
        best
    }

    fn drag_update(&self, sx: f64, sy: f64, ox: f64, oy: f64) {
        let Some(mut drag) = self.inner.drag.borrow().clone() else { return };
        let doc = &self.inner.doc;
        let Some(menu) = doc.current_menu() else { return };
        let v = self.view();
        let (ddx, ddy) = (ox / v.scale, oy / v.scale);
        let threshold = SNAP / v.scale.max(0.2) * 0.5;
        let mut guides = Vec::new();

        // Start of a real drag: one undo step for the whole gesture.
        let begin = |moved: &mut bool| -> bool {
            if !*moved {
                if ddx.abs() < 2.0 && ddy.abs() < 2.0 {
                    return false;
                }
                *moved = true;
                doc.checkpoint();
            }
            true
        };

        match &mut drag {
            Drag::Move { starts, moved } => {
                if !begin(moved) {
                    return;
                }
                let ids: Vec<Id> = starts.iter().map(|(id, _)| *id).collect();
                let bb = bounds(starts.iter().map(|(_, r)| *r)).unwrap();
                let (xs, ys) = self.snap_targets(&ids);
                let mut mx = ddx.clamp(-bb.x, DESIGN_WIDTH - bb.x - bb.w);
                let mut my = ddy.clamp(-bb.y, DESIGN_HEIGHT - bb.y - bb.h);
                let nb = Rect::new(bb.x + mx, bb.y + my, bb.w, bb.h);
                let grid = self.inner.grid.get();
                match Self::snap(&[nb.x, nb.x + nb.w / 2.0, nb.x + nb.w], &xs, threshold) {
                    Some((corr, t)) => {
                        mx += corr;
                        guides.push(Guide::V(t));
                    }
                    None if grid => mx = (nb.x / GRID).round() * GRID - bb.x,
                    None => {}
                }
                match Self::snap(&[nb.y, nb.y + nb.h / 2.0, nb.y + nb.h], &ys, threshold) {
                    Some((corr, t)) => {
                        my += corr;
                        guides.push(Guide::H(t));
                    }
                    None if grid => my = (nb.y / GRID).round() * GRID - bb.y,
                    None => {}
                }
                let starts = starts.clone();
                doc.edit_silent(Change::Content, |p| {
                    if let Some(m) = p.menu_mut(menu) {
                        for (id, r) in &starts {
                            if let Some(i) = m.item_mut(*id) {
                                i.rect = Rect::new((r.x + mx).round(), (r.y + my).round(), r.w, r.h).clamped();
                            }
                        }
                    }
                });
            }
            Drag::Resize { item, handle, start, moved } => {
                if !begin(moved) {
                    return;
                }
                let (xs, ys) = self.snap_targets(&[*item]);
                let s = *start;
                let h = *handle;
                let (mut x0, mut y0, mut x1, mut y1) = (s.x, s.y, s.x + s.w, s.y + s.h);
                let grid = self.inner.grid.get();
                let mut snap_edge = |v: &mut f64, targets: &[f64], vertical: bool| match Self::snap(&[*v], targets, threshold) {
                    Some((corr, t)) => {
                        *v += corr;
                        guides.push(if vertical { Guide::V(t) } else { Guide::H(t) });
                    }
                    None if grid => *v = (*v / GRID).round() * GRID,
                    None => {}
                };
                if matches!(h, 0 | 6 | 7) {
                    x0 = (s.x + ddx).min(x1 - 16.0);
                    snap_edge(&mut x0, &xs, true);
                }
                if matches!(h, 2..=4) {
                    x1 = (s.x + s.w + ddx).max(x0 + 16.0);
                    snap_edge(&mut x1, &xs, true);
                }
                if matches!(h, 0..=2) {
                    y0 = (s.y + ddy).min(y1 - 16.0);
                    snap_edge(&mut y0, &ys, false);
                }
                if matches!(h, 4..=6) {
                    y1 = (s.y + s.h + ddy).max(y0 + 16.0);
                    snap_edge(&mut y1, &ys, false);
                }
                let (x0, y0) = (x0.max(0.0), y0.max(0.0));
                let r = Rect::new(
                    x0.round(),
                    y0.round(),
                    (x1.min(DESIGN_WIDTH) - x0).max(16.0).round(),
                    (y1.min(DESIGN_HEIGHT) - y0).max(16.0).round(),
                );
                let item = *item;
                doc.edit_silent(Change::Content, |p| {
                    if let Some(i) = p.menu_mut(menu).and_then(|m| m.item_mut(item)) {
                        i.rect = r;
                    }
                });
            }
            Drag::Link { x1, y1, .. } => {
                let (ex, ey) = v.to_design(sx + ox, sy + oy);
                *x1 = ex;
                *y1 = ey;
                *self.inner.drag.borrow_mut() = Some(drag);
                self.queue_draw();
                return;
            }
            Drag::Marquee { x0, y0, x1, y1, base } => {
                let (ex, ey) = v.to_design(sx + ox, sy + oy);
                *x1 = ex;
                *y1 = ey;
                let band = Rect::new(x0.min(*x1), y0.min(*y1), (*x1 - *x0).abs(), (*y1 - *y0).abs());
                let mut sel = base.clone();
                if let Some(hits) = self.with_menu(|m| {
                    m.with_groups(&m.items.iter().filter(|i| !i.locked && intersects(i.rect, band)).map(|i| i.id).collect::<Vec<_>>())
                }) {
                    for h in hits {
                        if !sel.contains(&h) {
                            sel.push(h);
                        }
                    }
                }
                doc.set_selection(sel);
                self.queue_draw();
            }
        }
        *self.inner.guides.borrow_mut() = guides;
        *self.inner.drag.borrow_mut() = Some(drag);
    }

    fn drag_end(&self) {
        let drag = self.inner.drag.borrow_mut().take();
        if let Some(Drag::Link { from, x0, y0, x1, y1 }) = drag {
            self.link_buttons(from, x0, y0, x1, y1);
        }
        self.inner.guides.borrow_mut().clear();
        self.queue_draw();
    }

    /// Make the arrow key in the direction of the drag lead from `from`
    /// to the button under the end point.
    fn link_buttons(&self, from: Id, x0: f64, y0: f64, x1: f64, y1: f64) {
        let doc = &self.inner.doc;
        let Some(menu) = doc.current_menu() else { return };
        let target = self.with_menu(|m| m.buttons().rev().find(|i| i.id != from && i.rect.contains(x1, y1)).map(|i| i.id)).flatten();
        let Some(target) = target else { return };
        let (dx, dy) = (x1 - x0, y1 - y0);
        doc.edit(Change::Structure, |p| {
            if let Some(b) = p.menu_mut(menu).and_then(|m| m.item_mut(from)).and_then(|i| i.button_mut()) {
                let slot = if dx.abs() > dy.abs() {
                    if dx > 0.0 { &mut b.nav.right } else { &mut b.nav.left }
                } else if dy > 0.0 {
                    &mut b.nav.down
                } else {
                    &mut b.nav.up
                };
                *slot = Some(target);
            }
        });
        let name = self.with_menu(|m| m.item(target).and_then(|i| i.button()).map(|b| b.label.clone())).flatten().unwrap_or_default();
        let dir = if dx.abs() > dy.abs() {
            if dx > 0.0 { gettext("Right") } else { gettext("Left") }
        } else if dy > 0.0 {
            gettext("Down")
        } else {
            gettext("Up")
        };
        if let Some(f) = self.inner.on_message.borrow().clone() {
            f(gettext("{dir} now leads to “{name}”").replace("{dir}", &dir).replace("{name}", &name));
        }
    }

    fn motion(&self, x: f64, y: f64) {
        if self.inner.preview.get() {
            let (dx, dy) = self.view().to_design(x, y);
            let over = self.with_menu(|m| m.buttons().any(|i| i.rect.contains(dx, dy))).unwrap_or(false);
            self.inner.area.set_cursor_from_name(Some(if over { "pointer" } else { "default" }));
            return;
        }
        let v = self.view();
        let (dx, dy) = v.to_design(x, y);
        let hover = self.hit(dx, dy);
        let cursor = match self.hit_handle(x, y) {
            Some((_, h)) => ["nw-resize", "n-resize", "ne-resize", "e-resize", "se-resize", "s-resize", "sw-resize", "w-resize"][h as usize],
            None if hover.is_some() => "move",
            None => "default",
        };
        self.inner.area.set_cursor_from_name(Some(cursor));
        if hover != self.inner.hover.get() {
            self.inner.hover.set(hover);
            self.queue_draw();
        }
    }

    fn popup(&self, menu: &gio::Menu, x: f64, y: f64) {
        let pop = gtk::PopoverMenu::from_model(Some(menu));
        pop.set_parent(&self.inner.area);
        pop.set_has_arrow(false);
        pop.set_halign(gtk::Align::Start);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
    }

    fn context_menu(&self, x: f64, y: f64) {
        let doc = &self.inner.doc;
        if doc.current_menu().is_none() || self.inner.preview.get() {
            return;
        }
        self.inner.area.grab_focus();
        let (dx, dy) = self.view().to_design(x, y);
        let section = |items: &[(&str, &str)]| {
            let m = gio::Menu::new();
            for (label, action) in items {
                m.append(Some(&gettext(*label)), Some(action));
            }
            m
        };
        let menu = gio::Menu::new();
        let Some(item) = self.hit(dx, dy) else {
            doc.select_item(None);
            menu.append_section(
                None,
                &section(&[("Add _Button", "win.add-button"), ("Add _Text", "win.add-text"), ("Add _Image…", "win.add-image"), ("Add _Shape", "win.add-shape")]),
            );
            menu.append_section(None, &section(&[("_Paste", "win.paste"), ("Select _All", "win.select-all")]));
            if self.with_menu(|m| m.items.iter().any(|i| i.locked || i.hidden)).unwrap_or(false) {
                menu.append_section(None, &section(&[("_Unlock and Show All", "win.unlock-all")]));
            }
            self.popup(&menu, x, y);
            return;
        };
        if !doc.is_selected(item) {
            doc.select_item(Some(item));
        }
        let multi = doc.selection().len() > 1;
        let is_button = !multi && self.with_menu(|m| m.item(item).is_some_and(|i| i.button().is_some())).unwrap_or(false);

        menu.append_section(
            None,
            &section(&[("Cu_t", "win.cut"), ("_Copy", "win.copy"), ("_Paste", "win.paste"), ("_Duplicate", "win.duplicate-item")]),
        );
        let align = section(&[
            ("Align _Left", "win.align-left"),
            ("Center _Horizontally", "win.align-hcenter"),
            ("Align _Right", "win.align-right"),
            ("Align _Top", "win.align-top"),
            ("Center _Vertically", "win.align-vcenter"),
            ("Align _Bottom", "win.align-bottom"),
        ]);
        if doc.selection().len() > 2 {
            align.append_section(
                None,
                &section(&[("Distribute Hori_zontally", "win.distribute-h"), ("Distribute _Vertically", "win.distribute-v")]),
            );
        }
        let arrange = gio::Menu::new();
        arrange.append_submenu(Some(&gettext("_Align")), &align);
        arrange.append(Some(&gettext("Bring to _Front")), Some("win.raise-item"));
        arrange.append(Some(&gettext("Bring _Forward")), Some("win.forward-item"));
        arrange.append(Some(&gettext("Send Back_ward")), Some("win.backward-item"));
        arrange.append(Some(&gettext("Send to _Back")), Some("win.lower-item"));
        menu.append_section(None, &arrange);
        menu.append_section(None, &section(&[("_Group", "win.group-items"), ("U_ngroup", "win.ungroup-items")]));
        menu.append_section(None, &section(&[("_Lock", "win.lock-item"), ("_Hide", "win.hide-item")]));
        if is_button {
            menu.append_section(None, &section(&[("Make _Default Button", "win.default-button")]));
        } else if !multi {
            menu.append_section(None, &section(&[("Replace with _Image…", "win.replace-with-image")]));
        }
        menu.append_section(None, &section(&[("_Delete", "win.delete-item")]));
        self.popup(&menu, x, y);
    }

    fn key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let doc = &self.inner.doc;
        let Some(menu) = doc.current_menu() else { return glib::Propagation::Proceed };
        if self.inner.preview.get() {
            return self.preview_key(key);
        }
        let area = &self.inner.area;
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let act = |name: &str| {
            let _ = area.activate_action(name, None);
            glib::Propagation::Stop
        };
        if ctrl {
            return match key.to_lower() {
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                    self.zoom_by(1.25, None);
                    return glib::Propagation::Stop;
                }
                gdk::Key::minus | gdk::Key::KP_Subtract => {
                    self.zoom_by(0.8, None);
                    return glib::Propagation::Stop;
                }
                gdk::Key::_0 | gdk::Key::KP_0 => {
                    self.zoom_fit();
                    return glib::Propagation::Stop;
                }
                gdk::Key::g if state.contains(gdk::ModifierType::SHIFT_MASK) => act("win.ungroup-items"),
                gdk::Key::g => act("win.group-items"),
                gdk::Key::a => act("win.select-all"),
                gdk::Key::c => act("win.copy"),
                gdk::Key::x => act("win.cut"),
                gdk::Key::v => act("win.paste"),
                _ => glib::Propagation::Proceed,
            };
        }
        match key {
            gdk::Key::Tab | gdk::Key::ISO_Left_Tab if !self.with_menu(|m| m.items.is_empty()).unwrap_or(true) => {
                // Cycle through items.
                let back = key == gdk::Key::ISO_Left_Tab || state.contains(gdk::ModifierType::SHIFT_MASK);
                let next = self.with_menu(|m| {
                    let n = m.items.len();
                    let cur = doc.item().and_then(|id| m.items.iter().position(|i| i.id == id));
                    let idx = match (cur, back) {
                        (None, false) => 0,
                        (None, true) => n - 1,
                        (Some(i), false) => (i + 1) % n,
                        (Some(i), true) => (i + n - 1) % n,
                    };
                    m.items[idx].id
                });
                doc.select_item(next);
                return glib::Propagation::Stop;
            }
            gdk::Key::Delete | gdk::Key::BackSpace if doc.item().is_some() => return act("win.delete-item"),
            gdk::Key::Return | gdk::Key::KP_Enter if doc.item().is_some() => return act("win.edit-label"),
            gdk::Key::Escape if doc.item().is_some() => {
                doc.select_item(None);
                return glib::Propagation::Stop;
            }
            _ => {}
        }
        let sel = doc.selection();
        if sel.is_empty() {
            return glib::Propagation::Proceed;
        }
        let step = if state.contains(gdk::ModifierType::SHIFT_MASK) { 10.0 } else { 1.0 };
        let (dx, dy) = match key {
            gdk::Key::Left => (-step, 0.0),
            gdk::Key::Right => (step, 0.0),
            gdk::Key::Up => (0.0, -step),
            gdk::Key::Down => (0.0, step),
            _ => return glib::Propagation::Proceed,
        };
        doc.edit(Change::Content, |p| {
            if let Some(m) = p.menu_mut(menu) {
                for id in &sel {
                    if let Some(i) = m.item_mut(*id) {
                        i.rect = Rect::new(i.rect.x + dx, i.rect.y + dy, i.rect.w, i.rect.h).clamped();
                    }
                }
            }
        });
        glib::Propagation::Stop
    }

    /// Size of the item created when an asset is dropped.
    fn drop_size(kind: Option<AssetKind>) -> (f64, f64) {
        match kind {
            Some(AssetKind::Image) => (400.0, 260.0),
            _ => (480.0, 330.0),
        }
    }

    /// Create items for dropped assets around design point (dx, dy).
    /// Videos become thumbnail buttons, images become image items and
    /// audio becomes the menu's music.
    pub fn place_assets(&self, assets: &[Id], dx: f64, dy: f64) -> bool {
        let doc = &self.inner.doc;
        let Some(menu) = doc.current_menu() else { return false };
        let new_items = doc.edit(Change::Structure, |p| -> Vec<Id> {
            let mut out = Vec::new();
            for (n, &asset) in assets.iter().enumerate() {
                let Some(kind) = p.asset(asset).map(|a| a.kind) else { continue };
                let offset = n as f64 * 40.0;
                let (cx, cy) = (dx + offset, dy + offset);
                let item = match kind {
                    AssetKind::Video => {
                        let Some(title) = p.ensure_title_for(asset) else { continue };
                        let name = p.title(title).map(|t| t.name.clone()).unwrap_or_default();
                        let (w, h) = Self::drop_size(Some(kind));
                        let rect = Rect::new(cx - w / 2.0, cy - h / 2.0, w, h).clamped();
                        let mut item = MenuItem::new_button(&name, Action::PlayTitle { title, chapter: 0 }, rect);
                        let poster = p.title_poster(title);
                        if let Some(b) = item.button_mut() {
                            b.thumbnail = Some(asset);
                            b.thumbnail_time = poster;
                            b.text.font = "Cantarell Bold 32".into();
                        }
                        item
                    }
                    AssetKind::Image => {
                        let a = p.asset(asset).unwrap();
                        let aspect = if a.info.height > 0 { a.info.width as f64 / a.info.height as f64 } else { 1.0 };
                        let w = 400.0;
                        let h = (w / aspect).clamp(40.0, 800.0);
                        MenuItem::new_image(asset, Rect::new(cx - w / 2.0, cy - h / 2.0, w, h).clamped())
                    }
                    AssetKind::Audio => {
                        if let Some(m) = p.menu_mut(menu) {
                            m.audio = Some(asset);
                        }
                        continue;
                    }
                };
                out.push(item.id);
                if let Some(m) = p.menu_mut(menu) {
                    m.items.push(item);
                }
            }
            out
        });
        if !new_items.is_empty() {
            doc.select_many(Node::Menu(menu), new_items);
        }
        true
    }

    fn draw(&self, cr: &cairo::Context, w: f64, h: f64) {
        let doc = &self.inner.doc;
        let p = doc.project();
        let Some(menu) = doc.current_menu().and_then(|id| p.menu(id)) else { return };
        let v = self.view();
        let selection = doc.selection();

        cr.save().ok();
        cr.translate(v.ox, v.oy);
        cr.scale(v.scale, v.scale);

        // drop shadow
        for (off, alpha) in [(2.0, 0.10), (5.0, 0.08), (10.0, 0.05)] {
            cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
            cr.rectangle(-off / 2.0 / v.scale, off / v.scale, DESIGN_WIDTH + off / v.scale, DESIGN_HEIGHT);
            cr.fill().ok();
        }

        cr.save().ok();
        cr.rectangle(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT);
        cr.clip();
        let preview = self.inner.preview.get();
        let highlighted = if preview { self.inner.preview_button.get() } else { None };
        if menu.popup {
            render::popup_backdrop(cr, &p, &self.inner.images);
        }
        render::draw_static(cr, &p, menu, &self.inner.images, true);
        // Hidden items: a dashed outline so they can still be found.
        if !preview {
            for item in menu.items.iter().filter(|i| i.hidden) {
                let r = item.rect;
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.5);
                cr.set_line_width(1.5 / v.scale);
                cr.set_dash(&[4.0 / v.scale, 4.0 / v.scale], 0.0);
                cr.rectangle(r.x, r.y, r.w, r.h);
                cr.stroke().ok();
                cr.set_dash(&[], 0.0);
            }
        }
        for item in &menu.items {
            if let Some(b) = item.button() {
                let state = match (Some(item.id) == highlighted, self.inner.preview_activated.get()) {
                    (true, true) => ButtonState::Activated,
                    (true, false) => ButtonState::Selected,
                    _ => ButtonState::Normal,
                };
                render::draw_button(cr, &p, &self.inner.images, item, b, state);
            }
        }
        if preview {
            cr.restore().ok(); // clip
            cr.restore().ok();
            self.draw_hint(cr, w, h, &v, &gettext("Remote preview — use the arrow keys and Enter, Esc to leave"));
            return;
        }

        if self.inner.grid.get() {
            cr.set_line_width(1.0 / v.scale);
            let mut x = GRID;
            while x < DESIGN_WIDTH {
                cr.set_source_rgba(1.0, 1.0, 1.0, if (x % (GRID * 5.0)).abs() < 0.5 { 0.14 } else { 0.06 });
                cr.move_to(x, 0.0);
                cr.line_to(x, DESIGN_HEIGHT);
                cr.stroke().ok();
                x += GRID;
            }
            let mut y = GRID;
            while y < DESIGN_HEIGHT {
                cr.set_source_rgba(1.0, 1.0, 1.0, if (y % (GRID * 5.0)).abs() < 0.5 { 0.14 } else { 0.06 });
                cr.move_to(0.0, y);
                cr.line_to(DESIGN_WIDTH, y);
                cr.stroke().ok();
                y += GRID;
            }
        }
        if self.inner.show_safe_area.get() {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
            cr.set_line_width(1.5 / v.scale);
            cr.set_dash(&[6.0 / v.scale, 6.0 / v.scale], 0.0);
            let s = SAFE_AREA;
            cr.rectangle(s.x, s.y, s.w, s.h);
            cr.stroke().ok();
            cr.set_dash(&[], 0.0);
        }

        // Button link tags and default marker, for the buttons being worked
        // on (and any without an action, as a warning).
        for item in &menu.items {
            let Some(b) = item.button() else { continue };
            let focused = selection.contains(&item.id) || self.inner.hover.get() == Some(item.id);
            if !focused && b.action != Action::None {
                continue;
            }
            let tag = match b.action {
                Action::None => gettext("No action"),
                Action::PlayTitle { title, chapter } => {
                    let name = p.title(title).map(|t| t.name.clone()).unwrap_or_default();
                    if chapter > 0 {
                        format!("▶ {name} · {}", chapter + 1)
                    } else {
                        format!("▶ {name}")
                    }
                }
                Action::ShowMenu(m) => format!("☰ {}", p.menu(m).map(|m| m.name.clone()).unwrap_or_default()),
                Action::PlayAll => gettext("▶ Play all"),
                Action::SetLanguage { preset, .. } => format!("🌐 {}", p.language(preset).map(|l| l.name.clone()).unwrap_or_default()),
                Action::PlayChapter(c) => format!("⏭ {} {}", gettext("Chapter"), c + 1),
            };
            let is_default = menu.default_button == Some(item.id)
                || (menu.default_button.is_none() && menu.buttons().next().map(|i| i.id) == Some(item.id));
            let tag = if is_default { format!("★ {tag}") } else { tag };
            self.draw_tag(cr, &v, item.rect.x, item.rect.y, &tag, matches!(b.action, Action::None));
        }

        // Where the arrow keys lead from the selected button (manual
        // choices in orange).
        if let [only] = selection.as_slice() {
            let buttons: Vec<&MenuItem> = menu.buttons().collect();
            if let Some(bi) = buttons.iter().position(|i| i.id == *only) {
                let nav = menu.navigation();
                let from = buttons[bi];
                let manual = from.button().map(|b| [b.nav.up, b.nav.down, b.nav.left, b.nav.right]).unwrap_or_default();
                for (dir, target) in nav[bi].iter().enumerate() {
                    let Some(to) = buttons.iter().find(|i| i.id == *target && i.id != from.id) else { continue };
                    let (fr, tr) = (from.rect, to.rect);
                    // Leave from the edge facing the direction, arrive at the target's middle.
                    let start = match dir {
                        0 => (fr.x + fr.w / 2.0, fr.y),
                        1 => (fr.x + fr.w / 2.0, fr.y + fr.h),
                        2 => (fr.x, fr.y + fr.h / 2.0),
                        _ => (fr.x + fr.w, fr.y + fr.h / 2.0),
                    };
                    let end = tr.center();
                    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                    let len = (dx * dx + dy * dy).sqrt().max(1.0);
                    // Stop short of the target's middle so its label stays readable.
                    let stop = (len * 0.25).min(tr.w.min(tr.h) * 0.5);
                    let end = (end.0 - dx / len * stop, end.1 - dy / len * stop);
                    if manual[dir].is_some() {
                        cr.set_source_rgba(1.0, 0.6, 0.1, 0.9);
                    } else {
                        cr.set_source_rgba(0.3, 0.8, 1.0, 0.85);
                    }
                    cr.set_line_width(3.0 / v.scale);
                    cr.move_to(start.0, start.1);
                    cr.line_to(end.0, end.1);
                    cr.stroke().ok();
                    let head = 14.0 / v.scale;
                    let (ux, uy) = (dx / len, dy / len);
                    cr.move_to(end.0, end.1);
                    cr.line_to(end.0 - ux * head - uy * head * 0.6, end.1 - uy * head + ux * head * 0.6);
                    cr.line_to(end.0 - ux * head + uy * head * 0.6, end.1 - uy * head - ux * head * 0.6);
                    cr.close_path();
                    cr.fill().ok();
                }
            }
        }

        for g in self.inner.guides.borrow().iter() {
            cr.set_source_rgba(0.95, 0.3, 0.5, 0.9);
            cr.set_line_width(1.0 / v.scale);
            match *g {
                Guide::V(x) => {
                    cr.move_to(x, 0.0);
                    cr.line_to(x, DESIGN_HEIGHT);
                }
                Guide::H(y) => {
                    cr.move_to(0.0, y);
                    cr.line_to(DESIGN_WIDTH, y);
                }
            }
            cr.stroke().ok();
        }

        // Where a dropped asset will land
        if let Some((x, y)) = self.inner.drop_hover.get() {
            cr.set_source_rgba(0.21, 0.52, 0.89, 0.12);
            cr.paint().ok();
            let (dx, dy) = v.to_design(x, y);
            let (bw, bh) = Self::drop_size(None);
            let r = Rect::new(dx - bw / 2.0, dy - bh / 2.0, bw, bh).clamped();
            render::rounded_rect(cr, r, 12.0);
            accent(cr, 0.25);
            cr.fill_preserve().ok();
            accent(cr, 0.9);
            cr.set_line_width(3.0 / v.scale);
            cr.set_dash(&[10.0 / v.scale, 6.0 / v.scale], 0.0);
            cr.stroke().ok();
            cr.set_dash(&[], 0.0);
        }
        cr.restore().ok(); // clip

        if let Some(hover) = self.inner.hover.get().filter(|h| !selection.contains(h)) {
            if let Some(i) = menu.item(hover) {
                accent(cr, 0.6);
                cr.set_line_width(1.5 / v.scale);
                cr.rectangle(i.rect.x, i.rect.y, i.rect.w, i.rect.h);
                cr.stroke().ok();
            }
        }
        for (n, id) in selection.iter().enumerate() {
            let Some(i) = menu.item(*id) else { continue };
            accent(cr, 1.0);
            cr.set_line_width(if n == 0 { 2.0 } else { 1.5 } / v.scale);
            if n > 0 {
                cr.set_dash(&[5.0 / v.scale, 3.0 / v.scale], 0.0);
            }
            cr.rectangle(i.rect.x, i.rect.y, i.rect.w, i.rect.h);
            cr.stroke().ok();
            cr.set_dash(&[], 0.0);
        }
        if selection.len() == 1 {
            if let Some(i) = menu.item(selection[0]) {
                let hs = HANDLE * 0.8 / v.scale;
                for (hx, hy) in handle_points(i.rect) {
                    cr.rectangle(hx - hs / 2.0, hy - hs / 2.0, hs, hs);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    cr.fill_preserve().ok();
                    accent(cr, 1.0);
                    cr.set_line_width(1.5 / v.scale);
                    cr.stroke().ok();
                }
            }
        }
        if let Some(Drag::Link { x0, y0, x1, y1, .. }) = &*self.inner.drag.borrow() {
            cr.set_source_rgba(1.0, 0.6, 0.1, 0.95);
            cr.set_line_width(4.0 / v.scale);
            cr.set_dash(&[10.0 / v.scale, 6.0 / v.scale], 0.0);
            cr.move_to(*x0, *y0);
            cr.line_to(*x1, *y1);
            cr.stroke().ok();
            cr.set_dash(&[], 0.0);
        }
        if let Some(Drag::Marquee { x0, y0, x1, y1, .. }) = &*self.inner.drag.borrow() {
            cr.rectangle(x0.min(*x1), y0.min(*y1), (x1 - x0).abs(), (y1 - y0).abs());
            accent(cr, 0.15);
            cr.fill_preserve().ok();
            accent(cr, 0.8);
            cr.set_line_width(1.0 / v.scale);
            cr.stroke().ok();
        }
        cr.restore().ok();

        // Hints in screen space
        let hint = if self.inner.drop_hover.get().is_some() {
            Some(gettext("Drop to add to the menu"))
        } else if menu.items.is_empty() {
            Some(gettext("Drag videos here to create buttons"))
        } else {
            None
        };
        if let Some(text) = hint {
            self.draw_hint(cr, w, h, &v, &text);
        }

        // Zoom level while zoomed in.
        let zoom = self.inner.zoom.get();
        if zoom > 1.0 {
            let layout = pangocairo::functions::create_layout(cr);
            layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Cantarell Bold 10")));
            layout.set_text(&gettext("{} % · Ctrl+0 to fit").replace("{}", &format!("{:.0}", zoom * 100.0)));
            let (_, ext) = layout.pixel_extents();
            let (tw, th) = (ext.width() as f64 + 20.0, ext.height() as f64 + 10.0);
            render::rounded_rect(cr, Rect::new(12.0, h - th - 12.0, tw, th), th / 2.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.65);
            cr.fill().ok();
            cr.move_to(22.0, h - th - 7.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
            pangocairo::functions::show_layout(cr, &layout);
        }

        // Keyboard focus ring around the page
        if self.inner.area.has_focus() && self.inner.area.is_focus_visible_for_ring() {
            accent(cr, 0.5);
            cr.set_line_width(2.0);
            cr.rectangle(v.ox - 3.0, v.oy - 3.0, DESIGN_WIDTH * v.scale + 6.0, DESIGN_HEIGHT * v.scale + 6.0);
            cr.stroke().ok();
        }
    }

    /// A pill-shaped message near the bottom of the page, in screen space.
    fn draw_hint(&self, cr: &cairo::Context, w: f64, h: f64, v: &View, text: &str) {
        let layout = pangocairo::functions::create_layout(cr);
        layout.set_text(text);
        let mut fd = gtk::pango::FontDescription::from_string("Cantarell 13");
        fd.set_weight(gtk::pango::Weight::Bold);
        layout.set_font_description(Some(&fd));
        let (_, ext) = layout.pixel_extents();
        let (tw, th) = (ext.width() as f64 + 32.0, ext.height() as f64 + 16.0);
        // Below the page when there is room, else over its bottom edge.
        let page_bottom = v.oy + DESIGN_HEIGHT * v.scale;
        let by = if page_bottom + th + 10.0 <= h { page_bottom + (h - page_bottom - th) / 2.0 } else { page_bottom - th - 16.0 };
        let (bx, by) = ((w - tw) / 2.0, by.max(8.0));
        render::rounded_rect(cr, Rect::new(bx, by, tw, th), th / 2.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.65);
        cr.fill().ok();
        cr.move_to(bx + 16.0, by + 8.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
        pangocairo::functions::show_layout(cr, &layout);
    }

    /// Small label in screen pixels at a design-space position.
    fn draw_tag(&self, cr: &cairo::Context, v: &View, x: f64, y: f64, text: &str, warn: bool) {
        cr.save().ok();
        cr.identity_matrix();
        let sx = v.ox + x * v.scale;
        let sy = (v.oy + y * v.scale).max(v.oy + 20.0);
        let layout = pangocairo::functions::create_layout(cr);
        layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Cantarell Bold 8")));
        layout.set_text(text);
        let (_, ext) = layout.pixel_extents();
        let (tw, th) = (ext.width() as f64 + 10.0, ext.height() as f64 + 4.0);
        render::rounded_rect(cr, Rect::new(sx, sy - th - 3.0, tw, th), th / 2.0);
        if warn {
            cr.set_source_rgba(0.80, 0.40, 0.0, 0.92);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.72);
        }
        cr.fill().ok();
        cr.move_to(sx + 5.0, sy - th - 1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        pangocairo::functions::show_layout(cr, &layout);
        cr.restore().ok();
    }
}

trait FocusRing {
    fn is_focus_visible_for_ring(&self) -> bool;
}

impl FocusRing for gtk::DrawingArea {
    fn is_focus_visible_for_ring(&self) -> bool {
        self.root().and_downcast::<gtk::Window>().is_some_and(|w| w.property::<bool>("focus-visible"))
    }
}
