// SPDX-License-Identifier: GPL-3.0-or-later

//! "Build Disc" dialog: choose an output and queue the build as a job.

use crate::build;
use crate::document::{Change, Document, Node};
use crate::validate::{self, Severity, Target};
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

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

/// A short explanation of a build error, for the result page.
pub(super) fn summary(error: &str, stage: &str) -> String {
    let first = error.lines().find(|l| !l.trim().is_empty()).unwrap_or(error).trim();
    if first.starts_with("ffmpeg failed") {
        // The real reason is in ffmpeg's log (see Details).
        gettext("The encoder reported an error during this step: {}. See Details for more information.").replace("{}", stage)
    } else if error.lines().count() > 3 {
        format!("{first}\n{}", gettext("See Details for more information."))
    } else {
        error.trim().to_string()
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
                g.set_description(Some(&gettext("Builds the disc, burns it to a BD-R or BD-RE, and verifies the result.")));
                return;
            }
            let image = build::is_image(&output.borrow());
            folder_row.set_title(&if image { gettext("File") } else { gettext("Folder") });
            folder_row.set_subtitle(&output.borrow().to_string_lossy());
            g.set_description(Some(&if image {
                gettext("A single file that can be burned with any disc burning application or played in VLC or Kodi.")
            } else {
                gettext("A BDMV folder that plays in VLC or Kodi. Burning it requires a tool that writes UDF 2.50.")
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
        .tooltip_text(gettext("Set the video bitrate to fill a disc"))
        .popover(&gtk::Popover::builder().child(&fit_box).build())
        .build();
    size_row.add_suffix(&fit);
    g.add(&size_row);
    let time_row = adw::ActionRow::builder().title(gettext("Estimated Time")).subtitle_lines(2).build();
    g.add(&time_row);
    page.add(&g);

    // Encodes kept from earlier builds.
    let cache_size = crate::encode_cache::size();
    if cache_size > 0 {
        let cg = adw::PreferencesGroup::new();
        let row = adw::ActionRow::builder()
            .title(gettext("Encode Cache"))
            .subtitle(gettext("{} GB stored, so unchanged videos are not re-encoded when the disc is rebuilt").replace("{}", &format!("{:.1}", cache_size as f64 / 1e9)))
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
        let (doc, output) = (doc.clone(), output.clone());
        let (burning, options) = (burning.clone(), Rc::downgrade(&burn_options));
        let (dialog, parent) = (dialog.downgrade(), parent.as_ref().clone());
        let (size_row, time_row, fit_box, fit, build_btn, issues_group) =
            (size_row.clone(), time_row.clone(), fit_box.clone(), fit.clone(), build_btn.clone(), issues_group.clone());
        Rc::new(move || {
            let (issues, bytes, fits) = {
                let p = doc.project();
                let fits: Vec<(&'static str, u32)> = validate::DISCS
                    .iter()
                    .filter_map(|(name, cap)| Some((*name, validate::fit_bitrate(&p, *cap)?.min(validate::MAX_VIDEO_KBPS))))
                    .filter(|(_, kbps)| *kbps >= validate::min_video_kbps(&p))
                    .collect();
                (validate::check(&p), validate::estimated_bytes(&p), fits)
            };
            let disc = validate::disc_for(bytes).map_or_else(|| gettext("too large for any disc"), |(d, _)| d.to_string());
            size_row.set_subtitle(&format!("{:.1} GB · {disc}", bytes / 1e9));
            let (secs, measured) = build::estimate_seconds(&doc.project());
            let mut time = if secs < 60.0 { gettext("Under a minute") } else { gettext("About {}").replace("{}", &super::build_progress::duration_text(secs)) };
            if !measured && secs >= 60.0 {
                time.push_str(&format!(" · {}", gettext("a rough estimate until a build has been timed on this computer")));
            }
            if let Some(note) = super::jobs::queue_note() {
                time.push_str(&format!(" · {note}"));
            }
            time_row.set_subtitle(&time);

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
            // Burning also needs a disc that the build fits on, and two
            // builds can't write to the same place.
            let problem = if burning.get() {
                options.upgrade().and_then(|o| o.problem(bytes))
            } else {
                super::jobs::writes_to(&output.borrow()).then(|| gettext("Another job is already building to this location"))
            };
            build_btn.set_sensitive(errors == 0 && problem.is_none());
            build_btn.set_tooltip_text(problem.as_deref());
            build_btn.set_label(&match (burning.get(), super::jobs::building()) {
                (true, false) => gettext("_Build and Burn"),
                (false, false) => gettext("_Build"),
                (true, true) => gettext("Queue Build and _Burn"),
                (false, true) => gettext("Add to _Queue"),
            });
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
    let weak = Rc::downgrade(&refresh);
    super::jobs::connect(move || match weak.upgrade() {
        Some(r) => {
            r();
            true
        }
        None => false,
    });
    let again = Rc::downgrade(&refresh);
    dialog.connect_closed(move |_| {
        let _keep = &refresh;
    });

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
        let (describe, again) = (describe.clone(), again.clone());
        let done = move |res: Result<gio::File, glib::Error>| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                *out.borrow_mut() = with_image(&crate::media::portal::real_path(&p), image);
                describe();
                if let Some(r) = again.upgrade() {
                    r();
                }
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

    let doc = doc.clone();
    let d = dialog.clone();
    build_btn.connect_clicked(move |_| {
        let target = if burning.get() { burn_options.target() } else { None };
        let start = {
            let (doc, output, d) = (doc.clone(), output.clone(), d.clone());
            let target = target.clone();
            move || {
                let project = doc.project().clone();
                let name = if project.disc.name.is_empty() { gettext("Disc") } else { project.disc.name.clone() };
                let work = match target.clone() {
                    Some(target) => super::jobs::Work::BuildAndBurn { project, target },
                    None => {
                        let out = output.borrow().clone();
                        if let Some(settings) = crate::app_settings() {
                            let _ = settings.set_string("recent-output", &out.to_string_lossy());
                        }
                        super::jobs::Work::Build { project, out }
                    }
                };
                let job = super::jobs::add(work, name);
                let parent = d.root();
                d.close();
                if let Some(parent) = parent {
                    super::jobs::present(&job, &parent);
                }
            }
        };
        match &target {
            Some(t) => super::burn::confirm_erase(&d, t, start),
            None => start(),
        }
    });

    #[cfg(debug_assertions)]
    {
        if std::env::var("SPINDLE_SCREENSHOT_BURN").is_ok() {
            format_row.set_selected(2);
        }
        if std::env::var("SPINDLE_SCREENSHOT_BUILD_NOW").is_ok() {
            build_btn.emit_clicked();
        }
    }
    dialog.present(Some(parent));
}
