// SPDX-License-Identifier: GPL-3.0-or-later

//! Menu rendering with cairo/pango, shared by the editor canvas and the
//! disc builder so that what you see is what ends up on the disc.

use crate::bluray::ig::Bitmap;
use crate::media::thumbnail;
use crate::model::*;
use gtk::prelude::*;
use gtk::{cairo, gdk, pango};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Selected,
    Activated,
}

/// Loads image assets and video thumbnails. In async mode (editor), missing
/// thumbnails are extracted in the background and `on_ready` is invoked.
#[derive(Default)]
pub struct ImageCache {
    images: RefCell<HashMap<(PathBuf, i64), Option<cairo::ImageSurface>>>,
    pending: RefCell<HashSet<(PathBuf, i64)>>,
    on_ready: RefCell<Vec<Rc<dyn Fn()>>>,
    asynchronous: bool,
    this: RefCell<Weak<ImageCache>>,
}

pub const THUMB_WIDTH: u32 = 960;

impl ImageCache {
    pub fn new_async(on_ready: impl Fn() + 'static) -> Rc<Self> {
        let cache = Rc::new(ImageCache {
            on_ready: RefCell::new(vec![Rc::new(on_ready) as Rc<dyn Fn()>]),
            asynchronous: true,
            ..Default::default()
        });
        *cache.this.borrow_mut() = Rc::downgrade(&cache);
        cache
    }

    /// Also call `f` whenever a background thumbnail becomes available.
    pub fn connect_ready(&self, f: impl Fn() + 'static) {
        self.on_ready.borrow_mut().push(Rc::new(f));
    }

    pub fn new_sync() -> Self {
        ImageCache::default()
    }

    /// Image for an asset: the file itself for images, a frame at `time`
    /// seconds for videos.
    pub fn get(&self, project: &Project, asset: Id, time: f64) -> Option<cairo::ImageSurface> {
        let a = project.asset(asset)?;
        let key = match a.kind {
            AssetKind::Image => (a.path.clone(), -1),
            AssetKind::Video => (a.path.clone(), (time * 1000.0) as i64),
            AssetKind::Audio => return None,
        };
        if let Some(p) = self.images.borrow().get(&key) {
            return p.clone();
        }
        match a.kind {
            AssetKind::Image => {
                let p = load_image(&a.path);
                self.images.borrow_mut().insert(key, p.clone());
                p
            }
            _ if !self.asynchronous => {
                let p = thumbnail::frame(&a.path, time, THUMB_WIDTH).ok().and_then(|f| load_image(&f));
                self.images.borrow_mut().insert(key, p.clone());
                p
            }
            _ => {
                if self.pending.borrow_mut().insert(key.clone()) {
                    self.fetch(key);
                }
                None
            }
        }
    }

    fn fetch(&self, key: (PathBuf, i64)) {
        let (path, ms) = key.clone();
        let this = self.this.borrow().clone();
        let on_ready = self.on_ready.borrow().clone();
        gtk::glib::spawn_future_local(async move {
            // Extract and decode off the main thread.
            let texture = gtk::gio::spawn_blocking(move || {
                thumbnail::frame(&path, ms as f64 / 1000.0, THUMB_WIDTH).ok().and_then(|f| gdk::Texture::from_filename(f).ok())
            })
            .await
            .ok()
            .flatten();
            let pix = texture.and_then(|t| texture_to_surface(&t));
            let Some(cache) = this.upgrade() else { return };
            cache.images.borrow_mut().insert(key.clone(), pix);
            cache.pending.borrow_mut().remove(&key);
            for f in on_ready {
                f();
            }
        });
    }
}

fn set_color(cr: &cairo::Context, c: Rgba) {
    cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
}

pub fn rounded_rect(cr: &cairo::Context, r: Rect, radius: f64) {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    cr.new_sub_path();
    cr.arc(x + w - rad, y + rad, rad, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - rad, y + h - rad, rad, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + rad, y + h - rad, rad, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + rad, y + rad, rad, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

/// Copy a texture into a cairo image surface.
pub fn texture_to_surface(texture: &gdk::Texture) -> Option<cairo::ImageSurface> {
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, texture.width(), texture.height()).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        // GDK's default download format is premultiplied BGRA (little
        // endian), which is cairo's ARGB32 layout.
        texture.download(&mut data, stride);
    }
    surface.mark_dirty();
    Some(surface)
}

