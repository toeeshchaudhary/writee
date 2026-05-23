//! In-memory undo/redo with a coarse-grained operation log.
//!
//! Each [`Op`] describes one user-visible change against the document. The
//! stack pairs an op with its inverse so undo/redo are symmetric.

use writee_core::{Object, ObjectId};

#[derive(Debug, Clone)]
pub enum Op {
    Add { id: ObjectId, object: Object },
    Remove { id: ObjectId, object: Object },
    Replace { id: ObjectId, before: Object, after: Object },
}

#[derive(Debug, Default)]
pub struct UndoStack {
    undo: Vec<Op>,
    redo: Vec<Op>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new user action. Clears the redo branch — once you act, the
    /// future fork that was reachable via redo is abandoned.
    pub fn push(&mut self, op: Op) {
        self.undo.push(op);
        self.redo.clear();
    }

    pub fn pop_undo(&mut self) -> Option<Op> {
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Op> {
        self.redo.pop()
    }

    pub fn push_redo(&mut self, op: Op) {
        self.redo.push(op);
    }

    pub fn push_undo_without_clear(&mut self, op: Op) {
        self.undo.push(op);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
