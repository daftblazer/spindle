// SPDX-License-Identifier: GPL-3.0-or-later

//! Editor UI components.

pub mod build_dialog;
pub mod burn;
pub mod canvas;
pub mod compat_dialog;
pub mod frame_picker;
pub mod inspector;
pub mod media_bin;
pub mod player;
pub mod preview_dialog;
pub mod rows;
pub mod settings;
pub mod sidebar;
pub mod templates;
pub mod title_view;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

/// Show `menu` as a context menu at (`x`, `y`) in `parent`.
pub fn context_menu(parent: &impl IsA<gtk::Widget>, menu: &gio::Menu, x: f64, y: f64) {
    let pop = gtk::PopoverMenu::from_model(Some(menu));
    pop.set_parent(parent);
    pop.set_has_arrow(false);
    pop.set_halign(gtk::Align::Start);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    // Section separators are shown from an idle callback; popping up
    // before it runs sizes the menu without them, leaving it scrolling.
    glib::idle_add_local_once(move || pop.popup());
}
