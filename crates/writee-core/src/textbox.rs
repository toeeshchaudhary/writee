use crate::geom::Aabb;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// A floating text label anchored in world space. `font_size` is in world units
/// so text zooms with the canvas the way ink does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextBox {
    pub origin: Vec2,
    pub font_size: f32,
    pub content: String,
    /// Optional font family name. Resolved app-side from the user's font slot
    /// mapping. `None` ⇒ glyphon's default sans-serif.
    #[serde(default)]
    pub font_name: Option<String>,
    /// Insertion-point byte offset into `content`. Skipped during serialization
    /// so we don't pollute saved files with transient editor state; defaults to
    /// 0 on load (which `clamp_cursor` then re-pins to the end on first edit).
    #[serde(skip)]
    pub cursor: usize,
}

impl TextBox {
    pub fn new(origin: Vec2, font_size: f32) -> Self {
        Self {
            origin,
            font_size,
            content: String::new(),
            font_name: None,
            cursor: 0,
        }
    }

    /// Clamp the cursor to a valid byte boundary inside `content`. Useful
    /// after deserialization or content mutation.
    pub fn clamp_cursor(&mut self) {
        if self.cursor > self.content.len() {
            self.cursor = self.content.len();
        }
        // Snap to the next valid char boundary if we landed in the middle of
        // a multi-byte UTF-8 sequence.
        while self.cursor > 0 && !self.content.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    /// (line, column-in-bytes) for the cursor. Column is byte offset from the
    /// previous '\n'; line is 0-indexed.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let upto = &self.content[..self.cursor.min(self.content.len())];
        let line = upto.bytes().filter(|b| *b == b'\n').count();
        let col = upto.rfind('\n').map(|i| self.cursor - i - 1).unwrap_or(self.cursor);
        (line, col)
    }

    /// Approximate caret world-position using the same rough metrics
    /// [`TextBox::bbox`] uses (0.55 em char width, 1.25 em line height).
    /// Good enough for v1; will drift from true glyph layout for variable
    /// fonts, which is acceptable while typing.
    pub fn cursor_world_pos(&self) -> Vec2 {
        let (line, col_bytes) = self.cursor_line_col();
        // Count chars (not bytes) on the cursor's line up to the cursor.
        let upto = &self.content[..self.cursor.min(self.content.len())];
        let line_start = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col_chars = self.content[line_start..line_start + col_bytes]
            .chars()
            .count();
        let x = self.origin.x + (col_chars as f32) * self.font_size * 0.55;
        let y = self.origin.y + (line as f32) * self.font_size * 1.25;
        Vec2::new(x, y)
    }

    /// Insert a single char at the cursor and advance the cursor past it.
    pub fn insert_at_cursor(&mut self, ch: char) {
        self.clamp_cursor();
        self.content.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Delete the char before the cursor (if any). Returns true if anything
    /// was removed.
    pub fn backspace_at_cursor(&mut self) -> bool {
        self.clamp_cursor();
        if self.cursor == 0 {
            return false;
        }
        // Walk back to the start of the previous char.
        let mut start = self.cursor - 1;
        while start > 0 && !self.content.is_char_boundary(start) {
            start -= 1;
        }
        self.content.replace_range(start..self.cursor, "");
        self.cursor = start;
        true
    }

    /// Cursor → previous char boundary. No-op at start of content.
    pub fn cursor_left(&mut self) {
        self.clamp_cursor();
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        while self.cursor > 0 && !self.content.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    /// Cursor → next char boundary. No-op at end of content.
    pub fn cursor_right(&mut self) {
        self.clamp_cursor();
        if self.cursor >= self.content.len() {
            return;
        }
        self.cursor += 1;
        while self.cursor < self.content.len()
            && !self.content.is_char_boundary(self.cursor)
        {
            self.cursor += 1;
        }
    }

    /// Cursor → start of current line.
    pub fn cursor_home(&mut self) {
        self.clamp_cursor();
        let upto = &self.content[..self.cursor];
        self.cursor = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
    }

    /// Cursor → end of current line (just before the trailing '\n', if any).
    pub fn cursor_end(&mut self) {
        self.clamp_cursor();
        let rest = &self.content[self.cursor..];
        self.cursor += rest.find('\n').unwrap_or(rest.len());
    }

    /// Cursor → same column on the previous line. Falls off to start of
    /// content when already on the first line.
    pub fn cursor_up(&mut self) {
        self.clamp_cursor();
        let (line, _) = self.cursor_line_col();
        if line == 0 {
            self.cursor = 0;
            return;
        }
        let upto = &self.content[..self.cursor];
        let line_start = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col_chars = self.content[line_start..self.cursor].chars().count();
        let prev_line_end = line_start.saturating_sub(1); // position of the '\n' itself
        let prev_line_start = self.content[..prev_line_end]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let target = self.byte_offset_at_col(prev_line_start, prev_line_end, col_chars);
        self.cursor = target;
    }

    /// Cursor → same column on the next line. Falls off to end of content
    /// when already on the last line.
    pub fn cursor_down(&mut self) {
        self.clamp_cursor();
        let upto = &self.content[..self.cursor];
        let line_start = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col_chars = self.content[line_start..self.cursor].chars().count();
        let line_end = self.cursor
            + self.content[self.cursor..]
                .find('\n')
                .unwrap_or(self.content.len() - self.cursor);
        if line_end >= self.content.len() {
            self.cursor = self.content.len();
            return;
        }
        let next_line_start = line_end + 1;
        let next_line_end = next_line_start
            + self.content[next_line_start..]
                .find('\n')
                .unwrap_or(self.content.len() - next_line_start);
        self.cursor = self.byte_offset_at_col(next_line_start, next_line_end, col_chars);
    }

    fn byte_offset_at_col(&self, line_start: usize, line_end: usize, target_col_chars: usize) -> usize {
        let line = &self.content[line_start..line_end];
        let mut consumed = 0usize;
        for (i, _) in line.char_indices() {
            if consumed == target_col_chars {
                return line_start + i;
            }
            consumed += 1;
        }
        line_end
    }

    pub fn bbox(&self) -> Aabb {
        // Rough estimate without measuring glyphs: ~0.55 em per char width,
        // 1.25 em line height. Plenty for spatial-index culling.
        let lines = self.content.split('\n');
        let mut max_chars = 0usize;
        let mut line_count = 0usize;
        for line in lines {
            line_count += 1;
            max_chars = max_chars.max(line.chars().count());
        }
        if line_count == 0 {
            line_count = 1;
        }
        let w = (max_chars.max(1) as f32) * self.font_size * 0.55;
        let h = (line_count as f32) * self.font_size * 1.25;
        Aabb {
            min: self.origin,
            max: self.origin + Vec2::new(w.max(self.font_size), h),
        }
    }
}
