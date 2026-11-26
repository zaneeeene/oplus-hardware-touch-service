//! Shared Android power-wake helpers.
//!
//! OPlus power policy does not reliably wake Android from a raw uinput
//! `KEY_WAKEUP`, even when the touch driver reports it successfully. This is
//! the path used by the working raise-to-wake daemon: hold a short wake lock
//! and ask the power service to wake the device explicitly.

use std::process::{Command, Stdio};
use std::time::Duration;

/// Hold an expiring wake lock so the AP cannot suspend while the power command
/// is being delivered. Failure is non-fatal because some vendor policies only
/// permit the power-service path.
pub fn hold_wake_lock(name: &str, duration: Duration) {
    let timeout_ns = duration.as_nanos().min(u64::MAX as u128) as u64;
    let request = format!("{name} {timeout_ns}");
    let _ = std::fs::write("/sys/power/wake_lock", request);
}

/// Request a full Android wake through the power service.
///
/// This matches the proven `oplus-hardware-raise2wake` injection and avoids
/// treating a driver-emitted `KEY_WAKEUP` as proof that Android resumed.
pub fn request_power_wakeup() -> Result<(), String> {
    let result = Command::new("/system/bin/cmd")
        .args(["power", "wakeup"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("cmd power wakeup exited with {status}")),
        Err(error) => Err(format!("cmd power wakeup failed: {error}")),
    }
}
