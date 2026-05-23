//! egui chrome.
//!
//! Two-row top panel: tool selection on top, style + file ops on bottom.
//! A separate Settings window can be toggled to configure font slot mappings.

use std::path::PathBuf;

use egui::{Align, Color32, Frame, Layout, Stroke, Vec2 as EVec2};
use winit::event::WindowEvent;
use winit::window::Window;

use writee_render::EguiPass;

use crate::settings::{ActiveShape, FontMappings, FontSlot, InkColor, ToolSettings};
use crate::tool::Tool;

pub struct EguiChrome {
    pub ctx: egui::Context,
    pub state: egui_winit::State,
    pub pass: EguiPass,
    pub settings_open: bool,
}

impl EguiChrome {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        window: &Window,
    ) -> Self {
        let ctx = egui::Context::default();
        // Set a placeholder; App refreshes on every frame with the active
        // theme via `EguiChrome::apply_theme`.
        ctx.set_visuals(visuals_from_theme(&writee_core::ColorTheme::LIGHT));
        ctx.options_mut(|opts| {
            opts.zoom_with_keyboard = false;
        });
        let state = egui_winit::State::new(
            ctx.clone(),
            ctx.viewport_id(),
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let pass = EguiPass::new(device, format);
        Self { ctx, state, pass, settings_open: false }
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    pub fn apply_theme(&self, theme: &writee_core::ColorTheme) {
        self.ctx.set_visuals(visuals_from_theme(theme));
    }
}

pub struct UiInput<'a> {
    pub current_tool: Option<Tool>,
    pub settings: &'a mut ToolSettings,
    pub fonts: &'a mut FontMappings,
    pub can_undo: bool,
    pub can_redo: bool,
    pub current_file: Option<&'a str>,
    pub object_count: usize,
    pub last_pressure: f32,
    pub settings_open: &'a mut bool,
    pub on_welcome: bool,
    /// True when the current file is in markdown mode (drives the toolbar
    /// button label and hides canvas-style controls).
    pub is_markdown: bool,
}

#[derive(Debug, Default)]
pub struct UiActions {
    pub set_tool: Option<Tool>,
    pub undo: bool,
    pub redo: bool,
    pub new_file: bool,
    pub cycle_file: bool,
    pub export_web: bool,
    pub fit_to_content: bool,
    pub delete_selected: bool,
    pub clear_selection: bool,
    pub open_file: Option<PathBuf>,
    pub rename_file: Option<(PathBuf, String)>,
    pub delete_file: Option<PathBuf>,
    pub restore_file: Option<PathBuf>,
    pub purge_file: Option<PathBuf>,
    /// Toggle the current file's document mode (canvas ↔ markdown).
    pub toggle_markdown: bool,

    // -- Note context-menu actions (set by build_note_context_menu). --
    pub note_open: Option<u64>,
    pub note_edit_inline: Option<u64>,
    pub note_convert_to_linked: Option<u64>,
    pub note_convert_to_inline: Option<u64>,
    pub note_toggle_index: Option<u64>,
    pub note_edit_index_contents: Option<u64>,
    pub note_set_linked_mode: Option<(u64, bool)>, // (object_id, want_markdown)
    pub note_delete: Option<u64>,
    pub note_close_menu: bool,

    // -- Index-editor modal actions. --
    pub index_editor_commit: bool,
    pub index_editor_cancel: bool,

    // -- Recovery prompt --
    pub recovery_accept: bool,
    pub recovery_discard: bool,

    /// User clicked a tag in the sidebar. `None` = clear filter.
    pub set_tag_filter: Option<Option<String>>,

    // -- Command palette --
    /// Index of the palette row the user picked this frame.
    pub palette_pick: Option<usize>,
    pub palette_close: bool,
    pub palette_focus_next: bool,
    pub palette_focus_prev: bool,
    /// Set by build_command_palette when it has actually grabbed focus, so
    /// the App stops asking on subsequent frames.
    pub palette_focused: bool,
}

