//! Serialize a [`writee_core::Document`] into a static-site viewer folder.
//!
//! The output directory contains a self-contained viewer (`index.html`,
//! `app.js`, `app.css`) plus the document data in `doc.js` (assigned to
//! `window.WRITEE_DOC`). The file URL scheme is easier than `doc.json` +
//! `fetch()` because browsers refuse `fetch('file://...')` from
//! `file://`-loaded HTML. With an inline `doc.js`, double-clicking the
//! `index.html` just works.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::Path;
pub mod markdown;

use writee_core::{Document, Object};

const INDEX_HTML: &str = include_str!("../../../viewer-template/index.html");
const APP_JS: &str = include_str!("../../../viewer-template/app.js");
const APP_CSS: &str = include_str!("../../../viewer-template/app.css");

#[derive(Serialize)]
struct ExportedDoc {
    version: u32,
    objects: Vec<ExportedObj>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ExportedObj {
    Stroke {
        width: f32,
        color: [u8; 4],
        points: Vec<[f32; 3]>,
    },
    Arrow {
        start: [f32; 2],
        end: [f32; 2],
        width: f32,
        head: f32,
    },
    Text {
        x: f32,
        y: f32,
        size: f32,
        content: String,
        color: [u8; 4],
    },
    Shape {
        shape: &'static str,
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
        filled: bool,
        color: [u8; 4],
    },
    Note {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        title: String,
        target: String,
    },
    Link {
        from_id: u64,
        to_id: u64,
        width: f32,
        color: [u8; 4],
    },
}

pub fn export_to_folder(doc: &Document, out: &Path) -> Result<()> {
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;

    let exported = build_exported(doc);
    let json = serde_json::to_string(&exported).context("serializing doc")?;
    let doc_js = format!("window.WRITEE_DOC = {json};\n");

    fs::write(out.join("index.html"), INDEX_HTML).context("writing index.html")?;
    fs::write(out.join("app.js"), APP_JS).context("writing app.js")?;
    fs::write(out.join("app.css"), APP_CSS).context("writing app.css")?;
    fs::write(out.join("doc.js"), doc_js).context("writing doc.js")?;

    log::info!(
        "exported {} objects to {}",
        exported.objects.len(),
        out.display()
    );
    Ok(())
}

fn build_exported(doc: &Document) -> ExportedDoc {
    let mut objects = Vec::new();
    for (_, obj) in doc.objects() {
        match obj {
            Object::Stroke(s) => {
                let color = if s.color == 1 {
                    [255, 220, 60, 110]
                } else {
                    [18, 18, 18, 255]
                };
                let points: Vec<[f32; 3]> = s
                    .points
                    .iter()
                    .map(|p| [round2(p.x), round2(p.y), round2(p.pressure)])
                    .collect();
                objects.push(ExportedObj::Stroke { width: s.width_base, color, points });
            }
            Object::Arrow(a) => objects.push(ExportedObj::Arrow {
                start: [round2(a.start.x), round2(a.start.y)],
                end: [round2(a.end.x), round2(a.end.y)],
                width: a.width,
                head: a.head_size,
            }),
            Object::TextBox(t) => objects.push(ExportedObj::Text {
                x: round2(t.origin.x),
                y: round2(t.origin.y),
                size: t.font_size,
                content: t.content.clone(),
                color: [18, 18, 18, 255],
            }),
            Object::Shape(s) => {
                let kind = match s.kind {
                    writee_core::ShapeKind::Rectangle => "rect",
                    writee_core::ShapeKind::Ellipse => "ellipse",
                    writee_core::ShapeKind::Line => "line",
                };
                objects.push(ExportedObj::Shape {
                    shape: kind,
                    a: [round2(s.a.x), round2(s.a.y)],
                    b: [round2(s.b.x), round2(s.b.y)],
                    width: s.stroke_width,
                    filled: s.filled,
                    color: s.color,
                });
            }
            Object::SubNote(n) => objects.push(ExportedObj::Note {
                x: round2(n.origin.x),
                y: round2(n.origin.y),
                w: round2(n.size.x),
                h: round2(n.size.y),
                title: n.title.clone(),
                target: n.target_file.clone(),
            }),
            Object::Link(l) => objects.push(ExportedObj::Link {
                from_id: l.from.object_id,
                to_id: l.to.object_id,
                width: l.width,
                color: l.color,
            }),
            // Web export of images: skip for v1 (would need data: URLs +
            // viewer-side <img> rendering). The PNG/PDF exports in Phase 5
            // handle them properly.
            Object::Image(_) => {}
        }
    }
    ExportedDoc { version: 1, objects }
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}
