//! Minimal Linux uinput key injector for wake and OPlus AOD gestures.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::error::{TouchError, TouchResult};

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const SYN_REPORT: u16 = 0x00;
const KEY_WAKEUP: u16 = 143;
/// Linux KEY_F4. Android maps this to KEYCODE_F4 (134), which is the
/// screen-off gesture dispatch key used by OPlus SystemUI.
const KEY_F4: u16 = 62;

const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UInputUserDev {
    name: [libc::c_char; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

/// Small virtual input device used to wake Android without spawning
/// `/system/bin/input`.
pub struct WakeInjector {
    fd: OwnedFd,
}

impl WakeInjector {
    pub fn open(name: &str) -> TouchResult<Self> {
        let path = CString::new("/dev/uinput")
            .map_err(|_| TouchError::Internal("invalid uinput path".to_string()))?;
        let raw = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if raw < 0 {
            return Err(TouchError::Io(std::io::Error::last_os_error()));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };

        set_bit(fd.as_raw_fd(), UI_SET_EVBIT, EV_KEY)?;
        set_bit(fd.as_raw_fd(), UI_SET_KEYBIT, KEY_WAKEUP)?;
        set_bit(fd.as_raw_fd(), UI_SET_KEYBIT, KEY_F4)?;

        let mut dev: UInputUserDev = unsafe { std::mem::zeroed() };
        let bytes = name.as_bytes();
        let count = bytes.len().min(dev.name.len() - 1);
        for (dst, src) in dev.name.iter_mut().zip(bytes.iter().take(count)) {
            *dst = *src as libc::c_char;
        }
        dev.id = InputId {
            bustype: 0x06, // BUS_VIRTUAL
            vendor: 0x18d1,
            product: 0x0001,
            version: 1,
        };

        write_all(fd.as_raw_fd(), &dev)?;
        ioctl(fd.as_raw_fd(), UI_DEV_CREATE, 0)?;
        std::thread::sleep(std::time::Duration::from_millis(150));

        Ok(Self { fd })
    }

    /// Inject Linux KEY_WAKEUP. Android maps this to KEYCODE_WAKEUP.
    pub fn emit_wake(&self) -> TouchResult<()> {
        self.emit_key(KEY_WAKEUP)
    }

    /// Inject Linux KEY_F4 (Android keycode 134) for OPlus AOD dispatch.
    pub fn emit_aod_preview(&self) -> TouchResult<()> {
        self.emit_key(KEY_F4)
    }

    fn emit_key(&self, code: u16) -> TouchResult<()> {
        let events = [
            input_event(EV_KEY, code, 1),
            input_event(EV_SYN, SYN_REPORT, 0),
            input_event(EV_KEY, code, 0),
            input_event(EV_SYN, SYN_REPORT, 0),
        ];
        for event in &events {
            write_all(self.fd.as_raw_fd(), event)?;
        }
        Ok(())
    }
}

impl Drop for WakeInjector {
    fn drop(&mut self) {
        let _ = ioctl(self.fd.as_raw_fd(), UI_DEV_DESTROY, 0);
    }
}

fn set_bit(fd: i32, request: libc::c_ulong, bit: u16) -> TouchResult<()> {
    ioctl(fd, request, bit as libc::c_ulong)
}

fn ioctl(fd: i32, request: libc::c_ulong, value: libc::c_ulong) -> TouchResult<()> {
    let result = unsafe { libc::ioctl(fd, request as _, value) };
    if result < 0 {
        Err(TouchError::Io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn write_all<T>(fd: i32, value: &T) -> TouchResult<()> {
    let ptr = value as *const T as *const u8;
    let mut written = 0;
    let total = std::mem::size_of::<T>();

    while written < total {
        let result =
            unsafe { libc::write(fd, ptr.add(written) as *const libc::c_void, total - written) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(TouchError::Io(error));
        }
        if result == 0 {
            return Err(TouchError::Io(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "short write to uinput",
            )));
        }
        written += result as usize;
    }
    Ok(())
}

fn input_event(kind: u16, code: u16, value: i32) -> libc::input_event {
    libc::input_event {
        time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        type_: kind,
        code,
        value,
    }
}
