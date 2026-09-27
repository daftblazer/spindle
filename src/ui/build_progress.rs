// SPDX-License-Identifier: GPL-3.0-or-later

//! Progress of a build or burn: overall progress with the time left, and
//! each step (menus, every title, the disc image…) with its own.

use crate::build::{BuildEvent, TaskGroup, TaskState};
use adw::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Seconds of history used to measure speed.
const WINDOW: f64 = 90.0;

/// Estimates the time left from recent progress.
#[derive(Default)]
struct Rate {
    samples: VecDeque<(Instant, f64)>,
}

impl Rate {
    fn push(&mut self, fraction: f64) {
        let now = Instant::now();
        // Progress that went back (a retried step) starts over.
        if self.samples.back().is_some_and(|(_, f)| fraction < *f) {
            self.samples.clear();
        }
        self.samples.push_back((now, fraction));
        while self.samples.len() > 2 && self.samples.front().is_some_and(|(t, _)| now.duration_since(*t).as_secs_f64() > WINDOW) {
            self.samples.pop_front();
        }
    }

    /// Seconds left, once there is enough history to tell.
    fn remaining(&self) -> Option<f64> {
        let (t0, f0) = *self.samples.front()?;
        let (_, f1) = *self.samples.back()?;
        let dt = Instant::now().duration_since(t0).as_secs_f64();
        let df = f1 - f0;
        (dt >= 8.0 && df > 0.002 && f1 < 1.0).then(|| (1.0 - f1) * dt / df)
    }
}

/// "About 5 min left", "Less than a minute left".
pub fn time_left(secs: f64) -> String {
    if secs < 60.0 {
        return gettext("Less than a minute left");
    }
    gettext("About {} left").replace("{}", &duration_text(secs))
}

/// "1 h 5 min", "12 min", "40 s".
pub fn duration_text(secs: f64) -> String {
    let secs = secs.max(0.0).round() as u64;
    if secs < 60 {
        return format!("{secs} s");
    }
    let mins = (secs + 30) / 60;
    if mins < 60 {
        format!("{mins} min")
    } else {
        format!("{} h {} min", mins / 60, mins % 60)
    }
}