/// Per-note descriptor the App hands to `build_note_context_menu` so the menu
/// can show the right verbs (e.g. hide "Edit text" on a linked card).
#[derive(Debug, Clone)]
pub struct NoteMenuInfo {
    pub object_id: u64,
    pub screen_pos: egui::Pos2,
    pub is_inline: bool,
    pub is_linked: bool,
    pub is_index: bool,
    pub locked: bool,
    pub title: String,
    /// For linked cards: true if the child file is in markdown mode. Drives
    /// the "Switch linked file to canvas/markdown" toggle label.
    pub linked_is_markdown: bool,
}

/// Entry the user sees / edits inside the index modal. Mirrors the core
/// `IndexEntry` but lives in the UI crate to keep the dep direction clean.
#[derive(Debug, Clone)]
pub enum IndexEditorEntry {
    File { file: String },
    Heading { text: String },
}

/// Per-row edit action surfaced from the modal back to the App.
#[derive(Debug, Default)]
pub struct IndexEditorActions {
    /// Move row i one step up (towards 0). At most one per frame.
    pub move_up: Option<usize>,
    pub move_down: Option<usize>,
    pub remove: Option<usize>,
    /// New file to append to the entry list (workspace-relative filename).
    pub add_file: Option<String>,
    /// New heading to append.
    pub add_heading: Option<String>,
    /// Bulk: append every workspace file that isn't already in the list.
    pub add_all_missing: bool,
    /// Bulk: clear every row.
    pub clear_all: bool,
}

/// "Edit index contents" modal — Affine-style row-by-row curation. Mutates
/// nothing directly; surfaces user intent through `IndexEditorActions` +
/// the existing `UiActions.index_editor_commit/cancel` flags.
pub fn build_index_editor(
    ctx: &egui::Context,
    title: &str,
    entries: &[IndexEditorEntry],
    available_files: &[String],
    selected_file: &mut Option<String>,
    new_heading_text: &mut String,
    actions: &mut UiActions,
    row_actions: &mut IndexEditorActions,
) {
    egui::Window::new(format!("Edit index — {title}"))
        .resizable(true)
        .collapsible(false)
        .default_pos([120.0, 120.0])
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "Curate what this index shows. Reorder with ↑/↓; add files and section headings below.",
                )
                .small()
                .weak(),
            );
            ui.separator();

            // -- Current entries with per-row controls. ------------------
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    if entries.is_empty() {
                        ui.label(
                            egui::RichText::new("(no rows yet — add files or headings below)")
                                .italics()
                                .weak(),
                        );
                    }
                    let len = entries.len();
                    for (i, entry) in entries.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(i > 0, |ui| {
                                if ui.small_button("↑").on_hover_text("Move up").clicked() {
                                    row_actions.move_up = Some(i);
                                }
                            });
                            ui.add_enabled_ui(i + 1 < len, |ui| {
                                if ui.small_button("↓").on_hover_text("Move down").clicked() {
                                    row_actions.move_down = Some(i);
                                }
                            });
                            match entry {
                                IndexEditorEntry::File { file } => {
                                    ui.label(egui::RichText::new("file").weak().small());
                                    ui.label(file);
                                }
                                IndexEditorEntry::Heading { text } => {
                                    ui.label(egui::RichText::new("heading").weak().small());
                                    ui.label(egui::RichText::new(text).strong());
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("✕")
                                        .on_hover_text("Remove this row")
                                        .clicked()
                                    {
                                        row_actions.remove = Some(i);
                                    }
                                },
                            );
                        });
                    }
                });

            ui.separator();

            // -- Add controls. -------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Add file:");
                let selected_label = selected_file
                    .as_deref()
                    .unwrap_or("(none)")
                    .to_string();
                egui::ComboBox::from_id_salt("index-add-file")
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        for f in available_files {
                            let is_sel = selected_file.as_deref() == Some(f.as_str());
                            if ui.selectable_label(is_sel, f).clicked() {
                                *selected_file = Some(f.clone());
                            }
                        }
                    });
                if ui.button("+ file").clicked() {
                    if let Some(f) = selected_file.clone() {
                        row_actions.add_file = Some(f);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Add heading:");
                ui.text_edit_singleline(new_heading_text);
                if ui.button("+ heading").clicked() && !new_heading_text.trim().is_empty() {
                    row_actions.add_heading = Some(new_heading_text.trim().to_string());
                    new_heading_text.clear();
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button("Add all files")
                    .on_hover_text("Append every workspace file not already in the list")
                    .clicked()
                {
                    row_actions.add_all_missing = true;
                }
                if ui
                    .button("Clear")
                    .on_hover_text("Remove every row from this index")
                    .clicked()
                {
                    row_actions.clear_all = true;
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    actions.index_editor_commit = true;
                }
                if ui.button("Cancel").clicked() {
                    actions.index_editor_cancel = true;
                }
            });
        });
}

