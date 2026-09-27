// SPDX-License-Identifier: GPL-3.0-or-later

//! Movie styles: the first title is the film, the others are extras.

use super::show::{Card, Row};
use super::*;

pub(super) fn build(style: Style, input: Input) -> Output {
    match style {
        Style::MovieShowcase => showcase(input),
        Style::MovieMinimal => minimal(input),
        _ => classic(input),
    }
}

/// "2 h 14 min" or "48 min".
fn runtime(secs: f64) -> String {
    let m = (secs / 60.0).round() as u32;
    if m >= 60 {
        gettext("{h} h {m} min").replace("{h}", &(m / 60).to_string()).replace("{m}", &(m % 60).to_string())
    } else {
        gettext("{} min").replace("{}", &m.max(1).to_string())
    }
}

/// Everything a movie layout links to.
struct Parts {
    film: Option<Episode>,
    extras: Vec<Episode>,
    scenes: Vec<Menu>,
    extra_pages: Vec<Menu>,
    /// Extras per page.
    per_page: usize,
}

impl Parts {
    fn buttons(&self, setup: Option<Id>) -> Vec<(String, Action)> {
        let mut v = Vec::new();
        if let Some(f) = &self.film {
            v.push((gettext("Play Movie"), f.action()));
        }
        if let Some(s) = self.scenes.first() {
            v.push((gettext("Scene Selection"), Action::ShowMenu(s.id)));
        }
        if let Some(e) = self.extra_pages.first() {
            v.push((gettext("Extras"), Action::ShowMenu(e.id)));
        }
        if let Some(s) = setup {
            v.push((gettext("Setup"), Action::ShowMenu(s)));
        }
        v
    }

    fn finish(self, main: Menu, title_item: Option<Id>) -> Output {
        let mut return_to: Vec<(Id, Id)> = self.film.iter().map(|f| (f.title, main.id)).collect();
        for (i, e) in self.extras.iter().enumerate() {
            return_to.push((e.title, self.extra_pages[(i / self.per_page).min(self.extra_pages.len() - 1)].id));
        }
        let popup_links = self.scenes.first().map(|s| (gettext("Scene Selection"), Action::ShowMenu(s.id))).into_iter().collect();
        let mut menus = vec![main];
        menus.extend(self.scenes);
        menus.extend(self.extra_pages);
        Output { menus, return_to, title_item, popup_links }
    }
}

fn split_titles(episodes: Vec<Episode>) -> (Option<Episode>, Vec<Episode>) {
    let mut it = episodes.into_iter();
    (it.next(), it.collect())
}

fn classic(input: Input) -> Output {
    let (edition, home, setup, p) = (input.edition(), input.home(), input.setup, input.p);
    let Input { cx, name, mut main, episodes, .. } = input;
    let (film, extras) = split_titles(episodes);
    let scenes = film.as_ref().map(|f| cx.grid_pages(&gettext("Scene Selection"), &scene_entries(p, f.title), home.clone(), "")).unwrap_or_default();
    let extra_list: Vec<(String, Action)> = extras.iter().map(|e| (short(&e.name_or_title(), 36), e.action())).collect();
    let extra_pages = if extras.is_empty() { vec![] } else { cx.list_pages(&gettext("Extras"), &gettext("Extras"), &extra_list, home) };
    let parts = Parts { film, extras, scenes, extra_pages, per_page: LIST_PAGE };

    if let Some(f) = &parts.film {
        main.items.push(cx.still(f.asset, f.poster, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT)));
    }
    main.items.push(MenuItem::new_shape(alpha(cx.theme.bottom, 0.8), 0.0, Rect::new(0.0, 700.0, DESIGN_WIDTH, 380.0)));
    if let Some(e) = &edition {
        main.items.push(cx.caps(e, Rect::new(120.0, 712.0, 1680.0, 36.0), 22, cx.theme.selected, Align::Left, 5.0));
    }
    let title = cx.head(&name, Rect::new(120.0, 745.0, 1680.0, 130.0), 80, "ExtraBold", Align::Left);
    let title_item = Some(title.id);
    main.items.push(title);
    if parts.film.is_none() {
        main.items.push(cx.text(&gettext("Import your movie, then apply the template again"), Rect::new(120.0, 880.0, 1680.0, 80.0), 32, "", Align::Left, true));
    }
    let mut x = 110.0;
    for (i, (label, action)) in parts.buttons(setup).iter().enumerate() {
        let w = (80.0 + label.chars().count() as f64 * 24.0).max(260.0);
        let b = cx.button(label, *action, Rect::new(x, 895.0, w, 90.0), 42, Align::Center, Highlight::Frame);
        x += w + 30.0;
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    parts.finish(main, title_item)
}

