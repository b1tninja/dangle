# dangle roadmap

`dangle` ("a dongle adapter") bridges a Bluetooth LE serial dongle's GATT serial stream to a byte
stream and, in the binary, to a TCP server. The host radio owns the GATT connection and a TCP client
receives the dongle's serial bytes. The motivating dongle is an OBD adapter (e.g. an OBDLink MX), but
the bridge is protocol-agnostic. The crate is Windows-only for a live dongle, `unsafe` is forbidden,
and `publish = false`.

Rows are `Status | Obligation | Anchor | Test`, the grammar `projecthub sync` reads for Rust
projects. The registration section is the reason this file exists before that sync runs.

Status:

- `[ok]` the named test in this crate exists and passed on 2026-11-21. There is no break log here, so the mark means the test passed.
- `[open]` not covered by a test here, or it needs a BLE dongle.
- `[no]` a deliberate non-goal. The Test cell stays empty of a test name.

## Registration

Measured 2026-11-21 from `D:\code\projecthub` and `D:\code\PORTFOLIO.md`. This repository does not edit either file.

**Rename note (2026-11-21):** the GitHub repository and the Rust crate were renamed from `obd-bridge`
to `dangle`, and the project was reframed as a generic BLE serial-dongle bridge. GitHub keeps the
`obd-bridge` → `dangle` redirect, so old clones and URLs still work. The projecthub registry entry,
the `PORTFOLIO.md` section 3 line, the `RUST_OWNED` tuple and any sibling `ROADMAP.md` cross-refs
below still name the project `obd-bridge`; they are updated in the shared workspace by a separate
projecthub batch, not from this repository. The table rows record the pre-rename registry state.

| Field | Recorded value |
|---|---|
| Registry entry | `obd-bridge` in `src/projecthub/registry.py` (pending rename to `dangle`) |
| Kind, tier, wave | Tooling, 0, 0 |
| Directory | `obd-bridge` (local checkout; pending rename to `dangle`) |
| Depends | none |
| What | bridges a BLE serial dongle's GATT serial stream to a byte stream / TCP server |
| Registered | 2026-11-21, `PORTFOLIO.md` section 3, untiered and unsequenced |
| GitHub | <https://github.com/b1tninja/dangle> (was `obd-bridge`; GitHub redirects the old URL) |
| `RUST_OWNED` | absent. The tuple is berkit, cardkit, piv-card, vcxtoolkit, gattmirror, fakeobd, voltkit, obdhub |
| Source string | `PORTFOLIO.md s3 (registered 2026-11-21; https://github.com/b1tninja/obd-bridge); no obligation matrix, so it is not in RUST_OWNED` |
| Open row | `bt.obd-bridge-matrix` is `[ ]` in `projecthub/ROADMAP.md`. Its spec cell says `obd-bridge` has no `ROADMAP.md`, so it stays out of `RUST_OWNED` |
| Locking test | `tests/test_registry.py::test_the_bluetooth_mx_path_is_registered` asserts the name is absent from `RUST_OWNED` and that the row is `[ ]` |
| Portfolio evidence | this repository's `AGENTS.md`. Section 3 still says there is no obligation matrix yet, and that this tool is not a J2534 pass-through |

This file is that matrix. The next projecthub batch is what points the source string here, adds the name to `RUST_OWNED`, changes `bt.obd-bridge-matrix`, and reconciles the registry name with the `dangle` rename. Until that batch, `projecthub sync` does not read the rows below, and the locking test still expects the old sentences.

## Peers

The lab path uses a stub listener and does not share this radio. `voltkit` `tests/lab.rs` builds that stub, binds `127.0.0.1:35000`, writes `listening` to stderr, and opens no Bluetooth radio. The lab command in `voltkit` `src/lab.rs` is `fakeobd serve --listen 127.0.0.1:35000`. `serve_listens_on_127_0_0_1_35000` requires that argument list to omit Bluetooth and J2534.

`voltkit bench` is the peer that starts this binary: `dangle --name OBDLink`, dialing `127.0.0.1:35000` (`voltkit` `src/bench.rs`). That command does not pass `--listen`. voltkit does not open the radio. A live GATT session still needs Windows and an advertising dongle.

The default bind in this crate is `127.0.0.1:35000` (`DEFAULT_LISTEN`). No test in this crate reads that constant.

## Byte pipe

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | Relay TCP bytes to the serial link verbatim, split at the configured write size, and return notification bytes in order through to the `>` prompt | `AGENTS.md`; `docs/layout.md` | `bridge::tests::client_bytes_reach_link_verbatim_and_chunked`; `bridge::tests::notifications_reach_client_in_order`; `bridge::tests::split_response_is_relayed_byte_exact_to_prompt` |
| [ok] | A client disconnect is success, and the same link serves the next client | `README.md` | `bridge::tests::client_disconnect_is_ok_and_link_serves_next_client` |
| [ok] | A failed link write ends the session with that error | `src/bridge.rs` | `bridge::tests::failing_link_write_makes_run_err` |
| [open] | After a failed link write, the process drops the dongle and scans again | `src/main.rs` outer loop | none: the loop is not under test |
| [open] | Scan, connect, subscribe, and notify against a real BLE dongle | `README.md`; `src/ble/win.rs` | none: needs Windows and a dongle |
| [open] | On a build without Windows, `run` returns the WinRT requirement and `--help` still prints | `src/main.rs` | none: not under test |