pub fn build_note_context_menu(
    ctx: &egui::Context,
    info: &NoteMenuInfo,
    actions: &mut UiActions,
) {
    let area = egui::Area::new(egui::Id::new(("note-ctx", info.object_id)))
        .fixed_pos(info.screen_pos)
        .order(egui::Order::Foreground);
    area.show(ctx, |ui| {
        let style = ctx.style();
        egui::Frame::popup(&style)
            .fill(style.visuals.panel_fill)
            .stroke(style.visuals.window_stroke)
            .show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(egui::RichText::new(&info.title).strong());
                ui.separator();

                if info.is_inline {
                    if ui.button("Edit text").clicked() {
                        actions.note_edit_inline = Some(info.object_id);
                        actions.note_close_menu = true;
                    }
                    if ui.button("Convert to linked file…").clicked() {
                        actions.note_convert_to_linked = Some(info.object_id);
                        actions.note_close_menu = true;
                    }
                }
                if info.is_linked {
                    if ui.button("Open in editor").clicked() {
                        actions.note_open = Some(info.object_id);
                        actions.note_close_menu = true;
                    }
                    if ui.button("Inline this file's content").clicked() {
                        actions.note_convert_to_inline = Some(info.object_id);
                        actions.note_close_menu = true;
                    }
                    ui.separator();
                    if ui
                        .button(if info.linked_is_markdown {
                            "Switch linked file to canvas"
                        } else {
                            "Switch linked file to markdown"
                        })
                        .clicked()
                    {
                        actions.note_set_linked_mode =
                            Some((info.object_id, !info.linked_is_markdown));
                        actions.note_close_menu = true;
                    }
                }
                let idx_label = if info.is_index { "Unmark as index" } else { "Mark as index" };
                if ui.button(idx_label).clicked() {
                    actions.note_toggle_index = Some(info.object_id);
                    actions.note_close_menu = true;
                }
                if info.is_index && !info.locked {
                    if ui.button("Edit index contents…").clicked() {
                        actions.note_edit_index_contents = Some(info.object_id);
                        actions.note_close_menu = true;
                    }
                }
                ui.separator();
                ui.add_enabled_ui(!info.locked, |ui| {
                    if ui.button("Delete").clicked() {
                        actions.note_delete = Some(info.object_id);
                        actions.note_close_menu = true;
                    }
                });
                if info.locked {
                    ui.label(egui::RichText::new("(locked — can't be moved or deleted)").weak());
                }
                ui.separator();
                if ui.button("Close").clicked() {
                    actions.note_close_menu = true;
                }
            });
    });
}

