// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot-based undo/redo. Projects are small, so storing whole copies
//! keeps every edit trivially reversible.

use super::Project;

const LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct UndoStack {
    past: Vec<Project>,
    future: Vec<Project>,
}

impl UndoStack {
    /// Record the state *before* an edit.
    pub fn record(&mut self, before: Project) {
        if self.past.last() == Some(&before) {
            return;
        }
        self.past.push(before);
        if self.past.len() > LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
    }

    pub fn undo(&mut self, current: &Project) -> Option<Project> {
        let prev = self.past.pop()?;
        self.future.push(current.clone());
        Some(prev)
    }

    pub fn redo(&mut self, current: &Project) -> Option<Project> {
        let next = self.future.pop()?;
        self.past.push(current.clone());
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo() {
        let mut u = UndoStack::default();
        let a = Project::default();
        let mut b = a.clone();
        b.disc.name = "B".into();
        u.record(a.clone());
        let back = u.undo(&b).unwrap();
        assert_eq!(back, a);
        assert_eq!(u.redo(&back).unwrap(), b);
    }
}
