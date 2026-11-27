//! Always-on-display touch and panel management.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{debug, info, warn};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{interval, Duration};

use crate::aod_defaults::{self, AodDefaultsHandle};
use crate::config::{AodDefaultsConfig, AodKeepaliveConfig, TouchConfig};
use crate::doze::{self, DozeKeeperHandle};
use crate::error::TouchResult;
use crate::sysfs::SysfsManager;
use crate::xiaomi_touch::XiaomiTouch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AodState {
    Off,
    On,
    Dimmed,
    Seamless,
    FullScreen,
}

#[derive(Debug, Clone)]
struct AodConfig {
    enabled: bool,
    brightness: u8,
    full_screen: bool,
    seamless: bool,
    music_control: bool,
    keepalive: AodKeepaliveConfig,
    defaults: AodDefaultsConfig,
}

#[derive(Clone)]
pub struct AodManager {
    config: AodConfig,
    xiaomi_touch: Arc<XiaomiTouch>,
    sysfs: Arc<SysfsManager>,
    state: Arc<RwLock<AodState>>,
    desired_on: Arc<Mutex<bool>>,
    running: Arc<Mutex<bool>>,
    doze_running: Arc<AtomicBool>,
    defaults_running: Arc<AtomicBool>,
    doze_worker: Arc<Mutex<Option<DozeKeeperHandle>>>,
    defaults_worker: Arc<Mutex<Option<AodDefaultsHandle>>>,
}