/// Load an image file for drawing. Uses GTK's image loaders rather than
/// gdk-pixbuf, whose sandboxed loaders are unavailable in some Flatpak
/// environments.
pub fn load_image(path: &std::path::Path) -> Option<cairo::ImageSurface> {
    let texture = gdk::Texture::from_filename(path).ok()?;
    texture_to_surface(&texture)
}

/// Draw `pix` into `r`. `cover` fills the rect (cropping), otherwise the
/// image is fitted inside it.
fn draw_pixbuf(cr: &cairo::Context, pix: &cairo::ImageSurface, r: Rect, cover: bool) {
    let (pw, ph) = (pix.width() as f64, pix.height() as f64);
    let sx = r.w / pw;
    let sy = r.h / ph;
    let s = if cover { sx.max(sy) } else { sx.min(sy) };
    let (dw, dh) = (pw * s, ph * s);
    cr.save().ok();
    cr.rectangle(r.x, r.y, r.w, r.h);
    cr.clip();
    cr.translate(r.x + (r.w - dw) / 2.0, r.y + (r.h - dh) / 2.0);
    cr.scale(s, s);
    cr.set_source_surface(pix, 0.0, 0.0).ok();
    cr.source().set_filter(cairo::Filter::Good);
    cr.paint().ok();
    cr.restore().ok();
}

fn layout(cr: &cairo::Context, text: &str, style: &TextStyle, width: f64) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    // One point == one design pixel.
    pangocairo::functions::context_set_resolution(&layout.context(), 72.0);
    layout.context_changed();
    layout.set_font_description(Some(&pango::FontDescription::from_string(&style.font)));
    layout.set_width((width * pango::SCALE as f64) as i32);
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_alignment(match style.align {
        Align::Left => pango::Alignment::Left,
        Align::Center => pango::Alignment::Center,
        Align::Right => pango::Alignment::Right,
    });
    if style.line_spacing > 0.0 && (style.line_spacing - 1.0).abs() > 1e-3 {
        layout.set_line_spacing(style.line_spacing as f32);
    }
    if style.letter_spacing != 0.0 {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrInt::new_letter_spacing((style.letter_spacing * pango::SCALE as f64) as i32));
        layout.set_attributes(Some(&attrs));
    }
    layout.set_text(text);
    layout
}

/// Height of one line of text in `style`, in design pixels.
pub fn line_height(style: &TextStyle) -> f64 {
    let fd = pango::FontDescription::from_string(&style.font);
    let size = if fd.size() > 0 { fd.size() as f64 / pango::SCALE as f64 } else { 40.0 };
    size * 1.35 * if style.line_spacing > 0.0 { style.line_spacing } else { 1.0 }
}

fn draw_text(cr: &cairo::Context, text: &str, style: &TextStyle, color: Rgba, r: Rect, shadow: bool) {
    let l = layout(cr, text, style, r.w);
    let (_, logical) = l.pixel_extents();
    let y = r.y + (r.h - logical.height() as f64) / 2.0;
    if style.glow > 0.0 {
        // Layered soft strokes, widest and faintest first.
        cr.move_to(r.x, y);
        pangocairo::functions::layout_path(cr, &l);
        let path = cr.copy_path().ok();
        cr.new_path();
        if let Some(path) = path {
            cr.set_line_join(cairo::LineJoin::Round);
            let steps = 6;
            for k in (1..=steps).rev() {
                cr.append_path(&path);
                let c = style.glow_color;
                cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, c.a as f64 * 0.12);
                cr.set_line_width(style.glow * 2.0 * k as f64 / steps as f64);
                cr.stroke().ok();
            }
        }
    }
    if shadow {
        cr.move_to(r.x + 3.0, y + 3.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.6 * color.a as f64);
        pangocairo::functions::show_layout(cr, &l);
    }
    if style.outline > 0.0 {
        cr.move_to(r.x, y);
        pangocairo::functions::layout_path(cr, &l);
        set_color(cr, style.outline_color);
        cr.set_line_width(style.outline * 2.0);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.stroke().ok();
    }
    cr.move_to(r.x, y);
    set_color(cr, color);
    pangocairo::functions::show_layout(cr, &l);
}

