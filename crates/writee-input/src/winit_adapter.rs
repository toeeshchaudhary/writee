use crate::{InkSample, SamplePhase, ToolType};
use glam::Vec2;
use std::time::Instant;
use winit::event::{ElementState, MouseButton, Touch, TouchPhase, WindowEvent};

#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "linux")]
use std::sync::Arc;

/// Translates winit `WindowEvent`s into a stream of [`InkSample`]s.
///
/// Per-platform pressure sources:
/// * **Linux**: a background evdev reader fills in pressure / tilt /
///   eraser-end that winit's pointer stream doesn't carry (see
///   `linux_tablet.rs`).
/// * **Windows**: winit 0.30 surfaces stylus pen events as
///   `WindowEvent::Touch` with `Force::Normalized` carrying real pressure
///   (via WM_POINTER under the hood). The Touch arm below already prefers
///   `force.normalized()` — no extra plumbing needed.
/// * **macOS**: winit doesn't expose stylus pressure for Apple Pencil /
///   tablet input. A `macos_tablet.rs` side channel mirrors the Linux
///   pattern. (See Phase 6b in the project plan.)
pub struct WinitInput {
    epoch: Instant,
    mouse_pos: Vec2,
    mouse_down: bool,
    #[cfg(target_os = "linux")]
    pen: Arc<crate::linux_tablet::PenState>,
    #[cfg(target_os = "macos")]
    pen: Arc<crate::macos_tablet::PenState>,
}

impl Default for WinitInput {
    fn default() -> Self {
        Self::new()
    }
}

impl WinitInput {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            mouse_pos: Vec2::ZERO,
            mouse_down: false,
            #[cfg(target_os = "linux")]
            pen: crate::linux_tablet::spawn_pressure_reader(),
            #[cfg(target_os = "macos")]
            pen: crate::macos_tablet::spawn_pressure_reader(),
        }
    }

    fn now_ms(&self) -> u32 {
        self.epoch.elapsed().as_millis() as u32
    }

    /// Pressure sourced from the platform side channel where available.
    fn current_pressure(&self) -> f32 {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            return f32::from_bits(self.pen.pressure.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            1.0
        }
    }

    fn current_tilt(&self) -> (f32, f32) {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let tx = f32::from_bits(self.pen.tilt_x.load(Ordering::Relaxed)).clamp(-1.0, 1.0);
            let ty = f32::from_bits(self.pen.tilt_y.load(Ordering::Relaxed)).clamp(-1.0, 1.0);
            return (tx, ty);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            (0.0, 0.0)
        }
    }

    /// Whether the stylus is currently presenting its eraser end. Polled by
    /// the app each frame to temporarily override the active tool.
    pub fn eraser_active(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            return self.pen.eraser.load(Ordering::Relaxed);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            false
        }
    }

    pub fn handle(&mut self, ev: &WindowEvent) -> Option<InkSample> {
        match ev {
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = Vec2::new(position.x as f32, position.y as f32);
                if self.mouse_down {
                    let (tx, ty) = self.current_tilt();
                    return Some(InkSample {
                        pos: self.mouse_pos,
                        pressure: self.current_pressure(),
                        tilt_x: tx,
                        tilt_y: ty,
                        tool: ToolType::Mouse,
                        phase: SamplePhase::Move,
                        t_ms: self.now_ms(),
                    });
                }
                None
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let begin = matches!(state, ElementState::Pressed);
                self.mouse_down = begin;
                let (tx, ty) = self.current_tilt();
                Some(InkSample {
                    pos: self.mouse_pos,
                    pressure: self.current_pressure(),
                    tilt_x: tx,
                    tilt_y: ty,
                    tool: ToolType::Mouse,
                    phase: if begin { SamplePhase::Begin } else { SamplePhase::End },
                    t_ms: self.now_ms(),
                })
            }
            WindowEvent::Touch(Touch { location, force, phase, .. }) => {
                // Prefer the winit-reported force when present (Wayland
                // tablet-v2, macOS NSEvent). Otherwise fall back to the
                // platform side channel.
                let pressure = force
                    .map(|f| f.normalized() as f32)
                    .unwrap_or_else(|| self.current_pressure());
                let (tx, ty) = self.current_tilt();
                let phase = match phase {
                    TouchPhase::Started => SamplePhase::Begin,
                    TouchPhase::Moved => SamplePhase::Move,
                    TouchPhase::Ended => SamplePhase::End,
                    TouchPhase::Cancelled => SamplePhase::Cancel,
                };
                Some(InkSample {
                    pos: Vec2::new(location.x as f32, location.y as f32),
                    pressure,
                    tilt_x: tx,
                    tilt_y: ty,
                    tool: ToolType::Pen,
                    phase,
                    t_ms: self.now_ms(),
                })
            }
            _ => None,
        }
    }
}
