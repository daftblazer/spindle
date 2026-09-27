/* main.rs
 *
 * Copyright 2026 daftblazer
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

mod application;
mod bluray;
mod build;
mod config;
mod document;
mod media;
mod model;
mod preview;
mod render;
mod subtitles;
mod templates;
mod ui;
mod validate;
mod window;

use self::application::SpindleApplication;
use self::window::SpindleWindow;

use config::{GETTEXT_PACKAGE, LOCALEDIR, PKGDATADIR};
use gettextrs::{bind_textdomain_codeset, bindtextdomain, textdomain};
use gtk::prelude::*;
use gtk::{gio, glib};

/// Application settings. Optional so an uninstalled build (no compiled
/// schema) still runs.
pub fn app_settings() -> Option<gio::Settings> {
    let id = config::APP_ID;
    gio::SettingsSchemaSource::default()?.lookup(id, true)?;
    Some(gio::Settings::new(id))
}

/// `spindle --build PROJECT OUTDIR` builds a disc without the GUI.
fn build_cli(project: &str, out: &str) -> glib::ExitCode {
    let project = match model::Project::load(std::path::Path::new(project)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e:#}");
            return glib::ExitCode::FAILURE;
        }
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let last = std::cell::Cell::new(-1i32);
    let emit = |ev: build::BuildEvent| match ev {
        build::BuildEvent::Log(l) => eprintln!("{l}"),
        build::BuildEvent::Progress(p) => {
            let pct = (p * 100.0) as i32;
            if pct / 5 != last.get() / 5 {
                last.set(pct);
                eprintln!("  {pct}%");
            }
        }
        _ => {}
    };
    match build::build(&project, std::path::Path::new(out), &cancel, &emit) {
        Ok(p) => {
            eprintln!("Disc written to {}", p.display());
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}

/// `spindle --new-project OUT.spindle MEDIA...` creates a project from
/// media files (videos become titles).
fn new_project_cli(out: &str, files: &[String]) -> glib::ExitCode {
    let mut p = model::Project::default();
    for f in files {
        let path = std::path::PathBuf::from(f);
        match media::probe::probe(&path) {
            Ok(info) => {
                let id = model::new_id();
                let kind = if info.has_video() && info.duration >= 0.2 {
                    model::AssetKind::Video
                } else if info.has_video() {
                    model::AssetKind::Image
                } else {
                    model::AssetKind::Audio
                };
                p.assets.push(model::Asset { id, path: std::fs::canonicalize(&path).unwrap_or(path), kind, info });
                p.ensure_title_for(id);
            }
            Err(e) => eprintln!("skipping {f}: {e:#}"),
        }
    }
    match p.save(std::path::Path::new(out)) {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}

/// `spindle --apply-template show|movie|list THEME PROJECT` rewrites the
/// project's menus from a template.
fn template_cli(layout: &str, theme: &str, project: &str) -> glib::ExitCode {
    let layout = match layout {
        "show" => templates::Layout::Show,
        "show-list" => templates::Layout::ShowList,
        "movie" => templates::Layout::Movie,
        "list" => templates::Layout::List,
        _ => {
            eprintln!("unknown layout {layout} (show, show-list, movie, list)");
            return glib::ExitCode::FAILURE;
        }
    };
    let path = std::path::Path::new(project);
    let res = model::Project::load(path).and_then(|mut p| {
        let opts = templates::Options { layout, theme: theme.parse().unwrap_or(0), title: p.disc.name.clone(), logo: None };
        templates::apply(&mut p, &opts);
        p.save(path)
    });
    match res {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}

/// `spindle --preview PROJECT TITLE# START SECONDS [SUBTITLE#]` encodes a
/// preview clip (numbers are 1-based) and prints its path.
fn preview_cli(args: &[String]) -> glib::ExitCode {
    let run = || -> anyhow::Result<std::path::PathBuf> {
        let p = model::Project::load(std::path::Path::new(&args[0]))?;
        let n: usize = args[1].parse()?;
        let t = p.titles.get(n.saturating_sub(1)).ok_or_else(|| anyhow::anyhow!("no title {n}"))?;
        let subtitle = args.get(4).map(|s| s.parse::<usize>()).transpose()?.and_then(|i| t.subtitles.get(i.saturating_sub(1)).map(|s| s.id));
        let req = preview::PreviewRequest { title: t.id, start: args[2].parse()?, duration: args[3].parse()?, subtitle };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        preview::encode(&p, &req, &cancel, &|ev| {
            if let build::BuildEvent::Stage(s) = ev {
                eprintln!("{s}");
            }
        })
    };
    match run() {
        Ok(path) => {
            println!("{}", path.display());
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("spindle=info")).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--build" {
        return build_cli(&args[2], &args[3]);
    }
    if (4..=5).contains(&args.len()) && args[1] == "--make-image" {
        let (dir, iso) = (std::path::Path::new(&args[2]), std::path::Path::new(&args[3]));
        let label = args.get(4).cloned().unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        return match bluray::udf::write_image(dir, iso, &label, |_| true) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.len() >= 4 && args[1] == "--new-project" {
        return new_project_cli(&args[2], &args[3..]);
    }
    if (6..=7).contains(&args.len()) && args[1] == "--preview" {
        return preview_cli(&args[2..]);
    }
    if args.len() == 3 && args[1] == "--check-video" {
        return match media::compat::analyze(std::path::Path::new(&args[2])) {
            Ok(r) => {
                for c in &r.checks {
                    println!("{:?}\t{}\t{}", c.level, c.name, c.detail);
                }
                println!("=> {}", r.summary());
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.len() == 5 && args[1] == "--apply-template" {
        return template_cli(&args[2], &args[3], &args[4]);
    }

    // Set up gettext translations
    bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR).expect("Unable to bind the text domain");
    bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8")
        .expect("Unable to set the text domain encoding");
    textdomain(GETTEXT_PACKAGE).expect("Unable to switch to the text domain");

    // Load resources
    let resources = gio::Resource::load(PKGDATADIR.to_owned() + "/spindle.gresource")
        .expect("Could not load resources");
    gio::resources_register(&resources);

    let app = SpindleApplication::new(config::APP_ID, &gio::ApplicationFlags::HANDLES_OPEN);
    app.run()
}
