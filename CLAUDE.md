# writee — project state for the next session

A cross-platform Rust handwriting whiteboard + linked-notes app, intended
for shipping. Pen + stylus pressure, eraser, arrows, shapes, text, three
note flavours (inline sticky / linked file / index of files), smart
connector links, infinite canvas, embedded images, light + dark theme,
full-text search, command palette, tag filter, backlinks panel, soft
delete + recovery autosave, markdown + web export. Affine-style two-view
(edgeless ↔ page) over the same underlying objects.

This file is a handoff for resuming work in a new Claude session. Read top
to bottom before making changes.

---

## Run it

```
cd /home/toeesh/Documents/writee
cargo run --release -p writee-desktop
```

Workspace is at `~/.local/share/writee/Writee/` (the user's `~/Documents/`
doesn't exist on this machine, so `directories-next` falls through to
`data_dir`). The marker file at `~/.config/writee/workspace` records that
path.

First launch creates `_welcome.writee` from a seeded template. Settings
persist to `<workspace>/.writee-settings.toml`.

---

## Workspace layout

```
writee/
  Cargo.toml                         # workspace root
  CLAUDE.md                          # this file
  README.md                          # user-facing docs
  crates/
    writee-core/                     # data model + math + storage
      src/
        document.rs                  # Document, Object enum (Stroke/Arrow/TextBox/Shape/SubNote/Link/Image), ImageBlock
        stroke.rs                    # InkPoint, Stroke (with color_rgba override)
        textbox.rs                   # TextBox + cursor + nav helpers
        arrow.rs / shape.rs /
        subnote.rs / link.rs         # other object kinds (SubNote = inline | linked | index; IndexEntry)
        tessellate.rs                # InkVertex + tessellate(...), tessellate_arrow, etc.
        one_euro.rs                  # 1€ filter for pointer smoothing
        storage.rs                   # SQLite (rusqlite); KIND_* incl. KIND_IMAGE=6; meta table
        geom.rs                      # Aabb
        theme.rs                     # ColorTheme + ThemeName (LIGHT/DARK presets)
    writee-render/                   # wgpu pipelines
      src/
        lib.rs                       # Renderer (clear-colour from theme)
        grid.rs                      # procedural dot-grid background shader
        ink.rs                       # ink pipeline (SDF AA, per-vertex color)
        image.rs                     # textured-quad pipeline for ImageBlock
        text.rs                      # glyphon TextLayer
        egui_pass.rs                 # egui-wgpu chrome pass
        viewport.rs                  # Viewport uniform — 64B (offset/zoom/screen + bg/dot colour)
        shaders/                     # WGSL — grid.wgsl, ink.wgsl, image.wgsl
    writee-input/                    # InkSample abstraction
      src/
        winit_adapter.rs             # winit events → InkSample (cfg gates per OS)
        linux_tablet.rs              # evdev pressure / tilt / BTN_TOOL_RUBBER side channel
        macos_tablet.rs              # NSEvent stub — pressure=1.0 placeholder (Phase 6b TODO)
    writee-app/                      # winit ApplicationHandler, tools, undo, UI
      src/
        lib.rs                       # App + all tool handlers + main run() / run_android()
        tool.rs                      # Tool enum (Pen/Highlighter/Eraser/Arrow/Shape/Text/Note/Index/Link/Select)
        settings.rs                  # ToolSettings, FontMappings, WorkspaceConfig (TOML, incl. theme)
        ui.rs                        # egui chrome (sidebar, top bar, bottom toolbar, modals)
        geometry.rs                  # CommittedCache — tessellates committed doc once, theme-aware
        undo.rs                      # Op enum + UndoStack
        workspace.rs                 # discovery, welcome seeding, WorkspaceLock, trash, link rewrite, build_tree
        markdown.rs                  # page mode: stacked block editor (same blocks as edgeless)
        md_shortcuts.rs              # `# `/`- `/`[x] ` etc. line-level render transforms
        search.rs                    # SearchCache + workspace-wide substring search
        palette.rs                   # Ctrl-K command palette state + row collection + fuzzy match
        tags.rs                      # `#tag` extraction + `all_tags(root)`
        recovery.rs                  # RecoverySnapshot autosave (.writee-recovery.json)
        icon.rs                      # procedural 64×64 window icon (stylized "w")
    writee-export-web/               # exports
      src/
        lib.rs                       # static HTML/JS web bundle (Ctrl+E)
        markdown.rs                  # per-file `.md` exporter (reading-order block walk)
  viewer-template/                   # embedded HTML/JS for web export
  apps/
    desktop/                         # bin: writee-desktop
      writee.desktop                 # Linux launcher integration
      scripts/build-appimage.sh      # one-shot AppImage build
    android/                         # cdylib: writee-android (paused, untested on device)
