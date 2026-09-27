// SPDX-License-Identifier: GPL-3.0-or-later

//! TV show styles, and the page layouts they share with movies.

use super::*;

/// (label, action, thumbnail asset, frame time).
pub(super) type Card = (String, Action, Id, f64);
/// (label, action, text on the right such as a running time).
pub(super) type Row = (String, Action, String);
/// (label, action, still asset, frame time, text on the right).
pub(super) type ListEntry = (String, Action, Id, f64, String);

/// Rows per page of [`Ctx::artwork_list_pages`].
const ARTWORK_LIST_ROWS: usize = 8;

pub(super) fn build(style: Style, input: Input) -> Output {
    match style {
        Style::Classic => classic(input, false),
        Style::ClassicList => classic(input, true),
        Style::Showcase => showcase(input, false),
        Style::ShowcaseList => showcase(input, true),
        Style::Minimal => minimal(input),
        Style::Streaming => streaming(input),
        Style::Split => split(input),
        _ => broadcast(input),
    }
}

/// Where each episode returns: the page listing it.
fn returns(episodes: &[Episode], pages: &[Menu], per_page: usize) -> Vec<(Id, Id)> {
    episodes.iter().enumerate().map(|(i, e)| (e.title, pages[(i / per_page).min(pages.len() - 1)].id)).collect()
}

fn minutes(secs: f64) -> String {
    gettext("{} min").replace("{}", &((secs / 60.0).round().max(1.0) as u32).to_string())
}

