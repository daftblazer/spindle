// SPDX-License-Identifier: GPL-3.0-or-later

//! "Regenerate Episode Menus": make the episode pages again (after adding
//! titles, or in another style) without touching the other menus.

use crate::document::{Change, Document, Node};
use crate::model::Id;
use crate::templates::custom::{self, CustomTemplate};
use crate::templates::{self, Category, Options, Style, THEMES};
use std::sync::Arc;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

pub fn present(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>) {
    let (last, detected, menus) = {
        let p = doc.project();
        let menus: Vec<(Id, String, usize)> = p.menus.iter().filter(|m| !m.popup).map(|m| (m.id, m.name.clone(), m.buttons().count())).collect();
        (templates::last_options(&p), templates::episode_pages(&p), menus)
    };
    let dialog = adw::Dialog::builder().title(gettext("Regenerate Episode Menus")).content_width(520).content_height(640).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let g = adw::PreferencesGroup::builder()
        .title(gettext("Episode Pages"))
        .description(gettext(
            "Makes the episode pages again for the titles as they are now. Your other menus stay as they are, and buttons that led to the old pages lead to the new ones.",
        ))
        .build();
    // The built-in styles, then the saved templates.
    let styles = Category::TvShow.styles();
    let customs: Vec<Arc<CustomTemplate>> = custom::library().into_iter().map(|(_, t)| Arc::new(t)).collect();
    let mut names: Vec<String> = styles.iter().map(|s| s.name()).collect();
    names.extend(customs.iter().map(|t| t.name.clone()));
    let selected = match &last.custom {
        Some(c) => customs.iter().position(|t| t.name == c.name).map(|i| styles.len() + i),
        None => styles.iter().position(|s| *s == last.style),
    };
    let style_row = adw::ComboRow::builder()
        .title(gettext("Style"))
        .model(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>()))
        .selected(selected.unwrap_or(0) as u32)
        .build();
    let theme_row = adw::ComboRow::builder()
        .title(gettext("Palette"))
        .model(&gtk::StringList::new(&THEMES.iter().map(|t| t.name).collect::<Vec<_>>()))
        .selected(last.theme as u32)
        .build();
    let describe = {
        let (styles, customs, theme_row) = (styles.clone(), customs.clone(), theme_row.clone());
        move |r: &adw::ComboRow| {
            let i = r.selected() as usize;
            let (text, own_colours) = match styles.get(i) {
                Some(s) => (s.description(), false),
                None => (customs.get(i - styles.len()).map(|t| t.description.clone()).unwrap_or_default(), true),
            };
            r.set_subtitle(&text);
            theme_row.set_visible(!own_colours);
        }
    };
    describe(&style_row);
    style_row.set_subtitle_lines(2);
    style_row.connect_selected_notify(describe);
    g.add(&style_row);
    g.add(&theme_row);
    let season = adw::EntryRow::builder().title(gettext("Season (optional)")).text(&last.season).build();
    let disc = adw::EntryRow::builder().title(gettext("Disc (optional)")).text(&last.disc).build();
    g.add(&season);
    g.add(&disc);
    page.add(&g);

    let rg = adw::PreferencesGroup::builder()
        .title(gettext("Menus to Replace"))
        .description(gettext("The episode pages found are turned on. Everything else is kept."))
        .build();
    let chosen: Rc<RefCell<Vec<Id>>> = Rc::new(RefCell::new(detected.clone()));
    let go = gtk::Button::builder()
        .label(gettext("_Regenerate"))
        .use_underline(true)
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .sensitive(!detected.is_empty())
        .build();
    for (id, name, buttons) in &menus {
        let row = adw::SwitchRow::builder()
            .title(glib::markup_escape_text(name).as_str())
            .subtitle(ngettext("{} button", "{} buttons", *buttons as u32).replace("{}", &buttons.to_string()))
            .active(detected.contains(id))
            .build();
        let (chosen, go, id) = (chosen.clone(), go.clone(), *id);
        row.connect_active_notify(move |r| {
            let mut c = chosen.borrow_mut();
            c.retain(|x| *x != id);
            if r.is_active() {
                c.push(id);
            }
            go.set_sensitive(!c.is_empty());
        });
        rg.add(&row);
    }
    page.add(&rg);

    let bg = adw::PreferencesGroup::new();
    bg.add(&go);
    page.add(&bg);

    let (doc, d) = (doc.clone(), dialog.clone());
    go.connect_clicked(move |_| {
        let i = style_row.selected() as usize;
        let style: Style = styles.get(i).copied().unwrap_or(Style::Classic);
        let opts = Options {
            style,
            custom: i.checked_sub(styles.len()).and_then(|k| customs.get(k).cloned()),
            theme: (theme_row.selected() as usize).min(THEMES.len() - 1),
            season: season.text().to_string(),
            disc: disc.text().to_string(),
            ..last.clone()
        };
        // In the order of the menu list, so page 1 stays page 1.
        let old: Vec<Id> = {
            let p = doc.project();
            let c = chosen.borrow();
            p.menus.iter().map(|m| m.id).filter(|id| c.contains(id)).collect()
        };
        let new = doc.edit(Change::Structure, |p| templates::regenerate_pages(p, &opts, &old));
        if let Some(first) = new.first() {
            doc.select(Node::Menu(*first), None);
        }
        d.close();
    });
    dialog.present(Some(parent));
}
