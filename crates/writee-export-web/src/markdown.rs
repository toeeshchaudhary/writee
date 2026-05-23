//! Per-file markdown export.
//!
//! Walks a `Document`'s text-bearing blocks in reading order (Y then X) and
//! concatenates them as plain markdown. Title blocks for linked SubNote
//! cards become `[card title](target.writee)` references so the user can
//! navigate the export the same way they navigate inside writee.

use writee_core::{Document, Object};

pub fn render(doc: &Document, title: &str) -> String {
    let mut blocks: Vec<((f32, f32), String)> = Vec::new();
    for (_, obj) in doc.objects() {
        match obj {
            Object::TextBox(tb) => {
                if !tb.content.is_empty() {
                    blocks.push(((tb.origin.y, tb.origin.x), tb.content.clone()));
                }
            }
            Object::SubNote(n) => {
                let mut body = String::new();
                if !n.title.is_empty() {
                    body.push_str(&format!("### {}\n", n.title));
                }
                if let Some(inline) = &n.inline_content {
                    if !inline.is_empty() {
                        body.push_str(inline);
                    }
                } else if !n.target_file.is_empty() {
                    body.push_str(&format!(
                        "[Open linked note]({})",
                        n.target_file
                    ));
                }
                if !body.is_empty() {
                    blocks.push(((n.origin.y, n.origin.x), body));
                }
            }
            _ => {}
        }
    }
    blocks.sort_by(|a, b| {
        a.0.0
            .partial_cmp(&b.0.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.1.partial_cmp(&b.0.1).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut out = String::new();
    if !title.is_empty() {
        out.push_str(&format!("# {title}\n\n"));
    }
    for (_, body) in blocks {
        out.push_str(&body);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}