/// Split a button rect into (thumbnail area, label area).
fn button_areas(cr: &cairo::Context, item: &MenuItem, b: &ButtonItem) -> (Option<Rect>, Rect) {
    let r = item.rect;
    // Keep labels clear of the highlight frame.
    let pad = (r.w * 0.06).min(24.0);
    // Room for the pointer of the Arrow style, in every state.
    let arrow = if b.highlight == Highlight::Arrow { line_height(&b.text) * 0.6 } else { 0.0 };
    let inset = |a: Rect| Rect::new(a.x + pad + arrow, a.y, (a.w - 2.0 * pad - arrow).max(1.0), a.h);
    if b.thumbnail.is_none() {
        return (None, inset(r));
    }
    if b.label.is_empty() {
        return (Some(r), r);
    }
    // As tall as the label is once wrapped (long names take two or more
    // lines), up to half the button; the thumbnail gets the rest.
    let one = line_height(&b.text);
    let width = inset(r).w;
    let (_, logical) = layout(cr, &b.label, &b.text, width).pixel_extents();
    let lh = (logical.height() as f64 + one * 0.25).max(one).min(r.h / 2.0);
    (Some(Rect::new(r.x, r.y, r.w, r.h - lh)), inset(Rect::new(r.x, r.y + r.h - lh, r.w, lh)))
}

/// `p` with fractions of each side (left, top, right, bottom) cut off.
fn cropped(p: &cairo::ImageSurface, crop: [f64; 4]) -> cairo::ImageSurface {
    if crop.iter().all(|c| *c <= 0.0) {
        return p.clone();
    }
    let (w, h) = (p.width() as f64, p.height() as f64);
    let [l, t, r, b] = crop.map(|c| c.clamp(0.0, 0.45));
    let (cw, ch) = ((w * (1.0 - l - r)).round().max(1.0), (h * (1.0 - t - b)).round().max(1.0));
    let Ok(out) = cairo::ImageSurface::create(cairo::Format::ARgb32, cw as i32, ch as i32) else { return p.clone() };
    if let Ok(cr) = cairo::Context::new(&out) {
        cr.set_source_surface(p, -(w * l).round(), -(h * t).round()).ok();
        cr.paint().ok();
    }
    out
}

/// A soft drop shadow in the shape of the picture (its alpha).
fn draw_image_shadow(cr: &cairo::Context, p: &cairo::ImageSurface, r: Rect, opacity: f64) {
    let f = fitted_rect(p, r);
    let s = f.w / p.width() as f64;
    let offset = (f.w.min(f.h) * 0.03).clamp(4.0, 16.0);
    cr.save().ok();
    // Several offset copies approximate a blur.
    for (dx, dy, a) in [(0.0, 0.0, 0.18), (-2.0, 0.0, 0.1), (2.0, 0.0, 0.1), (0.0, -2.0, 0.1), (0.0, 2.0, 0.1), (3.0, 3.0, 0.08)] {
        cr.save().ok();
        cr.translate(f.x + offset + dx, f.y + offset + dy);
        cr.scale(s, s);
        cr.set_source_rgba(0.0, 0.0, 0.0, a * opacity.clamp(0.0, 1.0) * 2.0);
        cr.mask_surface(p, 0.0, 0.0).ok();
        cr.restore().ok();
    }
    cr.restore().ok();
}

/// Where an image is drawn when fitted into `r`.
fn fitted_rect(p: &cairo::ImageSurface, r: Rect) -> Rect {
    let (pw, ph) = (p.width() as f64, p.height() as f64);
    let s = (r.w / pw).min(r.h / ph);
    Rect::new(r.x + (r.w - pw * s) / 2.0, r.y + (r.h - ph * s) / 2.0, pw * s, ph * s)
}

