//! OPlus hardware touch service for Redmi Note 12 4G (topaz/SM6225)
//!
//! Implements AOD, DT2W, gestures, raise-to-wake, and the xiaomi-touch ABI.

pub mod aod;
pub mod aod_defaults;
pub mod config;
pub mod doze;
pub mod error;
pub mod evdev;
pub mod gesture;
pub mod hal;
pub mod panel_feature;
pub mod procfs;
pub mod raise_to_wake;
pub mod screen_state;
pub mod sysfs;
pub mod uinput;
pub mod wake;
pub mod xiaomi_touch;

pub use aod::{AodManager, AodState};
pub use config::TouchConfig;
pub use doze::{start as start_doze_keeper, DozeKeeperHandle};
pub use error::{TouchError, TouchResult};
pub use evdev::{configured_or_discover, discover_input_device, find_touchscreen};
pub use gesture::{AodGestureHandler, GestureEvent, GestureHandler, GestureType};
pub use hal::TouchHal;
pub use panel_feature::start as start_panel_feature_service;
pub use procfs::{create_gesture_mask, default_gesture_mask, gesture_ids, ProcfsTouch};
pub use raise_to_wake::{RaiseToWakeEvent, RaiseToWakeManager};
pub use sysfs::SysfsManager;
pub use uinput::WakeInjector;
pub use xiaomi_touch::{XiaomiTouch, XiaomiTouchCmd, XiaomiTouchMode};

/// Main entry point for the touch HAL service.
pub async fn run_touch_hal(config: TouchConfig) -> TouchResult<()> {
    log::info!("Starting OPlus hardware touch service for topaz (Redmi Note 12 4G)");

    let hal = TouchHal::new(config).await?;
    hal.start_all().await?;

    // Android init sends SIGTERM; interactive runs normally use SIGINT.
    shutdown_signal().await?;
    log::info!("Shutdown signal received, stopping services...");
    hal.stop_all().await?;
    log::info!("Hardware touch service stopped gracefully");

    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() -> TouchResult<()> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn shutdown_signal() -> TouchResult<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}
