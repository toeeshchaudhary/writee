//! writee desktop/Android app glue.
//!
//! Keyboard shortcuts:
//!   P / H / E / A / T / S — Pen / Highlighter / Eraser / Arrow / Text / Select
//!   Ctrl+Z / Ctrl+Shift+Z — undo / redo
//!   Ctrl+N — new whiteboard, Ctrl+O — cycle to next .writee
//!   Ctrl+E — export current doc to a static web folder
//!   Ctrl+F — fit document to viewport
//!   Delete / Backspace — remove current selection
//!   Esc — clear selection / cancel text edit / cancel marquee
//!
//! The toolbar at the top mirrors all of these plus runtime sliders for stroke
//! width, eraser radius, text size, and a pressure-sensitivity toggle.

pub mod geometry;
pub mod icon;
pub mod markdown;
pub mod md_shortcuts;
pub mod palette;
pub mod recovery;
pub mod search;
pub mod settings;
pub mod tags;
pub mod tool;
pub mod ui;
pub mod undo;
pub mod workspace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use egui_wgpu::ScreenDescriptor;
use glam::Vec2;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use settings::{ActiveShape, InkColor, ToolSettings};
use tool::{Tool, ToolState};
use ui::{EguiChrome, UiActions, UiInput};
use undo::{Op, UndoStack};
use workspace::Workspace;

use writee_core::{
    tessellate_arrow, tessellate_ellipse, tessellate_line, tessellate_opts, tessellate_rect,
    tessellate_rect_outline, tessellate_segment_strip, Aabb, Anchor, Arrow, DocStore, Document,
    DocumentMode, ImageBlock, InkPoint, InkVertex, Link, LinkEnd, Object, ObjectId, Shape,
    ShapeKind, Stroke, SubNote, TextBox, COLOR_LINK,
};
use writee_input::{InkSample, SamplePhase, WinitInput};
use writee_render::{EguiFrame, ImageQuad, Renderer, TextInstance, Viewport};

const PICK_SLACK: f32 = 6.0;

pub struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    chrome: Option<EguiChrome>,
    viewport: Viewport,
    input: WinitInput,

    document: Document,
    store: Option<DocStore>,
    workspace: Option<Workspace>,
    undo: UndoStack,
    cache: geometry::CommittedCache,
    settings: ToolSettings,
    fonts: settings::FontMappings,
    theme_name: writee_core::ThemeName,
    /// In-memory clipboard for copy/paste. Cleared on file switch.
    clipboard: Vec<Object>,
    /// Locked-notes state for the current file (mirrors `meta.locked_notes`
    /// from the store; refreshed on file switch).
    notes_locked: bool,

    tool: ToolState,
    cursor_px: Vec2,
    panning: bool,
    last_pan_px: Vec2,
    modifiers: ModifiersState,
    /// Last pen-pressure value observed (live in the toolbar).
    last_pressure: f32,
    /// The viewport has been centered on world (0, 0) for the current index
    /// file display at least once. Subsequent index visits don't re-center,
    /// so the user keeps wherever they panned to.
    index_initial_view_set: bool,

    wet: Vec<InkPoint>,
    drawing: bool,

    /// Loaded child documents for sub-note thumbnail previews. Keyed by
    /// absolute path; refreshed when the file's mtime advances.
    thumbnails: HashMap<PathBuf, ThumbnailEntry>,

    /// Active markdown editor state when the current file's mode is markdown.
    md: Option<markdown::MarkdownState>,

    /// (object_id, screen_pos) of an open right-click context menu, if any.
    context_menu: Option<(ObjectId, Vec2)>,

    /// Last time we wrote a recovery snapshot. Throttles disk IO.
    last_recovery_write: Option<Instant>,
    /// FNV-1a of (wet len, edit ids+content len) last written. Skip writing
    /// when nothing actually changed since the previous snapshot.
    last_recovery_sig: Option<u64>,
    /// If `Some`, a recovery snapshot was found on launch and the user
    /// hasn't yet decided whether to restore it.
    pending_recovery: Option<recovery::RecoverySnapshot>,

    /// Command-palette state (open flag, query, focused row).
    palette: palette::PaletteState,
    /// In-memory search cache shared by the palette.
    search_cache: search::SearchCache,
    /// If `Some`, the sidebar tree shows only files containing this tag.
    active_tag: Option<String>,

    /// Object id of the index card whose "edit contents" modal is open, plus
    /// the in-progress checkbox selection (a parallel-arrays view of every
    /// workspace file with whether it's currently included).
    index_editor: Option<IndexEditorState>,
}

struct IndexEditorState {
    object_id: ObjectId,
    /// Working set of entries the user is currently editing, in display order.
    /// Mix of files + headings; flushed back to the SubNote on commit.
    entries: Vec<writee_core::IndexEntry>,
    /// All workspace filenames at editor-open time, for the "add file" picker.
    available_files: Vec<String>,
    /// Buffer for "add heading" so the user can name the heading inline.
    new_heading_text: String,
    /// Currently selected filename in the "add file" dropdown.
    add_file_selected: Option<String>,
}

struct ThumbnailEntry {
    mtime: Option<SystemTime>,
    doc: Document,
}

impl App {
    pub fn new_with_workspace(
        workspace: Workspace,
        store: DocStore,
        document: Document,
        config: settings::WorkspaceConfig,
    ) -> Self {
        let mut tool = ToolState::default();
        tool.current = Some(Tool::Pen);
        let notes_locked = store.locked_notes().unwrap_or(false);
        let md = load_markdown_state(&store);
        Self {
            window: None,
            renderer: None,
            chrome: None,
            viewport: Viewport::new((1, 1)),
            input: WinitInput::new(),
            document,
            store: Some(store),
            workspace: Some(workspace),
            undo: UndoStack::new(),
            cache: geometry::CommittedCache::default(),
            settings: config.tools,
            fonts: config.fonts,
            theme_name: config.theme,
            clipboard: Vec::new(),
            notes_locked,
            tool,
            cursor_px: Vec2::ZERO,
            panning: false,
            last_pan_px: Vec2::ZERO,
            modifiers: ModifiersState::empty(),
            last_pressure: 1.0,
            index_initial_view_set: false,
            wet: Vec::new(),
            drawing: false,
            thumbnails: HashMap::new(),
            md,
            context_menu: None,
            index_editor: None,
            last_recovery_write: None,
            last_recovery_sig: None,
            pending_recovery: None,
            palette: palette::PaletteState::default(),
            search_cache: search::SearchCache::default(),
            active_tag: None,
        }
    }

    pub fn set_pending_recovery(&mut self, snap: recovery::RecoverySnapshot) {
        self.pending_recovery = Some(snap);
    }

    /// Snapshot in-progress state if it has actually changed and enough
    /// time has passed. Skips IO when nothing is transient or when the
    /// state matches the previous snapshot.
    fn maybe_write_recovery(&mut self) {
        let Some(ws) = self.workspace.as_ref() else { return };
        // Only TextBox edits triggered by the user qualify as "in progress" —
        // a fresh empty textbox we just created (no content yet) shouldn't
        // race a recovery write.
        let wet = if self.drawing && self.wet.len() >= 2 {
            self.wet.clone()
        } else {
            Vec::new()
        };
        let editing_text = self.tool.editing_text.and_then(|id| {
            self.document.get(id).and_then(|o| match o {
                Object::TextBox(tb) if !tb.content.is_empty() => Some((id, tb.content.clone())),
                _ => None,
            })
        });
        let editing_note = self.tool.editing_note.and_then(|id| {
            self.document.get(id).and_then(|o| match o {
                Object::SubNote(n) => {
                    n.inline_content
                        .clone()
                        .filter(|c| !c.is_empty())
                        .map(|c| (id, c))
                }
                _ => None,
            })
        });
        let has_content =
            !wet.is_empty() || editing_text.is_some() || editing_note.is_some();
        if !has_content {
            if self.last_recovery_write.is_some() {
                recovery::clear(&ws.root);
                self.last_recovery_write = None;
                self.last_recovery_sig = None;
            }
            return;
        }
        // Throttle to at most one write per interval.
        let now = Instant::now();
        if let Some(last) = self.last_recovery_write {
            if now.duration_since(last).as_secs() < recovery::SNAPSHOT_INTERVAL_SECS {
                return;
            }
        }
        // Content-fingerprint check — skip if nothing changed.
        let sig = recovery_signature(&wet, &editing_text, &editing_note);
        if self.last_recovery_sig == Some(sig) {
            self.last_recovery_write = Some(now);
            return;
        }
        let snap = recovery::RecoverySnapshot::now(
            ws.current_file.clone(),
            wet,
            editing_text,
            editing_note,
        );
        if let Err(e) = recovery::write(&ws.root, &snap) {
            log::warn!("recovery write failed: {e:?}");
            return;
        }
        self.last_recovery_write = Some(now);
        self.last_recovery_sig = Some(sig);
    }

    /// Apply a recovery snapshot the user accepted. Reinserts the lost work
    /// as a fresh stroke / re-opens the text edit.
    fn accept_recovery(&mut self, snap: recovery::RecoverySnapshot) {
        // Only restore into the same file the snapshot came from.
        if self.workspace.as_ref().map(|w| w.current_file.clone()) != Some(snap.current_file) {
            log::info!("recovery: snapshot is for a different file, skipping");
            return;
        }
        if snap.wet_stroke.len() >= 2 {
            let mut stroke = writee_core::Stroke::with_rgba(
                self.settings.stroke_width,
                self.settings.ink_color.to_idx(),
                self.settings.text_color,
            );
            stroke.points = snap.wet_stroke;
            self.add_object(Object::Stroke(stroke));
        }
        if let Some((id, content)) = snap.editing_text {
            if let Some(Object::TextBox(tb)) = self.document.get_mut(id) {
                tb.content = content;
                tb.clamp_cursor();
                if let Some(store) = &self.store {
                    let _ = store.update(id, &Object::TextBox(tb.clone()));
                }
            }
        }
        if let Some((id, content)) = snap.editing_note {
            if let Some(Object::SubNote(n)) = self.document.get_mut(id) {
                n.inline_content = Some(content);
                n.clamp_cursor();
                if let Some(store) = &self.store {
                    let _ = store.update(id, &Object::SubNote(n.clone()));
                }
            }
        }
        self.mark_doc_dirty();
    }