fn shape_path(cr: &cairo::Context, sh: &ShapeItem, r: Rect) {
    if sh.ellipse {
        cr.save().ok();
        cr.translate(r.x + r.w / 2.0, r.y + r.h / 2.0);
        cr.scale(r.w / 2.0, r.h / 2.0);
        cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
        cr.restore().ok();
    } else {
        rounded_rect(cr, r, sh.radius.min(r.w / 2.0).min(r.h / 2.0));
    }
}

pub fn draw_shape(cr: &cairo::Context, sh: &ShapeItem, r: Rect) {
    shape_path(cr, sh, r);
    match sh.gradient {
        Some(bottom) => {
            let g = if sh.horizontal {
                cairo::LinearGradient::new(r.x, 0.0, r.x + r.w, 0.0)
            } else {
                cairo::LinearGradient::new(0.0, r.y, 0.0, r.y + r.h)
            };
            g.add_color_stop_rgba(0.0, sh.fill.r as f64, sh.fill.g as f64, sh.fill.b as f64, sh.fill.a as f64);
            g.add_color_stop_rgba(1.0, bottom.r as f64, bottom.g as f64, bottom.b as f64, bottom.a as f64);
            cr.set_source(&g).ok();
        }
        None => set_color(cr, sh.fill),
    }
    cr.fill().ok();
    if sh.stroke > 0.0 {
        // Keep the border inside the shape's rectangle.
        let h = sh.stroke / 2.0;
        let inner = Rect::new(r.x + h, r.y + h, (r.w - sh.stroke).max(1.0), (r.h - sh.stroke).max(1.0));
        let sh2 = ShapeItem { radius: (sh.radius - h).max(0.0), ..sh.clone() };
        shape_path(cr, &sh2, inner);
        set_color(cr, sh.stroke_color);
        cr.set_line_width(sh.stroke);
        cr.stroke().ok();
    }
}

/// Everything that is not part of the IG button graphics: background,
/// texts, images and button thumbnails.
pub fn draw_static(cr: &cairo::Context, project: &Project, menu: &Menu, images: &ImageCache, with_background: bool) {
    if with_background {
        let bg = &menu.background;
        match bg.gradient {
            Some(bottom) => {
                let g = cairo::LinearGradient::new(0.0, 0.0, 0.0, DESIGN_HEIGHT);
                g.add_color_stop_rgba(0.0, bg.color.r as f64, bg.color.g as f64, bg.color.b as f64, bg.color.a as f64);
                g.add_color_stop_rgba(1.0, bottom.r as f64, bottom.g as f64, bottom.b as f64, bottom.a as f64);
                cr.set_source(&g).ok();
            }
            None => set_color(cr, bg.color),
        }
        cr.rectangle(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT);
        cr.fill().ok();
        let full = Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT);
        if let Some(v) = menu.background.video {
            if let Some(p) = images.get(project, v, menu.background.video_start) {
                draw_pixbuf(cr, &p, full, true);
            }
        } else if let Some(img) = menu.background.image {
            if let Some(p) = images.get(project, img, 0.0) {
                draw_pixbuf(cr, &p, full, true);
            }
        }
    }
    for item in menu.items.iter().filter(|i| !i.hidden) {
        match &item.kind {
            ItemKind::Text(t) => draw_text(cr, &t.text, &t.style, t.style.color, item.rect, t.style.shadow),
            ItemKind::Image(i) => {
                if let Some(p) = images.get(project, i.asset, i.time).map(|p| cropped(&p, i.crop)) {
                    if i.shadow {
                        draw_image_shadow(cr, &p, item.rect, i.opacity);
                    }
                    cr.push_group();
                    if i.radius > 0.0 {
                        let fitted = fitted_rect(&p, item.rect);
                        rounded_rect(cr, fitted, i.radius.min(fitted.w / 2.0).min(fitted.h / 2.0));
                        cr.clip();
                    }
                    draw_pixbuf(cr, &p, item.rect, false);
                    cr.pop_group_to_source().ok();
                    cr.paint_with_alpha(i.opacity.clamp(0.0, 1.0)).ok();
                }
            }
            ItemKind::Shape(sh) => draw_shape(cr, sh, item.rect),
            ItemKind::Button(b) => {
                if let (Some(area), Some(asset)) = (button_areas(cr, item, b).0, b.thumbnail) {
                    match images.get(project, asset, b.thumbnail_time) {
                        Some(p) => {
                            // Rounded corners on the fitted image.
                            let (pw, ph) = (p.width() as f64, p.height() as f64);
                            let s = (area.w / pw).min(area.h / ph);
                            let fitted = Rect::new(area.x + (area.w - pw * s) / 2.0, area.y + (area.h - ph * s) / 2.0, pw * s, ph * s);
                            cr.save().ok();
                            rounded_rect(cr, fitted, 10.0);
                            cr.clip();
                            draw_pixbuf(cr, &p, area, false);
                            cr.restore().ok();
                        }
                        None => {
                            cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
                            cr.rectangle(area.x, area.y, area.w, area.h);
                            cr.fill().ok();
                        }
                    }
                }
            }
        }
    }
}

