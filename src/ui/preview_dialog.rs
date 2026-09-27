// SPDX-License-Identifier: GPL-3.0-or-later

//! "Preview" dialog: encode a short clip of a title exactly like the disc
//! and open it in the video player.

use super::rows::format_time;
use crate::build::BuildEvent;
use crate::document::Document;
use crate::model::{codec_label, language_name, Id, SubtitleKind};
use crate::preview::{self, PreviewRequest};
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const LENGTHS: [f64; 4] = [15.0, 30.0, 60.0, 120.0];

fn open_file(path: &std::path::Path, parent: &impl IsA<gtk::Widget>) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    let root = parent.root().and_downcast::<gtk::Window>();
    launcher.launch(root.as_ref(), gio::Cancellable::NONE, |_| {});
}

pub fn present(doc: &Rc<Document>, title: Id, start: f64, parent: &impl IsA<gtk::Widget>) {
    let p = doc.project();
    let Some(t) = p.title(title) else { return };
    let Some(asset) = p.asset(t.asset) else { return };
    let duration = asset.info.duration.max(1.0);

    let dialog = adw::Dialog::builder().title(gettext("Preview")).content_width(480).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    // --- setup
    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::builder()
        .title(glib::markup_escape_text(&t.name).as_str())
        .description(format!(
            "{} · {} {}",
            gettext("Encoded exactly like the disc"),
            p.disc.video.label(),
            gettext("at {} Mbit/s").replace("{}", &format!("{:.0}", p.disc.video_bitrate as f64 / 1000.0))
        ))
        .build();
    let start_row = adw::SpinRow::with_range(0.0, (duration - 1.0).max(0.0), 1.0);
    start_row.set_title(&gettext("Start"));
    start_row.set_digits(1);
    start_row.set_value(start.clamp(0.0, (duration - 1.0).max(0.0)));
    start_row.set_subtitle(&format_time(start_row.value()));
    start_row.connect_value_notify(|r| r.set_subtitle(&format_time(r.value())));
    g.add(&start_row);

    let length_labels: Vec<String> = vec![gettext("15 seconds"), gettext("30 seconds"), gettext("1 minute"), gettext("2 minutes")];
    let strs: Vec<&str> = length_labels.iter().map(|s| s.as_str()).collect();
    let length_row = adw::ComboRow::builder().title(gettext("Length")).model(&gtk::StringList::new(&strs)).selected(1).build();
    g.add(&length_row);

    let tracks: Vec<(Option<Id>, String)> = std::iter::once((None, gettext("Off")))
        .chain(t.subtitles.iter().filter(|s| s.kind() != SubtitleKind::Unsupported).map(|s| {
            (Some(s.id), format!("{} ({}, {})", s.name, language_name(&s.lang), codec_label(&s.codec)))
        }))
        .collect();
    let preferred = t.default_subtitle.or_else(|| t.disc_subtitles().next().map(|s| s.id));
    let sel = tracks.iter().position(|(id, _)| *id == preferred).unwrap_or(0);
    let strs: Vec<&str> = tracks.iter().map(|(_, l)| l.as_str()).collect();
    let sub_row = adw::ComboRow::builder()
        .title(gettext("Subtitles"))
        .model(&gtk::StringList::new(&strs))
        .selected(sel as u32)
        .build();
    if tracks.len() == 1 {
        sub_row.set_sensitive(false);
    }
    g.add(&sub_row);
    page.add(&g);

    let bg = adw::PreferencesGroup::new();
    let go = gtk::Button::builder()
        .label(gettext("_Encode Preview"))
        .use_underline(true)
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    bg.add(&go);
    page.add(&bg);
    stack.add_named(&page, Some("setup"));
    drop(p);

    // --- progress
    let status = adw::StatusPage::builder().icon_name("video-x-generic-symbolic").title(gettext("Encoding Preview…")).build();
    status.add_css_class("compact");
    let pbox = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(24).margin_end(24).build();
    let bar = gtk::ProgressBar::new();
    pbox.append(&bar);
    let cancel_btn = gtk::Button::builder().label(gettext("_Cancel")).use_underline(true).halign(gtk::Align::Center).css_classes(["pill"]).build();
    pbox.append(&cancel_btn);
    status.set_child(Some(&pbox));
    stack.add_named(&status, Some("progress"));

    // --- result
    let result = adw::StatusPage::new();
    result.add_css_class("compact");
    let rbox = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    let play = gtk::Button::builder().label(gettext("_Play Again")).use_underline(true).css_classes(["pill", "suggested-action"]).build();
    let folder = gtk::Button::builder().label(gettext("Show in _Folder")).use_underline(true).css_classes(["pill"]).build();
    let again = gtk::Button::builder().label(gettext("_Another")).use_underline(true).css_classes(["pill"]).build();
    rbox.append(&play);
    rbox.append(&folder);
    rbox.append(&again);
    result.set_child(Some(&rbox));
    stack.add_named(&result, Some("result"));

    let output: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let cancel = Arc::new(AtomicBool::new(false));

    let c = cancel.clone();
    cancel_btn.connect_clicked(move |_| c.store(true, Ordering::Relaxed));
    let c = cancel.clone();
    dialog.connect_closed(move |_| c.store(true, Ordering::Relaxed));
    let (o, d) = (output.clone(), dialog.clone());
    play.connect_clicked(move |_| {
        if let Some(p) = o.borrow().as_ref() {
            open_file(p, &d);
        }
    });
    let (o, d) = (output.clone(), dialog.clone());
    folder.connect_clicked(move |_| {
        if let Some(p) = o.borrow().as_ref() {
            let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(p)));
            let root = d.root().and_downcast::<gtk::Window>();
            launcher.open_containing_folder(root.as_ref(), gio::Cancellable::NONE, |_| {});
        }
    });
    let s = stack.clone();
    again.connect_clicked(move |_| s.set_visible_child_name("setup"));

    let doc = doc.clone();
    let d = dialog.clone();
    go.connect_clicked(move |_| {
        cancel.store(false, Ordering::Relaxed);
        stack.set_visible_child_name("progress");
        bar.set_fraction(0.0);
        let req = PreviewRequest {
            title,
            start: start_row.value(),
            duration: LENGTHS[(length_row.selected() as usize).min(LENGTHS.len() - 1)],
            subtitle: tracks.get(sub_row.selected() as usize).and_then(|(id, _)| *id),
        };
        let project = doc.project().clone();
        let cancel = cancel.clone();
        let (tx, rx) = async_channel::unbounded::<BuildEvent>();
        std::thread::spawn(move || {
            let emit = |ev: BuildEvent| {
                let _ = tx.send_blocking(ev);
            };
            let res = preview::encode(&project, &req, &cancel, &emit).map_err(|e| format!("{e:#}"));
            emit(BuildEvent::Finished(res));
        });
        let (status, bar, result, stack, output, d, play) =
            (status.clone(), bar.clone(), result.clone(), stack.clone(), output.clone(), d.clone(), play.clone());
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                match ev {
                    BuildEvent::Stage(s) => status.set_description(Some(&glib::markup_escape_text(&s))),
                    BuildEvent::Progress(p) => bar.set_fraction(p),
                    BuildEvent::Log(_) | BuildEvent::AddTask { .. } | BuildEvent::Task { .. } => {}
                    BuildEvent::Finished(res) => {
                        match res {
                            Ok(path) => {
                                result.set_icon_name(Some("media-playback-start-symbolic"));
                                result.set_title(&gettext("Preview Ready"));
                                result.set_description(Some(&gettext("Opened in your video player. Subtitles are on by default.")));
                                play.set_visible(true);
                                open_file(&path, &d);
                                *output.borrow_mut() = Some(path);
                            }
                            Err(e) => {
                                let cancelled = e.contains("cancelled");
                                result.set_icon_name(Some(if cancelled { "process-stop-symbolic" } else { "dialog-error-symbolic" }));
                                result.set_title(&if cancelled { gettext("Preview Cancelled") } else { gettext("Preview Failed") });
                                result.set_description(Some(&glib::markup_escape_text(if cancelled { "" } else { &e })));
                                play.set_visible(false);
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
