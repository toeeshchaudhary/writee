//! Color tokens used across the whole app (renderer, geometry, egui chrome).
//!
//! Centralised here so we have exactly two presets (`LIGHT`, `DARK`) instead
//! of hex literals scattered across four crates. Every colour the user can
//! see goes through one of these tokens.
//!
//! Note: tessellated ink geometry bakes colours into each vertex, so a theme
//! switch must invalidate the geometry cache. The App calls
//! `CommittedCache::mark_dirty` on theme change.

use serde::{Deserialize, Serialize};

/// Top-level theme identifier (persisted to settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    Light,
    Dark,
}

impl ThemeName {
    pub fn label(self) -> &'static str {
        match self {
            ThemeName::Light => "Light",
            ThemeName::Dark => "Dark",
        }
    }

    pub fn theme(self) -> &'static ColorTheme {
        match self {
            ThemeName::Light => &ColorTheme::LIGHT,
            ThemeName::Dark => &ColorTheme::DARK,
        }
    }
}

/// Concrete colour palette. `_a` variants include alpha.
#[derive(Debug, Clone, Copy)]
pub struct ColorTheme {
    // -- Canvas / grid --
    pub canvas_bg: [u8; 4],
    pub grid_dot: [u8; 4],

    // -- Ink palette --
    pub ink: [u8; 4],
    pub highlight: [u8; 4],
    pub selection: [u8; 4],
    pub marquee: [u8; 4],
    pub link: [u8; 4],

    // -- Card chrome --
    pub card_inline_fill: [u8; 4],
    pub card_index_fill: [u8; 4],
    pub card_linked_fill: [u8; 4],
    pub card_border: [u8; 4],
    pub card_border_locked: [u8; 4],
    pub card_shadow: [u8; 4],

    // -- Text --
    pub text_default: [u8; 4],
    pub text_weak: [u8; 4],

    // -- egui chrome --
    pub chrome_bg: [u8; 4],
    pub chrome_panel_bg: [u8; 4],
    pub chrome_border: [u8; 4],
    pub chrome_hover_bg: [u8; 4],
    pub chrome_active_bg: [u8; 4],
    pub chrome_text: [u8; 4],
    pub chrome_text_on_active: [u8; 4],
}

impl ColorTheme {
    pub const LIGHT: ColorTheme = ColorTheme {
        canvas_bg: [251, 251, 251, 255],
        grid_dot: [158, 158, 158, 255],

        ink: [18, 18, 18, 255],
        highlight: [255, 220, 60, 110],
        selection: [255, 110, 30, 235],
        marquee: [120, 120, 180, 160],
        link: [60, 70, 90, 220],

        card_inline_fill: [255, 250, 222, 255],
        card_index_fill: [243, 240, 250, 255],
        card_linked_fill: [253, 253, 255, 255],
        card_border: [60, 60, 80, 255],
        card_border_locked: [80, 80, 120, 255],
        card_shadow: [40, 40, 60, 28],

        text_default: [18, 18, 18, 255],
        text_weak: [110, 110, 110, 255],

        chrome_bg: [252, 252, 252, 255],
        chrome_panel_bg: [248, 248, 250, 255],
        chrome_border: [220, 220, 224, 255],
        chrome_hover_bg: [220, 220, 220, 255],
        chrome_active_bg: [28, 28, 28, 255],
        chrome_text: [40, 40, 40, 255],
        chrome_text_on_active: [255, 255, 255, 255],
    };

    pub const DARK: ColorTheme = ColorTheme {
        canvas_bg: [22, 23, 26, 255],
        grid_dot: [70, 72, 78, 255],

        ink: [232, 232, 232, 255],
        highlight: [255, 210, 60, 100],
        selection: [255, 140, 60, 235],
        marquee: [150, 150, 210, 160],
        link: [180, 188, 210, 220],

        card_inline_fill: [42, 40, 30, 255],
        card_index_fill: [40, 38, 56, 255],
        card_linked_fill: [36, 38, 44, 255],
        card_border: [110, 112, 130, 255],
        card_border_locked: [150, 150, 190, 255],
        card_shadow: [0, 0, 0, 80],

        text_default: [232, 232, 232, 255],
        text_weak: [150, 150, 158, 255],

        chrome_bg: [28, 30, 34, 255],
        chrome_panel_bg: [22, 24, 28, 255],
        chrome_border: [55, 58, 66, 255],
        chrome_hover_bg: [50, 54, 60, 255],
        chrome_active_bg: [235, 235, 235, 255],
        chrome_text: [220, 220, 220, 255],
        chrome_text_on_active: [20, 20, 20, 255],
    };
}
