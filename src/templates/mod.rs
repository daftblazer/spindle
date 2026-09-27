// SPDX-License-Identifier: GPL-3.0-or-later

//! Menu templates: generate a complete, linked set of menus for the
//! project's titles. A template is a style (layout and typography, grouped
//! by category) in a color palette, with optional season and disc lines.

mod movie;
mod show;

use crate::model::*;
use gettextrs::gettext;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub top: Rgba,
    pub bottom: Rgba,
    /// Translucent panels behind text and buttons.
    pub panel: Rgba,
    pub text: Rgba,
    pub dim: Rgba,
    pub selected: Rgba,
    pub activated: Rgba,
    pub shadow: bool,
}

const fn rgb(hex: u32) -> Rgba {
    Rgba::new(((hex >> 16) & 0xFF) as f32 / 255.0, ((hex >> 8) & 0xFF) as f32 / 255.0, (hex & 0xFF) as f32 / 255.0, 1.0)
}

const fn alpha(c: Rgba, a: f32) -> Rgba {
    Rgba::new(c.r, c.g, c.b, a)
}

impl Theme {
    /// Readable text color on the highlight color.
    fn on_accent(&self) -> Rgba {
        let c = self.selected;
        if 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b > 0.55 {
            rgb(0x14141a)
        } else {
            rgb(0xffffff)
        }
    }

}

