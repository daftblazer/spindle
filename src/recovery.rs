// SPDX-License-Identifier: GPL-3.0-or-later

//! Autosaved copies of projects with unsaved changes, so work survives a
//! crash. The copy is removed on save or a normal close; one left behind
//! when Spindle starts means it didn't close normally.

use crate::model::{Id, Project};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn dir() -> PathBuf {
    gtk::glib::user_data_dir().join("spindle").join("recovery")
}

#[derive(Serialize, Deserialize)]
struct Meta {
    /// The project's own file, if it had been saved.
    original: Option<PathBuf>,
    name: String,
}

fn paths(d: &Path, session: Id) -> (PathBuf, PathBuf) {
    (d.join(format!("{session}.spindle")), d.join(format!("{session}.json")))
}

/// Save a recovery copy of `project` for `session`.
pub fn write(session: Id, project: &Project, original: Option<&Path>) -> Result<()> {
    write_in(&dir(), session, project, original)
}

fn write_in(d: &Path, session: Id, project: &Project, original: Option<&Path>) -> Result<()> {
    std::fs::create_dir_all(d)?;
    let (proj, meta) = paths(d, session);
    project.save(&proj)?;
    let m = Meta { original: original.map(Path::to_path_buf), name: project.disc.name.clone() };
    std::fs::write(meta, serde_json::to_vec(&m)?)?;
    Ok(())
}

pub fn remove(session: Id) {
    remove_in(&dir(), session);
}

fn remove_in(d: &Path, session: Id) {
    let (proj, meta) = paths(d, session);
    let _ = std::fs::remove_file(proj);
    let _ = std::fs::remove_file(meta);
}

/// A recovery copy left behind by an earlier run.
#[derive(Debug, Clone)]
pub struct Leftover {
    pub session: Id,
    pub project: PathBuf,
    pub original: Option<PathBuf>,
    pub name: String,
    pub saved: SystemTime,
}

impl Leftover {
    pub fn load(&self) -> Result<Project> {
        Project::load(&self.project)
    }

    pub fn discard(&self) {
        if let Some(d) = self.project.parent() {
            remove_in(d, self.session);
        }
    }
}

/// Copies left by earlier runs, newest first.
pub fn leftovers() -> Vec<Leftover> {
    leftovers_in(&dir())
}

fn leftovers_in(d: &Path) -> Vec<Leftover> {
    let Ok(rd) = std::fs::read_dir(d) else { return vec![] };
    let mut out: Vec<Leftover> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "spindle" {
                return None;
            }
            let session: Id = path.file_stem()?.to_string_lossy().parse().ok()?;
            let meta: Option<Meta> = std::fs::read(path.with_extension("json")).ok().and_then(|d| serde_json::from_slice(&d).ok());
            let saved = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let (original, name) = meta.map(|m| (m.original, m.name)).unwrap_or_default();
            Some(Leftover { session, project: path, original, name, saved })
        })
        .collect();
    out.sort_by_key(|l| std::cmp::Reverse(l.saved));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let d = std::env::temp_dir().join(format!("spindle-recovery-{}", crate::model::new_id()));
        let mut p = Project::default();
        p.disc.name = "Harbor Lights".into();
        let (a, b) = (crate::model::new_id(), crate::model::new_id());
        write_in(&d, a, &p, Some(Path::new("/tmp/show.spindle"))).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_in(&d, b, &p, None).unwrap();
        let found = leftovers_in(&d);
        assert_eq!(found.iter().map(|l| l.session).collect::<Vec<_>>(), vec![b, a], "newest first");
        assert_eq!(found[1].original.as_deref(), Some(Path::new("/tmp/show.spindle")));
        assert_eq!(found[0].name, "Harbor Lights");
        assert_eq!(found[0].load().unwrap().disc.name, "Harbor Lights");
        found[0].discard();
        assert_eq!(leftovers_in(&d).len(), 1);
        remove_in(&d, a);
        assert!(leftovers_in(&d).is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
