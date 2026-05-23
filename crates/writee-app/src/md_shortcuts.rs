//! Lightweight markdown line transforms for inline sub-note bodies.
//!
//! This is *render-time only* — the raw markup stays in `inline_content`,
//! so re-loading shows the same source. Each line is classified, given a
//! display font size, and has its prefix glyph-substituted (e.g. `- ` →
//! `• `, `[x] ` → `☑ `). Per-character styling (true bold/italic across
//! parts of a line) needs styled glyphon runs and is deferred.

pub struct RenderedLine {
    pub text: String,
    pub font_size: f32,
}

const BASE_SIZE: f32 = 16.0;

pub fn render_lines(source: &str) -> Vec<RenderedLine> {
    let mut out = Vec::new();
    for raw in source.split('\n') {
        let line = raw.trim_end();
        out.push(transform_line(line));
    }
    out
}

fn transform_line(line: &str) -> RenderedLine {
    // Heading levels.
    if let Some(rest) = line.strip_prefix("### ") {
        return RenderedLine {
            text: rest.to_string(),
            font_size: BASE_SIZE * 1.15,
        };
    }
    if let Some(rest) = line.strip_prefix("## ") {
        return RenderedLine {
            text: rest.to_string(),
            font_size: BASE_SIZE * 1.3,
        };
    }
    if let Some(rest) = line.strip_prefix("# ") {
        return RenderedLine {
            text: rest.to_string(),
            font_size: BASE_SIZE * 1.55,
        };
    }
    // Checkbox first (must come before bullet so "[ ]" doesn't get bulleted).
    if let Some(rest) = line.strip_prefix("[x] ").or_else(|| line.strip_prefix("[X] ")) {
        return RenderedLine {
            text: format!("☑  {rest}"),
            font_size: BASE_SIZE,
        };
    }
    if let Some(rest) = line
        .strip_prefix("[ ] ")
        .or_else(|| line.strip_prefix("[] "))
    {
        return RenderedLine {
            text: format!("☐  {rest}"),
            font_size: BASE_SIZE,
        };
    }
    // Bullet.
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return RenderedLine {
            text: format!("•  {rest}"),
            font_size: BASE_SIZE,
        };
    }
    RenderedLine {
        text: line.to_string(),
        font_size: BASE_SIZE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h1_h2_h3_sized() {
        let lines = render_lines("# big\n## mid\n### small\nplain");
        assert!(lines[0].font_size > lines[1].font_size);
        assert!(lines[1].font_size > lines[2].font_size);
        assert!(lines[3].font_size == BASE_SIZE);
    }

    #[test]
    fn bullet_and_checkbox() {
        let lines = render_lines("- one\n[x] done\n[ ] pending");
        assert!(lines[0].text.starts_with('•'));
        assert!(lines[1].text.starts_with('☑'));
        assert!(lines[2].text.starts_with('☐'));
    }
}
