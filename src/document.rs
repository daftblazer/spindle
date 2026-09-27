// SPDX-License-Identifier: GPL-3.0-or-later

//! Editor state shared by all UI components: the project, its file, the
//! current selection, undo history and change notification.

use crate::model::{Id, Project, UndoStack};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

/// What part of the editor is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Node {
    #[default]
    None,
    Menu(Id),
    Title(Id),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Lists of assets/menus/titles or names changed: rebuild everything.
    Structure,
    /// The selected node or item changed.
    Selection,
    /// Properties of the current menu/title changed (redraw, refresh values).
    Content,
    /// Only the file path / dirty state changed.
    File,
}

type Listener = Box<dyn Fn(Change)>;

#[derive(Default)]
pub struct Document {
    project: RefCell<Project>,
    path: RefCell<Option<PathBuf>>,
    dirty: Cell<bool>,
    undo: RefCell<UndoStack>,
    node: Cell<Node>,
    /// Selected menu items; the first one is the primary selection shown in
    /// the inspector.
    items: RefCell<Vec<Id>>,
    listeners: RefCell<Vec<Listener>>,
    notifying: Cell<bool>,
}

impl Document {
    pub fn new() -> Rc<Self> {
        let d = Rc::new(Document::default());
        d.select_default();
        d
    }

    pub fn connect(&self, f: impl Fn(Change) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    fn notify(&self, c: Change) {
        // Listeners must not re-enter edits synchronously; guard anyway.
        if self.notifying.replace(true) {
            log::warn!("nested document notification ({c:?}) ignored");
            return;
        }
        for l in self.listeners.borrow().iter() {
            l(c);
        }
        self.notifying.set(false);
    }

    pub fn project(&self) -> std::cell::Ref<'_, Project> {
        self.project.borrow()
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.path.borrow().clone()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    pub fn node(&self) -> Node {
        self.node.get()
    }

    /// Primary selected item.
    pub fn item(&self) -> Option<Id> {
        self.items.borrow().first().copied()
    }

    /// All selected items, primary first.
    pub fn selection(&self) -> Vec<Id> {
        self.items.borrow().clone()
    }

    pub fn is_selected(&self, id: Id) -> bool {
        self.items.borrow().contains(&id)
    }

    pub fn current_menu(&self) -> Option<Id> {
        match self.node.get() {
            Node::Menu(id) => Some(id),
            _ => None,
        }
    }

    fn select_default(&self) {
        let first = self.project.borrow().menus.first().map(|m| m.id);
        self.node.set(first.map_or(Node::None, Node::Menu));
        self.items.borrow_mut().clear();
    }

    /// Replace the whole project (new/open).
    pub fn replace(&self, project: Project, path: Option<PathBuf>) {
        *self.project.borrow_mut() = project;
        *self.path.borrow_mut() = path;
        self.dirty.set(false);
        self.undo.borrow_mut().clear();
        self.select_default();
        self.notify(Change::Structure);
    }

    /// Mark as having unsaved changes (e.g. a recovered project).
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
        self.notify(Change::File);
    }

    pub fn set_saved(&self, path: PathBuf) {
        *self.path.borrow_mut() = Some(path);
        self.dirty.set(false);
        self.notify(Change::File);
    }

    pub fn select(&self, node: Node, item: Option<Id>) {
        self.select_many(node, item.into_iter().collect());
    }

    pub fn select_many(&self, node: Node, items: Vec<Id>) {
        if self.node.get() == node && *self.items.borrow() == items {
            return;
        }
        self.node.set(node);
        *self.items.borrow_mut() = items;
        self.notify(Change::Selection);
    }

    pub fn select_item(&self, item: Option<Id>) {
        self.select(self.node.get(), item);
    }

    pub fn set_selection(&self, items: Vec<Id>) {
        self.select_many(self.node.get(), items);
    }

    /// Add or remove an item from the selection (shift/ctrl click).
    pub fn toggle_item(&self, id: Id) {
        let mut items = self.selection();
        if let Some(pos) = items.iter().position(|i| *i == id) {
            items.remove(pos);
        } else {
            items.push(id);
        }
        self.set_selection(items);
    }

    /// Record an undo point and apply `f`.
    pub fn edit<R>(&self, change: Change, f: impl FnOnce(&mut Project) -> R) -> R {
        self.checkpoint();
        self.edit_silent(change, f)
    }

    /// Record an undo point without changing anything (start of a drag).
    pub fn checkpoint(&self) {
        let snapshot = self.project.borrow().clone();
        self.undo.borrow_mut().record(snapshot);
    }

    /// Apply `f` without recording an undo point (continuation of a drag).
    pub fn edit_silent<R>(&self, change: Change, f: impl FnOnce(&mut Project) -> R) -> R {
        let r = f(&mut self.project.borrow_mut());
        self.dirty.set(true);
        self.fix_selection();
        self.notify(change);
        r
    }

    fn fix_selection(&self) {
        let p = self.project.borrow();
        let valid = match self.node.get() {
            Node::Menu(id) => p.menu(id).is_some(),
            Node::Title(id) => p.title(id).is_some(),
            Node::None => true,
        };
        if !valid {
            drop(p);
            self.select_default();
            return;
        }
        match self.node.get() {
            Node::Menu(m) => {
                let menu = p.menu(m);
                self.items.borrow_mut().retain(|i| menu.is_some_and(|m| m.item(*i).is_some()));
            }
            _ => self.items.borrow_mut().clear(),
        }
    }

    pub fn can_undo(&self) -> bool {
        self.undo.borrow().can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.borrow().can_redo()
    }

    pub fn undo(&self) {
        let prev = self.undo.borrow_mut().undo(&self.project.borrow());
        if let Some(p) = prev {
            *self.project.borrow_mut() = p;
            self.dirty.set(true);
            self.fix_selection();
            self.notify(Change::Structure);
        }
    }

    pub fn redo(&self) {
        let next = self.undo.borrow_mut().redo(&self.project.borrow());
        if let Some(p) = next {
            *self.project.borrow_mut() = p;
            self.dirty.set(true);
            self.fix_selection();
            self.notify(Change::Structure);
        }
    }

    /// Display name of the document.
    pub fn title(&self) -> String {
        self.path
            .borrow()
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| gettextrs::gettext("Untitled Disc"))
    }
}
