// SPDX-License-Identifier: GPL-3.0-or-later

//! "Check Compatibility" for keeping a title's original video: runs the
//! analysis, stores the result on the title and shows what passed. Every
//! title can be checked at once, too.

use crate::document::{Change, Document};
use crate::media::compat::{self, Level, Report};
use crate::model::Id;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use std::collections::HashMap;
use std::path::PathBuf;
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
                    .title(gettext("Could Not Check the Video"))
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
        let d = dialog.clone();
        stack.add_named(&results_page(&doc, title, &report, move || {
            d.close();
        }), Some("result"));
        stack.set_visible_child_name("result");
    });
}

/// What the check found; `on_keep` runs after Keep Original Video is chosen.
fn results_page(doc: &Rc<Document>, title: Id, report: &Report, on_keep: impl Fn() + 'static) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let (icon, heading, body) = if !report.compatible() {
        ("dialog-error-symbolic", gettext("Needs Re-encoding"), gettext("This video is not Blu-ray compatible in its current form. Spindle will re-encode it when the disc is built."))
    } else if report.has_warnings() {
        ("dialog-warning-symbolic", gettext("Can Be Kept"), gettext("The video can be used without re-encoding, but does not fully conform to the Blu-ray specification. Most players handle this without problems."))
    } else {
        ("object-select-symbolic", gettext("Can Be Kept"), gettext("The video conforms to the Blu-ray specification and can be used without re-encoding, which is faster and preserves the original quality."))
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
        let doc = doc.clone();
        keep.connect_clicked(move |_| {
            doc.edit(Change::Structure, |p| {
                if let Some(t) = p.title_mut(title) {
                    t.keep_video = true;
                }
            });
            on_keep();
        });
        g.add(&keep);
        page.add(&g);
    }
    page.upcast()
}

/// Icon and style class for a check's outcome.
fn outcome_icon(r: &Result<Report, String>) -> (&'static str, &'static str) {
    match r {
        Ok(r) if !r.compatible() => ("dialog-error-symbolic", "error"),
        Ok(r) if r.has_warnings() => ("dialog-warning-symbolic", "warning"),
        Ok(_) => ("object-select-symbolic", "success"),
        Err(_) => ("dialog-error-symbolic", "error"),
    }
}

