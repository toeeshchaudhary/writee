//! `#tag` parsing + workspace-wide aggregation.
//!
//! Tags live in the user's own text — no separate storage. Anything matching
//! `#[a-zA-Z][a-zA-Z0-9_/-]*` inside a `TextBox.content` or inline
//! `SubNote.inline_content` counts as a tag. Slash means hierarchy:
//! `#proj/foo` and `#proj/bar` both nest under `proj`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use writee_core::{DocStore, Object};

/// Pull every `#tag` token out of a freeform text body. Returns lowercase
/// tags without the leading `#`. Duplicates removed, order preserved.
pub fn extract_tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'#' {
            i += 1;
            continue;
        }
        // Tags must start with an ASCII letter (so `#1foo` isn't a tag and
        // `#` by itself isn't either).
        let start = i + 1;
        if start >= bytes.len() || !bytes[start].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let mut end = start;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'/' || c == b'-' {
                end += 1;
            } else {
                break;
            }
        }
        if end > start {
            let tag = std::str::from_utf8(&bytes[start..end])
                .unwrap_or("")
                .to_lowercase();
            if !tag.is_empty() && !out.iter().any(|t| t == &tag) {
                out.push(tag);
            }
        }
        i = end;
    }
    out
}

/// Workspace-wide aggregation: tag → files containing it.
pub fn all_tags(root: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let Ok(read) = std::fs::read_dir(root) else { return out };
    for entry in read.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("writee") {
            continue;
        }
        let Ok(store) = DocStore::open(&path) else { continue };
        let Ok(doc) = store.load_all() else { continue };
        let mut seen_in_file: Vec<String> = Vec::new();
        for (_, obj) in doc.objects() {
            let text: Option<&str> = match obj {
                Object::TextBox(tb) => Some(&tb.content),
                Object::SubNote(n) => n.inline_content.as_deref(),
                _ => None,
            };
            let Some(text) = text else { continue };
            for tag in extract_tags(text) {
                if !seen_in_file.iter().any(|t| t == &tag) {
                    seen_in_file.push(tag);
                }
            }
        }
        for tag in seen_in_file {
            out.entry(tag).or_default().push(path.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_simple_and_nested() {
        let tags = extract_tags("hello #foo world #proj/bar and #foo again");
        assert_eq!(tags, vec!["foo".to_string(), "proj/bar".to_string()]);
    }

    #[test]
    fn rejects_leading_digit_and_bare_hash() {
        assert!(extract_tags("#1foo #").is_empty());
    }

    #[test]
    fn case_folds() {
        let tags = extract_tags("#Foo #FOO");
        assert_eq!(tags, vec!["foo".to_string()]);
    }
}