pub const THEMES: [Theme; 11] = [
    Theme {
        name: "Midnight",
        top: rgb(0x1b2a5a),
        bottom: rgb(0x070b1c),
        panel: alpha(rgb(0x000000), 0.35),
        text: rgb(0xffffff),
        dim: alpha(rgb(0xc0cbf0), 0.85),
        selected: rgb(0xf6d32d),
        activated: rgb(0x62a0ea),
        shadow: true,
    },
    Theme {
        name: "Ember",
        top: rgb(0x7a2a0c),
        bottom: rgb(0x1e0706),
        panel: alpha(rgb(0x000000), 0.35),
        text: rgb(0xfff5e6),
        dim: alpha(rgb(0xffd9b3), 0.8),
        selected: rgb(0xffbe6f),
        activated: rgb(0xff7800),
        shadow: true,
    },
    Theme {
        name: "Forest",
        top: rgb(0x1f5b3f),
        bottom: rgb(0x06170f),
        panel: alpha(rgb(0x000000), 0.3),
        text: rgb(0xf0fff4),
        dim: alpha(rgb(0xc2f0d2), 0.8),
        selected: rgb(0x8ff0a4),
        activated: rgb(0x33d17a),
        shadow: true,
    },
    Theme {
        name: "Cinema",
        top: rgb(0x1a1a1a),
        bottom: rgb(0x000000),
        panel: alpha(rgb(0x000000), 0.55),
        text: rgb(0xffffff),
        dim: alpha(rgb(0xbbbbbb), 0.85),
        selected: rgb(0xe01b24),
        activated: rgb(0xff7b63),
        shadow: true,
    },
    Theme {
        name: "Paper",
        top: rgb(0xfafafa),
        bottom: rgb(0xd8d6d2),
        panel: alpha(rgb(0xffffff), 0.65),
        text: rgb(0x241f31),
        dim: alpha(rgb(0x5e5c64), 0.9),
        selected: rgb(0x1c71d8),
        activated: rgb(0xc01c28),
        shadow: false,
    },
    Theme {
        name: "Ocean",
        top: rgb(0x0b4f5c),
        bottom: rgb(0x031a20),
        panel: alpha(rgb(0x000000), 0.35),
        text: rgb(0xf0fbff),
        dim: alpha(rgb(0xa8dde6), 0.85),
        selected: rgb(0x5fe1d6),
        activated: rgb(0x2ec27e),
        shadow: true,
    },
    Theme {
        name: "Royal",
        top: rgb(0x3d1f6e),
        bottom: rgb(0x100720),
        panel: alpha(rgb(0x000000), 0.35),
        text: rgb(0xfaf5ff),
        dim: alpha(rgb(0xd4c2f0), 0.85),
        selected: rgb(0xc98bff),
        activated: rgb(0xf66151),
        shadow: true,
    },
    Theme {
        name: "Gold",
        top: rgb(0x221c10),
        bottom: rgb(0x050403),
        panel: alpha(rgb(0x000000), 0.5),
        text: rgb(0xfdf6e3),
        dim: alpha(rgb(0xd9c9a3), 0.85),
        selected: rgb(0xe5c07b),
        activated: rgb(0xffffff),
        shadow: true,
    },
    Theme {
        name: "Sakura",
        top: rgb(0x4a1d3b),
        bottom: rgb(0x14070f),
        panel: alpha(rgb(0x000000), 0.35),
        text: rgb(0xfff0f6),
        dim: alpha(rgb(0xf3c4d8), 0.85),
        selected: rgb(0xff8fb8),
        activated: rgb(0xffd166),
        shadow: true,
    },
    Theme {
        name: "Crimson",
        top: rgb(0x5c0a14),
        bottom: rgb(0x120205),
        panel: alpha(rgb(0x000000), 0.4),
        text: rgb(0xffffff),
        dim: alpha(rgb(0xf0b8b8), 0.85),
        selected: rgb(0xffffff),
        activated: rgb(0xff7b63),
        shadow: true,
    },
    Theme {
        name: "Arctic",
        top: rgb(0xf4f8fb),
        bottom: rgb(0xcfdce6),
        panel: alpha(rgb(0xffffff), 0.7),
        text: rgb(0x0f2233),
        dim: alpha(rgb(0x4a6275), 0.9),
        selected: rgb(0x0b6bcb),
        activated: rgb(0xe66100),
        shadow: false,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    TvShow,
    Movie,
    Collection,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::TvShow, Category::Movie, Category::Collection];

    pub fn name(self) -> String {
        match self {
            Category::TvShow => gettext("TV Show"),
            Category::Movie => gettext("Movie"),
            Category::Collection => gettext("Collection"),
        }
    }

    pub fn styles(self) -> Vec<Style> {
        Style::ALL.into_iter().filter(|s| s.category() == self).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Panel with the title and buttons, artwork on the right; thumbnail grid.
    Classic,
    /// Classic with a text episode list instead of thumbnails.
    ClassicList,
    /// Full-screen artwork fading into the title; large episode cards.
    Showcase,
    /// Showcase with a list of episodes over the artwork.
    ShowcaseList,
    /// Centred type on a plain background; a two-column episode list.
    Minimal,
    /// Artwork top right, title and buttons left; episodes as rows.
    Streaming,
    /// A colored side panel beside a mosaic of episode stills.
    Split,
    /// Bold condensed type and a TV-guide style episode listing.
    Broadcast,
    MovieClassic,
    MovieShowcase,
    MovieMinimal,
    Collection,
}

impl Style {
    pub const ALL: [Style; 12] = [
        Style::Classic,
        Style::ClassicList,
        Style::Showcase,
        Style::ShowcaseList,
        Style::Minimal,
        Style::Streaming,
        Style::Split,
        Style::Broadcast,
        Style::MovieClassic,
        Style::MovieShowcase,
        Style::MovieMinimal,
        Style::Collection,
    ];

    pub fn category(self) -> Category {
        match self {
            Style::Classic
            | Style::ClassicList
            | Style::Showcase
            | Style::ShowcaseList
            | Style::Minimal
            | Style::Streaming
            | Style::Split
            | Style::Broadcast => {
                Category::TvShow
            }
            Style::MovieClassic | Style::MovieShowcase | Style::MovieMinimal => Category::Movie,
            Style::Collection => Category::Collection,
        }
    }

    pub fn name(self) -> String {
        match self {
            Style::Classic | Style::MovieClassic => gettext("Classic"),
            Style::ClassicList => gettext("Classic List"),
            Style::Showcase | Style::MovieShowcase => gettext("Showcase"),
            Style::ShowcaseList => gettext("Showcase List"),
            Style::Minimal | Style::MovieMinimal => gettext("Minimal"),
            Style::Streaming => gettext("Streaming"),
            Style::Split => gettext("Split"),
            Style::Broadcast => gettext("Broadcast"),
            Style::Collection => gettext("Simple List"),
        }
    }

    pub fn description(self) -> String {
        match self {
            Style::Classic => gettext("A panel with the title and buttons beside the artwork, and an episode grid with thumbnails"),
            Style::ClassicList => gettext("The classic main menu with a list of episode names and running times"),
            Style::Showcase => gettext("Full-screen artwork fading into a large title, with big episode cards"),
            Style::ShowcaseList => gettext("The showcase main menu, with a list of episode names and running times over the artwork"),
            Style::Minimal => gettext("Elegant centred type on a plain background and a two-column episode list"),
            Style::Streaming => gettext("Artwork in the corner, the title and buttons on the left, and episode rows with stills"),
            Style::Split => gettext("A colored side panel with a season badge beside a mosaic of episode stills"),
            Style::Broadcast => gettext("Bold condensed type, a header bar and a TV-guide style episode listing"),
            Style::MovieClassic => gettext("Full-screen artwork with Play Movie, scene selection and extras"),
            Style::MovieShowcase => gettext("Artwork fading into a large title, with scene cards"),
            Style::MovieMinimal => gettext("Centred type on a plain background, with a chapter list"),
            Style::Collection => gettext("A simple list of every title with Play All"),
        }
    }

    /// Short name for the command line.
    pub fn id(self) -> &'static str {
        match self {
            Style::Classic => "show",
            Style::ClassicList => "show-list",
            Style::Showcase => "showcase",
            Style::ShowcaseList => "showcase-list",
            Style::Minimal => "minimal",
            Style::Streaming => "streaming",
            Style::Split => "split",
            Style::Broadcast => "broadcast",
            Style::MovieClassic => "movie",
            Style::MovieShowcase => "movie-showcase",
            Style::MovieMinimal => "movie-minimal",
            Style::Collection => "list",
        }
    }

    pub fn from_id(id: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|s| s.id() == id)
    }

    /// Palette the style looks best in.
    pub fn default_theme(self) -> usize {
        match self {
            Style::Classic | Style::ClassicList | Style::Collection => 0,
            Style::Showcase | Style::ShowcaseList | Style::MovieShowcase => 3,
            Style::Minimal | Style::MovieMinimal => 7,
            Style::Streaming => 9,
            Style::Split => 5,
            Style::Broadcast => 1,
            Style::MovieClassic => 3,
        }
    }

    /// (heading, body) font families.
    fn fonts(self) -> (&'static str, &'static str) {
        match self {
            Style::Classic | Style::ClassicList | Style::MovieClassic | Style::Collection => ("Cantarell", "Cantarell"),
            Style::Showcase | Style::ShowcaseList | Style::MovieShowcase | Style::Split => ("Montserrat", "Montserrat"),
            Style::Minimal | Style::MovieMinimal => ("Montserrat", "Lato"),
            Style::Streaming => ("Lato", "Lato"),
            Style::Broadcast => ("TeX Gyre Heros Cn", "TeX Gyre Heros Cn"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub style: Style,
    pub theme: usize,
    /// Show / movie name shown on the main menu.
    pub title: String,
    /// Optional lines such as "Season 2" and "Disc 1".
    pub season: String,
    pub disc: String,
    /// Image (e.g. a transparent PNG logo) shown instead of the name.
    pub logo: Option<Id>,
}

impl Options {
    pub fn new(style: Style, title: &str) -> Self {
        Options { style, theme: style.default_theme(), title: title.into(), season: String::new(), disc: String::new(), logo: None }
    }
}

/// Items per grid page (3 × 2) and per list page.
const GRID_PAGE: usize = 6;
const LIST_PAGE: usize = 7;
const EPISODE_ROWS: usize = 7;

struct Ctx {
    theme: Theme,
    heading: &'static str,
    body: &'static str,
}

impl Ctx {
    fn new(style: Style, theme: usize) -> Self {
        let (heading, body) = style.fonts();
        Ctx { theme: THEMES[theme.min(THEMES.len() - 1)], heading, body }
    }

    /// A menu with the palette's gradient background.
    fn menu(&self, name: &str) -> Menu {
        let mut m = Menu::new(name);
        m.background.color = self.theme.top;
        m.background.gradient = Some(self.theme.bottom);
        m
    }

    /// A menu with a plain background.
    fn plain_menu(&self, name: &str, color: Rgba) -> Menu {
        let mut m = Menu::new(name);
        m.background.color = color;
        m
    }

    fn styled(&self, text: &str, rect: Rect, font: String, color: Rgba, align: Align, shadow: bool) -> MenuItem {
        let mut it = MenuItem::new_text(text, rect);
        if let ItemKind::Text(t) = &mut it.kind {
            t.style = TextStyle { font, color, align, shadow, ..Default::default() };
        }
        it
    }

    /// Body text; `dim` for secondary lines.
    fn text(&self, text: &str, rect: Rect, size: u32, weight: &str, align: Align, dim: bool) -> MenuItem {
        let color = if dim { self.theme.dim } else { self.theme.text };
        self.styled(text, rect, format!("{} {weight} {size}", self.body), color, align, self.theme.shadow && !dim)
    }

    /// Heading text in the style's display font.
    fn head(&self, text: &str, rect: Rect, size: u32, weight: &str, align: Align) -> MenuItem {
        self.styled(text, rect, format!("{} {weight} {size}", self.heading), self.theme.text, align, self.theme.shadow)
    }

    /// Small spaced capitals, e.g. "SEASON 2 · DISC 1".
    fn caps(&self, text: &str, rect: Rect, size: u32, color: Rgba, align: Align, spacing: f64) -> MenuItem {
        let mut it = self.styled(&text.to_uppercase(), rect, format!("{} Bold {size}", self.body), color, align, false);
        if let ItemKind::Text(t) = &mut it.kind {
            t.style.letter_spacing = spacing;
        }
        it
    }

    fn button(&self, label: &str, action: Action, rect: Rect, size: u32, align: Align, highlight: Highlight) -> MenuItem {
        self.button_font(label, action, rect, format!("{} Bold {size}", self.body), align, highlight)
    }

    fn button_font(&self, label: &str, action: Action, rect: Rect, font: String, align: Align, highlight: Highlight) -> MenuItem {
        let mut it = MenuItem::new_button(label, action, rect);
        let b = it.button_mut().unwrap();
        b.text = TextStyle { font, color: self.theme.text, align, shadow: self.theme.shadow, ..Default::default() };
        b.selected_color = self.theme.selected;
        b.activated_color = self.theme.activated;
        b.highlight = highlight;
        if highlight == Highlight::Fill {
            b.highlight_text = Some(self.theme.on_accent());
            b.text.shadow = false;
        }
        it
    }

    fn thumb_button(&self, label: &str, action: Action, rect: Rect, asset: Id, time: f64) -> MenuItem {
        let mut it = self.button(label, action, rect, 30, Align::Center, Highlight::Frame);
        let b = it.button_mut().unwrap();
        b.thumbnail = Some(asset);
        b.thumbnail_time = time;
        it
    }

    fn panel(&self, rect: Rect, radius: f64) -> MenuItem {
        MenuItem::new_shape(self.theme.panel, radius, rect)
    }

    /// A shape with a gradient from `from` to `to`.
    fn gradient(&self, rect: Rect, from: Rgba, to: Rgba, horizontal: bool) -> MenuItem {
        let mut it = MenuItem::new_shape(from, 0.0, rect);
        if let ItemKind::Shape(s) = &mut it.kind {
            s.gradient = Some(to);
            s.horizontal = horizontal;
        }
        it
    }

    /// An episode still (a frame of `asset`) filling `rect`, cropped to its
    /// shape.
    fn still(&self, asset: Id, time: f64, rect: Rect) -> MenuItem {
        let mut it = MenuItem::new_image(asset, rect);
        if let ItemKind::Image(img) = &mut it.kind {
            img.time = time;
            // Crop a 16:9 frame to the rectangle's shape.
            let (aspect, video) = (rect.w / rect.h, 16.0 / 9.0);
            if aspect < video {
                let c = (1.0 - aspect / video) / 2.0;
                img.crop = [c, 0.0, c, 0.0];
            } else if aspect > video {
                let c = (1.0 - video / aspect) / 2.0;
                img.crop = [0.0, c, 0.0, c];
            }
        }
        it
    }

    /// Previous / home / next buttons along the bottom of a page.
    fn nav_bar(&self, m: &mut Menu, prev: Option<Id>, home: Option<(Id, String)>, next: Option<Id>) {
        // Bottom edge stays inside the title-safe area.
        self.nav_row(m, prev, home, next, (200.0, 944.0, 320.0, 600.0), 34);
    }

    /// Previous / home / next buttons at (x, y), each `w` wide and `step`
    /// apart, in `size` text.
    fn nav_row(&self, m: &mut Menu, prev: Option<Id>, home: Option<(Id, String)>, next: Option<Id>, (x, y, w, step): (f64, f64, f64, f64), size: u32) {
        let h = 76.0;
        if let Some(p) = prev {
            m.items.push(self.button(&gettext("‹ Previous"), Action::ShowMenu(p), Rect::new(x, y, w, h), size, Align::Center, Highlight::Underline));
        }
        if let Some((id, label)) = home {
            m.items.push(self.button(&label, Action::ShowMenu(id), Rect::new(x + step, y, w, h), size, Align::Center, Highlight::Underline));
        }
        if let Some(n) = next {
            m.items.push(self.button(&gettext("Next ›"), Action::ShowMenu(n), Rect::new(x + 2.0 * step, y, w, h), size, Align::Center, Highlight::Underline));
        }
    }

    /// `n` empty pages named after `heading`, and their ids.
    fn pages(&self, heading: &str, n: usize, make: impl Fn(&str) -> Menu) -> (Vec<Menu>, Vec<Id>) {
        let pages: Vec<Menu> = (0..n.max(1)).map(|i| make(&if n > 1 { format!("{heading} {}", i + 1) } else { heading.to_string() })).collect();
        let ids = pages.iter().map(|p| p.id).collect();
        (pages, ids)
    }

    fn page_label(&self, pi: usize, n: usize) -> String {
        gettext("Page {} of {}").replacen("{}", &(pi + 1).to_string(), 1).replacen("{}", &n.to_string(), 1)
    }

    /// Paginated grid of thumbnail buttons. `entries` are (label, action,
    /// thumbnail asset, frame time). Returns the pages.
    fn grid_pages(&self, heading: &str, entries: &[(String, Action, Id, f64)], home: Option<(Id, String)>, empty_hint: &str) -> Vec<Menu> {
        let n_pages = entries.len().div_ceil(GRID_PAGE).max(1);
        let (mut pages, ids) = self.pages(heading, n_pages, |n| self.menu(n));
        let (cw, gap_x, gap_y) = (460.0, 60.0, 44.0);
        let ch = cw * 9.0 / 16.0 + 64.0;
        let x0 = (DESIGN_WIDTH - (3.0 * cw + 2.0 * gap_x)) / 2.0;
        let y0 = 215.0;
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.head(heading, Rect::new(x0, 70.0, 1000.0, 110.0), 64, "ExtraBold", Align::Left));
            if n_pages > 1 {
                page.items.push(self.text(&self.page_label(pi, n_pages), Rect::new(DESIGN_WIDTH - x0 - 500.0, 95.0, 500.0, 70.0), 30, "", Align::Right, true));
            }
            let slice = entries.iter().skip(pi * GRID_PAGE).take(GRID_PAGE);
            for (k, (label, action, asset, time)) in slice.enumerate() {
                let (r, c) = (k / 3, k % 3);
                let rect = Rect::new(x0 + c as f64 * (cw + gap_x), y0 + r as f64 * (ch + gap_y), cw, ch);
                page.items.push(self.thumb_button(label, *action, rect, *asset, *time));
            }
            if entries.is_empty() {
                page.items.push(self.text(empty_hint, Rect::new(260.0, 420.0, 1400.0, 200.0), 36, "", Align::Center, true));
            }
            let prev = pi.checked_sub(1).map(|i| ids[i]);
            let next = ids.get(pi + 1).copied();
            self.nav_bar(page, prev, home.clone(), next);
        }
        pages
    }

    /// Paginated episode list: one full-width row per episode with its
    /// running time. `entries` are (label, action, duration).
    fn episode_list_pages(&self, heading: &str, entries: &[(String, Action, f64)], home: Option<(Id, String)>, empty_hint: &str) -> Vec<Menu> {
        let n_pages = entries.len().div_ceil(EPISODE_ROWS).max(1);
        let (mut pages, ids) = self.pages(heading, n_pages, |n| self.menu(n));
        let (x0, w) = (220.0, 1480.0);
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.head(heading, Rect::new(x0 + 20.0, 70.0, 1000.0, 110.0), 64, "ExtraBold", Align::Left));
            if n_pages > 1 {
                page.items.push(self.text(&self.page_label(pi, n_pages), Rect::new(x0 + w - 520.0, 95.0, 500.0, 70.0), 30, "", Align::Right, true));
            }
            page.items.push(self.panel(Rect::new(x0, 200.0, w, 730.0), 32.0));
            let slice = entries.iter().skip(pi * EPISODE_ROWS).take(EPISODE_ROWS);
            for (k, (label, action, duration)) in slice.enumerate() {
                let y = 222.0 + k as f64 * 98.0;
                page.items.push(self.text(&clock(*duration), Rect::new(x0 + w - 300.0, y, 260.0, 86.0), 30, "", Align::Right, true));
                page.items.push(self.button(label, *action, Rect::new(x0 + 20.0, y, w - 40.0, 86.0), 40, Align::Left, Highlight::Frame));
            }
            if entries.is_empty() {
                page.items.push(self.text(empty_hint, Rect::new(260.0, 480.0, 1400.0, 120.0), 36, "", Align::Center, true));
            }
            let prev = pi.checked_sub(1).map(|i| ids[i]);
            let next = ids.get(pi + 1).copied();
            self.nav_bar(page, prev, home.clone(), next);
        }
        pages
    }

    /// One row of pop-up buttons centred in the strip at the bottom.
    fn popup_row(&self, m: &mut Menu, entries: &[(String, Action)]) -> Vec<Id> {
        const Y: f64 = 872.0;
        const GAP: f64 = 24.0;
        let widths: Vec<f64> = entries.iter().map(|(l, _)| (56.0 + l.chars().count() as f64 * 19.0).max(88.0)).collect();
        let total: f64 = widths.iter().sum::<f64>() + GAP * (entries.len().saturating_sub(1)) as f64;
        let mut x = ((DESIGN_WIDTH - total) / 2.0).max(SAFE_AREA.x);
        let mut ids = Vec::new();
        for ((label, action), w) in entries.iter().zip(widths) {
            let b = self.button(label, *action, Rect::new(x, Y, w, 80.0), 34, Align::Center, Highlight::Frame);
            ids.push(b.id);
            m.items.push(b);
            x += w + GAP;
        }
        ids
    }

    fn popup_page(&self, name: &str, heading: &str) -> Menu {
        let mut m = Menu::new_popup(name);
        m.fade_in = 0.3;
        m.items.push(MenuItem::new_shape(alpha(self.theme.bottom, 0.82), 0.0, Rect::new(0.0, 800.0, DESIGN_WIDTH, 280.0)));
        m.items.push(MenuItem::new_shape(self.theme.selected, 0.0, Rect::new(0.0, 800.0, DESIGN_WIDTH, 4.0)));
        m.items.push(self.text(heading, Rect::new(SAFE_AREA.x, 812.0, 800.0, 50.0), 26, "Bold", Align::Left, true));
        m
    }

    /// Pop-up menu with chapters, audio & subtitles and links out, plus
    /// the pages it opens. The first menu is the pop-up itself.
    fn popup_menus(&self, name: &str, max_chapters: usize, languages: &[LanguagePreset], links: &[(String, Action)]) -> Vec<Menu> {
        let mut main = self.popup_page(&gettext("Pop-up Menu"), name);
        let mut chapters = self.popup_page(&gettext("Pop-up · Chapters"), &gettext("Chapters"));
        let mut audio = self.popup_page(&gettext("Pop-up · Audio & Subtitles"), &gettext("Audio & Subtitles"));
        let mut entries = Vec::new();
        if max_chapters > 1 {
            entries.push((gettext("Chapters"), Action::ShowMenu(chapters.id)));
        }
        if languages.len() >= 2 {
            entries.push((gettext("Audio & Subtitles"), Action::ShowMenu(audio.id)));
        }
        entries.extend(links.iter().cloned());
        self.popup_row(&mut main, &entries);

        let back = (gettext("‹ Back"), Action::ShowMenu(main.id));
        let mut ch = vec![back.clone()];
        ch.extend((0..max_chapters.min(12)).map(|i| ((i + 1).to_string(), Action::PlayChapter(i as u32))));
        let ids = self.popup_row(&mut chapters, &ch);
        chapters.default_button = ids.get(1).copied();

        let mut au = vec![back];
        au.extend(languages.iter().map(|l| (l.name.clone(), Action::SetLanguage { preset: l.id, menu: Some(main.id) })));
        let ids = self.popup_row(&mut audio, &au);
        audio.default_button = ids.get(1).copied();

        let mut out = vec![main];
        if max_chapters > 1 {
            out.push(chapters);
        }
        if languages.len() >= 2 {
            out.push(audio);
        }
        out
    }

    /// Language choices, each returning to `home`.
    fn setup_page(&self, languages: &[LanguagePreset], default: Option<Id>, home: Id) -> Menu {
        let mut m = self.menu(&gettext("Setup"));
        let n = languages.len().min(6);
        let h = 190.0 + n as f64 * 100.0;
        let top = ((DESIGN_HEIGHT - h) / 2.0).max(60.0);
        m.items.push(self.panel(Rect::new(430.0, top, 1060.0, h), 36.0));
        m.items.push(self.head(&gettext("Audio & Subtitles"), Rect::new(480.0, top + 30.0, 960.0, 110.0), 56, "ExtraBold", Align::Center));
        for (k, l) in languages.iter().take(6).enumerate() {
            let rect = Rect::new(530.0, top + 160.0 + k as f64 * 100.0, 860.0, 84.0);
            let b = self.button(&l.name, Action::SetLanguage { preset: l.id, menu: Some(home) }, rect, 40, Align::Center, Highlight::Frame);
            if default == Some(l.id) {
                m.default_button = Some(b.id);
            }
            m.items.push(b);
        }
        self.nav_bar(&mut m, None, Some((home, gettext("Main Menu"))), None);
        m
    }

    /// Paginated vertical list of text buttons.
    fn list_pages(&self, heading: &str, page_name: &str, entries: &[(String, Action)], home: Option<(Id, String)>) -> Vec<Menu> {
        let n_pages = entries.len().div_ceil(LIST_PAGE).max(1);
        let (mut pages, ids) = self.pages(page_name, n_pages, |n| self.menu(n));
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.panel(Rect::new(430.0, 60.0, 1060.0, 870.0), 36.0));
            page.items.push(self.head(heading, Rect::new(480.0, 90.0, 960.0, 120.0), 60, "ExtraBold", Align::Center));
            let slice: Vec<&(String, Action)> = entries.iter().skip(pi * LIST_PAGE).take(LIST_PAGE).collect();
            for (k, (label, action)) in slice.iter().enumerate() {
                let rect = Rect::new(530.0, 240.0 + k as f64 * 94.0, 860.0, 80.0);
                page.items.push(self.button(label, *action, rect, 38, Align::Center, Highlight::Underline));
            }
            let prev = pi.checked_sub(1).map(|i| ids[i]);
            let next = ids.get(pi + 1).copied();
            self.nav_bar(page, prev, home.clone(), next);
        }
        pages
    }
}

