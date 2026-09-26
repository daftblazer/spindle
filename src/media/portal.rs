// SPDX-License-Identifier: GPL-3.0-or-later

//! Document portal paths.
//!
//! Inside Flatpak, files chosen through the file chooser may arrive as
//! `/run/user/UID/doc/ID/name` links that only work for this app ID and
//! can be revoked. Projects must store real paths instead, so these are
//! translated with `org.freedesktop.portal.Documents.GetHostPaths`.

use gtk::glib::prelude::*;
use gtk::{gio, glib};
use std::path::{Component, Path, PathBuf};

/// Split a document portal path into (document id, path inside the document).
fn doc_parts(path: &Path) -> Option<(String, PathBuf)> {
    let mut comps = path.components();
    let expect = |c: Option<Component>, s: &str| matches!(c, Some(Component::Normal(n)) if n == s);
    if comps.next() != Some(Component::RootDir) || !expect(comps.next(), "run") || !expect(comps.next(), "user") {
        return None;
    }
    comps.next()?; // uid
    if !expect(comps.next(), "doc") {
        return None;
    }
    let Some(Component::Normal(id)) = comps.next() else { return None };
    let rest: PathBuf = comps.collect();
    Some((id.to_string_lossy().into_owned(), rest))
}

pub fn is_portal_path(path: &Path) -> bool {
    doc_parts(path).is_some()
}

/// The real location of a document portal path, if the portal knows it.
fn host_path(path: &Path) -> Option<PathBuf> {
    let (id, rest) = doc_parts(path)?;
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let reply = bus
        .call_sync(
            Some("org.freedesktop.portal.Documents"),
            "/org/freedesktop/portal/documents",
            "org.freedesktop.portal.Documents",
            "GetHostPaths",
            Some(&(vec![id.clone()],).to_variant()),
            Some(glib::VariantTy::new("(a{say})").unwrap()),
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let map = reply.child_value(0);
    let bytes: Vec<u8> = (0..map.n_children()).map(|i| map.child_value(i)).find_map(|entry| {
        let key: String = entry.child_value(0).get()?;
        (key == id).then(|| entry.child_value(1).get::<Vec<u8>>()).flatten()
    })?;
    // The byte string is NUL-terminated.
    let bytes = bytes.split(|b| *b == 0).next()?.to_vec();
    use std::os::unix::ffi::OsStringExt;
    let mut host = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    // The document is the file itself for single files, or a directory.
    if !rest.as_os_str().is_empty() && host.file_name() != rest.file_name() {
        host = host.join(rest);
    }
    Some(host)
}

/// Replace a document portal path by the real path when that path is
/// readable by the app (e.g. with `--filesystem=host`). Other paths are
/// returned unchanged.
pub fn real_path(path: &Path) -> PathBuf {
    if !is_portal_path(path) {
        return path.to_path_buf();
    }
    match host_path(path) {
        Some(host) if host.exists() => host,
        _ => path.to_path_buf(),
    }
}

/// Find files named `name` below `dir` (a few levels deep).
pub fn find_by_name(dir: &Path, names: &[String], depth: usize) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return found };
    let mut subdirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if let Some(n) = p.file_name().map(|n| n.to_string_lossy().into_owned()) {
            if names.contains(&n) && !found.iter().any(|(f, _): &(String, PathBuf)| *f == n) {
                found.push((n, p));
            }
        }
    }
    if depth > 0 {
        subdirs.sort();
        for d in subdirs {
            for (n, p) in find_by_name(&d, names, depth - 1) {
                if !found.iter().any(|(f, _)| *f == n) {
                    found.push((n, p));
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_portal_paths() {
        let (id, rest) = doc_parts(Path::new("/run/user/1000/doc/brbUj1FwYg0MM4C0crzwcQ/Episode 12.mkv")).unwrap();
        assert_eq!(id, "brbUj1FwYg0MM4C0crzwcQ");
        assert_eq!(rest, PathBuf::from("Episode 12.mkv"));
        assert!(!is_portal_path(Path::new("/var/home/me/Videos/a.mkv")));
        assert!(!is_portal_path(Path::new("/run/user/1000/gvfs/x")));
    }

    #[test]
    fn finds_files_in_subfolders() {
        let root = std::env::temp_dir().join(format!("spindle-find-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Season 2/extras")).unwrap();
        std::fs::write(root.join("Season 2/Episode 12.mkv"), b"").unwrap();
        std::fs::write(root.join("Season 2/extras/Episode 10.mkv"), b"").unwrap();
        let names = vec!["Episode 12.mkv".to_string(), "Episode 10.mkv".to_string(), "Nope.mkv".to_string()];
        let found = find_by_name(&root, &names, 4);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|(n, p)| n == "Episode 10.mkv" && p.ends_with("extras/Episode 10.mkv")));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod live_tests {
    /// Resolves a real document portal path given in SPINDLE_TEST_DOC_PATH.
    #[test]
    fn resolves_live_portal_path() {
        let Ok(p) = std::env::var("SPINDLE_TEST_DOC_PATH") else { return };
        let real = super::host_path(std::path::Path::new(&p));
        eprintln!("{p} -> {real:?}");
        assert!(real.is_some_and(|r| r.exists()));
    }
}
