// SPDX-License-Identifier: GPL-3.0-or-later

//! Background jobs: building and burning discs while the editor stays free
//! for the next project. Builds take turns on the processor, and burns take
//! turns on their drive, so an image can be burned while a disc is built.

use super::build_progress::{duration_text, time_left, ProgressView};
use super::burn::Target;
use crate::build::{self, BuildEvent};
use crate::burn::ReadScan;
use crate::model::Project;
use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// What a job does.
#[derive(Clone)]
pub enum Work {
    /// Build into `out`, an image or a folder.
    Build { project: Project, out: PathBuf },
    /// Build into a temporary image, then burn it.
    BuildAndBurn { project: Project, target: Target },
    /// Burn an existing image.
    Burn { image: PathBuf, target: Target },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// The part of a job that runs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Build,
    Burn,
}

pub struct Job {
    id: u64,
    /// The disc or image name.
    name: String,
    work: Work,
    status: Cell<Status>,
    step: Cell<Step>,
    view: Rc<ProgressView>,
    cancel: Arc<AtomicBool>,
    /// Burning may erase a rewritable disc that has data on it.
    erase_ok: Arc<AtomicBool>,
    result: RefCell<Option<Result<PathBuf, String>>>,
    /// The last stage reported, for explaining a failure.
    stage: RefCell<String>,
    scan: RefCell<Option<ReadScan>>,
    /// Why burning is waiting (for a disc, or to erase one).
    waiting: RefCell<Option<String>>,
    /// Keeps Spindle running while a step's thread is (so cancelling on
    /// close still stops ffmpeg and removes work files).
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    dialog: RefCell<glib::WeakRef<adw::Dialog>>,
}

impl Job {
    pub fn status(&self) -> Status {
        self.status.get()
    }

    fn finished(&self) -> bool {
        !matches!(self.status(), Status::Queued | Status::Running)
    }

    fn target(&self) -> Option<&Target> {
        match &self.work {
            Work::BuildAndBurn { target, .. } | Work::Burn { target, .. } => Some(target),
            Work::Build { .. } => None,
        }
    }

    fn burns(&self) -> bool {
        self.target().is_some()
    }

    /// Where a build writes, if it does.
    fn output(&self) -> Option<&Path> {
        match &self.work {
            Work::Build { out, .. } => Some(out),
            _ => None,
        }
    }

    /// The image a build and burn is built into.
    fn temp_image(&self) -> PathBuf {
        burn_dir().join(format!("job-{}.iso", self.id))
    }

    fn image(&self) -> PathBuf {
        match &self.work {
            Work::Burn { image, .. } => image.clone(),
            _ => self.temp_image(),
        }
    }

    fn title(&self) -> String {
        match &self.work {
            Work::Build { .. } => gettext("Building “{}”"),
            Work::BuildAndBurn { .. } => gettext("Building and Burning “{}”"),
            Work::Burn { .. } => gettext("Burning “{}”"),
        }
        .replace("{}", &self.name)
    }

    /// One line on how the job is going.
    fn summary(&self) -> String {
        match self.status() {
            Status::Queued if self.step.get() == Step::Burn => gettext("Waiting for the drive"),
            Status::Queued => gettext("Waiting for another build to finish"),
            Status::Running if self.waiting.borrow().is_some() => self.waiting.borrow().clone().unwrap_or_default(),
            Status::Running => {
                let pct = format!("{:.0} %", self.view.fraction() * 100.0);
                match self.view.remaining() {
                    Some(s) => format!("{pct} · {}", time_left(s)),
                    None => pct,
                }
            }
            Status::Done => match &self.work {
                Work::Build { out, .. } if build::is_image(out) => gettext("The disc image has been created"),
                Work::Build { .. } => gettext("The Blu-ray folder has been created"),
                _ => gettext("The disc has been burned"),
            },
            Status::Failed => match self.step.get() {
                Step::Build => gettext("Build failed"),
                Step::Burn => gettext("Burning failed"),
            },
            Status::Cancelled => gettext("Cancelled"),
        }
    }

    fn work_builds(&self) -> bool {
        !matches!(self.work, Work::Burn { .. })
    }

    /// Stop the job, or take it out of the queue.
    pub fn cancel(self: &Rc<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        if self.status() == Status::Queued {
            finish(self, Err("cancelled".into()));
        }
    }
}

