// SPDX-License-Identifier: GPL-3.0-or-later

//! Menu model. All coordinates are in a 1920×1080 design space and scaled
//! to the disc resolution when rendering.

use super::{new_id, Id};
use serde::{Deserialize, Serialize};

pub const DESIGN_WIDTH: f64 = 1920.0;
pub const DESIGN_HEIGHT: f64 = 1080.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Rgba { r, g, b, a }
    }
    pub const WHITE: Rgba = Rgba::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Rgba = Rgba::new(0.0, 0.0, 0.0, 1.0);
    pub const TRANSPARENT: Rgba = Rgba::new(0.0, 0.0, 0.0, 0.0);
    /// GNOME accent blue.
    pub const ACCENT: Rgba = Rgba::new(0.21, 0.52, 0.89, 1.0);
    pub const HIGHLIGHT: Rgba = Rgba::new(1.0, 0.78, 0.2, 1.0);
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    /// The largest rectangle with `aspect` (w/h) inside `self`, placed
    /// horizontally according to `align` and centred vertically.
    pub fn fit(&self, aspect: f64, align: Align) -> Rect {
        if aspect <= 0.0 || !aspect.is_finite() {
            return *self;
        }
        let (w, h) = if self.w / self.h > aspect { (self.h * aspect, self.h) } else { (self.w, self.w / aspect) };
        let x = match align {
            Align::Left => self.x,
            Align::Center => self.x + (self.w - w) / 2.0,
            Align::Right => self.x + self.w - w,
        };
        Rect::new(x.round(), (self.y + (self.h - h) / 2.0).round(), w.round(), h.round())
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Keep the rectangle inside the design area with a minimum size.
    pub fn clamped(mut self) -> Rect {
        self.w = self.w.clamp(8.0, DESIGN_WIDTH);
        self.h = self.h.clamp(8.0, DESIGN_HEIGHT);
        self.x = self.x.clamp(0.0, DESIGN_WIDTH - self.w);
        self.y = self.y.clamp(0.0, DESIGN_HEIGHT - self.h);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

fn one() -> f64 {
    1.0
}

fn black() -> Rgba {
    Rgba::new(0.0, 0.0, 0.0, 1.0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextStyle {
    /// Pango font description, e.g. "Cantarell Bold 40".
    pub font: String,
    pub color: Rgba,
    pub align: Align,
    pub shadow: bool,
    /// Outline width in design pixels (0 = none).
    #[serde(default)]
    pub outline: f64,
    #[serde(default = "black")]
    pub outline_color: Rgba,
    /// Extra space between letters, in design pixels.
    #[serde(default)]
    pub letter_spacing: f64,
    /// Line height as a multiple of the normal one.
    #[serde(default = "one")]
    pub line_spacing: f64,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font: "Cantarell Bold 40".into(),
            color: Rgba::WHITE,
            align: Align::Center,
            shadow: true,
            outline: 0.0,
            outline_color: black(),
            letter_spacing: 0.0,
            line_spacing: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Highlight {
    /// Recolor the label and draw a frame around the button.
    #[default]
    Frame,
    /// Recolor the label only.
    Text,
    /// Draw a bar under the label.
    Underline,
    /// Fill the button with the highlight color.
    Fill,
    /// Draw a pointer before the label.
    Arrow,
}

/// Pictures used as a button's graphics, per state (e.g. artwork exported
/// from an image editor). A missing state uses the normal picture.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct StateImages {
    pub normal: Option<Id>,
    pub selected: Option<Id>,
    pub activated: Option<Id>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Action {
    None,
    /// Play a title from the given chapter (0 = start).
    PlayTitle { title: Id, chapter: u32 },
    ShowMenu(Id),
    /// Play every title in order, then return to the menu.
    PlayAll,
    /// Choose a language preset, then show `menu` (or stay).
    SetLanguage { preset: Id, menu: Option<Id> },
    /// Pop-up menus: go to a chapter (0-based) of the playing title.
    PlayChapter(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NavOverride {
    pub up: Option<Id>,
    pub down: Option<Id>,
    pub left: Option<Id>,
    pub right: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ButtonItem {
    pub label: String,
    pub text: TextStyle,
    pub selected_color: Rgba,
    pub activated_color: Rgba,
    pub highlight: Highlight,
    /// Optional fill behind the label (part of the button graphics).
    pub fill: Option<Rgba>,
    pub action: Action,
    pub nav: NavOverride,
    /// Video asset whose frame is shown as a thumbnail in the button.
    pub thumbnail: Option<Id>,
    /// Thumbnail frame position in seconds.
    pub thumbnail_time: f64,
    /// Label color when selected or activated (defaults to the highlight
    /// color, or the normal color for Fill).
    #[serde(default)]
    pub highlight_text: Option<Rgba>,
    #[serde(default)]
    pub images: StateImages,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextItem {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageItem {
    pub asset: Id,
    /// Frame position when the asset is a video.
    #[serde(default)]
    pub time: f64,
    #[serde(default = "one")]
    pub opacity: f64,
    /// Corner radius in design pixels.
    #[serde(default)]
    pub radius: f64,
}

/// A decorative filled (rounded) rectangle or ellipse, e.g. a panel
/// behind buttons, or a thin one as a line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShapeItem {
    pub fill: Rgba,
    pub radius: f64,
    /// Bottom color of a vertical gradient from `fill`.
    #[serde(default)]
    pub gradient: Option<Rgba>,
    /// Border width (0 = none).
    #[serde(default)]
    pub stroke: f64,
    #[serde(default = "black")]
    pub stroke_color: Rgba,
    #[serde(default)]
    pub ellipse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
// Menus hold at most a few hundred items; boxing buttons isn't worth it.
#[allow(clippy::large_enum_variant)]
pub enum ItemKind {
    Button(ButtonItem),
    Text(TextItem),
    Image(ImageItem),
    Shape(ShapeItem),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MenuItem {
    pub id: Id,
    pub rect: Rect,
    pub kind: ItemKind,
    /// Left out of the menu (kept for later).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Can't be selected or moved on the canvas.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

impl ImageItem {
    pub fn new(asset: Id) -> Self {
        ImageItem { asset, time: 0.0, opacity: 1.0, radius: 0.0 }
    }
}

impl ShapeItem {
    pub fn new(fill: Rgba, radius: f64) -> Self {
        ShapeItem { fill, radius, gradient: None, stroke: 0.0, stroke_color: black(), ellipse: false }
    }
}

impl MenuItem {
    pub fn new(kind: ItemKind, rect: Rect) -> Self {
        MenuItem { id: new_id(), rect, kind, hidden: false, locked: false }
    }

    pub fn new_button(label: &str, action: Action, rect: Rect) -> Self {
        MenuItem::new(
            ItemKind::Button(ButtonItem {
                label: label.into(),
                text: TextStyle::default(),
                selected_color: Rgba::HIGHLIGHT,
                activated_color: Rgba::ACCENT,
                highlight: Highlight::default(),
                fill: None,
                action,
                nav: NavOverride::default(),
                thumbnail: None,
                thumbnail_time: 10.0,
                highlight_text: None,
                images: StateImages::default(),
            }),
            rect,
        )
    }

    pub fn new_text(text: &str, rect: Rect) -> Self {
        MenuItem::new(
            ItemKind::Text(TextItem { text: text.into(), style: TextStyle { font: "Cantarell Bold 64".into(), ..Default::default() } }),
            rect,
        )
    }

    pub fn new_image(asset: Id, rect: Rect) -> Self {
        MenuItem::new(ItemKind::Image(ImageItem::new(asset)), rect)
    }

    pub fn new_shape(fill: Rgba, radius: f64, rect: Rect) -> Self {
        MenuItem::new(ItemKind::Shape(ShapeItem::new(fill, radius)), rect)
    }

    pub fn button(&self) -> Option<&ButtonItem> {
        match &self.kind {
            ItemKind::Button(b) => Some(b),
            _ => None,
        }
    }

    pub fn button_mut(&mut self) -> Option<&mut ButtonItem> {
        match &mut self.kind {
            ItemKind::Button(b) => Some(b),
            _ => None,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            ItemKind::Button(_) => "Button",
            ItemKind::Text(_) => "Text",
            ItemKind::Image(_) => "Image",
            ItemKind::Shape(_) => "Shape",
        }
    }
}

/// Layout operations on a set of items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignOp {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
    DistributeH,
    DistributeV,
}

/// The title-safe area items are aligned to when only one is selected.
pub const SAFE_AREA: Rect = Rect::new(DESIGN_WIDTH * 0.05, DESIGN_HEIGHT * 0.05, DESIGN_WIDTH * 0.9, DESIGN_HEIGHT * 0.9);

/// Bounding box of rectangles.
pub fn bounds(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().fold(None, |acc: Option<Rect>, r| {
        Some(match acc {
            None => r,
            Some(a) => {
                let x0 = a.x.min(r.x);
                let y0 = a.y.min(r.y);
                let x1 = (a.x + a.w).max(r.x + r.w);
                let y1 = (a.y + a.h).max(r.y + r.h);
                Rect::new(x0, y0, x1 - x0, y1 - y0)
            }
        })
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Background {
    pub color: Rgba,
    /// Image asset scaled to fill the screen.
    pub image: Option<Id>,
    /// Video asset used as a motion background.
    pub video: Option<Id>,
    /// Bottom color of a vertical gradient starting at `color`.
    #[serde(default)]
    pub gradient: Option<Rgba>,
    /// Where in the motion video the loop starts, in seconds.
    #[serde(default)]
    pub video_start: f64,
}

impl Default for Background {
    fn default() -> Self {
        Background { color: Rgba::new(0.08, 0.09, 0.12, 1.0), image: None, video: None, gradient: None, video_start: 0.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Menu {
    pub id: Id,
    pub name: String,
    pub background: Background,
    /// Audio asset looped with the menu.
    pub audio: Option<Id>,
    /// Loop length for motion backgrounds / audio, in seconds.
    pub duration: f64,
    pub items: Vec<MenuItem>,
    pub default_button: Option<Id>,
    /// Shown over a title with the remote's Pop-up key instead of being a
    /// menu screen of its own.
    #[serde(default)]
    pub popup: bool,
    /// Video played before the menu appears.
    #[serde(default)]
    pub intro: Option<Id>,
    /// Play the intro every time the menu is shown, not just the first.
    #[serde(default)]
    pub intro_every_time: bool,
    /// Do something by itself after a while on the menu.
    #[serde(default)]
    pub timeout: Option<MenuTimeout>,
    /// Seconds the buttons take to fade in when the menu appears / out
    /// when it's left for another pop-up page (0 = no fade).
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MenuTimeout {
    pub seconds: u32,
    pub action: Action,
}

impl Menu {
    pub fn new(name: &str) -> Self {
        Menu {
            id: new_id(),
            name: name.into(),
            background: Background::default(),
            audio: None,
            duration: 30.0,
            items: Vec::new(),
            default_button: None,
            popup: false,
            intro: None,
            intro_every_time: false,
            timeout: None,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }

    /// An empty pop-up menu.
    pub fn new_popup(name: &str) -> Self {
        let mut m = Menu::new(name);
        m.popup = true;
        m.background.color = Rgba::new(0.0, 0.0, 0.0, 0.0);
        m
    }

    pub fn item(&self, id: Id) -> Option<&MenuItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: Id) -> Option<&mut MenuItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// Buttons that are part of the menu (not hidden).
    pub fn buttons(&self) -> impl Iterator<Item = &MenuItem> {
        self.items.iter().filter(|i| i.button().is_some() && !i.hidden)
    }

    /// Align or distribute `ids`. A single item is aligned to the safe
    /// area (centers use the whole screen); several items to their bounds.
    pub fn align_items(&mut self, ids: &[Id], op: AlignOp) {
        let rects: Vec<(Id, Rect)> = self.items.iter().filter(|i| ids.contains(&i.id)).map(|i| (i.id, i.rect)).collect();
        if rects.is_empty() {
            return;
        }
        let area = if rects.len() == 1 {
            match op {
                AlignOp::HCenter | AlignOp::VCenter => Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT),
                _ => SAFE_AREA,
            }
        } else {
            bounds(rects.iter().map(|(_, r)| *r)).unwrap()
        };
        let mut new: Vec<(Id, Rect)> = rects.clone();
        match op {
            AlignOp::Left => new.iter_mut().for_each(|(_, r)| r.x = area.x),
            AlignOp::HCenter => new.iter_mut().for_each(|(_, r)| r.x = area.x + (area.w - r.w) / 2.0),
            AlignOp::Right => new.iter_mut().for_each(|(_, r)| r.x = area.x + area.w - r.w),
            AlignOp::Top => new.iter_mut().for_each(|(_, r)| r.y = area.y),
            AlignOp::VCenter => new.iter_mut().for_each(|(_, r)| r.y = area.y + (area.h - r.h) / 2.0),
            AlignOp::Bottom => new.iter_mut().for_each(|(_, r)| r.y = area.y + area.h - r.h),
            AlignOp::DistributeH | AlignOp::DistributeV => {
                if new.len() < 3 {
                    return;
                }
                let horizontal = op == AlignOp::DistributeH;
                let key = |r: &Rect| if horizontal { r.x } else { r.y };
                let size = |r: &Rect| if horizontal { r.w } else { r.h };
                new.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
                let start = key(&new[0].1);
                let end = key(&new[new.len() - 1].1) + size(&new[new.len() - 1].1);
                let total: f64 = new.iter().map(|(_, r)| size(r)).sum();
                let gap = (end - start - total) / (new.len() - 1) as f64;
                let mut pos = start;
                for (_, r) in new.iter_mut() {
                    if horizontal {
                        r.x = pos;
                    } else {
                        r.y = pos;
                    }
                    pos += size(r) + gap;
                }
            }
        }
        for (id, r) in new {
            if let Some(i) = self.item_mut(id) {
                i.rect = Rect::new(r.x.round(), r.y.round(), r.w, r.h).clamped();
            }
        }
    }

    pub fn is_motion(&self) -> bool {
        self.background.video.is_some() || self.audio.is_some()
    }

    /// Drop references to a removed title or menu.
    pub fn forget_target(&mut self, target: Id) {
        if let Some(t) = self.timeout {
            let gone = match t.action {
                Action::PlayTitle { title, .. } => title == target,
                Action::ShowMenu(m) => m == target,
                Action::SetLanguage { preset, .. } => preset == target,
                _ => false,
            };
            if gone {
                self.timeout = None;
            }
        }
        for it in &mut self.items {
            if let Some(b) = it.button_mut() {
                match b.action {
                    Action::PlayTitle { title, .. } if title == target => b.action = Action::None,
                    Action::ShowMenu(m) if m == target => b.action = Action::None,
                    Action::SetLanguage { preset, .. } if preset == target => b.action = Action::None,
                    Action::SetLanguage { preset, menu: Some(m) } if m == target => b.action = Action::SetLanguage { preset, menu: None },
                    _ => {}
                }
            }
        }
    }

    /// Drop references to a removed asset.
    pub fn forget_asset(&mut self, asset: Id) {
        self.items.retain(|i| !matches!(&i.kind, ItemKind::Image(img) if img.asset == asset));
        for it in &mut self.items {
            if let Some(b) = it.button_mut() {
                if b.thumbnail == Some(asset) {
                    b.thumbnail = None;
                }
            }
        }
        if self.background.image == Some(asset) {
            self.background.image = None;
        }
        if self.background.video == Some(asset) {
            self.background.video = None;
        }
        if self.intro == Some(asset) {
            self.intro = None;
        }
        if self.audio == Some(asset) {
            self.audio = None;
        }
    }

    /// Compute button navigation neighbours: explicit overrides win,
    /// otherwise the nearest button in each direction.
    /// Returns (up, down, left, right) per button in `buttons()` order.
    pub fn navigation(&self) -> Vec<[Id; 4]> {
        let buttons: Vec<&MenuItem> = self.buttons().collect();
        buttons
            .iter()
            .map(|b| {
                let (cx, cy) = b.rect.center();
                let pick = |dir: usize| -> Id {
                    let mut best: Option<(f64, Id)> = None;
                    for o in &buttons {
                        if o.id == b.id {
                            continue;
                        }
                        let (ox, oy) = o.rect.center();
                        let (dx, dy) = (ox - cx, oy - cy);
                        // primary axis distance must be positive in the direction
                        let (along, across) = match dir {
                            0 => (-dy, dx),
                            1 => (dy, dx),
                            2 => (-dx, dy),
                            _ => (dx, dy),
                        };
                        if along <= 1.0 {
                            continue;
                        }
                        let score = along + across.abs() * 2.0;
                        if best.is_none_or(|(s, _)| score < s) {
                            best = Some((score, o.id));
                        }
                    }
                    best.map_or(b.id, |(_, id)| id)
                };
                let nav = &b.button().unwrap().nav;
                let valid = |o: Option<Id>| o.filter(|id| buttons.iter().any(|x| x.id == *id));
                [
                    valid(nav.up).unwrap_or_else(|| pick(0)),
                    valid(nav.down).unwrap_or_else(|| pick(1)),
                    valid(nav.left).unwrap_or_else(|| pick(2)),
                    valid(nav.right).unwrap_or_else(|| pick(3)),
                ]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect() {
        let r = Rect::new(100.0, 100.0, 600.0, 300.0);
        assert_eq!(r.fit(4.0, Align::Left), Rect::new(100.0, 175.0, 600.0, 150.0));
        assert_eq!(r.fit(1.0, Align::Center), Rect::new(250.0, 100.0, 300.0, 300.0));
        assert_eq!(r.fit(1.0, Align::Right), Rect::new(400.0, 100.0, 300.0, 300.0));
    }

    #[test]
    fn align_and_distribute() {
        let mut m = Menu::new("t");
        for (x, w) in [(100.0, 100.0), (250.0, 50.0), (700.0, 100.0)] {
            m.items.push(MenuItem::new_text("t", Rect::new(x, x, w, 40.0)));
        }
        let ids: Vec<Id> = m.items.iter().map(|i| i.id).collect();
        m.align_items(&ids, AlignOp::Top);
        assert!(m.items.iter().all(|i| i.rect.y == 100.0));
        m.align_items(&ids, AlignOp::DistributeH);
        // total width 250 over span 100..800 → gaps of 225
        assert_eq!(m.items[1].rect.x, 425.0);
        m.align_items(&ids[..1], AlignOp::HCenter);
        assert_eq!(m.items[0].rect.x, DESIGN_WIDTH / 2.0 - 50.0);
    }

    #[test]
    fn grid_navigation() {
        let mut m = Menu::new("t");
        let r = |x, y| Rect::new(x, y, 100.0, 50.0);
        for (x, y) in [(100.0, 100.0), (400.0, 100.0), (100.0, 300.0), (400.0, 300.0)] {
            m.items.push(MenuItem::new_button("b", Action::None, r(x, y)));
        }
        let ids: Vec<Id> = m.items.iter().map(|i| i.id).collect();
        let nav = m.navigation();
        // top-left: up=self, down=bottom-left, left=self, right=top-right
        assert_eq!(nav[0], [ids[0], ids[2], ids[0], ids[1]]);
        // bottom-right: up=top-right, left=bottom-left
        assert_eq!(nav[3][0], ids[1]);
        assert_eq!(nav[3][2], ids[2]);
    }
}