impl Ctx {
    /// Large thumbnail cards, 4 × 2 per page, over a darkened still of the
    /// page's first entry.
    pub(super) fn card_pages(&self, heading: &str, edition: Option<&str>, entries: &[Card], home: Option<(Id, String)>, empty_hint: &str) -> Vec<Menu> {
        const PER_ROW: usize = 4;
        const PER_PAGE: usize = 8;
        let bg = self.theme.bottom;
        let n_pages = entries.len().div_ceil(PER_PAGE).max(1);
        let (mut pages, ids) = self.pages(heading, n_pages, |n| self.plain_menu(n, bg));
        let (cw, th, gap_x, gap_y) = (380.0, 214.0, 36.0, 40.0);
        let ch = th + 64.0;
        let x0 = (DESIGN_WIDTH - (PER_ROW as f64 * cw + 3.0 * gap_x)) / 2.0;
        let y0 = 212.0;
        for (pi, page) in pages.iter_mut().enumerate() {
            let slice: Vec<&Card> = entries.iter().skip(pi * PER_PAGE).take(PER_PAGE).collect();
            if let Some((_, _, asset, time)) = slice.first() {
                page.items.push(self.still(*asset, *time, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT)));
                page.items.push(MenuItem::new_shape(alpha(bg, 0.86), 0.0, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT)));
            }
            page.items.push(self.head(heading, Rect::new(x0, 80.0, 900.0, 90.0), 60, "ExtraBold", Align::Left));
            let right = match (edition, n_pages > 1) {
                (Some(e), true) => Some(format!("{e}  ·  {}", self.page_label(pi, n_pages))),
                (Some(e), false) => Some(e.to_string()),
                (None, true) => Some(self.page_label(pi, n_pages)),
                (None, false) => None,
            };
            if let Some(r) = right {
                page.items.push(self.caps(&r, Rect::new(DESIGN_WIDTH - x0 - 800.0, 104.0, 800.0, 44.0), 24, self.theme.selected, Align::Right, 4.0));
            }
            for (k, (label, action, asset, time)) in slice.iter().enumerate() {
                let (r, c) = (k / PER_ROW, k % PER_ROW);
                let rect = Rect::new(x0 + c as f64 * (cw + gap_x), y0 + r as f64 * (ch + gap_y), cw, ch);
                let mut b = self.button_font(label, *action, rect, format!("{} SemiBold 26", self.body), Align::Left, Highlight::Frame);
                if let Some(b) = b.button_mut() {
                    b.thumbnail = Some(*asset);
                    b.thumbnail_time = *time;
                }
                page.items.push(b);
            }
            if entries.is_empty() {
                page.items.push(self.text(empty_hint, Rect::new(260.0, 420.0, 1400.0, 200.0), 36, "", Align::Center, true));
            }
            let prev = pi.checked_sub(1).map(|i| ids[i]);
            self.nav_bar(page, prev, home.clone(), ids.get(pi + 1).copied());
        }
        pages
    }

    /// A list of rows over the artwork of the page's first entry, which
    /// shows through on the right.
    pub(super) fn artwork_list_pages(&self, heading: &str, edition: Option<&str>, entries: &[ListEntry], home: Option<(Id, String)>, empty_hint: &str) -> Vec<Menu> {
        let bg = self.theme.bottom;
        let n_pages = entries.len().div_ceil(ARTWORK_LIST_ROWS).max(1);
        let (mut pages, ids) = self.pages(heading, n_pages, |n| self.plain_menu(n, bg));
        let (x0, w, note_w) = (140.0, 900.0, 170.0);
        for (pi, page) in pages.iter_mut().enumerate() {
            let slice: Vec<&ListEntry> = entries.iter().skip(pi * ARTWORK_LIST_ROWS).take(ARTWORK_LIST_ROWS).collect();
            if let Some((_, _, asset, time, _)) = slice.first() {
                page.items.push(self.still(*asset, *time, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT)));
                page.items.push(self.gradient(Rect::new(0.0, 0.0, 1500.0, DESIGN_HEIGHT), alpha(bg, 0.96), alpha(bg, 0.25), true));
                page.items.push(self.gradient(Rect::new(0.0, 700.0, DESIGN_WIDTH, 380.0), alpha(bg, 0.0), alpha(bg, 0.9), false));
            }
            page.items.push(self.head(heading, Rect::new(x0, 80.0, 900.0, 90.0), 60, "ExtraBold", Align::Left));
            let mut sub: Vec<String> = edition.map(str::to_string).into_iter().collect();
            if n_pages > 1 {
                sub.push(self.page_label(pi, n_pages));
            }
            if !sub.is_empty() {
                page.items.push(self.caps(&sub.join("  ·  "), Rect::new(x0, 170.0, 1000.0, 40.0), 22, self.theme.selected, Align::Left, 4.0));
            }
            for (k, (label, action, _, _, note)) in slice.iter().enumerate() {
                let y = 236.0 + k as f64 * 80.0;
                // The running time sits beside the button, where the
                // highlight doesn't cover it.
                page.items.push(self.text(note, Rect::new(x0 + w + 16.0, y, note_w, 68.0), 26, "Medium", Align::Right, true));
                // The label's inset lines it up with the heading.
                page.items.push(self.button_font(label, *action, Rect::new(x0 - 24.0, y, w + 24.0, 68.0), format!("{} SemiBold 30", self.body), Align::Left, Highlight::Fill));
            }
            if entries.is_empty() {
                page.items.push(self.text(empty_hint, Rect::new(260.0, 480.0, 1400.0, 120.0), 36, "", Align::Center, true));
            }
            let prev = pi.checked_sub(1).map(|i| ids[i]);
            self.nav_bar(page, prev, home.clone(), ids.get(pi + 1).copied());
        }
        pages
    }

    /// Two columns of text rows (7 each) with a note on the right of each,
    /// on a plain background.
    pub(super) fn column_pages(&self, heading: &str, edition: Option<&str>, entries: &[Row], home: Option<(Id, String)>, empty_hint: &str) -> Vec<Menu> {
        const ROWS: usize = 7;
        const PER_PAGE: usize = 2 * ROWS;
        let n_pages = entries.len().div_ceil(PER_PAGE).max(1);
        let (mut pages, ids) = self.pages(heading, n_pages, |n| self.plain_menu(n, self.theme.bottom));
        let (xs, w) = ([170.0, 990.0], 760.0);
        for (pi, page) in pages.iter_mut().enumerate() {
            page.items.push(self.styled(heading, Rect::new(160.0, 76.0, 1600.0, 90.0), format!("{} Light 56", self.heading), self.theme.text, Align::Center, false));
            let mut sub: Vec<String> = edition.map(str::to_string).into_iter().collect();
            if n_pages > 1 {
                sub.push(self.page_label(pi, n_pages));
            }
            if !sub.is_empty() {
                page.items.push(self.caps(&sub.join("  ·  "), Rect::new(160.0, 170.0, 1600.0, 36.0), 20, self.theme.dim, Align::Center, 6.0));
            }
            let slice = entries.iter().skip(pi * PER_PAGE).take(PER_PAGE);
            for (k, (label, action, note)) in slice.enumerate() {
                let (x, y) = (xs[k / ROWS], 240.0 + (k % ROWS) as f64 * 88.0);
                page.items.push(self.text(note, Rect::new(x + w - 170.0, y, 160.0, 76.0), 26, "Regular", Align::Right, true));
                let mut b = self.button_font(label, *action, Rect::new(x, y, w, 76.0), format!("{} Regular 32", self.body), Align::Left, Highlight::Arrow);
                if let Some(b) = b.button_mut() {
                    b.text.shadow = false;
                }
                page.items.push(b);
            }
            if entries.is_empty() {
                page.items.push(self.text(empty_hint, Rect::new(260.0, 480.0, 1400.0, 120.0), 36, "", Align::Center, true));
            }
            let prev = pi.checked_sub(1).map(|i| ids[i]);
            self.nav_bar(page, prev, home.clone(), ids.get(pi + 1).copied());
        }
        pages
    }

    /// A row of pill buttons from `x`; returns their ids.
    pub(super) fn pill_row(&self, m: &mut Menu, entries: &[(String, Action)], x: f64, y: f64, size: u32) -> Vec<Id> {
        let mut x = x;
        let mut ids = Vec::new();
        for (label, action) in entries {
            let w = (64.0 + label.chars().count() as f64 * size as f64 * 0.62).max(220.0);
            let mut b = self.button_font(label, *action, Rect::new(x, y, w, 84.0), format!("{} SemiBold {size}", self.body), Align::Center, Highlight::Fill);
            if let Some(b) = b.button_mut() {
                b.fill = Some(alpha(self.theme.text, 0.12));
            }
            ids.push(b.id);
            m.items.push(b);
            x += w + 24.0;
        }
        ids
    }
}