thread_local! {
    static JOBS: RefCell<Vec<Rc<Job>>> = RefCell::default();
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    /// Called when jobs are added, start, finish or are removed; each
    /// returns false once it is no longer needed.
    static LISTENERS: RefCell<Vec<Box<dyn Fn() -> bool>>> = RefCell::default();
    /// Run once no step is running (quitting after cancelling the jobs).
    static WHEN_IDLE: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::default();
}

fn jobs() -> Vec<Rc<Job>> {
    JOBS.with_borrow(|j| j.clone())
}

fn burn_dir() -> PathBuf {
    crate::media::cache_dir().join("burn")
}

/// Call `f` whenever the jobs change, while it returns true.
pub fn connect(f: impl Fn() -> bool + 'static) {
    LISTENERS.with_borrow_mut(|l| l.push(Box::new(f)));
}

fn notify() {
    // Listeners may add listeners, so they are called outside the borrow.
    let listeners = LISTENERS.take();
    let mut kept: Vec<_> = listeners.into_iter().filter(|f| f()).collect();
    LISTENERS.with_borrow_mut(|l| {
        kept.append(l);
        *l = kept;
    });
}

/// Jobs that are running or waiting to.
pub fn active() -> usize {
    jobs().iter().filter(|j| !j.finished()).count()
}

/// Whether a new build would wait for one already queued or running.
pub fn building() -> bool {
    jobs().iter().any(|j| !j.finished() && j.work_builds() && j.step.get() == Step::Build)
}

/// Whether a job still to finish builds into `path`.
pub fn writes_to(path: &Path) -> bool {
    jobs().iter().any(|j| !j.finished() && j.output() == Some(path))
}

/// The name of the first job still to finish that burns to `drive`.
pub fn drive_user(drive: &Path) -> Option<String> {
    jobs().into_iter().find(|j| !j.finished() && j.target().is_some_and(|t| t.drive == drive)).map(|j| j.name.clone())
}

/// Queue `work`; `name` names the disc or image.
pub fn add(work: Work, name: String) -> Rc<Job> {
    // Images left by a build and burn that didn't finish (Spindle quit).
    if !jobs().iter().any(|j| !j.finished() && matches!(j.work, Work::BuildAndBurn { .. })) {
        let _ = std::fs::remove_dir_all(burn_dir());
    }
    let id = NEXT_ID.get();
    NEXT_ID.set(id + 1);
    let erases = match &work {
        Work::BuildAndBurn { target, .. } | Work::Burn { target, .. } => target.erases,
        Work::Build { .. } => false,
    };
    let job = Rc::new(Job {
        id,
        name,
        status: Cell::new(Status::Queued),
        step: Cell::new(if matches!(work, Work::Burn { .. }) { Step::Burn } else { Step::Build }),
        work,
        view: ProgressView::new(""),
        cancel: Arc::default(),
        erase_ok: Arc::new(AtomicBool::new(erases)),
        result: RefCell::default(),
        stage: RefCell::default(),
        scan: RefCell::default(),
        waiting: RefCell::default(),
        hold: RefCell::default(),
        dialog: RefCell::default(),
    });
    job.view.set_title(&job.title());
    {
        let weak = Rc::downgrade(&job);
        job.view.cancel_button().connect_clicked(move |b| {
            if let Some(j) = weak.upgrade() {
                b.set_sensitive(false);
                j.cancel();
            }
        });
    }
    // The burning steps show from the start.
    if let Some(t) = job.target() {
        super::burn::add_tasks(t, &|ev| job.view.handle(&ev));
    }
    JOBS.with_borrow_mut(|j| j.push(job.clone()));
    schedule();
    job
}

/// Start every queued step whose processor or drive is free. Builds go in
/// the order they were queued, and so do burns to each drive.
fn schedule() {
    let all = jobs();
    let mut cpu = all.iter().any(|j| j.status() == Status::Running && j.step.get() == Step::Build);
    // Drives a running job uses, or that an earlier queued job will burn to.
    let mut drives: HashSet<PathBuf> = HashSet::new();
    for j in all.iter().filter(|j| !j.finished()) {
        let drive = j.target().map(|t| t.drive.clone());
        if j.status() == Status::Queued {
            match j.step.get() {
                Step::Build if !cpu => {
                    cpu = true;
                    start(j);
                }
                Step::Burn if drive.as_ref().is_some_and(|d| !drives.contains(d)) => start(j),
                _ => {}
            }
        }
        drives.extend(drive);
    }
    for j in all.iter().filter(|j| j.status() == Status::Queued) {
        j.view.set_waiting(&j.summary());
    }
    notify();
}