/// A title as shown in templates.
#[derive(Debug, Clone)]
struct Episode {
    title: Id,
    asset: Id,
    number: usize,
    /// Name without episode numbering ("" when there is none).
    name: String,
    /// Thumbnail frame.
    poster: f64,
    duration: f64,
}

impl Episode {
    fn action(&self) -> Action {
        Action::PlayTitle { title: self.title, chapter: 0 }
    }

    /// "3 · Name", or "Episode 3" without a name.
    fn label(&self, sep: &str, max: usize) -> String {
        if self.name.is_empty() {
            gettext("Episode {}").replace("{}", &self.number.to_string())
        } else {
            format!("{}{sep}{}", self.number, short(&self.name, max))
        }
    }
}

/// What every style gets to build from.
struct Input<'a> {
    p: &'a Project,
    cx: Ctx,
    /// Show or movie name.
    name: String,
    season: String,
    disc: String,
    episodes: Vec<Episode>,
    main: Menu,
    setup: Option<Id>,
}

impl Input<'_> {
    /// "Season 2 · Disc 1" (either part optional).
    fn edition(&self) -> Option<String> {
        let parts: Vec<&str> = [self.season.trim(), self.disc.trim()].into_iter().filter(|s| !s.is_empty()).collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    fn count(&self) -> String {
        let n = self.episodes.len() as u32;
        gettextrs::ngettext("{} Episode", "{} Episodes", n).replace("{}", &n.to_string())
    }

    fn home(&self) -> Option<(Id, String)> {
        Some((self.main.id, gettext("Main Menu")))
    }

    fn hint(&self) -> String {
        gettext("Import your episodes, then apply the template again")
    }
}

