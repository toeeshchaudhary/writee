//! Committed-geometry cache.
//!
//! Naïvely re-tessellating the whole document every frame chokes once you
//! have more than ~500 strokes. This cache stores the tessellated bytes for
//! everything *committed* to the document; the App only invalidates it on
//! object add/remove/edit. Each frame copies the cache and appends live
//! preview geometry (wet stroke, arrow drag, selection cue, marquee).
//!
//! This is the M2-polish stand-in for the proper tile cache in the original
//! plan. Cheaper to implement and good enough for whiteboards in the
//! single-digit-thousands of objects range.

use crate::settings::ToolSettings;
use writee_core::{
    tessellate_arrow, tessellate_ellipse, tessellate_line, tessellate_opts, tessellate_rect,
    tessellate_rect_outline, tessellate_segment_strip, ColorTheme, Document, InkVertex, Object,
    ShapeKind, COLOR_LINK,
};

pub struct CommittedCache {
    pub verts: Vec<InkVertex>,
    dirty: bool,
}

impl Default for CommittedCache {
    fn default() -> Self {
        Self { verts: Vec::new(), dirty: true }
    }
}

impl CommittedCache {
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn ensure_fresh(&mut self, doc: &Document, settings: &ToolSettings, theme: &ColorTheme) {
        if !self.dirty {
            return;
        }
        self.verts.clear();
        let mut append = |chunk: Vec<InkVertex>| {
            if chunk.is_empty() {
                return;
            }
            if !self.verts.is_empty() {
                let last = *self.verts.last().unwrap();
                let first = chunk[0];
                self.verts.push(last);
                self.verts.push(first);
            }
            self.verts.extend(chunk);
        };
        for (_, obj) in doc.objects() {
            match obj {
                Object::Stroke(s) => append(tessellate_opts(
                    &s.points,
                    s.width_base,
                    s.effective_color(),
                    settings.pressure_sensitive,
                    settings.tilt_modulation,
                )),
                Object::Arrow(a) => append(tessellate_arrow(a, theme.ink)),
                Object::TextBox(_) => {} // glyphon handles text
                Object::Shape(s) => {
                    let min = s.a.min(s.b);
                    let max = s.a.max(s.b);
                    match s.kind {
                        ShapeKind::Rectangle => {
                            if s.filled {
                                append(tessellate_rect(min, max, s.color));
                            } else {
                                append(tessellate_rect_outline(
                                    min,
                                    max,
                                    s.stroke_width * 0.5,
                                    s.color,
                                ));
                            }
                        }
                        ShapeKind::Ellipse => {
                            append(tessellate_ellipse(min, max, s.color, s.filled));
                        }
                        ShapeKind::Line => {
                            append(tessellate_line(s.a, s.b, s.stroke_width, s.color));
                        }
                    }
                }
                Object::SubNote(n) => {
                    let min = n.origin;
                    let max = n.origin + n.size;
                    let shadow_offset = glam::Vec2::new(3.0, 4.0);
                    append(tessellate_rect(
                        min + shadow_offset,
                        max + shadow_offset,
                        theme.card_shadow,
                    ));
                    let card_fill = if n.is_index {
                        theme.card_index_fill
                    } else if n.is_inline() {
                        theme.card_inline_fill
                    } else {
                        theme.card_linked_fill
                    };
                    let card_border = if n.locked {
                        theme.card_border_locked
                    } else {
                        theme.card_border
                    };
                    append(tessellate_rect(min, max, card_fill));
                    append(tessellate_rect_outline(min, max, 0.9, card_border));
                    if n.has_title_bar() {
                        let title_bar_y = min.y + 28.0;
                        append(tessellate_segment_strip(
                            glam::Vec2::new(min.x, title_bar_y),
                            glam::Vec2::new(max.x, title_bar_y),
                            0.5,
                            card_border,
                        ));
                    }
                }
                Object::Link(l) => {
                    // Look up both endpoints. If either is missing, render an
                    // orphan stub so the user can see + delete the dead link.
                    let from = doc.get(l.from.object_id).and_then(|o| o.anchor_pos(l.from.anchor));
                    let to = doc.get(l.to.object_id).and_then(|o| o.anchor_pos(l.to.anchor));
                    if let (Some(a), Some(b)) = (from, to) {
                        let half = l.width * 0.5;
                        append(tessellate_segment_strip(a, b, half, l.color));
                        // Small disc at each endpoint to make connections
                        // feel explicit.
                        let r = l.width * 1.4;
                        append(tessellate_ellipse(
                            a - glam::Vec2::splat(r),
                            a + glam::Vec2::splat(r),
                            l.color,
                            true,
                        ));
                        append(tessellate_ellipse(
                            b - glam::Vec2::splat(r),
                            b + glam::Vec2::splat(r),
                            l.color,
                            true,
                        ));
                    } else {
                        log::trace!("orphan link {}->{}", l.from.object_id, l.to.object_id);
                    }
                    let _ = COLOR_LINK; // reserved fallback if l.color is 0
                }
                // Images are drawn by the dedicated image pipeline in the
                // renderer; nothing to add to the ink vertex stream.
                Object::Image(_) => {}
            }
        }
        self.dirty = false;
    }
}
