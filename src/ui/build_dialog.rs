// SPDX-License-Identifier: GPL-3.0-or-later

//! "Build Disc" dialog: choose an output folder, run the build pipeline on
//! a worker thread and show progress.

use crate::build::{self, BuildEvent};
use crate::document::{Change, Document, Node};
use crate::validate::{self, Severity, Target};
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Last output path, if any.
fn recent_output() -> Option<PathBuf> {
    crate::app_settings().map(|s| s.string("recent-output").to_string()).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Default output: an image, unless the last build made a folder.
fn default_output(doc: &Document) -> PathBuf {
    let name = doc.project().disc.name.clone();
    let folder = format!("{} Blu-ray", if name.is_empty() { "Disc" } else { &name });
    // Next to the last output, else next to the project, else in Videos.
    let recent = recent_output().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let base = recent
        .or_else(|| doc.path().and_then(|p| p.parent().map(|d| d.to_path_buf())))
        .or_else(|| glib::user_special_dir(glib::UserDirectory::Videos))
        .unwrap_or_else(glib::home_dir);
    let out = base.join(folder);
    let folder = recent_output().is_some_and(|p| !build::is_image(&p));
    with_image(&out, !folder)
}

/// Switch an output path between "X" (folder) and "X.iso" (image).
fn with_image(path: &std::path::Path, image: bool) -> PathBuf {
    match (build::is_image(path), image) {
        (false, true) => {
            let mut s = path.as_os_str().to_os_string();
            s.push(".iso");
            s.into()
        }
        (true, false) => path.with_extension(""),
        _ => path.to_path_buf(),
    }
}

pub fn present(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::Dialog::builder().title(gettext("Build Disc")).content_width(560).content_height(620).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    let output = Rc::new(RefCell::new(default_output(doc)));

    // --- setup page
    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::builder().title(gettext("Output")).build();
    let format_row = adw::ComboRow::builder()
        .title(gettext("Format"))
        .model(&gtk::StringList::new(&[&gettext("Disc Image (ISO)"), &gettext("Blu-ray Folder (BDMV)"), &gettext("Burn to Disc")]))
        .selected(if build::is_image(&output.borrow()) { 0 } else { 1 })
        .build();
    g.add(&format_row);
    let folder_row = adw::ActionRow::builder().subtitle(output.borrow().to_string_lossy().as_ref()).subtitle_selectable(true).build();
    let choose = gtk::Button::builder().label(gettext("Choose…")).valign(gtk::Align::Center).build();
    folder_row.add_suffix(&choose);
    g.add(&folder_row);
    let burning = Rc::new(Cell::new(false));
    let burn_options = super::burn::BurnOptions::new(&g);
    burn_options.set_visible(false);
    let describe = {
        let (g, folder_row, output, burning) = (g.clone(), folder_row.clone(), output.clone(), burning.clone());
        move || {
            folder_row.set_visible(!burning.get());
            if burning.get() {
                g.set_description(Some(&gettext("Builds the disc, burns it to a BD-R or BD-RE, and checks what was written.")));
                return;
            }
            let image = build::is_image(&output.borrow());
            folder_row.set_title(&if image { gettext("File") } else { gettext("Folder") });
            folder_row.set_subtitle(&output.borrow().to_string_lossy());
            g.set_description(Some(&if image {
                gettext("A single file to burn to a BD-R with any disc burning app, or to open in VLC or Kodi.")
            } else {
                gettext("A BDMV folder that plays in VLC or Kodi; burning it needs a tool that writes UDF 2.50.")
            }));
        }
    };
    describe();
    {
        let (output, describe, burning, options) = (output.clone(), describe.clone(), burning.clone(), burn_options.clone());
        format_row.connect_selected_notify(move |r| {
            burning.set(r.selected() == 2);
            options.set_visible(burning.get());
            if !burning.get() {
                let path = with_image(&output.borrow(), r.selected() == 0);
                *output.borrow_mut() = path;
            }
            describe();
        });
    }
    let size_row = adw::ActionRow::builder().title(gettext("Estimated Size")).build();
    let fit_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    let fit = gtk::MenuButton::builder()
        .label(gettext("Fit to Disc"))
        .valign(gtk::Align::Center)
        .tooltip_text(gettext("Choose the video bitrate that fills a disc"))
        .popover(&gtk::Popover::builder().child(&fit_box).build())
        .build();
    size_row.add_suffix(&fit);
    g.add(&size_row);
    page.add(&g);

    // Encodes kept from earlier builds.
    let cache_size = crate::encode_cache::size();
    if cache_size > 0 {
        let cg = adw::PreferencesGroup::new();
        let row = adw::ActionRow::builder()
            .title(gettext("Earlier Encodes"))
            .subtitle(gettext("{} GB kept, so rebuilding after changes skips encoding videos again").replace("{}", &format!("{:.1}", cache_size as f64 / 1e9)))
            .subtitle_lines(2)
            .build();
        let clear = gtk::Button::builder().label(gettext("Clear")).valign(gtk::Align::Center).build();
        let r = row.clone();
        clear.connect_clicked(move |b| {
            crate::encode_cache::clear();
            r.set_subtitle(&gettext("Cleared"));
            b.set_sensitive(false);
        });
        row.add_suffix(&clear);
        cg.add(&row);
        page.add(&cg);
    }

    let issues_group = adw::PreferencesGroup::new();
    page.add(&issues_group);

    let bg = adw::PreferencesGroup::new();
    let build_btn = gtk::Button::builder()
        .label(gettext("_Build"))
        .use_underline(true)
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    bg.add(&build_btn);
    page.add(&bg);
    stack.add_named(&page, Some("setup"));

    // Problems and size follow the project (Fit to Disc changes it).
    let issue_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let doc = doc.clone();
        let (burning, options) = (burning.clone(), Rc::downgrade(&burn_options));
        let (dialog, parent) = (dialog.downgrade(), parent.as_ref().clone());
        let (size_row, fit_box, fit, build_btn, issues_group) = (size_row.clone(), fit_box.clone(), fit.clone(), build_btn.clone(), issues_group.clone());
        Rc::new(move || {
            let (issues, bytes, fits) = {
                let p = doc.project();
                let fits: Vec<(&'static str, u32)> = validate::DISCS
                    .iter()
                    .filter_map(|(name, cap)| Some((*name, validate::fit_bitrate(&p, *cap)?.min(validate::MAX_VIDEO_KBPS))))
                    .filter(|(_, kbps)| *kbps >= validate::MIN_VIDEO_KBPS)
                    .collect();
                (validate::check(&p), validate::estimated_bytes(&p), fits)
            };
            let disc = validate::disc_for(bytes).map_or_else(|| gettext("too big for any disc"), |(d, _)| d.to_string());
            size_row.set_subtitle(&format!("{:.1} GB · {disc}", bytes / 1e9));

            while let Some(c) = fit_box.first_child() {
                fit_box.remove(&c);
            }
            fit.set_visible(!fits.is_empty());
            for (name, kbps) in fits {
                let b = gtk::Button::builder()
                    .label(format!("{name} · {:.1} Mbit/s", kbps as f64 / 1000.0))
                    .css_classes(["flat"])
                    .build();
                let (doc, fit) = (doc.clone(), fit.clone());
                b.connect_clicked(move |_| {
                    fit.popdown();
                    doc.edit(Change::Content, |p| p.disc.video_bitrate = kbps);
                });
                fit_box.append(&b);
            }

            for r in issue_rows.borrow_mut().drain(..) {
                issues_group.remove(&r);
            }
            let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
            issues_group.set_visible(!issues.is_empty());
            issues_group.set_title(&if errors > 0 { gettext("Fix Before Building") } else { gettext("Suggestions") });
            for issue in issues {
                let row = adw::ActionRow::builder().title(glib::markup_escape_text(&issue.message).as_str()).title_lines(3).build();
                let (icon, class) = match issue.severity {
                    Severity::Error => ("dialog-error-symbolic", "error"),
                    Severity::Warning => ("dialog-warning-symbolic", "warning"),
                };
                row.add_prefix(&gtk::Image::builder().icon_name(icon).css_classes([class]).build());
                if let Some(target) = issue.target {
                    row.set_activatable(true);
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    let (doc, dialog, parent) = (doc.clone(), dialog.clone(), parent.clone());
                    row.connect_activated(move |_| {
                        if let Some(d) = dialog.upgrade() {
                            d.close();
                        }
                        match target {
                            Target::Menu(m) => doc.select(Node::Menu(m), None),
                            Target::Item { menu, item } => doc.select(Node::Menu(menu), Some(item)),
                            Target::Title(t) => doc.select(Node::Title(t), None),
                            Target::Settings => {
                                let _ = WidgetExt::activate_action(&parent, "win.disc-settings", None);
                            }
                        }
                    });
                }
                issues_group.add(&row);
                issue_rows.borrow_mut().push(row);
            }
            // Burning also needs a disc that the build fits on.
            let disc_problem = if burning.get() { options.upgrade().and_then(|o| o.problem(bytes)) } else { None };
            build_btn.set_sensitive(errors == 0 && disc_problem.is_none());
            build_btn.set_tooltip_text(disc_problem.as_deref());
            build_btn.set_label(&if burning.get() { gettext("_Build and Burn") } else { gettext("_Build") });
        })
    };
    refresh();
    {
        let weak = Rc::downgrade(&refresh);
        let again = move || {
            if let Some(r) = weak.upgrade() {
                r();
            }
        };
        burn_options.connect_changed(again.clone());
        format_row.connect_selected_notify(move |_| again());
    }
    // The document outlives the dialog, so it only holds a weak reference.
    let weak = Rc::downgrade(&refresh);
    doc.connect(move |c| {
        if let (Some(r), true) = (weak.upgrade(), matches!(c, Change::Content | Change::Structure)) {
            r();
        }
    });
    dialog.connect_closed(move |_| {
        let _keep = &refresh;
    });

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
    choose.connect_clicked(move |_| {
        let image = build::is_image(&out.borrow());
        let name = out.borrow().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fd = gtk::FileDialog::builder()
            .title(if image { gettext("Save Disc Image") } else { gettext("Choose Output Folder") })
            .modal(true)
            .build();
        if let Some(parent) = out.borrow().parent() {
            fd.set_initial_folder(Some(&gio::File::for_path(parent)));
        }
        let out = out.clone();
        let describe = describe.clone();
        let done = move |res: Result<gio::File, glib::Error>| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                *out.borrow_mut() = with_image(&crate::media::portal::real_path(&p), image);
                describe();
            }
        };
        let root = d.root().and_downcast::<gtk::Window>();
        if image {
            fd.set_initial_name(Some(&name));
            fd.save(root.as_ref(), gio::Cancellable::NONE, done);
        } else {
            fd.select_folder(root.as_ref(), gio::Cancellable::NONE, done);
        }
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
        let path = out.borrow().clone();
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&path)));
        let root = d.root().and_downcast::<gtk::Window>();
        if build::is_image(&path) {
            launcher.open_containing_folder(root.as_ref(), gio::Cancellable::NONE, |_| {});
        } else {
            launcher.launch(root.as_ref(), gio::Cancellable::NONE, |_| {});
        }
    });

    let doc = doc.clone();
    let d = dialog.clone();
    build_btn.connect_clicked(move |_| {
        let target = if burning.get() { burn_options.target() } else { None };
        let start = {
            let (doc, output, cancel, stack, status, bar, log, result, open_btn) =
                (doc.clone(), output.clone(), cancel.clone(), stack.clone(), status.clone(), bar.clone(), log.clone(), result.clone(), open_btn.clone());
            let target = target.clone();
            move || {
        let target = target.clone();
        stack.set_visible_child_name("progress");
        let project = doc.project().clone();
        let out = output.borrow().clone();
        if target.is_none() {
            if let Some(settings) = crate::app_settings() {
                let _ = settings.set_string("recent-output", &out.to_string_lossy());
            }
        }
        let cancel = cancel.clone();
        let (tx, rx) = async_channel::unbounded::<BuildEvent>();
        let burn_target = target.clone();
        std::thread::spawn(move || {
            let emit = |ev: BuildEvent| {
                let _ = tx.send_blocking(ev);
            };
            let res = match &burn_target {
                None => build::build(&project, &out, &cancel, &emit),
                // Build an image in the cache, burn it, then remove it.
                Some(t) => (|| {
                    let dir = crate::media::cache_dir().join("burn");
                    std::fs::create_dir_all(&dir)?;
                    let image = dir.join("disc.iso");
                    let scaled = |ev: BuildEvent| match ev {
                        BuildEvent::Progress(p) => emit(BuildEvent::Progress(p * 0.6)),
                        BuildEvent::Finished(_) => {}
                        ev => emit(ev),
                    };
                    let r = build::build(&project, &image, &cancel, &scaled).and_then(|_| super::burn::run(&image, t, &cancel, &emit, 0.6));
                    let _ = std::fs::remove_file(&image);
                    r.map(|_| t.drive.clone())
                })(),
            }
            .map_err(|e| format!("{e:#}"));
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
                    BuildEvent::Finished(res) if target.is_some() => {
                        super::burn::show_result(&result, res.map(|_| ()));
                        open_btn.set_visible(false);
                        stack.set_visible_child_name("result");
                        break;
                    }
                    BuildEvent::Finished(res) => {
                        match res {
                            Ok(path) => {
                                result.set_icon_name(Some("object-select-symbolic"));
                                result.set_title(&gettext("Disc Ready"));
                                result.set_description(Some(&format!(
                                    "{}\n<small>{}</small>",
                                    if build::is_image(&path) { gettext("Your disc image has been written.") } else { gettext("Your Blu-ray folder has been written.") },
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
            }
        };
        match &target {
            Some(t) => super::burn::confirm_erase(&d, t, start),
            None => start(),
        }
    });

    #[cfg(debug_assertions)]
    if std::env::var("SPINDLE_SCREENSHOT_BURN").is_ok() {
        format_row.set_selected(2);
    }
    dialog.present(Some(parent));
}