/// What a style made.
struct Output {
    /// Main menu first.
    menus: Vec<Menu>,
    /// (title, menu it returns to).
    return_to: Vec<(Id, Id)>,
    /// Text item with the name, replaced by the logo when there is one.
    title_item: Option<Id>,
    /// Extra buttons for the pop-up menu (e.g. Episodes).
    popup_links: Vec<(String, Action)>,
}

/// The theme the project's menus were made with (by background color).
pub fn detect_theme(p: &Project) -> usize {
    p.first_menu()
        .and_then(|m| THEMES.iter().position(|t| t.top == m.background.color || t.bottom == m.background.color))
        .unwrap_or(0)
}

/// Paginated scene selection for a title, in the project's theme; buttons
/// on the last row lead back to `home`. Returns the pages.
pub fn chapter_menus(p: &Project, title: Id, home: Option<Id>) -> Vec<Menu> {
    let cx = Ctx::new(Style::Classic, detect_theme(p));
    let Some(t) = p.title(title) else { return vec![] };
    let back = home.and_then(|h| p.menu(h)).map(|m| (m.id, m.name.clone()));
    cx.grid_pages(&format!("{} – {}", t.name, gettext("Chapters")), &scene_entries(p, title), back, "")
}

/// (label, action, asset, frame) for each chapter of a title.
fn scene_entries(p: &Project, title: Id) -> Vec<(String, Action, Id, f64)> {
    let Some(t) = p.title(title) else { return vec![] };
    let starts: Vec<f64> = std::iter::once(0.0).chain(t.chapters.iter().copied()).collect();
    let duration = p.asset(t.asset).map_or(0.0, |a| a.info.duration);
    starts
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let label = format!("{} {} · {}", gettext("Chapter"), i + 1, clock(c));
            // A frame a little into the chapter, avoiding black cuts.
            let frame = (c + 3.0).min((duration - 0.5).max(c));
            (label, Action::PlayTitle { title, chapter: i as u32 }, t.asset, frame)
        })
        .collect()
}

