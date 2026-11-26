//! Procfs interface for /proc/tp_gesture and /proc/xiaomi-touch

use log::debug;
use std::fs;
use std::path::Path;

use crate::error::{TouchError, TouchResult};

/// Procfs touch interface
pub struct ProcfsTouch {
    tp_gesture_path: String,
    xiaomi_touch_path: String,
}

impl ProcfsTouch {
    pub fn new(tp_gesture_path: &str, xiaomi_touch_path: &str) -> Self {
        Self {
            tp_gesture_path: tp_gesture_path.to_string(),
            xiaomi_touch_path: xiaomi_touch_path.to_string(),
        }
    }

    /// Write gesture to tp_gesture
    pub fn write_gesture(&self, gesture: &str) -> TouchResult<()> {
        fs::write(&self.tp_gesture_path, format!("{gesture}\n")).map_err(TouchError::Io)?;
        debug!("Wrote gesture to tp_gesture: {}", gesture);
        Ok(())
    }

    /// Read gesture from tp_gesture
    pub fn read_gesture(&self) -> TouchResult<String> {
        let content = fs::read_to_string(&self.tp_gesture_path).map_err(|e| TouchError::Io(e))?;
        Ok(content.trim().to_string())
    }

    /// Enable DT2W via tp_gesture
    pub fn enable_dt2w(&self) -> TouchResult<()> {
        self.write_gesture("1")
    }

    /// Disable DT2W via tp_gesture
    pub fn disable_dt2w(&self) -> TouchResult<()> {
        self.write_gesture("0")
    }

    /// Arm the gesture engine. Novatek/Xiaomi `/proc/tp_gesture` nodes take
    /// one boolean, not the gesture IDs themselves.
    pub fn set_gesture_mask(&self, mask: u64) -> TouchResult<()> {
        self.write_gesture(if mask == 0 { "0" } else { "1" })
    }

    /// Get current gesture mask
    pub fn get_gesture_mask(&self) -> TouchResult<u64> {
        let content = self.read_gesture()?;
        content
            .trim()
            .parse()
            .map_err(|_| TouchError::Internal("Failed to parse gesture mask".to_string()))
    }

    /// Read xiaomi-touch procfs
    pub fn read_xiaomi_touch(&self) -> TouchResult<String> {
        fs::read_to_string(&self.xiaomi_touch_path).map_err(TouchError::Io)
    }

    /// Write xiaomi-touch procfs
    pub fn write_xiaomi_touch(&self, data: &str) -> TouchResult<()> {
        fs::write(&self.xiaomi_touch_path, data).map_err(TouchError::Io)
    }

    /// Check if tp_gesture exists
    pub fn tp_gesture_exists(&self) -> bool {
        Path::new(&self.tp_gesture_path).exists()
    }

    /// Check if xiaomi-touch procfs exists
    pub fn xiaomi_touch_exists(&self) -> bool {
        Path::new(&self.xiaomi_touch_path).exists()
    }

    /// Parse gesture info from tp_gesture
    pub fn parse_gesture_info(&self) -> TouchResult<GestureInfo> {
        let content = self.read_gesture()?;
        parse_gesture_info(&content)
    }
}

/// Parsed gesture information
#[derive(Debug, Clone)]
pub struct GestureInfo {
    pub gesture_id: u8,
    pub x: i32,
    pub y: i32,
    pub timestamp: u64,
    pub extra: String,
}

fn parse_gesture_info(content: &str) -> TouchResult<GestureInfo> {
    // Format varies by kernel, try common formats
    // Format 1: "gesture_id x y"
    // Format 2: "gesture_id x y timestamp"
    // Format 3: JSON

    let parts: Vec<&str> = content.split_whitespace().collect();

    if parts.len() >= 3 {
        let gesture_id = parts[0].parse().unwrap_or(0);
        let x = parts[1].parse().unwrap_or(0);
        let y = parts[2].parse().unwrap_or(0);
        let timestamp = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let extra = parts.get(4..).map(|s| s.join(" ")).unwrap_or_default();

        return Ok(GestureInfo {
            gesture_id,
            x,
            y,
            timestamp,
            extra,
        });
    }

    // Try JSON format
    if content.trim_start().starts_with('{') {
        return Ok(GestureInfo {
            gesture_id: 0,
            x: 0,
            y: 0,
            timestamp: 0,
            extra: content.to_string(),
        });
    }

    Err(TouchError::Internal(
        "Failed to parse gesture info".to_string(),
    ))
}

/// Gesture IDs for tp_gesture (common values)
pub mod gesture_ids {
    pub const DOUBLE_TAP: u8 = 0x22;
    pub const SINGLE_TAP: u8 = 0x23;
    pub const SWIPE_UP: u8 = 0x24;
    pub const SWIPE_DOWN: u8 = 0x25;
    pub const SWIPE_LEFT: u8 = 0x26;
    pub const SWIPE_RIGHT: u8 = 0x27;
    pub const LONG_PRESS: u8 = 0x28;
    pub const DOUBLE_SWIPE: u8 = 0x29;
    pub const HEART_RATE: u8 = 0x30;
    pub const FOD_PRESS: u8 = 0x31;
    pub const FOD_RELEASE: u8 = 0x32;
}

/// Helper to create gesture mask from enabled gestures
pub fn create_gesture_mask(gestures: &[u8]) -> u64 {
    let mut mask = 0u64;
    for &g in gestures {
        if g < 64 {
            mask |= 1u64 << g;
        }
    }
    mask
}

/// Default gesture mask for DT2W + common gestures
pub fn default_gesture_mask() -> u64 {
    create_gesture_mask(&[
        gesture_ids::DOUBLE_TAP,
        gesture_ids::SWIPE_UP,
        gesture_ids::SWIPE_DOWN,
        gesture_ids::SWIPE_LEFT,
        gesture_ids::SWIPE_RIGHT,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_handles_high_gesture_ids_without_overflow() {
        let mask = create_gesture_mask(&[gesture_ids::DOUBLE_TAP, gesture_ids::FOD_RELEASE]);
        assert_eq!(
            mask,
            (1u64 << gesture_ids::DOUBLE_TAP) | (1u64 << gesture_ids::FOD_RELEASE)
        );
    }

    #[test]
    fn parses_kernel_gesture_tuple() {
        let info = parse_gesture_info("34 540 1200 99").expect("parse gesture");
        assert_eq!(info.gesture_id, 34);
        assert_eq!(info.x, 540);
        assert_eq!(info.y, 1200);
        assert_eq!(info.timestamp, 99);
    }

    #[test]
    fn tp_gesture_uses_boolean_arming_value() {
        let path = std::env::temp_dir().join(format!(
            "oplus-hardware-touch-service-tp-gesture-{}-{}",
            std::process::id(),
            now_nonce()
        ));
        std::fs::write(&path, "0\n").expect("create test node");

        let node = ProcfsTouch::new(&path.to_string_lossy(), "/nonexistent");
        node.set_gesture_mask(1u64 << gesture_ids::DOUBLE_TAP)
            .expect("arm gestures");
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "1");

        node.set_gesture_mask(0).expect("disarm gestures");
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "0");

        let _ = std::fs::remove_file(path);
    }

    fn now_nonce() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }
}