fn start(job: &Rc<Job>) {
    job.status.set(Status::Running);
    *job.hold.borrow_mut() = gio::Application::default().map(|a| a.hold());
    let (tx, rx) = async_channel::unbounded::<BuildEvent>();
    let (work, step, cancel, erase_ok, image) = (job.work.clone(), job.step.get(), job.cancel.clone(), job.erase_ok.clone(), job.image());
    std::thread::spawn(move || {
        let emit = |ev: BuildEvent| {
            let _ = tx.send_blocking(ev);
        };
        let res = match (&work, step) {
            (Work::Build { project, out }, _) => build::build(project, out, &cancel, &emit),
            // The build takes the first 60% of the progress.
            (Work::BuildAndBurn { project, .. }, Step::Build) => (|| {
                std::fs::create_dir_all(burn_dir())?;
                let scaled = |ev: BuildEvent| match ev {
                    BuildEvent::Progress(p) => emit(BuildEvent::Progress(p * 0.6)),
                    ev => emit(ev),
                };
                build::build(project, &image, &cancel, &scaled)
            })(),
            (Work::BuildAndBurn { target, .. } | Work::Burn { target, .. }, Step::Burn) => {
                let from = if matches!(work, Work::Burn { .. }) { 0.0 } else { 0.6 };
                super::burn::run(&image, target, &cancel, &erase_ok, &emit, from).map(|_| target.drive.clone())
            }
            (Work::Burn { .. }, Step::Build) => unreachable!("burning an image has no build step"),
        }
        .map_err(|e| format!("{e:#}"));
        emit(BuildEvent::Finished(res));
    });
    let job = job.clone();
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                BuildEvent::Finished(res) => {
                    step_done(&job, res);
                    break;
                }
                BuildEvent::EraseDisc => ask_erase(&job),
                ev => {
                    match &ev {
                        BuildEvent::Stage(s) => *job.stage.borrow_mut() = s.clone(),
                        BuildEvent::Scan(s) => *job.scan.borrow_mut() = Some(s.clone()),
                        BuildEvent::Task { key, state, detail, .. } if key == "burn" => {
                            *job.waiting.borrow_mut() = (*state == crate::build::TaskState::Waiting).then(|| detail.clone());
                            notify();
                        }
                        _ => {}
                    }
                    job.view.handle(&ev);
                }
            }
        }
    });
    notify();
}

fn step_done(job: &Rc<Job>, res: Result<PathBuf, String>) {
    job.hold.take();
    if res.is_ok() && job.step.get() == Step::Build && job.burns() {
        // Built: wait for the drive.
        job.step.set(Step::Burn);
        job.status.set(Status::Queued);
        schedule();
    } else {
        finish(job, res);
    }
    if !jobs().iter().any(|j| j.status() == Status::Running) {
        if let Some(f) = WHEN_IDLE.take() {
            f();
        }
    }
}

fn finish(job: &Rc<Job>, res: Result<PathBuf, String>) {
    let cancelled = res.as_ref().is_err_and(|e| e.contains("cancelled"));
    job.status.set(match &res {
        Ok(_) => Status::Done,
        Err(_) if cancelled => Status::Cancelled,
        Err(_) => Status::Failed,
    });
    if matches!(job.work, Work::BuildAndBurn { .. }) {
        let _ = std::fs::remove_file(job.temp_image());
    }
    *job.result.borrow_mut() = Some(res);
    job.view.finish();
    if !cancelled {
        send_notification(job);
    }
    schedule();
}

/// Tell the user a job has finished when Spindle isn't in front.
fn send_notification(job: &Job) {
    let Some(app) = gio::Application::default().and_downcast::<gtk::Application>() else { return };
    if app.active_window().is_some_and(|w| w.is_active()) {
        return;
    }
    let ok = job.status() == Status::Done;
    let title = match (job.burns(), ok) {
        (false, true) => gettext("Disc Ready"),
        (false, false) => gettext("Build Failed"),
        (true, true) => gettext("Disc Burned"),
        (true, false) => gettext("Burning Failed"),
    };
    let n = gio::Notification::new(&title);
    n.set_body(Some(&job.name));
    app.send_notification(Some(&format!("job-{}", job.id)), &n);
}

