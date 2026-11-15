# Crate layout

A Cargo workspace. The root `Cargo.toml` holds shared package metadata and the lint table; each
member lives in `crates/<name>/` and inherits both with `workspace = true`, mirroring the sibling
`piv-card` project.

- `crates/obd-bridge/` — the bridge binary.
  - `src/main.rs` — entry point, argument handling, the TCP accept loop and the BLE
    (re)connect loop. Windows-only logic is behind `#[cfg(windows)]`; a non-Windows build still
    compiles and supports `--help`, but `run()` returns an error explaining the platform
    requirement.
  - `src/config.rs` — configuration parsing (args + environment), UUID/address parsing, and the
    unit tests. Platform-independent, so the tests run anywhere.
  - `src/ble.rs` — the WinRT Bluetooth LE backend: scan, connect, service/characteristic discovery,
    notification subscription, and chunked writes. Windows-only.
  - `src/bridge.rs` — the byte pipe: two directions relayed inside a `std::thread::scope`.

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
