# Crate layout

A Cargo workspace. The root `Cargo.toml` holds shared package metadata and the lint table; each
member lives in `crates/<name>/` and inherits both with `workspace = true`, mirroring the sibling
`piv-card` project.

- `crates/obd-bridge/` — a library (`[lib] obd_bridge`) plus a thin binary (`[[bin]] obd-bridge`)
  built on it, so a sibling crate (e.g. `voltkit`) can link the host-OS BLE scan/connect API
  in-process instead of spawning the binary.
  - `src/lib.rs` — the library root: declares the modules below and re-exports the public API
    (`scan_advertisers`, `connect`, `BleSerial`, `SerialLink`, the config types and helpers, and
    `BridgeResult`). Documents the transport-only scope and the Windows requirement.
  - `src/main.rs` — the binary: a thin wrapper over the library holding argument handling, the TCP
    accept loop and the BLE (re)connect loop, plus the `scan` doctor. Windows-only logic is behind
    `#[cfg(windows)]`; a non-Windows build still compiles and supports `--help`, but `run()` returns
    an error explaining the platform requirement.
  - `src/config.rs` — configuration parsing (args + environment), UUID/address parsing, the typed
    scan types (`Seen`, `Advert`, `ScanFilter`, `SerialConfig`), the platform-independent
    `rank_advertisers`/`doctor_row` formatting, and the unit tests. Platform-independent, so the
    tests run anywhere.
  - `src/ble.rs` — the platform-neutral BLE facade: re-exports the Windows backend
    (`src/ble/win.rs`: scan, connect, service/characteristic discovery, notification subscription,
    `BleSerial`) on Windows, and a compiling stub (`src/ble/unsupported.rs`: the same public items
    returning an error, `BleSerial` uninhabited) elsewhere.
  - `src/link.rs` — the `SerialLink` trait (the write half of a serial link; notifications stay a
    `Receiver<Vec<u8>>`) and `write_chunked`, the MTU-splitting rule shared by the BLE backend and
    the test mock. Platform-independent.
  - `src/bridge.rs` — the byte pipe: two directions relayed inside a `std::thread::scope`, generic
    over `SerialLink`. Platform-independent; its tests drive it with an in-memory mock link and real
    loopback TCP, so the relay and error/reconnect contract are checked without a radio.
  - `src/capture.rs` — the optional capture tap (`--capture`): pcap (link type 201), btsnoop and text
    writers, std only, wrapping each relayed GATT value in synthesized HCI ACL / L2CAP / ATT framing.
    `bridge::run` takes a `&Tap` and records after delivery. Platform-independent; see
    [`capture.md`](capture.md).

## Dependencies

Only Microsoft's first-party [`windows`](https://crates.io/crates/windows) crate, and only on
Windows targets (`[target.'cfg(windows)'.dependencies]`). The enabled WinRT features are:

- `Foundation`, `Foundation_Collections` — `IAsyncOperation`, `TypedEventHandler`,
  `EventRegistrationToken`, and the collection views GATT queries return.
- `Devices_Bluetooth`, `Devices_Bluetooth_Advertisement` — `BluetoothLEDevice` and the
  advertisement watcher used to discover an adapter.
- `Devices_Bluetooth_GenericAttributeProfile` — the GATT service/characteristic types.
- `Storage_Streams` — `DataReader`/`DataWriter` to move bytes in and out of GATT `IBuffer` values.

No third-party BLE stack, no async runtime. See `ble.rs` for the COM-apartment note (the bridge
relies on the Windows implicit MTA, which keeps the crate free of `unsafe`).

## Invariants

- `unsafe` is forbidden by the lint table in the root `Cargo.toml`; do not relax it to make
  something compile.
- Library/binary lints are configured in the manifest (`[lints] workspace = true`), not with
  `#![...]` attributes in files.
- Write scratch output outside the repository; `.gitignore` covers `/target`, `*.log`, `*.out`.