    fn save_config(&self) {
        let Some(ws) = &self.workspace else { return };
        let cfg = settings::WorkspaceConfig {
            tools: self.settings,
            fonts: self.fonts.clone(),
            window: settings::WindowConfig {
                width: self.viewport.screen.0,
                height: self.viewport.screen.1,
            },
            state: settings::PersistedState {
                last_file: ws
                    .current_file
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string()),
            },
            theme: self.theme_name,
        };
        if let Err(e) = cfg.save(&ws.root) {
            log::warn!("save settings failed: {e:?}");
        }
    }

    fn fonts_theme(&self) -> &'static writee_core::ColorTheme {
        self.theme_name.theme()
    }

    fn toggle_theme(&mut self) {
        self.theme_name = match self.theme_name {
            writee_core::ThemeName::Light => writee_core::ThemeName::Dark,
            writee_core::ThemeName::Dark => writee_core::ThemeName::Light,
        };
        self.cache.mark_dirty();
        self.save_config();
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn screen_to_world(&self, p: Vec2) -> Vec2 {
        self.viewport.offset + p / self.viewport.zoom
    }

    fn update_title(&self) {
        let Some(window) = &self.window else { return };
        let file = self
            .workspace
            .as_ref()
            .map(|w| {
                w.current_file
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string()
            })
            .unwrap_or_else(|| "untitled".to_string());
        let tool_name = self.tool.current.map(Tool::short_name).unwrap_or("-");
        window.set_title(&format!("writee — {file}  ·  {tool_name}"));
    }

    fn mark_doc_dirty(&mut self) {
        self.cache.mark_dirty();
    }

    // ------------------------------------------------------------------
    // Tool dispatch
    // ------------------------------------------------------------------

    fn on_sample(&mut self, s: InkSample) {
        self.last_pressure = s.pressure.clamp(0.0, 1.0);
        // Any pointer interaction outside the context menu dismisses it.
        if s.phase == SamplePhase::Begin {
            self.context_menu = None;
        }
        let world = self.screen_to_world(s.pos);
        if self.input.eraser_active() {
            self.handle_eraser(world, s);
            return;
        }
        match self.tool.current {
            Some(Tool::Pen) => self.handle_ink(world, s, InkColor::Pen),
            Some(Tool::Highlighter) => self.handle_ink(world, s, InkColor::Highlighter),
            Some(Tool::Eraser) => self.handle_eraser(world, s),
            Some(Tool::Arrow) => self.handle_arrow(world, s),
            Some(Tool::Text) => self.handle_text_tool(world, s),
            Some(Tool::Shape) => self.handle_shape(world, s),
            Some(Tool::Note) => self.handle_note(world, s),
            Some(Tool::Index) => self.handle_index_tool(world, s),
            Some(Tool::Link) => self.handle_link(world, s),
            Some(Tool::Select) => self.handle_select(world, s),
            None => {}
        }
    }

    fn handle_shape(&mut self, world: glam::Vec2, s: InkSample) {
        match s.phase {
            SamplePhase::Begin => {
                self.tool.shape_start = Some(world);
                self.tool.shape_end = Some(world);
            }
            SamplePhase::Move => {
                if self.tool.shape_start.is_some() {
                    self.tool.shape_end = Some(world);
                }
            }
            SamplePhase::End => {
                if let (Some(a), Some(b)) = (self.tool.shape_start, self.tool.shape_end) {
                    if a.distance(b) > 2.0 {
                        let color = self.settings.text_color; // shape draws in the user's chosen text/ink color
                        let kind = match self.settings.active_shape {
                            ActiveShape::Rectangle => ShapeKind::Rectangle,
                            ActiveShape::Ellipse => ShapeKind::Ellipse,
                            ActiveShape::Line => ShapeKind::Line,
                        };
                        let shape = Shape {
                            kind,
                            a,
                            b,
                            stroke_width: self.settings.stroke_width.max(1.0),
                            filled: self.settings.shape_filled && kind != ShapeKind::Line,
                            color,
                        };
                        self.add_object(Object::Shape(shape));
                    }
                }
                self.tool.shape_start = None;
                self.tool.shape_end = None;
            }
            SamplePhase::Cancel => {
                self.tool.shape_start = None;
                self.tool.shape_end = None;
            }
        }
    }

    fn handle_note(&mut self, world: glam::Vec2, s: InkSample) {
        if s.phase != SamplePhase::Begin {
            return;
        }
        // Affine semantics: clicking an existing note focuses it for typing;
        // clicking empty space drops a new sticky AND focuses it.
        if let Some(id) = self.pick_subnote_at_world(world) {
            if let Some(Object::SubNote(n)) = self.document.get(id) {
                if n.is_inline() {
                    self.begin_inline_note_edit(id, Some(world));
                    return;
                }
                // Linked / index → behave like the Select-tool click.
                self.handle_subnote_click(id, world);
                return;
            }
        }
        let note = SubNote::new_inline(world, String::new());
        let id = self.add_object(Object::SubNote(note));
        // Drop straight into edit mode — Affine never makes you pick a second
        // tool just to type into the sticky you just placed.
        self.begin_inline_note_edit(id, None);
    }

    /// React to a click-without-drag on a sub-note. Inline = open the in-place
    /// text editor; linked = jump to the linked file; index = switch to the
    /// row the user clicked inside the body picker.
    fn handle_subnote_click(&mut self, id: ObjectId, world: Vec2) {
        let Some(Object::SubNote(n)) = self.document.get(id).cloned() else { return };
        if n.is_index {
            if let Some(path) = self.index_card_row_at(&n, world) {
                self.switch_to_file(path);
            }
            return;
        }
        if n.is_linked() {
            if let Some(ws) = &self.workspace {
                let target = ws.root.join(&n.target_file);
                self.switch_to_file(target);
            }
            return;
        }
        if n.is_inline() {
            self.begin_inline_note_edit(id, Some(world));
        }
    }

    /// World→row mapping for an index card. Returns the target file the user
    /// clicked, or `None` when the click landed on a heading row or empty
    /// space.
    fn index_card_row_at(&self, n: &SubNote, world: Vec2) -> Option<PathBuf> {
        let body = n.body_rect();
        if world.x < body.min.x || world.x > body.max.x
            || world.y < body.min.y || world.y > body.max.y
        {
            return None;
        }
        let entries = self.resolve_index_entries(n);
        if entries.is_empty() {
            return None;
        }
        let row_h = 22.0_f32;
        let idx = ((world.y - body.min.y) / row_h).floor() as i32;
        if idx < 0 {
            return None;
        }
        match entries.get(idx as usize) {
            Some(ResolvedIndexEntry::File { path, .. }) => Some(path.clone()),
            _ => None,
        }
    }

    /// Resolve which `.writee` paths an index card should render, as plain
    /// Resolve the full ordered list of rows (files + headings) for an index
    /// card. When the card has no curated entries, fall back to "every file
    /// in the workspace" (welcome behaviour).
    fn resolve_index_entries(&self, n: &SubNote) -> Vec<ResolvedIndexEntry> {
        let Some(ws) = self.workspace.as_ref() else { return Vec::new() };
        let entries: Option<Vec<writee_core::IndexEntry>> = n
            .index_entries
            .clone()
            .or_else(|| {
                n.index_files
                    .as_ref()
                    .map(|files| files.iter().map(|f| writee_core::IndexEntry::File { file: f.clone() }).collect())
            });
        let raw = match entries {
            Some(v) if !v.is_empty() => v,
            _ => ws
                .list_files()
                .into_iter()
                .filter_map(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| writee_core::IndexEntry::File { file: s.to_string() })
                })
                .collect(),
        };
        raw.into_iter()
            .filter_map(|e| match e {
                writee_core::IndexEntry::File { file } => {
                    let path = ws.root.join(&file);
                    if path.exists() {
                        Some(ResolvedIndexEntry::File {
                            path,
                            display: display_name_for_file(&file),
                        })
                    } else {
                        None
                    }
                }
                writee_core::IndexEntry::Heading { text } => {
                    Some(ResolvedIndexEntry::Heading { text })
                }
            })
            .collect()
    }

    fn begin_inline_note_edit(&mut self, id: ObjectId, click_world: Option<Vec2>) {
        // Finish any other edit first.
        self.finish_text_edit();
        self.finish_inline_note_edit();
        let Some(Object::SubNote(n)) = self.document.get_mut(id) else { return };
        if n.inline_content.is_none() {
            // First time editing → upgrade to inline with empty content.
            n.inline_content = Some(String::new());
        }
        n.clamp_cursor();
        if let Some(world) = click_world {
            // Place the caret at the click position by counting lines/cols.
            let body = n.body_rect();
            let rel_y = (world.y - body.min.y).max(0.0);
            let line_h = 18.0; // matches the body font we render at
            let line = (rel_y / line_h).floor() as usize;
            let rel_x = (world.x - body.min.x).max(0.0);
            let char_w = 18.0 * 0.55;
            let col_target = (rel_x / char_w).round() as usize;
            if let Some(text) = n.inline_content.as_ref() {
                let line_start = text
                    .match_indices('\n')
                    .nth(line.saturating_sub(1))
                    .map(|(i, _)| i + 1)
                    .unwrap_or(0);
                let line_end = text[line_start..]
                    .find('\n')
                    .map(|i| line_start + i)
                    .unwrap_or(text.len());
                let mut consumed = 0usize;
                let mut new_cursor = line_end;
                for (i, _) in text[line_start..line_end].char_indices() {
                    if consumed == col_target {
                        new_cursor = line_start + i;
                        break;
                    }
                    consumed += 1;
                }
                n.cursor = new_cursor;
                n.clamp_cursor();
            }
        }
        self.tool.editing_note = Some(id);
        self.mark_doc_dirty();
    }

    /// Promote an inline sticky into its own `.writee` file. The note's
    /// `inline_content` becomes the new file's markdown source, and the
    /// in-canvas card is rewritten as a `target_file` link to it.
    fn convert_note_inline_to_linked(&mut self, id: ObjectId) {
        let Some(ws) = self.workspace.as_ref().cloned() else { return };
        let Some(Object::SubNote(mut n)) = self.document.get(id).cloned() else { return };
        if !n.is_inline() || n.locked {
            return;
        }
        let stem = if n.title.is_empty() { "note".to_string() } else { slugify(&n.title) };
        // Avoid filename collisions.
        let mut file_name = format!("{stem}.writee");
        let mut k = 2usize;
        while ws.root.join(&file_name).exists() {
            file_name = format!("{stem}-{k}.writee");
            k += 1;
        }
        let path_on_disk = ws.root.join(&file_name);
        let content = n.inline_content.clone().unwrap_or_default();
        if let Ok(store) = DocStore::open(&path_on_disk) {
            let _ = store.set_title(&n.title);
            let _ = store.set_document_mode(DocumentMode::Markdown);
            let _ = store.set_meta(markdown::META_KEY_MARKDOWN_SOURCE, &content);
        }
        n.inline_content = None;
        n.target_file = file_name;
        n.mode = writee_core::NoteMode::Markdown;
        n.cursor = 0;
        // Drop any in-progress inline edit on this card.
        if self.tool.editing_note == Some(id) {
            self.tool.editing_note = None;
        }
        // Persist the rewritten card.
        if let Some(store) = &self.store {
            let _ = store.update(id, &Object::SubNote(n.clone()));
        }
        if let Some(slot) = self.document.get_mut(id) {
            *slot = Object::SubNote(n);
        }
        self.mark_doc_dirty();
    }

    /// Pull a linked file's markdown source back into the card body and
    /// delete the child file. No-op if the child file is also a canvas
    /// (we don't have a sane way to inline a whole canvas right now).
    fn convert_note_linked_to_inline(&mut self, id: ObjectId) {
        let Some(ws) = self.workspace.as_ref().cloned() else { return };
        let Some(Object::SubNote(mut n)) = self.document.get(id).cloned() else { return };
        if !n.is_linked() || n.locked {
            return;
        }
        let path = ws.root.join(&n.target_file);
        let content = DocStore::open(&path)
            .ok()
            .and_then(|s| {
                let mode = s.document_mode().unwrap_or(DocumentMode::Canvas);
                if mode != DocumentMode::Markdown {
                    log::warn!("inline-from-linked: {} is canvas; keeping linked", path.display());
                    return None;
                }
                s.get_meta(markdown::META_KEY_MARKDOWN_SOURCE).ok().flatten()
            });
        let Some(content) = content else { return };
        n.inline_content = Some(content);
        n.target_file.clear();
        n.cursor = 0;
        if let Some(store) = &self.store {
            let _ = store.update(id, &Object::SubNote(n.clone()));
        }
        if let Some(slot) = self.document.get_mut(id) {
            *slot = Object::SubNote(n);
        }
        // Best-effort remove the now-orphaned file.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("writee-wal"));
        let _ = std::fs::remove_file(path.with_extension("writee-shm"));
        self.mark_doc_dirty();
    }

    fn open_index_editor(&mut self, id: ObjectId) {
        let Some(Object::SubNote(n)) = self.document.get(id) else { return };
        if !n.is_index || n.locked {
            return;
        }
        let Some(ws) = self.workspace.as_ref() else { return };
        let available_files: Vec<String> = ws
            .list_files()
            .iter()
            .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()))
            .collect();
        // Seed the working set from existing data:
        //   1. new-style index_entries (preferred)
        //   2. legacy index_files (migrate to entries)
        //   3. neither → empty (user explicitly picks what shows up)
        let entries: Vec<writee_core::IndexEntry> = if let Some(existing) = &n.index_entries {
            existing.clone()
        } else if let Some(legacy) = &n.index_files {
            legacy
                .iter()
                .map(|f| writee_core::IndexEntry::File { file: f.clone() })
                .collect()
        } else {
            Vec::new()
        };
        self.index_editor = Some(IndexEditorState {
            object_id: id,
            entries,
            add_file_selected: available_files.first().cloned(),
            available_files,
            new_heading_text: String::new(),
        });
    }

    fn commit_index_editor(&mut self) {
        let Some(state) = self.index_editor.take() else { return };
        if let Some(Object::SubNote(n)) = self.document.get_mut(state.object_id) {
            n.index_entries = Some(state.entries);
            // Clear the legacy field — `index_entries` is now authoritative
            // for this card and we don't want stale data shadowing it on load.
            n.index_files = None;
            let snapshot = Object::SubNote(n.clone());
            if let Some(store) = &self.store {
                let _ = store.update(state.object_id, &snapshot);
            }
        }
        self.mark_doc_dirty();
    }

    /// Flip a linked card's child file between canvas and markdown mode
    /// without having to open it first. Initialises an empty markdown
    /// source when switching to markdown for the first time.
    fn set_linked_note_mode(&mut self, id: ObjectId, want_markdown: bool) {
        let Some(ws) = self.workspace.as_ref().cloned() else { return };
        let Some(Object::SubNote(n)) = self.document.get_mut(id) else { return };
        if !n.is_linked() {
            return;
        }
        let path = ws.root.join(&n.target_file);
        let Ok(store) = DocStore::open(&path) else { return };
        if want_markdown {
            let _ = store.set_document_mode(DocumentMode::Markdown);
            if store
                .get_meta(markdown::META_KEY_MARKDOWN_SOURCE)
                .ok()
                .flatten()
                .is_none()
            {
                let _ = store.set_meta(
                    markdown::META_KEY_MARKDOWN_SOURCE,
                    &format!("# {}\n\n", n.title),
                );
            }
            n.mode = writee_core::NoteMode::Markdown;
        } else {
            let _ = store.set_document_mode(DocumentMode::Canvas);
            n.mode = writee_core::NoteMode::Canvas;
        }
        let snapshot = Object::SubNote(n.clone());
        if let Some(store) = &self.store {
            let _ = store.update(id, &snapshot);
        }
        // If the user is currently in that file, refresh the in-memory mode.
        if self.workspace.as_ref().map(|w| w.current_file.clone()) == Some(path.clone()) {
            self.md = load_markdown_state(&store);
        }
        self.mark_doc_dirty();
    }

    fn toggle_note_index(&mut self, id: ObjectId) {
        let Some(Object::SubNote(n)) = self.document.get_mut(id) else { return };
        if n.locked {
            return;
        }
        n.is_index = !n.is_index;
        // Index cards are larger by convention to fit the file list.
        if n.is_index && (n.size.x < 320.0 || n.size.y < 360.0) {
            n.size = Vec2::new(n.size.x.max(320.0), n.size.y.max(360.0));
        }
        let snapshot = Object::SubNote(n.clone());
        if let Some(store) = &self.store {
            let _ = store.update(id, &snapshot);
        }
        self.mark_doc_dirty();
    }

    fn finish_inline_note_edit(&mut self) {
        let Some(id) = self.tool.editing_note.take() else { return };
        // Persist current state. Empty inline notes are kept (the user might
        // type into them later); they're cheap and the visual placeholder
        // tells the user where they are.
        if let Some(store) = &self.store {
            if let Some(obj) = self.document.get(id).cloned() {
                let _ = store.update(id, &obj);
            }
        }
        self.mark_doc_dirty();
    }

    fn handle_index_tool(&mut self, world: glam::Vec2, s: InkSample) {
        if s.phase != SamplePhase::Begin {
            return;
        }
        // If the click lands on an existing index card, just focus the
        // file-list editor on it; otherwise drop a fresh, user-owned index.
        if let Some(id) = self.pick_subnote_at_world(world) {
            if let Some(Object::SubNote(n)) = self.document.get(id) {
                if n.is_index {
                    self.open_index_editor(id);
                    return;
                }
                self.handle_subnote_click(id, world);
                return;
            }
        }
        let mut card = SubNote::new_inline(world, String::new());
        card.is_index = true;
        // Default to a curated empty list so the welcome's "show all" doesn't
        // leak in — the user explicitly chose to make this index.
        card.index_entries = Some(Vec::new());
        card.title = "Index".into();
        if card.size.x < 320.0 || card.size.y < 360.0 {
            card.size = glam::Vec2::new(card.size.x.max(320.0), card.size.y.max(360.0));
        }
        let id = self.add_object(Object::SubNote(card));
        self.open_index_editor(id);
    }

    fn handle_link(&mut self, world: glam::Vec2, s: InkSample) {
        match s.phase {
            SamplePhase::Begin => {
                if let Some((id, anchor, pos)) =
                    self.document.pick_anchor(world, self.anchor_slack())
                {
                    self.tool.link_in_progress = Some((id, anchor, pos, world));
                }
            }
            SamplePhase::Move => {
                if let Some((_, _, _, ref mut cur)) = self.tool.link_in_progress {
                    *cur = world;
                }
            }
            SamplePhase::End => {
                if let Some((src_id, src_anchor, _src_pos, end_world)) =
                    self.tool.link_in_progress.take()
                {
                    if let Some((dst_id, dst_anchor, _)) =
                        self.document.pick_anchor(end_world, self.anchor_slack())
                    {
                        if dst_id != src_id {
                            let link = Link::new(
                                LinkEnd { object_id: src_id, anchor: src_anchor },
                                LinkEnd { object_id: dst_id, anchor: dst_anchor },
                            );
                            self.add_object(Object::Link(link));
                        }
                    }
                }
            }
            SamplePhase::Cancel => {
                self.tool.link_in_progress = None;
            }
        }
    }

    fn anchor_slack(&self) -> f32 {
        12.0 / self.viewport.zoom.max(0.05)
    }

    fn handle_ink(&mut self, world: Vec2, s: InkSample, color: InkColor) {
        let pt = InkPoint {
            x: world.x,
            y: world.y,
            pressure: s.pressure.clamp(0.0, 1.0),
            tilt_x: s.tilt_x,
            tilt_y: s.tilt_y,
            t_ms: s.t_ms,
        };
        match s.phase {
            SamplePhase::Begin => {
                self.wet.clear();
                self.wet.push(pt);
                self.drawing = true;
                self.settings.ink_color = color;
            }
            SamplePhase::Move if self.drawing => {
                if let Some(last) = self.wet.last() {
                    let dpx = (Vec2::new(last.x - pt.x, last.y - pt.y)) * self.viewport.zoom;
                    if dpx.length_squared() < 0.25 {
                        return;
                    }
                }
                self.wet.push(pt);
            }
            SamplePhase::End if self.drawing => {
                self.wet.push(pt);
                self.commit_wet_stroke(color);
                self.drawing = false;
            }
            SamplePhase::Cancel => {
                self.wet.clear();
                self.drawing = false;
            }
            _ => {}
        }
    }

    fn commit_wet_stroke(&mut self, color: InkColor) {
        if self.wet.len() < 2 {
            self.wet.clear();
            return;
        }
        // Pen takes the user's chosen text/ink color from the picker;
        // highlighter stays the dedicated semi-transparent yellow so it
        // visibly differs from regular ink.
        let theme = self.fonts_theme();
        let rgba = match color {
            InkColor::Pen => self.settings.text_color,
            InkColor::Highlighter => theme.highlight,
        };
        let mut s = Stroke::with_rgba(self.settings.stroke_width, color.to_idx(), rgba);
        s.points = std::mem::take(&mut self.wet);
        self.add_object(Object::Stroke(s));
    }

    fn handle_eraser(&mut self, world: Vec2, s: InkSample) {
        match s.phase {
            SamplePhase::Begin => {
                self.tool.eraser_dragging = true;
                self.erase_at(world);
            }
            SamplePhase::Move if self.tool.eraser_dragging => self.erase_at(world),
            SamplePhase::End | SamplePhase::Cancel => self.tool.eraser_dragging = false,
            _ => {}
        }
    }

    fn erase_at(&mut self, world: Vec2) {
        while let Some(id) = self.document.pick(world, self.settings.eraser_radius) {
            self.remove_object(id);
        }
    }

    fn handle_arrow(&mut self, world: Vec2, s: InkSample) {
        match s.phase {
            SamplePhase::Begin => {
                self.tool.arrow_start = Some(world);
                self.tool.arrow_end = Some(world);
            }
            SamplePhase::Move => {
                if self.tool.arrow_start.is_some() {
                    self.tool.arrow_end = Some(world);
                }
            }
            SamplePhase::End => {
                if let (Some(start), Some(end)) = (self.tool.arrow_start, self.tool.arrow_end) {
                    if start.distance(end) > 3.0 {
                        self.add_object(Object::Arrow(Arrow::new(start, end)));
                    }
                }
                self.tool.arrow_start = None;
                self.tool.arrow_end = None;
            }
            SamplePhase::Cancel => {
                self.tool.arrow_start = None;
                self.tool.arrow_end = None;
            }
        }
    }

    fn handle_text_tool(&mut self, world: Vec2, s: InkSample) {
        if s.phase != SamplePhase::Begin {
            return;
        }
        if let Some(id) = self.tool.editing_text {
            if let Some(Object::TextBox(tb)) = self.document.get(id) {
                if inside_bbox(world, tb.bbox()) {
                    return;
                }
            }
            self.finish_text_edit();
        }
        if let Some(id) = self.pick_textbox_at(world) {
            if let Some(Object::TextBox(tb)) = self.document.get_mut(id) {
                self.tool.edit_text_before = Some(tb.content.clone());
                // Place caret near the click (rough char-grid). Anything more
                // precise needs glyphon layout, which we'd query at render time.
                let rel = world - tb.origin;
                let line = ((rel.y / (tb.font_size * 1.25)).floor() as i32).max(0) as usize;
                let col = ((rel.x / (tb.font_size * 0.55)).round() as i32).max(0) as usize;
                let line_start = tb
                    .content
                    .match_indices('\n')
                    .nth(line.saturating_sub(1))
                    .map(|(i, _)| i + 1)
                    .unwrap_or(0);
                let line_end = tb.content[line_start..]
                    .find('\n')
                    .map(|i| line_start + i)
                    .unwrap_or(tb.content.len());
                let mut consumed = 0usize;
                let mut new_cursor = line_end;
                for (i, _) in tb.content[line_start..line_end].char_indices() {
                    if consumed == col {
                        new_cursor = line_start + i;
                        break;
                    }
                    consumed += 1;
                }
                tb.cursor = new_cursor;
                tb.clamp_cursor();
                self.tool.editing_text = Some(id);
            }
        } else {
            let mut tb = TextBox::new(world, self.settings.text_size);
            tb.font_name = Some(self.fonts.resolve(self.settings.font_slot).to_string());
            let id = self.document.add(Object::TextBox(tb));
            if let Some(store) = &self.store {
                if let Some(obj) = self.document.get(id) {
                    let _ = store.insert(id, obj);
                }
            }
            self.mark_doc_dirty();
            self.tool.edit_text_before = None;
            self.tool.editing_text = Some(id);
        }
    }

    fn pick_subnote_at_world(&self, world: Vec2) -> Option<ObjectId> {
        for (id, obj) in self.document.objects() {
            if let Object::SubNote(_) = obj {
                if inside_bbox(world, obj.bbox()) {
                    return Some(id);
                }
            }
        }
        None
    }

    fn pick_textbox_at(&self, world: Vec2) -> Option<ObjectId> {
        let mut best: Option<ObjectId> = None;
        for (id, obj) in self.document.objects() {
            if let Object::TextBox(_) = obj {
                if inside_bbox(world, obj.bbox()) {
                    best = Some(id);
                }
            }
        }
        best
    }

    fn finish_text_edit(&mut self) {
        let Some(id) = self.tool.editing_text.take() else { return };
        let before = self.tool.edit_text_before.take();
        let Some(Object::TextBox(tb)) = self.document.get(id).cloned() else { return };

        match before {
            None => {
                // Newly created box: only keep it if non-empty.
                if tb.content.is_empty() {
                    self.document.remove(id);
                    if let Some(store) = &self.store {
                        let _ = store.delete(id);
                    }
                } else {
                    self.undo.push(Op::Add { id, object: Object::TextBox(tb) });
                }
            }
            Some(before_text) => {
                if before_text == tb.content {
                    return; // no change
                }
                let mut before_box = tb.clone();
                before_box.content = before_text;
                self.undo.push(Op::Replace {
                    id,
                    before: Object::TextBox(before_box),
                    after: Object::TextBox(tb),
                });
            }
        }
        self.mark_doc_dirty();
    }

    fn handle_select(&mut self, world: Vec2, s: InkSample) {
        match s.phase {
            SamplePhase::Begin => {
                let hit = self.document.pick(world, PICK_SLACK);
                let multi = self.modifiers.shift_key() || self.modifiers.control_key();
                match hit {
                    Some(id) => {
                        if self.tool.selected.contains(&id) {
                            // Begin a drag without changing selection.
                        } else if multi {
                            self.tool.selected.push(id);
                        } else {
                            self.tool.selected.clear();
                            self.tool.selected.push(id);
                        }
                        self.tool.drag_origin_world = Some(world);
                        self.tool.gesture_origin_world = Some(world);
                        self.tool.drag_active = false;
                        self.tool.drag_before = self
                            .tool
                            .selected
                            .iter()
                            .filter_map(|&id| self.document.get(id).cloned().map(|o| (id, o)))
                            .collect();
                        self.tool.marquee = None;
                        // Remember whether this gesture started on a sub-note —
                        // if the user releases without dragging, we treat it as
                        // a click-to-open.
                        if let Some(Object::SubNote(_)) = self.document.get(id) {
                            self.tool.click_target_subnote = Some(id);
                        } else {
                            self.tool.click_target_subnote = None;
                        }
                    }
                    None => {
                        // Empty space: start marquee, clearing selection if no
                        // modifier held.
                        if !multi {
                            self.tool.selected.clear();
                        }
                        self.tool.drag_before.clear();
                        self.tool.drag_origin_world = None;
                        self.tool.marquee = Some((world, world));
                    }
                }
            }
            SamplePhase::Move => {
                if let (Some(origin), false) =
                    (self.tool.drag_origin_world, self.tool.marquee.is_some())
                {
                    // Hold off on translating until the cursor moves past a
                    // small threshold — otherwise hand jitter on a click
                    // teleports the card and the click-to-edit gesture
                    // never fires.
                    if !self.tool.drag_active {
                        let gesture_origin =
                            self.tool.gesture_origin_world.unwrap_or(origin);
                        let drift_world = (world - gesture_origin).length();
                        let threshold = 4.0 / self.viewport.zoom.max(0.05);
                        if drift_world < threshold {
                            return;
                        }
                        self.tool.drag_active = true;
                    }
                    let delta = world - origin;
                    for &id in &self.tool.selected {
                        if let Some(Object::SubNote(n)) = self.document.get(id) {
                            if n.locked {
                                continue;
                            }
                        }
                        self.document.translate(id, delta);
                    }
                    self.tool.drag_origin_world = Some(world);
                    self.mark_doc_dirty();
                } else if let Some((start, _)) = self.tool.marquee {
                    self.tool.marquee = Some((start, world));
                }
            }
            SamplePhase::End => {
                if self.tool.marquee.is_some() {
                    if let Some((a, b)) = self.tool.marquee.take() {
                        let min = a.min(b);
                        let max = a.max(b);
                        let hits = self.document.pick_in_rect(min, max);
                        if !hits.is_empty() {
                            self.tool.selected = hits;
                        } else {
                            self.tool.selected.clear();
                        }
                    }
                } else if !self.tool.drag_before.is_empty() {
                    // Drag committed: persist + record one Replace per object.
                    // (When the threshold was never crossed, no objects moved
                    // and `moved` stays false → handle_subnote_click runs.)
                    let befores: Vec<(ObjectId, Object)> =
                        std::mem::take(&mut self.tool.drag_before);
                    let mut moved = false;
                    for (id, before) in befores {
                        if let Some(after) = self.document.get(id).cloned() {
                            if positions_differ(&before, &after) {
                                moved = true;
                                if let Some(store) = &self.store {
                                    if let Err(e) = store.update(id, &after) {
                                        log::error!("persist update {id}: {e:?}");
                                    }
                                }
                                self.undo.push(Op::Replace { id, before, after });
                            }
                        }
                    }
                    // If nothing moved and the gesture started on a sub-note,
                    // treat as a click. Inline → start editing the body.
                    // Linked → open the file. Index card body → switch to the
                    // file the user clicked inside the picker.
                    if !moved {
                        if let Some(id) = self.tool.click_target_subnote.take() {
                            self.handle_subnote_click(id, world);
                        }
                    } else {
                        self.tool.click_target_subnote = None;
                    }
                }
                self.tool.drag_origin_world = None;
                self.tool.gesture_origin_world = None;
                self.tool.drag_active = false;
            }
            SamplePhase::Cancel => {
                self.tool.drag_origin_world = None;
                self.tool.gesture_origin_world = None;
                self.tool.drag_active = false;
                self.tool.drag_before.clear();
                self.tool.marquee = None;
            }
        }
    }

    // ------------------------------------------------------------------
    // Object mutation (with undo + persistence)
    // ------------------------------------------------------------------

    fn add_object(&mut self, obj: Object) -> ObjectId {
        let id = self.document.add(obj.clone());
        if let Some(store) = &self.store {
            if let Err(e) = store.insert(id, &obj) {
                log::error!("persist insert {id}: {e:?}");
            }
        }
        self.undo.push(Op::Add { id, object: obj });
        self.mark_doc_dirty();
        id
    }

    fn remove_object(&mut self, id: ObjectId) -> bool {
        // Honour the per-note `locked` flag (welcome workspace card et al.).
        if let Some(Object::SubNote(n)) = self.document.get(id) {
            if n.locked {
                log::info!("refusing to delete locked note {id}");
                return false;
            }
        }
        let Some(obj) = self.document.remove(id) else { return false };
        if let Some(store) = &self.store {
            if let Err(e) = store.delete(id) {
                log::error!("persist delete {id}: {e:?}");
            }
        }
        self.undo.push(Op::Remove { id, object: obj });
        self.mark_doc_dirty();
        true
    }

    fn apply_undo(&mut self) {
        let Some(op) = self.undo.pop_undo() else { return };
        match op {
            Op::Add { id, object } => {
                self.document.remove(id);
                if let Some(store) = &self.store {
                    let _ = store.delete(id);
                }
                self.undo.push_redo(Op::Add { id, object });
            }
            Op::Remove { id, object } => {
                self.document.reinsert(id, object.clone());
                if let Some(store) = &self.store {
                    let _ = store.insert(id, &object);
                }
                self.undo.push_redo(Op::Remove { id, object });
            }
            Op::Replace { id, before, after } => {
                if let Some(slot) = self.document.get_mut(id) {
                    *slot = before.clone();
                }
                if let Some(store) = &self.store {
                    let _ = store.update(id, &before);
                }
                self.undo.push_redo(Op::Replace { id, before, after });
            }
        }
        self.mark_doc_dirty();
    }

    fn apply_redo(&mut self) {
        let Some(op) = self.undo.pop_redo() else { return };
        match op {
            Op::Add { id, object } => {
                self.document.reinsert(id, object.clone());
                if let Some(store) = &self.store {
                    let _ = store.insert(id, &object);
                }
                self.undo.push_undo_without_clear(Op::Add { id, object });
            }
            Op::Remove { id, object } => {
                self.document.remove(id);
                if let Some(store) = &self.store {
                    let _ = store.delete(id);
                }
                self.undo.push_undo_without_clear(Op::Remove { id, object });
            }
            Op::Replace { id, before, after } => {
                if let Some(slot) = self.document.get_mut(id) {
                    *slot = after.clone();
                }
                if let Some(store) = &self.store {
                    let _ = store.update(id, &after);
                }
                self.undo.push_undo_without_clear(Op::Replace { id, before, after });
            }
        }
        self.mark_doc_dirty();
    }

    // ------------------------------------------------------------------
    // Text editing
    // ------------------------------------------------------------------

    fn text_edit_input_char(&mut self, ch: char) {
        let Some(id) = self.tool.editing_text else { return };
        if let Some(Object::TextBox(tb)) = self.document.get_mut(id) {
            tb.insert_at_cursor(ch);
            if let Some(store) = &self.store {
                let snapshot = Object::TextBox(tb.clone());
                let _ = store.update(id, &snapshot);
            }
            self.mark_doc_dirty();
        }
    }

    fn text_edit_backspace(&mut self) {
        let Some(id) = self.tool.editing_text else { return };
        if let Some(Object::TextBox(tb)) = self.document.get_mut(id) {
            if tb.backspace_at_cursor() {
                if let Some(store) = &self.store {
                    let snapshot = Object::TextBox(tb.clone());
                    let _ = store.update(id, &snapshot);
                }
                self.mark_doc_dirty();
            }
        }
    }

    fn text_edit_newline(&mut self) {
        self.text_edit_input_char('\n');
    }

    /// Apply a cursor-only mutation that doesn't touch persisted content but
    /// does need a redraw so the caret moves on-screen.
    fn text_edit_nav(&mut self, mutate: impl FnOnce(&mut TextBox)) {
        let Some(id) = self.tool.editing_text else { return };
        if let Some(Object::TextBox(tb)) = self.document.get_mut(id) {
            mutate(tb);
            self.mark_doc_dirty();
        }
    }

    // --- Inline sub-note editing (mirrors the text-edit helpers above) ---

    fn note_edit_input_char(&mut self, ch: char) {
        let Some(id) = self.tool.editing_note else { return };
        if let Some(Object::SubNote(n)) = self.document.get_mut(id) {
            n.insert_at_cursor(ch);
            if let Some(store) = &self.store {
                let _ = store.update(id, &Object::SubNote(n.clone()));
            }
            self.mark_doc_dirty();
        }
    }

    fn note_edit_backspace(&mut self) {
        let Some(id) = self.tool.editing_note else { return };
        if let Some(Object::SubNote(n)) = self.document.get_mut(id) {
            if n.backspace_at_cursor() {
                if let Some(store) = &self.store {
                    let _ = store.update(id, &Object::SubNote(n.clone()));
                }
                self.mark_doc_dirty();
            }
        }
    }

    fn note_edit_nav(&mut self, mutate: impl FnOnce(&mut SubNote)) {
        let Some(id) = self.tool.editing_note else { return };
        if let Some(Object::SubNote(n)) = self.document.get_mut(id) {
            mutate(n);
            self.mark_doc_dirty();
        }
    }

    // ------------------------------------------------------------------
    // Geometry assembly
    // ------------------------------------------------------------------

    /// Ensure thumbnail caches for every visible sub-note are fresh on-disk.
    /// Cheap when the file hasn't changed (mtime check only).
    fn refresh_subnote_thumbnails(&mut self) {
        let Some(ws) = self.workspace.as_ref() else { return };
        let targets: Vec<PathBuf> = self
            .document
            .objects()
            .filter_map(|(_, obj)| match obj {
                Object::SubNote(n) => Some(ws.root.join(&n.target_file)),
                _ => None,
            })
            .collect();
        for path in targets {
            let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            let needs_reload = match self.thumbnails.get(&path) {
                None => true,
                Some(entry) => entry.mtime != mtime,
            };
            if !needs_reload {
                continue;
            }
            if !path.exists() {
                self.thumbnails.remove(&path);
                continue;
            }
            // Open read-only-ish: DocStore opens RW, fine for a quick load.
            match DocStore::open(&path).and_then(|s| s.load_all()) {
                Ok(doc) => {
                    self.thumbnails
                        .insert(path.clone(), ThumbnailEntry { mtime, doc });
                }
                Err(e) => log::debug!("thumbnail load {} failed: {e:?}", path.display()),
            }
        }
    }

    /// Append embedded child geometry for each sub-note onto `out`. Caller is
    /// responsible for first running `refresh_subnote_thumbnails`.
    fn append_subnote_thumbnails(&self, out: &mut Vec<InkVertex>) {
        let Some(ws) = self.workspace.as_ref() else { return };
        let theme = self.fonts_theme();
        let append = |chunk: Vec<InkVertex>, sink: &mut Vec<InkVertex>| {
            if chunk.is_empty() {
                return;
            }
            if !sink.is_empty() {
                let last = *sink.last().unwrap();
                let first = chunk[0];
                sink.push(last);
                sink.push(first);
            }
            sink.extend(chunk);
        };
        for (_, obj) in self.document.objects() {
            let Object::SubNote(n) = obj else { continue };
            let path = ws.root.join(&n.target_file);
            let Some(entry) = self.thumbnails.get(&path) else { continue };
            let (scale, offset) = match thumbnail_transform(n, &entry.doc) {
                Some(t) => t,
                None => continue,
            };
            // Re-tessellate each child object pre-transformed.
            for (_, child) in entry.doc.objects() {
                let verts =
                    tessellate_child_for_thumbnail(child, self.settings.pressure_sensitive, theme);
                let xformed = transform_verts(&verts, scale, offset);
                append(xformed, out);
            }
        }
    }

    fn build_ink_geometry(&mut self) -> Vec<InkVertex> {
        self.refresh_subnote_thumbnails();
        let theme = *self.fonts_theme();
        self.cache.ensure_fresh(&self.document, &self.settings, &theme);
        let mut out = self.cache.verts.clone();
        self.append_subnote_thumbnails(&mut out);

        let append = |verts: Vec<InkVertex>, sink: &mut Vec<InkVertex>| {
            if verts.is_empty() {
                return;
            }
            if !sink.is_empty() {
                let last = *sink.last().unwrap();
                let first = verts[0];
                sink.push(last);
                sink.push(first);
            }
            sink.extend(verts);
        };

        // Wet stroke preview — match the color the commit will use.
        if self.drawing && self.wet.len() >= 2 {
            let rgba = match self.tool.current {
                Some(Tool::Highlighter) => theme.highlight,
                _ => self.settings.text_color,
            };
            append(
                tessellate_opts(
                    &self.wet,
                    self.settings.stroke_width,
                    rgba,
                    self.settings.pressure_sensitive,
                    self.settings.tilt_modulation,
                ),
                &mut out,
            );
        }

        // Wet arrow preview.
        if let (Some(start), Some(end)) = (self.tool.arrow_start, self.tool.arrow_end) {
            if start.distance(end) > 1.0 {
                append(
                    tessellate_arrow(&Arrow::new(start, end), theme.ink),
                    &mut out,
                );
            }
        }

        // Wet shape preview.
        if let (Some(a), Some(b)) = (self.tool.shape_start, self.tool.shape_end) {
            if a.distance(b) > 1.0 {
                let min = a.min(b);
                let max = a.max(b);
                match self.settings.active_shape {
                    ActiveShape::Rectangle => {
                        if self.settings.shape_filled {
                            append(tessellate_rect(min, max, self.settings.text_color), &mut out);
                        } else {
                            append(
                                tessellate_rect_outline(
                                    min,
                                    max,
                                    self.settings.stroke_width * 0.5,
                                    self.settings.text_color,
                                ),
                                &mut out,
                            );
                        }
                    }
                    ActiveShape::Ellipse => append(
                        tessellate_ellipse(min, max, self.settings.text_color, self.settings.shape_filled),
                        &mut out,
                    ),
                    ActiveShape::Line => append(
                        tessellate_line(a, b, self.settings.stroke_width, self.settings.text_color),
                        &mut out,
                    ),
                }
            }
        }

        // Wet link preview while drag-creating.
        if let Some((_id, _anchor, start, cur)) = self.tool.link_in_progress {
            if start.distance(cur) > 1.0 {
                append(
                    tessellate_segment_strip(start, cur, 0.8, COLOR_LINK),
                    &mut out,
                );
            }
        }

        // Anchor dots — visible whenever the Link tool is active so the user
        // sees where they can connect.
        if matches!(self.tool.current, Some(Tool::Link)) {
            let r = 4.0 / self.viewport.zoom.max(0.05);
            for (_, obj) in self.document.objects() {
                if !obj.is_anchorable() {
                    continue;
                }
                for anchor in Anchor::all() {
                    if let Some(pos) = obj.anchor_pos(anchor) {
                        append(
                            tessellate_ellipse(
                                pos - glam::Vec2::splat(r),
                                pos + glam::Vec2::splat(r),
                                COLOR_LINK,
                                true,
                            ),
                            &mut out,
                        );
                    }
                }
            }
        }

        // Selection outlines.
        for &id in &self.tool.selected {
            if let Some(obj) = self.document.get(id) {
                let bb = obj.bbox();
                let pad = 5.0 / self.viewport.zoom.max(0.05);
                append(
                    tessellate_rect_outline(
                        bb.min - Vec2::splat(pad),
                        bb.max + Vec2::splat(pad),
                        0.8,
                        theme.selection,
                    ),
                    &mut out,
                );
            }
        }

        // Editing-text outline + caret.
        if let Some(id) = self.tool.editing_text {
            if let Some(obj) = self.document.get(id) {
                let bb = obj.bbox();
                let pad = 3.0 / self.viewport.zoom.max(0.05);
                append(
                    tessellate_rect_outline(
                        bb.min - Vec2::splat(pad),
                        bb.max + Vec2::splat(pad),
                        0.5,
                        theme.selection,
                    ),
                    &mut out,
                );
                if let Object::TextBox(tb) = obj {
                    let caret_top = tb.cursor_world_pos();
                    let caret_bot = caret_top + Vec2::new(0.0, tb.font_size * 1.1);
                    let half_w = (1.0 / self.viewport.zoom.max(0.05)).max(0.4);
                    append(
                        tessellate_segment_strip(caret_top, caret_bot, half_w, theme.selection),
                        &mut out,
                    );
                }
            }
        }

        // Inline-note edit outline + caret.
        if let Some(id) = self.tool.editing_note {
            if let Some(Object::SubNote(n)) = self.document.get(id) {
                let body = n.body_rect();
                let pad = 2.0 / self.viewport.zoom.max(0.05);
                append(
                    tessellate_rect_outline(
                        body.min - Vec2::splat(pad),
                        body.max + Vec2::splat(pad),
                        0.4,
                        theme.selection,
                    ),
                    &mut out,
                );
                let caret_pos = note_caret_world_pos(n);
                let line_h = 16.0;
                let caret_bot = caret_pos + Vec2::new(0.0, line_h * 1.1);
                let half_w = (1.0 / self.viewport.zoom.max(0.05)).max(0.4);
                append(
                    tessellate_segment_strip(caret_pos, caret_bot, half_w, theme.selection),
                    &mut out,
                );
            }
        }

        // Marquee preview while dragging.
        if let Some((a, b)) = self.tool.marquee {
            let min = a.min(b);
            let max = a.max(b);
            if (max - min).length_squared() > 1.0 {
                append(
                    tessellate_rect_outline(min, max, 0.5, theme.marquee),
                    &mut out,
                );
            }
        }

        out
    }

    fn text_instances(&self) -> Vec<TextInstance> {
        let mut out = Vec::new();
        let ws_root = self.workspace.as_ref().map(|w| w.root.clone());
        // Reading order: sort sub-notes by Y, then X (Affine-style "number
        // under each note in the edgeless mode").
        let mut ordering: Vec<(ObjectId, Vec2)> = self
            .document
            .objects()
            .filter_map(|(id, obj)| match obj {
                Object::SubNote(n) => Some((id, n.origin)),
                _ => None,
            })
            .collect();
        ordering.sort_by(|a, b| {
            a.1.y
                .partial_cmp(&b.1.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.x.partial_cmp(&b.1.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        let order_of: std::collections::HashMap<ObjectId, usize> = ordering
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i + 1))
            .collect();
        for (obj_id, obj) in self.document.objects() {
            match obj {
                Object::TextBox(tb) => out.push(TextInstance::from_textbox(tb)),
                Object::SubNote(n) => {
                    // Reading-order badge for linked + index cards only.
                    // Inline stickies are headerless to stay out of the way
                    // of pure writing.
                    let order_num = order_of.get(&obj_id).copied().unwrap_or(0);
                    if order_num > 0 && n.has_title_bar() {
                        let badge = TextBox {
                            origin: n.origin + glam::Vec2::new(n.size.x - 22.0, 4.0),
                            font_size: 11.0,
                            content: order_num.to_string(),
                            font_name: None,
                            cursor: 0,
                        };
                        out.push(TextInstance::from_textbox(&badge));
                    }
                    // Title bar text — only for cards that actually have a
                    // title bar (linked + index). Inline stickies are
                    // headerless to match Affine.
                    if n.has_title_bar() && !n.title.is_empty() {
                        let title_box = TextBox {
                            origin: n.origin + glam::Vec2::new(SubNote::BODY_INSET, 4.0),
                            font_size: 16.0,
                            content: n.title.clone(),
                            font_name: None,
                            cursor: 0,
                        };
                        out.push(TextInstance::from_textbox(&title_box));
                    }

                    if n.is_index {
                        // Render the (possibly curated) entries inside the body.
                        if let Some(root) = &ws_root {
                            let body = n.body_rect();
                            let entries = self.resolve_index_entries(n);
                            let row_h = 22.0_f32;
                            let max_rows = ((body.max.y - body.min.y) / row_h).floor() as usize;
                            for (i, entry) in entries.iter().take(max_rows).enumerate() {
                                let (text, font_size) = match entry {
                                    ResolvedIndexEntry::File { path, display } => {
                                        let marker = if Some(path.clone())
                                            == self.workspace.as_ref().map(|w| w.current_file.clone())
                                        {
                                            "● "
                                        } else if path
                                            == &root.join(crate::workspace::WELCOME_FILE_NAME)
                                        {
                                            "★ "
                                        } else {
                                            "  "
                                        };
                                        (format!("{marker}{display}"), 14.0)
                                    }
                                    ResolvedIndexEntry::Heading { text } => {
                                        // Render as a slightly larger label so headings
                                        // stand out from file rows.
                                        (text.clone().to_uppercase(), 12.0)
                                    }
                                };
                                let row = TextBox {
                                    origin: body.min + glam::Vec2::new(0.0, i as f32 * row_h),
                                    font_size,
                                    content: text,
                                    font_name: None,
                                    cursor: 0,
                                };
                                out.push(TextInstance::from_textbox(&row));
                            }
                            if entries.is_empty() {
                                let row = TextBox {
                                    origin: body.min,
                                    font_size: 13.0,
                                    content: "(empty index — right-click → Edit index contents)"
                                        .into(),
                                    font_name: None,
                                    cursor: 0,
                                };
                                out.push(TextInstance::from_textbox(&row));
                            }
                        }
                    } else if let Some(content) = &n.inline_content {
                        // Inline sticky body. Each line goes through the
                        // markdown shortcut pass so `# heading`, `- bullet`,
                        // and `[x] check` render with the appropriate
                        // glyphs / sizes.
                        let body = n.body_rect();
                        let editing_this = self.tool.editing_note == Some(obj_id);
                        if content.is_empty() && !editing_this {
                            let placeholder = TextBox {
                                origin: body.min,
                                font_size: 16.0,
                                content: "(empty — click to write)".into(),
                                font_name: None,
                                cursor: 0,
                            };
                            out.push(TextInstance::from_textbox(&placeholder));
                        } else {
                            let lines = md_shortcuts::render_lines(content);
                            let mut y = body.min.y;
                            for line in lines {
                                let tb = TextBox {
                                    origin: glam::Vec2::new(body.min.x, y),
                                    font_size: line.font_size,
                                    content: line.text,
                                    font_name: None,
                                    cursor: 0,
                                };
                                out.push(TextInstance::from_textbox(&tb));
                                y += line.font_size * 1.25;
                            }
                        }
                    } else if n.is_linked() {
                        // Linked-file hint under the title.
                        let path_box = TextBox {
                            origin: n.origin + glam::Vec2::new(SubNote::BODY_INSET, 32.0),
                            font_size: 12.0,
                            content: format!("→ {}", n.target_file),
                            font_name: None,
                            cursor: 0,
                        };
                        out.push(TextInstance::from_textbox(&path_box));

                        // Live thumbnail text — scaled glyphon instances for
                        // each child TextBox/SubNote-title.
                        if let Some(root) = &ws_root {
                            let path = root.join(&n.target_file);
                            if let Some(entry) = self.thumbnails.get(&path) {
                                if let Some((scale, offset)) = thumbnail_transform(n, &entry.doc) {
                                    for (_, child) in entry.doc.objects() {
                                        if let Object::TextBox(tb) = child {
                                            let mut clone = tb.clone();
                                            clone.origin = tb.origin * scale + offset;
                                            clone.font_size = (tb.font_size * scale).max(1.0);
                                            out.push(TextInstance::from_textbox(&clone));
                                        } else if let Object::SubNote(child_sn) = child {
                                            let title_box = TextBox {
                                                origin: (child_sn.origin + glam::Vec2::new(4.0, 2.0))
                                                    * scale
                                                    + offset,
                                                font_size: (14.0 * scale).max(1.0),
                                                content: child_sn.title.clone(),
                                                font_name: None,
                                                cursor: 0,
                                            };
                                            out.push(TextInstance::from_textbox(&title_box));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // Workspace / file ops
    // ------------------------------------------------------------------

    fn switch_to_file(&mut self, path: PathBuf) {
        match DocStore::open(&path) {
            Ok(store) => match store.load_all() {
                Ok(doc) => {
                    log::info!("switched to {} ({} objects)", path.display(), doc.len());
                    self.notes_locked = store.locked_notes().unwrap_or(false);
                    self.md = load_markdown_state(&store);
                    self.document = doc;
                    self.store = Some(store);
                    if let Some(ws) = &mut self.workspace {
                        ws.current_file = path;
                    }
                    self.undo = UndoStack::new();
                    self.tool.selected.clear();
                    self.tool.editing_text = None;
                    self.tool.editing_note = None;
                    self.tool.edit_text_before = None;
                    self.context_menu = None;
                    self.index_editor = None;
                    self.clipboard.clear();
                    self.index_initial_view_set = false;
                    // File switched → any in-progress recovery state belongs
                    // to the previous file; drop it so we don't false-prompt
                    // on next launch.
                    if let Some(ws) = &self.workspace {
                        recovery::clear(&ws.root);
                    }
                    self.last_recovery_write = None;
                    self.last_recovery_sig = None;
                    self.mark_doc_dirty();
                    self.update_title();
                    self.save_config();
                }
                Err(e) => log::error!("loading {} failed: {e:?}", path.display()),
            },
            Err(e) => log::error!("opening {} failed: {e:?}", path.display()),
        }
    }

    fn rename_file(&mut self, old: PathBuf, new_stem: String) {
        let Some(parent) = old.parent() else { return };
        let safe = slugify(&new_stem);
        let new_path = parent.join(format!("{safe}.writee"));
        if new_path == old { return; }
        if new_path.exists() {
            log::warn!("rename: {} already exists", new_path.display());
            return;
        }
        // Close the store before renaming so SQLite lets go of the file handle.
        if let Some(ws) = &self.workspace {
            if ws.current_file == old {
                self.store = None;
            }
        }
        if let Err(e) = std::fs::rename(&old, &new_path) {
            log::error!("rename failed: {e:?}");
            return;
        }
        // Move WAL/SHM sidecars too so SQLite picks them up cleanly.
        for ext in ["writee-wal", "writee-shm"] {
            let src = old.with_extension(ext);
            if src.exists() {
                let _ = std::fs::rename(&src, new_path.with_extension(ext));
            }
        }
        // Rewrite every incoming link in every other file in the workspace
        // so SubNote.target_file (and index entries) still resolve after the
        // rename. Without this, every parent that linked the renamed file
        // would point at a dead path.
        let old_name = old.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let new_name = new_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if let Some(ws) = &self.workspace {
            match ws.rewrite_links(&old_name, &new_name) {
                Ok(n) if n > 0 => log::info!("rewrote links in {n} file(s)"),
                Ok(_) => {}
                Err(e) => log::warn!("link rewrite failed: {e:?}"),
            }
        }
        // Invalidate caches that referenced the old name.
        self.thumbnails.clear();
        self.cache.mark_dirty();
        if let Some(ws) = &mut self.workspace {
            if ws.current_file == old {
                ws.current_file = new_path.clone();
                self.switch_to_file(new_path);
            }
        }
    }

    fn delete_file(&mut self, path: PathBuf) {
        let Some(ws) = self.workspace.as_ref() else { return };
        if ws.current_file == path {
            log::warn!("refusing to delete the currently-open file");
            return;
        }
        if path == ws.welcome_file_path() {
            log::warn!("refusing to delete the welcome canvas");
            return;
        }
        match ws.trash_file(&path) {
            Ok(target) => log::info!("trashed {} → {}", path.display(), target.display()),
            Err(e) => log::error!("trash failed: {e:?}"),
        }
    }

    fn restore_file(&mut self, trashed: PathBuf) {
        let Some(ws) = self.workspace.as_ref() else { return };
        match ws.restore_from_trash(&trashed) {
            Ok(dest) => log::info!("restored {}", dest.display()),
            Err(e) => log::error!("restore failed: {e:?}"),
        }
    }

    fn purge_file(&mut self, trashed: PathBuf) {
        let Some(ws) = self.workspace.as_ref() else { return };
        match ws.purge_from_trash(&trashed) {
            Ok(()) => log::info!("purged {}", trashed.display()),
            Err(e) => log::error!("purge failed: {e:?}"),
        }
    }

    /// When we land on the index file for the first time, center the
    /// viewport on world (0, 0) so the picker (pinned at that world coord) is
    /// visible. After that the user can pan freely and the picker stays
    /// glued to its world position.
    fn ensure_index_view_initialized(&mut self) {
        if !self.is_on_index_file() || self.index_initial_view_set {
            return;
        }
        let (sw, sh) = self.viewport.screen;
        // We want screen (sw/2, sh/2) to map to world (0,0). Inverse of the
        // screen_to_world transform.
        let half = Vec2::new(sw as f32 * 0.5, sh as f32 * 0.5);
        self.viewport.offset = -half / self.viewport.zoom;
        self.index_initial_view_set = true;
    }

    fn new_whiteboard(&mut self) {
        let Some(ws) = &self.workspace else { return };
        let path = ws.next_untitled();
        self.switch_to_file(path);
    }

    fn cycle_next_file(&mut self) {
        if let Some(ws) = &self.workspace {
            if let Some(next) = ws.next_file() {
                self.switch_to_file(next);
            }
        }
    }

    fn export_current_markdown(&mut self) {
        let Some(ws) = &self.workspace else { return };
        let title = ws
            .current_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        let body = writee_export_web::markdown::render(&self.document, &title);
        let default_name = format!("{title}.md");
        let dest = rfd::FileDialog::new()
            .set_title("Export as Markdown")
            .set_file_name(&default_name)
            .add_filter("Markdown", &["md"])
            .save_file();
        let Some(path) = dest else { return };
        if let Err(e) = std::fs::write(&path, body) {
            log::error!("markdown export failed: {e:?}");
        } else {
            log::info!("exported markdown to {}", path.display());
        }
    }

    fn export_web(&mut self) {
        let Some(ws) = &self.workspace else { return };
        let stem = ws
            .current_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("export");
        let out = ws.root.join(format!("{stem}-export"));
        match writee_export_web::export_to_folder(&self.document, &out) {
            Ok(()) => log::info!("exported to {}", out.display()),
            Err(e) => log::error!("export failed: {e:?}"),
        }
    }

    /// The "workspace home" view — currently the welcome file. The picker
    /// panel shows here so the user always has a way to see their files when
    /// they land on this canvas.
    fn is_on_index_file(&self) -> bool {
        let Some(ws) = &self.workspace else { return false };
        ws.current_file == ws.welcome_file_path()
    }

    fn fit_to_content(&mut self) {
        // Compute world-AABB of every object, then frame it in the viewport.
        let mut bb = Aabb::EMPTY;
        let mut any = false;
        for (_, obj) in self.document.objects() {
            let b = obj.bbox();
            if !b.min.x.is_finite() || !b.max.x.is_finite() {
                continue;
            }
            if !any {
                bb = b;
                any = true;
            } else {
                bb.expand_point(b.min);
                bb.expand_point(b.max);
            }
        }
        if !any {
            self.viewport.offset = Vec2::splat(-100.0);
            self.viewport.zoom = 1.0;
            return;
        }
        let pad = 60.0;
        let span = bb.max - bb.min + Vec2::splat(pad * 2.0);
        let (sw, sh) = self.viewport.screen;
        let zx = sw as f32 / span.x.max(1.0);
        let zy = sh as f32 / span.y.max(1.0);
        let zoom = zx.min(zy).clamp(0.05, 8.0);
        self.viewport.zoom = zoom;
        let visible_world = Vec2::new(sw as f32 / zoom, sh as f32 / zoom);
        self.viewport.offset = bb.min - Vec2::splat(pad)
            - (visible_world - span) * 0.5;
    }

    // ------------------------------------------------------------------
    // Keyboard
    // ------------------------------------------------------------------

    fn handle_key(&mut self, key: &Key, code: PhysicalKey, pressed: bool) {
        if !pressed {
            return;
        }
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();

        // When the command palette is open, egui owns keystrokes (text input,
        // arrows, Enter, Esc). Only Ctrl-K (toggle) still flows through so
        // the user can dismiss it from anywhere.
        if self.palette.open {
            if ctrl && matches!(code, PhysicalKey::Code(KeyCode::KeyK)) {
                self.palette.toggle();
                self.request_redraw();
            }
            return;
        }

        if self.tool.editing_text.is_some() {
            match key {
                Key::Named(NamedKey::Escape) => {
                    self.finish_text_edit();
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::Backspace) => {
                    self.text_edit_backspace();
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::Enter) => {
                    self.text_edit_newline();
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::ArrowLeft) => {
                    self.text_edit_nav(|tb| tb.cursor_left());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::ArrowRight) => {
                    self.text_edit_nav(|tb| tb.cursor_right());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::ArrowUp) => {
                    self.text_edit_nav(|tb| tb.cursor_up());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::ArrowDown) => {
                    self.text_edit_nav(|tb| tb.cursor_down());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::Home) => {
                    self.text_edit_nav(|tb| tb.cursor_home());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::End) => {
                    self.text_edit_nav(|tb| tb.cursor_end());
                    self.request_redraw();
                    return;
                }
                Key::Character(s) if !ctrl => {
                    for ch in s.chars() {
                        self.text_edit_input_char(ch);
                    }
                    self.request_redraw();
                    return;
                }
                _ => {}
            }
        }

        if self.tool.editing_note.is_some() {
            match key {
                Key::Named(NamedKey::Escape) => {
                    self.finish_inline_note_edit();
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::Backspace) => {
                    self.note_edit_backspace();
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::Enter) => {
                    self.note_edit_input_char('\n');
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::ArrowLeft) => {
                    self.note_edit_nav(|n| n.cursor_left());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::ArrowRight) => {
                    self.note_edit_nav(|n| n.cursor_right());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::Home) => {
                    self.note_edit_nav(|n| n.cursor_home());
                    self.request_redraw();
                    return;
                }
                Key::Named(NamedKey::End) => {
                    self.note_edit_nav(|n| n.cursor_end());
                    self.request_redraw();
                    return;
                }
                Key::Character(s) if !ctrl => {
                    for ch in s.chars() {
                        self.note_edit_input_char(ch);
                    }
                    self.request_redraw();
                    return;
                }
                _ => {}
            }
        }

        if ctrl {
            let center = Vec2::new(
                self.viewport.screen.0 as f32 * 0.5,
                self.viewport.screen.1 as f32 * 0.5,
            );
            match code {
                PhysicalKey::Code(KeyCode::KeyK) => {
                    self.palette.toggle();
                }
                PhysicalKey::Code(KeyCode::KeyZ) => {
                    if shift { self.apply_redo(); } else { self.apply_undo(); }
                }
                PhysicalKey::Code(KeyCode::KeyY) => self.apply_redo(),
                PhysicalKey::Code(KeyCode::KeyA) => self.select_all(),
                PhysicalKey::Code(KeyCode::KeyC) => self.copy_selection(),
                PhysicalKey::Code(KeyCode::KeyV) => self.paste_clipboard(),
                PhysicalKey::Code(KeyCode::KeyD) => self.duplicate_selection(),
                PhysicalKey::Code(KeyCode::KeyN) => self.new_whiteboard(),
                PhysicalKey::Code(KeyCode::KeyO) => self.cycle_next_file(),
                PhysicalKey::Code(KeyCode::KeyE) => self.export_web(),
                PhysicalKey::Code(KeyCode::KeyF) => self.fit_to_content(),
                // Canvas zoom in: Ctrl+= / Ctrl+Plus / Ctrl+NumpadAdd.
                // The "=" key reports as KeyCode::Equal regardless of shift.
                PhysicalKey::Code(KeyCode::Equal) | PhysicalKey::Code(KeyCode::NumpadAdd) => {
                    self.viewport.zoom_about(center, 1.25);
                }
                // Canvas zoom out: Ctrl+- / Ctrl+NumpadSubtract.
                PhysicalKey::Code(KeyCode::Minus) | PhysicalKey::Code(KeyCode::NumpadSubtract) => {
                    self.viewport.zoom_about(center, 1.0 / 1.25);
                }
                // Ctrl+0 → reset zoom to 1.0 centered.
                PhysicalKey::Code(KeyCode::Digit0) | PhysicalKey::Code(KeyCode::Numpad0) => {
                    let world_center = self.screen_to_world(center);
                    self.viewport.zoom = 1.0;
                    self.viewport.offset = world_center - center / self.viewport.zoom;
                }
                _ => {}
            }
            self.request_redraw();
            return;
        }

        match key {
            Key::Named(NamedKey::Escape) => {
                self.tool.selected.clear();
                self.tool.reset_transient();
                self.finish_text_edit();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace) => {
                let ids: Vec<ObjectId> = std::mem::take(&mut self.tool.selected);
                for id in ids {
                    self.remove_object(id);
                }
                self.request_redraw();
                return;
            }
            _ => {}
        }

        match code {
            PhysicalKey::Code(KeyCode::KeyP) => self.set_tool(Tool::Pen),
            PhysicalKey::Code(KeyCode::KeyH) => self.set_tool(Tool::Highlighter),
            PhysicalKey::Code(KeyCode::KeyE) => self.set_tool(Tool::Eraser),
            PhysicalKey::Code(KeyCode::KeyA) => self.set_tool(Tool::Arrow),
            PhysicalKey::Code(KeyCode::KeyT) => self.set_tool(Tool::Text),
            PhysicalKey::Code(KeyCode::KeyI) => self.set_tool(Tool::Index),
            PhysicalKey::Code(KeyCode::KeyS) => self.set_tool(Tool::Select),
            _ => {}
        }
    }

    fn set_tool(&mut self, t: Tool) {
        self.finish_text_edit();
        self.tool.current = Some(t);
        self.tool.reset_transient();
        self.update_title();
        // Update the system cursor to telegraph the active tool.
        if let Some(w) = &self.window {
            w.set_cursor(t.cursor_icon());
        }
    }

    fn select_all(&mut self) {
        self.tool.selected.clear();
        for (id, obj) in self.document.objects() {
            if obj.is_anchorable() {
                self.tool.selected.push(id);
            }
        }
    }

    fn copy_selection(&mut self) {
        self.clipboard.clear();
        for &id in &self.tool.selected {
            if let Some(obj) = self.document.get(id) {
                self.clipboard.push(obj.clone());
            }
        }
        log::info!("copied {} object(s)", self.clipboard.len());
    }

    /// Try the OS clipboard for an image. Returns true on success.
    fn try_paste_image_from_clipboard(&mut self) -> bool {
        let mut cb = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                log::debug!("clipboard open failed: {e:?}");
                return false;
            }
        };
        let img = match cb.get_image() {
            Ok(i) => i,
            Err(_) => return false,
        };
        let (w, h) = (img.width as u32, img.height as u32);
        if w == 0 || h == 0 {
            return false;
        }
        // arboard returns raw RGBA; re-encode as PNG so storage is portable.
        let mut png = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png);
        if let Err(e) = image::ImageEncoder::write_image(
            encoder,
            img.bytes.as_ref(),
            w,
            h,
            image::ExtendedColorType::Rgba8,
        ) {
            log::warn!("png encode failed: {e:?}");
            return false;
        }
        self.insert_image(png, w, h);
        true
    }

    /// Add an Image object centred on the current viewport (or at world-origin
    /// when one isn't initialised yet) with the image's natural size, capped
    /// so giant pastes don't fill the screen.
    fn insert_image(&mut self, encoded: Vec<u8>, w: u32, h: u32) {
        let center_world = self.screen_to_world(Vec2::new(
            self.viewport.screen.0 as f32 * 0.5,
            self.viewport.screen.1 as f32 * 0.5,
        ));
        let max_dim_world = 400.0_f32;
        let scale = (max_dim_world / w.max(h) as f32).min(1.0);
        let size = Vec2::new(w as f32 * scale, h as f32 * scale);
        let img = ImageBlock {
            origin: center_world - size * 0.5,
            size,
            bytes: encoded,
            natural_w: w,
            natural_h: h,
        };
        self.add_object(Object::Image(img));
    }

    fn try_paste_image_from_file(&mut self, path: &std::path::Path) -> bool {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("read {} failed: {e:?}", path.display());
                return false;
            }
        };
        let decoded = match image::load_from_memory(&bytes) {
            Ok(d) => d,
            Err(_) => return false,
        };
        let (w, h) = (decoded.width(), decoded.height());
        self.insert_image(bytes, w, h);
        true
    }

    fn duplicate_selection(&mut self) {
        if self.tool.selected.is_empty() {
            return;
        }
        let offset = Vec2::new(24.0, 24.0) / self.viewport.zoom.max(0.05);
        let items: Vec<Object> = self
            .tool
            .selected
            .iter()
            .filter_map(|&id| self.document.get(id).cloned())
            .collect();
        self.tool.selected.clear();
        for mut obj in items {
            translate_object_in_place(&mut obj, offset);
            let id = self.add_object(obj);
            self.tool.selected.push(id);
        }
    }

    fn paste_clipboard(&mut self) {
        // Image takes precedence over in-app object clipboard.
        if self.try_paste_image_from_clipboard() {
            return;
        }
        if self.clipboard.is_empty() {
            return;
        }
        // Paste each object offset by a small amount so the copy is visible.
        let offset = Vec2::new(20.0, 20.0) / self.viewport.zoom.max(0.05);
        let items: Vec<Object> = self.clipboard.iter().cloned().collect();
        self.tool.selected.clear();
        for mut obj in items {
            translate_object_in_place(&mut obj, offset);
            let id = self.add_object(obj);
            self.tool.selected.push(id);
        }
    }

    fn palette_row_count(&mut self) -> usize {
        let files: Vec<PathBuf> = self
            .workspace
            .as_ref()
            .map(|w| w.list_files())
            .unwrap_or_default();
        let ws_root = self
            .workspace
            .as_ref()
            .map(|w| w.root.clone())
            .unwrap_or_default();
        palette::collect_rows(&self.palette, &files, &mut self.search_cache, &ws_root).len()
    }

    fn handle_palette_pick(&mut self, idx: usize) {
        // Recompute the rows for the current query, then act on the one at `idx`.
        let files: Vec<PathBuf> = self
            .workspace
            .as_ref()
            .map(|w| w.list_files())
            .unwrap_or_default();
        let ws_root = self
            .workspace
            .as_ref()
            .map(|w| w.root.clone())
            .unwrap_or_default();
        let rows = palette::collect_rows(&self.palette, &files, &mut self.search_cache, &ws_root);
        let Some(row) = rows.get(idx).cloned() else {
            self.palette.close();
            return;
        };
        match row {
            palette::PaletteRow::File(p) | palette::PaletteRow::Content { file: p, .. } => {
                self.switch_to_file(p);
            }
            palette::PaletteRow::Action(a) => {
                use palette::PaletteAction;
                match a {
                    PaletteAction::NewWhiteboard => self.new_whiteboard(),
                    PaletteAction::FitToContent => self.fit_to_content(),
                    PaletteAction::ToggleMarkdownMode => self.toggle_markdown_mode(),
                    PaletteAction::ToggleTheme => self.toggle_theme(),
                    PaletteAction::ExportMarkdown => self.export_current_markdown(),
                    PaletteAction::ExportPdf | PaletteAction::ExportPng => {
                        log::info!(
                            "PDF / PNG export are coming in the next release — for now use \
                             web export (Ctrl+E) which produces a printable HTML bundle."
                        );
                    }
                }
            }
        }
        self.palette.close();
    }

    fn flush_markdown_if_dirty(&mut self) {
        // Page mode now mutates the doc objects directly (and persists them
        // through DocStore::update during the editor pass), so there's no
        // separate "source" to flush. The append-new-block intent comes
        // through `state.append_requested` and is handled here.
        let Some(state) = self.md.as_mut() else { return };
        if !state.append_requested {
            return;
        }
        state.append_requested = false;
        // Compute insertion position: just below the lowest existing block,
        // aligned to its X so blocks stack visibly in the same column.
        let mut max_y: f32 = 0.0;
        let mut col_x: f32 = 0.0;
        let mut any = false;
        for (_, obj) in self.document.objects() {
            let (x, y) = match obj {
                Object::TextBox(tb) => (tb.origin.x, tb.origin.y + tb.font_size * 2.0),
                Object::SubNote(n) if n.is_inline() => (n.origin.x, n.origin.y + n.size.y),
                _ => continue,
            };
            if !any || y > max_y {
                max_y = y;
                col_x = x;
                any = true;
            }
        }
        let (new_x, new_y) = if any {
            (col_x, max_y + 24.0)
        } else {
            // Fresh page mode in an empty doc — drop the first block at the
            // current viewport's top-left in world coords so it's visible.
            let world = self.screen_to_world(Vec2::new(40.0, 80.0));
            (world.x, world.y)
        };
        let mut tb = TextBox::new(Vec2::new(new_x, new_y), self.settings.text_size);
        tb.font_name = Some(self.fonts.resolve(self.settings.font_slot).to_string());
        let id = self.document.add(Object::TextBox(tb.clone()));
        if let Some(store) = &self.store {
            let _ = store.insert(id, &Object::TextBox(tb));
        }
        self.mark_doc_dirty();
    }

    fn toggle_markdown_mode(&mut self) {
        let Some(store) = &self.store else { return };
        match self.md.is_some() {
            true => {
                let _ = store.set_document_mode(DocumentMode::Canvas);
                self.md = None;
            }
            false => {
                let _ = store.set_document_mode(DocumentMode::Markdown);
                self.md = Some(markdown::MarkdownState::default());
            }
        }
        self.update_title();
    }

    fn apply_ui_actions(&mut self, a: UiActions) {
        if let Some(t) = a.set_tool {
            self.set_tool(t);
        }
        if a.undo { self.apply_undo(); }
        if a.redo { self.apply_redo(); }
        if a.new_file { self.new_whiteboard(); }
        if a.cycle_file { self.cycle_next_file(); }
        if a.export_web { self.export_web(); }
        if a.fit_to_content { self.fit_to_content(); }
        if a.delete_selected {
            let ids: Vec<ObjectId> = std::mem::take(&mut self.tool.selected);
            for id in ids {
                self.remove_object(id);
            }
        }
        if a.clear_selection {
            self.tool.selected.clear();
        }
        if let Some(open) = a.open_file {
            self.switch_to_file(open);
        }
        if let Some((path, stem)) = a.rename_file {
            self.rename_file(path, stem);
        }
        if let Some(path) = a.delete_file {
            self.delete_file(path);
        }
        if let Some(path) = a.restore_file {
            self.restore_file(path);
        }
        if let Some(path) = a.purge_file {
            self.purge_file(path);
        }
        if a.toggle_markdown {
            self.toggle_markdown_mode();
        }
        if a.recovery_accept {
            if let Some(snap) = self.pending_recovery.take() {
                self.accept_recovery(snap);
            }
            if let Some(ws) = &self.workspace {
                recovery::clear(&ws.root);
            }
        }
        if a.recovery_discard {
            self.pending_recovery = None;
            if let Some(ws) = &self.workspace {
                recovery::clear(&ws.root);
            }
        }
        if a.palette_close {
            self.palette.close();
        }
        if a.palette_focused {
            self.palette.focused = true;
        }
        if a.palette_focus_next {
            self.palette.focus = self.palette.focus.saturating_add(1);
        }
        if a.palette_focus_prev {
            self.palette.focus = self.palette.focus.saturating_sub(1);
        }
        // Cap focus to the visible row count to avoid scrolling off the end.
        let row_count = self.palette_row_count();
        if row_count > 0 && self.palette.focus >= row_count {
            self.palette.focus = row_count - 1;
        }
        if let Some(idx) = a.palette_pick {
            self.handle_palette_pick(idx);
        }
        if let Some(new_filter) = a.set_tag_filter {
            self.active_tag = new_filter;
        }
        if let Some(id) = a.note_open {
            if let Some(Object::SubNote(n)) = self.document.get(id).cloned() {
                if let Some(ws) = &self.workspace {
                    if !n.target_file.is_empty() {
                        let target = ws.root.join(&n.target_file);
                        self.switch_to_file(target);
                    }
                }
            }
        }
        if let Some(id) = a.note_edit_inline {
            self.begin_inline_note_edit(id, None);
        }
        if let Some(id) = a.note_convert_to_linked {
            self.convert_note_inline_to_linked(id);
        }
        if let Some(id) = a.note_convert_to_inline {
            self.convert_note_linked_to_inline(id);
        }
        if let Some(id) = a.note_toggle_index {
            self.toggle_note_index(id);
        }
        if let Some(id) = a.note_delete {
            self.remove_object(id);
        }
        if a.note_close_menu {
            self.context_menu = None;
        }
        if let Some((id, want_markdown)) = a.note_set_linked_mode {
            self.set_linked_note_mode(id, want_markdown);
        }
        if let Some(id) = a.note_edit_index_contents {
            self.open_index_editor(id);
        }
        if a.index_editor_commit {
            self.commit_index_editor();
        }
        if a.index_editor_cancel {
            self.index_editor = None;
        }
    }
}

/// Compute the (uniform scale, translation) that fits `child_doc`'s content
/// AABB into the body region of `card` (below the title bar, with a small
/// inset). Returns None when the child is empty or degenerate.
fn thumbnail_transform(card: &SubNote, child_doc: &Document) -> Option<(f32, Vec2)> {
    let mut bb = Aabb::EMPTY;
    let mut any = false;
    for (_, obj) in child_doc.objects() {
        let b = obj.bbox();
        if !b.min.x.is_finite() || !b.max.x.is_finite() {
            continue;
        }
        if !any {
            bb = b;
            any = true;
        } else {
            bb.expand_point(b.min);
            bb.expand_point(b.max);
        }
    }
    if !any {
        return None;
    }
    let span = bb.max - bb.min;
    if span.x <= 0.0 || span.y <= 0.0 {
        return None;
    }
    let title_bar = 28.0_f32;
    let inset = 6.0_f32;
    let card_min = card.origin + Vec2::new(inset, title_bar + inset);
    let card_max = card.origin + card.size - Vec2::splat(inset);
    let avail = card_max - card_min;
    if avail.x <= 1.0 || avail.y <= 1.0 {
        return None;
    }
    let scale = (avail.x / span.x).min(avail.y / span.y);
    // Centre within the available area.
    let scaled = span * scale;
    let pad = (avail - scaled) * 0.5;
    let offset = card_min + pad - bb.min * scale;
    Some((scale, offset))
}

/// Tessellate a child object for inclusion in a sub-note thumbnail. We
/// tessellate at the child's natural size; `transform_verts` then scales
/// into the card area.
fn tessellate_child_for_thumbnail(
    obj: &Object,
    pressure_sensitive: bool,
    theme: &writee_core::ColorTheme,
) -> Vec<InkVertex> {
    use writee_core::ShapeKind;
    match obj {
        Object::Stroke(s) => tessellate_opts(
            &s.points,
            s.width_base,
            s.effective_color(),
            pressure_sensitive,
            false,
        ),
        Object::Arrow(a) => tessellate_arrow(a, theme.ink),
        Object::Shape(s) => {
            let min = s.a.min(s.b);
            let max = s.a.max(s.b);
            match s.kind {
                ShapeKind::Rectangle => {
                    if s.filled {
                        tessellate_rect(min, max, s.color)
                    } else {
                        tessellate_rect_outline(min, max, s.stroke_width * 0.5, s.color)
                    }
                }
                ShapeKind::Ellipse => tessellate_ellipse(min, max, s.color, s.filled),
                ShapeKind::Line => tessellate_line(s.a, s.b, s.stroke_width, s.color),
            }
        }
        // Nested sub-notes, text, links: skip in the thumbnail (text is
        // emitted separately via TextInstance; nested cards would need
        // recursion which we deliberately bound at depth 1).
        _ => Vec::new(),
    }
}

fn transform_verts(verts: &[InkVertex], scale: f32, offset: Vec2) -> Vec<InkVertex> {
    verts
        .iter()
        .map(|v| InkVertex {
            pos: v.pos * scale + offset,
            // half_width is a real "huge fill" sentinel for filled shapes; do
            // not scale that or filled rects/ellipses will lose their edge.
            half_width: if v.half_width > 100.0 {
                v.half_width
            } else {
                v.half_width * scale
            },
            signed_offset: if v.half_width > 100.0 {
                v.signed_offset
            } else {
                v.signed_offset * scale
            },
            color: v.color,
        })
        .collect()
}

/// One resolved row inside an index card. Files carry both the path (for
/// the click router) and a friendly display name (for the body renderer).
#[derive(Debug, Clone)]
enum ResolvedIndexEntry {
    File { path: PathBuf, display: String },
    Heading { text: String },
}

/// `.writee` filename → friendly label: strip the extension and the
/// "_" prefix that special files (`_welcome`) use.
fn display_name_for_file(file: &str) -> String {
    let stripped = file.strip_suffix(".writee").unwrap_or(file);
    stripped.trim_start_matches('_').to_string()
}

/// Approximate caret world-position for an inline sub-note. Uses the same
/// rough 0.55em char-width / 1.25 line-height metrics as `TextBox`.
fn note_caret_world_pos(n: &SubNote) -> Vec2 {
    let body = n.body_rect();
    let font_size = 16.0_f32;
    let Some(text) = &n.inline_content else { return body.min };
    let cursor = n.cursor.min(text.len());
    let upto = &text[..cursor];
    let line = upto.bytes().filter(|b| *b == b'\n').count();
    let line_start = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col_chars = text[line_start..cursor].chars().count();
    let x = body.min.x + (col_chars as f32) * font_size * 0.55;
    let y = body.min.y + (line as f32) * font_size * 1.25;
    Vec2::new(x, y)
}

/// Cheap fingerprint for the autosave recovery state — used to skip writes
/// when nothing has changed since the previous snapshot.
fn recovery_signature(
    wet: &[InkPoint],
    editing_text: &Option<(ObjectId, String)>,
    editing_note: &Option<(ObjectId, String)>,
) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mix = |h: &mut u64, v: u64| {
        *h ^= v;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(&mut h, wet.len() as u64);
    if let Some(last) = wet.last() {
        mix(&mut h, last.x.to_bits() as u64);
        mix(&mut h, last.y.to_bits() as u64);
        mix(&mut h, last.t_ms as u64);
    }
    if let Some((id, c)) = editing_text {
        mix(&mut h, *id);
        mix(&mut h, c.len() as u64);
        for b in c.as_bytes().iter().rev().take(16) {
            mix(&mut h, *b as u64);
        }
    }
    if let Some((id, c)) = editing_note {
        mix(&mut h, *id);
        mix(&mut h, c.len() as u64);
        for b in c.as_bytes().iter().rev().take(16) {
            mix(&mut h, *b as u64);
        }
    }
    h
}

/// FNV-1a 64-bit hash of the encoded image bytes — used as a stable texture
/// cache key without storing one separately in the document.
fn image_id(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

impl App {
    fn collect_image_quads(&self) -> Vec<ImageQuad> {
        let mut out = Vec::new();
        for (_, obj) in self.document.objects() {
            if let Object::Image(img) = obj {
                let id = image_id(&img.bytes);
                out.push(ImageQuad {
                    texture_id: id,
                    world_min: [img.origin.x, img.origin.y],
                    world_max: [img.origin.x + img.size.x, img.origin.y + img.size.y],
                });
            }
        }
        out
    }

    /// (id, ImageBlock clone) pairs the renderer hasn't yet uploaded.
    fn collect_pending_image_uploads(&self, r: &Renderer) -> Vec<(u64, ImageBlock)> {
        let mut out = Vec::new();
        for (_, obj) in self.document.objects() {
            if let Object::Image(img) = obj {
                let id = image_id(&img.bytes);
                if !r.image_cached(id) {
                    out.push((id, img.clone()));
                }
            }
        }
        out
    }
}

fn load_markdown_state(store: &DocStore) -> Option<markdown::MarkdownState> {
    let mode = store.document_mode().unwrap_or(DocumentMode::Canvas);
    if mode != DocumentMode::Markdown {
        return None;
    }
    let source = store
        .get_meta(markdown::META_KEY_MARKDOWN_SOURCE)
        .ok()
        .flatten()
        .unwrap_or_default();
    Some(markdown::MarkdownState::new(source))
}

fn translate_object_in_place(obj: &mut Object, delta: Vec2) {
    match obj {
        Object::Stroke(s) => {
            for p in s.points.iter_mut() {
                p.x += delta.x;
                p.y += delta.y;
            }
        }
        Object::Arrow(a) => {
            a.start += delta;
            a.end += delta;
        }
        Object::TextBox(t) => t.origin += delta,
        Object::Shape(s) => {
            s.a += delta;
            s.b += delta;
        }
        Object::SubNote(n) => n.origin += delta,
        Object::Image(i) => i.origin += delta,
        Object::Link(_) => {}
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() { "note".into() } else { trimmed }
}

fn inside_bbox(p: Vec2, b: writee_core::Aabb) -> bool {
    p.x >= b.min.x && p.x <= b.max.x && p.y >= b.min.y && p.y <= b.max.y
}

fn positions_differ(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Arrow(a), Object::Arrow(b)) => a.start != b.start || a.end != b.end,
        (Object::TextBox(a), Object::TextBox(b)) => a.origin != b.origin,
        (Object::Stroke(a), Object::Stroke(b)) => {
            a.points.first().map(|p| (p.x, p.y))
                != b.points.first().map(|p| (p.x, p.y))
        }
        _ => false,
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (iw, ih, ibytes) = icon::rgba_64();
        let window_icon = winit::window::Icon::from_rgba(ibytes, iw, ih).ok();
        let mut attrs = Window::default_attributes()
            .with_title("writee")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        if let Some(ic) = window_icon {
            attrs = attrs.with_window_icon(Some(ic));
        }
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create_window failed"),
        );

        let renderer = pollster::block_on(Renderer::new(window.clone()))
            .expect("Renderer init failed");

        let chrome = EguiChrome::new(&renderer.device, renderer.format, window.as_ref());

        self.viewport = Viewport::new(renderer.surface_size());
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.chrome = Some(chrome);
        self.update_title();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        // Forward to egui first; track both consumption (suppress canvas
        // handling) and repaint hint (trigger a redraw so the click/keystroke
        // actually flows through ctx.run() this frame).
        let mut consumed_by_ui = false;
        let mut ui_wants_repaint = false;
        if let (Some(chrome), Some(window)) = (&mut self.chrome, &self.window) {
            let resp = chrome.on_window_event(window, &event);
            consumed_by_ui = resp.consumed;
            ui_wants_repaint = resp.repaint;
        }
        if ui_wants_repaint {
            self.request_redraw();
        }

        match &event {
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let p = Vec2::new(position.x as f32, position.y as f32);
                if self.panning {
                    let delta = p - self.last_pan_px;
                    self.viewport.pan(delta);
                    self.last_pan_px = p;
                    self.request_redraw();
                }
                self.cursor_px = p;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = matches!(state, ElementState::Pressed);
                if matches!(button, MouseButton::Middle) {
                    self.panning = down;
                    self.last_pan_px = self.cursor_px;
                }
                if matches!(button, MouseButton::Right) && down && !consumed_by_ui {
                    let world = self.screen_to_world(self.cursor_px);
                    if let Some(id) = self.pick_subnote_at_world(world) {
                        self.context_menu = Some((id, self.cursor_px));
                        self.request_redraw();
                    } else {
                        self.context_menu = None;
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !consumed_by_ui {
                    let lines = match delta {
                        MouseScrollDelta::LineDelta(_x, y) => *y,
                        MouseScrollDelta::PixelDelta(d) => (d.y as f32) / 60.0,
                    };
                    let factor = (1.15f32).powf(lines);
                    self.viewport.zoom_about(self.cursor_px, factor);
                    self.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if !consumed_by_ui {
                    let pressed = matches!(event.state, ElementState::Pressed);
                    self.handle_key(&event.logical_key, event.physical_key, pressed);
                }
            }
            _ => {}
        }

        if !self.panning && !consumed_by_ui {
            if let Some(sample) = self.input.handle(&event) {
                self.on_sample(sample);
                self.request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                self.finish_text_edit();
                self.finish_inline_note_edit();
                // Graceful exit → no in-progress work, drop the recovery file
                // so next launch doesn't offer to restore stale state.
                if let Some(ws) = &self.workspace {
                    recovery::clear(&ws.root);
                }
                event_loop.exit();
            }
            WindowEvent::DroppedFile(path) => {
                if !self.try_paste_image_from_file(&path) {
                    log::info!("dropped file {} is not a recognised image", path.display());
                }
                self.request_redraw();
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                    self.viewport.screen = r.surface_size();
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }
}

impl App {
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        // Initialise the index view *once* per file open (no-op if not on the
        // index file or already centered).
        self.ensure_index_view_initialized();

        // Snapshot settings + figure out if we're on the index file so we can
        // detect any cache-invalidating changes the toolbar made this frame.
        let pre_pressure = self.settings.pressure_sensitive;
        let pre_settings_snapshot = (
            self.settings.stroke_width,
            self.settings.eraser_radius,
            self.settings.text_size,
            self.settings.ink_color,
            self.settings.text_color,
            self.settings.active_shape,
            self.settings.shape_filled,
            self.settings.tilt_modulation,
            self.settings.font_slot,
        );
        let pre_fonts = self.fonts.clone();
        let _on_index = self.is_on_index_file();
        // Build the hierarchical workspace tree for the sidebar. Cheap enough
        // at workspace-scale; per-index-card pickers still use their own list.
        let workspace_tree = self
            .workspace
            .as_ref()
            .map(|w| w.build_tree())
            .unwrap_or_default();
        let workspace_root_for_sidebar = self.workspace.as_ref().map(|w| w.root.clone());
        let trash_files: Vec<PathBuf> = self
            .workspace
            .as_ref()
            .map(|w| w.list_trash())
            .unwrap_or_default();
        let backlinks: Vec<String> = self
            .workspace
            .as_ref()
            .and_then(|w| {
                w.current_file
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|name| w.backlinks(name))
            })
            .unwrap_or_default();
        let tags_map = self
            .workspace
            .as_ref()
            .map(|w| tags::all_tags(&w.root))
            .unwrap_or_default();
        // If a tag filter is active, restrict the tree to files containing
        // it. Tagged files whose linking parents are NOT tagged get promoted
        // to root level so they remain reachable.
        let filtered_tree = if let Some(t) = &self.active_tag {
            let allowed: std::collections::HashSet<String> = tags_map
                .get(t)
                .map(|files| {
                    files
                        .iter()
                        .filter_map(|p| {
                            p.file_name().and_then(|s| s.to_str()).map(String::from)
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Surviving parent → child edges (both endpoints allowed).
            let children: std::collections::BTreeMap<String, Vec<String>> = workspace_tree
                .children
                .iter()
                .filter_map(|(k, v)| {
                    if allowed.contains(k) {
                        let kids: Vec<String> = v
                            .iter()
                            .filter(|c| allowed.contains(*c))
                            .cloned()
                            .collect();
                        Some((k.clone(), kids))
                    } else {
                        None
                    }
                })
                .collect();
            // A file is a "root" if it's in `allowed` AND it has no surviving
            // parent that is also in `allowed` (which would render it nested).
            let has_surviving_parent: std::collections::HashSet<&str> = children
                .values()
                .flat_map(|kids| kids.iter().map(|s| s.as_str()))
                .collect();
            let mut roots: Vec<String> = allowed
                .iter()
                .filter(|f| !has_surviving_parent.contains(f.as_str()))
                .cloned()
                .collect();
            roots.sort();
            workspace::WorkspaceTree { roots, children }
        } else {
            workspace_tree.clone()
        };
        let active_tag_owned = self.active_tag.clone();
        let current_path = self
            .workspace
            .as_ref()
            .map(|w| w.current_file.clone());

        // Snapshot the context-menu target (if any) so we can build the menu
        // inside the egui closure without re-borrowing `self.document`.
        let ws_root_for_menu = self.workspace.as_ref().map(|w| w.root.clone());
        let context_menu_info = self.context_menu.and_then(|(id, screen_pos)| {
            self.document.get(id).and_then(|obj| match obj {
                Object::SubNote(n) => {
                    let linked_is_markdown = if n.is_linked() {
                        ws_root_for_menu
                            .as_ref()
                            .and_then(|root| DocStore::open(root.join(&n.target_file)).ok())
                            .and_then(|s| s.document_mode().ok())
                            == Some(DocumentMode::Markdown)
                    } else {
                        false
                    };
                    Some(ui::NoteMenuInfo {
                        object_id: id,
                        screen_pos: egui::pos2(screen_pos.x, screen_pos.y),
                        is_inline: n.is_inline(),
                        is_linked: n.is_linked(),
                        is_index: n.is_index,
                        locked: n.locked,
                        title: n.title.clone(),
                        linked_is_markdown,
                    })
                }
                _ => None,
            })
        });
        let recovery_prompt_open = self.pending_recovery.is_some();
        let index_editor_active = self.index_editor.is_some();
        // Palette rows snapshot — compute before the egui closure to keep
        // borrows clean.
        let palette_open = self.palette.open;
        let palette_needs_focus = palette_open && !self.palette.focused;
        let (palette_rows, palette_query_snapshot, palette_focus) = if palette_open {
            let files: Vec<PathBuf> = self
                .workspace
                .as_ref()
                .map(|w| w.list_files())
                .unwrap_or_default();
            let ws_root = self
                .workspace
                .as_ref()
                .map(|w| w.root.clone())
                .unwrap_or_default();
            let rows =
                palette::collect_rows(&self.palette, &files, &mut self.search_cache, &ws_root);
            (rows, self.palette.query.clone(), self.palette.focus)
        } else {
            (Vec::new(), String::new(), 0)
        };
        let mut index_editor_title = String::new();
        let mut index_editor_entries: Vec<ui::IndexEditorEntry> = Vec::new();
        let mut index_editor_available: Vec<String> = Vec::new();
        let mut index_editor_selected_file: Option<String> = None;
        let mut index_editor_new_heading: String = String::new();
        if let Some(state) = &self.index_editor {
            index_editor_title = self
                .document
                .get(state.object_id)
                .and_then(|o| match o {
                    Object::SubNote(n) => Some(if n.title.is_empty() { "Index".to_string() } else { n.title.clone() }),
                    _ => None,
                })
                .unwrap_or_else(|| "Index".into());
            index_editor_entries = state
                .entries
                .iter()
                .map(|e| match e {
                    writee_core::IndexEntry::File { file } => {
                        ui::IndexEditorEntry::File { file: file.clone() }
                    }
                    writee_core::IndexEntry::Heading { text } => {
                        ui::IndexEditorEntry::Heading { text: text.clone() }
                    }
                })
                .collect();
            index_editor_available = state.available_files.clone();
            index_editor_selected_file = state.add_file_selected.clone();
            index_editor_new_heading = state.new_heading_text.clone();
        }
        let mut index_row_actions = ui::IndexEditorActions::default();

        let mut ui_actions = UiActions::default();
        // Refresh egui theme (cheap; only diffs reach the GPU).
        if let Some(chrome) = &self.chrome {
            chrome.apply_theme(self.fonts_theme());
        }
        let egui_output = if let (Some(chrome), Some(window)) = (&mut self.chrome, &self.window) {
            let current_file = self
                .workspace
                .as_ref()
                .and_then(|w| w.current_file.file_name().and_then(|s| s.to_str()))
                .map(|s| s.to_string());
            let raw = chrome.state.take_egui_input(window.as_ref());

            let mut local_actions = None;
            let settings_ref = &mut self.settings;
            let fonts_ref = &mut self.fonts;
            let settings_open_ref = &mut chrome.settings_open;
            let tool_ref = self.tool.current;
            let can_undo = self.undo.can_undo();
            let can_redo = self.undo.can_redo();
            let obj_count = self.document.len();
            let current_file_str = current_file.clone();
            let last_pressure = self.last_pressure;
            let notes_locked = self.notes_locked;
            let is_markdown = self.md.is_some();
            // Take ownership of the markdown state for the duration of the
            // egui closure (which the borrow checker treats as FnMut and so
            // can't borrow `self.md` directly across iterations). Put it back
            // immediately after.
            let mut taken_md = self.md.take();
            let mut palette_query_after: Option<String> = None;
            let output = chrome.ctx.run(raw, |ctx| {
                let mut actions = ui::build_ui(ctx, UiInput {
                    current_tool: tool_ref,
                    settings: settings_ref,
                    fonts: fonts_ref,
                    can_undo,
                    can_redo,
                    current_file: current_file_str.as_deref(),
                    object_count: obj_count,
                    last_pressure,
                    settings_open: settings_open_ref,
                    on_welcome: notes_locked,
                    is_markdown,
                });
                // Persistent hierarchical file tree on the left.
                if let Some(root) = &workspace_root_for_sidebar {
                    if let Some(opened) = ui::build_file_tree_sidebar(
                        ctx,
                        root,
                        &filtered_tree,
                        current_path.as_deref(),
                        workspace::WELCOME_FILE_NAME,
                        &trash_files,
                        &backlinks,
                        &tags_map,
                        active_tag_owned.as_deref(),
                        &mut actions,
                    ) {
                        actions.open_file = Some(opened);
                    }
                }
                if let Some(info) = &context_menu_info {
                    ui::build_note_context_menu(ctx, info, &mut actions);
                }
                if palette_open {
                    let mut query_buf = palette_query_snapshot.clone();
                    ui::build_command_palette(
                        ctx,
                        &mut query_buf,
                        palette_focus,
                        palette_needs_focus,
                        &palette_rows,
                        &mut actions,
                    );
                    palette_query_after = Some(query_buf);
                }
                if recovery_prompt_open {
                    egui::Window::new("Recover unsaved work?")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label(
                                "writee found work from a previous session that wasn't saved \
                                 (likely a crash or kill). Restore it?",
                            );
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button("Restore").clicked() {
                                    actions.recovery_accept = true;
                                }
                                if ui.button("Discard").clicked() {
                                    actions.recovery_discard = true;
                                }
                            });
                        });
                }
                if index_editor_active {
                    ui::build_index_editor(
                        ctx,
                        &index_editor_title,
                        &index_editor_entries,
                        &index_editor_available,
                        &mut index_editor_selected_file,
                        &mut index_editor_new_heading,
                        &mut actions,
                        &mut index_row_actions,
                    );
                }
                if let Some(state) = taken_md.as_mut() {
                    let md_actions = markdown::build_page_editor(
                        ctx,
                        &mut self.document,
                        self.store.as_ref(),
                        state,
                    );
                    if md_actions.leave_markdown {
                        actions.toggle_markdown = true;
                    }
                    if md_actions.source_changed {
                        self.cache.mark_dirty();
                    }
                }
                local_actions = Some(actions);
            });
            self.md = taken_md;
            // Push palette query edits back.
            if let Some(q) = palette_query_after {
                if q != self.palette.query {
                    self.palette.query = q;
                    self.palette.focus = 0;
                }
            }
            // Apply the user's edits to the working index editor state.
            if let Some(state) = self.index_editor.as_mut() {
                state.add_file_selected = index_editor_selected_file;
                state.new_heading_text = index_editor_new_heading;
                if let Some(i) = index_row_actions.move_up {
                    if i > 0 && i < state.entries.len() {
                        state.entries.swap(i, i - 1);
                    }
                }
                if let Some(i) = index_row_actions.move_down {
                    if i + 1 < state.entries.len() {
                        state.entries.swap(i, i + 1);
                    }
                }
                if let Some(i) = index_row_actions.remove {
                    if i < state.entries.len() {
                        state.entries.remove(i);
                    }
                }
                if let Some(file) = index_row_actions.add_file {
                    state
                        .entries
                        .push(writee_core::IndexEntry::File { file });
                }
                if let Some(text) = index_row_actions.add_heading {
                    state
                        .entries
                        .push(writee_core::IndexEntry::Heading { text });
                }
                if index_row_actions.add_all_missing {
                    let existing: std::collections::HashSet<&str> = state
                        .entries
                        .iter()
                        .filter_map(|e| match e {
                            writee_core::IndexEntry::File { file } => Some(file.as_str()),
                            _ => None,
                        })
                        .collect();
                    let missing: Vec<String> = state
                        .available_files
                        .iter()
                        .filter(|f| !existing.contains(f.as_str()))
                        .cloned()
                        .collect();
                    for f in missing {
                        state
                            .entries
                            .push(writee_core::IndexEntry::File { file: f });
                    }
                }
                if index_row_actions.clear_all {
                    state.entries.clear();
                }
            }
            chrome
                .state
                .handle_platform_output(window.as_ref(), output.platform_output.clone());
            if let Some(a) = local_actions.take() {
                ui_actions = a;
            }
            Some(output)
        } else {
            None
        };

        // Pressure toggle requires re-tessellating committed strokes.
        if self.settings.pressure_sensitive != pre_pressure {
            self.mark_doc_dirty();
        }

        // Persist any settings change.
        let post_settings_snapshot = (
            self.settings.stroke_width,
            self.settings.eraser_radius,
            self.settings.text_size,
            self.settings.ink_color,
            self.settings.text_color,
            self.settings.active_shape,
            self.settings.shape_filled,
            self.settings.tilt_modulation,
            self.settings.font_slot,
        );
        let settings_changed = post_settings_snapshot != pre_settings_snapshot
            || self.settings.pressure_sensitive != pre_pressure
            || pre_fonts.default != self.fonts.default
            || pre_fonts.mono != self.fonts.mono
            || pre_fonts.serif != self.fonts.serif
            || pre_fonts.slab != self.fonts.slab
            || pre_fonts.thematic != self.fonts.thematic;
        if settings_changed {
            self.save_config();
        }

        self.apply_ui_actions(ui_actions);
        self.flush_markdown_if_dirty();
        self.maybe_write_recovery();

        let geom = self.build_ink_geometry();
        let text = self.text_instances();
        let theme = self.fonts_theme();
        let image_quads = self.collect_image_quads();
        // Determine which image bytes the renderer hasn't seen yet *before*
        // borrowing the renderer mutably below.
        let pending_uploads: Vec<(u64, ImageBlock)> = if let Some(r) = &self.renderer {
            self.collect_pending_image_uploads(r)
        } else {
            Vec::new()
        };
        if let Some(r) = &mut self.renderer {
            for (id, img) in pending_uploads {
                if let Ok(decoded) = image::load_from_memory(&img.bytes) {
                    let rgba = decoded.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    r.upload_image_rgba(id, w, h, rgba.as_raw());
                }
            }
            r.set_image_quads(&image_quads);
            r.upload_ink_geometry(&geom);

            let egui_frame = match (egui_output, &mut self.chrome) {
                (Some(output), Some(chrome)) => {
                    let (w, h) = r.surface_size();
                    let pixels_per_point = chrome.ctx.pixels_per_point();
                    Some(EguiFrame {
                        pass: &mut chrome.pass,
                        ctx: &chrome.ctx,
                        output,
                        screen: ScreenDescriptor {
                            size_in_pixels: [w, h],
                            pixels_per_point,
                        },
                    })
                }
                _ => None,
            };

            match r.render(&self.viewport, &text, theme, egui_frame) {
                Ok(()) => {}
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    let size = r.window().inner_size();
                    r.resize(size.width, size.height);
                    self.viewport.screen = r.surface_size();
                }
                Err(wgpu::SurfaceError::OutOfMemory) => {
                    log::error!("wgpu out of memory");
                    event_loop.exit();
                }
                Err(e) => log::warn!("frame error: {e:?}"),
            }
        }
    }
}

pub fn run() -> Result<()> {
    let event_loop = winit::event_loop::EventLoop::new()?;
    run_with_event_loop(event_loop)
}

#[cfg(target_os = "android")]
pub fn run_android(android_app: winit::platform::android::activity::AndroidApp) -> Result<()> {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    let event_loop = winit::event_loop::EventLoop::builder()
        .with_android_app(android_app)
        .build()?;
    run_with_event_loop(event_loop)
}

fn run_with_event_loop(event_loop: winit::event_loop::EventLoop<()>) -> Result<()> {
    let mut workspace = Workspace::discover_or_create().context("workspace setup")?;
    log::info!("workspace: {}", workspace.root.display());

    // Refuse to start if another writee window already has this workspace
    // open. SQLite WAL + our autosave loop are not safe under concurrent
    // writers from two processes.
    let lock = match workspace::WorkspaceLock::try_acquire(&workspace.root) {
        Ok(l) => l,
        Err(e) => {
            log::error!("{e:?}");
            eprintln!(
                "Another writee window appears to have this workspace open:\n  {}\n\nClose it (or pick a different workspace by editing {}) and try again.",
                workspace.root.display(),
                directories_next::ProjectDirs::from("", "", "writee")
                    .map(|d| d.config_dir().join("workspace").display().to_string())
                    .unwrap_or_else(|| "~/.config/writee/workspace".into()),
            );
            return Err(e);
        }
    };

    let config = settings::WorkspaceConfig::load(&workspace.root);
    if let Some(last) = config.state.last_file.as_deref() {
        let candidate = workspace.root.join(last);
        if candidate.exists() {
            workspace.current_file = candidate;
        }
    }
    log::info!("opening: {}", workspace.current_file.display());

    let store = DocStore::open(&workspace.current_file)
        .with_context(|| format!("opening {}", workspace.current_file.display()))?;
    let document = store.load_all().context("loading document")?;
    log::info!("loaded {} object(s)", document.len());

    // Check for an interrupted-session recovery snapshot belonging to the
    // file we're about to open. App will surface the prompt on first redraw.
    let pending = recovery::load(&workspace.root).filter(|snap| {
        snap.current_file == workspace.current_file
    });

    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    let mut app = App::new_with_workspace(workspace, store, document, config);
    if let Some(snap) = pending {
        app.set_pending_recovery(snap);
    }
    event_loop.run_app(&mut app)?;
    drop(lock);
    Ok(())
}
