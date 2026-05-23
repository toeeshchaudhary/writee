# writee-android

Android port of writee. Built as a `cdylib` that `android-activity`'s
NativeActivity loader picks up via `android_main`.

## Status

Code-complete scaffold; **not verified on a device** from the development
machine that built this. Treat the build as work-in-progress until the first
APK runs on real hardware.

## Build

You need:

- Rust toolchain (the desktop build's toolchain is fine)
- Android NDK installed; `ANDROID_NDK_HOME` set
- Either `cargo-apk` or `xbuild` installed:
  - `cargo install cargo-apk`, or
  - `cargo install xbuild`

### With `cargo-apk`

```
rustup target add aarch64-linux-android
cargo apk run -p writee-android
```

(Connects to a device or running emulator via `adb`.)

### With `xbuild`

```
x doctor                       # sanity check
x build --device adb:<serial> -p writee-android
```

## Known gaps to address on first device run

1. **Stylus pressure/tilt** — winit's Android backend may not expose all axes.
   The fallback is to read `MotionEvent` directly via the `ndk` crate and merge
   with winit's pointer events. See `writee-input/src/winit_adapter.rs` and the
   palm-rejection plan in `~/.claude/plans/optimized-stirring-engelbart.md`.
2. **Workspace path** — `directories-next` returns an Android-internal data
   directory, which is fine for storage but invisible to the user. To let the
   user pick a folder they can sync (Syncthing etc.), wire a Storage Access
   Framework picker via JNI.
3. **On-screen toolbar** — the desktop build is keyboard-driven. Android has
   no physical keyboard by default; a small on-screen tool bar will be needed
   in the next pass. Reuse the existing tool state machine.
4. **SurfaceView lifecycle** — winit handles `suspended` / `resumed` already
   via `ApplicationHandler`; the renderer recreates its surface on resume.
   Verify this still holds when the device rotates or the app goes to
   background.