/// The disc to burn to has data on it: ask before erasing it.
fn ask_erase(job: &Rc<Job>) {
    let Some(parent) = gio::Application::default().and_downcast::<gtk::Application>().and_then(|a| a.active_window()) else { return };
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Erase the Disc?"))
        .body(gettext("The rewritable disc in the drive has data on it. Erase it to burn “{}”, or insert a blank disc.").replace("{}", &job.name))
        .close_response("wait")
        .default_response("wait")
        .build();
    dialog.add_responses(&[("cancel", &gettext("Cancel _Job")), ("wait", &gettext("_Wait")), ("erase", &gettext("_Erase and Burn"))]);
    dialog.set_response_appearance("erase", adw::ResponseAppearance::Destructive);
    let job = job.clone();
    dialog.connect_response(None, move |_, r| match r {
        "erase" => job.erase_ok.store(true, Ordering::Relaxed),
        "cancel" => job.cancel(),
        _ => {}
    });
    dialog.present(Some(&parent));
}

/// Ask before closing Spindle cancels the jobs; `then` closes it.
pub fn confirm_close(parent: &impl IsA<gtk::Widget>, then: impl Fn() + 'static) {
    let n = active() as u32;
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Cancel the Jobs?"))
        .body(ngettext("A job is still running or waiting. Closing Spindle cancels it.", "{} jobs are still running or waiting. Closing Spindle cancels them.", n).replace("{}", &n.to_string()))
        .close_response("keep")
        .default_response("keep")
        .build();
    dialog.add_responses(&[("keep", &gettext("_Keep Open")), ("close", &gettext("_Cancel Jobs and Close"))]);
    dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, r| {
        if r == "close" {
            then();
        }
    });
    dialog.present(Some(parent));
}

/// Cancel every job, then call `then` once their work has stopped.
pub fn cancel_all(then: impl FnOnce() + 'static) {
    for j in jobs() {
        j.cancel();
    }
    if jobs().iter().any(|j| j.status() == Status::Running) {
        WHEN_IDLE.set(Some(Box::new(then)));
    } else {
        then();
    }
}

fn remove(job: &Job) {
    JOBS.with_borrow_mut(|j| j.retain(|j| j.id != job.id));
    notify();
}

/// Show a job's progress, or how it ended.
pub fn present(job: &Rc<Job>, parent: &impl IsA<gtk::Widget>) {
    if let Some(d) = job.dialog.borrow().upgrade() {
        d.close();
    }
    // A dialog that is still closing lets go of the progress view now.
    let view = job.view.widget().clone();
    if let Some(s) = view.parent().and_downcast::<gtk::Stack>() {
        s.remove(&view);
    }
    let dialog = adw::Dialog::builder().title(&job.name).content_width(560).content_height(620).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));
    stack.add_named(&view, Some("progress"));
    let show = {
        let (job, stack, dialog) = (job.clone(), stack.clone(), dialog.downgrade());
        move || {
            if !job.finished() || stack.child_by_name("result").is_some() {
                return;
            }
            let Some(d) = dialog.upgrade() else { return };
            stack.add_named(&result_page(&job, &d), Some("result"));
            stack.set_visible_child_name("result");
        }
    };
    show();
    let weak = dialog.downgrade();
    connect(move || {
        if weak.upgrade().is_none() {
            return false;
        }
        show();
        true
    });
    // The progress view is kept for showing again.
    let s = stack.clone();
    dialog.connect_closed(move |_| {
        if view.parent().as_ref() == Some(s.upcast_ref()) {
            s.remove(&view);
        }
    });
    *job.dialog.borrow_mut() = dialog.downgrade();
    dialog.present(Some(parent));
}

