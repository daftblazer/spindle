// SPDX-License-Identifier: GPL-3.0-or-later

//! "Save Menus as Template": the project's main menu and episode page
//! layout become a custom template, kept in the app or exported to a file.

use crate::document::Document;
use crate::templates::custom;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::rc::Rc;

pub fn present(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>, toast: impl Fn(String) + 'static) {
    let dialog = adw::Dialog::builder().title(gettext("Save Menus as Template")).content_width(520).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let g = adw::PreferencesGroup::builder()
        .description(gettext(
            "Saves the main menu and the layout of the episode pages. Applied to another show, the title, season, episode names, running times and stills are filled in, and there are as many pages as its episodes need. Setup and pop-up menus are made in the template's colours.\n\nTo give a template 4:3 menus as well, make 4:3 menus (Shape in the menu's properties) and save them under the same name.",
        ))
        .build();
    let name = adw::EntryRow::builder().title(gettext("Name")).text(format!("{} {}", doc.project().disc.name, gettext("Menus"))).build();
    let description = adw::EntryRow::builder().title(gettext("Description (optional)")).build();
    let author = adw::EntryRow::builder().title(gettext("Made By (optional)")).build();
    g.add(&name);
    g.add(&description);
    g.add(&author);
    page.add(&g);

    // Problems converting the menus (no episode pages…).
    let problem = adw::PreferencesGroup::new();
    let problem_row = adw::ActionRow::builder().title_lines(4).build();
    problem_row.add_prefix(&gtk::Image::builder().icon_name("dialog-warning-symbolic").css_classes(["warning"]).build());
    problem.add(&problem_row);
    problem.set_visible(false);
    page.add(&problem);

    let bg = adw::PreferencesGroup::new();
    let buttons = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    let save = gtk::Button::builder().label(gettext("_Save")).use_underline(true).css_classes(["suggested-action", "pill"]).build();
    let export = gtk::Button::builder().label(gettext("_Export to File…")).use_underline(true).css_classes(["pill"]).build();
    buttons.append(&save);
    buttons.append(&export);
    bg.add(&buttons);
    page.add(&bg);

    let make = {
        let (doc, name, description, author, problem, problem_row) = (doc.clone(), name.clone(), description.clone(), author.clone(), problem.clone(), problem_row.clone());
        move || -> Option<custom::CustomTemplate> {
            let n = name.text().trim().to_string();
            let n = if n.is_empty() { gettext("My Template") } else { n };
            match custom::from_project(&doc.project(), &n, description.text().trim(), author.text().trim()) {
                Ok(t) => Some(t),
                Err(e) => {
                    problem_row.set_title(&glib::markup_escape_text(&format!("{}: {e}", gettext("Can't make a template"))));
                    problem.set_visible(true);
                    None
                }
            }
        }
    };
    let toast = Rc::new(toast);
    {
        let (make, d, toast) = (make.clone(), dialog.clone(), toast.clone());
        save.connect_clicked(move |_| {
            let Some(t) = make() else { return };
            // Saving the other shape under the same name adds it: 16:9 and
            // 4:3 designs travel in one template.
            let (t, joined) = match custom::find(&t.name) {
                Some(old) if old.standard.is_some() || t.standard.is_some() => (old.combine(&t), true),
                _ => (t, false),
            };
            match custom::install(&t) {
                Ok(_) => {
                    toast(if joined && t.has_standard() {
                        gettext("Saved to My Templates, with 16:9 and 4:3 menus")
                    } else {
                        gettext("Saved to My Templates")
                    });
                    d.close();
                }
                Err(e) => toast(e.to_string()),
            }
        });
    }
    {
        let d = dialog.clone();
        export.connect_clicked(move |_| {
            let Some(t) = make() else { return };
            export_file(&d, t, toast.clone());
        });
    }
    dialog.present(Some(parent));
}

/// Ask where to write `t`, then write it.
pub fn export_file(parent: &impl IsA<gtk::Widget>, t: custom::CustomTemplate, toast: Rc<dyn Fn(String)>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&gettext("Spindle Templates")));
    filter.add_pattern(&format!("*.{}", custom::EXTENSION));
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let file_name: String = t.name.chars().map(|c| if c == '/' { '-' } else { c }).collect();
    let fd = gtk::FileDialog::builder()
        .title(gettext("Export Template"))
        .initial_name(format!("{file_name}.{}", custom::EXTENSION))
        .filters(&filters)
        .modal(true)
        .build();
    let root = parent.root().and_downcast::<gtk::Window>();
    fd.save(root.as_ref(), gio::Cancellable::NONE, move |res| {
        let Some(path) = res.ok().and_then(|f| f.path()) else { return };
        let path = crate::media::portal::real_path(&path);
        match custom::save(&t, &path) {
            Ok(()) => toast(gettext("Template exported")),
            Err(e) => toast(e.to_string()),
        }
    });
}