## Emulator dial address

The bridge binds a host address. The standard Android emulator reaches the host's IPv4 loopback as `10.0.2.2` (`EMULATOR_HOST`). That address is the Wi-Fi host the operator types. It is refused as a `--listen` address.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | When the bound address is `127.0.0.1` or IPv4 `0.0.0.0`, the notice tells the emulator to dial `10.0.2.2` on the bound port, and names the `adb reverse` fallback that then uses host `127.0.0.1` | `src/config.rs` `emulator_wifi_target`, `listen_notice` | `config::tests::ipv4_loopback_and_wildcard_are_dialed_as_the_emulator_alias` |
| [ok] | Any other bind, including the rest of `127.0.0.0/8` and any IPv6 address, is dialed as itself. The notice says `10.0.2.2` reaches `127.0.0.1` only | `src/config.rs` | `config::tests::other_binds_are_not_the_emulator_alias` |
| [ok] | `--listen 10.0.2.2:35000` is an error that names that address, the loopback bind, and the Wi-Fi host | `src/config.rs` `from_args_and_env` | `config::tests::emulator_alias_is_refused_as_a_bind_address` |

## Configuration

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | The default GATT service is the 16-bit value `FFF0` expanded onto the Bluetooth base UUID, and both the short form and the dashed 128-bit form parse to it | `docs/ble-re.md`; `DEFAULT_SERVICE_UUID` | `config::tests::short_uuid_expands_onto_bluetooth_base`; `config::tests::parse_uuid_accepts_short_and_full` |
| [ok] | A Bluetooth address is 12 hex digits, with colons or dashes optional | `src/config.rs` `parse_mac` | `config::tests::parse_mac_handles_separators` |
| [ok] | `--listen`, `--name`, and `--mtu` override those fields, and an omitted `--service` stays `FFF0` | `src/config.rs` | `config::tests::args_override_defaults` |
| [ok] | The serial GATT service is a first-class choice: `--service` names a built-in preset (`fff0`, `nordic-uart`, `microchip-uart`, `hm10`, with aliases) or a raw UUID; a preset supplies the service and, where fixed, the write/notify characteristics; `--write`/`--notify` override per field; omitting `--service` stays the `FFF0`-by-property default | `src/config.rs` `Preset`, `PRESETS`, `find_preset`, `resolve_serial` | `config::tests::find_preset_matches_name_and_aliases_case_insensitively`; `config::tests::resolve_serial_defaults_to_fff0_by_property`; `config::tests::resolve_serial_expands_a_preset_name`; `config::tests::resolve_serial_accepts_a_raw_uuid_as_the_service`; `config::tests::resolve_serial_lets_explicit_chars_override_a_preset`; `config::tests::resolve_serial_rejects_an_unknown_service`; `config::tests::service_preset_flows_through_from_args_and_env` |
| [ok] | The Nordic UART, Microchip/ISSC Transparent UART and HM-10 preset UUIDs match their documented values; every preset name/alias is unique and resolves | `src/config.rs` `NORDIC_UART_*`, `MICROCHIP_UART_*`, `HM10_*`, `PRESETS` | `config::tests::nordic_uart_constants_round_trip_through_uuid_text`; `config::tests::microchip_and_hm10_presets_carry_their_characteristics`; `config::tests::every_preset_name_and_alias_is_unique` |
| [ok] | `--help` returns no config, and an unknown flag is an error | `src/config.rs` | `config::tests::help_returns_none`; `config::tests::unknown_flag_errors` |
| [open] | The live scan applies `--name` and `--mac` to an advertisement | `src/ble/win.rs` `scan_for_adapter` | none: needs a dongle |

## Pre-flight scan (doctor)

The `scan` subcommand (alias `doctor`) is a read-only check an operator runs before a capture: it
lists the host radio and the BLE advertisers it sees, marking the one that carries the serial
service, then exits. It opens no GATT connection and binds no socket.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | The `scan` (alias `doctor`) subcommand selects the doctor mode; without it the bridge runs | `src/config.rs` `from_args_and_env`, `Mode` | `config::tests::scan_subcommand_sets_doctor_mode` |
| [ok] | A seen advertiser renders as address, signal, name and services, marking the one that advertises the serial service; a UUID on the Bluetooth base shows in its short form | `src/config.rs` `doctor_row`, `uuid_label`, `format_addr` | `config::tests::doctor_row_marks_the_serial_service`; `config::tests::uuid_label_shortens_base_uuids_only`; `config::tests::format_addr_is_colon_hex` |
| [ok] | A `WinRT` GUID round-trips to its 128-bit UUID | `src/ble/win.rs` `guid_to_u128` | `ble::win::tests::guid_to_u128_inverts_from_u128` |
| [ok] | Collected advertisers are filtered by name/address, flagged for the serial service, and ordered serial-service first then strongest signal | `src/config.rs` `rank_advertisers`, `ScanFilter` | `config::tests::rank_advertisers_filters_flags_and_orders` |
| [ok] | `scan --json` emits the advertisers as a JSON array (address, name, rssi, services, serial flag) with control characters escaped; an empty scan is `[]` | `src/config.rs` `adverts_json`, `json_escape`; `src/ble/win.rs` `doctor` | `config::tests::adverts_json_empty_is_an_empty_array`; `config::tests::adverts_json_renders_rows_with_escaping`; `config::tests::json_escape_covers_control_characters`; `config::tests::json_flag_defaults_off_and_is_set_by_scan_json` |
| [open] | The live `scan` reports the host radio and lists nearby BLE advertisers over the radio; with `--json` the advertisers go to stdout and the notes to stderr | `src/ble/win.rs` `doctor`, `scan_advertisers` | none: needs a dongle |

