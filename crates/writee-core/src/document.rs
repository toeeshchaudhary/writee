use crate::arrow::Arrow;
use crate::geom::Aabb;
use crate::link::{Anchor, Link};
use crate::shape::Shape;
use crate::stroke::Stroke;
use crate::subnote::SubNote;
use crate::textbox::TextBox;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Bitmap block placed on the canvas. Source bytes are the original encoded
/// PNG/JPEG so we don't lose compression on round-trip; the renderer decodes
/// once per file load and caches the resulting GPU texture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageBlock {
    pub origin: Vec2,
    pub size: Vec2,
    /// Encoded image bytes (PNG or JPEG).
    pub bytes: Vec<u8>,
    /// Logical width / height of the image in pixels — handy for "fit to
    /// native size" without round-tripping through the decoder.
    pub natural_w: u32,
    pub natural_h: u32,
}

impl ImageBlock {
    pub fn bbox(&self) -> Aabb {
        Aabb { min: self.origin, max: self.origin + self.size }
    }
}

pub type ObjectId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Object {
    Stroke(Stroke),
    Arrow(Arrow),
    TextBox(TextBox),
    Shape(Shape),
    SubNote(SubNote),
    Link(Link),
    Image(ImageBlock),
}

impl Object {
    pub fn bbox(&self) -> Aabb {
        match self {
            Object::Stroke(s) => s.bbox(),
            Object::Arrow(a) => a.bbox(),
            Object::TextBox(t) => t.bbox(),
            Object::Shape(s) => s.bbox(),
            Object::SubNote(n) => n.bbox(),
            Object::Image(i) => i.bbox(),
            // Links don't have a stable bbox; computed at render time from
            // resolved endpoints.
            Object::Link(_) => Aabb {
                min: glam::Vec2::ZERO,
                max: glam::Vec2::ZERO,
            },
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Object::Stroke(_) => "stroke",
            Object::Arrow(_) => "arrow",
            Object::TextBox(_) => "text",
            Object::Shape(_) => "shape",
            Object::SubNote(_) => "note",
            Object::Link(_) => "link",
            Object::Image(_) => "image",
        }
    }

    /// Whether this kind of object can serve as a link anchor target.
    /// Links themselves can't be anchored to (avoid recursion); everything
    /// else can.
    pub fn is_anchorable(&self) -> bool {
        !matches!(self, Object::Link(_))
    }

    /// World-space anchor position for this object. Returns `None` for
    /// objects without a stable bbox (links).
    pub fn anchor_pos(&self, anchor: Anchor) -> Option<glam::Vec2> {
        if !self.is_anchorable() {
            return None;
        }
        Some(anchor.position(self.bbox()))
    }
}

#[derive(Debug, Default)]
pub struct Document {
    next_id: ObjectId,
    objects: Vec<(ObjectId, Object)>,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_records(records: Vec<(ObjectId, Object)>) -> Self {
        let next_id = records.iter().map(|(id, _)| *id).max().unwrap_or(0);
        Self { next_id, objects: records }
    }

    pub fn add(&mut self, obj: Object) -> ObjectId {
        self.next_id += 1;
        let id = self.next_id;
        self.objects.push((id, obj));
        id
    }

    pub fn reinsert(&mut self, id: ObjectId, obj: Object) {
        if id > self.next_id {
            self.next_id = id;
        }
        let pos = self
            .objects
            .binary_search_by_key(&id, |(i, _)| *i)
            .unwrap_or_else(|p| p);
        self.objects.insert(pos, (id, obj));
    }