/// How the job ended, with what to do next.
fn result_page(job: &Rc<Job>, dialog: &adw::Dialog) -> gtk::Widget {
    let result = adw::StatusPage::new();
    result.add_css_class("compact");
    let rcol = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(24).margin_end(24).build();
    let res = job.result.borrow().clone().unwrap_or_else(|| Err("cancelled".into()));
    let stage = job.stage.borrow().clone();
    let cancelled = res.as_ref().is_err_and(|e| e.contains("cancelled"));
    // On failure: the whole error and the log.
    let details = match &res {
        Err(e) if !cancelled => Some(format!("Spindle {}\nFailed while: {stage}\n\n{e}\n\n--- Build log ---\n{}", crate::config::VERSION, job.view.log_text())),
        _ => None,
    };
    if let Some(text) = &details {
        let details_view = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).build();
        details_view.buffer().set_text(text);
        rcol.append(&gtk::Expander::builder().label(gettext("Details")).child(&gtk::ScrolledWindow::builder().min_content_height(160).child(&details_view).build()).build());
    }
    // After burning with Verify: how the disc read back.
    let graph = adw::Bin::new();
    rcol.append(&graph);
    let rbox = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    if job.burns() {
        let res = res.clone().map(|_| ()).map_err(|e| if matches!(job.work, Work::BuildAndBurn { .. }) { super::build_dialog::summary(&e, &stage) } else { e });
        super::burn::show_result(&result, &graph, res, job.scan.borrow().as_ref());
    } else {
        match &res {
            Ok(path) => {
                result.set_icon_name(Some("object-select-symbolic"));
                result.set_title(&gettext("Disc Ready"));
                result.set_description(Some(&format!(
                    "{}\n<small>{}</small>",
                    if build::is_image(path) { gettext("The disc image has been created.") } else { gettext("The Blu-ray folder has been created.") },
                    glib::markup_escape_text(&path.to_string_lossy())
                )));
                let open_btn = gtk::Button::builder().label(gettext("_Open Folder")).use_underline(true).css_classes(["pill", "suggested-action"]).build();
                let (path, d) = (path.clone(), dialog.downgrade());
                open_btn.connect_clicked(move |_| {
                    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&path)));
                    let root = d.upgrade().and_then(|d| d.root()).and_downcast::<gtk::Window>();
                    if build::is_image(&path) {
                        launcher.open_containing_folder(root.as_ref(), gio::Cancellable::NONE, |_| {});
                    } else {
                        launcher.launch(root.as_ref(), gio::Cancellable::NONE, |_| {});
                    }
                });
                rbox.append(&open_btn);
            }
            Err(e) => {
                result.set_icon_name(Some(if cancelled { "process-stop-symbolic" } else { "dialog-error-symbolic" }));
                result.set_title(&if cancelled { gettext("Build Cancelled") } else { gettext("Build Failed") });
                result.set_description(Some(&glib::markup_escape_text(&if cancelled { String::new() } else { super::build_dialog::summary(e, &stage) })));
            }
        }
    }
    if let Some(text) = details {
        let copy_btn = gtk::Button::builder().label(gettext("Copy _Details")).use_underline(true).css_classes(["pill"]).build();
        copy_btn.connect_clicked(move |b| {
            b.clipboard().set_text(&text);
            b.set_label(&gettext("Copied"));
        });
        rbox.append(&copy_btn);
    }
    let close_btn = gtk::Button::builder().label(gettext("_Close")).use_underline(true).css_classes(["pill"]).build();
    let d = dialog.downgrade();
    close_btn.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    rbox.append(&close_btn);
    rcol.append(&rbox);
    result.set_child(Some(&rcol));
    result.upcast()
}