/// Drop leading episode numbering such as "E01 - ", "S01E02 ", "1x03. "
/// or "03 " from a file-derived name.
pub fn episode_name(name: &str) -> String {
    let s = name.trim();
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    let upper = |c: char| c.to_ascii_uppercase();
    // [S<n>]E<n> | <n>x<n> | <n>
    let mut matched = false;
    if i < bytes.len() && upper(bytes[i]) == 'S' {
        let mut j = i + 1;
        if digits(&mut j) && j < bytes.len() && upper(bytes[j]) == 'E' {
            j += 1;
            if digits(&mut j) {
                i = j;
                matched = true;
            }
        }
    }
    if !matched && i < bytes.len() && upper(bytes[i]) == 'E' {
        let mut j = i + 1;
        if digits(&mut j) {
            i = j;
            matched = true;
        }
    }
    if !matched {
        let mut j = i;
        if digits(&mut j) {
            if j < bytes.len() && upper(bytes[j]) == 'X' {
                let mut k = j + 1;
                if digits(&mut k) {
                    j = k;
                }
            }
            i = j;
            matched = true;
        }
    }
    if !matched {
        return s.to_string();
    }
    let rest: String = bytes[i..].iter().collect();
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || "-–—._:·".contains(c)).trim();
    rest.replace(['_', '.'], " ")
}

/// Display size for a title: smaller when it will wrap.
fn title_size(name: &str, large: u32, small: u32, limit: usize) -> u32 {
    if name.chars().count() > limit {
        small
    } else {
        large
    }
}