/// The IG graphics of one button in the given state.
pub fn draw_button(cr: &cairo::Context, project: &Project, images: &ImageCache, item: &MenuItem, b: &ButtonItem, state: ButtonState) {
    if item.hidden {
        return;
    }
    let (thumb, label_area) = button_areas(cr, item, b);
    let accent = match state {
        ButtonState::Normal => None,
        ButtonState::Selected => Some(b.selected_color),
        ButtonState::Activated => Some(b.activated_color),
    };
    // Artwork for this state (falling back to the normal picture).
    let art = match state {
        ButtonState::Normal => b.images.normal,
        ButtonState::Selected => b.images.selected.or(b.images.normal),
        ButtonState::Activated => b.images.activated.or(b.images.selected).or(b.images.normal),
    };
    let has_art_state = match state {
        ButtonState::Normal => b.images.normal.is_some(),
        ButtonState::Selected => b.images.selected.is_some(),
        ButtonState::Activated => b.images.activated.is_some() || b.images.selected.is_some(),
    };
    if let Some(p) = art.and_then(|a| images.get(project, a, 0.0)) {
        draw_pixbuf(cr, &p, item.rect, false);
    }
    if let Some(fill) = b.fill {
        rounded_rect(cr, item.rect, 12.0);
        set_color(cr, fill);
        cr.fill().ok();
    }
    // Pictures made for the state are the highlight; no drawn one on top.
    let accent_drawn = accent.filter(|_| !has_art_state);
    if let (Some(c), Highlight::Fill) = (accent_drawn, b.highlight) {
        rounded_rect(cr, item.rect, 12.0);
        set_color(cr, c);
        cr.fill().ok();
    }
    let text_color = match (b.highlight, accent) {
        (_, None) => b.text.color,
        (_, Some(_)) if b.highlight_text.is_some() => b.highlight_text.unwrap_or(b.text.color),
        (Highlight::Frame | Highlight::Text | Highlight::Arrow, Some(c)) if !has_art_state => c,
        _ => b.text.color,
    };
    if !b.label.is_empty() {
        draw_text(cr, &b.label, &b.text, text_color, label_area, b.text.shadow);
    }
    if let Some(c) = accent_drawn {
        set_color(cr, c);
        match b.highlight {
            Highlight::Frame => {
                let inset = 3.0;
                // Around the thumbnail picture itself (it may not fill its area).
                let r = match (thumb, b.thumbnail.and_then(|a| images.get(project, a, b.thumbnail_time))) {
                    (Some(area), Some(pic)) => {
                        let f = fitted_rect(&pic, area);
                        Rect::new(f.x - inset * 2.0, f.y - inset * 2.0, f.w + inset * 4.0, f.h + inset * 4.0)
                    }
                    (Some(area), None) => area,
                    (None, _) => item.rect,
                };
                rounded_rect(cr, Rect::new(r.x + inset, r.y + inset, r.w - 2.0 * inset, r.h - 2.0 * inset), 10.0);
                cr.set_line_width(6.0);
                cr.stroke().ok();
            }
            Highlight::Underline => {
                let r = label_area;
                cr.rectangle(r.x + r.w * 0.1, r.y + r.h - 8.0, r.w * 0.8, 6.0);
                cr.fill().ok();
            }
            Highlight::Arrow => {
                // A triangle before the label, level with it.
                let lh = line_height(&b.text);
                let size = (lh * 0.4).min(item.rect.h * 0.5);
                let cy = label_area.y + label_area.h / 2.0;
                let x = label_area.x - lh * 0.55;
                cr.move_to(x, cy - size / 2.0);
                cr.line_to(x + size * 0.85, cy);
                cr.line_to(x, cy + size / 2.0);
                cr.close_path();
                cr.fill().ok();
            }
            Highlight::Text | Highlight::Fill => {}
        }
    }
}