fn classic(input: Input, list: bool) -> Output {
    let (edition, count, home, hint) = (input.edition(), input.count(), input.home(), input.hint());
    let setup = input.setup;
    let Input { cx, name, mut main, episodes, .. } = input;
    let (pages, per_page) = if list {
        let entries: Vec<(String, Action, f64)> = episodes.iter().map(|e| (e.label(".   ", 44), e.action(), e.duration)).collect();
        (cx.episode_list_pages(&gettext("Episodes"), &entries, home, &hint), EPISODE_ROWS)
    } else {
        let entries: Vec<Card> = episodes.iter().map(|e| (e.label(" · ", 28), e.action(), e.asset, e.poster)).collect();
        (cx.grid_pages(&gettext("Episodes"), &entries, home, &hint), GRID_PAGE)
    };
    main.items.push(cx.panel(Rect::new(96.0, 96.0, 700.0, 888.0), 36.0));
    if let Some(e) = &edition {
        main.items.push(cx.caps(e, Rect::new(150.0, 128.0, 600.0, 40.0), 22, cx.theme.selected, Align::Left, 4.0));
    }
    let title = cx.head(&name, Rect::new(150.0, 170.0, 600.0, 280.0), 80, "ExtraBold", Align::Left);
    let title_item = Some(title.id);
    main.items.push(title);
    main.items.push(MenuItem::new_shape(cx.theme.selected, 3.0, Rect::new(150.0, 455.0, 90.0, 6.0)));
    main.items.push(cx.text(&count, Rect::new(150.0, 475.0, 600.0, 60.0), 32, "", Align::Left, true));
    // Two or three buttons down the panel.
    let buttons = show_buttons(&pages, &episodes, setup);
    let (y, step) = if buttons.len() > 2 { (566.0, 128.0) } else { (590.0, 146.0) };
    for (i, (label, action)) in buttons.iter().enumerate() {
        let b = cx.button(label, *action, Rect::new(130.0, y + i as f64 * step, 560.0, 96.0), 50, Align::Left, Highlight::Frame);
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    if let Some(e) = episodes.first() {
        main.items.push(cx.panel(Rect::new(862.0, 262.0, 976.0, 556.0), 24.0));
        main.items.push(cx.still(e.asset, e.poster, Rect::new(870.0, 270.0, 960.0, 540.0)));
    }
    finish(main, pages, &episodes, per_page, title_item)
}

/// Main menu, episode pages and the links between them.
fn finish(main: Menu, pages: Vec<Menu>, episodes: &[Episode], per_page: usize, title_item: Option<Id>) -> Output {
    let return_to = returns(episodes, &pages, per_page);
    let popup_links = match (pages.first(), episodes.is_empty()) {
        (Some(first), false) => vec![(gettext("Episodes"), Action::ShowMenu(first.id))],
        _ => vec![],
    };
    let mut menus = vec![main];
    menus.extend(pages);
    Output { menus, return_to, title_item, popup_links }
}

fn showcase(input: Input, list: bool) -> Output {
    let (edition, count, home, hint) = (input.edition(), input.count(), input.home(), input.hint());
    let setup = input.setup;
    let Input { cx, name, mut main, episodes, .. } = input;
    let bg = cx.theme.bottom;
    let (pages, per_page) = if list {
        let rows: Vec<ListEntry> = episodes.iter().map(|e| (e.label(".   ", 40), e.action(), e.asset, e.poster, clock(e.duration))).collect();
        (cx.artwork_list_pages(&gettext("Episodes"), edition.as_deref(), &rows, home, &hint), ARTWORK_LIST_ROWS)
    } else {
        let cards: Vec<Card> = episodes.iter().map(|e| (e.label(".  ", 24), e.action(), e.asset, e.poster)).collect();
        (cx.card_pages(&gettext("Episodes"), edition.as_deref(), &cards, home, &hint), 8)
    };

    main.background = Background { color: bg, ..Default::default() };
    if let Some(e) = episodes.first() {
        main.items.push(cx.still(e.asset, e.poster, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT)));
        main.items.push(cx.gradient(Rect::new(0.0, 0.0, 1340.0, DESIGN_HEIGHT), alpha(bg, 0.94), alpha(bg, 0.0), true));
        main.items.push(cx.gradient(Rect::new(0.0, 560.0, DESIGN_WIDTH, 520.0), alpha(bg, 0.0), alpha(bg, 0.96), false));
    }
    if let Some(e) = &edition {
        main.items.push(cx.caps(e, Rect::new(140.0, 400.0, 1100.0, 44.0), 28, cx.theme.selected, Align::Left, 6.0));
    }
    let mut title = cx.head(&name, Rect::new(140.0, 450.0, 1150.0, 250.0), title_size(&name, 104, 80, 18), "ExtraBold", Align::Left);
    if let ItemKind::Text(t) = &mut title.kind {
        t.style.line_spacing = 0.9;
    }
    let title_item = Some(title.id);
    main.items.push(title);
    main.items.push(cx.text(&count, Rect::new(140.0, 705.0, 800.0, 44.0), 28, "Medium", Align::Left, true));
    let buttons = show_buttons(&pages, &episodes, setup);
    let ids = cx.pill_row(&mut main, &buttons, 140.0, 800.0, 32);
    main.default_button = ids.first().copied();
    finish(main, pages, &episodes, per_page, title_item)
}

