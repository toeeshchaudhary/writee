//! macOS stylus pressure side channel (stub).
//!
//! winit 0.30 on macOS does not surface `NSEvent.pressure()` or `NSEvent.tilt`
//! for tablet input. To get real Apple Pencil / Wacom pressure on macOS,
//! we need a global NSEvent monitor (registered via
//! `+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]`) that filters
//! for `NSTabletPointEventSubtype` events and publishes pressure / tilt /
//! `pointingDeviceType == eraser` into the atomics below.
//!
//! That work needs a macOS dev box to verify the Objective-C block ABI and
//! NSEvent introspection. This stub keeps the cross-platform glue in place
//! so a macOS contributor can fill in `spawn_pressure_reader` without
//! touching the rest of the codebase. Today: pressure defaults to 1.0 (the
//! same behaviour as Windows did before that platform's path was verified).
//!
//! See: `crates/writee-input/src/linux_tablet.rs` for the working evdev
//! analogue this should mirror.

use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;

pub struct PenState {
    pub pressure: AtomicU32,
    pub tilt_x: AtomicU32,
    pub tilt_y: AtomicU32,
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

/// Today: returns a no-op `PenState` (pressure=1.0). When a macOS dev wires
/// the NSEvent monitor up, replace the body of this fn with the equivalent
/// of `linux_tablet::spawn_pressure_reader` — same atomic semantics, same
/// shared return type.
pub fn spawn_pressure_reader() -> Arc<PenState> {
    log::info!(
        "writee-input: macOS stylus pressure side channel is not yet implemented; \
         pressure will read as 1.0 (flat). See crates/writee-input/src/macos_tablet.rs."
    );
    Arc::new(PenState::new())
}