```

---

## What works

- **Pen / Highlighter / Eraser** with real stylus pressure (via evdev on Linux).
- **Pressure toggle** in toolbar; toggling re-tessellates the committed cache.
- **Tilt-aware ink width** — `tilt_modulation` toggle multiplies width by
  `1 + 0.6 * tilt_magnitude` per sample. evdev side-channel reads
  `ABS_TILT_X` / `ABS_TILT_Y` alongside pressure and publishes them through
  atomics into every `InkPoint`.
- **Stylus eraser end** — `BTN_TOOL_RUBBER` from the evdev reader flips
  `PenState.eraser`; while held, `App::on_sample` routes to `handle_eraser`
  regardless of the toolbar's current tool. Tool selection is unchanged, so
  flipping back to the pen tip resumes the previous tool.
- **Stroke color picker** drives Pen ink (via `Stroke.color_rgba`); Highlighter
  is locked to the dedicated yellow.
- **Arrow** tool (parametric body + head, AA shaft).
- **Shape** tool with Rect / Ellipse / Line submodes + Fill toggle.
- **Text** boxes (glyphon-rendered), with per-textbox `font_name`.
- **Text caret + nav** — `TextBox.cursor` is a byte offset (`#[serde(skip)]`,
  so it doesn't pollute saved files). Insert/backspace happen *at* the caret,
  not at end. Arrow keys, Home, End, and click-to-position-cursor all work;
  Up/Down preserve column. Caret is rendered as a thin tessellated segment
  in `build_ink_geometry`. Position uses the same rough `0.55em * font_size`
  / `1.25em * font_size` grid as `TextBox::bbox` — drifts slightly from true
  glyphon layout for proportional fonts, which is acceptable for v1.
