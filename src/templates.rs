// SPDX-License-Identifier: GPL-3.0-or-later

//! Menu templates: generate a complete, linked set of menus for the
//! project's titles in one of several layouts and color themes.

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

pub const THEMES: [Theme; 5] = [
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
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Show,
    /// TV show with a text episode list instead of thumbnails.
    ShowList,
    Movie,
    List,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::Show, Layout::ShowList, Layout::Movie, Layout::List];

    pub fn name(self) -> String {
        match self {
            Layout::Show => gettext("TV Show"),
            Layout::ShowList => gettext("TV Show (List)"),
            Layout::Movie => gettext("Movie"),
            Layout::List => gettext("Simple List"),
        }
    }

    pub fn description(self) -> String {
        match self {
            Layout::Show => gettext("Play All and an episode selection menu with thumbnails"),
            Layout::ShowList => gettext("Play All and an episode list with names and running times"),
            Layout::Movie => gettext("Play Movie, scene selection for the first title, and extras"),
            Layout::List => gettext("A simple list of every title with Play All"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub layout: Layout,
    pub theme: usize,
    /// Show / movie name shown on the main menu.
    pub title: String,
    /// Image (e.g. a transparent PNG logo) shown instead of the name.
    pub logo: Option<Id>,
}

const FONT: &str = "Cantarell";
/// Items per grid page (3 × 2) and per list page.
const GRID_PAGE: usize = 6;
const LIST_PAGE: usize = 7;
const EPISODE_ROWS: usize = 7;

struct Ctx {
    theme: Theme,
}

impl Ctx {
    fn menu(&self, name: &str) -> Menu {
        let mut m = Menu::new(name);
        m.background.color = self.theme.top;
        m.background.gradient = Some(self.theme.bottom);
        m
    }

    fn text(&self, text: &str, rect: Rect, size: u32, weight: &str, align: Align, dim: bool) -> MenuItem {
        let mut it = MenuItem::new_text(text, rect);
        if let ItemKind::Text(t) = &mut it.kind {
            t.style = TextStyle {
                font: format!("{FONT} {weight} {size}"),
                color: if dim { self.theme.dim } else { self.theme.text },
                align,
                shadow: self.theme.shadow && !dim,
                ..Default::default()
            };
        }
        it
    }

    fn button(&self, label: &str, action: Action, rect: Rect, size: u32, align: Align, highlight: Highlight) -> MenuItem {
        let mut it = MenuItem::new_button(label, action, rect);
        let b = it.button_mut().unwrap();
        b.text = TextStyle { font: format!("{FONT} Bold {size}"), color: self.theme.text, align, shadow: self.theme.shadow, ..Default::default() };
        b.selected_color = self.theme.selected;
        b.activated_color = self.theme.activated;
        b.highlight = highlight;
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

    /// Previous / home / next buttons along the bottom of a page.
    fn nav_bar(&self, m: &mut Menu, prev: Option<Id>, home: Option<(Id, String)>, next: Option<Id>) {
        // Bottom edge stays inside the title-safe area.
        let y = 944.0;
        let (w, h) = (320.0, 76.0);
        if let Some(p) = prev {
            m.items.push(self.button(&gettext("‹ Previous"), Action::ShowMenu(p), Rect::new(200.0, y, w, h), 34, Align::Center, Highlight::Underline));
        }
        if let Some((id, label)) = home {
            m.items.push(self.button(&label, Action::ShowMenu(id), Rect::new(800.0, y, w, h), 34, Align::Center, Highlight::Underline));
        }
        if let Some(n) = next {
            m.items.push(self.button(&gettext("Next ›"), Action::ShowMenu(n), Rect::new(1400.0, y, w, h), 34, Align::Center, Highlight::Underline));
        }
    }

    /// Paginated grid of thumbnail buttons. `entries` are (label, action,
    /// thumbnail asset, frame time). Returns the pages.
    fn grid_pages(&self, heading: &str, entries: &[(String, Action, Id, f64)], home: Option<(Id, String)>, empty_hint: &str) -> Vec<Menu> {
        let n_pages = entries.len().div_ceil(GRID_PAGE).max(1);
        let mut pages: Vec<Menu> = (0..n_pages)
            .map(|i| self.menu(&if n_pages > 1 { format!("{heading} {}", i + 1) } else { heading.to_string() }))
            .collect();
        let ids: Vec<Id> = pages.iter().map(|p| p.id).collect();
        let (cw, gap_x, gap_y) = (460.0, 60.0, 44.0);
        let ch = cw * 9.0 / 16.0 + 64.0;
        let x0 = (DESIGN_WIDTH - (3.0 * cw + 2.0 * gap_x)) / 2.0;
        let y0 = 215.0;
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.text(heading, Rect::new(x0, 70.0, 1000.0, 110.0), 64, "ExtraBold", Align::Left, false));
            if n_pages > 1 {
                let label = gettext("Page {} of {}").replacen("{}", &(pi + 1).to_string(), 1).replacen("{}", &n_pages.to_string(), 1);
                page.items.push(self.text(&label, Rect::new(DESIGN_WIDTH - x0 - 500.0, 95.0, 500.0, 70.0), 30, "", Align::Right, true));
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
        let mut pages: Vec<Menu> = (0..n_pages)
            .map(|i| self.menu(&if n_pages > 1 { format!("{heading} {}", i + 1) } else { heading.to_string() }))
            .collect();
        let ids: Vec<Id> = pages.iter().map(|p| p.id).collect();
        let (x0, w) = (220.0, 1480.0);
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.text(heading, Rect::new(x0 + 20.0, 70.0, 1000.0, 110.0), 64, "ExtraBold", Align::Left, false));
            if n_pages > 1 {
                let label = gettext("Page {} of {}").replacen("{}", &(pi + 1).to_string(), 1).replacen("{}", &n_pages.to_string(), 1);
                page.items.push(self.text(&label, Rect::new(x0 + w - 520.0, 95.0, 500.0, 70.0), 30, "", Align::Right, true));
            }
            page.items.push(self.panel(Rect::new(x0, 200.0, w, 730.0), 32.0));
            let slice = entries.iter().skip(pi * EPISODE_ROWS).take(EPISODE_ROWS);
            for (k, (label, action, duration)) in slice.enumerate() {
                let y = 222.0 + k as f64 * 98.0;
                page.items.push(self.text(&crate::ui::rows::format_time(*duration), Rect::new(x0 + w - 300.0, y, 260.0, 86.0), 30, "", Align::Right, true));
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
        m.items.push(self.text(&gettext("Audio & Subtitles"), Rect::new(480.0, top + 30.0, 960.0, 110.0), 56, "ExtraBold", Align::Center, false));
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

    /// Paginated vertical list of text buttons (with optional leading items
    /// on the first page).
    fn list_pages(&self, heading: &str, page_name: &str, entries: &[(String, Action)], home: Option<(Id, String)>) -> Vec<Menu> {
        let n_pages = entries.len().div_ceil(LIST_PAGE).max(1);
        let mut pages: Vec<Menu> = (0..n_pages)
            .map(|i| self.menu(&if n_pages > 1 { format!("{page_name} {}", i + 1) } else { page_name.to_string() }))
            .collect();
        let ids: Vec<Id> = pages.iter().map(|p| p.id).collect();
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.panel(Rect::new(430.0, 60.0, 1060.0, 870.0), 36.0));
            page.items.push(self.text(heading, Rect::new(480.0, 90.0, 960.0, 120.0), 60, "ExtraBold", Align::Center, false));
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
    let cx = Ctx { theme: THEMES[opts.theme.min(THEMES.len() - 1)] };
    let titles: Vec<(Id, Id, String)> = p.titles.iter().map(|t| (t.id, t.asset, t.name.clone())).collect();
    let name = if opts.title.trim().is_empty() { p.disc.name.clone() } else { opts.title.clone() };

    let mut main = cx.menu(&gettext("Main Menu"));
    let main_id = main.id;
    let back = Some((main_id, gettext("Main Menu")));
    let mut menus = Vec::new();
    let mut return_to: Vec<(Id, Id)> = Vec::new(); // (title, menu)

    // Setup menu for choosing audio and subtitles, when there is a choice.
    if p.disc.languages.is_empty() {
        p.disc.languages = p.suggest_languages();
    }
    let setup = (p.disc.languages.len() >= 2).then(|| cx.setup_page(&p.disc.languages, p.default_language().map(|l| l.id), main_id));
    let setup_id = setup.as_ref().map(|m| m.id);

    match opts.layout {
        Layout::Show | Layout::ShowList => {
            let hint = gettext("Import your episodes, then apply the template again");
            let label_for = |i: usize, tname: &str, sep: &str, max: usize| {
                let name = episode_name(tname);
                if name.is_empty() {
                    gettext("Episode {}").replace("{}", &(i + 1).to_string())
                } else {
                    format!("{}{sep}{}", i + 1, short(&name, max))
                }
            };
            let (pages, per_page) = if opts.layout == Layout::Show {
                let entries: Vec<(String, Action, Id, f64)> = titles
                    .iter()
                    .enumerate()
                    .map(|(i, (tid, asset, tname))| {
                        (label_for(i, tname, " · ", 28), Action::PlayTitle { title: *tid, chapter: 0 }, *asset, p.title_poster(*tid))
                    })
                    .collect();
                (cx.grid_pages(&gettext("Episodes"), &entries, back.clone(), &hint), GRID_PAGE)
            } else {
                let entries: Vec<(String, Action, f64)> = titles
                    .iter()
                    .enumerate()
                    .map(|(i, (tid, asset, tname))| {
                        let duration = p.asset(*asset).map_or(0.0, |a| a.info.duration);
                        (label_for(i, tname, ".   ", 44), Action::PlayTitle { title: *tid, chapter: 0 }, duration)
                    })
                    .collect();
                (cx.episode_list_pages(&gettext("Episodes"), &entries, back.clone(), &hint), EPISODE_ROWS)
            };
            for (i, (tid, _, _)) in titles.iter().enumerate() {
                return_to.push((*tid, pages[i / per_page].id));
            }

            main.items.push(cx.panel(Rect::new(96.0, 96.0, 700.0, 888.0), 36.0));
            main.items.push(cx.text(&name, Rect::new(150.0, 170.0, 600.0, 280.0), 80, "ExtraBold", Align::Left, false));
            let count = titles.len() as u32;
            let episodes = gettextrs::ngettext("{} Episode", "{} Episodes", count).replace("{}", &count.to_string());
            main.items.push(MenuItem::new_shape(cx.theme.selected, 3.0, Rect::new(150.0, 455.0, 90.0, 6.0)));
            main.items.push(cx.text(&episodes, Rect::new(150.0, 475.0, 600.0, 60.0), 32, "", Align::Left, true));
            // Two or three buttons down the panel.
            let (y, step) = if setup_id.is_some() { (566.0, 128.0) } else { (590.0, 146.0) };
            let play_all = cx.button(&gettext("Play All"), Action::PlayAll, Rect::new(130.0, y, 560.0, 96.0), 50, Align::Left, Highlight::Frame);
            main.default_button = Some(play_all.id);
            main.items.push(play_all);
            main.items.push(cx.button(&gettext("Episodes"), Action::ShowMenu(pages[0].id), Rect::new(130.0, y + step, 560.0, 96.0), 50, Align::Left, Highlight::Frame));
            if let Some(setup) = setup_id {
                main.items.push(cx.button(&gettext("Setup"), Action::ShowMenu(setup), Rect::new(130.0, y + 2.0 * step, 560.0, 96.0), 50, Align::Left, Highlight::Frame));
            }
            if let Some((tid, asset, _)) = titles.first() {
                main.items.push(cx.panel(Rect::new(862.0, 262.0, 976.0, 556.0), 24.0));
                let mut hero = MenuItem::new_image(*asset, Rect::new(870.0, 270.0, 960.0, 540.0));
                if let ItemKind::Image(img) = &mut hero.kind {
                    img.time = p.title_poster(*tid);
                }
                main.items.push(hero);
            }
            menus.push(main);
            menus.extend(pages);
        }
        Layout::Movie => {
            let film = titles.first().cloned();
            if let Some((tid, asset, _)) = &film {
                let mut hero = MenuItem::new_image(*asset, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT));
                if let ItemKind::Image(img) = &mut hero.kind {
                    img.time = p.title_poster(*tid);
                }
                main.items.push(hero);
            }
            main.items.push(MenuItem::new_shape(alpha(cx.theme.bottom, 0.8), 0.0, Rect::new(0.0, 700.0, DESIGN_WIDTH, 380.0)));
            main.items.push(cx.text(&name, Rect::new(120.0, 720.0, 1680.0, 140.0), 80, "ExtraBold", Align::Left, false));
            let mut x = 110.0;
            let mut add = |m: &mut Menu, label: String, action: Action, w: f64| {
                let b = cx.button(&label, action, Rect::new(x, 895.0, w, 90.0), 42, Align::Center, Highlight::Frame);
                x += w + 30.0;
                let id = b.id;
                m.items.push(b);
                id
            };
            if let Some((tid, asset, _)) = &film {
                let play = add(&mut main, gettext("Play Movie"), Action::PlayTitle { title: *tid, chapter: 0 }, 400.0);
                main.default_button = Some(play);
                return_to.push((*tid, main_id));
                let chapters: Vec<f64> =
                    std::iter::once(0.0).chain(p.title(*tid).map(|t| t.chapters.clone()).unwrap_or_default()).collect();
                let entries: Vec<(String, Action, Id, f64)> = chapters
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| {
                        let label = format!("{} {} · {}", gettext("Chapter"), i + 1, crate::ui::rows::format_time(c));
                        (label, Action::PlayTitle { title: *tid, chapter: i as u32 }, *asset, c + 3.0)
                    })
                    .collect();
                let scenes = cx.grid_pages(&gettext("Scene Selection"), &entries, back.clone(), "");
                add(&mut main, gettext("Scene Selection"), Action::ShowMenu(scenes[0].id), 480.0);
                if titles.len() > 1 {
                    let extras: Vec<(String, Action)> = titles[1..]
                        .iter()
                        .map(|(t, _, n)| (short(n, 36), Action::PlayTitle { title: *t, chapter: 0 }))
                        .collect();
                    let pages = cx.list_pages(&gettext("Extras"), &gettext("Extras"), &extras, back.clone());
                    for (i, (t, _, _)) in titles[1..].iter().enumerate() {
                        return_to.push((*t, pages[i / LIST_PAGE].id));
                    }
                    add(&mut main, gettext("Extras"), Action::ShowMenu(pages[0].id), 300.0);
                    if let Some(setup) = setup_id {
                        add(&mut main, gettext("Setup"), Action::ShowMenu(setup), 260.0);
                    }
                    menus.push(main);
                    menus.extend(scenes);
                    menus.extend(pages);
                } else {
                    if let Some(setup) = setup_id {
                        add(&mut main, gettext("Setup"), Action::ShowMenu(setup), 260.0);
                    }
                    menus.push(main);
                    menus.extend(scenes);
                }
            } else {
                main.items.push(cx.text(&gettext("Import your movie, then apply the template again"), Rect::new(120.0, 880.0, 1680.0, 80.0), 32, "", Align::Left, true));
                menus.push(main);
            }
        }
        Layout::List => {
            let mut entries: Vec<(String, Action)> = Vec::new();
            if titles.len() > 1 {
                entries.push((gettext("Play All"), Action::PlayAll));
            }
            entries.extend(titles.iter().map(|(t, _, n)| (short(n, 36), Action::PlayTitle { title: *t, chapter: 0 })));
            if let Some(setup) = setup_id {
                entries.push((gettext("Setup"), Action::ShowMenu(setup)));
            }
            let mut pages = cx.list_pages(&name, &gettext("More Titles"), &entries, None);
            // The first page is the main menu.
            let first = pages.remove(0);
            main.items = first.items;
            let first_button = main.buttons().next().map(|b| b.id);
            main.default_button = first_button;
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
            let offset = usize::from(titles.len() > 1);
            for (i, (t, _, _)) in titles.iter().enumerate() {
                let page = (i + offset) / LIST_PAGE;
                return_to.push((*t, if page == 0 { main_id } else { pages[page - 1].id }));
            }
            menus.push(main);
            menus.extend(pages);
        }
    }

    // Use the logo in place of the name on the main menu.
    if let Some(logo) = opts.logo.and_then(|id| p.asset(id)) {
        let aspect = if logo.info.height > 0 { logo.info.width as f64 / logo.info.height as f64 } else { 1.0 };
        let logo_id = logo.id;
        if let Some(main) = menus.first_mut() {
            if let Some(item) = main.items.iter_mut().find(|i| matches!(&i.kind, ItemKind::Text(t) if t.text == name)) {
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
    let mut links = Vec::new();
    match opts.layout {
        Layout::Show | Layout::ShowList => {
            if let Some(episodes) = menus.get(1).filter(|_| !titles.is_empty()) {
                links.push((gettext("Episodes"), Action::ShowMenu(episodes.id)));
            }
        }
        Layout::Movie => {
            if let Some(scenes) = menus.iter().find(|m| m.name.starts_with(&gettext("Scene Selection"))) {
                links.push((gettext("Scene Selection"), Action::ShowMenu(scenes.id)));
            }
        }
        Layout::List => {}
    }
    links.push((gettext("Top Menu"), Action::ShowMenu(main_id)));
    let popups = cx.popup_menus(&name, max_chapters, &p.disc.languages, &links);
    p.disc.popup_menu = popups.first().map(|m| m.id);
    menus.extend(popups);
    p.menus = menus;
    for t in &mut p.titles {
        t.return_menu = return_to.iter().find(|(id, _)| *id == t.id).map(|(_, m)| *m);
    }
    p.first_play = FirstPlay::FirstMenu;
    main_id
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
        let warnings: Vec<_> = crate::validate::check(p).into_iter().filter(|i| i.severity == crate::validate::Severity::Warning).collect();
        assert!(warnings.is_empty(), "{warnings:#?}");
    }

    #[test]
    fn setup_menu_for_two_languages() {
        for layout in [Layout::Show, Layout::ShowList, Layout::Movie, Layout::List] {
            let mut p = project(3);
            for t in &mut p.titles {
                t.audio = ["eng", "jpn"]
                    .iter()
                    .map(|l| AudioTrack { id: new_id(), source: AudioSource::Embedded { index: 0 }, lang: l.to_string(), name: l.to_string(), enabled: true })
                    .collect();
            }
            let main = apply(&mut p, &Options { layout, theme: 1, title: "Show".into(), logo: None });
            check(&p);
            assert_eq!(p.disc.languages.len(), 2);
            let setup = p.menus.iter().find(|m| m.name == "Setup").expect("setup menu");
            let choices: Vec<Action> = setup.buttons().map(|b| b.button().unwrap().action).filter(|a| matches!(a, Action::SetLanguage { .. })).collect();
            assert_eq!(choices.len(), 2);
            assert!(choices.iter().all(|a| matches!(a, Action::SetLanguage { menu: Some(m), .. } if *m == main)));
            let reaches_setup = p.menus.iter().any(|m| m.buttons().any(|b| b.button().unwrap().action == Action::ShowMenu(setup.id)));
            assert!(reaches_setup, "{layout:?}");
        }
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
        let main = apply(&mut p, &Options { layout: Layout::Show, theme: 0, title: "My Show".into(), logo: None });
        check(&p);
        // main + 2 episode pages
        assert_eq!(p.disc_menus().count(), 3);
        let m = p.menu(main).unwrap();
        assert!(m.buttons().any(|b| b.button().unwrap().action == Action::PlayAll));
        let episode_buttons: usize = p.disc_menus().skip(1)
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
        for layout in Layout::ALL {
            let main = apply(&mut p, &Options { layout, theme: 0, title: "My Show".into(), logo: Some(logo) });
            let m = p.menu(main).unwrap();
            let img = m.items.iter().find(|i| matches!(&i.kind, ItemKind::Image(img) if img.asset == logo));
            let img = img.unwrap_or_else(|| panic!("{layout:?} has no logo"));
            assert!((img.rect.w / img.rect.h - 3.0).abs() < 0.05, "logo keeps its 3:1 shape");
            assert!(!m.items.iter().any(|i| matches!(&i.kind, ItemKind::Text(t) if t.text == "My Show")));
        }
    }

    #[test]
    fn show_list_template_pages_episodes() {
        let mut p = project(9);
        apply(&mut p, &Options { layout: Layout::ShowList, theme: 2, title: "My Show".into(), logo: None });
        check(&p);
        // main + 2 list pages (7 + 2)
        assert_eq!(p.disc_menus().count(), 3);
        assert!(p.disc_menus().skip(1).all(|m| m.buttons().all(|b| b.button().unwrap().thumbnail.is_none())));
        assert_eq!(p.titles[8].return_menu, Some(p.menus[2].id));
    }

    #[test]
    fn other_layouts_are_consistent() {
        for layout in [Layout::Movie, Layout::List] {
            for n in [0, 1, 3, 12] {
                let mut p = project(n);
                apply(&mut p, &Options { layout, theme: 4, title: String::new(), logo: None });
                if n > 0 {
                    check(&p);
                }
            }
        }
    }
}