## Connect-and-pipe (pipe)

The `pipe` subcommand connects to the dongle (same `--name`/`--mac`/`--service` selection as the
bridge) and relays its serial stream on stdin/stdout, with no TCP server. It is a connect-and-pipe
demo of the library's scan/connect path, for quick local checks and scripting.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | The `pipe` subcommand selects pipe mode, and the selection flags still parse after it | `src/config.rs` `take_mode`, `Mode::Pipe` | `config::tests::pipe_subcommand_sets_pipe_mode` |
| [open] | `pipe` connects, relays stdin→dongle and dongle→stdout, and ends on stdin EOF or a link drop | `src/main.rs` `pipe` | none: needs Windows and a dongle |

## Library API

The crate is a library (`[lib] dangle`) plus a thin binary built on it, so a sibling crate
(e.g. `voltkit`) can do host-OS BLE in-process rather than spawning the binary. `src/lib.rs`
re-exports `scan_advertisers`, `connect`, `BleSerial`, `SerialLink`, the config types/helpers, and
`BridgeResult`. The live BLE items are `#[cfg(windows)]`; a non-Windows build gets a compiling stub.

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [ok] | `scan_advertisers` returns typed `Advert` rows (address, name, rssi, service UUIDs, serial flag), serial-service first then strongest, as data not stdout; the binary's `scan` just formats them | `src/ble/win.rs` `scan_advertisers`; `src/config.rs` `rank_advertisers` | `config::tests::rank_advertisers_filters_flags_and_orders` |
| [ok] | `Config` yields a `SerialConfig` (service/char/MTU) for `connect` and a `ScanFilter` (name/address) for `scan_advertisers` | `src/config.rs` `Config::serial`, `Config::scan_filter` | `config::tests::config_derives_serial_and_scan_filter` |
| [open] | `connect(address, &SerialConfig)` opens the serial service, subscribes, and exposes byte read/write against a real dongle | `src/ble/win.rs` `connect`, `BleSerial` | none: needs Windows and a dongle |
| [open] | On a non-Windows build the library items compile and return the WinRT requirement; `BleSerial` is uninhabited | `src/ble/unsupported.rs` | none: not cross-built here |

## Future ideas (floated, not planned)

Not scheduled, no obligation rows, nothing started — recorded so the idea is not lost. Each stays
out of `RUST_OWNED` and `projecthub sync` until it is actually picked up.

- **Visual patchbay GUI (optional extra).** A graphical front-end that represents each device as a
  module on a canvas and lets the operator wire them together with virtual "patch cables", loosely
  after a modular-rack routing view (see Reason's audio/CV routing:
  <https://docs.reasonstudios.com/reason14/routing-audio-and-cv>). In dangle's terms a module would
  be a scanned BLE dongle (from `scan`/`scan_advertisers`) or a sink (a TCP endpoint, a file, another
  dongle), and a cable would be a `connect` + byte pipe between two endpoints — a visual way to build
  the same bridges the CLI builds today. It would be a **separate crate/binary** layered on this
  library's public API (`scan_advertisers`, `connect`, `bridge::run`), keeping this crate a headless
  transport bridge; it must not pull a GUI toolkit or an async runtime into `dangle` itself. Open
  questions: which GUI stack (and whether it honours the first-party-only policy), and whether the
  patch graph is a throwaway demo or a saved document.

## Left unbuilt

| Status | Obligation | Anchor | Test |
|---|---|---|---|
| [no] | Parse ELM327, UDS or any other wire protocol, or issue a write of its own. The client speaks the protocol. This crate moves bytes | `AGENTS.md`; `README.md` | none: there is no parser |
| [no] | A J2534 pass-through. No `PassThru` export and no `PassThruSupport.04.04` writer. SAE J2534 is not imported | `PORTFOLIO.md` section 3; `AGENTS.md` | none: the calls stay unbuilt |
| [no] | Serve the lab path. That path uses a stub listener on `127.0.0.1:35000` and does not share this radio | `voltkit` `tests/lab.rs`; `voltkit` `src/lab.rs` | none: the stub is not this binary |
