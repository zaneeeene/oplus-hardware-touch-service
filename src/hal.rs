//! Main touch-HAL coordinator.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{debug, info, warn};
use tokio::sync::Mutex;

use crate::aod::{AodManager, AodState};
use crate::config::TouchConfig;
use crate::error::TouchResult;
use crate::gesture::{AodGestureHandler, GestureHandler};
use crate::procfs::ProcfsTouch;
use crate::raise_to_wake::RaiseToWakeManager;
use crate::sysfs::SysfsManager;
use crate::xiaomi_touch::XiaomiTouch;

pub struct TouchHal {
    config: TouchConfig,
    xiaomi_touch: Arc<XiaomiTouch>,
    sysfs: Arc<SysfsManager>,
    procfs: Arc<ProcfsTouch>,
    gesture_handler: Arc<Mutex<Option<GestureHandler>>>,
    aod_gesture_handler: Arc<Mutex<Option<AodGestureHandler>>>,
    aod_manager: Arc<Mutex<Option<Arc<AodManager>>>>,
    raise_to_wake: Arc<Mutex<Option<RaiseToWakeManager>>>,
    dt2w_enabled: Arc<AtomicBool>,
    running: Arc<Mutex<bool>>,
}

impl TouchHal {
    pub async fn new(config: TouchConfig) -> TouchResult<Self> {
        info!("Initializing Touch HAL for topaz (Redmi Note 12 4G)");

        let xiaomi_touch = Arc::new(XiaomiTouch::open_optional(
            &config.xiaomi_touch_path.to_string_lossy(),
        ));
        let sysfs = Arc::new(SysfsManager::new(config.sysfs.clone()));
        if let Err(error) = sysfs.initialize().await {
            log::debug!("sysfs initialization incomplete: {error}");
        }
        let procfs = Arc::new(ProcfsTouch::new(
            &config.tp_gesture_path.to_string_lossy(),
            "/proc/xiaomi-touch",
        ));

        if !xiaomi_touch.is_available() {
            warn!("Continuing without /dev/xiaomi-touch");
        }
        if !procfs.tp_gesture_exists() {
            warn!(
                "tp_gesture not found at {}",
                config.tp_gesture_path.display()
            );
        }
        if !procfs.xiaomi_touch_exists() {
            debug!("/proc/xiaomi-touch not present");
        }

        let dt2w_enabled = Arc::new(AtomicBool::new(config.enable_dt2w));
        Ok(Self {
            config,
            xiaomi_touch,
            sysfs,
            procfs,
            gesture_handler: Arc::new(Mutex::new(None)),
            aod_gesture_handler: Arc::new(Mutex::new(None)),
            aod_manager: Arc::new(Mutex::new(None)),
            raise_to_wake: Arc::new(Mutex::new(None)),
            dt2w_enabled,
            running: Arc::new(Mutex::new(false)),
        })
    }

    pub async fn start_all(&self) -> TouchResult<()> {
        *self.running.lock().await = true;
        info!("Starting all touch services");

        // Gestures must be installed before the AOD manager so the AOD
        // expand/collapse handler can receive events.
        if self.config.enable_gestures || self.config.enable_dt2w {
            self.start_gestures().await?;
        }

        if self.config.enable_aod || self.config.enable_raise_to_wake {
            self.start_aod().await?;
        }

        if self.config.enable_raise_to_wake {
            self.start_raise_to_wake().await?;
        }

        if let Err(error) = self.xiaomi_touch.enable_aod(self.config.enable_aod).await {
            warn!("xiaomi-touch AOD setup skipped: {error}");
        }
        if let Err(error) = self.xiaomi_touch.enable_dt2w(self.config.enable_dt2w).await {
            warn!("xiaomi-touch DT2W setup skipped: {error}");
        }
        if let Err(error) = self.xiaomi_touch.enable_fod(true).await {
            debug!("xiaomi-touch FOD setup skipped: {error}");
        }
        if let Err(error) = self.xiaomi_touch.set_report_rate(60).await {
            debug!("xiaomi-touch report-rate setup skipped: {error}");
        }

        info!("All touch services started");
        Ok(())
    }

    pub async fn stop_all(&self) -> TouchResult<()> {
        *self.running.lock().await = false;
        info!("Stopping all touch services");

        if let Some(rtw) = self.raise_to_wake.lock().await.take() {
            let _ = rtw.stop().await;
        }
        if let Some(aod) = self.aod_manager.lock().await.take() {
            let _ = aod.stop().await;
        }
        if let Some(gesture) = self.gesture_handler.lock().await.take() {
            let _ = gesture.stop().await;
        }
        if let Err(error) = self.xiaomi_touch.enable_dt2w(false).await {
            debug!("xiaomi-touch DT2W disable skipped: {error}");
        }
        if let Err(error) = self.procfs.disable_dt2w() {
            debug!("Could not disarm tp_gesture: {error}");
        }

        info!("All touch services stopped");
        Ok(())
    }

