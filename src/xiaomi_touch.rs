//! Xiaomi touch misc-device ABI (`/dev/xiaomi-touch`).

use std::os::unix::io::RawFd;
use std::sync::Arc;

use log::{debug, info, warn};
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::close;
use tokio::sync::Mutex;

use crate::error::{TouchError, TouchResult};

/// `XIAOMI_TOUCH_IOC_*` values are `_IO('T', command)` on arm64.
const XIAOMI_TOUCH_MAGIC: u32 = b'T' as u32;
const IOC_TYPESHIFT: u32 = 8;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XiaomiTouchCmd {
    SetCurValue = 0,
    GetCurValue = 1,
    GetDefValue = 2,
    GetMinValue = 3,
    GetMaxValue = 4,
    GetModeValue = 5,
    ResetMode = 6,
    SetLongValue = 7,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XiaomiTouchMode {
    GameMode = 0,
    ActiveMode = 1,
    UpThreshold = 2,
    Tolerance = 3,
    WghMin = 4,
    WghMax = 5,
    WghStep = 6,
    EdgeFilter = 7,
    PanelOrientation = 8,
    ReportRate = 9,
    FodEnable = 10,
    AodEnable = 11,
    ResistRf = 12,
    IdleTime = 13,
    DoubletapMode = 14,
    GripMode = 15,
    ModeNum = 16,
}

const MAX_BUF_SIZE: usize = 256;
const VALUE_TYPE_SIZE: usize = 6;
type ModeCache = Arc<Mutex<[[i32; VALUE_TYPE_SIZE]; XiaomiTouchMode::ModeNum as usize]>>;

/// Xiaomi touch misc-device wrapper.
pub struct XiaomiTouch {
    fd: RawFd,
    path: String,
    mode_cache: ModeCache,
}

impl XiaomiTouch {
    /// Open `/dev/xiaomi-touch`.
    pub fn open(path: &str) -> TouchResult<Self> {
        let fd = open(path, OFlag::O_RDWR, Mode::empty())
            .map_err(|e| TouchError::Io(std::io::Error::from(e)))?;
        info!("Opened xiaomi-touch device at {path}");
        Ok(Self::from_fd(fd, path))
    }

    /// Create a harmless stand-in when the optional misc device is absent.
    pub fn unavailable(path: &str) -> Self {
        warn!("xiaomi-touch device is unavailable at {path}");
        Self::from_fd(-1, path)
    }

    pub fn open_optional(path: &str) -> Self {
        match Self::open(path) {
            Ok(device) => device,
            Err(error) => {
                warn!("Cannot open {path}: {error}; continuing with procfs/sysfs");
                Self::unavailable(path)
            }
        }
    }

    fn from_fd(fd: RawFd, path: &str) -> Self {
        let mut mode_cache = [[0i32; VALUE_TYPE_SIZE]; XiaomiTouchMode::ModeNum as usize];
        for values in &mut mode_cache {
            values[0] = 1; // current
            values[1] = 1; // default
            values[2] = 0; // minimum
            values[3] = 1; // maximum
        }

        mode_cache[XiaomiTouchMode::AodEnable as usize] = [1, 1, 0, 1, 0, 0];
        mode_cache[XiaomiTouchMode::DoubletapMode as usize] = [1, 1, 0, 1, 0, 0];
        mode_cache[XiaomiTouchMode::FodEnable as usize] = [1, 1, 0, 1, 0, 0];
        mode_cache[XiaomiTouchMode::ReportRate as usize] = [60, 60, 30, 120, 0, 0];

        Self {
            fd,
            path: path.to_string(),
            mode_cache: Arc::new(Mutex::new(mode_cache)),
        }
    }

    fn ioctl_request(cmd: XiaomiTouchCmd) -> libc::c_ulong {
        (XIAOMI_TOUCH_MAGIC << IOC_TYPESHIFT | cmd as u32) as libc::c_ulong
    }

    /// Send a real xiaomi-touch ioctl. The kernel ABI uses an `int[256]`
    /// where `buf[0]` is the mode and `buf[1]` is the value.
    pub async fn ioctl(
        &self,
        cmd: XiaomiTouchCmd,
        mode: XiaomiTouchMode,
        value: i32,
    ) -> TouchResult<i32> {
        if self.fd < 0 {
            return Err(TouchError::NotSupported(format!(
                "{} is not available",
                self.path
            )));
        }

        let mut buffer = [0i32; MAX_BUF_SIZE];
        buffer[0] = mode as i32;
        buffer[1] = value;

        let result = unsafe {
            libc::ioctl(
                self.fd,
                Self::ioctl_request(cmd) as _,
                buffer.as_mut_ptr() as *mut libc::c_void,
            )
        };
        if result < 0 {
            return Err(TouchError::Io(std::io::Error::last_os_error()));
        }

        if cmd == XiaomiTouchCmd::SetCurValue || cmd == XiaomiTouchCmd::ResetMode {
            let mut cache = self.mode_cache.lock().await;
            cache[mode as usize][0] = buffer[1];
        }
        debug!(
            "xiaomi-touch ioctl cmd={cmd:?} mode={mode:?} value={value} result={}",
            buffer[1]
        );
        Ok(buffer[1])
    }

    pub async fn get_mode(&self, mode: XiaomiTouchMode) -> TouchResult<i32> {
        self.ioctl(XiaomiTouchCmd::GetCurValue, mode, 0).await
    }

    pub async fn set_mode(&self, mode: XiaomiTouchMode, value: i32) -> TouchResult<i32> {
        self.ioctl(XiaomiTouchCmd::SetCurValue, mode, value).await
    }

    pub async fn enable_aod(&self, enable: bool) -> TouchResult<()> {
        self.set_mode(XiaomiTouchMode::AodEnable, i32::from(enable))
            .await?;
        info!("AOD {}", if enable { "enabled" } else { "disabled" });
        Ok(())
    }

    pub async fn enable_dt2w(&self, enable: bool) -> TouchResult<()> {
        self.set_mode(XiaomiTouchMode::DoubletapMode, i32::from(enable))
            .await?;
        info!("DT2W {}", if enable { "enabled" } else { "disabled" });
        Ok(())
    }

    pub async fn enable_fod(&self, enable: bool) -> TouchResult<()> {
        self.set_mode(XiaomiTouchMode::FodEnable, i32::from(enable))
            .await?;
        info!("FOD {}", if enable { "enabled" } else { "disabled" });
        Ok(())
    }

    pub async fn set_report_rate(&self, rate: i32) -> TouchResult<()> {
        self.set_mode(XiaomiTouchMode::ReportRate, rate).await?;
        info!("Report rate set to {rate}");
        Ok(())
    }

    pub async fn set_game_mode(&self, enable: bool) -> TouchResult<()> {
        self.set_mode(XiaomiTouchMode::GameMode, i32::from(enable))
            .await?;
        info!("Game mode {}", if enable { "enabled" } else { "disabled" });
        Ok(())
    }

    pub async fn get_all_modes(
        &self,
    ) -> TouchResult<[[i32; VALUE_TYPE_SIZE]; XiaomiTouchMode::ModeNum as usize]> {
        let cache = self.mode_cache.lock().await;
        Ok(*cache)
    }

    pub fn is_available(&self) -> bool {
        self.fd >= 0
    }
}

impl Drop for XiaomiTouch {
    fn drop(&mut self) {
        if self.fd >= 0 {
            let _ = close(self.fd);
            debug!("Closed xiaomi-touch device");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_request_uses_xiaomi_io_macro_layout() {
        assert_eq!(
            XiaomiTouch::ioctl_request(XiaomiTouchCmd::SetCurValue),
            0x5400
        );
        assert_eq!(
            XiaomiTouch::ioctl_request(XiaomiTouchCmd::SetLongValue),
            0x5407
        );
    }

    #[test]
    fn abi_mode_numbers_are_stable() {
        assert_eq!(XiaomiTouchMode::AodEnable as u32, 11);
        assert_eq!(XiaomiTouchMode::DoubletapMode as u32, 14);
        assert_eq!(XiaomiTouchMode::ModeNum as u32, 16);
    }
}
