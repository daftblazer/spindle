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
mod burn;
mod config;
mod document;
mod encode_cache;
mod media;
mod model;
mod preview;
mod recovery;
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
    let last = std::sync::atomic::AtomicI32::new(-1);
    let emit = |ev: build::BuildEvent| match ev {
        build::BuildEvent::Log(l) => eprintln!("{l}"),
        build::BuildEvent::Progress(p) => {
            let pct = (p * 100.0) as i32;
            if pct / 5 != last.load(std::sync::atomic::Ordering::Relaxed) / 5 {
                last.store(pct, std::sync::atomic::Ordering::Relaxed);
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

/// `spindle --apply-template STYLE THEME PROJECT [SEASON [DISC]]` rewrites the
/// project's menus from a template.
/// STYLE can also be a `.spindle-template` file or the name of a saved one.
fn template_cli(style: &str, theme: &str, project: &str, extra: &[String]) -> glib::ExitCode {
    let custom = if style.ends_with(".spindle-template") {
        match templates::custom::load(std::path::Path::new(style)) {
            Ok(t) => Some(std::sync::Arc::new(t)),
            Err(e) => {
                eprintln!("error: {e:#}");
                return glib::ExitCode::FAILURE;
            }
        }
    } else {
        templates::custom::find(style).map(std::sync::Arc::new)
    };
    let Some(style) = templates::Style::from_id(style).or(custom.as_ref().map(|_| templates::Style::Classic)) else {
        let ids: Vec<&str> = templates::Style::ALL.iter().map(|s| s.id()).collect();
        eprintln!("unknown style {style} ({})", ids.join(", "));
        return glib::ExitCode::FAILURE;
    };
    let path = std::path::Path::new(project);
    let res = model::Project::load(path).and_then(|mut p| {
        let mut opts = templates::Options::new(style, &p.disc.name);
        opts.custom = custom;
        if let Ok(t) = theme.parse() {
            opts.theme = t;
        }
        opts.season = extra.first().cloned().unwrap_or_default();
        opts.disc = extra.get(1).cloned().unwrap_or_default();
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
    // --burn IMAGE DRIVE [--verify-only]: burn (and verify) an image.
    if (4..=5).contains(&args.len()) && args[1] == "--burn" {
        let (image, drive) = (std::path::Path::new(&args[2]), std::path::Path::new(&args[3]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let last = std::cell::Cell::new(-1i32);
        let report = |what: &str, f: f64| {
            let pct = (f * 100.0) as i32;
            if pct / 10 != last.get() / 10 {
                last.set(pct);
                eprintln!("{what} {pct}%");
            }
        };
        let run = || -> anyhow::Result<()> {
            if args.get(4).map(String::as_str) != Some("--verify-only") {
                burn::burn(image, drive, &cancel, |f| report("burning", f))?;
            }
            last.set(-1);
            burn::verify(image, drive, &cancel, |f| report("verifying", f))
        };
        return match run() {
            Ok(()) => {
                eprintln!("The disc matches the image.");
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.len() == 3 && args[1] == "--disc-status" {
        eprintln!("{:?}", burn::check(std::path::Path::new(&args[2])));
        return glib::ExitCode::SUCCESS;
    }
    // --render-menus PROJECT DIR: every menu as a PNG, for reviewing designs.
    if args.len() == 4 && args[1] == "--render-menus" {
        let run = || -> anyhow::Result<()> {
            let p = model::Project::load(std::path::Path::new(&args[2]))?;
            let dir = std::path::Path::new(&args[3]);
            std::fs::create_dir_all(dir)?;
            let images = render::ImageCache::new_sync();
            for (i, m) in p.menus.iter().enumerate() {
                let name: String = m.name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                render::menu_png(&p, m, &images, 1920, &dir.join(format!("{i:02}-{name}.png")))?;
            }
            Ok(())
        };
        return match run() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    // --apply-template STYLE THEME PROJECT [SEASON [DISC]] (THEME "-" = the style's own)
    // --save-template PROJECT OUT.spindle-template NAME [DESCRIPTION]: save the
    // project's main menu and episode pages as a custom template.
    if (5..=6).contains(&args.len()) && args[1] == "--save-template" {
        let res = model::Project::load(std::path::Path::new(&args[2]))
            .and_then(|p| templates::custom::from_project(&p, &args[4], args.get(5).map_or("", String::as_str), ""))
            .and_then(|t| templates::custom::save(&t, std::path::Path::new(&args[3])));
        return match res {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    // --install-template FILE: add a template to the ones in the app.
    if args.len() == 3 && args[1] == "--install-template" {
        let res = templates::custom::load(std::path::Path::new(&args[2])).and_then(|t| templates::custom::install(&t));
        return match res {
            Ok(path) => {
                println!("{}", path.display());
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if (5..=7).contains(&args.len()) && args[1] == "--apply-template" {
        return template_cli(&args[2], &args[3], &args[4], &args[5..]);
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