/// Scale factors from design space to a disc picture (they differ for the
/// wide pixels of SD formats).
pub fn disc_scale(width: u32, height: u32) -> (f64, f64) {
    (width as f64 / DESIGN_WIDTH, height as f64 / DESIGN_HEIGHT)
}

/// Button position and bitmap size at disc resolution.
pub fn button_geometry(item: &MenuItem, disc_w: u32, disc_h: u32) -> (u16, u16, u16, u16) {
    let (sx, sy) = disc_scale(disc_w, disc_h);
    let x = (item.rect.x * sx).floor().clamp(0.0, disc_w as f64 - 8.0);
    let y = (item.rect.y * sy).floor().clamp(0.0, disc_h as f64 - 8.0);
    let w = (item.rect.w * sx).ceil().clamp(8.0, disc_w as f64 - x);
    let h = (item.rect.h * sy).ceil().clamp(8.0, disc_h as f64 - y);
    (x as u16, y as u16, w as u16, h as u16)
}

fn surface_to_bitmap(surface: &mut cairo::ImageSurface) -> Bitmap {
    surface.flush();
    let (w, h) = (surface.width() as usize, surface.height() as usize);
    let stride = surface.stride() as usize;
    let data = surface.data().expect("surface data");
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let p = &data[y * stride + x * 4..y * stride + x * 4 + 4];
            // ARGB32 native endian (little endian: B G R A), premultiplied
            let px = u32::from_ne_bytes([p[0], p[1], p[2], p[3]]);
            let a = (px >> 24) as u8;
            let un = |c: u32| -> u8 {
                if a == 0 {
                    0
                } else {
                    ((c * 255 + a as u32 / 2) / a as u32).min(255) as u8
                }
            };
            rgba.extend_from_slice(&[un((px >> 16) & 0xFF), un((px >> 8) & 0xFF), un(px & 0xFF), a]);
        }
    }
    Bitmap { width: w as u16, height: h as u16, rgba }
}

/// Render one button state as an IG bitmap at disc resolution.
pub fn render_button_bitmap(project: &Project, images: &ImageCache, item: &MenuItem, b: &ButtonItem, state: ButtonState, disc_w: u32, disc_h: u32) -> Bitmap {
    let (x, y, w, h) = button_geometry(item, disc_w, disc_h);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w as i32, h as i32).expect("surface");
    {
        let cr = cairo::Context::new(&surface).expect("cairo context");
        let (sx, sy) = disc_scale(disc_w, disc_h);
        cr.translate(-(x as f64), -(y as f64));
        cr.scale(sx, sy);
        draw_button(&cr, project, images, item, b, state);
    }
    surface_to_bitmap(&mut surface)
}

/// Stand-in for the movie behind a pop-up menu while editing: the first
/// title's thumbnail, darkened, or a neutral gradient.
pub fn popup_backdrop(cr: &cairo::Context, project: &Project, images: &ImageCache) {
    let full = Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT);
    let g = cairo::LinearGradient::new(0.0, 0.0, 0.0, DESIGN_HEIGHT);
    g.add_color_stop_rgb(0.0, 0.25, 0.27, 0.3);
    g.add_color_stop_rgb(1.0, 0.1, 0.1, 0.12);
    cr.set_source(&g).ok();
    cr.rectangle(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT);
    cr.fill().ok();
    if let Some(t) = project.titles.first() {
        if let Some(p) = images.get(project, t.asset, project.title_poster(t.id)) {
            draw_pixbuf(cr, &p, full, true);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            cr.rectangle(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT);
            cr.fill().ok();
        }
    }
}

