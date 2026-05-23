//! Workspace-wide substring search.
//!
//! Walks every `.writee` in the workspace, scans the user-visible text
//! inside `TextBox`, inline `SubNote`, and `SubNote.title`, and returns
//! hits with a short snippet centred on the match.
//!
//! v1 is naive O(files × objects) — fine to ~10k objects. An LRU cache
//! keyed by `(path, mtime)` prevents re-walking unchanged files between
//! consecutive queries from the command palette.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use writee_core::{DocStore, Object, ObjectId};

/// One match emitted by `search_workspace`.
#[derive(Debug, Clone)]
pub struct Hit {
    pub file: PathBuf,
    pub object_id: ObjectId,
    pub kind: HitKind,
    pub snippet: String,
}

#[derive(Debug, Clone, Copy)]
pub enum HitKind {
    TextBox,
    NoteBody,
    NoteTitle,
}

/// LRU-ish cache (just a HashMap; workspace size is small enough).
#[derive(Default)]
pub struct SearchCache {
    entries: HashMap<PathBuf, CacheEntry>,
}

struct CacheEntry {
    mtime: Option<SystemTime>,
    /// Pre-extracted (object_id, kind, lowercased text) for every text-bearing
    /// object. Kept as `String` so the hot path is just `contains`.
    rows: Vec<(ObjectId, HitKind, String, String)>, // (id, kind, lower, original)
}

impl SearchCache {
    pub fn search(&mut self, root: &Path, query: &str) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let Ok(read) = std::fs::read_dir(root) else { return out };
        for entry in read.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("writee") {
                continue;
            }
            self.ensure_fresh(&path);
            let Some(ce) = self.entries.get(&path) else { continue };
            for (id, kind, lower, original) in &ce.rows {
                if let Some(idx) = lower.find(&q) {
                    out.push(Hit {
                        file: path.clone(),
                        object_id: *id,
                        kind: *kind,
                        snippet: snippet_around(original, idx, q.len()),
                    });
                    // Cap per-file matches so a giant doc doesn't drown out
                    // matches from other files.
                    if out.iter().filter(|h| h.file == path).count() >= 5 {
                        break;
                    }
                }
            }
        }
        out
    }

    fn ensure_fresh(&mut self, path: &Path) {
        let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        if let Some(existing) = self.entries.get(path) {
            if existing.mtime == mtime {
                return;
            }
        }
        let Ok(store) = DocStore::open(path) else {
            self.entries.remove(path);
            return;
        };
        let Ok(doc) = store.load_all() else {
            self.entries.remove(path);
            return;
        };
        let mut rows: Vec<(ObjectId, HitKind, String, String)> = Vec::new();
        for (id, obj) in doc.objects() {
            match obj {
                Object::TextBox(tb) if !tb.content.is_empty() => {
                    rows.push((id, HitKind::TextBox, tb.content.to_lowercase(), tb.content.clone()));
                }
                Object::SubNote(n) => {
                    if !n.title.is_empty() {
                        rows.push((id, HitKind::NoteTitle, n.title.to_lowercase(), n.title.clone()));
                    }
                    if let Some(body) = &n.inline_content {
                        if !body.is_empty() {
                            rows.push((id, HitKind::NoteBody, body.to_lowercase(), body.clone()));
                        }
                    }
                }
                _ => {}
            }
        }
        self.entries.insert(path.to_path_buf(), CacheEntry { mtime, rows });
    }
}

fn snippet_around(text: &str, match_byte: usize, match_len: usize) -> String {
    const PAD: usize = 30;
    let start = text[..match_byte]
        .char_indices()
        .rev()
        .nth(PAD)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let after = match_byte + match_len;
    let end = text[after..]
        .char_indices()
        .nth(PAD)
        .map(|(i, _)| after + i)
        .unwrap_or(text.len());
    let mut snip = String::new();
    if start > 0 {
        snip.push('…');
    }
    snip.push_str(&text[start..end].replace('\n', " "));
    if end < text.len() {
        snip.push('…');
    }
    snip
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_centres_around_match() {
        let s = "the quick brown fox jumps over the lazy dog";
        let snip = snippet_around(s, s.find("fox").unwrap(), 3);
        assert!(snip.contains("fox"));
    }
}
