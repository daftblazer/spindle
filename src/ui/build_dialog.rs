// SPDX-License-Identifier: GPL-3.0-or-later

//! "Build Disc" dialog: choose an output folder, run the build pipeline on
//! a worker thread and show progress.

use crate::build::{self, BuildEvent};
use crate::document::Document;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub fn estimated_size_gb(doc: &Document) -> f64 {
    let p = doc.project();
    let secs: f64 = p.titles.iter().filter_map(|t| p.asset(t.asset)).map(|a| a.info.duration).sum();
    let kbps = p.disc.video_bitrate as f64 + p.disc.audio_bitrate as f64 * 1.1 + 200.0;
    secs * kbps * 1000.0 / 8.0 / 1e9
}

fn default_output(doc: &Document) -> PathBuf {
    let name = doc.project().disc.name.clone();
    let folder = format!("{} Blu-ray", if name.is_empty() { "Disc" } else { &name });
    // Next to the last output, else next to the project, else in Videos.
    let recent = crate::app_settings()
        .map(|s| s.string("recent-output").to_string())
        .filter(|s| !s.is_empty())
        .and_then(|s| PathBuf::from(s).parent().map(|p| p.to_path_buf()));
    let base = recent
        .or_else(|| doc.path().and_then(|p| p.parent().map(|d| d.to_path_buf())))
        .or_else(|| glib::user_special_dir(glib::UserDirectory::Videos))
        .unwrap_or_else(glib::home_dir);
    base.join(folder)
}