/// "23:40" or "1:02:15".
fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// Replace the project's menus with a generated set. Titles, assets and
/// disc settings are kept. Returns the id of the main menu.
pub fn apply(p: &mut Project, opts: &Options) -> Id {
    let cx = Ctx::new(opts.style, opts.theme);
    let name = if opts.title.trim().is_empty() { p.disc.name.clone() } else { opts.title.clone() };

    // Setup menu for choosing audio and subtitles, when there is a choice.
    if p.disc.languages.is_empty() {
        p.disc.languages = p.suggest_languages();
    }
    let main = cx.menu(&gettext("Main Menu"));
    let main_id = main.id;
    let setup = (p.disc.languages.len() >= 2).then(|| cx.setup_page(&p.disc.languages, p.default_language().map(|l| l.id), main_id));

    let episodes = episodes(p);
    let input = Input {
        p,
        cx,
        name: name.clone(),
        season: opts.season.clone(),
        disc: opts.disc.clone(),
        episodes,
        main,
        setup: setup.as_ref().map(|m| m.id),
    };
    let out = match opts.style.category() {
        Category::TvShow => show::build(opts.style, input),
        Category::Movie => movie::build(opts.style, input),
        Category::Collection => collection(input),
    };
    let cx = Ctx::new(opts.style, opts.theme);
    let page_ids: Vec<Id> = out.menus.iter().skip(1).map(|m| m.id).collect();
    let mut menus = out.menus;

    // Use the logo in place of the name on the main menu.
    if let Some(logo) = opts.logo.and_then(|id| p.asset(id)) {
        let aspect = if logo.info.height > 0 { logo.info.width as f64 / logo.info.height as f64 } else { 1.0 };
        let logo_id = logo.id;
        if let (Some(main), Some(title)) = (menus.first_mut(), out.title_item) {
            if let Some(item) = main.items.iter_mut().find(|i| i.id == title) {
                let align = match &item.kind {
                    ItemKind::Text(t) => t.style.align,
                    _ => Align::Center,
                };
                item.rect = item.rect.fit(aspect, align);
                item.kind = ItemKind::Image(ImageItem::new(logo_id));
            }
        }
    }
    menus.extend(setup);

    // Pop-up menu for every title.
    let max_chapters = p.titles.iter().map(|t| t.chapters.len() + 1).max().unwrap_or(0);
    let mut links = out.popup_links;
    links.push((gettext("Top Menu"), Action::ShowMenu(main_id)));
    let popups = cx.popup_menus(&name, max_chapters, &p.disc.languages, &links);
    p.disc.popup_menu = popups.first().map(|m| m.id);
    menus.extend(popups);
    p.menus = menus;
    for t in &mut p.titles {
        t.return_menu = out.return_to.iter().find(|(id, _)| *id == t.id).map(|(_, m)| *m);
    }
    p.first_play = FirstPlay::FirstMenu;
    p.template = Some(TemplateInfo {
        style: opts.style.id().into(),
        theme: opts.theme,
        title: opts.title.clone(),
        season: opts.season.clone(),
        disc: opts.disc.clone(),
        main: main_id,
        pages: page_ids,
    });
    main_id
}

fn episodes(p: &Project) -> Vec<Episode> {
    p.titles
        .iter()
        .enumerate()
        .map(|(i, t)| Episode {
            title: t.id,
            asset: t.asset,
            number: i + 1,
            name: episode_name(&t.name),
            poster: p.title_poster(t.id),
            duration: p.asset(t.asset).map_or(0.0, |a| a.info.duration),
        })
        .collect()
}

/// The episode pages of the project: those the last template made, or
/// menus named like them.
pub fn episode_pages(p: &Project) -> Vec<Id> {
    if let Some(t) = p.template.as_ref().filter(|t| Style::from_id(&t.style).is_some_and(|s| s.category() == Category::TvShow)) {
        let ids: Vec<Id> = t.pages.iter().copied().filter(|id| p.menu(*id).is_some()).collect();
        if !ids.is_empty() {
            return ids;
        }
    }
    let heading = gettext("Episodes");
    p.menus
        .iter()
        .filter(|m| !m.popup && (m.name == heading || m.name.strip_prefix(&heading).is_some_and(|rest| rest.trim().parse::<u32>().is_ok())))
        .map(|m| m.id)
        .collect()
}

/// The options of the last template applied, or a guess from the menus.
pub fn last_options(p: &Project) -> Options {
    match &p.template {
        Some(t) => Options {
            style: Style::from_id(&t.style).filter(|s| s.category() == Category::TvShow).unwrap_or(Style::Showcase),
            theme: t.theme.min(THEMES.len() - 1),
            title: t.title.clone(),
            season: t.season.clone(),
            disc: t.disc.clone(),
            logo: None,
        },
        None => Options { theme: detect_theme(p), ..Options::new(Style::Classic, &p.disc.name) },
    }
}

/// Make the episode pages again in `opts`' style, replacing menus `old`
/// (keeping their place in the list). Links from other menus, titles'
/// return menus and the Top Menu setting move to the new pages. Returns
/// the new pages.
pub fn regenerate_pages(p: &mut Project, opts: &Options, old: &[Id]) -> Vec<Id> {
    let main = p
        .template
        .as_ref()
        .map(|t| t.main)
        .filter(|m| p.menu(*m).is_some() && !old.contains(m))
        .or_else(|| p.menus.iter().find(|m| !m.popup && !old.contains(&m.id)).map(|m| m.id));
    let cx = Ctx::new(opts.style, opts.theme);
    let temp_main = cx.menu(&gettext("Main Menu"));
    let temp_id = temp_main.id;
    let name = if opts.title.trim().is_empty() { p.disc.name.clone() } else { opts.title.clone() };
    let input = Input { p, cx, name, season: opts.season.clone(), disc: opts.disc.clone(), episodes: episodes(p), main: temp_main, setup: None };
    let out = show::build(opts.style, input);
    // The main menu comes first; the rest are the pages.
    let mut pages: Vec<Menu> = out.menus.into_iter().skip(1).collect();
    let home = |id: Id| (id == temp_id).then_some(main).flatten();
    for m in &mut pages {
        m.relink(&home);
    }
    let new_ids: Vec<Id> = pages.iter().map(|m| m.id).collect();
    // Old page n becomes new page n (or the first when there are fewer).
    let map = |id: Id| old.iter().position(|o| *o == id).and_then(|i| new_ids.get(i).or(new_ids.first()).copied());

    let at = p.menus.iter().take_while(|m| !old.contains(&m.id)).count();
    p.menus.retain(|m| !old.contains(&m.id));
    let at = at.min(p.menus.len());
    p.menus.splice(at..at, pages);
    for m in &mut p.menus {
        m.relink(&map);
    }
    if let Some(top) = p.disc.top_menu {
        p.disc.top_menu = map(top).or(Some(top));
    }
    for t in &mut p.titles {
        let from_template = t.return_menu.is_none_or(|m| old.contains(&m));
        if from_template {
            t.return_menu = out.return_to.iter().find(|(id, _)| *id == t.id).map(|(_, m)| *m).or(t.return_menu.and_then(map));
        }
    }
    let info = p.template.get_or_insert(TemplateInfo {
        style: String::new(),
        theme: 0,
        title: opts.title.clone(),
        season: opts.season.clone(),
        disc: opts.disc.clone(),
        main: main.unwrap_or(temp_id),
        pages: vec![],
    });
    info.style = opts.style.id().into();
    info.theme = opts.theme;
    info.season = opts.season.clone();
    info.disc = opts.disc.clone();
    info.pages = new_ids.clone();
    new_ids
}

