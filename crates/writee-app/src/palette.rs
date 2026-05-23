//! Command palette — the Ctrl/Cmd-K modal.
//!
//! Three result sections, all filtered by the same query string:
//!   1. **Files** — fuzzy match on filename (subsequence match: chars of
//!      query must appear in order anywhere in the name).
//!   2. **Content** — substring hits inside any file's text, via the
//!      shared `SearchCache`. Triggered only once query.len >= 2 to avoid
//!      thrashing while the user is still typing.
//!   3. **Actions** — a static list (toggle theme, new whiteboard, fit,
//!      export, …) filtered by fuzzy match on the label.

use std::path::PathBuf;

use crate::search;

#[derive(Debug, Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    /// Index of the focused row across the combined result list.
    pub focus: usize,
    /// True once the TextEdit has been auto-focused for the current open
    /// session. Prevents the palette stealing focus every single frame.
    pub focused: bool,
}

impl PaletteState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.focus = 0;
            self.focused = false;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.focus = 0;
        self.focused = false;
    }
}

/// A row the user can pick.
#[derive(Debug, Clone)]
pub enum PaletteRow {
    File(PathBuf),
    Content { file: PathBuf, snippet: String },
    Action(PaletteAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteAction {
    NewWhiteboard,
    FitToContent,
    ToggleMarkdownMode,
    ToggleTheme,
    ExportMarkdown,
    ExportPdf,
    ExportPng,
}

impl PaletteAction {
    pub fn label(self) -> &'static str {
        match self {
            PaletteAction::NewWhiteboard => "New whiteboard",
            PaletteAction::FitToContent => "Fit canvas to content",
            PaletteAction::ToggleMarkdownMode => "Toggle page / edgeless mode",
            PaletteAction::ToggleTheme => "Toggle dark / light theme",
            PaletteAction::ExportMarkdown => "Export current file as Markdown",
            PaletteAction::ExportPdf => "Export current file as PDF",
            PaletteAction::ExportPng => "Export current file as PNG",
        }
    }

    pub fn all() -> [PaletteAction; 7] {
        [
            PaletteAction::NewWhiteboard,
            PaletteAction::FitToContent,
            PaletteAction::ToggleMarkdownMode,
            PaletteAction::ToggleTheme,
            PaletteAction::ExportMarkdown,
            PaletteAction::ExportPdf,
            PaletteAction::ExportPng,
        ]
    }
}

/// Sublime-text-style subsequence match: every char of `needle` must appear
/// in `haystack` in order (case-insensitive). Empty needle matches everything.
pub fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let h: String = haystack.to_lowercase();
    let mut it = h.chars();
    for c in needle.to_lowercase().chars() {
        match it.find(|x| *x == c) {
            Some(_) => continue,
            None => return false,
        }
    }
    true
}

/// Build the result list. Called per-frame while the palette is open.
pub fn collect_rows(
    state: &PaletteState,
    workspace_files: &[PathBuf],
    search_cache: &mut search::SearchCache,
    workspace_root: &std::path::Path,
) -> Vec<PaletteRow> {
    let q = state.query.trim();
    let mut out: Vec<PaletteRow> = Vec::new();

    // Actions first — small, always-visible list of jump-to-feature shortcuts.
    for a in PaletteAction::all() {
        if fuzzy_match(a.label(), q) {
            out.push(PaletteRow::Action(a));
        }
    }

    for path in workspace_files {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if fuzzy_match(name, q) {
            out.push(PaletteRow::File(path.clone()));
        }
    }

    if q.len() >= 2 {
        for hit in search_cache.search(workspace_root, q) {
            out.push(PaletteRow::Content {
                file: hit.file,
                snippet: hit.snippet,
            });
        }
    }

    out
}
