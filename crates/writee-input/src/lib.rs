//! Platform-agnostic input abstraction.

use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolType {
    Mouse,
    Pen,
    Finger,
    Eraser,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplePhase {
    Begin,
    Move,
    End,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub struct InkSample {
    pub pos: Vec2,
    /// 0..=1; comes from a real driver when available (libinput tablet,
    /// XInput, Wayland tablet-v2, NSEvent, Android MotionEvent, or — on
    /// Linux — a direct evdev side channel). Falls back to 1.0 if no
    /// pressure source is present.
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub tool: ToolType,
    pub phase: SamplePhase,
    pub t_ms: u32,
}

#[cfg(target_os = "linux")]
mod linux_tablet;

#[cfg(target_os = "macos")]
mod macos_tablet;

pub mod winit_adapter;
pub use winit_adapter::WinitInput;