/// Play All, Episodes and Setup.
fn show_buttons(pages: &[Menu], episodes: &[Episode], setup: Option<Id>) -> Vec<(String, Action)> {
    let mut v = vec![(gettext("Play All"), Action::PlayAll)];
    if let (Some(first), false) = (pages.first(), episodes.is_empty()) {
        v.push((gettext("Episodes"), Action::ShowMenu(first.id)));
    }
    if let Some(s) = setup {
        v.push((gettext("Setup"), Action::ShowMenu(s)));
    }
    v
}

fn minimal(input: Input) -> Output {
    let (edition, count, home, hint) = (input.edition(), input.count(), input.home(), input.hint());
    let setup = input.setup;
    let Input { cx, name, mut main, episodes, .. } = input;
    let rows: Vec<Row> = episodes
        .iter()
        .map(|e| {
            let label = if e.name.is_empty() { gettext("Episode {}").replace("{}", &e.number.to_string()) } else { format!("{:02}   {}", e.number, short(&e.name, 30)) };
            (label, e.action(), minutes(e.duration))
        })
        .collect();
    let pages = cx.column_pages(&gettext("Episodes"), edition.as_deref(), &rows, home, &hint);

    main.background = Background { color: cx.theme.bottom, ..Default::default() };
    let caps = match &edition {
        Some(e) => format!("{e}  ·  {count}"),
        None => count,
    };
    main.items.push(cx.caps(&caps, Rect::new(160.0, 300.0, 1600.0, 40.0), 24, cx.theme.dim, Align::Center, 10.0));
    let mut title = cx.styled(&name, Rect::new(160.0, 350.0, 1600.0, 200.0), format!("{} Light 110", cx.heading), cx.theme.text, Align::Center, false);
    if let ItemKind::Text(t) = &mut title.kind {
        t.style.letter_spacing = 3.0;
        t.style.line_spacing = 0.9;
    }
    let title_item = Some(title.id);
    main.items.push(title);
    main.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(910.0, 575.0, 100.0, 3.0)));
    for (i, (label, action)) in show_buttons(&pages, &episodes, setup).iter().enumerate() {
        let mut b = cx.button_font(label, *action, Rect::new(710.0, 630.0 + i as f64 * 92.0, 500.0, 76.0), format!("{} Medium 34", cx.body), Align::Center, Highlight::Underline);
        if let Some(b) = b.button_mut() {
            b.text.shadow = false;
        }
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    finish(main, pages, &episodes, 14, title_item)
}

