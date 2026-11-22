# dangle

A **dangle** is a dongle adapter: `dangle` connects to a **Bluetooth LE serial dongle** with the
host's Bluetooth radio and bridges its GATT serial stream to a plain byte stream — as a library, or
as an **ELM327-over-TCP-style** TCP server. A BLE dongle that exposes a serial service (a
Nordic-UART-style or FFF0-style profile) becomes something a non-Bluetooth TCP client can drive.

The motivating dongle is an OBD adapter such as an OBDLink MX — bridge it over TCP and it looks like
a Wi-Fi OBD dongle — but nothing here is OBD-specific. `dangle` scans for a BLE peripheral with a
serial GATT service, connects, and moves raw bytes; the bytes mean whatever the dongle and the client
agree they mean.

Motivating use case for the TCP server: an Android emulator (and other host tools) cannot use host
Bluetooth, but it can reach a TCP endpoint. Point such a tool at `host:port` and it drives a real BLE
dongle over the host radio.

## What it is — and is not

- **It is** a transport bridge: a byte pipe. It relays raw bytes between the client and the dongle's
  BLE serial characteristics. It manages the BLE connection (scan, connect, discover the serial
  service, subscribe to notifications, chunk writes to the MTU, reconnect) and nothing else.
- **It is not** a protocol implementation. It parses no messages and issues **no writes** of its own.
  Any protocol on the wire — an ELM327/OBD session being the obvious example, where the client sends
  `AT` lines terminated by `\r` and reads responses ending at the `>` prompt — is entirely the
  client's doing. The bridge only moves bytes.

Exercising it end to end needs a **real BLE dongle** on the other side; there is no simulated dongle
here.

## Platform

Windows only. The BLE backend uses Microsoft's first-party [`windows`](https://crates.io/crates/windows)
crate (WinRT `Windows.Devices.Bluetooth` / GATT). There is no third-party BLE stack and no async
runtime: WinRT async operations are driven synchronously on worker threads, and the byte pipe is
plain `std::net` + `std::thread`. `unsafe` is forbidden crate-wide.

## Build

```
cargo build --release
```

The executable is `target/release/dangle.exe`.

## Usage

```
dangle [OPTIONS]

OPTIONS:
    --listen <ADDR>      TCP listen address            (env DANGLE_LISTEN) [default 127.0.0.1:35000]
    --name <SUBSTR>      dongle name must contain      (env DANGLE_NAME)
    --mac <AA:BB:..>     dongle Bluetooth address      (env DANGLE_MAC)
    --service <PRESET|UUID>  serial GATT service        (env DANGLE_SERVICE) [default fff0]
    --write <UUID>       write characteristic UUID     (env DANGLE_WRITE)  [default: preset/by property]
    --notify <UUID>      notify characteristic UUID    (env DANGLE_NOTIFY) [default: preset/by property]
    --mtu <N>            max payload bytes per write   (env DANGLE_MTU)    [default 20]
    --scan-timeout <S>   seconds per scan attempt      (env DANGLE_SCAN)   [default 20]
    --capture <PATH>     record relayed traffic to PATH (env DANGLE_CAPTURE) [default: off]
    --capture-format <F> pcap | btsnoop | text         (env DANGLE_CAPTURE_FORMAT) [default pcap]
    -h, --help           print this help
```

A Bluetooth address is 12 hex digits, separators optional.

### Choosing the serial profile

The GATT serial service is a **first-class, user-selectable choice**, not a fixed OBD assumption.
`--service` takes either a named preset or a raw UUID:

| Preset | Aliases | Service | Write / Notify |
|---|---|---|---|
| `fff0` (default) | `obd`, `elm327` | `0xFFF0` | discovered by property |
| `nordic-uart` | `nordic`, `nus` | `6E400001-B5A3-F393-E0A9-E50E24DCCA9E` | `…0002-…` / `…0003-…` |

