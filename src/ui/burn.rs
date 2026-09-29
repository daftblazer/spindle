// SPDX-License-Identifier: GPL-3.0-or-later

//! Burning: choosing a drive (with the disc's state kept up to date) and
//! burning and verifying an image, for the Build dialog and for burning an
//! existing image.

use crate::build::{BuildEvent, TaskGroup, TaskState};
use crate::burn::{self, Disc, DiscState, Drive};
use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// What to burn to.
#[derive(Debug, Clone)]
pub struct Target {
    pub drive: PathBuf,
    pub verify: bool,
    pub eject: bool,
    /// A rewritable disc with data on it will be erased.
    pub erases: bool,
}

/// Drive, disc state, and options rows.
pub struct BurnOptions {
    drive_row: adw::ComboRow,
    status_row: adw::ActionRow,
    status_icon: gtk::Image,
    verify_row: adw::SwitchRow,
    eject_row: adw::SwitchRow,
    drives: RefCell<Vec<Drive>>,
    state: RefCell<Option<DiscState>>,
    checking: Cell<bool>,
    on_change: RefCell<Option<Rc<dyn Fn()>>>,
}

impl BurnOptions {
    pub fn new(group: &adw::PreferencesGroup) -> Rc<Self> {
        let drive_row = adw::ComboRow::builder().title(gettext("Drive")).build();
        let status_icon = gtk::Image::from_icon_name("media-optical-symbolic");
        let status_row = adw::ActionRow::builder().title(gettext("Disc")).subtitle(gettext("Checking…")).subtitle_lines(3).build();
        status_row.add_prefix(&status_icon);
        let verify_row = adw::SwitchRow::builder()
            .title(gettext("Verify After Burning"))
            .subtitle(gettext("Read the disc back and compare it with the image"))
            .active(true)
            .build();
        let eject_row = adw::SwitchRow::builder().title(gettext("Eject When Done")).active(true).build();
        for r in [drive_row.upcast_ref::<gtk::Widget>(), status_row.upcast_ref(), verify_row.upcast_ref(), eject_row.upcast_ref()] {
            group.add(r);
        }
        let this = Rc::new(BurnOptions {
            drive_row,
            status_row,
            status_icon,
            verify_row,
            eject_row,
            drives: RefCell::new(Vec::new()),
            state: RefCell::new(None),
            checking: Cell::new(false),
            on_change: RefCell::new(None),
        });
        this.fill_drives();
        let weak = Rc::downgrade(&this);
        this.drive_row.connect_selected_notify(move |_| {
            if let Some(t) = weak.upgrade() {
                *t.state.borrow_mut() = None;
                t.check();
            }
        });
        // Keep the disc state current (discs get inserted and ejected).
        let weak = Rc::downgrade(&this);
        glib::timeout_add_seconds_local(3, move || {
            let Some(t) = weak.upgrade() else { return glib::ControlFlow::Break };
            if t.drive_row.root().is_none() {
                return glib::ControlFlow::Break;
            }
            // Only while the options are on screen: not during a burn,
            // when the drive is busy (and asking it would only get in the way).
            if t.drive_row.is_visible() && t.drive_row.is_mapped() {
                t.check();
            }
            glib::ControlFlow::Continue
        });
        this.check();
        this
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_visible(self: &Rc<Self>, visible: bool) {
        for r in [self.drive_row.upcast_ref::<gtk::Widget>(), self.status_row.upcast_ref(), self.verify_row.upcast_ref(), self.eject_row.upcast_ref()] {
            r.set_visible(visible);
        }
        if visible {
            self.fill_drives();
            self.check();
        }
    }

    fn fill_drives(&self) {
        let found = burn::drives();
        if *self.drives.borrow() == found {
            return;
        }
        let labels: Vec<String> = found.iter().map(|d| format!("{} ({})", d.name, d.path.display())).collect();
        let strs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        self.drive_row.set_model(Some(&gtk::StringList::new(&strs)));
        self.drive_row.set_sensitive(!found.is_empty());
        *self.drives.borrow_mut() = found;
    }

    fn drive(&self) -> Option<PathBuf> {
        self.drives.borrow().get(self.drive_row.selected() as usize).map(|d| d.path.clone())
    }

    /// Look at the disc in the background.
    fn check(self: &Rc<Self>) {
        if self.checking.replace(true) {
            return;
        }
        let Some(drive) = self.drive() else {
            self.checking.set(false);
            self.show();
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let state = gio::spawn_blocking(move || burn::check(&drive)).await.ok();
            let Some(t) = weak.upgrade() else { return };
            t.checking.set(false);
            let changed = *t.state.borrow() != state;
            *t.state.borrow_mut() = state;
            if changed {
                t.show();
            }
        });
    }

