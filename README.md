# writee

A cross-platform handwriting whiteboard, written in Rust. Infinite canvas,
pressure-sensitive pen, eraser, arrows for mindmapping, text, and
selection — exported to a self-contained web folder when you want to share.

The UI is intentionally minimal: black, white, and a light dot grid.

## Running on desktop

```
cargo run --release -p writee-desktop
```

On first launch the app creates `~/Documents/Writee/` as its workspace and
opens `default.writee` inside it. Each whiteboard is a SQLite file; you can
sync the folder with anything (Syncthing, Dropbox, git-annex).

## Tools

| Tool        | Keyboard | What it does                                   |
| ----------- | -------- | ---------------------------------------------- |
| Pen         | `P`      | Black ink, pressure-responsive width           |
| Highlighter | `H`      | Semi-transparent yellow highlighter            |
| Eraser      | `E`      | Object-eraser; drag through ink to delete      |
| Arrow       | `A`      | Drag to draw a parametric arrow                |
| Text        | `T`      | Click to place a text box; click again to edit |
| Select      | `S`      | Click an object, drag to move; drag empty space to marquee-select |

The toolbar at the top mirrors these and adds runtime sliders for stroke
width, eraser radius, and text size, plus a **Pressure** checkbox that
toggles pressure-sensitive width on/off.

## Canvas

- Middle/right-button drag → pan
- Mouse wheel → zoom (pivots on cursor)
- `Ctrl+F` or the **Fit** button → frame the document
- Shift- or Ctrl-click an object → add it to the current selection

## File ops

| Shortcut         | Action                                                    |
| ---------------- | --------------------------------------------------------- |
| `Ctrl+N`         | New whiteboard in workspace                               |
| `Ctrl+O`         | Cycle to next `.writee` in workspace                      |
| `Ctrl+E`         | Export current doc to `<name>-export/` (static web view)  |
| `Ctrl+Z`         | Undo                                                      |
| `Ctrl+Shift+Z`   | Redo (also `Ctrl+Y`)                                      |
| `Delete`/`Backspace` | Remove current selection                              |
| `Esc`            | Clear selection / cancel text edit / cancel marquee       |

## Architecture

Cargo workspace, five core crates plus the desktop/Android binaries:

- `writee-core` — data model, stroke math (one-Euro filter, Catmull-Rom
  resample, variable-width SDF tessellation), SQLite store with R-tree
  spatial index.
- `writee-render` — wgpu pipelines (dot grid, ink with per-vertex color &
  SDF anti-aliasing, glyphon-backed text, egui chrome pass).
- `writee-input` — `InkSample` abstraction over winit pointer/touch events
  (pressure, tilt, tool-type, palm-rejection-friendly phases).
- `writee-app` — winit `ApplicationHandler`, tool state machine, undo log,
  workspace management, toolbar UI.
- `writee-export-web` — emits a static-site viewer (HTML/CSS/JS) +
  `doc.js` JSON payload.

Renderer details: one shared `Viewport` uniform; ink and grid pipelines
share it. Ink vertices are `(pos, signed_offset, half_width, color)` so
the fragment shader can render any color with proper SDF-based AA. The
committed document is tessellated into a cache that the App invalidates on
every mutation; only the wet stroke + selection overlay is re-tessellated
per frame.

## Web export

`Ctrl+E` (or the toolbar **Export** button) writes a folder next to your
`.writee`:

```
default-export/
  index.html
  app.js
  app.css
  doc.js          # window.WRITEE_DOC = {...}
```

Open `index.html` directly in any browser — no server, no dependencies.
Pan/zoom/pinch supported. The viewer re-runs the same Catmull-Rom +
variable-width algorithm as the editor, so strokes match.

## Android

The `apps/android` crate is a scaffold (cdylib + `android_main`). It builds
on a host as an empty stub; producing an APK requires the Android NDK and
either `cargo install cargo-apk` or `cargo install xbuild`. See
`apps/android/README.md` for build steps and the list of known gaps the
first device run needs to address (palm rejection, SAF picker, etc.).

## Status

Tests pass on desktop (host build). Stroke quality has been tuned but
benefits from a tablet in hand — the relevant knobs (`mincutoff`, `beta`,
`PRESSURE_GAMMA`, `MIN_HALF_WIDTH`) live in
`crates/writee-core/src/{one_euro.rs,tessellate.rs}`.