pub fn build_ui(ctx: &egui::Context, input: UiInput<'_>) -> UiActions {
    let mut actions = UiActions::default();

    // -- TOP BAR: slim header — file name on the left, file/mode ops on the right.
    let top_frame = Frame::none()
        .inner_margin(EVec2::new(10.0, 6.0))
        .fill(ctx.style().visuals.panel_fill)
        .stroke(Stroke::new(1.0, ctx.style().visuals.window_stroke.color));
    egui::TopBottomPanel::top("topbar")
        .frame(top_frame)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new("writee").strong().size(15.0));
                ui.separator();
                if let Some(file) = input.current_file {
                    let tag = if input.on_welcome {
                        format!("{file}  •  welcome")
                    } else {
                        file.to_string()
                    };
                    ui.label(egui::RichText::new(tag).weak());
                }
                ui.label(
                    egui::RichText::new(format!("· {} obj", input.object_count))
                        .weak()
                        .small(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Settings").on_hover_text("Preferences").clicked() {
                        *input.settings_open = !*input.settings_open;
                    }
                    let md_label = if input.is_markdown { "Edgeless" } else { "Page" };
                    if ui
                        .button(md_label)
                        .on_hover_text("Toggle this file between edgeless (canvas) and page (markdown) view")
                        .clicked()
                    {
                        actions.toggle_markdown = true;
                    }
                    if ui.button("Export").on_hover_text("Export to web folder (Ctrl+E)").clicked() {
                        actions.export_web = true;
                    }
                    if ui.button("Fit").on_hover_text("Frame document (Ctrl+F)").clicked() {
                        actions.fit_to_content = true;
                    }
                    ui.add_enabled_ui(input.can_undo, |ui| {
                        if ui.button("Undo").on_hover_text("Ctrl+Z").clicked() {
                            actions.undo = true;
                        }
                    });
                    ui.add_enabled_ui(input.can_redo, |ui| {
                        if ui.button("Redo").on_hover_text("Ctrl+Shift+Z").clicked() {
                            actions.redo = true;
                        }
                    });
                });
            });
        });

    // -- BOTTOM CENTRE: floating pill with the tool buttons + style controls.
    // Skip the floating toolbar in page mode — there's no canvas to tool on.
    if !input.is_markdown {
        let screen_rect = ctx.screen_rect();
        let anchor = egui::pos2(screen_rect.center().x, screen_rect.max.y - 16.0);
        egui::Area::new(egui::Id::new("edgeless-toolbar"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, 0.0])
            .fixed_pos(anchor)
            .interactable(true)
            .show(ctx, |ui| {
                // Pill background derives from the active theme so dark mode
                // doesn't show a bright white slab over the canvas.
                let style = ctx.style();
                egui::Frame::popup(&style)
                    .fill(style.visuals.panel_fill)
                    .stroke(style.visuals.window_stroke)
                    .rounding(egui::Rounding::same(12.0))
                    .inner_margin(egui::Vec2::new(10.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for tool in Tool::drawing_tools() {
                                tool_button(ui, *tool, &mut actions, input.current_tool);
                            }
                            ui.separator();
                            for tool in Tool::annotation_tools() {
                                tool_button(ui, *tool, &mut actions, input.current_tool);
                                if *tool == Tool::Shape && input.current_tool == Some(Tool::Shape) {
                                    ui.menu_button(input.settings.active_shape.label(), |sub| {
                                        for s in [
                                            ActiveShape::Rectangle,
                                            ActiveShape::Ellipse,
                                            ActiveShape::Line,
                                        ] {
                                            if sub
                                                .selectable_label(
                                                    input.settings.active_shape == s,
                                                    s.label(),
                                                )
                                                .clicked()
                                            {
                                                input.settings.active_shape = s;
                                                sub.close_menu();
                                            }
                                        }
                                    });
                                    ui.checkbox(&mut input.settings.shape_filled, "Fill");
                                }
                            }
                            ui.separator();
                            for tool in Tool::selection_tools() {
                                tool_button(ui, *tool, &mut actions, input.current_tool);
                            }
                            ui.separator();
                            // Compact style controls.
                            let _ = ui.add(
                                egui::Slider::new(&mut input.settings.stroke_width, 1.0..=24.0)
                                    .show_value(false)
                                    .text("width"),
                            );
                            let highlighter_active =
                                input.settings.ink_color == InkColor::Highlighter;
                            let resp = ui.selectable_label(highlighter_active, "Highlight");
                            if resp.clicked() {
                                input.settings.ink_color = if highlighter_active {
                                    InkColor::Pen
                                } else {
                                    InkColor::Highlighter
                                };
                            }
                            let mut col = [
                                input.settings.text_color[0] as f32 / 255.0,
                                input.settings.text_color[1] as f32 / 255.0,
                                input.settings.text_color[2] as f32 / 255.0,
                            ];
                            if ui
                                .color_edit_button_rgb(&mut col)
                                .on_hover_text("Ink / text colour")
                                .changed()
                            {
                                input.settings.text_color = [
                                    (col[0] * 255.0).round() as u8,
                                    (col[1] * 255.0).round() as u8,
                                    (col[2] * 255.0).round() as u8,
                                    255,
                                ];
                            }
                            // Tiny pressure indicator.
                            let pcolor = if input.last_pressure < 0.999 {
                                egui::Color32::from_rgb(20, 110, 30)
                            } else {
                                egui::Color32::from_rgb(140, 140, 140)
                            };
                            ui.label(
                                egui::RichText::new(format!("{:.2}", input.last_pressure))
                                    .color(pcolor)
                                    .small(),
                            )
                            .on_hover_text("Pen pressure (1.00 = no pressure data)");
                        });
                    });
            });
    }

    // Settings window — populated when toggled open.
    if *input.settings_open {
        egui::Window::new("Settings")
            .resizable(false)
            .collapsible(false)
            .default_pos([60.0, 80.0])
            .show(ctx, |ui| {
                ui.heading("Drawing");
                ui.horizontal(|ui| {
                    ui.label("Eraser radius");
                    ui.add(egui::Slider::new(
                        &mut input.settings.eraser_radius,
                        4.0..=64.0,
                    ));
                });
                ui.horizontal(|ui| {
                    ui.label("Text size  ");
                    ui.add(egui::Slider::new(&mut input.settings.text_size, 10.0..=96.0));
                });
                ui.checkbox(&mut input.settings.pressure_sensitive, "Pressure-sensitive width")
                    .on_hover_text(
                        "Needs a tablet driver that reports pressure — OpenTabletDriver users: \
                         use a tablet output plugin, not mouse-emulation.",
                    );
                ui.checkbox(&mut input.settings.tilt_modulation, "Tilt-sensitive width")
                    .on_hover_text("Wider stroke when the stylus is tilted (chisel feel)");

                ui.add_space(10.0);
                ui.separator();
                ui.heading("Font slots");
                ui.label("Map abstract slots to installed font family names.");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Default");
                    ui.text_edit_singleline(&mut input.fonts.default);
                });
                ui.horizontal(|ui| {
                    ui.label("Mono   ");
                    ui.text_edit_singleline(&mut input.fonts.mono);
                });
                ui.horizontal(|ui| {
                    ui.label("Serif  ");
                    ui.text_edit_singleline(&mut input.fonts.serif);
                });
                ui.horizontal(|ui| {
                    ui.label("Slab   ");
                    ui.text_edit_singleline(&mut input.fonts.slab);
                });
                ui.horizontal(|ui| {
                    ui.label("Theme  ");
                    ui.text_edit_singleline(&mut input.fonts.thematic);
                });
                ui.horizontal(|ui| {
                    ui.label("Active font slot");
                    egui::ComboBox::from_id_salt("font_slot")
                        .selected_text(input.settings.font_slot.label())
                        .show_ui(ui, |ui| {
                            for slot in FontSlot::all() {
                                ui.selectable_value(
                                    &mut input.settings.font_slot,
                                    slot,
                                    format!("{}  ({})", slot.label(), input.fonts.resolve(slot)),
                                );
                            }
                        });
                });
                ui.add_space(8.0);
                ui.separator();
                ui.label(
                    egui::RichText::new("Settings auto-save to .writee-settings.toml in your workspace.")
                        .weak(),
                );
                if ui.button("Close").clicked() {
                    *input.settings_open = false;
                }
            });
    }

    actions
}

