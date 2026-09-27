// SPDX-License-Identifier: GPL-3.0-or-later

//! "Check Compatibility" for keeping a title's original video: runs the
//! analysis, stores the result on the title and shows what passed.

use crate::document::{Change, Document};
use crate::media::compat::{self, Level, Report};
use crate::model::Id;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::rc::Rc;

pub fn check(doc: &Rc<Document>, title: Id, parent: &impl IsA<gtk::Widget>) {
    let Some(path) = ({
        let p = doc.project();
        p.title(title).and_then(|t| p.asset(t.asset)).map(|a| a.path.clone())
    }) else {
        return;
    };

    let dialog = adw::Dialog::builder().title(gettext("Video Compatibility")).content_width(520).content_height(640).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    let busy = adw::StatusPage::builder()
        .title(gettext("Checking…"))
        .description(glib::markup_escape_text(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).as_str())
        .child(&adw::Spinner::builder().width_request(32).height_request(32).build())
        .build();
    busy.add_css_class("compact");
    stack.add_named(&busy, Some("busy"));
    dialog.present(Some(parent));

    let doc = doc.clone();
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || compat::analyze(&path).map_err(|e| format!("{e:#}"))).await;
        let report = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                let err = adw::StatusPage::builder()
                    .icon_name("dialog-error-symbolic")
                    .title(gettext("Couldn't Check the Video"))
                    .description(glib::markup_escape_text(&e).as_str())
                    .build();
                err.add_css_class("compact");
                stack.add_named(&err, Some("result"));
                stack.set_visible_child_name("result");
                return;
            }
            Err(_) => return,
        };
        // Remember the result on the title; drop the option if it no longer fits.
        let compatible = report.compatible();
        doc.edit(Change::Structure, |p| {
            if let Some(t) = p.title_mut(title) {
                t.video_check = Some(report.clone());
                if !compatible {
                    t.keep_video = false;
                }
            }
        });
        stack.add_named(&results_page(&doc, title, &report, &dialog), Some("result"));
        stack.set_visible_child_name("result");
    });
}

fn results_page(doc: &Rc<Document>, title: Id, report: &Report, dialog: &adw::Dialog) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let (icon, heading, body) = if !report.compatible() {
        ("dialog-error-symbolic", gettext("Needs Re-encoding"), gettext("This video can't go on a Blu-ray as it is. Spindle will convert it when building the disc."))
    } else if report.has_warnings() {
        ("dialog-warning-symbolic", gettext("Can Be Kept"), gettext("The video can go on the disc without re-encoding, but isn't fully within the Blu-ray specification. Most players handle this."))
    } else {
        ("object-select-symbolic", gettext("Can Be Kept"), gettext("The video meets the Blu-ray specification and can go on the disc without re-encoding — faster, and with no loss of quality."))
    };
    let header = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_bottom(6).build();
    let class = if !report.compatible() {
        "error"
    } else if report.has_warnings() {
        "warning"
    } else {
        "success"
    };
    header.append(&gtk::Image::builder().icon_name(icon).pixel_size(64).css_classes([class]).build());
    header.append(&gtk::Label::builder().label(heading).css_classes(["title-2"]).build());
    header.append(&gtk::Label::builder().label(body).wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).build());
    let g = adw::PreferencesGroup::new();
    g.add(&header);
    page.add(&g);

    let g = adw::PreferencesGroup::new();
    for c in &report.checks {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&c.name).as_str())
            .subtitle(glib::markup_escape_text(&c.detail).as_str())
            .subtitle_lines(3)
            .build();
        let (icon, class) = match c.level {
            Level::Pass => ("object-select-symbolic", "success"),
            Level::Warn => ("dialog-warning-symbolic", "warning"),
            Level::Fail => ("dialog-error-symbolic", "error"),
        };
        row.add_prefix(&gtk::Image::builder().icon_name(icon).css_classes([class]).build());
        g.add(&row);
    }
    page.add(&g);

    let kept = doc.project().title(title).is_some_and(|t| t.keep_video);
    if report.compatible() && !kept {
        let g = adw::PreferencesGroup::new();
        let keep = gtk::Button::builder()
            .label(gettext("_Keep Original Video"))
            .use_underline(true)
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let (doc, dialog) = (doc.clone(), dialog.clone());
        keep.connect_clicked(move |_| {
            doc.edit(Change::Structure, |p| {
                if let Some(t) = p.title_mut(title) {
                    t.keep_video = true;
                }
            });
            dialog.close();
        });
        g.add(&keep);
        page.add(&g);
    }
    page.upcast()
}