/// Check every title's video, one after another, listing what each can do.
pub fn check_all(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>) {
    let titles: Vec<(Id, String, Option<PathBuf>)> = {
        let p = doc.project();
        p.titles.iter().map(|t| (t.id, t.name.clone(), p.asset(t.asset).map(|a| a.path.clone()))).collect()
    };
    if titles.is_empty() {
        return;
    }
    let dialog = adw::Dialog::builder().title(gettext("Video Compatibility")).content_width(520).content_height(640).build();
    let nav = adw::NavigationView::new();
    dialog.set_child(Some(&nav));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    let summary = adw::PreferencesGroup::builder().title(gettext("Checking…")).build();
    page.add(&summary);
    let list = adw::PreferencesGroup::new();
    page.add(&list);
    let keep_group = adw::PreferencesGroup::builder().visible(false).build();
    let keep_all = gtk::Button::builder().use_underline(true).halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
    keep_group.add(&keep_all);
    page.add(&keep_group);
    toolbar.set_content(Some(&page));
    nav.add(&adw::NavigationPage::builder().title(gettext("Video Compatibility")).child(&toolbar).build());

    // One row per title, filled in as its check finishes.
    let rows: Vec<(adw::ActionRow, gtk::Stack)> = titles
        .iter()
        .map(|(_, name, _)| {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(name).as_str())
                .subtitle(gettext("Waiting"))
                .subtitle_lines(2)
                .build();
            let lead = gtk::Stack::new();
            lead.add_named(&adw::Spinner::new(), Some("busy"));
            lead.add_named(&gtk::Image::new(), Some("icon"));
            lead.set_visible_child_name("icon");
            row.add_prefix(&lead);
            list.add(&row);
            (row, lead)
        })
        .collect();
    dialog.present(Some(parent));

    let doc = doc.clone();
    glib::spawn_future_local(async move {
        // Titles that share a video are checked once.
        let mut done: HashMap<PathBuf, Result<Report, String>> = HashMap::new();
        let mut results: Vec<(Id, Result<Report, String>)> = Vec::new();
        for ((id, _, path), (row, lead)) in titles.iter().zip(&rows) {
            if dialog.root().is_none() {
                return;
            }
            let res = match path {
                None => Err(gettext("The video file is missing")),
                Some(path) => match done.get(path) {
                    Some(r) => r.clone(),
                    None => {
                        row.set_subtitle(&gettext("Checking…"));
                        lead.set_visible_child_name("busy");
                        let p = path.clone();
                        let r = gio::spawn_blocking(move || compat::analyze(&p).map_err(|e| format!("{e:#}")))
                            .await
                            .unwrap_or_else(|_| Err(gettext("The check stopped unexpectedly")));
                        done.insert(path.clone(), r.clone());
                        r
                    }
                },
            };
            let (icon, class) = outcome_icon(&res);
            if let Some(img) = lead.child_by_name("icon").and_downcast::<gtk::Image>() {
                img.set_icon_name(Some(icon));
                img.set_css_classes(&[class]);
            }
            lead.set_visible_child_name("icon");
            row.set_subtitle(&glib::markup_escape_text(&match &res {
                Ok(r) => r.summary(),
                Err(e) => e.lines().next().unwrap_or_default().to_string(),
            }));
            results.push((*id, res));
        }

        // Remember the results, as checking one title does, in one step.
        doc.edit(Change::Structure, |p| {
            for (id, res) in &results {
                let Ok(report) = res else { continue };
                if let Some(t) = p.title_mut(*id) {
                    t.video_check = Some(report.clone());
                    if !report.compatible() {
                        t.keep_video = false;
                    }
                }
            }
        });

        let compatible: Vec<Id> = results.iter().filter(|(_, r)| r.as_ref().is_ok_and(|r| r.compatible())).map(|(id, _)| *id).collect();
        let total = results.len() as u32;
        let n = compatible.len() as u32;
        summary.set_title(&ngettext("{n} of {total} title can be kept", "{n} of {total} titles can be kept", total).replace("{n}", &n.to_string()).replace("{total}", &total.to_string()));
        summary.set_description(Some(&if n == total {
            gettext("Every video is Blu-ray compatible and can be used without re-encoding.")
        } else if n == 0 {
            gettext("Spindle will re-encode every video when the disc is built.")
        } else {
            gettext("The others will be re-encoded when the disc is built. Select a title for details.")
        }));

        // Keep the original video of every compatible title.
        let show_keep = {
            let (doc, keep_group, keep_all, compatible) = (doc.clone(), keep_group.clone(), keep_all.clone(), compatible.clone());
            move || {
                let p = doc.project();
                let todo = compatible.iter().filter(|id| p.title(**id).is_some_and(|t| !t.keep_video)).count() as u32;
                keep_all.set_label(&ngettext("_Keep Original Video for {} Title", "_Keep Original Video for {} Titles", todo).replace("{}", &todo.to_string()));
                keep_group.set_visible(todo > 0);
            }
        };
        show_keep();
        {
            let (doc, show_keep) = (doc.clone(), show_keep.clone());
            keep_all.connect_clicked(move |_| {
                doc.edit(Change::Structure, |p| {
                    for id in &compatible {
                        if let Some(t) = p.title_mut(*id) {
                            t.keep_video = true;
                        }
                    }
                });
                show_keep();
            });
        }

        // A title's full report.
        for ((id, res), (row, _)) in results.into_iter().zip(&rows) {
            let Ok(report) = res else { continue };
            row.set_activatable(true);
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let (doc, nav, show_keep, name) = (doc.clone(), nav.clone(), show_keep.clone(), row.title());
            row.connect_activated(move |_| {
                let back = {
                    let (nav, show_keep) = (nav.clone(), show_keep.clone());
                    move || {
                        show_keep();
                        nav.pop();
                    }
                };
                let toolbar = adw::ToolbarView::new();
                toolbar.add_top_bar(&adw::HeaderBar::new());
                toolbar.set_content(Some(&results_page(&doc, id, &report, back)));
                nav.push(&adw::NavigationPage::builder().title(name.as_str()).child(&toolbar).build());
            });
        }
    });
}