fn clock(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

struct TaskRow {
    key: String,
    name: String,
    group: TaskGroup,
    state: TaskState,
    detail: String,
    fraction: f64,
    rate: Rate,
    row: gtk::ListBoxRow,
    icon: gtk::Image,
    spinner: adw::Spinner,
    title: gtk::Label,
    subtitle: gtk::Label,
    status: gtk::Label,
    bar: gtk::ProgressBar,
}

impl TaskRow {
    fn new(key: &str, name: &str, group: TaskGroup) -> Self {
        let icon = gtk::Image::builder().pixel_size(16).build();
        let spinner = adw::Spinner::builder().width_request(16).height_request(16).visible(false).build();
        let lead = gtk::Box::builder().valign(gtk::Align::Center).width_request(20).build();
        lead.append(&icon);
        lead.append(&spinner);
        let title = gtk::Label::builder().label(name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let subtitle = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label"]).build();
        let status = gtk::Label::builder().xalign(1.0).valign(gtk::Align::Center).css_classes(["caption", "numeric", "dim-label"]).build();
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        text.append(&title);
        text.append(&subtitle);
        let top = gtk::Box::builder().spacing(12).build();
        top.append(&lead);
        top.append(&text);
        top.append(&status);
        let bar = gtk::ProgressBar::builder().margin_start(32).visible(false).css_classes(["task-bar"]).build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&top);
        content.append(&bar);
        let row = gtk::ListBoxRow::builder().child(&content).activatable(false).selectable(false).build();
        let r = TaskRow {
            key: key.into(),
            name: name.into(),
            group,
            state: TaskState::Waiting,
            detail: String::new(),
            fraction: 0.0,
            rate: Rate::default(),
            row,
            icon,
            spinner,
            title,
            subtitle,
            status,
            bar,
        };
        r.show();
        r
    }

    fn show(&self) {
        let running = self.state == TaskState::Running;
        self.spinner.set_visible(running);
        self.icon.set_visible(!running);
        self.bar.set_visible(running);
        self.bar.set_fraction(self.fraction);
        for c in ["success", "error", "dim-label"] {
            self.icon.remove_css_class(c);
        }
        let (icon, class) = match self.state {
            TaskState::Waiting => ("content-loading-symbolic", "dim-label"),
            TaskState::Running => ("content-loading-symbolic", "dim-label"),
            TaskState::Done => ("object-select-symbolic", "success"),
            TaskState::Failed => ("dialog-error-symbolic", "error"),
        };
        self.icon.set_icon_name(Some(icon));
        self.icon.add_css_class(class);
        if self.state == TaskState::Waiting {
            self.title.add_css_class("dim-label");
        } else {
            self.title.remove_css_class("dim-label");
        }
        let subtitle = match self.state {
            TaskState::Waiting if self.detail.is_empty() => gettext("Waiting"),
            _ => self.detail.clone(),
        };
        self.subtitle.set_label(&subtitle);
        self.subtitle.set_visible(!subtitle.is_empty());
        self.status.set_label(&match self.state {
            TaskState::Running => {
                let pct = format!("{:.0} %", self.fraction * 100.0);
                match self.rate.remaining() {
                    Some(s) => format!("{pct}\n{}", time_left(s)),
                    None => pct,
                }
            }
            _ => String::new(),
        });
    }
}

pub struct ProgressView {
    root: gtk::Box,
    headline: gtk::Label,
    bar: gtk::ProgressBar,
    percent: gtk::Label,
    remaining: gtk::Label,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    tasks: RefCell<Vec<TaskRow>>,
    log: gtk::TextView,
    cancel: gtk::Button,
    stage: RefCell<String>,
    fraction: RefCell<(f64, Rate)>,
    started: RefCell<Option<Instant>>,
}

impl ProgressView {
    pub fn new(title: &str) -> Rc<Self> {
        let heading = gtk::Label::builder().label(title).xalign(0.0).wrap(true).css_classes(["title-2"]).build();
        let headline = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label"]).build();
        let bar = gtk::ProgressBar::builder().margin_top(6).build();
        let percent = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["numeric", "caption"]).build();
        let remaining = gtk::Label::builder().xalign(1.0).css_classes(["numeric", "caption"]).build();
        let under = gtk::Box::builder().build();
        under.append(&percent);
        under.append(&remaining);

        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).valign(gtk::Align::Start).build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let log = gtk::TextView::builder()
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(6)
            .right_margin(6)
            .css_classes(["card"])
            .build();
        let log_scroller = gtk::ScrolledWindow::builder().min_content_height(140).child(&log).build();
        let expander = gtk::Expander::builder().label(gettext("Log")).child(&log_scroller).build();

        let cancel = gtk::Button::builder()
            .label(gettext("_Cancel"))
            .use_underline(true)
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        root.append(&heading);
        root.append(&headline);
        root.append(&bar);
        root.append(&under);
        let steps = gtk::Label::builder().label(gettext("Steps")).xalign(0.0).margin_top(12).css_classes(["heading"]).build();
        root.append(&steps);
        root.append(&scroller);
        root.append(&expander);
        cancel.set_margin_top(6);
        root.append(&cancel);

        let view = Rc::new(ProgressView {
            root,
            headline,
            bar,
            percent,
            remaining,
            list,
            scroller,
            tasks: RefCell::default(),
            log,
            cancel,
            stage: RefCell::default(),
            fraction: RefCell::new((0.0, Rate::default())),
            started: RefCell::default(),
        });
        // Times move on even without news from the build.
        let weak = Rc::downgrade(&view);
        glib::timeout_add_local(Duration::from_secs(1), move || match weak.upgrade() {
            Some(v) => {
                v.refresh();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn cancel_button(&self) -> &gtk::Button {
        &self.cancel
    }

    pub fn log_text(&self) -> String {
        let buf = self.log.buffer();
        buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string()
    }

    pub fn handle(&self, ev: &BuildEvent) {
        self.started.borrow_mut().get_or_insert_with(Instant::now);
        match ev {
            BuildEvent::Stage(s) => *self.stage.borrow_mut() = s.clone(),
            BuildEvent::Progress(p) => {
                let mut f = self.fraction.borrow_mut();
                f.0 = *p;
                f.1.push(*p);
                self.bar.set_fraction(*p);
            }
            BuildEvent::Log(l) => {
                let buf = self.log.buffer();
                let mut end = buf.end_iter();
                buf.insert(&mut end, &format!("{l}\n"));
            }
            BuildEvent::AddTask { key, name, group } => {
                let mut tasks = self.tasks.borrow_mut();
                if tasks.iter().any(|t| &t.key == key) {
                    return;
                }
                let row = TaskRow::new(key, name, *group);
                let pos = tasks.iter().position(|t| t.group > *group).unwrap_or(tasks.len());
                self.list.insert(&row.row, pos as i32);
                tasks.insert(pos, row);
            }
            BuildEvent::Task { key, state, detail, fraction } => {
                let mut tasks = self.tasks.borrow_mut();
                let Some(t) = tasks.iter_mut().find(|t| &t.key == key) else { return };
                if t.state != TaskState::Running && *state == TaskState::Running {
                    t.rate = Rate::default();
                    // Bring a step that starts into view.
                    let (row, list, adj) = (t.row.clone(), self.list.clone(), self.scroller.vadjustment());
                    glib::idle_add_local_once(move || {
                        if let Some(b) = row.compute_bounds(&list) {
                            adj.clamp_page(b.y() as f64, (b.y() + b.height()) as f64);
                        }
                    });
                }
                t.state = *state;
                t.detail = detail.clone();
                t.fraction = *fraction;
                if *state == TaskState::Running {
                    t.rate.push(*fraction);
                }
                t.show();
            }
            BuildEvent::Finished(_) => {}
        }
        self.show_headline();
    }

    fn show_headline(&self) {
        let tasks = self.tasks.borrow();
        let running: Vec<&TaskRow> = tasks.iter().filter(|t| t.state == TaskState::Running).collect();
        let text = match running.as_slice() {
            [] => self.stage.borrow().clone(),
            [t] => format!("{} · {}", t.name, t.detail),
            many if many.iter().all(|t| t.group == TaskGroup::Titles) => gettext("Encoding {} titles at once").replace("{}", &many.len().to_string()),
            many => many.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "),
        };
        self.headline.set_label(&text);
    }

    /// Update the times.
    fn refresh(&self) {
        let Some(start) = *self.started.borrow() else { return };
        let (fraction, remaining) = {
            let f = self.fraction.borrow();
            (f.0, f.1.remaining())
        };
        let elapsed = gettext("{} elapsed").replace("{}", &clock(start.elapsed().as_secs()));
        self.percent.set_label(&format!("{:.0} % · {elapsed}", fraction * 100.0));
        self.remaining.set_label(&remaining.map(time_left).unwrap_or_default());
        for t in self.tasks.borrow().iter().filter(|t| t.state == TaskState::Running) {
            t.show();
        }
    }

    /// Stop the clock (the build finished).
    pub fn finish(&self) {
        self.cancel.set_sensitive(false);
        self.refresh();
        *self.started.borrow_mut() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration_text(40.0), "40 s");
        assert_eq!(duration_text(12.0 * 60.0 + 10.0), "12 min");
        assert_eq!(duration_text(65.0 * 60.0), "1 h 5 min");
        assert_eq!(clock(3725), "1:02:05");
        assert_eq!(clock(125), "2:05");
    }
}
