//! Host-OS Bluetooth LE access to the OBD adapter: scan for advertisers and open the serial GATT
//! service as a [`SerialLink`](crate::link::SerialLink) byte pipe.
//!
//! The real backend is Windows-only (`WinRT` `Windows.Devices.Bluetooth`, in the private `win`
//! submodule). On other platforms a stub provides the same public items so the crate compiles; the
//! functions return an error explaining that a Windows host radio is required, and [`BleSerial`] is
//! an uninhabited type that cannot be constructed.
//!
//! The public surface is the same on every platform:
//! - [`scan_advertisers`] — typed BLE scan.
//! - [`connect`] — open the serial service and subscribe to notifications.
//! - [`BleSerial`] — the connected adapter as a byte sink (the read half is the returned
//!   [`Receiver`](std::sync::mpsc::Receiver)).

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{BleSerial, connect, doctor, scan_advertisers, scan_for_adapter};

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub use unsupported::{BleSerial, connect, scan_advertisers};