/// Render everything but the buttons (pop-up menus have no background) as
/// one IG bitmap, cropped to what is drawn: (x, y, bitmap).
pub fn render_static_bitmap(project: &Project, menu: &Menu, images: &ImageCache, disc_w: u32, disc_h: u32) -> Option<(u16, u16, Bitmap)> {
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, disc_w as i32, disc_h as i32).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        let (sx, sy) = disc_scale(disc_w, disc_h);
        cr.scale(sx, sy);
        draw_static(&cr, project, menu, images, false);
    }
    let full = surface_to_bitmap(&mut surface);
    let (w, h) = (full.width as usize, full.height as usize);
    let alpha = |x: usize, y: usize| full.rgba[(y * w + x) * 4 + 3] > 0;
    let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| alpha(x, y))).collect();
    let (&y0, &y1) = (rows.first()?, rows.last()?);
    let x0 = (0..w).find(|&x| (y0..=y1).any(|y| alpha(x, y)))?;
    let x1 = (0..w).rev().find(|&x| (y0..=y1).any(|y| alpha(x, y)))?;
    // IG objects are at least 8×8.
    let (x1, y1) = ((x1 + 1).max(x0 + 8).min(w), (y1 + 1).max(y0 + 8).min(h));
    let (x0, y0) = (x1.saturating_sub(8).min(x0), y1.saturating_sub(8).min(y0));
    let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        rgba.extend_from_slice(&full.rgba[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    Some((x0 as u16, y0 as u16, Bitmap { width: (x1 - x0) as u16, height: (y1 - y0) as u16, rgba }))
}

/// Render the static layer as a PNG at disc resolution.
pub fn render_static_png(
    project: &Project,
    menu: &Menu,
    images: &ImageCache,
    disc_w: u32,
    disc_h: u32,
    with_background: bool,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, disc_w as i32, disc_h as i32)?;
    {
        let cr = cairo::Context::new(&surface)?;
        let (sx, sy) = disc_scale(disc_w, disc_h);
        cr.scale(sx, sy);
        draw_static(&cr, project, menu, images, with_background);
    }
    let mut f = std::fs::File::create(out)?;
    surface.write_to_png(&mut f)?;
    Ok(())
}

/// A small preview of a menu (as the player shows it, without editor
/// decorations) for lists and thumbnails.
/// A menu as a player shows it, with the default (or first) button
/// selected, written to a PNG `width` pixels wide.
pub fn menu_png(project: &Project, menu: &Menu, images: &ImageCache, width: i32, out: &std::path::Path) -> anyhow::Result<()> {
    let height = (width as f64 * DESIGN_HEIGHT / DESIGN_WIDTH).round() as i32;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    {
        let cr = cairo::Context::new(&surface)?;
        let s = width as f64 / DESIGN_WIDTH;
        cr.scale(s, s);
        if menu.popup {
            popup_backdrop(&cr, project, images);
        }
        draw_static(&cr, project, menu, images, true);
        let selected = menu.default_button.or_else(|| menu.buttons().next().map(|b| b.id));
        for item in &menu.items {
            if let Some(b) = item.button() {
                let state = if Some(item.id) == selected { ButtonState::Selected } else { ButtonState::Normal };
                draw_button(&cr, project, images, item, b, state);
            }
        }
    }
    let mut f = std::fs::File::create(out)?;
    surface.write_to_png(&mut f)?;
    Ok(())
}

pub fn menu_thumbnail(project: &Project, menu: &Menu, images: &ImageCache, width: i32) -> Option<gtk::gdk::Texture> {
    let height = (width as f64 * DESIGN_HEIGHT / DESIGN_WIDTH).round() as i32;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        let s = width as f64 / DESIGN_WIDTH;
        cr.scale(s, s);
        if menu.popup {
            popup_backdrop(&cr, project, images);
        }
        draw_static(&cr, project, menu, images, true);
        for item in &menu.items {
            if let Some(b) = item.button() {
                draw_button(&cr, project, images, item, b, ButtonState::Normal);
            }
        }
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?.to_vec();
    let bytes = gtk::glib::Bytes::from_owned(data);
    Some(
        gtk::gdk::MemoryTexture::new(width, height, gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, stride)
            .upcast(),
    )
}
