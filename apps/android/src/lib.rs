//! Android entry point. Built as a `cdylib`; the loader invokes
//! [`android_main`] via the NativeActivity glue from `android-activity`.
//!
//! Most of the platform logic lives in `writee-app`; this file is just the
//! `#[no_mangle]` glue that hands the `AndroidApp` to winit.

#![cfg(target_os = "android")]

use android_activity::AndroidApp;

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("writee"),
    );
    if let Err(e) = writee_app::run_android(app) {
        log::error!("writee exited with error: {e:?}");
    }
}
