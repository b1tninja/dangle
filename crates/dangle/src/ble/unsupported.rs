//! Non-Windows stub for the BLE backend.
//!
//! The only host-radio backend is `WinRT` (see [`super::win`]). On other platforms these items exist
//! so the crate compiles and the public API is the same shape everywhere, but there is no radio:
//! the functions return an error and [`BleSerial`] is uninhabited, so it can never be constructed.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use crate::BridgeResult;
use crate::config::{Advert, ScanFilter, SerialConfig};
use crate::link::SerialLink;

/// The platform requirement, returned by every call on a non-Windows host.
fn unsupported() -> Box<dyn std::error::Error + Send + Sync + 'static> {
    "dangle BLE requires Windows: the Bluetooth LE backend uses WinRT GATT".into()
}

/// A connected BLE adapter exposed as a byte sink — the Windows [`super::win::BleSerial`] on
/// Windows. This platform has no backend, so the type is uninhabited and [`connect`] never yields
/// one; it exists only so the public API type-checks everywhere.
pub enum BleSerial {}

impl SerialLink for BleSerial {
    fn write(&self, _data: &[u8]) -> BridgeResult<()> {
        match *self {}
    }
}

/// Stub for [`super::win::scan_advertisers`]: always an error on a non-Windows host.
///
/// # Errors
/// Always returns an error: scanning requires a Windows host radio.
pub fn scan_advertisers(
    _scan_timeout: Duration,
    _filter: &ScanFilter,
    _serial: u128,
) -> BridgeResult<Vec<Advert>> {
    Err(unsupported())
}

/// Stub for [`super::win::connect`]: always an error on a non-Windows host.
///
/// # Errors
/// Always returns an error: connecting requires a Windows host radio.
pub fn connect(_address: u64, _cfg: &SerialConfig) -> BridgeResult<(BleSerial, Receiver<Vec<u8>>)> {
    Err(unsupported())
}
