pub mod arrow;
pub mod document;
pub mod geom;
pub mod link;
pub mod one_euro;
pub mod shape;
pub mod storage;
pub mod stroke;
pub mod subnote;
pub mod tessellate;
pub mod textbox;
pub mod theme;

pub use arrow::Arrow;
pub use document::{Document, ImageBlock, Object, ObjectId};
pub use geom::Aabb;
pub use link::{Anchor, Link, LinkEnd};
pub use shape::{Shape, ShapeKind};
pub use storage::{DocStore, DocumentMode, StorageError};
pub use stroke::{InkPoint, Stroke};
pub use subnote::{IndexEntry, NoteMode, SubNote};
pub use tessellate::{
    tessellate, tessellate_arrow, tessellate_ellipse, tessellate_line, tessellate_opts,
    tessellate_rect, tessellate_rect_outline, tessellate_segment_strip, InkVertex, COLOR_HIGHLIGHT,
    COLOR_INK, COLOR_LINK, COLOR_MARQUEE, COLOR_SELECTION,
};
pub use textbox::TextBox;
pub use theme::{ColorTheme, ThemeName};
