//! Screen-power detection shared by gesture and panel workers.

#[cfg(target_os = "android")]
use std::ffi::CStr;

/// OPlus/QTI `debug.tracing.screen_state` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    Off,
    On,
    Doze,
    DozeSuspend,
    Unknown,
}

impl ScreenState {
    pub fn read() -> Self {
        Self::resolve(Self::read_tracing(), Self::read_brightness())
    }

    fn resolve(tracing: Option<Self>, brightness: Option<u32>) -> Self {
        match (tracing, brightness) {
            // debug.tracing.screen_state can remain at 2 for a short period
            // after the panel has already blanked. That stale value made DT2W
            // userspace classification wait for full doze. A live zero
            // backlight proves the panel is no longer showing a normal frame.
            (Some(Self::On), Some(0)) => Self::Off,
            (Some(state), _) => state,

            // Fallback for ROMs without the tracing property. A lit panel is
            // treated as fully on; the explicit property above is required to
            // distinguish normal AOD from a dimmed AOD panel.
            (None, Some(brightness)) if brightness > 0 => Self::On,
            (None, Some(_)) => Self::Off,
            (None, None) => Self::Unknown,
        }
    }

    fn read_brightness() -> Option<u32> {
        for path in [
            "/sys/class/backlight/panel0-backlight/brightness",
            "/sys/class/backlight/panel1-backlight/brightness",
            "/sys/class/backlight/backlight/brightness",
        ] {
            let Ok(value) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(brightness) = value.trim().parse::<u32>() else {
                continue;
            };
            return Some(brightness);
        }
        None
    }

    /// Read only `debug.tracing.screen_state`; unlike `read()`, this never
    /// guesses a doze state from the backlight.
    pub fn read_tracing() -> Option<Self> {
        android_screen_state().map(Self::from)
    }

    pub fn is_on(self) -> bool {
        self == Self::On
    }

    pub fn is_off(self) -> bool {
        self == Self::Off
    }

    pub fn is_doze(self) -> bool {
        matches!(self, Self::Doze | Self::DozeSuspend)
    }

    /// True when the panel is not fully awake. Unknown is deliberately
    /// treated as awake for userspace wake gestures, preventing wake loops.
    pub fn is_screen_off_or_doze(self) -> bool {
        matches!(self, Self::Off | Self::Doze | Self::DozeSuspend)
    }

    pub fn raw(self) -> i32 {
        match self {
            Self::Off => 1,
            Self::On => 2,
            Self::Doze => 3,
            Self::DozeSuspend => 4,
            Self::Unknown => -1,
        }
    }
}

impl From<i32> for ScreenState {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Off,
            2 => Self::On,
            3 => Self::Doze,
            4 => Self::DozeSuspend,
            _ => Self::Unknown,
        }
    }
}

/// Returns whether the display is fully on.
pub fn screen_is_on() -> Option<bool> {
    match ScreenState::read() {
        ScreenState::On => Some(true),
        ScreenState::Unknown => None,
        _ => Some(false),
    }
}

pub fn screen_is_off_or_doze() -> Option<bool> {
    match ScreenState::read() {
        ScreenState::Unknown => None,
        state => Some(state.is_screen_off_or_doze()),
    }
}

#[cfg(target_os = "android")]
fn android_screen_state() -> Option<i32> {
    let mut value = [0 as libc::c_char; 92];
    let length = unsafe {
        libc::__system_property_get(c"debug.tracing.screen_state".as_ptr(), value.as_mut_ptr())
    };
    if length <= 0 {
        return None;
    }

    let text = unsafe { CStr::from_ptr(value.as_ptr()) }.to_str().ok()?;
    text.trim().parse().ok()
}

#[cfg(not(target_os = "android"))]
fn android_screen_state() -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_state_result_is_well_formed() {
        if let Some(state) = android_screen_state() {
            assert!((1..=4).contains(&state), "unexpected screen state: {state}");
        }
    }

    #[test]
    fn state_classification_matches_tracing_values() {
        assert_eq!(ScreenState::from(1), ScreenState::Off);
        assert_eq!(ScreenState::from(2), ScreenState::On);
        assert_eq!(ScreenState::from(3), ScreenState::Doze);
        assert_eq!(ScreenState::from(4), ScreenState::DozeSuspend);
        assert!(ScreenState::Doze.is_doze());
        assert!(ScreenState::Off.is_screen_off_or_doze());
        assert!(!ScreenState::On.is_screen_off_or_doze());
        assert!(!ScreenState::Unknown.is_screen_off_or_doze());
    }

    #[test]
    fn stale_on_state_yields_to_a_blank_panel() {
        assert_eq!(
            ScreenState::resolve(Some(ScreenState::On), Some(0)),
            ScreenState::Off
        );
        assert_eq!(
            ScreenState::resolve(Some(ScreenState::On), Some(100)),
            ScreenState::On
        );
        assert_eq!(
            ScreenState::resolve(Some(ScreenState::Doze), Some(0)),
            ScreenState::Doze
        );
        assert_eq!(ScreenState::resolve(None, Some(100)), ScreenState::On);
        assert_eq!(ScreenState::resolve(None, None), ScreenState::Unknown);
    }
}
