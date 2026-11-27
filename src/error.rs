//! Error types for the touch HAL

use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TouchError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Nix error: {0}")]
    Nix(#[from] nix::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Operation not supported: {0}")]
    NotSupported(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("AOD error: {0}")]
    Aod(String),

    #[error("Gesture error: {0}")]
    Gesture(String),

    #[error("Raise to wake error: {0}")]
    RaiseToWake(String),

    #[error("Xiaomi touch error: {0}")]
    XiaomiTouch(String),
}

pub type TouchResult<T> = Result<T, TouchError>;
