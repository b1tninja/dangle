//! `dangle`: host-OS Bluetooth LE access to a serial dongle ("a dongle adapter"), as both a
//! library and a binary.
//!
//! A *dangle* is any BLE peripheral that exposes a serial stream over a GATT serial service
//! (a Nordic-UART-style or FFF0-style profile). The common motivating case is an OBD adapter such
//! as an `OBDLink` MX, but nothing here is OBD-specific: the crate scans for such a peripheral,
//! connects to its serial service, and moves raw bytes to and from it.
//!
//! The binary (`src/main.rs`) scans for a dongle with the host radio and republishes its serial
//! stream as a TCP server, so a tool that can reach a TCP endpoint but not host Bluetooth (e.g. an
//! Android emulator) can drive a real BLE dongle; it also has a `scan` doctor subcommand.
//!
//! This library exposes the same host-OS BLE building blocks so a sibling crate (e.g. `voltkit`)
//! can scan and connect to the dongle **in-process**, without spawning the binary:
//!
//! - [`scan_advertisers`] runs the `WinRT` advertisement watcher and returns typed [`Advert`] rows
//!   (address, name, RSSI, advertised service UUIDs, and a serial-service flag), serial-service
//!   first then strongest signal — the same data the `scan` doctor prints.
//! - [`connect`] opens the dongle's serial GATT service, subscribes to notifications, and returns a
//!   [`BleSerial`] byte sink plus a [`std::sync::mpsc::Receiver`] of the bytes the dongle sends
//!   back. The service and characteristics come from a [`SerialConfig`]; named [`Preset`]s
//!   ([`PRESETS`], e.g. `fff0` and `nordic-uart`) select a profile, with `fff0` the default.
//! - [`SerialLink`] / [`write_chunked`] are the transport abstraction, and [`bridge::run`] is the
//!   byte pipe the binary uses to relay a TCP client to a [`SerialLink`].
//!
//! # Scope
//!
//! This crate is a **transport bridge only**: it moves raw bytes between a client and the dongle's
//! serial characteristics. It parses nothing on the wire and issues no writes of its own; any
//! protocol (an ELM327/OBD session, for example) is the client's responsibility.
//!
//! # Platforms
//!
//! The BLE backend is Windows-only (`WinRT` `Windows.Devices.Bluetooth`). On other platforms the BLE
//! items ([`scan_advertisers`], [`connect`], [`BleSerial`]) still exist so the crate compiles, but
//! the functions return an error explaining the platform requirement. The pure parts (configuration
//! parsing, advertiser ranking/formatting, the byte pipe, the capture tap) build and are tested
//! everywhere.

pub mod ble;
pub mod bridge;
pub mod capture;
pub mod config;
pub mod link;

/// Crate-wide fallible result with a boxed, thread-safe error.
pub type BridgeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

pub use ble::{BleSerial, connect, scan_advertisers};
pub use config::{
    Advert, Config, ConfigError, DEFAULT_LISTEN, DEFAULT_MAX_WRITE, DEFAULT_SCAN_SECS,
    DEFAULT_SERVICE_UUID, HM10_CHAR_UUID, HM10_SERVICE_UUID, MICROCHIP_UART_NOTIFY_UUID,
    MICROCHIP_UART_SERVICE_UUID, MICROCHIP_UART_WRITE_UUID, NORDIC_UART_NOTIFY_UUID,
    NORDIC_UART_SERVICE_UUID, NORDIC_UART_WRITE_UUID, PRESETS, Preset, ScanFilter, Seen,
    SerialConfig, advertises, adverts_json, doctor_row, find_preset, format_addr, parse_mac,
    parse_uuid, resolve_serial, short_uuid, uuid_label,
};
pub use link::{SerialLink, write_chunked};
