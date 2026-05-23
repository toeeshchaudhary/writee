//! Tool state machine.

use glam::Vec2;
use writee_core::{Anchor, ObjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
    Arrow,
    Text,
    Shape,
    Note,
    Index,
    Link,
    Select,
}

impl Tool {
    pub fn short_name(self) -> &'static str {
        match self {
            Tool::Pen => "pen",
            Tool::Highlighter => "highlight",
            Tool::Eraser => "eraser",
            Tool::Arrow => "arrow",
            Tool::Text => "text",
            Tool::Shape => "shape",
            Tool::Note => "note",
            Tool::Index => "index",
            Tool::Link => "link",
            Tool::Select => "select",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Pen => "Pen (P)",
            Tool::Highlighter => "Highlight (H)",
            Tool::Eraser => "Eraser (E)",
            Tool::Arrow => "Arrow (A)",
            Tool::Text => "Text (T)",
            Tool::Shape => "Shape (R)",
            Tool::Note => "Note (N)",
            Tool::Index => "Index (I)",
            Tool::Link => "Link (L)",
            Tool::Select => "Select (S)",
        }
    }

    pub fn drawing_tools() -> &'static [Tool] {
        &[Tool::Pen, Tool::Highlighter, Tool::Eraser]
    }
    pub fn annotation_tools() -> &'static [Tool] {
        &[Tool::Arrow, Tool::Shape, Tool::Text, Tool::Note, Tool::Index, Tool::Link]
    }
    pub fn selection_tools() -> &'static [Tool] {
        &[Tool::Select]
    }
    pub fn all() -> [Tool; 10] {
        [
            Tool::Pen, Tool::Highlighter, Tool::Eraser,
            Tool::Arrow, Tool::Shape, Tool::Text, Tool::Note, Tool::Index, Tool::Link,
            Tool::Select,
        ]
    }

    pub fn cursor_icon(self) -> winit::window::CursorIcon {
        use winit::window::CursorIcon;
        match self {
            Tool::Pen | Tool::Highlighter => CursorIcon::Crosshair,
            Tool::Eraser => CursorIcon::Crosshair,
            Tool::Arrow | Tool::Shape => CursorIcon::Crosshair,
            Tool::Text => CursorIcon::Text,
            Tool::Note => CursorIcon::Copy,
            Tool::Index => CursorIcon::Copy,
            Tool::Link => CursorIcon::Alias,
            Tool::Select => CursorIcon::Default,
        }
    }
}

#[derive(Debug, Default)]
pub struct ToolState {
    pub current: Option<Tool>,
    pub arrow_start: Option<Vec2>,
    pub arrow_end: Option<Vec2>,
    pub editing_text: Option<ObjectId>,
    /// Inline sub-note (sticky) whose body is currently being typed into.
    /// Mutually exclusive with `editing_text` in practice; tools clear both
    /// when switching focus.
    pub editing_note: Option<ObjectId>,
    /// Snapshot of the text content at the start of an edit session, so undo
    /// can restore it rather than deleting the whole box per keypress.
    pub edit_text_before: Option<String>,
    pub selected: Vec<ObjectId>,
    pub drag_origin_world: Option<Vec2>,
    /// Original click position (in world units), used to detect whether the
    /// gesture has exceeded the click-vs-drag threshold yet.
    pub gesture_origin_world: Option<Vec2>,
    /// True once the gesture has moved past the click-vs-drag threshold and
    /// is being treated as a drag (not a click-to-edit).
    pub drag_active: bool,
    pub eraser_dragging: bool,
    /// Pre-drag positions for each currently-selected object so we can build
    /// an Op::Replace on drag end.
    pub drag_before: Vec<(ObjectId, writee_core::Object)>,
    /// World-space marquee rectangle while the user drags in empty space with
    /// the select tool. (start, current).
    pub marquee: Option<(Vec2, Vec2)>,
    /// While dragging out a shape (rect/ellipse/line), the start corner.
    pub shape_start: Option<Vec2>,
    pub shape_end: Option<Vec2>,
    /// In-progress link: (source object id, source anchor, source world pos,
    /// current target world pos).
    pub link_in_progress: Option<(ObjectId, Anchor, Vec2, Vec2)>,
    /// If the current Select gesture started on a SubNote card and the user
    /// hasn't dragged, releasing the pointer should *open* the sub-note
    /// rather than just leaving it selected.
    pub click_target_subnote: Option<ObjectId>,
}

impl ToolState {
    pub fn reset_transient(&mut self) {
        self.arrow_start = None;
        self.arrow_end = None;
        self.drag_origin_world = None;
        self.gesture_origin_world = None;
        self.drag_active = false;
        self.eraser_dragging = false;
        self.drag_before.clear();
        self.marquee = None;
        self.shape_start = None;
        self.shape_end = None;
        self.link_in_progress = None;
        self.click_target_subnote = None;
        self.editing_note = None;
    }
}
