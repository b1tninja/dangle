//! `obd-bridge`: host-OS Bluetooth LE access to an ELM327/STN OBD adapter, as both a library and a
//! binary.
//!
//! The binary (`src/main.rs`) scans for a BLE adapter with the host radio and republishes it as an
//! ELM327-over-TCP server, so a tool that can reach a TCP endpoint but not host Bluetooth (e.g. an
//! Android emulator) can drive a real BLE adapter; it also has a `scan` doctor subcommand.
//!
//! This library exposes the same host-OS BLE building blocks so a sibling crate (e.g. `voltkit`)
//! can scan and connect to the adapter **in-process**, without spawning the binary:
//!
//! - [`scan_advertisers`] runs the `WinRT` advertisement watcher and returns typed [`Advert`] rows
//!   (address, name, RSSI, advertised service UUIDs, and a serial-service flag), serial-service
//!   first then strongest signal — the same data the `scan` doctor prints.
//! - [`connect`] opens the adapter's serial GATT service (default `0xFFF0`), subscribes to
//!   notifications, and returns a [`BleSerial`] byte sink plus a [`std::sync::mpsc::Receiver`] of
//!   the bytes the adapter sends back.
//! - [`SerialLink`] / [`write_chunked`] are the transport abstraction, and [`bridge::run`] is the
//!   byte pipe the binary uses to relay a TCP client to a [`SerialLink`].
//!
//! # Scope
//!
//! This crate is a **transport bridge only**: it moves raw bytes between a client and the adapter's
//! serial characteristics. It performs no OBD/UDS parsing and issues no vehicle writes; that logic
//! belongs to the client that drives the ELM327 session.
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
    DEFAULT_SERVICE_UUID, ScanFilter, Seen, SerialConfig, advertises, doctor_row, format_addr,
    parse_mac, parse_uuid, short_uuid, uuid_label,
};
pub use link::{SerialLink, write_chunked};