/// A simple list of every title; its first page is the main menu.
fn collection(input: Input) -> Output {
    let Input { cx, name, mut main, episodes, setup, .. } = input;
    let main_id = main.id;
    let mut entries: Vec<(String, Action)> = Vec::new();
    if episodes.len() > 1 {
        entries.push((gettext("Play All"), Action::PlayAll));
    }
    entries.extend(episodes.iter().map(|e| (short(&e.name_or_title(), 36), e.action())));
    if let Some(s) = setup {
        entries.push((gettext("Setup"), Action::ShowMenu(s)));
    }
    let mut pages = cx.list_pages(&name, &gettext("More Titles"), &entries, None);
    let first = pages.remove(0);
    main.items = first.items;
    let first_button = main.buttons().next().map(|b| b.id);
    main.default_button = first_button;
    let title_item = main.items.iter().find(|i| matches!(&i.kind, ItemKind::Text(t) if t.text == name)).map(|i| i.id);
    // Re-point links that targeted the first page.
    for m in &mut pages {
        for it in &mut m.items {
            if let Some(b) = it.button_mut() {
                if b.action == Action::ShowMenu(first.id) {
                    b.action = Action::ShowMenu(main_id);
                }
            }
        }
    }
    let offset = usize::from(episodes.len() > 1);
    let return_to = episodes
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let page = (i + offset) / LIST_PAGE;
            (e.title, if page == 0 { main_id } else { pages[page - 1].id })
        })
        .collect();
    let mut menus = vec![main];
    menus.extend(pages);
    Output { menus, return_to, title_item, popup_links: vec![] }
}

