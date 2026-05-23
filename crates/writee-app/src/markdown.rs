//! Page-mode "block editor" — Affine-style.
//!
//! A doc in writee is a set of `Object`s with world positions. In edgeless
//! (canvas) mode we render them at those positions. In page mode (this
//! module) we surface the readable blocks — `TextBox` and inline `SubNote`
//! bodies — stacked vertically in Y order, each editable in place.
//!
//! The two modes share the *same data* (mirroring Affine's "two views of one
//! atomic data unit"). Edits in page mode mutate the underlying objects and
//! persist via `DocStore`; toggling back to edgeless preserves canvas
//! positions because we never touch them.
//!
//! Blocks are rendered top-to-bottom in Y order. A small "+ block" footer
//! lets the user append a new TextBox below the last one.

use egui::{FontId, RichText, ScrollArea, TextEdit};
use writee_core::{DocStore, Document, Object, ObjectId};

pub const META_KEY_MARKDOWN_SOURCE: &str = "markdown_source"; // legacy; still read for back-compat

/// Tag for what we're editing — drives action handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    TextBox,
    InlineNote,
}

#[derive(Debug, Default)]
pub struct MarkdownState {
    /// In-progress edits keyed by object id; flushed back to the Document on
    /// the next frame so we can borrow the doc immutably during egui run.
    pending_edits: Vec<(ObjectId, BlockKind, String)>,
    pub append_requested: bool,
}

impl MarkdownState {
    pub fn new(_seed_source: String) -> Self {
        // We no longer maintain a separate "markdown source" string. The
        // legacy meta key is kept readable so old files don't lose their
        // content (the App can migrate it into a TextBox if desired).
        Self::default()
    }
}

#[derive(Debug, Default)]
pub struct MdActions {
    pub source_changed: bool,
    pub leave_markdown: bool,
}

pub fn build_page_editor(
    ctx: &egui::Context,
    doc: &mut Document,
    store: Option<&DocStore>,
    state: &mut MarkdownState,
) -> MdActions {
    let mut actions = MdActions::default();

    // Collect editable blocks tagged with their (y, x) world position so we
    // can sort top-to-bottom into reading order before rendering.
    let mut with_pos: Vec<((ObjectId, BlockKind, String), (f32, f32))> = doc
        .objects()
        .filter_map(|(id, obj)| match obj {
            Object::TextBox(tb) => Some((
                (id, BlockKind::TextBox, tb.content.clone()),
                (tb.origin.y, tb.origin.x),
            )),
            Object::SubNote(n) if n.is_inline() => Some((
                (id, BlockKind::InlineNote, n.inline_content.clone().unwrap_or_default()),
                (n.origin.y, n.origin.x),
            )),
            _ => None,
        })
        .collect();
    with_pos.sort_by(|a, b| {
        a.1
            .0
            .partial_cmp(&b.1.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.1.partial_cmp(&b.1.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut blocks: Vec<(ObjectId, BlockKind, String)> =
        with_pos.into_iter().map(|(b, _)| b).collect();

    egui::CentralPanel::default()
        .frame(
            egui::Frame::default()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Vec2::new(24.0, 12.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Page view").strong().size(15.0));
                ui.label(
                    RichText::new("(same blocks as the edgeless canvas)")
                        .weak()
                        .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Edgeless view")
                        .on_hover_text("Switch back to the canvas view")
                        .clicked()
                    {
                        actions.leave_markdown = true;
                    }
                });
            });
            ui.separator();

            ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(720.0);
                if blocks.is_empty() {
                    ui.label(
                        RichText::new(
                            "No text blocks yet. Add text or sticky notes on the edgeless canvas \
                             (T or N), then come back here to read & edit them as a flowing page.",
                        )
                        .italics()
                        .weak(),
                    );
                }
                for (id, kind, content) in blocks.iter_mut() {
                    let label = match kind {
                        BlockKind::TextBox => "¶",
                        BlockKind::InlineNote => "□",
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).weak().monospace());
                        let resp = ui.add(
                            TextEdit::multiline(content)
                                .font(FontId::proportional(16.0))
                                .desired_width(640.0)
                                .desired_rows(2),
                        );
                        if resp.changed() {
                            state.pending_edits.push((*id, *kind, content.clone()));
                            actions.source_changed = true;
                        }
                    });
                    ui.add_space(6.0);
                }

                ui.add_space(12.0);
                ui.separator();
                if ui
                    .button("+ new text block")
                    .on_hover_text("Append a TextBox below the last block on the canvas")
                    .clicked()
                {
                    state.append_requested = true;
                }
            });
        });

    // 2. Flush pending edits back into the doc + persist.
    for (id, kind, new_content) in state.pending_edits.drain(..) {
        if let Some(obj) = doc.get_mut(id) {
            match (kind, obj) {
                (BlockKind::TextBox, Object::TextBox(tb)) => {
                    tb.content = new_content;
                    tb.clamp_cursor();
                    if let Some(s) = store {
                        let _ = s.update(id, &Object::TextBox(tb.clone()));
                    }
                }
                (BlockKind::InlineNote, Object::SubNote(n)) => {
                    n.inline_content = Some(new_content);
                    n.clamp_cursor();
                    if let Some(s) = store {
                        let _ = s.update(id, &Object::SubNote(n.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    actions
}