fn streaming(input: Input) -> Output {
    let (edition, count, home, hint) = (input.edition(), input.count(), input.home(), input.hint());
    let setup = input.setup;
    let Input { cx, name, mut main, episodes, .. } = input;
    let bg = cx.theme.bottom;
    const PER_PAGE: usize = 4;

    // Episode rows: a still, then the name and running time.
    let n_pages = episodes.len().div_ceil(PER_PAGE).max(1);
    let (mut pages, ids) = cx.pages(&gettext("Episodes"), n_pages, |n| cx.plain_menu(n, bg));
    for (pi, page) in pages.iter_mut().enumerate() {
        page.items.push(cx.head(&gettext("Episodes"), Rect::new(140.0, 70.0, 900.0, 80.0), 56, "Black", Align::Left));
        let mut right: Vec<String> = edition.iter().cloned().collect();
        if n_pages > 1 {
            right.push(cx.page_label(pi, n_pages));
        }
        if !right.is_empty() {
            page.items.push(cx.caps(&right.join("  ·  "), Rect::new(1000.0, 92.0, 780.0, 40.0), 22, cx.theme.selected, Align::Right, 4.0));
        }
        for (k, e) in episodes.iter().skip(pi * PER_PAGE).take(PER_PAGE).enumerate() {
            let y = 190.0 + k as f64 * 180.0;
            page.items.push(cx.still(e.asset, e.poster, Rect::new(140.0, y, 288.0, 162.0)));
            page.items.push(cx.text(&minutes(e.duration), Rect::new(1480.0, y, 280.0, 162.0), 28, "Regular", Align::Right, true));
            // A frame, not a fill, so the running time stays readable.
            let mut b = cx.button_font(&e.label("   ", 40), e.action(), Rect::new(452.0, y, 1328.0, 162.0), format!("{} Bold 34", cx.body), Align::Left, Highlight::Frame);
            if let Some(b) = b.button_mut() {
                b.fill = Some(alpha(cx.theme.text, 0.06));
            }
            page.items.push(b);
        }
        if episodes.is_empty() {
            page.items.push(cx.text(&hint, Rect::new(260.0, 420.0, 1400.0, 200.0), 36, "", Align::Center, true));
        }
        let prev = pi.checked_sub(1).map(|i| ids[i]);
        cx.nav_bar(page, prev, home.clone(), ids.get(pi + 1).copied());
    }

    main.background = Background { color: bg, ..Default::default() };
    if let Some(e) = episodes.first() {
        main.items.push(cx.still(e.asset, e.poster, Rect::new(576.0, 0.0, 1344.0, 756.0)));
        main.items.push(cx.gradient(Rect::new(576.0, 0.0, 720.0, 756.0), bg, alpha(bg, 0.0), true));
        main.items.push(cx.gradient(Rect::new(576.0, 456.0, 1344.0, 300.0), alpha(bg, 0.0), bg, false));
    }
    if let Some(e) = &edition {
        main.items.push(cx.caps(e, Rect::new(140.0, 230.0, 800.0, 40.0), 26, cx.theme.selected, Align::Left, 4.0));
    }
    let title = cx.head(&name, Rect::new(140.0, 280.0, 860.0, 250.0), 96, "Black", Align::Left);
    let title_item = Some(title.id);
    main.items.push(title);
    main.items.push(cx.text(&count, Rect::new(140.0, 540.0, 800.0, 40.0), 28, "Regular", Align::Left, true));
    let mut buttons = show_buttons(&pages, &episodes, setup);
    buttons[0].0 = format!("▶  {}", buttons[0].0);
    for (i, (label, action)) in buttons.iter().enumerate() {
        let mut b = cx.button_font(label, *action, Rect::new(120.0, 620.0 + i as f64 * 100.0, 440.0, 84.0), format!("{} Bold 32", cx.body), Align::Left, Highlight::Fill);
        if let Some(b) = b.button_mut() {
            b.fill = Some(alpha(cx.theme.text, 0.12));
        }
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    finish(main, pages, &episodes, PER_PAGE, title_item)
}

/// Stills of `episodes` filling the right of the screen: a 2 × 2 mosaic
/// when there are four or more, else the first one.
fn mosaic(cx: &Ctx, m: &mut Menu, episodes: &[&Episode]) {
    let area = Rect::new(760.0, 0.0, DESIGN_WIDTH - 760.0, DESIGN_HEIGHT);
    if episodes.len() >= 4 {
        let (w, h) = ((area.w - 6.0) / 2.0, (area.h - 6.0) / 2.0);
        for (k, e) in episodes.iter().take(4).enumerate() {
            let (r, c) = (k / 2, k % 2);
            m.items.push(cx.still(e.asset, e.poster, Rect::new(area.x + c as f64 * (w + 6.0), r as f64 * (h + 6.0), w, h)));
        }
    } else if let Some(e) = episodes.first() {
        m.items.push(cx.still(e.asset, e.poster, area));
    }
    m.items.push(MenuItem::new_shape(alpha(cx.theme.bottom, 0.25), 0.0, area));
    m.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(760.0, 0.0, 6.0, DESIGN_HEIGHT)));
}

