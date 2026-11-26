//! Main entry point for the touch HAL daemon

use clap::Parser;
use log::{info, warn};

use oplus_hardware_touch_service::{
    procfs, raise_to_wake, run_touch_hal, start_panel_feature_service, xiaomi_touch, TouchConfig,
    TouchResult,
};

#[derive(Parser, Debug)]
#[command(name = "oplus-hardware-touch-service")]
#[command(about = "OPlus hardware touch service for Redmi Note 12 4G (topaz/SM6225)")]
struct Args {
    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Run in foreground (don't daemonize)
    #[arg(short, long)]
    foreground: bool,

    /// Test mode - run self tests
    #[arg(long)]
    test: bool,

    /// List Android sensors and print pickup events for on-device diagnosis
    #[arg(long)]
    diagnose_sensors: bool,

    /// Duration of --diagnose-sensors capture
    #[arg(long, default_value_t = 30)]
    sensor_seconds: u64,
}

#[tokio::main]
async fn main() -> TouchResult<()> {
    let args = Args::parse();

    let log_level = args.log_level.parse().unwrap_or(log::LevelFilter::Info);
    init_logging(log_level);

    if args.test {
        return run_tests().await;
    }

    if args.diagnose_sensors {
        return raise_to_wake::diagnose_sensors(args.sensor_seconds);
    }

    info!("OPlus hardware touch service starting...");
    info!("Device: Redmi Note 12 4G (topaz/SM6225)");

    // The configuration is compiled into the HAL. There is no runtime
    // config file and no writable vendor dependency.
    let config = TouchConfig::default();

    if !args.foreground {
        info!("Running as a managed foreground service");
    }

    if config.enable_aod && config.aod_panel_service.enabled {
        if let Err(error) = start_panel_feature_service(config.aod_panel_service.clone()) {
            warn!("Display panel capability service unavailable: {error}");
        }
    }

    // Run the touch HAL
    run_touch_hal(config).await
}

#[cfg(target_os = "android")]
fn init_logging(level: log::LevelFilter) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("oplus-touch-service")
            .with_max_level(level),
    );
}

#[cfg(not(target_os = "android"))]
fn init_logging(level: log::LevelFilter) {
    env_logger::Builder::new()
        .filter_level(level)
        .format_timestamp_millis()
        .init();
}

async fn run_tests() -> TouchResult<()> {
    info!("Running self-tests...");

    // Test 1: Standalone compiled-in defaults
    let config = TouchConfig::default();
    assert!(config.enable_aod);
    assert!(config.enable_dt2w);
    assert!(config.aod_keepalive.enabled);
    info!("✓ Standalone config defaults test passed");

    // Test 2: Xiaomi touch mode enums
    use xiaomi_touch::XiaomiTouchMode;
    assert_eq!(XiaomiTouchMode::AodEnable as u32, 11);
    assert_eq!(XiaomiTouchMode::DoubletapMode as u32, 14);
    assert_eq!(XiaomiTouchMode::FodEnable as u32, 10);
    info!("✓ Mode enum test passed");

    // Test 3: Gesture IDs
    use procfs::gesture_ids;
    assert_eq!(gesture_ids::DOUBLE_TAP, 0x22);
    assert_eq!(gesture_ids::SWIPE_UP, 0x24);
    info!("✓ Gesture IDs test passed");

    // Test 4: /proc/tp_gesture boolean arming. The topaz driver takes 1/0,
    // not a bitmap containing the gesture IDs.
    let node = std::env::temp_dir().join(format!(
        "oplus-hardware-touch-service-selftest-tp-gesture-{}",
        std::process::id()
    ));
    std::fs::write(&node, "0\n")?;
    let touch = procfs::ProcfsTouch::new(&node.to_string_lossy(), "/nonexistent");
    touch.set_gesture_mask(u64::MAX)?;
    assert_eq!(std::fs::read_to_string(&node)?.trim(), "1");
    touch.set_gesture_mask(0)?;
    assert_eq!(std::fs::read_to_string(&node)?.trim(), "0");
    let _ = std::fs::remove_file(node);
    info!("✓ tp_gesture boolean arming test passed");

    info!("All self-tests passed!");
    Ok(())
}