A preset sets the service and, where the profile fixes them, the write/notify characteristics;
`--write` / `--notify` still override per field. Instead of a preset name, `--service` /
`--write` / `--notify` accept a UUID — a 4-hex short form (`FFF0`) or a full 128-bit UUID
(`0000fff0-0000-1000-8000-00805f9b34fb`).

`fff0` is the **default** profile, a common value — not a universal one: it is used by many
ELM327/OBD dongles. For a dongle that matches neither preset, give `--service <uuid>` and, if it
pins its characteristics, `--write` / `--notify`; otherwise the write/notify characteristics are
discovered by property within the chosen service. See [`docs/ble-re.md`](docs/ble-re.md) for how the
`fff0` default was chosen.

By default the bridge binds **loopback only** (`127.0.0.1`), so the dongle is not exposed to the
network until you choose a different `--listen` address. From the standard Android emulator that
socket is `10.0.2.2` on the same port: that name is the emulator's alias for the host's IPv4
loopback. The bridge rejects `--listen 10.0.2.2:<port>` and tells you to bind `127.0.0.1` instead.

### Example: drive a BLE OBD dongle from an Android emulator

1. Pair the BLE dongle with Windows (or at least have it powered and advertising).
2. Start the bridge on the default loopback socket, filtering to your dongle by name:

   ```
   dangle --name "OBDLink"
   ```

   It prints the bind address (`127.0.0.1:35000`) and the Wi-Fi host and port to type into the emulator.
3. In the OBD app running in the emulator, choose a **Wi-Fi / network** adapter (not Bluetooth).
   Set the host to `10.0.2.2` and the port to the printed port (`35000` unless you changed it).
   When the emulator cannot open `10.0.2.2`, forward the port and use the emulator's own loopback:

   ```
   adb -s emulator-<digits> reverse tcp:35000 tcp:35000
   ```

   The serial is `emulator-` followed by digits. Then set the host to `127.0.0.1` and the same port.
4. The app's `AT`/OBD traffic now flows to the real BLE dongle.

The bridge serves one TCP client at a time. If the BLE link drops it reconnects automatically; if a
client disconnects, the dongle stays connected for the next one.

## Pre-flight scan

`dangle scan` (alias `doctor`) is a read-only check: it lists the host radio and the BLE advertisers
it can see, marking the one that carries the serial service, then exits. Run it to confirm a dongle
is visible and read off the `--mac` / `--service` to use, without opening a GATT connection or
binding a socket.

## Capture

`--capture <path>` writes the relayed traffic as a pcap (link type 201), btsnoop or text file. The
Bluetooth framing in pcap/btsnoop is synthesized around GATT values; see
[`docs/capture.md`](docs/capture.md) for exactly what that means and what was verified.

## Serial GATT profile

The default profile (`--service fff0`) is GATT service `0xFFF0` with the write/notify characteristics
discovered by property. This matches the common BLE serial profile used by OBDLink-class and many
other dongles. A second built-in preset, `nordic-uart`, selects the Nordic UART Service and its fixed
write/notify characteristics; any other dongle is reached with a raw `--service <uuid>` (plus
`--write` / `--notify` if it pins its characteristics). See [Choosing the serial profile](#choosing-the-serial-profile)
above for the preset table, and [`docs/ble-re.md`](docs/ble-re.md) for how the `fff0` default was
chosen.

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — how the bridge works and how the default BLE
  profile was chosen, with diagrams (Mermaid inline + SVG under `docs/diagrams/`).
- [`docs/ble-re.md`](docs/ble-re.md) — the reverse-engineering write-up behind the default GATT table.
- [`docs/capture.md`](docs/capture.md) — the optional capture tap and its synthesized framing.
- [`docs/layout.md`](docs/layout.md) — crate/module layout and dependency rationale.

A Cargo workspace; the bridge library and binary are in `crates/dangle/`.

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).
