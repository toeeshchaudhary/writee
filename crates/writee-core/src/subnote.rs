//! Notes placed on a parent canvas.
//!
//! A `SubNote` has two flavours:
//!
//! * **Inline (sticky)** — `inline_content = Some(text)`, `target_file` empty.
//!   The text lives directly on the parent's document, the card renders the
//!   body in-place, and the user can edit it without opening anything.
//! * **Linked** — `inline_content = None`, `target_file` populated. The card
//!   is a pointer to a separate `.writee` file; clicking it opens that file
//!   in its own editor (canvas or markdown, per the child's meta).
//!
//! An `is_index` flag overlays either flavour with a workspace file-picker
//! body (the welcome canvas uses an `is_index = true, locked = true` card as
//! its workspace home).
//!
//! Conversions between flavours happen at the app layer:
//! see `App::convert_note_inline_to_linked` / `convert_note_linked_to_inline`.

use crate::geom::Aabb;
use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NoteMode {
    #[default]
    Canvas,
    Markdown,
}

/// One row inside an index card body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndexEntry {
    /// A workspace-relative `.writee` filename. Clicking the row jumps to it.
    File { file: String },
    /// A bold label used to group files visually within the card body.
    Heading { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubNote {
    pub origin: Vec2,
    pub size: Vec2,

    /// What the user sees printed on the card.
    pub title: String,

    // -- Linked flavour (legacy fields; default-empty for inline notes). --
    /// Path relative to the workspace root, e.g. `coulombs-law.writee`.
    /// Empty string when this is an inline sticky note.
    #[serde(default)]
    pub target_file: String,
    /// Hint to the child file's renderer when the user first opens it.
    /// Only meaningful for linked notes.
    #[serde(default)]
    pub mode: NoteMode,

    // -- Inline flavour (added later; serde defaults keep old files loadable). --
    /// `Some(text)` means this is an inline sticky note with its content
    /// stored right here in the parent document. `None` means it's a card
    /// linking to `target_file`.
    #[serde(default)]
    pub inline_content: Option<String>,

    /// If true, the card body renders the workspace file picker instead of
    /// inline content or the linked-file hint. The welcome canvas uses
    /// `is_index = true, locked = true` to anchor the workspace home.
    #[serde(default)]
    pub is_index: bool,

    /// (Legacy) When `is_index` and this is `Some`, only the listed file
    /// names are shown. Kept for back-compat reads; new edits write to
    /// `index_entries` and clear this field.
    #[serde(default)]
    pub index_files: Option<Vec<String>>,

    /// New-style curated list — a mix of file rows and section headings,
    /// preserving the user's chosen order. When both `index_files` and
    /// `index_entries` are present, `index_entries` wins. `None` (or both
    /// empty) on an index card means "show every file in the workspace".
    #[serde(default)]
    pub index_entries: Option<Vec<IndexEntry>>,

    /// If true, drag / delete / convert are refused. Used to pin the welcome
    /// index card so the user can't lose their workspace home by accident.
    #[serde(default)]
    pub locked: bool,

    /// Caret byte-offset into `inline_content` while editing. Skipped during
    /// serialization so we don't pollute saved files with transient editor
    /// state. Defaults to 0 on load.
    #[serde(skip)]
    pub cursor: usize,
}

impl SubNote {
    pub const DEFAULT_W: f32 = 220.0;
    pub const DEFAULT_H: f32 = 150.0;
    /// Reserved strip at the top of every card for the title text + border.
    pub const TITLE_BAR_H: f32 = 28.0;
    /// Insets for body content (text or picker), measured from the card edges.
    pub const BODY_INSET: f32 = 10.0;

    /// Build an inline sticky note with empty body and no title (matching
    /// Affine: stickies are just text, no header chrome). Callers pass the
    /// title in only for linked / index cards.
    pub fn new_inline(origin: Vec2, _title_hint: String) -> Self {
        Self {
            origin,
            size: Vec2::new(Self::DEFAULT_W, Self::DEFAULT_H),
            title: String::new(),
            target_file: String::new(),
            mode: NoteMode::Markdown,
            inline_content: Some(String::new()),
            is_index: false,
            index_files: None,
            index_entries: None,
            locked: false,
            cursor: 0,
        }
    }

    /// Inline stickies have no title chrome (matches Affine). Linked + index
    /// cards keep the title bar so their identity is visible at a glance.
    pub fn has_title_bar(&self) -> bool {
        !self.is_inline() || self.is_index
    }

    /// Build a linked card pointing at the given child `.writee` file.
    pub fn new_linked(origin: Vec2, target_file: String, title: String, mode: NoteMode) -> Self {
        Self {
            origin,
            size: Vec2::new(Self::DEFAULT_W, Self::DEFAULT_H),
            title,
            target_file,
            mode,
            inline_content: None,
            is_index: false,
            index_files: None,
            index_entries: None,
            locked: false,
            cursor: 0,
        }
    }

    /// Convenience for the welcome seeder — an immovable index card.
    pub fn new_locked_index(origin: Vec2, title: String) -> Self {
        Self {
            origin,
            size: Vec2::new(360.0, 420.0),
            title,
            target_file: String::new(),
            mode: NoteMode::Canvas,
            inline_content: None,
            is_index: true,
            // Welcome card shows everything — that's the workspace home.
            index_files: None,
            index_entries: None,
            locked: true,
            cursor: 0,
        }
    }

    pub fn is_inline(&self) -> bool {
        self.inline_content.is_some()
    }

    pub fn is_linked(&self) -> bool {
        !self.is_inline() && !self.target_file.is_empty()
    }

    pub fn bbox(&self) -> Aabb {
        Aabb { min: self.origin, max: self.origin + self.size }
    }

    pub fn center(&self) -> Vec2 {
        self.origin + self.size * 0.5
    }

    /// World-space rectangle for the body (everything below the title bar
    /// when one exists, inset by `BODY_INSET`). Inline stickies have no
    /// title bar so their body fills the full card.
    pub fn body_rect(&self) -> Aabb {
        let inset = Self::BODY_INSET;
        let top_pad = if self.has_title_bar() { Self::TITLE_BAR_H } else { 0.0 };
        let min = self.origin + Vec2::new(inset, top_pad + inset);
        let max = self.origin + self.size - Vec2::splat(inset);
        Aabb { min, max }
    }

    // -- Cursor helpers, mirroring those on `TextBox`. ----------------------

    pub fn clamp_cursor(&mut self) {
        let len = self.inline_content.as_ref().map(|s| s.len()).unwrap_or(0);
        if self.cursor > len {
            self.cursor = len;
        }
        if let Some(s) = &self.inline_content {
            while self.cursor > 0 && !s.is_char_boundary(self.cursor) {
                self.cursor -= 1;
            }
        }
    }

    pub fn insert_at_cursor(&mut self, ch: char) {
        self.clamp_cursor();
        let Some(s) = self.inline_content.as_mut() else { return };
        s.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn backspace_at_cursor(&mut self) -> bool {
        self.clamp_cursor();
        let Some(s) = self.inline_content.as_mut() else { return false };
        if self.cursor == 0 {
            return false;
        }
        let mut start = self.cursor - 1;
        while start > 0 && !s.is_char_boundary(start) {
            start -= 1;
        }
        s.replace_range(start..self.cursor, "");
        self.cursor = start;
        true
    }

    pub fn cursor_left(&mut self) {
        self.clamp_cursor();
        let Some(s) = &self.inline_content else { return };
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        while self.cursor > 0 && !s.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    pub fn cursor_right(&mut self) {
        self.clamp_cursor();
        let Some(s) = &self.inline_content else { return };
        if self.cursor >= s.len() {
            return;
        }
        self.cursor += 1;
        while self.cursor < s.len() && !s.is_char_boundary(self.cursor) {
            self.cursor += 1;
        }
    }

    pub fn cursor_home(&mut self) {
        self.clamp_cursor();
        let Some(s) = &self.inline_content else { return };
        let upto = &s[..self.cursor];
        self.cursor = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
    }

    pub fn cursor_end(&mut self) {
        self.clamp_cursor();
        let Some(s) = &self.inline_content else { return };
        let rest = &s[self.cursor..];
        self.cursor += rest.find('\n').unwrap_or(rest.len());
    }
}