fn tool_button(ui: &mut egui::Ui, tool: Tool, actions: &mut UiActions, current: Option<Tool>) {
    let selected = current == Some(tool);
    let txt = egui::RichText::new(short_button_label(tool)).size(13.0);
    // Wider than the previous icon-only buttons because the labels are now
    // real words. egui shrinks the actual label box to fit so this only
    // sets an upper bound.
    let resp = ui.add_sized([56.0, 26.0], egui::SelectableLabel::new(selected, txt));
    if resp.clicked() {
        actions.set_tool = Some(tool);
    }
    resp.on_hover_text(tool.label());
}

/// Persistent left-side hierarchical file tree. Files are nested under any
/// other file whose SubNotes link to them, mirroring the parent → child
/// relationship in the workspace. Files no one links to appear at the root.
pub fn build_file_tree_sidebar(
    ctx: &egui::Context,
    workspace_root: &std::path::Path,
    tree: &crate::workspace::WorkspaceTree,
    current: Option<&std::path::Path>,
    welcome_name: &str,
    trash: &[PathBuf],
    backlinks: &[String],
    tags: &std::collections::BTreeMap<String, Vec<PathBuf>>,
    active_tag: Option<&str>,
    actions: &mut UiActions,
) -> Option<PathBuf> {
    let mut opened = None;
    egui::SidePanel::left("file-tree")
        .resizable(true)
        .default_width(240.0)
        .width_range(180.0..=420.0)
        .frame(
            egui::Frame::default()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Vec2::new(8.0, 8.0))
                .stroke(egui::Stroke::new(1.0, ctx.style().visuals.window_stroke.color)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Workspace").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("+ New").on_hover_text("New whiteboard (Ctrl+N)").clicked() {
                        actions.new_file = true;
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                if tree.roots.is_empty() {
                    ui.label(
                        egui::RichText::new("No files yet. Click + New to create one.")
                            .italics()
                            .weak(),
                    );
                } else {
                    let mut visited: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for root in &tree.roots {
                        render_tree_node(
                            ui,
                            workspace_root,
                            tree,
                            root,
                            current,
                            welcome_name,
                            0,
                            &mut visited,
                            actions,
                            &mut opened,
                        );
                    }
                }

                // -- Tags section -----------------------------------------------
                if !tags.is_empty() {
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("# Tags ({})", tags.len()))
                            .small()
                            .weak(),
                    )
                    .default_open(false)
                    .show(ui, |ui| {
                        if active_tag.is_some() {
                            if ui
                                .selectable_label(false, "× clear filter")
                                .on_hover_text("Show all files again")
                                .clicked()
                            {
                                actions.set_tag_filter = Some(None);
                            }
                        }
                        for (tag, files) in tags {
                            let label = format!("#{}  ({})", tag, files.len());
                            let on = active_tag == Some(tag.as_str());
                            if ui.selectable_label(on, label).clicked() {
                                actions.set_tag_filter = Some(if on {
                                    None
                                } else {
                                    Some(tag.clone())
                                });
                            }
                        }
                    });
                }

                // -- Backlinks section -------------------------------------------
                if !backlinks.is_empty() {
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("Backlinks ({})", backlinks.len()))
                            .small()
                            .weak(),
                    )
                    .default_open(true)
                    .show(ui, |ui| {
                        for name in backlinks {
                            let path = workspace_root.join(name);
                            let row = ui.add_sized(
                                [ui.available_width(), 20.0],
                                egui::SelectableLabel::new(false, name.as_str()),
                            );
                            if row.clicked() {
                                opened = Some(path);
                            }
                        }
                    });
                }

                // -- Trash section ------------------------------------------------
                if !trash.is_empty() {
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("Trash ({})", trash.len()))
                            .small()
                            .weak(),
                    )
                    .default_open(false)
                    .show(ui, |ui| {
                        for path in trash {
                            let display = path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .map(strip_trash_prefix)
                                .unwrap_or_else(|| "?".to_string());
                            let row = ui.add_sized(
                                [ui.available_width(), 20.0],
                                egui::SelectableLabel::new(false, display),
                            );
                            row.context_menu(|cm| {
                                if cm.button("Restore").clicked() {
                                    actions.restore_file = Some(path.clone());
                                    cm.close_menu();
                                }
                                if cm.button("Delete forever").clicked() {
                                    actions.purge_file = Some(path.clone());
                                    cm.close_menu();
                                }
                            });
                        }
                    });
                }
            });
        });
    opened
}