fn split(input: Input) -> Output {
    let (edition, count, home, hint) = (input.edition(), input.count(), input.home(), input.hint());
    let (season, disc, setup) = (input.season.trim().to_string(), input.disc.trim().to_string(), input.setup);
    let Input { cx, name, mut main, episodes, .. } = input;
    const PER_PAGE: usize = 7;
    let panel = |m: &mut Menu| {
        m.background = Background { color: cx.theme.bottom, ..Default::default() };
        m.items.push(cx.gradient(Rect::new(0.0, 0.0, 760.0, DESIGN_HEIGHT), cx.theme.top, cx.theme.bottom, false));
    };

    let n_pages = episodes.len().div_ceil(PER_PAGE).max(1);
    let (mut pages, ids) = cx.pages(&gettext("Episodes"), n_pages, |n| cx.plain_menu(n, cx.theme.bottom));
    for (pi, page) in pages.iter_mut().enumerate() {
        let slice: Vec<&Episode> = episodes.iter().skip(pi * PER_PAGE).take(PER_PAGE).collect();
        panel(page);
        mosaic(&cx, page, &slice);
        page.items.push(cx.head(&gettext("Episodes"), Rect::new(130.0, 90.0, 580.0, 80.0), 56, "Bold", Align::Left));
        let mut sub: Vec<String> = edition.iter().cloned().collect();
        if n_pages > 1 {
            sub.push(cx.page_label(pi, n_pages));
        }
        if !sub.is_empty() {
            page.items.push(cx.caps(&sub.join("  ·  "), Rect::new(130.0, 172.0, 600.0, 36.0), 20, cx.theme.dim, Align::Left, 3.0));
        }
        for (k, e) in slice.iter().enumerate() {
            let b = cx.button_font(&e.label(".  ", 24), e.action(), Rect::new(110.0, 240.0 + k as f64 * 86.0, 600.0, 74.0), format!("{} SemiBold 28", cx.body), Align::Left, Highlight::Arrow);
            page.items.push(b);
        }
        if episodes.is_empty() {
            page.items.push(cx.text(&hint, Rect::new(130.0, 400.0, 580.0, 200.0), 30, "", Align::Left, true));
        }
        let prev = pi.checked_sub(1).map(|i| ids[i]);
        cx.nav_row(page, prev, home.clone(), ids.get(pi + 1).copied(), (110.0, 944.0, 200.0, 205.0), 28);
    }

    panel(&mut main);
    let all: Vec<&Episode> = episodes.iter().collect();
    mosaic(&cx, &mut main, &all);
    // Season badge, then the disc.
    let mut x = 130.0;
    if !season.is_empty() {
        let w = 60.0 + season.chars().count() as f64 * 19.0;
        main.items.push(MenuItem::new_shape(cx.theme.selected, 28.0, Rect::new(x, 230.0, w, 56.0)));
        main.items.push(cx.caps(&season, Rect::new(x, 230.0, w, 56.0), 24, cx.theme.on_accent(), Align::Center, 3.0));
        x += w + 20.0;
    }
    if !disc.is_empty() {
        main.items.push(cx.caps(&disc, Rect::new(x, 230.0, 740.0 - x, 56.0), 24, cx.theme.dim, Align::Left, 3.0));
    }
    let mut title = cx.head(&name, Rect::new(130.0, 320.0, 580.0, 290.0), 78, "Bold", Align::Left);
    if let ItemKind::Text(t) = &mut title.kind {
        t.style.line_spacing = 0.95;
    }
    let title_item = Some(title.id);
    main.items.push(title);
    main.items.push(cx.text(&count, Rect::new(130.0, 620.0, 580.0, 40.0), 28, "Medium", Align::Left, true));
    for (i, (label, action)) in show_buttons(&pages, &episodes, setup).iter().enumerate() {
        let b = cx.button_font(label, *action, Rect::new(110.0, 690.0 + i as f64 * 96.0, 580.0, 80.0), format!("{} SemiBold 36", cx.body), Align::Left, Highlight::Arrow);
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    finish(main, pages, &episodes, PER_PAGE, title_item)
}

fn broadcast(input: Input) -> Output {
    let (edition, count, home, hint) = (input.edition(), input.count(), input.home(), input.hint());
    let setup = input.setup;
    let Input { cx, name, mut main, episodes, .. } = input;
    const PER_PAGE: usize = 6;
    let ink = cx.theme.on_accent();
    let block = alpha(cx.theme.text, 0.08);
    // Plain background with faint scan lines and a header bar.
    let frame = |m: &mut Menu, title: &str, right: Option<String>| -> Id {
        m.background = Background { color: cx.theme.bottom, ..Default::default() };
        for k in 0..18 {
            m.items.push(MenuItem::new_shape(alpha(cx.theme.text, 0.035), 0.0, Rect::new(0.0, k as f64 * 60.0 + 30.0, DESIGN_WIDTH, 2.0)));
        }
        m.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(0.0, 80.0, DESIGN_WIDTH, 180.0)));
        let t = cx.styled(&title.to_uppercase(), Rect::new(140.0, 100.0, 1100.0, 140.0), format!("{} Bold 96", cx.heading), ink, Align::Left, false);
        let id = t.id;
        m.items.push(t);
        if let Some(r) = right {
            m.items.push(cx.caps(&r, Rect::new(1000.0, 140.0, 780.0, 60.0), 34, ink, Align::Right, 4.0));
        }
        id
    };

    let n_pages = episodes.len().div_ceil(PER_PAGE).max(1);
    let (mut pages, ids) = cx.pages(&gettext("Episodes"), n_pages, |n| cx.plain_menu(n, cx.theme.bottom));
    for (pi, page) in pages.iter_mut().enumerate() {
        let mut right: Vec<String> = edition.iter().cloned().collect();
        if n_pages > 1 {
            right.push(cx.page_label(pi, n_pages));
        }
        frame(page, &gettext("Episode Guide"), (!right.is_empty()).then(|| right.join("  ·  ")));
        for (k, e) in episodes.iter().skip(pi * PER_PAGE).take(PER_PAGE).enumerate() {
            let y = 300.0 + k as f64 * 104.0;
            page.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(140.0, y, 190.0, 90.0)));
            page.items.push(cx.caps(&minutes(e.duration), Rect::new(140.0, y, 190.0, 90.0), 30, ink, Align::Center, 2.0));
            let label = e.label("   ", 40).to_uppercase();
            let mut b = cx.button_font(&label, e.action(), Rect::new(346.0, y, 1434.0, 90.0), format!("{} Bold 40", cx.body), Align::Left, Highlight::Fill);
            if let Some(b) = b.button_mut() {
                b.fill = Some(block);
            }
            page.items.push(b);
        }
        if episodes.is_empty() {
            page.items.push(cx.text(&hint, Rect::new(260.0, 480.0, 1400.0, 120.0), 36, "", Align::Center, true));
        }
        let prev = pi.checked_sub(1).map(|i| ids[i]);
        cx.nav_bar(page, prev, home.clone(), ids.get(pi + 1).copied());
    }

    let title_item = Some(frame(&mut main, &name, edition.clone()));
    main.items.push(cx.caps(&count, Rect::new(140.0, 316.0, 800.0, 44.0), 30, cx.theme.dim, Align::Left, 4.0));
    if let Some(e) = episodes.first() {
        main.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(1052.0, 382.0, 776.0, 444.0)));
        main.items.push(cx.still(e.asset, e.poster, Rect::new(1060.0, 390.0, 760.0, 428.0)));
    }
    // A ticker along the bottom listing the episodes.
    if !episodes.is_empty() {
        main.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(0.0, 890.0, DESIGN_WIDTH, 4.0)));
        main.items.push(MenuItem::new_shape(alpha(cx.theme.text, 0.06), 0.0, Rect::new(0.0, 894.0, DESIGN_WIDTH, 86.0)));
        let names: Vec<String> = episodes.iter().take(5).map(|e| short(&e.name_or_title(), 18)).collect();
        let more = if episodes.len() > 5 { "  ·  …" } else { "" };
        let ticker = format!("{}{more}", names.join("  ·  "));
        main.items.push(cx.caps(&gettext("On This Disc"), Rect::new(140.0, 894.0, 330.0, 86.0), 26, cx.theme.selected, Align::Left, 4.0));
        main.items.push(cx.caps(&ticker, Rect::new(470.0, 894.0, 1310.0, 86.0), 26, cx.theme.dim, Align::Left, 2.0));
    }
    for (i, (label, action)) in show_buttons(&pages, &episodes, setup).iter().enumerate() {
        let mut b = cx.button_font(&label.to_uppercase(), *action, Rect::new(140.0, 400.0 + i as f64 * 132.0, 820.0, 108.0), format!("{} Bold 52", cx.body), Align::Left, Highlight::Fill);
        if let Some(b) = b.button_mut() {
            b.fill = Some(block);
        }
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    finish(main, pages, &episodes, PER_PAGE, title_item)
}
