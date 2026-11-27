//! Sysfs management for touch features

use log::{error, info};
use std::path::Path;
use tokio::fs;

use crate::config::SysfsConfig;
use crate::error::{TouchError, TouchResult};

/// Sysfs manager for touch-related sysfs entries
pub struct SysfsManager {
    config: SysfsConfig,
}

impl SysfsManager {
    pub fn new(config: SysfsConfig) -> Self {
        Self { config }
    }

    /// Write a boolean value to sysfs
    pub async fn write_bool(&self, name: &str, value: bool) -> TouchResult<()> {
        let path = self.get_path(name)?;
        let content = if value { "1" } else { "0" };
        self.write_file(&path, content).await
    }

    /// Write a u8 value to sysfs
    pub async fn write_u8(&self, name: &str, value: u8) -> TouchResult<()> {
        let path = self.get_path(name)?;
        let content = value.to_string();
        self.write_file(&path, &content).await
    }

    /// Write a u32 value to sysfs
    pub async fn write_u32(&self, name: &str, value: u32) -> TouchResult<()> {
        let path = self.get_path(name)?;
        let content = value.to_string();
        self.write_file(&path, &content).await
    }

    /// Write a string value to sysfs
    pub async fn write_string(&self, name: &str, value: &str) -> TouchResult<()> {
        let path = self.get_path(name)?;
        self.write_file(&path, value).await
    }

    /// Read a boolean value from sysfs
    pub async fn read_bool(&self, name: &str) -> TouchResult<bool> {
        let path = self.get_path(name)?;
        let content = self.read_file(&path).await?;
        Ok(content.trim() == "1")
    }

    /// Read a u8 value from sysfs
    pub async fn read_u8(&self, name: &str) -> TouchResult<u8> {
        let path = self.get_path(name)?;
        let content = self.read_file(&path).await?;
        content
            .trim()
            .parse()
            .map_err(|_| TouchError::Internal("Failed to parse u8".to_string()))
    }

    /// Read a u32 value from sysfs
    pub async fn read_u32(&self, name: &str) -> TouchResult<u32> {
        let path = self.get_path(name)?;
        let content = self.read_file(&path).await?;
        content
            .trim()
            .parse()
            .map_err(|_| TouchError::Internal("Failed to parse u32".to_string()))
    }

    /// Read a string value from sysfs
    pub async fn read_string(&self, name: &str) -> TouchResult<String> {
        let path = self.get_path(name)?;
        self.read_file(&path).await
    }

    /// Get the sysfs path for a feature
    fn get_path(&self, name: &str) -> TouchResult<std::path::PathBuf> {
        match name {
            "touch_mode" => Ok(self.config.touch_mode_path.clone()),
            "aod_enable" => Ok(self.config.aod_enable_path.clone()),
            "aod_brightness" => {
                if let Some(path) = &self.config.aod_brightness_path {
                    if path.exists() {
                        return Ok(path.clone());
                    }
                }
                let candidates = [
                    "/sys/class/touch/aod_brightness",
                    "/sys/class/touch/touch_dev/aod_brightness",
                ];
                if let Some(path) = candidates.iter().find(|path| Path::new(path).exists()) {
                    return Ok(std::path::PathBuf::from(path));
                }
                return Err(TouchError::NotSupported(
                    "no AOD brightness node found".to_string(),
                ));
            }
            "aod_full_screen" => {
                optional_path(&self.config.aod_full_screen_path, "full-screen AOD")
            }
            "aod_seamless" => optional_path(&self.config.aod_seamless_path, "seamless AOD"),
            "aod_expand_music" => {
                optional_path(&self.config.aod_expand_music_path, "AOD music expansion")
            }
            "wake_screen" => optional_path(&self.config.wake_screen_path, "touch wake node"),
            "dt2w_enable" => Ok(self.config.dt2w_enable_path.clone()),
            "gesture_enable" => Ok(self.config.gesture_enable_path.clone()),
            "panel_vendor" => Ok(self.config.panel_vendor_path.clone()),
            "fod_enable" => Ok(self.config.fod_enable_path.clone()),
            "report_rate" => Ok(self.config.report_rate_path.clone()),
            "game_mode" => Ok(self.config.game_mode_path.clone()),
            "raise_to_wake_enable" => {
                optional_path(&self.config.raise_to_wake_enable_path, "raise-to-wake")
            }
            _ => Err(TouchError::Config(format!("Unknown sysfs entry: {}", name))),
        }
    }

    /// Write content to a file
    async fn write_file(&self, path: &Path, content: &str) -> TouchResult<()> {
        fs::write(path, content).await.map_err(|e| {
            error!("Failed to write {} to {}: {}", content, path.display(), e);
            TouchError::Io(e)
        })
    }

    /// Read content from a file
    async fn read_file(&self, path: &Path) -> TouchResult<String> {
        fs::read_to_string(path).await.map_err(|e| {
            error!("Failed to read {}: {}", path.display(), e);
            TouchError::Io(e)
        })
    }

    /// Check if sysfs entry exists
    pub async fn exists(&self, name: &str) -> bool {
        if let Ok(path) = self.get_path(name) {
            path.exists()
        } else {
            false
        }
    }

    /// Initialize all sysfs entries with default values
    pub async fn initialize(&self) -> TouchResult<()> {
        // Enable touch mode
        self.write_string("touch_mode", "normal").await.ok();

        // Enable DT2W
        self.write_bool("dt2w_enable", true).await.ok();

        // Enable gestures
        self.write_bool("gesture_enable", true).await.ok();

        // Set report rate to 60Hz default
        self.write_u32("report_rate", 60).await.ok();

        // Enable FOD
        self.write_bool("fod_enable", true).await.ok();

        // Read panel vendor
        let _ = self.read_string("panel_vendor").await;

        info!("Sysfs entries initialized");
        Ok(())
    }

    /// Set touch mode (normal, game, glove, etc.)
    pub async fn set_touch_mode(&self, mode: &str) -> TouchResult<()> {
        self.write_string("touch_mode", mode).await
    }

    /// Get touch mode
    pub async fn get_touch_mode(&self) -> TouchResult<String> {
        self.read_string("touch_mode").await
    }

    /// Set report rate
    pub async fn set_report_rate(&self, rate: u32) -> TouchResult<()> {
        self.write_u32("report_rate", rate).await
    }

    /// Get report rate
    pub async fn get_report_rate(&self) -> TouchResult<u32> {
        self.read_u32("report_rate").await
    }

    /// Enable/disable game mode
    pub async fn set_game_mode(&self, enable: bool) -> TouchResult<()> {
        self.write_bool("game_mode", enable).await
    }

    /// Get panel vendor info
    pub async fn get_panel_vendor(&self) -> TouchResult<String> {
        self.read_string("panel_vendor").await
    }

    /// Trigger wake screen
    pub async fn wake_screen(&self) -> TouchResult<()> {
        self.write_bool("wake_screen", true).await?;
        // Auto-reset after short delay
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        self.write_bool("wake_screen", false).await
    }
}

fn optional_path(
    path: &Option<std::path::PathBuf>,
    feature: &str,
) -> TouchResult<std::path::PathBuf> {
    path.clone().ok_or_else(|| {
        TouchError::NotSupported(format!(
            "{feature} is not exposed by this kernel; configure its sysfs path if present"
        ))
    })
}