    pub fn remove(&mut self, id: ObjectId) -> Option<Object> {
        let pos = self.objects.iter().position(|(i, _)| *i == id)?;
        // Removing an anchorable object orphans any links pointing at it.
        // Caller may want to handle that; for v1 we leave orphans in place
        // (they render as zero-length lines until manually cleaned).
        Some(self.objects.remove(pos).1)
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.objects.iter().find(|(i, _)| *i == id).map(|(_, o)| o)
    }

    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.objects.iter_mut().find(|(i, _)| *i == id).map(|(_, o)| o)
    }

    pub fn objects(&self) -> impl Iterator<Item = (ObjectId, &Object)> {
        self.objects.iter().map(|(id, o)| (*id, o))
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn pick(&self, world_pt: glam::Vec2, slack: f32) -> Option<ObjectId> {
        let mut best: Option<(ObjectId, f32)> = None;
        for (id, obj) in &self.objects {
            if !obj.is_anchorable() {
                continue;
            }
            let bb = obj.bbox();
            if world_pt.x < bb.min.x - slack
                || world_pt.x > bb.max.x + slack
                || world_pt.y < bb.min.y - slack
                || world_pt.y > bb.max.y + slack
            {
                continue;
            }
            let d = match obj {
                Object::Stroke(s) => distance_to_stroke(s, world_pt),
                Object::Arrow(a) => distance_to_segment(a.start, a.end, world_pt),
                // Image / shape / subnote / textbox already passed the
                // bbox-with-slack check above; treat them as inside.
                _ => 0.0,
            };
            if d <= slack {
                match best {
                    Some((_, bd)) if d >= bd => {}
                    _ => best = Some((*id, d)),
                }
            }
        }
        best.map(|(id, _)| id)
    }

    pub fn pick_in_rect(&self, min: glam::Vec2, max: glam::Vec2) -> Vec<ObjectId> {
        let mut out = Vec::new();
        for (id, obj) in &self.objects {
            if !obj.is_anchorable() {
                continue;
            }
            let bb = obj.bbox();
            if bb.max.x < min.x || bb.min.x > max.x {
                continue;
            }
            if bb.max.y < min.y || bb.min.y > max.y {
                continue;
            }
            out.push(*id);
        }
        out
    }

    pub fn translate(&mut self, id: ObjectId, delta: glam::Vec2) -> bool {
        let Some(obj) = self.get_mut(id) else { return false };
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
            // Links follow their endpoint objects, so direct translation
            // is a no-op.
            Object::Link(_) => return false,
        }
        true
    }

    /// Find the object whose nearest anchor is within `slack` of `world_pt`.
    /// Returns `(object_id, anchor, anchor_world_pos)` so the link tool can
    /// commit a real connection with the right anchor preset.
    pub fn pick_anchor(
        &self,
        world_pt: glam::Vec2,
        slack: f32,
    ) -> Option<(ObjectId, Anchor, glam::Vec2)> {
        let mut best: Option<(ObjectId, Anchor, glam::Vec2, f32)> = None;
        for (id, obj) in &self.objects {
            if !obj.is_anchorable() {
                continue;
            }
            let bb = obj.bbox();
            // Quick reject if even the closest anchor would be > slack away.
            let pad = slack;
            if world_pt.x < bb.min.x - pad
                || world_pt.x > bb.max.x + pad
                || world_pt.y < bb.min.y - pad
                || world_pt.y > bb.max.y + pad
            {
                continue;
            }
            for anchor in Anchor::all() {
                let pos = anchor.position(bb);
                let d = (pos - world_pt).length();
                if d <= slack {
                    match best {
                        Some((_, _, _, bd)) if d >= bd => {}
                        _ => best = Some((*id, anchor, pos, d)),
                    }
                }
            }
        }
        best.map(|(id, anc, pos, _)| (id, anc, pos))
    }
}

fn distance_to_stroke(s: &Stroke, p: glam::Vec2) -> f32 {
    if s.points.len() < 2 {
        return f32::INFINITY;
    }
    let mut best = f32::INFINITY;
    for w in s.points.windows(2) {
        let a = glam::Vec2::new(w[0].x, w[0].y);
        let b = glam::Vec2::new(w[1].x, w[1].y);
        let d = distance_to_segment(a, b, p);
        if d < best {
            best = d;
        }
    }
    best
}

fn distance_to_segment(a: glam::Vec2, b: glam::Vec2, p: glam::Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-6 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}
