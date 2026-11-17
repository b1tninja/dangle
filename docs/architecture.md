# Architecture

This document describes how `obd-bridge` works and how its default Bluetooth LE profile was
recovered. Each diagram is given twice: an inline **Mermaid** block (GitHub renders it) and a
standalone **SVG** under [`diagrams/`](diagrams/) for viewers that prefer an image.

Companion docs: [`layout.md`](layout.md) (crate/module layout, dependencies) and
[`ble-re.md`](ble-re.md) (the full reverse-engineering write-up and GATT table).

---

## 1. What the bridge does

A Bluetooth LE ELM327/STN OBD adapter exposes its serial stream over a GATT service. Tools that can
open a TCP socket but cannot use the host's Bluetooth radio — most importantly an Android emulator —
cannot talk to such an adapter directly. `obd-bridge` sits in the middle: it owns the BLE connection
on the host and re-exposes the adapter's serial stream as an **ELM327-over-TCP** server, so the
adapter looks like a Wi-Fi OBD dongle.

The bridge binds the host address (loopback `127.0.0.1:35000` by default). Inside the standard
Android emulator that IPv4 loopback is named `10.0.2.2`, so the app's Wi-Fi host is `10.0.2.2` and
the port is the bound port. `10.0.2.2` is rejected as a `--listen` address. When the emulator
cannot open it, `adb -s emulator-<digits> reverse tcp:<port> tcp:<port>` makes the emulator's
`127.0.0.1:<port>` reach the host socket, and the app uses host `127.0.0.1`.

It is a **transport bridge only**. It relays raw ASCII bytes (ELM327 `AT` command lines terminated
by `\r`, responses ending at the `>` prompt) and performs no OBD/UDS parsing and no vehicle writes.
This is roadmap item A1 in the `volt` repo's `docs/OBD-CONFIG-ROADMAP.md`.

### Runtime topology

![Runtime architecture](diagrams/runtime-architecture.svg)

```mermaid
flowchart LR
    subgraph client["TCP client (no host Bluetooth)"]
        EMU["Android emulator /<br/>scan tool / app<br/>(Wi-Fi OBD mode)"]
    end

    subgraph host["Host — obd-bridge.exe"]
        LST["TcpListener<br/>(default 127.0.0.1:35000)"]
        subgraph pipe["bridge::run — thread::scope"]
            T1["tcp → ble pump<br/>(spawned thread)"]
            T2["ble → tcp pump<br/>(scope thread)"]
        end
        BLE["ble module<br/>WinRT GATT backend"]
    end

    subgraph adapter["BLE OBD adapter"]
        GATT["GATT service 0xFFF0<br/>write + notify chars"]
    end

    VEH["Vehicle / bench<br/>(OBD-II bus)"]

    EMU <-->|"TCP bytes"| LST
    LST --> pipe
    T1 -->|"write char (chunked to MTU)"| BLE
    BLE -->|"notify (ValueChanged)"| T2
    BLE <-->|"BLE GATT"| GATT
    GATT <-->|"UART / OBD-II"| VEH
```

The relay is symmetric and byte-exact: bytes from the socket are written to the adapter's write
characteristic (split into MTU-sized GATT writes); bytes the adapter sends via GATT notifications are
written straight back to the socket.

---

## 2. Connection and relay sequence

The bridge runs two nested loops: an outer loop that (re)connects the adapter, and an inner loop that
serves one TCP client at a time while the adapter stays connected. A BLE write failure breaks back to
the outer loop to reconnect; a client disconnect keeps the adapter and waits for the next client.

![Connection sequence](diagrams/connection-sequence.svg)

```mermaid
sequenceDiagram
    autonumber
    participant M as main (run)
    participant B as ble module
    participant W as WinRT / adapter
    participant C as TCP client

    rect rgb(238,244,250)
    note over M,W: outer loop — establish adapter
    M->>B: scan_for_adapter(cfg)
    B->>W: BluetoothLEAdvertisementWatcher (name/MAC filter)
    W-->>B: Bluetooth address
    M->>B: connect(cfg, address)
    B->>W: FromBluetoothAddressAsync → GetGattServicesForUuid(FFF0)
    B->>W: GetCharacteristics → pick write/notify by property
    B->>W: subscribe (WriteCccd Notify) + ValueChanged handler
    W-->>B: (BleSerial, notify Receiver)
    end

    rect rgb(240,247,240)
    note over M,C: inner loop — serve a client
    C->>M: TCP connect
    M->>M: bridge::run(stream, &ble, &notify)
    loop until disconnect / BLE error
        C->>M: command bytes (ATSP6\r …)
        M->>W: write char (chunked)
        W-->>M: notify chunks
        M->>C: response bytes (… >)
    end
    end

    alt BLE write fails
        M->>M: drop adapter, back to outer loop (reconnect)
    else client closes
        M->>M: keep adapter, accept next client
    end
```