impl AodManager {
    pub fn new(
        config: &TouchConfig,
        xiaomi_touch: Arc<XiaomiTouch>,
        sysfs: Arc<SysfsManager>,
    ) -> Self {
        let aod_config = AodConfig {
            enabled: config.enable_aod,
            brightness: config.aod_brightness,
            full_screen: config.enable_full_screen_aod,
            seamless: config.enable_seamless_aod,
            music_control: true,
            keepalive: config.aod_keepalive.clone(),
            defaults: config.aod_defaults.clone(),
        };

        Self {
            config: aod_config,
            xiaomi_touch,
            sysfs,
            state: Arc::new(RwLock::new(AodState::Off)),
            desired_on: Arc::new(Mutex::new(config.enable_aod)),
            running: Arc::new(Mutex::new(false)),
            doze_running: Arc::new(AtomicBool::new(false)),
            defaults_running: Arc::new(AtomicBool::new(false)),
            doze_worker: Arc::new(Mutex::new(None)),
            defaults_worker: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the monitor as a background task. This method deliberately does
    /// not await the infinite monitor loop.
    pub async fn start(&self) -> TouchResult<()> {
        {
            let mut running = self.running.lock().await;
            if *running {
                return Ok(());
            }
            *running = true;
        }

        if self.config.enabled {
            self.enable_aod().await?;
        }

        if self.config.keepalive.enabled {
            self.doze_running.store(true, Ordering::Release);
            let worker = doze::start(self.config.keepalive.clone(), self.doze_running.clone());
            *self.doze_worker.lock().await = Some(worker);
        }

        if self.config.defaults.enabled {
            self.defaults_running.store(true, Ordering::Release);
            let worker =
                aod_defaults::start(self.config.defaults.clone(), self.defaults_running.clone());
            *self.defaults_worker.lock().await = Some(worker);
        }

        let monitor = self.clone();
        tokio::spawn(async move {
            if let Err(error) = monitor.run_monitor_loop().await {
                log::error!("AOD monitor stopped: {error}");
            }
        });

        info!("AOD manager started");
        Ok(())
    }

    pub async fn stop(&self) -> TouchResult<()> {
        *self.running.lock().await = false;
        self.doze_running.store(false, Ordering::Release);
        self.defaults_running.store(false, Ordering::Release);
        if let Some(worker) = self.doze_worker.lock().await.take() {
            worker.stop();
        }
        if let Some(worker) = self.defaults_worker.lock().await.take() {
            worker.stop();
        }
        if self.config.enabled {
            self.disable_aod().await?;
        }
        info!("AOD manager stopped");
        Ok(())
    }

    /// Configure every available AOD interface. A missing optional node is
    /// logged rather than treated as fatal.
    pub async fn enable_aod(&self) -> TouchResult<()> {
        *self.desired_on.lock().await = true;
        if let Err(error) = self.xiaomi_touch.enable_aod(true).await {
            warn!("Could not enable AOD through xiaomi-touch: {error}");
        }
        if let Err(error) = self.sysfs.write_bool("aod_enable", true).await {
            warn!("Could not enable AOD through sysfs: {error}");
        }
        if let Err(error) = self.set_brightness(self.config.brightness).await {
            debug!("AOD brightness control unavailable: {error}");
        }
        if self.config.full_screen {
            if let Err(error) = self.enable_full_screen().await {
                debug!("Full-screen AOD panel node unavailable: {error}");
            }
        }
        if self.config.seamless {
            if let Err(error) = self.enable_seamless().await {
                debug!("Seamless AOD panel node unavailable: {error}");
            }
        }

        *self.state.write().await = AodState::On;
        info!(
            "AOD enabled (full_screen_requested={}, seamless_requested={})",
            self.config.full_screen, self.config.seamless
        );
        Ok(())
    }

    pub async fn disable_aod(&self) -> TouchResult<()> {
        *self.desired_on.lock().await = false;
        if let Err(error) = self.xiaomi_touch.enable_aod(false).await {
            debug!("xiaomi-touch AOD disable skipped: {error}");
        }
        if let Err(error) = self.sysfs.write_bool("aod_enable", false).await {
            debug!("sysfs AOD disable skipped: {error}");
        }
        *self.state.write().await = AodState::Off;
        info!("AOD disabled");
        Ok(())
    }

    pub async fn set_brightness(&self, brightness: u8) -> TouchResult<()> {
        self.sysfs.write_u8("aod_brightness", brightness).await?;
        debug!("AOD brightness set to {brightness}");
        Ok(())
    }

    pub async fn enable_full_screen(&self) -> TouchResult<()> {
        self.sysfs.write_bool("aod_full_screen", true).await?;
        *self.state.write().await = AodState::FullScreen;
        info!("Full-screen AOD enabled");
        Ok(())
    }

    pub async fn disable_full_screen(&self) -> TouchResult<()> {
        self.sysfs.write_bool("aod_full_screen", false).await?;
        *self.state.write().await = AodState::On;
        info!("Full-screen AOD disabled");
        Ok(())
    }

    pub async fn enable_seamless(&self) -> TouchResult<()> {
        self.sysfs.write_bool("aod_seamless", true).await?;
        *self.state.write().await = AodState::Seamless;
        info!("Seamless AOD enabled");
        Ok(())
    }

    pub async fn disable_seamless(&self) -> TouchResult<()> {
        self.sysfs.write_bool("aod_seamless", false).await?;
        *self.state.write().await = AodState::On;
        info!("Seamless AOD disabled");
        Ok(())
    }

    pub async fn dim(&self) -> TouchResult<()> {
        *self.state.write().await = AodState::Dimmed;
        self.set_brightness(10).await?;
        debug!("AOD dimmed");
        Ok(())
    }

    pub async fn undim(&self) -> TouchResult<()> {
        if *self.state.read().await == AodState::Dimmed {
            *self.state.write().await = AodState::On;
            self.set_brightness(self.config.brightness).await?;
            debug!("AOD undimmed");
        }
        Ok(())
    }

    pub async fn get_state(&self) -> AodState {
        *self.state.read().await
    }

    pub async fn is_enabled(&self) -> bool {
        matches!(
            *self.state.read().await,
            AodState::On | AodState::Dimmed | AodState::Seamless | AodState::FullScreen
        )
    }

    pub async fn on_proximity(&self, near: bool) -> TouchResult<()> {
        if near {
            self.dim().await
        } else {
            self.undim().await
        }
    }

    /// Wake the panel from AOD through the kernel wake node when supplied.
    /// GestureHandler owns all userspace key injection, so this method never
    /// emits a second KEY_WAKEUP.
    pub async fn on_dt2w(&self) -> TouchResult<()> {
        if !self.is_enabled().await {
            return Ok(());
        }

        info!("DT2W on AOD - waking screen");
        if let Err(error) = self.sysfs.write_bool("wake_screen", true).await {
            debug!("Configured touch wake node unavailable: {error}");
        }
        Ok(())
    }

    pub async fn on_music_double_tap(&self, x: i32, y: i32) -> TouchResult<bool> {
        if self.config.music_control && self.is_enabled().await {
            info!("AOD music/activity double tap at ({x}, {y})");
            self.sysfs.write_bool("aod_expand_music", true).await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn collapse_music_view(&self) -> TouchResult<()> {
        self.sysfs.write_bool("aod_expand_music", false).await?;
        Ok(())
    }

    async fn run_monitor_loop(&self) -> TouchResult<()> {
        let mut ticker = interval(Duration::from_secs(10));
        loop {
            ticker.tick().await;
            if !*self.running.lock().await {
                break;
            }
            let desired = *self.desired_on.lock().await;
            if desired {
                // The xiaomi touch mode is cheap to reassert and can be reset
                // by a framework/touch-driver suspend transition. Do not touch
                // the mi_display doze level here; the dedicated worker owns it.
                if let Err(error) = self.xiaomi_touch.enable_aod(true).await {
                    debug!("AOD touch-mode keepalive skipped: {error}");
                }
                let _ = self.sysfs.write_bool("aod_enable", true).await;
            }
        }
        Ok(())
    }
}
