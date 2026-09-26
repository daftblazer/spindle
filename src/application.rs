/* application.rs
 *
 * Copyright 2026 daftblazer
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use gettextrs::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config::VERSION;
use crate::SpindleWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct SpindleApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for SpindleApplication {
        const NAME: &'static str = "SpindleApplication";
        type Type = super::SpindleApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for SpindleApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_gactions();
            obj.set_accels_for_action("app.quit", &["<control>q"]);
            for (action, accels) in [
                ("win.new", &["<control>n"][..]),
                ("win.open", &["<control>o"]),
                ("win.save", &["<control>s"]),
                ("win.save-as", &["<control><shift>s"]),
                ("win.undo", &["<control>z"]),
                ("win.redo", &["<control><shift>z", "<control>y"]),
                ("win.import", &["<control>i"]),
                ("win.build", &["<control>b"]),
                ("win.duplicate-item", &["<control>d"]),
                ("win.add-menu", &["<control>m"]),
                ("win.remote-preview", &["F5"]),
                ("win.templates", &["<control>t"]),
                ("win.preview-title", &["<control>p"]),
            ] {
                obj.set_accels_for_action(action, accels);
            }
        }
    }

    impl ApplicationImpl for SpindleApplication {
        // We connect to the activate callback to create a window when the application
        // has been launched. Additionally, this callback notifies us when the user
        // tries to launch a "second instance" of the application. When they try
        // to do that, we'll just present any existing window.
        fn activate(&self) {
            let application = self.obj();
            // Get the current window or create one if necessary
            let window = application.active_window().unwrap_or_else(|| {
                let window = SpindleWindow::new(&*application);
                window.upcast()
            });

            // Ask the window manager/compositor to present the window
            window.present();
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            let application = self.obj();
            let window = SpindleWindow::new(&*application);
            if let Some(path) = files.first().and_then(|f| f.path()) {
                window.open_path(&path);
            }
            window.present();
        }
    }

    impl GtkApplicationImpl for SpindleApplication {}
    impl AdwApplicationImpl for SpindleApplication {}
}

glib::wrapper! {
    pub struct SpindleApplication(ObjectSubclass<imp::SpindleApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SpindleApplication {
    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .property("resource-base-path", "/io/github/daftblazer/Spindle")
            .build()
    }

    fn setup_gactions(&self) {
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit())
            .build();
        let about_action = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        self.add_action_entries([quit_action, about_action]);
    }

    fn show_about(&self) {
        let window = self.active_window().unwrap();
        let about = adw::AboutDialog::builder()
            .application_name("Spindle")
            .application_icon("io.github.daftblazer.Spindle")
            .developer_name("daftblazer")
            .version(VERSION)
            .developers(vec!["daftblazer"])
            .comments(gettext("Author Blu-ray discs with interactive menus"))
            .license_type(gtk::License::Gpl30)
            // Translators: Replace "translator-credits" with your name/username, and optionally an email or URL.
            .translator_credits(gettext("translator-credits"))
            .copyright("© 2026 daftblazer")
            .build();

        about.present(Some(&window));
    }
}