    fn show(&self) {
        let (text, icon, class) = match (self.drive(), &*self.state.borrow()) {
            (None, _) => (gettext("No disc burner found"), "dialog-warning-symbolic", "warning"),
            (Some(_), None) => (gettext("Checking…"), "media-optical-symbolic", ""),
            (Some(_), Some(state)) => match describe(state) {
                (text, true) => (text, "object-select-symbolic", "success"),
                (text, false) => (text, "dialog-warning-symbolic", "warning"),
            },
        };
        self.status_row.set_subtitle(&text);
        self.status_icon.set_icon_name(Some(icon));
        let classes: Vec<&str> = if class.is_empty() { vec![] } else { vec![class] };
        self.status_icon.set_css_classes(&classes);
        let f = self.on_change.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    /// Why the disc can't take `bytes` (None when it can).
    pub fn problem(&self, bytes: f64) -> Option<String> {
        if self.drive().is_none() {
            return Some(gettext("Connect a Blu-ray burner"));
        }
        match &*self.state.borrow() {
            None => Some(gettext("Checking the disc…")),
            Some(DiscState::Ready(d)) if describe_disc(d).1 => (bytes > d.free as f64 && !d.rewritable).then(|| {
                gettext("The disc has {free} GB free; this disc needs {need} GB")
                    .replace("{free}", &format!("{:.1}", d.free as f64 / 1e9))
                    .replace("{need}", &format!("{:.1}", bytes / 1e9))
            }),
            Some(state) => Some(describe(state).0),
        }
    }

    pub fn target(&self) -> Option<Target> {
        let erases = matches!(&*self.state.borrow(), Some(DiscState::Ready(d)) if d.rewritable && !d.blank);
        Some(Target { drive: self.drive()?, verify: self.verify_row.is_active(), eject: self.eject_row.is_active(), erases })
    }
}

/// A line about the disc, and whether it can be burned.
fn describe(state: &DiscState) -> (String, bool) {
    match state {
        DiscState::NoDisc => (gettext("Insert a blank BD-R or a BD-RE"), false),
        DiscState::Busy => (gettext("The disc is in use (mounted or open in another app). Eject it, or close the app using it."), false),
        DiscState::Ready(d) => describe_disc(d),
    }
}

fn describe_disc(d: &Disc) -> (String, bool) {
    let gb = format!("{:.1}", d.free as f64 / 1e9);
    if !d.is_bluray() {
        return (gettext("This is a {kind}; Blu-ray discs need a BD-R or BD-RE").replace("{kind}", &d.kind), false);
    }
    if d.blank {
        return (gettext("Blank {kind} · {gb} GB free").replace("{kind}", &d.kind).replace("{gb}", &gb), true);
    }
    if d.rewritable {
        return (gettext("{kind} with data on it · it will be erased").replace("{kind}", &d.kind), true);
    }
    (gettext("This {kind} has already been written; insert a blank disc").replace("{kind}", &d.kind), false)
}

/// Burn `image` to `target` (then verify and eject), reporting progress
/// from `start` to 1.0. Runs on a worker thread.
/// List the burning steps (before the build, so they show from the start).
pub fn add_tasks(target: &Target, emit: &dyn Fn(BuildEvent)) {
    let name = if target.erases { gettext("Erase and Burn") } else { gettext("Burn") };
    emit(BuildEvent::AddTask { key: "burn".into(), name, group: TaskGroup::Burn });
    if target.verify {
        emit(BuildEvent::AddTask { key: "verify".into(), name: gettext("Verify"), group: TaskGroup::Burn });
    }
}

pub fn run(image: &Path, target: &Target, cancel: &AtomicBool, emit: &dyn Fn(BuildEvent), start: f64) -> anyhow::Result<()> {
    let task = |key: &str, state, detail: String, fraction| emit(BuildEvent::Task { key: key.into(), state, detail, fraction });
    let fail = |key: &str| {
        if !cancel.load(Ordering::Relaxed) {
            task(key, TaskState::Failed, gettext("Failed"), 0.0);
        }
    };
    let span = if target.verify { (1.0 - start) * 0.6 } else { 1.0 - start };
    emit(BuildEvent::Stage(if target.erases { gettext("Erasing and burning the disc") } else { gettext("Burning the disc") }));
    emit(BuildEvent::Log(format!("Burning {} to {}", image.display(), target.drive.display())));
    let size = std::fs::metadata(image).map_or(0, |m| m.len()) as f64;
    let writing = |f: f64| format!("{:.1} of {:.1} GB", f * size / 1e9, size / 1e9);
    task("burn", TaskState::Running, gettext("Starting"), 0.0);
    burn::burn(image, &target.drive, cancel, |f| {
        emit(BuildEvent::Progress(start + f * span));
        task("burn", TaskState::Running, writing(f), f);
    })
    .inspect_err(|_| fail("burn"))?;
    task("burn", TaskState::Done, format!("{:.1} GB", size / 1e9), 1.0);
    if target.verify {
        emit(BuildEvent::Stage(gettext("Verifying the disc")));
        let from = start + span;
        task("verify", TaskState::Running, gettext("Reading the disc back"), 0.0);
        burn::verify(image, &target.drive, cancel, |f| {
            emit(BuildEvent::Progress(from + f * (1.0 - from)));
            task("verify", TaskState::Running, gettext("Reading the disc back"), f);
        })
        .inspect_err(|_| fail("verify"))?;
        task("verify", TaskState::Done, gettext("The disc matches the image"), 1.0);
        emit(BuildEvent::Log("The disc matches the image".into()));
    }
    if target.eject {
        burn::eject(&target.drive);
    }
    emit(BuildEvent::Progress(1.0));
    Ok(())
}

/// Ask before erasing a rewritable disc; calls `go` when it's fine.
pub fn confirm_erase(parent: &impl IsA<gtk::Widget>, target: &Target, go: impl Fn() + 'static) {
    if !target.erases {
        go();
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Erase the Disc?"))
        .body(gettext("Everything on the rewritable disc will be erased before burning."))
        .build();
    dialog.add_responses(&[("cancel", &gettext("_Cancel")), ("erase", &gettext("_Erase and Burn"))]);
    dialog.set_response_appearance("erase", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.connect_response(None, move |_, r| {
        if r == "erase" {
            go();
        }
    });
    dialog.present(Some(parent));
}

/// Dialog to burn an existing disc image.
pub fn present_image(parent: &impl IsA<gtk::Widget>, image: PathBuf) {
    let dialog = adw::Dialog::builder().title(gettext("Burn Disc Image")).content_width(520).content_height(520).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::new();
    let size = std::fs::metadata(&image).map(|m| m.len()).unwrap_or(0) as f64;
    g.add(
        &adw::ActionRow::builder()
            .title(image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            .subtitle(format!("{:.1} GB", size / 1e9))
            .build(),
    );
    let options = BurnOptions::new(&g);
    page.add(&g);
    let bg = adw::PreferencesGroup::new();
    let burn_btn = gtk::Button::builder().label(gettext("_Burn")).use_underline(true).halign(gtk::Align::Center).css_classes(["suggested-action", "pill"]).build();
    bg.add(&burn_btn);
    page.add(&bg);
    stack.add_named(&page, Some("setup"));
    let (o, b) = (Rc::downgrade(&options), burn_btn.clone());
    let update = move || {
        if let Some(o) = o.upgrade() {
            let problem = o.problem(size);
            b.set_sensitive(problem.is_none());
            b.set_tooltip_text(problem.as_deref());
        }
    };
    update();
    options.connect_changed(update);

    let result = adw::StatusPage::new();
    result.add_css_class("compact");
    let close_btn = gtk::Button::builder().label(gettext("_Close")).use_underline(true).halign(gtk::Align::Center).css_classes(["pill"]).build();
    result.set_child(Some(&close_btn));
    stack.add_named(&result, Some("result"));

    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    dialog.connect_closed(move |_| c.store(true, Ordering::Relaxed));
    let d = dialog.clone();
    close_btn.connect_clicked(move |_| {
        d.close();
    });
    let d = dialog.clone();
    burn_btn.connect_clicked(move |_| {
        let Some(target) = options.target() else { return };
        let (stack, result, cancel, image) = (stack.clone(), result.clone(), cancel.clone(), image.clone());
        confirm_erase(&d, &target.clone(), move || {
            let name = image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let view = super::build_progress::ProgressView::new(&gettext("Burning “{}”").replace("{}", &name));
            let c = cancel.clone();
            view.cancel_button().connect_clicked(move |b| {
                c.store(true, Ordering::Relaxed);
                b.set_sensitive(false);
            });
            if let Some(old) = stack.child_by_name("progress") {
                stack.remove(&old);
            }
            stack.add_named(view.widget(), Some("progress"));
            stack.set_visible_child_name("progress");
            let (tx, rx) = async_channel::unbounded::<BuildEvent>();
            let (target, cancel, image) = (target.clone(), cancel.clone(), image.clone());
            std::thread::spawn(move || {
                let emit = |ev: BuildEvent| {
                    let _ = tx.send_blocking(ev);
                };
                add_tasks(&target, &emit);
                let res = run(&image, &target, &cancel, &emit, 0.0).map(|_| image.clone()).map_err(|e| format!("{e:#}"));
                emit(BuildEvent::Finished(res));
            });
            let (stack, result) = (stack.clone(), result.clone());
            glib::spawn_future_local(async move {
                while let Ok(ev) = rx.recv().await {
                    if let BuildEvent::Finished(res) = ev {
                        view.finish();
                        show_result(&result, res.map(|_| ()));
                        stack.set_visible_child_name("result");
                        break;
                    }
                    view.handle(&ev);
                }
            });
        });
    });
    dialog.present(Some(parent));
}

/// Fill the result page after burning.
pub fn show_result(result: &adw::StatusPage, res: Result<(), String>) {
    match res {
        Ok(()) => {
            result.set_icon_name(Some("object-select-symbolic"));
            result.set_title(&gettext("Disc Burned"));
            result.set_description(Some(&gettext("The disc has been written.")));
        }
        Err(e) => {
            let cancelled = e.contains("cancelled");
            result.set_icon_name(Some(if cancelled { "process-stop-symbolic" } else { "dialog-error-symbolic" }));
            result.set_title(&if cancelled { gettext("Burning Stopped") } else { gettext("Burning Failed") });
            result.set_description(Some(&glib::markup_escape_text(if cancelled {
                "A partly written BD-R can't be used again; a BD-RE can be erased and burned again."
            } else {
                &e
            })));
        }
    }
}
