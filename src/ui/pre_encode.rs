// SPDX-License-Identifier: GPL-3.0-or-later

//! "Encode Titles" dialog: encode the titles ahead of building the disc,
//! at constant quality, so every title looks as good as the others and
//! the build only has to put the disc together.

use crate::build::{self, BuildEvent};
use crate::document::{Change, Document};
use crate::media::transcode::Quality;
use crate::validate;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::glib;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// "3 of 12 titles encoded · 6.1 GB" for the encodes made ahead.
pub fn status(p: &crate::model::Project) -> String {
    let files = build::pre_encodes(p);
    let wanted = p.titles.iter().filter(|t| !t.keep_video).count();
    let sizes: Vec<u64> = files.iter().flatten().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).collect();
    if sizes.is_empty() {
        return ngettext("{} title to encode", "{} titles to encode", wanted as u32).replace("{}", &wanted.to_string());
    }
    let text = gettext("{done} of {all} encoded · {size} GB");
    text.replace("{done}", &sizes.len().to_string())
        .replace("{all}", &wanted.to_string())
        .replace("{size}", &format!("{:.1}", sizes.iter().sum::<u64>() as f64 / 1e9))
}

pub fn present(doc: &Rc<Document>, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::Dialog::builder().title(gettext("Encode Titles")).content_width(560).content_height(620).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    // --- setup page
    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::builder()
        .title(gettext("Video"))
        .description(gettext(
            "Encodes the titles now, each to the same picture quality rather than the same size. Building the disc then uses them as they are.",
        ))
        .build();
    let d = doc.project().disc.clone();
    let crf = super::rows::spin(doc, &gettext("CRF"), d.crf as f64, 12.0, 28.0, 1.0, 0, Change::Content, |p, v| p.disc.crf = v as u8);
    crf.set_subtitle(&gettext("Constant quality: lower looks better and takes more room. 18 is hard to tell from the original, 23 is much smaller"));
    crf.set_subtitle_lines(3);
    g.add(&crf);
    let labels = [gettext("Fast"), gettext("Balanced"), gettext("Best")];
    let explain = |q: Quality| match q {
        Quality::Fast => gettext("Quick to encode, but larger files for the same quality"),
        Quality::Balanced => gettext("A reasonable time and size"),
        Quality::Best => gettext("The smallest files for the quality; takes about twice as long"),
    };
    let sel = Quality::ALL.iter().position(|q| *q == d.quality).unwrap_or(1);
    let speed = super::rows::combo(doc, &gettext("Speed"), &labels, sel, Change::Content, |p, i| {
        p.disc.quality = Quality::ALL[i.min(Quality::ALL.len() - 1)];
    });
    speed.set_subtitle(&explain(d.quality));
    speed.set_subtitle_lines(2);
    speed.connect_selected_notify(move |r| r.set_subtitle(&explain(Quality::ALL[(r.selected() as usize).min(2)])));
    g.add(&speed);
    super::settings::tune_row(doc, &g, d.tune);
    page.add(&g);

    let sg = adw::PreferencesGroup::new();
    let titles_row = adw::ActionRow::builder().title(gettext("Titles")).subtitle_lines(2).build();
    sg.add(&titles_row);
    let time_row = adw::ActionRow::builder().title(gettext("Estimated Time")).subtitle_lines(2).build();
    sg.add(&time_row);
    page.add(&sg);

    let bg = adw::PreferencesGroup::new();
    let encode_btn = gtk::Button::builder()
        .label(gettext("_Encode"))
        .use_underline(true)
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    bg.add(&encode_btn);
    page.add(&bg);
    stack.add_named(&page, Some("setup"));

    // What there is to do follows the settings.
    let refresh: Rc<dyn Fn()> = {
        let (doc, titles_row, time_row, encode_btn) = (doc.clone(), titles_row.clone(), time_row.clone(), encode_btn.clone());
        Rc::new(move || {
            let p = doc.project();
            titles_row.set_subtitle(&status(&p));
            let (secs, measured) = build::pre_encode_seconds(&p);
            let mut time = if secs <= 0.0 {
                gettext("Nothing left to encode")
            } else if secs < 60.0 {
                gettext("Under a minute")
            } else {
                gettext("About {}").replace("{}", &super::build_progress::duration_text(secs))
            };
            if !measured && secs >= 60.0 {
                time.push_str(&format!(" · {}", gettext("a rough guess until Spindle has timed an encode on this computer")));
            }
            time_row.set_subtitle(&time);
            encode_btn.set_sensitive(secs > 0.0);
        })
    };
    refresh();
    // The document outlives the dialog, so it only holds a weak reference.
    let weak = Rc::downgrade(&refresh);
    doc.connect(move |c| {
        if let (Some(r), true) = (weak.upgrade(), matches!(c, Change::Content | Change::Structure)) {
            r();
        }
    });
    {
        let refresh = refresh.clone();
        dialog.connect_closed(move |_| {
            let _keep = &refresh;
        });
    }

    // --- result page
    let result = adw::StatusPage::new();
    result.add_css_class("compact");
    let rcol = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(24).margin_end(24).build();
    let details_view = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).build();
    let details = gtk::Expander::builder()
        .label(gettext("Details"))
        .child(&gtk::ScrolledWindow::builder().min_content_height(160).child(&details_view).build())
        .visible(false)
        .build();
    rcol.append(&details);
    let rbox = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    let build_btn = gtk::Button::builder().label(gettext("_Build Disc…")).use_underline(true).css_classes(["pill", "suggested-action"]).build();
    let close_btn = gtk::Button::builder().label(gettext("_Close")).use_underline(true).css_classes(["pill"]).build();
    rbox.append(&build_btn);
    rbox.append(&close_btn);
    rcol.append(&rbox);
    result.set_child(Some(&rcol));
    stack.add_named(&result, Some("result"));
    {
        let d = dialog.clone();
        close_btn.connect_clicked(move |_| {
            d.close();
        });
        let (d, parent) = (dialog.clone(), parent.as_ref().clone());
        build_btn.connect_clicked(move |_| {
            d.close();
            let _ = WidgetExt::activate_action(&parent, "win.build", None);
        });
    }

    // --- behaviour
    let cancel = Arc::new(AtomicBool::new(false));
    {
        // Closing the dialog stops the encode.
        let c = cancel.clone();
        dialog.connect_closed(move |_| c.store(true, Ordering::Relaxed));
    }
    let doc = doc.clone();
    encode_btn.connect_clicked(move |_| {
        let project = doc.project().clone();
        let view = super::build_progress::ProgressView::new(&gettext("Encoding Titles"));
        cancel.store(false, Ordering::Relaxed);
        {
            let c = cancel.clone();
            view.cancel_button().connect_clicked(move |b| {
                c.store(true, Ordering::Relaxed);
                b.set_sensitive(false);
            });
        }
        if let Some(old) = stack.child_by_name("progress") {
            stack.remove(&old);
        }
        stack.add_named(view.widget(), Some("progress"));
        stack.set_visible_child_name("progress");
        let (tx, rx) = async_channel::unbounded::<BuildEvent>();
        let c = cancel.clone();
        std::thread::spawn(move || {
            let emit = |ev: BuildEvent| {
                let _ = tx.send_blocking(ev);
            };
            let res = build::pre_encode(&project, &c, &emit).map(|_| std::path::PathBuf::new()).map_err(|e| format!("{e:#}"));
            emit(BuildEvent::Finished(res));
        });
        let (doc, result, stack, build_btn) = (doc.clone(), result.clone(), stack.clone(), build_btn.clone());
        let (details, details_view) = (details.clone(), details_view.clone());
        glib::spawn_future_local(async move {
            let mut stage = String::new();
            while let Ok(ev) = rx.recv().await {
                let BuildEvent::Finished(res) = ev else {
                    if let BuildEvent::Stage(s) = &ev {
                        stage = s.clone();
                    }
                    view.handle(&ev);
                    continue;
                };
                view.finish();
                build_btn.set_visible(res.is_ok());
                match res {
                    Ok(_) => {
                        let p = doc.project();
                        let bytes = validate::estimated_bytes(&p);
                        let fits = match validate::disc_for(bytes) {
                            Some((disc, _)) => gettext("With its menus the disc is about {size} GB, which fits a {disc}.").replace("{disc}", disc),
                            None => gettext("With its menus the disc is about {size} GB, too big for any disc: encode at a higher CRF."),
                        };
                        result.set_icon_name(Some("object-select-symbolic"));
                        result.set_title(&gettext("Titles Encoded"));
                        result.set_description(Some(&format!("{}\n{}", status(&p), fits.replace("{size}", &format!("{:.1}", bytes / 1e9)))));
                    }
                    Err(e) => {
                        let cancelled = e.contains("cancelled");
                        result.set_icon_name(Some(if cancelled { "process-stop-symbolic" } else { "dialog-error-symbolic" }));
                        result.set_title(&if cancelled { gettext("Encoding Cancelled") } else { gettext("Encoding Failed") });
                        let text = if cancelled { gettext("Titles that finished are kept.") } else { super::build_dialog::summary(&e, &stage) };
                        result.set_description(Some(&glib::markup_escape_text(&text)));
                        if !cancelled {
                            details_view.buffer().set_text(&format!(
                                "Spindle {}\nFailed while: {stage}\n\n{e}\n\n--- Log ---\n{}",
                                crate::config::VERSION,
                                view.log_text()
                            ));
                            details.set_visible(true);
                        }
                    }
                }
                stack.set_visible_child_name("result");
                break;
            }
        });
    });
    dialog.present(Some(parent));
}