impl Episode {
    /// The name, or the full title name when it had only a number.
    fn name_or_title(&self) -> String {
        if self.name.is_empty() {
            gettext("Title {}").replace("{}", &self.number.to_string())
        } else {
            self.name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::probe::MediaInfo;

    fn project(n: usize) -> Project {
        let mut p = Project::default();
        for i in 0..n {
            let id = new_id();
            p.assets.push(Asset {
                id,
                path: format!("/tmp/ep{i}.mkv").into(),
                kind: AssetKind::Video,
                info: MediaInfo { duration: 1200.0, video_codec: Some("h264".into()), ..Default::default() },
            });
            p.ensure_title_for(id);
        }
        p
    }

    fn opts(style: Style, theme: usize) -> Options {
        Options { theme, ..Options::new(style, "My Show") }
    }

    #[test]
    fn regenerating_keeps_other_menus() {
        let mut p = project(12);
        let main = apply(&mut p, &opts(Style::Showcase, 3));
        let old = episode_pages(&p);
        assert_eq!(old.len(), 2);
        // Work on the main menu, and a menu of the user's own linking to
        // the second episode page.
        let note = MenuItem::new_text("My own note", Rect::new(1200.0, 100.0, 500.0, 80.0));
        let note_id = note.id;
        p.menu_mut(main).unwrap().items.push(note);
        let mut extras = Menu::new("Extras");
        extras.items.push(MenuItem::new_button("More episodes", Action::ShowMenu(old[1]), Rect::new(200.0, 200.0, 600.0, 90.0)));
        let extras_id = extras.id;
        p.menus.insert(1, extras);
        // Six more episodes arrive.
        for i in 12..18 {
            let id = new_id();
            p.assets.push(Asset { id, path: format!("/tmp/ep{i}.mkv").into(), kind: AssetKind::Video, info: MediaInfo { duration: 1200.0, video_codec: Some("h264".into()), ..Default::default() } });
            p.ensure_title_for(id);
        }

        let o = last_options(&p);
        assert_eq!((o.style, o.theme), (Style::Showcase, 3));
        let new = regenerate_pages(&mut p, &Options { style: Style::ShowcaseList, ..o }, &old);
        assert_eq!(new.len(), 3);
        assert!(old.iter().all(|id| p.menu(*id).is_none()));
        assert!(p.menu(main).unwrap().item(note_id).is_some(), "main menu edits are kept");
        let extras = p.menu(extras_id).unwrap();
        assert_eq!(extras.buttons().next().unwrap().button().unwrap().action, Action::ShowMenu(new[1]));
        // The main menu's Episodes button and the pop-up link follow.
        assert!(p.menus.iter().flat_map(|m| m.buttons()).any(|b| b.button().unwrap().action == Action::ShowMenu(new[0])));
        // Pages link home to the real main menu, and every title returns to its page.
        assert!(p.menu(new[2]).unwrap().buttons().any(|b| b.button().unwrap().action == Action::ShowMenu(main)));
        assert_eq!(p.titles[17].return_menu, Some(new[2]));
        assert_eq!(episode_pages(&p), new);
        // No link is left pointing at a removed page.
        for m in &p.menus {
            for b in m.buttons() {
                if let Some(target) = b.button().unwrap().action.menu() {
                    assert!(p.menu(target).is_some(), "{} links to a missing menu", m.name);
                }
            }
        }
        assert!(p.titles.iter().all(|t| t.return_menu.is_some_and(|m| p.menu(m).is_some())));
    }

    #[test]
    fn finds_episode_pages_by_name() {
        let mut p = project(3);
        apply(&mut p, &opts(Style::Classic, 0));
        p.template = None;
        let found = episode_pages(&p);
        assert_eq!(found.len(), 1);
        assert_eq!(p.menu(found[0]).unwrap().name, "Episodes");
    }

    /// Every link points at an existing menu/title and every item is on screen.
    fn check(p: &Project) {
        for m in &p.menus {
            for it in &m.items {
                let r = it.rect;
                assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= DESIGN_WIDTH + 0.5 && r.y + r.h <= DESIGN_HEIGHT + 0.5, "{} off screen: {r:?}", m.name);
                if let Some(b) = it.button() {
                    match b.action {
                        Action::ShowMenu(id) => assert!(p.menu(id).is_some()),
                        Action::PlayTitle { title, .. } => assert!(p.title(title).is_some()),
                        _ => {}
                    }
                }
            }
            assert!(m.buttons().count() > 0, "menu {} has no buttons", m.name);
        }
        // Templates produce no warnings (the test videos don't exist, so
        // only missing-file errors are expected).
        let warnings: Vec<_> = crate::validate::check(p)
            .into_iter()
            .filter(|i| i.severity == crate::validate::Severity::Warning && i.target != Some(crate::validate::Target::Settings))
            .collect();
        assert!(warnings.is_empty(), "{warnings:#?}");
    }

    #[test]
    fn every_style_is_consistent() {
        for style in Style::ALL {
            for n in [0, 1, 3, 9, 20] {
                for (season, disc) in [("", ""), ("Season 2", "Disc 1")] {
                    let mut p = project(n);
                    let o = Options { season: season.into(), disc: disc.into(), ..opts(style, style.default_theme()) };
                    let main = apply(&mut p, &o);
                    if n > 0 {
                        check(&p);
                    }
                    // Every title is reachable from the main menu's pages.
                    assert!(p.menu(main).is_some(), "{style:?}");
                    let played: std::collections::HashSet<Id> = p
                        .disc_menus()
                        .flat_map(|m| m.buttons().filter_map(|b| match b.button().unwrap().action {
                            Action::PlayTitle { title, .. } => Some(title),
                            _ => None,
                        }))
                        .collect();
                    if style.category() == Category::Movie {
                        continue;
                    }
                    for t in &p.titles {
                        assert!(played.contains(&t.id), "{style:?} with {n} titles can't play {}", t.name);
                    }
                }
            }
        }
    }

    #[test]
    fn edition_lines_appear() {
        for style in Style::ALL.into_iter().filter(|s| s.category() != Category::Collection) {
            let mut p = project(4);
            let o = Options { season: "Season 2".into(), disc: "Disc 1".into(), ..opts(style, 0) };
            let main = apply(&mut p, &o);
            let texts: String = p.menu(main).unwrap().items.iter().filter_map(|i| match &i.kind {
                ItemKind::Text(t) => Some(t.text.to_lowercase()),
                _ => None,
            }).collect::<Vec<_>>().join("|");
            assert!(texts.contains("season 2"), "{style:?}: {texts}");
            assert!(texts.contains("disc 1"), "{style:?}: {texts}");
        }
    }

    #[test]
    fn setup_menu_for_two_languages() {
        for style in Style::ALL {
            let mut p = project(3);
            for t in &mut p.titles {
                t.audio = ["eng", "jpn"]
                    .iter()
                    .map(|l| AudioTrack { id: new_id(), source: AudioSource::Embedded { index: 0 }, lang: l.to_string(), name: l.to_string(), enabled: true, layout: Default::default(), reencode: false })
                    .collect();
            }
            let main = apply(&mut p, &opts(style, 1));
            check(&p);
            assert_eq!(p.disc.languages.len(), 2);
            let setup = p.menus.iter().find(|m| m.name == "Setup").expect("setup menu");
            let choices: Vec<Action> = setup.buttons().map(|b| b.button().unwrap().action).filter(|a| matches!(a, Action::SetLanguage { .. })).collect();
            assert_eq!(choices.len(), 2);
            assert!(choices.iter().all(|a| matches!(a, Action::SetLanguage { menu: Some(m), .. } if *m == main)));
            let reaches_setup = p.menus.iter().any(|m| m.buttons().any(|b| b.button().unwrap().action == Action::ShowMenu(setup.id)));
            assert!(reaches_setup, "{style:?}");
        }
    }

    #[test]
    fn chapter_menu_pages() {
        let mut p = project(1);
        let t = p.titles[0].id;
        p.titles[0].chapters = (1..14).map(|i| i as f64 * 60.0).collect();
        let main = apply(&mut p, &opts(Style::Collection, 2));
        let pages = chapter_menus(&p, t, Some(main));
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].background.color, THEMES[2].top);
        let chapters: Vec<u32> = pages
            .iter()
            .flat_map(|m| m.buttons().filter_map(|b| match b.button().unwrap().action {
                Action::PlayTitle { chapter, .. } => Some(chapter),
                _ => None,
            }))
            .collect();
        assert_eq!(chapters, (0..14).collect::<Vec<_>>());
        // Linked in from the main menu, the pages pass the template checks.
        p.menus.extend(pages.clone());
        let first = pages[0].id;
        p.menus[0].items.push(MenuItem::new_button("Chapters", Action::ShowMenu(first), Rect::new(200.0, 900.0, 300.0, 80.0)));
        check(&p);
    }

    #[test]
    fn strips_episode_numbers() {
        assert_eq!(episode_name("E01 - Pilot"), "Pilot");
        assert_eq!(episode_name("S02E13.The_Return"), "The Return");
        assert_eq!(episode_name("1x03 Something"), "Something");
        assert_eq!(episode_name("07 Finale"), "Finale");
        assert_eq!(episode_name("Pilot"), "Pilot");
        assert_eq!(episode_name("Episode 4"), "Episode 4");
        assert_eq!(episode_name("E05"), "");
    }

    #[test]
    fn show_template_pages_episodes() {
        let mut p = project(8);
        let main = apply(&mut p, &opts(Style::Classic, 0));
        check(&p);
        // main + 2 episode pages
        assert_eq!(p.disc_menus().count(), 3);
        let m = p.menu(main).unwrap();
        assert!(m.buttons().any(|b| b.button().unwrap().action == Action::PlayAll));
        let episode_buttons: usize = p
            .disc_menus()
            .skip(1)
            .map(|m| m.buttons().filter(|b| matches!(b.button().unwrap().action, Action::PlayTitle { .. })).count())
            .sum();
        assert_eq!(episode_buttons, 8);
        // Episodes on page 2 return to page 2.
        assert_eq!(p.titles[7].return_menu, Some(p.menus[2].id));
    }

    #[test]
    fn logo_replaces_title_text() {
        let mut p = project(3);
        let logo = new_id();
        p.assets.push(Asset {
            id: logo,
            path: "/tmp/logo.png".into(),
            kind: AssetKind::Image,
            info: MediaInfo { width: 600, height: 200, ..Default::default() },
        });
        for style in Style::ALL {
            let main = apply(&mut p, &Options { logo: Some(logo), ..opts(style, 0) });
            let m = p.menu(main).unwrap();
            let img = m.items.iter().find(|i| matches!(&i.kind, ItemKind::Image(img) if img.asset == logo));
            let img = img.unwrap_or_else(|| panic!("{style:?} has no logo"));
            assert!((img.rect.w / img.rect.h - 3.0).abs() < 0.05, "logo keeps its 3:1 shape");
            assert!(!m.items.iter().any(|i| matches!(&i.kind, ItemKind::Text(t) if t.text.eq_ignore_ascii_case("My Show"))), "{style:?}");
        }
    }

    #[test]
    fn show_list_template_pages_episodes() {
        let mut p = project(9);
        apply(&mut p, &opts(Style::ClassicList, 2));
        check(&p);
        // main + 2 list pages (7 + 2)
        assert_eq!(p.disc_menus().count(), 3);
        assert!(p.disc_menus().skip(1).all(|m| m.buttons().all(|b| b.button().unwrap().thumbnail.is_none())));
        assert_eq!(p.titles[8].return_menu, Some(p.menus[2].id));
    }

    #[test]
    fn styles_and_ids_round_trip() {
        for s in Style::ALL {
            assert_eq!(Style::from_id(s.id()), Some(s));
            assert!(Category::ALL.contains(&s.category()));
            assert!(s.default_theme() < THEMES.len());
        }
        assert!(Category::TvShow.styles().len() >= 5);
    }
}