/// Command palette modal.
pub fn build_command_palette(
    ctx: &egui::Context,
    query: &mut String,
    focus_idx: usize,
    needs_focus: bool,
    rows: &[crate::palette::PaletteRow],
    actions: &mut UiActions,
) {
    egui::Window::new("Command palette")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
        .default_width(560.0)
        .frame(
            egui::Frame::popup(&ctx.style())
                .fill(ctx.style().visuals.panel_fill)
                .stroke(ctx.style().visuals.window_stroke)
                .rounding(egui::Rounding::same(10.0))
                .inner_margin(egui::Vec2::new(12.0, 10.0)),
        )
        .show(ctx, |ui| {
            ui.set_width(540.0);
            let resp = ui.add(
                egui::TextEdit::singleline(query)
                    .hint_text("Search files, content, or actions…")
                    .font(egui::FontId::proportional(15.0))
                    .desired_width(f32::INFINITY),
            );
            // Only auto-focus the very first frame the palette is open —
            // re-requesting every frame breaks subsequent interaction.
            if needs_focus {
                resp.request_focus();
                actions.palette_focused = true;
            }
            // Enter → pick focused row; Esc → close. Arrow keys → move focus.
            ui.input(|i| {
                if i.key_pressed(egui::Key::Enter) && !rows.is_empty() {
                    actions.palette_pick = Some(focus_idx.min(rows.len() - 1));
                }
                if i.key_pressed(egui::Key::Escape) {
                    actions.palette_close = true;
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    actions.palette_focus_next = true;
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    actions.palette_focus_prev = true;
                }
            });
            ui.separator();
            if rows.is_empty() {
                ui.label(egui::RichText::new("(no matches)").weak().italics());
                return;
            }
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, row) in rows.iter().enumerate() {
                        let (icon, primary, secondary) = match row {
                            crate::palette::PaletteRow::Action(a) => {
                                (">", a.label().to_string(), String::new())
                            }
                            crate::palette::PaletteRow::File(p) => (
                                "▢",
                                p.file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                "file".to_string(),
                            ),
                            crate::palette::PaletteRow::Content { file, snippet } => (
                                "¶",
                                file.file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                snippet.clone(),
                            ),
                        };
                        let selected = i == focus_idx;
                        let label = if secondary.is_empty() {
                            format!("  {icon}  {primary}")
                        } else {
                            format!("  {icon}  {primary}    — {secondary}")
                        };
                        let row_resp = ui.add_sized(
                            [ui.available_width(), 22.0],
                            egui::SelectableLabel::new(selected, label),
                        );
                        if row_resp.clicked() {
                            actions.palette_pick = Some(i);
                        }
                    }
                });
        });
}