/// Set up `button`, in a window's header bar: shown while there are jobs,
/// with their overall progress, and listing them in its popover.
pub fn setup_button(button: &gtk::MenuButton) {
    let pie = gtk::DrawingArea::builder().content_width(16).content_height(16).build();
    pie.set_draw_func(|area, cr, w, h| {
        let running: Vec<f64> = jobs().iter().filter(|j| j.status() == Status::Running).map(|j| j.view.fraction()).collect();
        let fraction = if running.is_empty() { 0.0 } else { running.iter().sum::<f64>() / running.len() as f64 };
        let c = area.color();
        let (x, y, r) = (w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0 - 1.0);
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.25);
        cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 1.0);
        let start = -std::f64::consts::FRAC_PI_2;
        cr.move_to(x, y);
        cr.arc(x, y, r, start, start + fraction.max(0.02) * std::f64::consts::TAU);
        cr.close_path();
        let _ = cr.fill();
    });
    let icon = gtk::Image::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&pie);
    content.append(&icon);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(&gettext("Jobs")));

    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
    let clear = gtk::Button::builder().label(gettext("_Clear Finished")).use_underline(true).halign(gtk::Align::Center).css_classes(["flat"]).build();
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).width_request(340).build();
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&list).build();
    col.append(&scroller);
    col.append(&clear);
    let popover = gtk::Popover::builder().child(&col).build();
    button.set_popover(Some(&popover));
    let p = popover.downgrade();
    list.connect_row_activated(move |list, row| {
        let job = jobs().into_iter().rev().nth(row.index() as usize);
        if let (Some(job), Some(p), Some(root)) = (job, p.upgrade(), list.root()) {
            p.popdown();
            present(&job, &root);
        }
    });
    clear.connect_clicked(|_| {
        JOBS.with_borrow_mut(|j| j.retain(|j| !j.finished()));
        notify();
    });

    let update = {
        let (button, pie, icon, list, clear, popover) = (button.downgrade(), pie.downgrade(), icon.downgrade(), list.downgrade(), clear.downgrade(), popover.downgrade());
        let scroller = scroller.downgrade();
        move || -> bool {
            let (Some(button), Some(pie), Some(icon), Some(list), Some(clear), Some(popover), Some(scroller)) =
                (button.upgrade(), pie.upgrade(), icon.upgrade(), list.upgrade(), clear.upgrade(), popover.upgrade(), scroller.upgrade())
            else {
                return false;
            };
            let all = jobs();
            button.set_visible(!all.is_empty());
            let running = all.iter().any(|j| j.status() == Status::Running);
            pie.set_visible(running);
            icon.set_visible(!running);
            let failed = all.iter().any(|j| j.status() == Status::Failed);
            icon.set_icon_name(Some(if failed {
                "dialog-error-symbolic"
            } else if all.iter().any(|j| j.status() == Status::Queued) {
                "content-loading-symbolic"
            } else {
                "object-select-symbolic"
            }));
            pie.queue_draw();
            clear.set_visible(all.iter().any(|j| j.finished()));
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            for job in all.iter().rev() {
                list.append(&job_row(job));
            }
            // As tall as the jobs, scrolling past a point.
            let (_, natural, _, _) = list.measure(gtk::Orientation::Vertical, 340);
            scroller.set_min_content_height(natural.min(480));
            if all.is_empty() {
                popover.popdown();
            }
            true
        }
    };
    update();
    connect(update);
    // Progress moves on between job changes.
    let pie = pie.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || match pie.upgrade() {
        Some(p) => {
            if p.is_visible() {
                p.queue_draw();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
}

fn job_row(job: &Rc<Job>) -> gtk::ListBoxRow {
    let title = gtk::Label::builder().label(job.title()).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
    let subtitle = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label", "numeric"]).build();
    let bar = gtk::ProgressBar::new();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).valign(gtk::Align::Center).build();
    text.append(&title);
    text.append(&subtitle);
    text.append(&bar);
    let top = gtk::Box::builder().spacing(12).margin_top(8).margin_bottom(8).margin_start(12).margin_end(6).build();
    let icon = match job.status() {
        Status::Done => Some(("object-select-symbolic", "success")),
        Status::Failed => Some(("dialog-error-symbolic", "error")),
        Status::Cancelled => Some(("process-stop-symbolic", "dim-label")),
        _ => None,
    };
    if let Some((name, class)) = icon {
        top.append(&gtk::Image::builder().icon_name(name).css_classes([class]).build());
    }
    top.append(&text);
    // Queued jobs can be taken out, and finished ones cleared. A running
    // job is cancelled from its progress, so it isn't stopped by a slip.
    if job.status() != Status::Running {
        let queued = job.status() == Status::Queued;
        let b = gtk::Button::builder()
            .icon_name(if queued { "process-stop-symbolic" } else { "window-close-symbolic" })
            .tooltip_text(if queued { gettext("Cancel") } else { gettext("Remove") })
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        let job = job.clone();
        b.connect_clicked(move |_| if queued { job.cancel() } else { remove(&job) });
        top.append(&b);
    }
    let row = gtk::ListBoxRow::builder().child(&top).activatable(true).build();
    let show = {
        let (job, subtitle, bar) = (job.clone(), subtitle.downgrade(), bar.downgrade());
        move || -> bool {
            let (Some(subtitle), Some(bar)) = (subtitle.upgrade(), bar.upgrade()) else { return false };
            subtitle.set_label(&job.summary());
            bar.set_visible(job.status() == Status::Running && job.waiting.borrow().is_none());
            bar.set_fraction(job.view.fraction());
            true
        }
    };
    show();
    glib::timeout_add_local(Duration::from_secs(1), move || if show() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    row
}

/// "Starts after 1 h 5 min of queued work", for the Build dialog.
pub fn queue_note() -> Option<String> {
    let left: f64 = jobs()
        .iter()
        .filter(|j| !j.finished() && j.work_builds() && j.step.get() == Step::Build)
        .map(|j| match (&j.work, j.status()) {
            (_, Status::Running) => j.view.remaining().unwrap_or(0.0),
            (Work::Build { project, .. } | Work::BuildAndBurn { project, .. }, _) => build::estimate_seconds(project).0,
            _ => 0.0,
        })
        .sum();
    building().then(|| gettext("starts after the builds already queued (about {})").replace("{}", &duration_text(left)))
}
