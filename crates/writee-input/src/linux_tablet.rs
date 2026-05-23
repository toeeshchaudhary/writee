//! Linux pen-pressure / tilt / eraser side channel.
//!
//! winit on Linux receives tablet pens as plain pointer events
//! (`CursorMoved` / `MouseInput`), which have no pressure, tilt, or
//! eraser-end information. To get those into the app we open every input
//! device that exposes an `ABS_PRESSURE` axis and read its event stream in
//! a background thread, publishing the latest values to shared atomics that
//! the main thread samples on every winit pointer event.
//!
//! This works for:
//!   * Real tablets the user can open directly (in the `input` group).
//!   * OpenTabletDriver in *tablet output* mode (it creates a virtual
//!     uinput device that re-emits ABS_PRESSURE and friends).
//!
//! It does NOT work for:
//!   * OpenTabletDriver in *mouse-emulation* output mode — there's no
//!     pressure axis anywhere in the system. Switch OTD to a tablet
//!     output plugin to get pressure.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};

/// Shared pen state populated by the background evdev reader threads.
///
/// All fields are encoded as atomics so the main thread can sample without
/// locking. `pressure`/`tilt_x`/`tilt_y` are normalised f32 in [-1, 1] (tilt)
/// or [0, 1] (pressure), stored via `f32::to_bits`.
pub struct PenState {
    pub pressure: AtomicU32,
    pub tilt_x: AtomicU32,
    pub tilt_y: AtomicU32,
    /// True while BTN_TOOL_RUBBER is asserted (eraser end of the stylus
    /// touching the surface).
    pub eraser: AtomicBool,
}

impl PenState {
    fn new() -> Self {
        Self {
            pressure: AtomicU32::new(f32::to_bits(1.0)),
            tilt_x: AtomicU32::new(f32::to_bits(0.0)),
            tilt_y: AtomicU32::new(f32::to_bits(0.0)),
            eraser: AtomicBool::new(false),
        }
    }
}

/// Best-effort: spawn one reader thread per pressure-capable device.
/// Returns the shared [`PenState`]. Defaults to pressure=1.0 / no tilt /
/// eraser=false if nothing ever updates it, so non-tablet users see
/// "full pressure" rather than zero-width strokes.
pub fn spawn_pressure_reader() -> Arc<PenState> {
    let shared = Arc::new(PenState::new());
    let devices: Vec<_> = evdev::enumerate()
        .filter(|(_, dev)| has_pressure_axis(dev))
        .collect();

    if devices.is_empty() {
        log::info!("writee-input: no /dev/input device exposes ABS_PRESSURE");
        return shared;
    }

    for (path, dev) in devices {
        log::info!(
            "writee-input: reading pressure from {} ({})",
            path.display(),
            dev.name().unwrap_or("?"),
        );
        let pressure_max: f32 = abs_axis_max(&dev, AbsoluteAxisCode::ABS_PRESSURE).unwrap_or(1024.0);
        let tilt_x_max: f32 = abs_axis_max(&dev, AbsoluteAxisCode::ABS_TILT_X).unwrap_or(64.0);
        let tilt_y_max: f32 = abs_axis_max(&dev, AbsoluteAxisCode::ABS_TILT_Y).unwrap_or(64.0);
        let shared = shared.clone();
        let path_for_log = path.clone();
        thread::spawn(move || {
            // Re-open the device inside the thread so we own a fresh handle
            // (the enumerate iterator holds the original).
            let mut dev = match Device::open(&path_for_log) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!(
                        "writee-input: open {} failed: {e:?}",
                        path_for_log.display()
                    );
                    return;
                }
            };
            loop {
                match dev.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            match ev.destructure() {
                                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_PRESSURE, raw) => {
                                    let n = (raw as f32 / pressure_max).clamp(0.0, 1.0);
                                    shared.pressure.store(f32::to_bits(n), Ordering::Relaxed);
                                }
                                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_TILT_X, raw) => {
                                    let n = (raw as f32 / tilt_x_max).clamp(-1.0, 1.0);
                                    shared.tilt_x.store(f32::to_bits(n), Ordering::Relaxed);
                                }
                                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_TILT_Y, raw) => {
                                    let n = (raw as f32 / tilt_y_max).clamp(-1.0, 1.0);
                                    shared.tilt_y.store(f32::to_bits(n), Ordering::Relaxed);
                                }
                                EventSummary::Key(_, KeyCode::BTN_TOOL_RUBBER, value) => {
                                    shared.eraser.store(value != 0, Ordering::Relaxed);
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "writee-input: fetch_events on {} failed: {e:?}",
                            path_for_log.display()
                        );
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        });
    }

    shared
}

fn has_pressure_axis(dev: &Device) -> bool {
    dev.supported_absolute_axes()
        .map(|set| set.contains(AbsoluteAxisCode::ABS_PRESSURE))
        .unwrap_or(false)
}

fn abs_axis_max(dev: &Device, axis: AbsoluteAxisCode) -> Option<f32> {
    dev.get_absinfo().ok().and_then(|mut infos| {
        infos
            .find(|(a, _)| *a == axis)
            .map(|(_, info)| (info.maximum() as f32).abs().max(1.0))
    })
}