/// "1716843200_my-note.writee" → "my-note" for the trash row label.
fn strip_trash_prefix(name: &str) -> String {
    let stem = name.strip_suffix(".writee").unwrap_or(name);
    stem.split_once('_').map(|(_, rest)| rest.to_string()).unwrap_or_else(|| stem.to_string())
}

#[allow(clippy::too_many_arguments)]
fn render_tree_node(
    ui: &mut egui::Ui,
    workspace_root: &std::path::Path,
    tree: &crate::workspace::WorkspaceTree,
    name: &str,
    current: Option<&std::path::Path>,
    welcome_name: &str,
    depth: usize,
    visited: &mut std::collections::HashSet<String>,
    actions: &mut UiActions,
    opened: &mut Option<PathBuf>,
) {
    if !visited.insert(name.to_string()) {
        return;
    }
    let path = workspace_root.join(name);
    let is_current = Some(path.as_path()) == current;
    let is_welcome = name == welcome_name;
    let children = tree
        .children
        .get(name)
        .cloned()
        .unwrap_or_default();
    let indent = (depth as f32) * 14.0;

    ui.horizontal(|ui| {
        ui.add_space(indent);
        if !children.is_empty() {
            ui.label(egui::RichText::new("v").weak().monospace());
        } else {
            ui.label(egui::RichText::new("-").weak().monospace());
        }
        let display = if is_welcome {
            format!("* {name}")
        } else {
            name.to_string()
        };
        let row = ui
            .add_sized(
                [ui.available_width(), 22.0],
                egui::SelectableLabel::new(is_current, display),
            )
            .on_hover_text(path.display().to_string());
        if row.clicked() && !is_current {
            *opened = Some(path.clone());
        }
        row.context_menu(|cm| {
            if cm.button("Rename…").clicked() {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("note");
                actions.rename_file = Some((path.clone(), stem.to_string()));
                cm.close_menu();
            }
            if !is_welcome {
                if cm.button("Delete").clicked() {
                    actions.delete_file = Some(path.clone());
                    cm.close_menu();
                }
            }
        });
    });
    for child in children {
        render_tree_node(
            ui,
            workspace_root,
            tree,
            &child,
            current,
            welcome_name,
            depth + 1,
            visited,
            actions,
            opened,
        );
    }
}

