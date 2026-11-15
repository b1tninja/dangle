# obd-bridge

Republish a **Bluetooth LE** ELM327/STN-class OBD adapter as an **ELM327-over-TCP** server. The
bridge connects to a BLE dongle with the host's Bluetooth radio and relays its serial stream over a
TCP socket, so it looks like a Wi-Fi OBD dongle.

Motivating use case: an Android emulator (and other host tools) cannot use host Bluetooth, but it can
reach a TCP endpoint. Point such a tool at `host:port` and it drives a real BLE adapter. This is
roadmap item A1 in the `volt` repo's `docs/OBD-CONFIG-ROADMAP.md`.

## What it is — and is not

- **It is** a transport bridge: a byte pipe. It relays raw ASCII command/response bytes (ELM327 `AT`
  lines terminated by `\r`, responses ending at the `>` prompt) between the TCP client and the
  adapter's BLE serial characteristics. It manages the BLE connection (scan, connect, discover the
  serial service, subscribe to notifications, chunk writes to the MTU, reconnect) and nothing else.
- **It is not** an OBD/UDS implementation. It parses no OBD messages, builds no UDS requests, and
  issues **no vehicle writes** of its own. The client (scan tool / app) speaks the full protocol; the
  bridge only moves bytes. Configuring the adapter (`ATSP6`, `ATSH`, `ATCRA`, …) and any diagnostic
  or write activity is entirely the client's doing.

Exercising it end to end needs a **real BLE adapter and a vehicle or bench** on the other side; there
is no simulated adapter here.

## Platform

Windows only. The BLE backend uses Microsoft's first-party [`windows`](https://crates.io/crates/windows)
crate (WinRT `Windows.Devices.Bluetooth` / GATT). There is no third-party BLE stack and no async
runtime: WinRT async operations are driven synchronously on worker threads, and the byte pipe is
plain `std::net` + `std::thread`. `unsafe` is forbidden crate-wide.

## Build

```
cargo build --release
```

The executable is `target/release/obd-bridge.exe`.

## Usage

```
obd-bridge [OPTIONS]

OPTIONS:
    --listen <ADDR>      TCP listen address            (env OBD_BRIDGE_LISTEN) [default 127.0.0.1:35000]
    --name <SUBSTR>      adapter name must contain     (env OBD_BRIDGE_NAME)
    --mac <AA:BB:..>     adapter Bluetooth address     (env OBD_BRIDGE_MAC)
    --service <UUID>     serial GATT service UUID       (env OBD_BRIDGE_SERVICE) [default FFF0]
    --write <UUID>       write characteristic UUID     (env OBD_BRIDGE_WRITE)  [default: by property]
    --notify <UUID>      notify characteristic UUID    (env OBD_BRIDGE_NOTIFY) [default: by property]
    --mtu <N>            max payload bytes per write   (env OBD_BRIDGE_MTU)    [default 20]
    --scan-timeout <S>   seconds per scan attempt      (env OBD_BRIDGE_SCAN)   [default 20]
    -h, --help           print this help
```

A UUID may be a 4-hex short form (`FFF0`) or a full 128-bit UUID
(`0000fff0-0000-1000-8000-00805f9b34fb`). A Bluetooth address is 12 hex digits, separators optional.

By default the bridge binds **loopback only** (`127.0.0.1`), so the adapter is not exposed to the
network until you choose a different `--listen` address.

### Example: drive a BLE adapter from an Android emulator

1. Pair the BLE adapter with Windows (or at least have it powered and advertising).
2. Start the bridge, filtering to your adapter by name and listening where the emulator can reach it:

   ```
   obd-bridge --name "OBDLink" --listen 10.0.2.2:35000
   ```

   `10.0.2.2` is the host as seen from the standard Android emulator; adjust for your networking.
3. In the OBD app running in the emulator, choose a **Wi-Fi / network** adapter (not Bluetooth) and
   set the host and port to the bridge's `--listen` address.
4. The app's `AT`/OBD traffic now flows to the real BLE dongle.

The bridge serves one TCP client at a time. If the BLE link drops it reconnects automatically; if a
client disconnects, the adapter stays connected for the next one.

## Adapter BLE profile

The default GATT service is `0xFFF0` with the write/notify characteristics discovered by property.
This matches the OBDLink-class BLE serial profile; see [`docs/ble-re.md`](docs/ble-re.md) for how it
was recovered and for notes on other adapters (e.g. Nordic-UART clones), which the `--service` /
`--write` / `--notify` overrides accommodate.

## Layout

A Cargo workspace; the bridge binary is in `crates/obd-bridge/`. See [`docs/layout.md`](docs/layout.md).

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).
