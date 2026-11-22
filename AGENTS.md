# Instructions for coding agents

`dangle` is a Rust workspace (edition 2024, MSRV 1.85), mirroring the sibling `piv-card`
project's structure and first-party-dependencies-only policy. Read `docs/layout.md` before adding
modules.

## Commands

Run these from this directory; all must pass before a change is done.

    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo doc --no-deps

(There is no `tools/matrix.py` here; `piv-card`'s fifth gate is specific to that project.)

## Scope

`dangle` ("a dongle adapter") is a **transport bridge only**: it relays raw bytes between a client
and a Bluetooth LE serial dongle's serial GATT characteristics. A dongle is any BLE peripheral that
exposes a serial stream over a GATT serial service (a Nordic-UART-style or FFF0-style profile); the
motivating case is an OBD adapter such as an OBDLink MX, but nothing here is OBD-specific. The crate
must not grow protocol parsing (ELM327, UDS, OBD) or any write logic of its own — that belongs to the
client. Keep `ble.rs` to connection management and `bridge.rs` to the byte pipe.

## Invariants

- `unsafe` is forbidden by the lint table in the root `Cargo.toml`; do not relax it. The WinRT BLE
  APIs used here are safe; the crate relies on the Windows implicit MTA rather than an `unsafe`
  `CoInitializeEx`/`RoInitialize` (see the module docs in `ble.rs`).
- First-party dependencies only: Microsoft's `windows` crate for BLE, `std` for everything else. No
  third-party BLE stack (no btleplug) and no async runtime. Do not add a dependency without saying
  why in the commit message.
- Default to the least-exposed choice: the TCP server binds loopback by default.
- No identifying detail of a real dongle, vehicle, person or machine in the tree: no Bluetooth
  addresses, VINs, serial numbers or local paths. The GATT UUIDs in `docs/ble-re.md` are generic
  profile constants and are fine.
- Read a standard before citing it, and cite it by section. Never state normative wording from
  memory.
- Write scratch output outside the repository; `git add -A` will commit it.
- Commit messages carry no AI attribution trailer.
