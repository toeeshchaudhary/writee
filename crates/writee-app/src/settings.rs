//! Runtime tool settings + workspace-level persistence.
//!
//! The settings TOML lives at `<workspace>/.writee-settings.toml`, so each
//! workspace is fully portable — copying or syncing the directory keeps the
//! user's preferences with the documents they belong to.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InkColor {
    Pen,
    Highlighter,
}

impl InkColor {
    pub fn to_idx(self) -> u8 {
        match self {
            InkColor::Pen => 0,
            InkColor::Highlighter => 1,
        }
    }

    pub fn rgba(self) -> [u8; 4] {
        use writee_core::{COLOR_HIGHLIGHT, COLOR_INK};
        match self {
            InkColor::Pen => COLOR_INK,
            InkColor::Highlighter => COLOR_HIGHLIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontSlot {
    #[default]
    Default,
    Mono,
    Serif,
    Slab,
    Thematic,
}

impl FontSlot {
    pub fn all() -> [FontSlot; 5] {
        [
            FontSlot::Default,
            FontSlot::Mono,
            FontSlot::Serif,
            FontSlot::Slab,
            FontSlot::Thematic,
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            FontSlot::Default => "Default",
            FontSlot::Mono => "Mono",
            FontSlot::Serif => "Serif",
            FontSlot::Slab => "Slab",
            FontSlot::Thematic => "Thematic",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActiveShape {
    Rectangle,
    Ellipse,
    Line,
}

impl ActiveShape {
    pub fn label(self) -> &'static str {
        match self {
            ActiveShape::Rectangle => "Rect",
            ActiveShape::Ellipse => "Ellipse",
            ActiveShape::Line => "Line",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ToolSettings {
    pub stroke_width: f32,
    pub eraser_radius: f32,
    pub text_size: f32,
    pub pressure_sensitive: bool,
    pub ink_color: InkColor,
    pub text_color: [u8; 4],
    pub active_shape: ActiveShape,
    pub shape_filled: bool,
    pub font_slot: FontSlot,
    /// Modulate stroke width by tilt magnitude (asymmetric chisel feel).
    /// Width-only for v1 (no rotation of the cross-section).
    pub tilt_modulation: bool,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            stroke_width: 4.0,
            eraser_radius: 14.0,
            text_size: 24.0,
            pressure_sensitive: true,
            ink_color: InkColor::Pen,
            text_color: [18, 18, 18, 255],
            active_shape: ActiveShape::Rectangle,
            shape_filled: false,
            font_slot: FontSlot::Default,
            tilt_modulation: true,
        }
    }
}

/// Maps abstract font slots to actual font family names. The renderer feeds
/// these into glyphon when laying out text. Users edit them in the settings
/// page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontMappings {
    pub default: String,
    pub mono: String,
    pub serif: String,
    pub slab: String,
    pub thematic: String,
}

impl Default for FontMappings {
    fn default() -> Self {
        Self {
            default: "Inter".into(),
            mono: "JetBrains Mono".into(),
            serif: "Source Serif Pro".into(),
            slab: "Roboto Slab".into(),
            thematic: "Inter".into(),
        }
    }
}

impl FontMappings {
    pub fn resolve(&self, slot: FontSlot) -> &str {
        match slot {
            FontSlot::Default => &self.default,
            FontSlot::Mono => &self.mono,
            FontSlot::Serif => &self.serif,
            FontSlot::Slab => &self.slab,
            FontSlot::Thematic => &self.thematic,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub tools: ToolSettings,
    pub fonts: FontMappings,
    pub window: WindowConfig,
    pub state: PersistedState,
    /// Light or dark colour palette (one of writee_core::ThemeName).
    pub theme: writee_core::ThemeName,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 1280, height: 800 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedState {
    pub last_file: Option<String>,
}

const FILE_NAME: &str = ".writee-settings.toml";

impl WorkspaceConfig {
    pub fn path(workspace_root: &Path) -> PathBuf {
        workspace_root.join(FILE_NAME)
    }

    pub fn load(workspace_root: &Path) -> Self {
        let path = Self::path(workspace_root);
        match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                log::warn!("failed to parse {}: {e:?}; using defaults", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let path = Self::path(workspace_root);
        let body = toml::to_string_pretty(self).context("serialize settings")?;
        fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }
}

pub fn color_from_idx(idx: u8) -> [u8; 4] {
    use writee_core::{COLOR_HIGHLIGHT, COLOR_INK};
    match idx {
        1 => COLOR_HIGHLIGHT,
        _ => COLOR_INK,
    }
}