- **Notes (the new model)** — every note placed on a canvas is a `SubNote`
  card with a title bar + body, in one of three flavours:
    1. **Inline (sticky)** — default for Note (N) tool. Body holds editable
       markdown text right on the parent canvas. Click to type, arrow keys
       to navigate, caret rendered in `build_ink_geometry`.
    2. **Linked** — body is a pointer to a separate `.writee` file. Click
       opens it in its own editor (canvas or markdown per the child's meta).
    3. **Index** — body renders a curated list of *entries* (files +
       section headings). Click a file row to jump to it; heading rows
       group files visually. The welcome canvas has one locked + index
       card acting as the workspace home; users can drop their own via the
       **Index tool** (I) or right-click → Mark as index on a sticky.
       The "Edit index contents…" modal supports per-row reorder (↑/↓),
       remove, "+ file" picker, "+ heading" text-edit, "Add all files",
       and "Clear". Storage: `SubNote.index_entries: Option<Vec<IndexEntry>>`
       is authoritative; the legacy `index_files: Option<Vec<String>>`
       field is still read for back-compat and migrated to entries on first
       edit. Empty entry list → falls back to "show every file"
       (welcome behaviour).
  Notes are converted between flavours via the right-click context menu
  (Edit text / Open in editor / Convert to linked / Convert to inline /
  Mark as index / Edit index contents… / Switch linked file to
  canvas-or-markdown / Delete). Locked notes refuse drag + delete.
  See `crates/writee-core/src/subnote.rs` for the model, `handle_subnote_click`
  + `convert_note_inline_to_linked` / `convert_note_linked_to_inline` /
  `toggle_note_index` / `set_linked_note_mode` / `open_index_editor` /
  `commit_index_editor` for the wiring.
- **Live sub-note thumbnails** — child `.writee` files are loaded into an
  in-memory `HashMap<PathBuf, ThumbnailEntry>` keyed by absolute path,
  refreshed when mtime advances. Each visible card embeds the child's
  geometry (strokes/arrows/shapes) and text (glyphon `TextInstance`s)
  transformed into the card body via `thumbnail_transform`. Depth-1 only —
  nested children render as plain cards inside the parent's thumbnail.
- **Links** — any-to-any smart connectors. Anchor dots are visible while the
  Link tool is active. Endpoints track object movement.
- **Select** — single + multi-select (Shift/Ctrl click), drag-to-move,
  Delete/Backspace to remove, marquee from empty space.
- **Undo / redo** (Ctrl+Z / Ctrl+Shift+Z) with op log; covers add, remove,
  replace.
- **Ctrl+A / Ctrl+C / Ctrl+V** — select all, copy, paste with offset.
- **Pan / zoom**: middle-mouse drag, wheel, Ctrl++ / Ctrl+- / Ctrl+0.
- **Fit to content** (Ctrl+F or toolbar).
- **Workspace tree sidebar** — persistent `SidePanel::left`. Files are
  rendered hierarchically: any file whose SubNotes link to another file is
  shown as that file's parent, indented. Files with no parent show at root.
  Cycles in the link graph are guarded by a `visited` set in
  `render_tree_node`. Right-click a row for Rename / Delete.
- **Edgeless bottom toolbar** — Affine-style floating pill anchored to
  `Align2::CENTER_BOTTOM`. Holds tool buttons + compact style controls
  (width slider, Highlight toggle, ink-colour swatch, pen-pressure
  readout). Only shown in edgeless mode; page mode hides it. Top bar is a
  slim header (file name + Undo/Redo/Fit/Export/Page-toggle/Settings).
  Tool buttons use short text labels (Pen / Mark / Erase / Arrow / Shape
  / Text / Note / Index / Link / Select) — egui's bundled font doesn't
  carry the unicode glyphs I originally tried, so we stick to ASCII.
- **Sticky-note styled cards** — `geometry.rs` paints a soft drop-shadow
  rectangle behind each SubNote and tints the fill by kind: cream for
  inline stickies, lavender for index cards, white for linked. Locked
  cards get a slightly darker border.
- **Reading-order badge** — each SubNote's title is prefixed with its
  Y-then-X reading order (Affine "number under each note in edgeless").
- **Welcome canvas** — seeded with a short intro callout + a locked,
  index-flavour SubNote (workspace home). The Note tool works on the
  welcome canvas like anywhere else; only the welcome's index card itself
  is `locked` (refuses drag/delete/conversion). The legacy file-level
  `locked_notes` flag is still read for back-compat but no longer used.
- **Settings persistence** — TOML in workspace. Width / eraser radius /
  text size / pressure toggle / tilt toggle / colors / shape state / font
  slot / theme name all persist. Edit via the **Settings** button (top-right).
- **Font slots** — abstract slots (Default / Mono / Serif / Slab / Thematic);
  Settings dialog maps them to font family names. Glyphon picks them up
  by `Family::Name`.
- **Page mode (Affine-style two views of one doc)** — toolbar toggle flips
  `meta.mode` between `canvas` (edgeless) and `markdown` (page). Page mode
  renders the *same* TextBox + inline-SubNote blocks as the canvas, stacked
  top-to-bottom by Y position in a centered ScrollArea. Each block is an
  editable `TextEdit::multiline`; edits mutate the underlying object and
  persist through `DocStore::update`. "+ new text block" appends a TextBox
  below the lowest existing block. Switching back to edgeless preserves
  positions (we only ever touch the `content` field). The legacy
  `meta.markdown_source` key is read for back-compat but no longer the
  source of truth. See `crates/writee-app/src/markdown.rs`.
- **Tool cursors** — system cursor changes per active tool.
- **Web export** (Ctrl+E) — emits `<file>-export/` with self-contained
  viewer; includes shape / arrow / text / sub-note / link rendering.
- **Markdown export** (command palette → "Export current file as Markdown")
  — serializes TextBoxes + inline SubNote bodies in reading order into a
  single `.md` via `rfd` save dialog. See
  `crates/writee-export-web/src/markdown.rs`. PDF + PNG exports are stubbed
  with a console hint pointing at web export for now.
- **Image blocks** — paste from clipboard (Ctrl-V) or drag-drop a PNG/JPEG
  onto the canvas. Each image is an `Object::Image(ImageBlock { origin,
  size, bytes, natural_w, natural_h })` storing the encoded bytes inline.
  Render path: new `crates/writee-render/src/image.rs` pipeline + WGSL,
  texture cache keyed by FNV-1a hash of the bytes, draws below ink so the
  user can annotate over images.
- **Full-text search + command palette** — Ctrl-K opens a centered
  palette with three sections: actions, files (fuzzy match on name),
  content (substring match in `TextBox.content` / `SubNote.inline_content`
  / `SubNote.title`). Backed by `SearchCache` keyed on `(path, mtime)`.
  See `crates/writee-app/src/{search.rs, palette.rs}`.
- **Backlinks panel** — sidebar section listing every file whose SubNotes
  link to the currently-open file. `Workspace::backlinks(file)`.
- **Tags** — `#tag` (with `#proj/sub` hierarchy) inside any text content
  is parsed by `crates/writee-app/src/tags.rs`. Sidebar "Tags" section
  shows the global tag list with counts; clicking a tag filters the file
  tree to files containing it.
- **Markdown shortcuts in inline notes** — render-time pass in
  `crates/writee-app/src/md_shortcuts.rs` recognises `# `/`## `/`### ` for
  heading sizes, `- ` / `* ` for bullet (`•`), `[x] ` / `[ ] ` for
  checkboxes (`☑` / `☐`). Stored raw, rendered styled.
- **Themes** — `writee_core::ColorTheme` with `LIGHT` / `DARK` presets
  threaded through the renderer, geometry cache, egui chrome, and the
  grid shader (via the viewport uniform). Toggle from the command palette
  or the Settings dialog. Persists to `WorkspaceConfig.theme`.
- **Soft-delete + Trash** — sidebar shows a `Trash (N)` collapsible.
  Deleting a file moves it to `<workspace>/.trash/` with a `<unixts>_<stem>`
  prefix; right-click in trash → Restore or Delete forever. WAL/SHM
  sidecars move alongside on both trash and restore.
- **Workspace lock** — `.writee.lock` (held via `fs2::FileExt`) prevents
  two writee windows from racing SQLite writes on the same workspace.
  Second window prints a friendly error and exits.
- **Rename rewrites links** — `Workspace::rewrite_links` walks every
  `.writee` and updates `SubNote.target_file` + `index_entries` references
  when a file is renamed, so backlinks stay intact.
- **Crash recovery** — `RecoverySnapshot` (wet stroke + in-progress text /
  inline-note edits) autosaves to `<workspace>/.writee-recovery.json`
  every ~2s. On next launch, if a recent snapshot exists for the current
  file, a modal offers "Restore" / "Discard". Cleared on graceful exit.
- **Multi-select duplicate** — Ctrl-D clones the selection with a small
  offset and selects the copies.
- **Window icon + .desktop file + AppImage build script** — procedural
  window icon (a stylized "w") set via `winit::window::Icon::from_rgba`
  in `crates/writee-app/src/icon.rs`. Linux desktop integration:
  `apps/desktop/writee.desktop`. One-shot installer:
  `apps/desktop/scripts/build-appimage.sh`.

---

## What's stubbed or known-bad

| Item | Status |
| --- | --- |
| **PDF + PNG export** | Palette entries exist; handler logs "use web export". PDF needs `printpdf` integration; PNG needs offscreen wgpu render-to-texture + readback. |
| **macOS pen pressure** | `crates/writee-input/src/macos_tablet.rs` is a stub returning pressure=1.0. Real impl = global NSEvent monitor via `cocoa` + `objc`, gated `#[cfg(target_os = "macos")]`. Needs a macOS dev box to verify the block ABI. |
| **True inline rich text** | `**bold**` / `*italic*` inside inline notes still renders as raw markup. Heading lines / bullets / checkboxes work (line-level transforms via `md_shortcuts`). Per-character styling needs styled glyphon runs. |
| **Thumbnail texture cache** | Thumbnails currently re-embed child geometry into the parent's vertex stream every frame. Cheap per child (and the loaded `Document`s are cached by mtime), but profile if a parent has many cards. Render-to-texture would be cleaner. |
| **Text caret precision** | Caret position uses the same rough `0.55em` char-grid as `TextBox::bbox`. Drifts from true glyphon layout for proportional fonts. Real fix needs `Buffer::layout_cursor` queries. |
| **Incremental cache invalidation** | `CommittedCache` rebuilds the whole document on any mutation. Fine to ~thousands of objects, then will hitch. |
| **Async SQLite writes** | All writes are on the main thread. |
| **Viewport culling** | All objects always tessellated regardless of on-screen state. |
| **Android port** | Paused. Scaffold builds on host as empty stub; never tested on a device. |
| **Drag-drop image on Wayland** | Relies on the compositor delivering `WindowEvent::DroppedFile`. Works on X11 + some Wayland setups; on others the portal handover isn't wired. Paste-from-clipboard (Ctrl-V) works on both. |
| **Auto-update / code signing / crash reporting** | Need accounts + hosting — deferred operational workstream. |
| **Resize/rotate selection handles** | Not done. |

---

## Critical knowledge (don't relearn the hard way)

### Pressure on Linux
winit on Linux does **not** expose stylus pressure through normal pointer
events. We read it via a side channel: `writee-input/src/linux_tablet.rs`
spawns one evdev reader thread per device that exposes `ABS_PRESSURE`,
publishes the latest value to an `Arc<AtomicU32>`, and `WinitInput` samples
that atomic on every `CursorMoved`/`MouseInput`. Works for:
- Real tablets the user can read via `/dev/input/event*` (needs `input` group).
- OpenTabletDriver in **tablet output** mode (it creates a uinput virtual
  device with ABS_PRESSURE).

Does **not** work for OTD in mouse-emulation mode. User runs `otd-daemon`
and confirmed pressure works after switching OTD's output. The toolbar's
green/gray pressure readout is the diagnostic.

### egui consumed vs repaint
`egui_winit::State::on_window_event` returns `EventResponse { consumed,
repaint }`. **You must request_redraw on `repaint == true`** or button
clicks land in egui's internal state and never become `UiActions` because
no redraw runs to drain them. We learned this one the hard way.

### egui claims Ctrl+= / Ctrl+- by default
Disabled via `ctx.options_mut(|o| o.zoom_with_keyboard = false)` in
`EguiChrome::new`. Otherwise it'd zoom the UI. Canvas zoom shortcuts are
handled in `App::handle_key`.

### wgpu / glyphon / egui version lock
Pinned to wgpu 22 + glyphon 0.6 + egui 0.29. Newer egui versions need
newer wgpu, but glyphon 0.6 is the most recent that ships for wgpu 22.
Bumping any of these is a coordinated upgrade. `clamp_to_range(true)`
deprecation warnings are unavoidable until that upgrade.

### wgpu 22 RenderPass lifetimes
`wgpu::RenderPass::forget_lifetime()` consumes the pass by value. We can't
share one pass between the canvas draw and egui — that's why
`EguiPass::prepare_and_render` opens its own pass with `LoadOp::Load`
*after* the canvas pass closes. Don't try to merge them again; the
lifetime gymnastics are not worth it.

### Stroke color compatibility
Old `Stroke` had `color: u8` only (0 = ink, 1 = highlight). New `Stroke`
also has `color_rgba: Option<[u8;4]>` with `#[serde(default)]`. New pens
write to `color_rgba`; renderer prefers it. Don't remove the old `color`
field — files saved before the rgba field exist in the wild.

### SubNote click semantics
With Select tool active: single click + release (no drag) routes to
`handle_subnote_click(id, world)`:
- inline → start in-place body edit
- linked → open the linked file
- index → switch to the file the user clicked inside the body picker

`click_target_subnote` is set at gesture start; we only act on it if no
object moved between Begin and End. Right-click on a note opens a context
menu (handled in `WindowEvent::MouseInput` → `pick_subnote_at_world` →
`App.context_menu`); right-click on empty space does nothing (middle-mouse
pans, not right).

### Welcome = index
The welcome file `_welcome.writee` is both the onboarding canvas *and* the
workspace picker host. `is_on_index_file()` actually checks
`welcome_file_path()`. The legacy `_index.writee` path exists in code but
is unused. Clean it up later if you want.

### Workspace path quirk
On this machine, `~/Documents` doesn't exist, so the workspace fell back
to `~/.local/share/writee/Writee/` (the `data_dir` from
`directories-next`). The path is recorded in
`~/.config/writee/workspace` — edit that file (plain text) to point
elsewhere.

### egui font coverage
The bundled font only ships basic Latin + a tiny symbol set. Anything
fancier (✎ ⌫ ⛓ 🖍 🗑 etc.) renders as empty boxes. Use ASCII / short
text labels everywhere egui draws, including buttons + section headers.
Glyphon text (canvas text, card titles, badges, body text) uses system
fonts via cosmic-text and can render the broader set fine — that's why
`●` / `★` work inside SubNote cards but not in the egui sidebar.

### Don't set `Visuals::override_text_color`
It forces every widget (including selected `SelectableLabel`s) to a
single foreground colour, so selected items show dark-on-dark and become
invisible. Set per-state `widgets.{inactive,hovered,active,open}.fg_stroke`
and `selection.stroke` instead. `visuals_from_theme` does this correctly;
do not "simplify" it back to an override.

### Viewport uniform is 64 bytes now, not 32
`to_uniform(bg_rgba, dot_rgba) -> [f32; 16]`. The two `vec4` colour slots
feed the grid shader's bg/dot via the shared viewport uniform. Both the
grid and ink WGSL `Viewport` structs declare these fields so layouts
match. Adding a new pipeline that binds the viewport must also declare
both colour slots even if it ignores them, or the uniform layout will
mismatch at validation.

### Workspace lockfile is a single OS-level flock
`.writee.lock` is held via `fs2::FileExt::try_lock_exclusive`. fs2 wraps
`flock(2)` on Linux + `LockFileEx` on Windows, which the kernel releases
on process exit (even on `kill -9`) — so a stale lockfile after a crash
is harmless. We do remove it in `Drop` for tidiness, but never rely on
that for correctness.

### Recovery autosave fingerprint
`maybe_write_recovery` only writes when there's actually transient state
(wet stroke ≥ 2 points OR an in-progress text/note edit with non-empty
content) AND the content fingerprint differs from the previous snapshot.
Without the fingerprint, the throttle alone would still let us thrash
disk every 2s during idle text-edit sessions. Don't re-introduce that.

### Switching files clears editor + recovery state
`switch_to_file` resets `editing_text`, `editing_note`,
`edit_text_before`, `context_menu`, `index_editor`, `last_recovery_*`,
and removes the `.writee-recovery.json` for the previous file. Without
this, switching mid-edit would orphan UI state and prompt to "restore"
content from the wrong file on next launch.

---

## User context (saved to memory)

- User runs **OpenTabletDriver** with `otd-daemon`. Pressure requires the
  tablet output plugin, not mouse-emulation.
- User is strict about **minimal black-and-white UI**. Avoid cluttered
  toolbar, avoid color on chrome (orange/yellow stay as in-canvas accents).
- The **welcome / index page concept** is intentional: a real whiteboard
  the user can annotate around the picker. Don't replace it with a modal.
- Sub-notes should feel like **hierarchical .writee files**, not embedded
  cards. The card is just the link representation; the data lives in a
  child file.

These are saved at `~/.claude/projects/-home-toeesh-Documents-writee/memory/`.

---

## Recommended next steps (in order)

1. **PDF + PNG export** — palette already exposes them; today they log a
   hint. PDF: `printpdf` crate, walk objects, render strokes as polylines,
   images as XObjects, text as runs. PNG: extend `Renderer` with a
   `render_to_image(w, h) -> Vec<u8>` that targets an offscreen
   `Rgba8UnormSrgb` texture and reads back via a mapped buffer.
2. **macOS pen pressure** — fill in `macos_tablet::spawn_pressure_reader`.
   Mirrors the Linux pattern. Needs a Mac to verify the Objective-C block
   ABI and NSEvent introspection.
3. **Per-character inline styling** (bold / italic). Needs styled glyphon
   runs (`Attrs` per range). Probably split the inline-note renderer into
   "produce styled spans" + "emit per-span TextInstance".
4. **Caret precision via glyphon** — `Buffer::layout_cursor` instead of
   the rough char-grid.
5. **Thumbnail render-to-texture** — current implementation embeds child
   geometry every frame. Move to an offscreen texture per child, refreshed
   on mtime change.
6. **Incremental cache invalidation** — `CommittedCache` currently rebuilds
   the whole document on any mutation.
7. **Resize/rotate selection handles** — straightforward UX win.
8. **Differentiator (TBD)** — user said they'll specify. Candidates:
   handwriting-OCR, LAN sync, single-surface WYSIWYG.

Lower priority: async SQLite, viewport culling, Android resume, app icon.

---

## Common commands

```
cargo build --workspace            # build everything
cargo test --workspace             # run all tests (8 in writee-core)
cargo run --release -p writee-desktop
RUST_LOG=writee_input=info cargo run --release -p writee-desktop   # verify evdev pressure detection
```

To reset the workspace from scratch:

```
rm -f ~/.local/share/writee/Writee/*.writee* ~/.local/share/writee/Writee/.writee-settings.toml
rm -rf ~/.local/share/writee/Writee/*-export
```

The marker file at `~/.config/writee/workspace` is preserved across resets;
delete it too if you want to re-pick the workspace location.