### Why two threads

A blocking socket read and a blocking notification receive cannot share one thread, so the pipe runs
inside a [`std::thread::scope`]: the TCP→BLE direction is a spawned thread, the BLE→TCP direction runs
on the scope's own thread, and both borrow the same `&BleSerial`. A shared `AtomicBool` stop flag and
a 200 ms poll timeout on each side let either direction notice a shutdown promptly without
busy-waiting. The notification `Receiver` is deliberately kept out of `BleSerial` so the struct stays
`Sync` and can be borrowed across the two threads.

```mermaid
flowchart TD
    subgraph scope["thread::scope"]
        direction LR
        RS["TcpStream (read clone)"] -->|read| P1["tcp → ble"]
        P1 -->|BleSerial::write| WC["write characteristic"]
        NR["notify Receiver"] -->|recv_timeout| P2["ble → tcp"]
        P2 -->|write_all| WSS["TcpStream (write)"]
    end
    STOP(["AtomicBool stop"]) -.-> P1
    STOP -.-> P2
    P1 -. "on BLE error: record + set stop" .-> STOP
```

---

## 3. How the default BLE profile was recovered

The default GATT UUIDs are not guessed — they were read statically out of the **OBDLink** companion
app (a .NET MAUI application). No app was installed or run; the APK was treated as data. Full detail,
including the recovered UUID table, is in [`ble-re.md`](ble-re.md); the pipeline is summarised here.

![RE pipeline](diagrams/re-pipeline.svg)

```mermaid
flowchart TD
    A["Pixel phone<br/>OBDLink app"] -->|"adb pull base + arm64 split"| B["APK splits"]
    B -->|"unzip"| C["lib/arm64-v8a/<br/>libassembly-store.so"]
    C --> D{".NET AssemblyStore"}
    D -->|"205 uncompressed"| E["BSJB metadata<br/>(UTF-16 #US strings)"]
    D -->|"185 compressed"| F["XALZ / LZ4 blobs"]
    F -->|"pure-Python LZ4 decompress"| G["managed assembly bytes"]
    E --> H["extract ASCII + UTF-16LE strings"]
    G --> H
    H -->|"grep UUIDs + BLE identifiers"| I["GATT serial profile"]
    I --> J["service 0xFFF0<br/>chars 0xFFF1 / 0xFFF2"]
    I --> K["DiscoveredTxCharacteristic /<br/>DiscoveredRxCharacteristic<br/>→ choose by GATT property"]
```

### GATT profile (result)

| Role           | 16-bit | Full UUID                              |
| -------------- | ------ | -------------------------------------- |
| Serial service | 0xFFF0 | `0000fff0-0000-1000-8000-00805f9b34fb` |
| Characteristic | 0xFFF1 | `0000fff1-0000-1000-8000-00805f9b34fb` |
| Characteristic | 0xFFF2 | `0000fff2-0000-1000-8000-00805f9b34fb` |

The app discovers which characteristic is write and which is notify **by property** (it holds
`DiscoveredTxCharacteristic`/`DiscoveredRxCharacteristic`), so the bridge does the same: the
**notify/indicate** characteristic is the adapter→host path, the **write/write-without-response**
characteristic is the host→adapter path. `--service`/`--write`/`--notify` override this for adapters
that differ (e.g. Nordic-UART clones).

---

## 4. Module map

| File         | Responsibility                                                                        |
| ------------ | ------------------------------------------------------------------------------------- |
| `main.rs`    | argument handling; outer BLE reconnect loop; inner TCP accept loop                    |
| `config.rs`  | args + env parsing, UUID/address parsing, unit tests (platform-independent)           |
| `ble.rs`     | WinRT backend: scan, connect, service/characteristic discovery, subscribe, write      |
| `bridge.rs`  | the byte pipe: two relay directions inside a `thread::scope`                          |

See [`layout.md`](layout.md) for the dependency rationale and the enabled `windows`-crate features.

[`std::thread::scope`]: https://doc.rust-lang.org/std/thread/fn.scope.html