fn short_button_label(t: Tool) -> &'static str {
    // Short text labels — egui's default bundled font only covers basic
    // Latin + a handful of symbols, so fancier unicode glyphs (✎ ⛓ etc.)
    // would render as empty boxes. Hover tooltip carries the verbose name
    // and shortcut.
    match t {
        Tool::Pen => "Pen",
        Tool::Highlighter => "Mark",
        Tool::Eraser => "Erase",
        Tool::Arrow => "Arrow",
        Tool::Shape => "Shape",
        Tool::Text => "Text",
        Tool::Note => "Note",
        Tool::Index => "Index",
        Tool::Link => "Link",
        Tool::Select => "Select",
    }
}

/// Build egui Visuals from a writee colour theme.
///
/// Notable: we do NOT use `override_text_color` — it forces every widget
/// to the same colour and the selected-row foreground stops contrasting
/// against the dark selection background. Per-state `fg_stroke` settings
/// are how egui's `SelectableLabel` picks readable text.
pub fn visuals_from_theme(theme: &writee_core::ColorTheme) -> egui::Visuals {
    let to = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let mut v = if theme.canvas_bg[0] < 50 {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.window_fill = to(theme.chrome_bg);
    v.panel_fill = to(theme.chrome_bg);
    v.window_stroke = Stroke::new(1.0, to(theme.chrome_border));

    let text = to(theme.chrome_text);
    let text_on_active = to(theme.chrome_text_on_active);

    v.widgets.noninteractive.bg_fill = to(theme.chrome_bg);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);

    v.widgets.inactive.bg_fill = to(theme.chrome_panel_bg);
    v.widgets.inactive.weak_bg_fill = to(theme.chrome_panel_bg);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);

    v.widgets.hovered.bg_fill = to(theme.chrome_hover_bg);
    v.widgets.hovered.weak_bg_fill = to(theme.chrome_hover_bg);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, text);

    v.widgets.active.bg_fill = to(theme.chrome_active_bg);
    v.widgets.active.weak_bg_fill = to(theme.chrome_active_bg);
    v.widgets.active.fg_stroke = Stroke::new(1.5, text_on_active);

    // Open/expanded widgets (selected SelectableLabels) sit in `open`.
    v.widgets.open.bg_fill = to(theme.chrome_active_bg);
    v.widgets.open.weak_bg_fill = to(theme.chrome_active_bg);
    v.widgets.open.fg_stroke = Stroke::new(1.5, text_on_active);

    v.selection.bg_fill = to(theme.chrome_active_bg);
    v.selection.stroke = Stroke::new(1.5, text_on_active);

    v
}