    async fn start_aod(&self) -> TouchResult<()> {
        let aod = Arc::new(AodManager::new(
            &self.config,
            self.xiaomi_touch.clone(),
            self.sysfs.clone(),
        ));
        aod.start().await?;

        if let Some(gesture) = self.gesture_handler.lock().await.clone() {
            let aod_gesture = AodGestureHandler::new(Arc::new(gesture.clone()));
            *self.aod_gesture_handler.lock().await = Some(aod_gesture.clone());

            let mut events = gesture.subscribe();
            let aod_for_task = aod.clone();
            tokio::spawn(async move {
                while let Ok(event) = events.recv().await {
                    if event.gesture_type == crate::gesture::GestureType::DoubleTap
                        && aod_for_task.is_enabled().await
                    {
                        let _ = aod_for_task.on_music_double_tap(event.x, event.y).await;
                        let _ = aod_gesture.handle_aod_double_tap(event.x, event.y).await;
                    }
                }
            });
        }
        *self.aod_manager.lock().await = Some(aod);
        Ok(())
    }

    async fn start_gestures(&self) -> TouchResult<()> {
        let mut gestures = self.config.gestures.clone();
        if !self.config.enable_dt2w {
            gestures.double_tap.enabled = false;
        }
        let gesture = GestureHandler::new(
            gestures,
            self.xiaomi_touch.clone(),
            &self.config.touchscreen_path.to_string_lossy(),
            &self.config.tp_gesture_path.to_string_lossy(),
            self.config.enable_uinput_wake,
            self.dt2w_enabled.clone(),
        )?;
        gesture.start().await?;
        *self.gesture_handler.lock().await = Some(gesture);
        Ok(())
    }

    async fn start_raise_to_wake(&self) -> TouchResult<()> {
        let rtw = RaiseToWakeManager::new(self.config.raise_to_wake.clone(), self.sysfs.clone())?;
        rtw.start().await?;
        *self.raise_to_wake.lock().await = Some(rtw);
        Ok(())
    }

    pub async fn aod_manager(&self) -> Arc<Mutex<Option<Arc<AodManager>>>> {
        self.aod_manager.clone()
    }

    pub async fn gesture_handler(&self) -> Arc<Mutex<Option<GestureHandler>>> {
        self.gesture_handler.clone()
    }

    pub async fn raise_to_wake(&self) -> Arc<Mutex<Option<RaiseToWakeManager>>> {
        self.raise_to_wake.clone()
    }

    pub async fn handle_kernel_gesture(&self, gesture_id: u8) -> TouchResult<()> {
        if let Some(gesture) = self.gesture_handler.lock().await.as_ref() {
            gesture.handle_kernel_gesture(gesture_id).await?;
        }
        if gesture_id == crate::procfs::gesture_ids::DOUBLE_TAP {
            if let Some(aod_gesture) = self.aod_gesture_handler.lock().await.as_ref() {
                aod_gesture.handle_aod_double_tap(0, 0).await?;
            }
        }
        Ok(())
    }

    pub async fn handle_proximity(&self, near: bool) -> TouchResult<()> {
        if let Some(aod) = self.aod_manager.lock().await.as_ref() {
            aod.on_proximity(near).await?;
        }
        Ok(())
    }

    pub async fn get_aod_state(&self) -> AodState {
        if let Some(aod) = self.aod_manager.lock().await.as_ref() {
            aod.get_state().await
        } else {
            AodState::Off
        }
    }

    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    pub fn xiaomi_touch(&self) -> Arc<XiaomiTouch> {
        self.xiaomi_touch.clone()
    }

    pub fn sysfs(&self) -> Arc<SysfsManager> {
        self.sysfs.clone()
    }

    pub fn procfs(&self) -> Arc<ProcfsTouch> {
        self.procfs.clone()
    }

    pub async fn set_aod_brightness(&self, brightness: u8) -> TouchResult<()> {
        if let Some(aod) = self.aod_manager.lock().await.as_ref() {
            aod.set_brightness(brightness).await?;
        }
        Ok(())
    }

    pub async fn set_dt2w(&self, enable: bool) -> TouchResult<()> {
        self.dt2w_enabled.store(enable, Ordering::Release);
        self.xiaomi_touch.enable_dt2w(enable).await?;
        if enable {
            self.procfs.enable_dt2w()?;
        } else {
            self.procfs.disable_dt2w()?;
        }
        Ok(())
    }

    pub async fn set_aod(&self, enable: bool) -> TouchResult<()> {
        if let Some(aod) = self.aod_manager.lock().await.as_ref() {
            if enable {
                aod.enable_aod().await?;
            } else {
                aod.disable_aod().await?;
            }
        }
        Ok(())
    }

    pub async fn set_full_screen_aod(&self, enable: bool) -> TouchResult<()> {
        if let Some(aod) = self.aod_manager.lock().await.as_ref() {
            if enable {
                aod.enable_full_screen().await?;
            } else {
                aod.disable_full_screen().await?;
            }
        }
        Ok(())
    }

    pub async fn set_seamless_aod(&self, enable: bool) -> TouchResult<()> {
        if let Some(aod) = self.aod_manager.lock().await.as_ref() {
            if enable {
                aod.enable_seamless().await?;
            } else {
                aod.disable_seamless().await?;
            }
        }
        Ok(())
    }

    pub async fn set_game_mode(&self, enable: bool) -> TouchResult<()> {
        self.xiaomi_touch.set_game_mode(enable).await?;
        self.sysfs.set_game_mode(enable).await?;
        Ok(())
    }

    pub async fn set_report_rate(&self, rate: u32) -> TouchResult<()> {
        self.xiaomi_touch.set_report_rate(rate as i32).await?;
        self.sysfs.set_report_rate(rate).await?;
        Ok(())
    }

    pub async fn get_all_modes(&self) -> TouchResult<[[i32; 6]; 16]> {
        self.xiaomi_touch.get_all_modes().await
    }
}
