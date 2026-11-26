//! Linux input-device discovery.

use std::path::{Path, PathBuf};

/// Return the configured device when usable, otherwise discover an input
/// event node whose kernel name or handler matches one of `keywords`.
pub fn configured_or_discover(configured: &Path, keywords: &[&str]) -> Option<PathBuf> {
    if configured.exists() {
        return Some(configured.to_path_buf());
    }
    discover_input_device(keywords)
}

/// Inspect `/proc/bus/input/devices` and return the first matching event node.
pub fn discover_input_device(keywords: &[&str]) -> Option<PathBuf> {
    let table = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    let keywords: Vec<String> = keywords.iter().map(|s| s.to_ascii_lowercase()).collect();

    for block in table.split("\n\n") {
        let lower = block.to_ascii_lowercase();
        if !keywords.iter().any(|keyword| lower.contains(keyword)) {
            continue;
        }

        for line in block.lines() {
            let Some(handlers) = line.strip_prefix("H: Handlers=") else {
                continue;
            };
            for handler in handlers.split_whitespace() {
                if handler.starts_with("event") {
                    let path = PathBuf::from("/dev/input").join(handler);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

pub fn find_touchscreen() -> Option<PathBuf> {
    discover_input_device(&[
        "touchscreen",
        "touch screen",
        "touch_dev",
        "nt36",
        "focaltech",
        "ft3",
        "ft5",
        "goodix",
        "synaptics",
        "sec_ts",
        "ili2",
        "ili9",
    ])
}

pub fn find_accelerometer() -> Option<PathBuf> {
    discover_input_device(&[
        "accelerometer",
        "accel_3d",
        "accelerometer_3d",
        "gsensor",
        "g-sensor",
        "kxtj",
        "bmi1",
        "bmi2",
        "bma2",
        "lis2",
        "lsm6",
        "icm4",
        "mpu6",
        "sc7a",
    ])
}