fn showcase(input: Input) -> Output {
    let (edition, home, setup, p) = (input.edition(), input.home(), input.setup, input.p);
    let Input { cx, name, mut main, episodes, .. } = input;
    let bg = cx.theme.bottom;
    let (film, extras) = split_titles(episodes);
    let scenes = film
        .as_ref()
        .map(|f| cx.card_pages(&gettext("Scene Selection"), edition.as_deref(), &scene_entries(p, f.title), home.clone(), ""))
        .unwrap_or_default();
    let cards: Vec<Card> = extras.iter().map(|e| (short(&e.name_or_title(), 28), e.action(), e.asset, e.poster)).collect();
    let extra_pages = if extras.is_empty() { vec![] } else { cx.card_pages(&gettext("Extras"), edition.as_deref(), &cards, home, "") };
    let parts = Parts { film, extras, scenes, extra_pages, per_page: 8 };

    main.background = Background { color: bg, ..Default::default() };
    if let Some(f) = &parts.film {
        main.items.push(cx.still(f.asset, f.poster, Rect::new(0.0, 0.0, DESIGN_WIDTH, DESIGN_HEIGHT)));
        main.items.push(cx.gradient(Rect::new(0.0, 0.0, 1340.0, DESIGN_HEIGHT), alpha(bg, 0.9), alpha(bg, 0.0), true));
        main.items.push(cx.gradient(Rect::new(0.0, 520.0, DESIGN_WIDTH, 560.0), alpha(bg, 0.0), alpha(bg, 0.97), false));
    }
    if let Some(e) = &edition {
        main.items.push(cx.caps(e, Rect::new(140.0, 400.0, 1100.0, 44.0), 28, cx.theme.selected, Align::Left, 6.0));
    }
    let mut title = cx.head(&name, Rect::new(140.0, 450.0, 1300.0, 250.0), title_size(&name, 108, 80, 18), "ExtraBold", Align::Left);
    if let ItemKind::Text(t) = &mut title.kind {
        t.style.line_spacing = 0.9;
    }
    let title_item = Some(title.id);
    main.items.push(title);
    if let Some(f) = &parts.film {
        main.items.push(cx.text(&runtime(f.duration), Rect::new(140.0, 705.0, 800.0, 44.0), 28, "Medium", Align::Left, true));
    }
    let buttons = parts.buttons(setup);
    // Four pills fit one row at a smaller size.
    let size = if buttons.len() > 3 { 28 } else { 32 };
    let ids = cx.pill_row(&mut main, &buttons, 140.0, 800.0, size);
    main.default_button = ids.first().copied();
    parts.finish(main, title_item)
}

fn minimal(input: Input) -> Output {
    let (edition, home, setup, p) = (input.edition(), input.home(), input.setup, input.p);
    let Input { cx, name, mut main, episodes, .. } = input;
    let (film, extras) = split_titles(episodes);
    let scene_rows: Vec<Row> = film
        .as_ref()
        .map(|f| {
            let t = p.title(f.title);
            std::iter::once(0.0)
                .chain(t.map(|t| t.chapters.clone()).unwrap_or_default())
                .enumerate()
                .map(|(i, c)| (format!("{} {}", gettext("Chapter"), i + 1), Action::PlayTitle { title: f.title, chapter: i as u32 }, clock(c)))
                .collect()
        })
        .unwrap_or_default();
    let scenes = if film.is_some() { cx.column_pages(&gettext("Scene Selection"), edition.as_deref(), &scene_rows, home.clone(), "") } else { vec![] };
    let rows: Vec<Row> = extras.iter().map(|e| (short(&e.name_or_title(), 30), e.action(), runtime(e.duration))).collect();
    let extra_pages = if extras.is_empty() { vec![] } else { cx.column_pages(&gettext("Extras"), edition.as_deref(), &rows, home, "") };
    let parts = Parts { film, extras, scenes, extra_pages, per_page: 14 };

    main.background = Background { color: cx.theme.bottom, ..Default::default() };
    let mut caps: Vec<String> = edition.iter().cloned().collect();
    if let Some(f) = &parts.film {
        caps.push(runtime(f.duration));
    }
    if !caps.is_empty() {
        main.items.push(cx.caps(&caps.join("  ·  "), Rect::new(160.0, 280.0, 1600.0, 40.0), 24, cx.theme.dim, Align::Center, 10.0));
    }
    let mut title = cx.styled(&name, Rect::new(160.0, 330.0, 1600.0, 200.0), format!("{} Light 110", cx.heading), cx.theme.text, Align::Center, false);
    if let ItemKind::Text(t) = &mut title.kind {
        t.style.letter_spacing = 3.0;
        t.style.line_spacing = 0.9;
    }
    let title_item = Some(title.id);
    main.items.push(title);
    main.items.push(MenuItem::new_shape(cx.theme.selected, 0.0, Rect::new(910.0, 555.0, 100.0, 3.0)));
    for (i, (label, action)) in parts.buttons(setup).iter().enumerate() {
        let mut b = cx.button_font(label, *action, Rect::new(660.0, 610.0 + i as f64 * 92.0, 600.0, 76.0), format!("{} Medium 34", cx.body), Align::Center, Highlight::Underline);
        if let Some(b) = b.button_mut() {
            b.text.shadow = false;
        }
        if i == 0 {
            main.default_button = Some(b.id);
        }
        main.items.push(b);
    }
    parts.finish(main, title_item)
}