pub fn present(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::Dialog::builder().title(gettext("Build Disc")).content_width(520).content_height(460).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    let output = Rc::new(RefCell::new(default_output(doc)));

    // --- setup page
    let page = adw::PreferencesPage::new();
    let problems = build::check(&doc.project());
    if !problems.is_empty() {
        let g = adw::PreferencesGroup::builder().title(gettext("Fix Before Building")).build();
        for pr in &problems {
            let row = adw::ActionRow::builder().title(pr.as_str()).title_lines(3).build();
            row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            g.add(&row);
        }
        page.add(&g);
    }
    let g = adw::PreferencesGroup::builder()
        .title(gettext("Output"))
        .description(gettext("Spindle writes a BDMV folder that can be played with VLC or burned to a BD-R with any disc burning tool."))
        .build();
    let folder_row = adw::ActionRow::builder()
        .title(gettext("Folder"))
        .subtitle(output.borrow().to_string_lossy().as_ref())
        .subtitle_selectable(true)
        .build();
    let choose = gtk::Button::builder().label(gettext("Choose…")).valign(gtk::Align::Center).build();
    folder_row.add_suffix(&choose);
    g.add(&folder_row);
    let size = estimated_size_gb(doc);
    let size_row = adw::ActionRow::builder()
        .title(gettext("Estimated Size"))
        .subtitle(format!("{size:.2} GB {} ({})", gettext("of video"), if size <= 23.0 { "BD-25" } else if size <= 46.0 { "BD-50" } else { "BD-100" }))
        .build();
    g.add(&size_row);
    page.add(&g);

    let bg = adw::PreferencesGroup::new();
    let build_btn = gtk::Button::builder()
        .label(gettext("_Build"))
        .use_underline(true)
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .sensitive(problems.is_empty())
        .build();
    bg.add(&build_btn);
    page.add(&bg);
    stack.add_named(&page, Some("setup"));

    // --- progress page
    let status = adw::StatusPage::builder()
        .icon_name("media-optical-symbolic")
        .title(gettext("Building Disc…"))
        .build();
    status.add_css_class("compact");
    let pbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .build();
    let bar = gtk::ProgressBar::builder().show_text(true).build();
    pbox.append(&bar);
    let log = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).build();
    let scroller = gtk::ScrolledWindow::builder().min_content_height(120).child(&log).build();
    let expander = gtk::Expander::builder().label(gettext("Details")).child(&scroller).build();
    pbox.append(&expander);
    let cancel_btn = gtk::Button::builder()
        .label(gettext("_Cancel"))
        .use_underline(true)
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    pbox.append(&cancel_btn);
    status.set_child(Some(&pbox));
    stack.add_named(&status, Some("progress"));

    // --- result page
    let result = adw::StatusPage::new();
    result.add_css_class("compact");
    let rbox = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    let open_btn = gtk::Button::builder().label(gettext("_Open Folder")).use_underline(true).css_classes(["pill", "suggested-action"]).build();
    let close_btn = gtk::Button::builder().label(gettext("_Close")).use_underline(true).css_classes(["pill"]).build();
    rbox.append(&open_btn);
    rbox.append(&close_btn);
    result.set_child(Some(&rbox));
    stack.add_named(&result, Some("result"));

    // --- behaviour
    let d = dialog.clone();
    let out = output.clone();
    let fr = folder_row.clone();
    choose.connect_clicked(move |_| {
        let fd = gtk::FileDialog::builder().title(gettext("Choose Output Folder")).modal(true).build();
        if let Some(parent) = out.borrow().parent() {
            fd.set_initial_folder(Some(&gio::File::for_path(parent)));
        }
        let out = out.clone();
        let fr = fr.clone();
        let root = d.root().and_downcast::<gtk::Window>();
        fd.select_folder(root.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Ok(f) = res {
                if let Some(p) = f.path() {
                    fr.set_subtitle(&p.to_string_lossy());
                    *out.borrow_mut() = p;
                }
            }
        });
    });

    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    cancel_btn.connect_clicked(move |b| {
        c.store(true, Ordering::Relaxed);
        b.set_sensitive(false);
    });
    let c = cancel.clone();
    dialog.connect_closed(move |_| c.store(true, Ordering::Relaxed));
    let d = dialog.clone();
    close_btn.connect_clicked(move |_| {
        d.close();
    });
    let out = output.clone();
    let d = dialog.clone();
    open_btn.connect_clicked(move |_| {
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&*out.borrow())));
        let root = d.root().and_downcast::<gtk::Window>();
        launcher.launch(root.as_ref(), gio::Cancellable::NONE, |_| {});
    });

    let doc = doc.clone();
    build_btn.connect_clicked(move |_| {
        stack.set_visible_child_name("progress");
        let project = doc.project().clone();
        let out = output.borrow().clone();
        if let Some(settings) = crate::app_settings() {
            let _ = settings.set_string("recent-output", &out.to_string_lossy());
        }
        let cancel = cancel.clone();
        let (tx, rx) = async_channel::unbounded::<BuildEvent>();
        std::thread::spawn(move || {
            let emit = |ev: BuildEvent| {
                let _ = tx.send_blocking(ev);
            };
            let res = build::build(&project, &out, &cancel, &emit).map_err(|e| format!("{e:#}"));
            emit(BuildEvent::Finished(res));
        });
        let (status, bar, log, result, stack, open_btn) =
            (status.clone(), bar.clone(), log.clone(), result.clone(), stack.clone(), open_btn.clone());
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                match ev {
                    BuildEvent::Stage(s) => status.set_description(Some(&s)),
                    BuildEvent::Progress(p) => {
                        bar.set_fraction(p);
                        bar.set_text(Some(&format!("{:.0} %", p * 100.0)));
                    }
                    BuildEvent::Log(l) => {
                        let buf = log.buffer();
                        let mut end = buf.end_iter();
                        buf.insert(&mut end, &format!("{l}\n"));
                    }
                    BuildEvent::Finished(res) => {
                        match res {
                            Ok(path) => {
                                result.set_icon_name(Some("emblem-ok-symbolic"));
                                result.set_title(&gettext("Disc Ready"));
                                result.set_description(Some(&format!(
                                    "{}\n<small>{}</small>",
                                    gettext("Your Blu-ray folder has been written."),
                                    glib::markup_escape_text(&path.to_string_lossy())
                                )));
                                open_btn.set_visible(true);
                            }
                            Err(e) => {
                                let cancelled = e.contains("cancelled");
                                result.set_icon_name(Some(if cancelled { "process-stop-symbolic" } else { "dialog-error-symbolic" }));
                                result.set_title(&if cancelled { gettext("Build Cancelled") } else { gettext("Build Failed") });
                                result.set_description(Some(&glib::markup_escape_text(if cancelled { "" } else { &e })));
                                open_btn.set_visible(false);
                            }
                        }
                        stack.set_visible_child_name("result");
                        break;
                    }
                }
            }
        });
    });

    dialog.present(Some(parent));
}
