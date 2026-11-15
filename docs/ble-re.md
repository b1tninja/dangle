# Adapter BLE profile — how it was recovered

The default GATT UUIDs this bridge uses were recovered statically from the **OBDLink** companion
app (which also backs OBDFusion; both are OCTech products), not guessed. This note records the
method and the result so the defaults are auditable and so other adapters can be configured against
the `--service` / `--write` / `--notify` overrides.

No app was installed or run during this: the APK was read as data and decompiled statically.

## Method

The OBDLink Android app is a **.NET MAUI** application (Mono/AOT), so the logic is in managed .NET
assemblies, not in the Android dex (the dex only bootstraps Mono). The managed assemblies ship in
the per-architecture split APK at `lib/arm64-v8a/libassembly-store.so` — a .NET "AssemblyStore"
blob holding each assembly, some stored as plain metadata (`BSJB` signature) and most LZ4-compressed
with the `XALZ` container.

Recovery steps (all static):

1. Pull the base + arm64 split APKs from the device with `adb pull`.
2. `unzip` `libassembly-store.so` from the split.
3. Decompress every `XALZ` blob (LZ4 block format) to recover the managed assembly bytes.
4. Extract ASCII and UTF-16LE strings (.NET stores string literals as UTF-16 in the `#US` stream).
5. Grep for 128-bit UUIDs on the Bluetooth SIG base and for BLE-related identifiers.

## Result — GATT serial profile

Classic "FFF0" BLE serial profile, on the Bluetooth base UUID
`0000xxxx-0000-1000-8000-00805f9b34fb`:

| Role               | 16-bit | Full UUID                              |
| ------------------ | ------ | -------------------------------------- |
| Serial service     | 0xFFF0 | `0000fff0-0000-1000-8000-00805f9b34fb` |
| Characteristic     | 0xFFF1 | `0000fff1-0000-1000-8000-00805f9b34fb` |
| Characteristic     | 0xFFF2 | `0000fff2-0000-1000-8000-00805f9b34fb` |

The classic-Bluetooth Serial Port Profile (`00001101-…`, RFCOMM) is also present, for non-BLE
adapters, and is not used by this bridge.

### Write vs notify is decided by property, not by number

The app does **not** hard-code which characteristic is write and which is notify. It holds
`DiscoveredTxCharacteristic` / `DiscoveredRxCharacteristic` fields — i.e. it discovers the serial
characteristics by GATT property at connect time. This bridge does the same:

- the **notify** characteristic (adapter → host, "Rx" from the client's view) is the one with the
  **Notify** (or Indicate) property;
- the **write** characteristic (host → adapter, "Tx") is the one with **Write** or
  **WriteWithoutResponse** (the latter is preferred when available).

For the FFF0 profile the usual assignment is FFF1 = notify, FFF2 = write, but the property-based
selection does not depend on that. Explicit `--write` / `--notify` UUIDs override the discovery when
an adapter needs it.

## Other adapters

The app's strings name the OBDLink MX+, MX, CX, LX, SX and EX adapters and the STN1100 / STN2320
chips ("Multiprotocol OBD II to UART Interpreter" — an ELM327-compatible AT layer over a UART that
the BLE characteristic tunnels). No Nordic-UART service (`6e400001-…`) appears in this APK, but some
other adapters (e.g. vLinker MC/MS) use it; point `--service`/`--write`/`--notify` at those UUIDs for
such adapters, or rely on property-based discovery within the given service.

## ELM327 transport (reference only — the bridge does not parse this)

The stream is ASCII command/response lines, each command terminated by `\r`, each response ending at
the `>` prompt. A client configures the adapter with commands such as `ATZ`, `ATE0`, `ATL0`, `ATH0`,
`ATS0`, `ATSP6`, `ATSH<id>`, `ATCRA<id>`, plus the ISO 15765 flow-control programming commands
(`ATFCSH` / `ATFCSD` / `ATFCSM`). All of that is the client's responsibility; the bridge relays the
bytes untouched. The full protocol context lives in the `volt` repo's `docs/OBD-CONFIG.md`.
