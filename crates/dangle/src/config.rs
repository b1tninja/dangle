//! Runtime configuration, parsed from command-line arguments and environment variables.
//!
//! Every option has an environment-variable fallback so the bridge can be configured either way;
//! an explicit command-line flag wins over the environment. See [`Config::from_args_and_env`].

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::capture::Format;

/// The Bluetooth SIG base UUID `00000000-0000-1000-8000-00805F9B34FB`, into which a 16-bit
/// "short" UUID is slotted at bits 96..=111. How the default serial profile's 16-bit assignments
/// were chosen (from an `OBDLink` adapter, as a worked example) is in `docs/ble-re.md`.
const BLUETOOTH_BASE_UUID: u128 = 0x0000_0000_0000_1000_8000_0080_5f9b_34fb;

/// Expand a 16-bit "short" Bluetooth UUID into its full 128-bit form.
#[must_use]
pub const fn short_uuid(short: u16) -> u128 {
    BLUETOOTH_BASE_UUID | ((short as u128) << 96)
}

/// Default GATT service that carries the serial stream on `FFF0`-style BLE serial dongles
/// (16-bit `0xFFF0`). A common default, not a universal one: override it for dongles that use
/// another service (e.g. a Nordic-UART clone). See `docs/ble-re.md` for how this default was chosen.
pub const DEFAULT_SERVICE_UUID: u128 = short_uuid(0xFFF0);

/// Nordic UART Service (NUS) base service UUID `6E400001-B5A3-F393-E0A9-E50E24DCCA9E`.
///
/// NUS is a widely cloned 128-bit serial profile (distinct from the `FFF0` family); many non-OBD
/// BLE serial dongles expose it. It is selectable by name as the `nordic-uart` preset. The
/// characteristic UUIDs follow by incrementing the first 32-bit field: write
/// ([`NORDIC_UART_WRITE_UUID`], `…0002…`) and notify ([`NORDIC_UART_NOTIFY_UUID`], `…0003…`).
pub const NORDIC_UART_SERVICE_UUID: u128 = 0x6e40_0001_b5a3_f393_e0a9_e50e_24dc_ca9e;

/// Nordic UART Service write characteristic `6E400002-B5A3-F393-E0A9-E50E24DCCA9E` (host → dongle).
pub const NORDIC_UART_WRITE_UUID: u128 = 0x6e40_0002_b5a3_f393_e0a9_e50e_24dc_ca9e;

/// Nordic UART Service notify characteristic `6E400003-B5A3-F393-E0A9-E50E24DCCA9E` (dongle → host).
pub const NORDIC_UART_NOTIFY_UUID: u128 = 0x6e40_0003_b5a3_f393_e0a9_e50e_24dc_ca9e;

/// Default TCP endpoint the bridge listens on. Loopback-only by default so the republished
/// dongle is not exposed to the network without a deliberate choice.
///
/// The standard Android emulator reaches this socket as [`EMULATOR_HOST`] on the same port.
/// That address is the emulator's name for the host's IPv4 loopback; it is not a bind address.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:35000";

/// IPv4 address the standard Android emulator uses to reach the host's loopback
/// (`127.0.0.1` on the machine running the bridge).
///
/// The address is assigned inside the emulator's network. The bridge binds a host address
/// (by default [`DEFAULT_LISTEN`]) and the emulator's Wi-Fi client dials this alias.
pub const EMULATOR_HOST: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);

/// Default maximum number of payload bytes written to the adapter in a single GATT write.
/// 20 bytes is the safe ATT payload for the default 23-byte MTU; raise it only if the adapter
/// and stack negotiate a larger MTU.
pub const DEFAULT_MAX_WRITE: usize = 20;

/// Default time to scan for a matching adapter before giving up one attempt.
pub const DEFAULT_SCAN_SECS: u64 = 20;

/// What the binary should do this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Scan for the configured dongle, connect, and run the TCP bridge (the default).
    Bridge,
    /// List the host radio and nearby BLE advertisers, then exit (the `scan` subcommand).
    Doctor,
}

/// Fully resolved configuration for one run of the bridge.
#[derive(Debug, Clone)]
pub struct Config {
    /// What to do this run: run the bridge, or just scan and report.
    pub mode: Mode,
    /// Address the TCP server binds to.
    pub listen: SocketAddr,
    /// Case-insensitive substring the adapter's advertised name must contain, if set.
    pub name_filter: Option<String>,
    /// Exact Bluetooth device address (48-bit) the adapter must have, if set.
    pub mac_filter: Option<u64>,
    /// GATT service UUID carrying the serial stream (128-bit).
    pub service_uuid: u128,
    /// Optional explicit write-characteristic UUID; when `None` it is discovered by property.
    pub write_uuid: Option<u128>,
    /// Optional explicit notify-characteristic UUID; when `None` it is discovered by property.
    pub notify_uuid: Option<u128>,
    /// Maximum payload bytes per GATT write.
    pub max_write: usize,
    /// How long one scan attempt runs before retrying.
    pub scan_timeout: Duration,
    /// Optional capture of the relayed traffic.
    pub capture: Option<CaptureConfig>,
}

/// Where and how to record the relayed traffic (`--capture`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureConfig {
    /// File to create (truncated if it exists).
    pub path: PathBuf,
    /// File format.
    pub format: Format,
}

/// Error produced while parsing configuration.
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Short usage text, printed for `--help` or on a parse error.
pub const USAGE: &str = "\
dangle - bridge a Bluetooth LE serial dongle's GATT serial stream to a TCP server

USAGE:
    dangle [OPTIONS]                 connect the dongle and run the TCP bridge
    dangle scan [OPTIONS]            list the host radio and nearby BLE advertisers, then exit

The `scan` subcommand is a pre-flight doctor: it shows which BLE devices the host radio can see,
their addresses and signal, and which one advertises the serial service (default FFF0) - so you
can confirm the dongle is visible and read off the --mac/--service to use, without connecting.

OPTIONS:
    --listen <ADDR>      TCP listen address            (env DANGLE_LISTEN) [default 127.0.0.1:35000]
    --name <SUBSTR>      adapter name must contain     (env DANGLE_NAME)
    --mac <AA:BB:..>     adapter Bluetooth address     (env DANGLE_MAC)
    --service <PRESET|UUID>  serial GATT service        (env DANGLE_SERVICE) [default fff0]
    --write <UUID>       write characteristic UUID     (env DANGLE_WRITE)  [default: preset/by property]
    --notify <UUID>      notify characteristic UUID    (env DANGLE_NOTIFY) [default: preset/by property]
    --mtu <N>            max payload bytes per write   (env DANGLE_MTU)    [default 20]
    --scan-timeout <S>   seconds per scan attempt      (env DANGLE_SCAN)   [default 20]
    --capture <PATH>     record relayed traffic to PATH (env DANGLE_CAPTURE) [default: off]
    --capture-format <F> pcap | btsnoop | text         (env DANGLE_CAPTURE_FORMAT) [default pcap]
    -h, --help           print this help

SERVICE PRESETS (--service <name>, a named profile that sets the service and, where the profile
fixes them, the write/notify characteristics; --write/--notify still override):
    fff0          FFF0-style serial service, characteristics by property; a common default on
                  many ELM327/OBD dongles (aliases: obd, elm327)
    nordic-uart   Nordic UART Service (6E400001-…): write 6E400002-…, notify 6E400003-…
                  (aliases: nordic, nus)

Instead of a preset name, --service / --write / --notify accept a UUID: a 4-hex-digit short form
(e.g. FFF0) or a full 128-bit UUID (e.g. 0000fff0-0000-1000-8000-00805f9b34fb).";

/// Parse a UUID given as a 4-hex short form or a full 36-character (dashed) / 32-hex UUID.
///
/// # Errors
/// Returns [`ConfigError`] if the string is neither a 16-bit short nor a 128-bit UUID.
pub fn parse_uuid(s: &str) -> Result<u128, ConfigError> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    match hex.len() {
        4 => u16::from_str_radix(&hex, 16)
            .map(short_uuid)
            .map_err(|_| ConfigError(format!("invalid short UUID: {s:?}"))),
        32 => u128::from_str_radix(&hex, 16)
            .map_err(|_| ConfigError(format!("invalid 128-bit UUID: {s:?}"))),
        _ => Err(ConfigError(format!(
            "UUID must be 4 or 32 hex digits (dashes ignored), got {s:?}"
        ))),
    }
}

/// Parse a Bluetooth device address such as `AA:BB:CC:DD:EE:FF` (colons/dashes optional) into the
/// 48-bit integer the `WinRT` API uses.
///
/// # Errors
/// Returns [`ConfigError`] if the value is not 12 hex digits.
pub fn parse_mac(s: &str) -> Result<u64, ConfigError> {
    let hex: String = s.chars().filter(|c| *c != ':' && *c != '-').collect();
    if hex.len() != 12 {
        return Err(ConfigError(format!(
            "Bluetooth address must be 12 hex digits, got {s:?}"
        )));
    }
    u64::from_str_radix(&hex, 16)
        .map_err(|_| ConfigError(format!("invalid Bluetooth address: {s:?}")))
}

/// Format a 48-bit Bluetooth address as `AA:BB:CC:DD:EE:FF`.
#[must_use]
pub fn format_addr(addr: u64) -> String {
    let b = addr.to_be_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

/// Render a 128-bit UUID as its 4-hex short form when it sits on the Bluetooth SIG base,
/// otherwise as the full dashed 36-character form.
#[must_use]
pub fn uuid_label(u: u128) -> String {
    // Clear the 16-bit "short" slot (bits 96..=111); if the rest equals the base, it is a short UUID.
    let base_mask: u128 = !(0xffff_u128 << 96);
    if (u & base_mask) == BLUETOOTH_BASE_UUID {
        format!("{:04X}", (u >> 96) & 0xffff)
    } else {
        let b = u.to_be_bytes();
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

/// Accumulator for one advertiser while a scan is in progress: the strongest signal and the union
/// of names/services seen across its advertisement packets. [`rank_advertisers`] turns a set of
/// these into the public [`Advert`] rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// 48-bit Bluetooth device address.
    pub addr: u64,
    /// Advertised local name, empty when none was advertised.
    pub name: String,
    /// Strongest signal seen, in dBm.
    pub rssi: i16,
    /// Advertised 128-bit service UUIDs.
    pub services: Vec<u128>,
}

/// A Bluetooth LE advertiser returned by a scan, with the serial-service flag already resolved.
///
/// This is the typed result [`crate::ble::scan_advertisers`] produces (the same data the `scan`
/// doctor prints, but as data rather than stdout). A consumer such as `voltkit` reads these to
/// present a chooser and then calls [`crate::ble::connect`] with the chosen [`Advert::address`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advert {
    /// 48-bit Bluetooth device address, as [`format_addr`] renders it.
    pub address: u64,
    /// Advertised local name, empty when none was advertised.
    pub name: String,
    /// Strongest signal seen during the scan, in dBm (higher is closer).
    pub rssi: i16,
    /// Advertised 128-bit service UUIDs.
    pub services: Vec<u128>,
    /// Whether [`services`](Self::services) includes the serial service the scan looked for
    /// (default [`DEFAULT_SERVICE_UUID`], 16-bit `0xFFF0`).
    pub has_serial_service: bool,
}

/// Filters applied to a scan: an optional case-insensitive name substring and an optional exact
/// device address. `Default` (both `None`) matches every advertiser.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanFilter {
    /// Keep only advertisers whose local name contains this substring (case-insensitive).
    pub name_contains: Option<String>,
    /// Keep only the advertiser with this exact 48-bit address.
    pub address: Option<u64>,
}

impl ScanFilter {
    /// Whether an advertiser with `name`/`addr` passes this filter.
    #[must_use]
    pub fn matches(&self, name: &str, addr: u64) -> bool {
        self.name_contains
            .as_deref()
            .is_none_or(|n| name.to_lowercase().contains(&n.to_lowercase()))
            && self.address.is_none_or(|a| a == addr)
    }
}

/// The GATT parameters needed to open and drive the serial service: the subset of [`Config`] that
/// the connect path uses. `Default` is the common `FFF0`-style serial profile (serial service
/// `0xFFF0`, write/notify characteristics discovered by property, 20-byte writes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConfig {
    /// GATT service UUID carrying the serial stream (128-bit).
    pub service_uuid: u128,
    /// Optional explicit write-characteristic UUID; when `None` it is discovered by property.
    pub write_uuid: Option<u128>,
    /// Optional explicit notify-characteristic UUID; when `None` it is discovered by property.
    pub notify_uuid: Option<u128>,
    /// Maximum payload bytes per GATT write.
    pub max_write: usize,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            service_uuid: DEFAULT_SERVICE_UUID,
            write_uuid: None,
            notify_uuid: None,
            max_write: DEFAULT_MAX_WRITE,
        }
    }
}

/// A named BLE serial profile: a GATT service UUID and, where the profile fixes them, the write and
/// notify characteristic UUIDs. Presets make the service UUID a first-class, user-selectable choice
/// (`--service <name>`) rather than only a raw UUID, so the bridge is not tied to the OBD-style
/// `FFF0` default. [`PRESETS`] lists the built-in profiles; [`find_preset`] looks one up by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// Canonical preset name, as typed on the CLI (e.g. `"nordic-uart"`).
    pub name: &'static str,
    /// Accepted alternative names for the same preset.
    pub aliases: &'static [&'static str],
    /// GATT service UUID carrying the serial stream.
    pub service_uuid: u128,
    /// Write-characteristic UUID when the profile fixes it; `None` means discover it by property.
    pub write_uuid: Option<u128>,
    /// Notify-characteristic UUID when the profile fixes it; `None` means discover it by property.
    pub notify_uuid: Option<u128>,
    /// One-line human description, shown in `--help`.
    pub description: &'static str,
}

impl Preset {
    /// Whether `name` is this preset's canonical name or one of its aliases (case-insensitive).
    #[must_use]
    pub fn matches_name(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }

    /// The [`SerialConfig`] this preset implies, with the default write size.
    #[must_use]
    pub fn serial_config(&self) -> SerialConfig {
        SerialConfig {
            service_uuid: self.service_uuid,
            write_uuid: self.write_uuid,
            notify_uuid: self.notify_uuid,
            max_write: DEFAULT_MAX_WRITE,
        }
    }
}

/// The built-in serial profiles selectable by name on `--service`.
///
/// `fff0` is the default profile (it expands to [`DEFAULT_SERVICE_UUID`]); `nordic-uart` is the
/// Nordic UART Service. Both can still be overridden per field with `--service <uuid>` / `--write`
/// / `--notify` for a dongle that matches neither.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "fff0",
        aliases: &["obd", "elm327"],
        service_uuid: DEFAULT_SERVICE_UUID,
        write_uuid: None,
        notify_uuid: None,
        description: "FFF0-style serial service, characteristics by property; a common default on \
                      many ELM327/OBD dongles",
    },
    Preset {
        name: "nordic-uart",
        aliases: &["nordic", "nus"],
        service_uuid: NORDIC_UART_SERVICE_UUID,
        write_uuid: Some(NORDIC_UART_WRITE_UUID),
        notify_uuid: Some(NORDIC_UART_NOTIFY_UUID),
        description: "Nordic UART Service (6E400001-…): write 6E400002-…, notify 6E400003-…",
    },
];

/// Look up a built-in [`Preset`] by canonical name or alias (case-insensitive).
#[must_use]
pub fn find_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.matches_name(name))
}

/// Resolve the serial GATT parameters from the `--service` / `--write` / `--notify` values.
///
/// `service` may name a built-in [`Preset`] (e.g. `nordic-uart` or `fff0`) or a raw UUID. A preset
/// supplies the service UUID and, where it fixes them, the write/notify characteristics; an explicit
/// `write`/`notify` overrides the preset's. A raw UUID sets only the service, leaving the
/// characteristics to property-based discovery. When `service` is `None` the default is the `FFF0`
/// profile with characteristics discovered by property — the historical default behaviour.
///
/// # Errors
/// Returns [`ConfigError`] if `service` is neither a known preset nor a valid UUID, or if `write`
/// or `notify` is not a valid UUID.
pub fn resolve_serial(
    service: Option<&str>,
    write: Option<&str>,
    notify: Option<&str>,
) -> Result<(u128, Option<u128>, Option<u128>), ConfigError> {
    let (service_uuid, base_write, base_notify) = match service {
        None => (DEFAULT_SERVICE_UUID, None, None),
        Some(s) => match find_preset(s) {
            Some(p) => (p.service_uuid, p.write_uuid, p.notify_uuid),
            None => (parse_uuid(s)?, None, None),
        },
    };
    let write_uuid = match write {
        Some(w) => Some(parse_uuid(w)?),
        None => base_write,
    };
    let notify_uuid = match notify {
        Some(n) => Some(parse_uuid(n)?),
        None => base_notify,
    };
    Ok((service_uuid, write_uuid, notify_uuid))
}

/// Whether `services` includes the serial service `serial`.
#[must_use]
pub fn advertises(services: &[u128], serial: u128) -> bool {
    services.contains(&serial)
}

/// Apply `filter`, mark the serial-service carriers, and order the survivors serial-service first,
/// then strongest signal. This is the platform-independent core of a scan: the `WinRT` watcher
/// collects a [`Seen`] per address and hands them here, so the filtering, flagging and ordering are
/// testable without a radio.
#[must_use]
pub fn rank_advertisers(
    seen: impl IntoIterator<Item = Seen>,
    filter: &ScanFilter,
    serial: u128,
) -> Vec<Advert> {
    let mut rows: Vec<Advert> = seen
        .into_iter()
        .filter(|s| filter.matches(&s.name, s.addr))
        .map(|s| {
            let has_serial_service = advertises(&s.services, serial);
            Advert {
                address: s.addr,
                name: s.name,
                rssi: s.rssi,
                services: s.services,
                has_serial_service,
            }
        })
        .collect();
    // Serial-service advertisers first, then strongest signal.
    rows.sort_by(|a, b| {
        b.has_serial_service
            .cmp(&a.has_serial_service)
            .then(b.rssi.cmp(&a.rssi))
    });
    rows
}

/// One display row for a ranked advertiser, marking the one that carries the serial service.
#[must_use]
pub fn doctor_row(advert: &Advert, serial: u128) -> String {
    let name = if advert.name.is_empty() {
        "<unnamed>"
    } else {
        advert.name.as_str()
    };
    let services = if advert.services.is_empty() {
        "-".to_owned()
    } else {
        advert
            .services
            .iter()
            .map(|u| uuid_label(*u))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mark = if advert.has_serial_service {
        format!("  <- serial service ({})", uuid_label(serial))
    } else {
        String::new()
    };
    format!(
        "{}  {:>5} dBm  {name}  [{services}]{mark}",
        format_addr(advert.address),
        advert.rssi
    )
}

/// Host and port an Android emulator's Wi-Fi OBD client should dial to reach `bound`.
///
/// Returns `Some` when `bound` is `127.0.0.1` or IPv4 `0.0.0.0`. The emulator forwards
/// [`EMULATOR_HOST`] to the host's `127.0.0.1`, and both of those sockets accept that
/// connection. The port is the port the bridge actually bound.
///
/// Returns `None` for every other address, including the rest of `127.0.0.0/8` and any
/// IPv6 address. [`EMULATOR_HOST`] is forwarded only to `127.0.0.1`; for any other bind
/// address the client uses `bound` itself.
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn emulator_wifi_target(bound: SocketAddr) -> Option<(String, u16)> {
    let reachable = match bound.ip() {
        IpAddr::V4(ip) => ip == Ipv4Addr::LOCALHOST || ip.is_unspecified(),
        IpAddr::V6(_) => false,
    };
    if reachable {
        Some((EMULATOR_HOST.to_string(), bound.port()))
    } else {
        None
    }
}

/// Text to print once the TCP socket is bound.
///
/// The first line is the host bind address. When [`emulator_wifi_target`] applies, the
/// following lines are the Wi-Fi host and port to enter in the emulator, plus the
/// `adb reverse` fallback the emulator uses when `10.0.2.2` is not reachable (the app
/// then dials `127.0.0.1` on the same port). Otherwise the lines say the client uses
/// `bound` itself.
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn listen_notice(bound: SocketAddr) -> String {
    match emulator_wifi_target(bound) {
        Some((host, port)) => format!(
            "listening on {bound}\n\
             emulator Wi-Fi adapter: host {host} port {port}\n\
             {EMULATOR_HOST} is the emulator's name for this host's IPv4 loopback\n\
             if the emulator cannot open {host}: adb -s emulator-<digits> reverse tcp:{port} tcp:{port}, then use host 127.0.0.1"
        ),
        None => format!(
            "listening on {bound}\n\
             Wi-Fi client host:port is {bound}\n\
             the emulator alias {EMULATOR_HOST} reaches 127.0.0.1 only"
        ),
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Consume a leading `scan`/`doctor` subcommand, if present, and return the resulting mode.
fn take_mode<I: Iterator<Item = String>>(it: &mut std::iter::Peekable<I>) -> Mode {
    if matches!(it.peek().map(String::as_str), Some("scan" | "doctor")) {
        it.next();
        Mode::Doctor
    } else {
        Mode::Bridge
    }
}

impl Config {
    /// The GATT serial parameters this configuration implies, for [`crate::ble::connect`].
    #[must_use]
    pub fn serial(&self) -> SerialConfig {
        SerialConfig {
            service_uuid: self.service_uuid,
            write_uuid: self.write_uuid,
            notify_uuid: self.notify_uuid,
            max_write: self.max_write,
        }
    }

    /// The name/address filter this configuration implies, for [`crate::ble::scan_advertisers`].
    #[must_use]
    pub fn scan_filter(&self) -> ScanFilter {
        ScanFilter {
            name_contains: self.name_filter.clone(),
            address: self.mac_filter,
        }
    }

    /// Build a [`Config`] from process arguments and environment variables.
    ///
    /// Returns `Ok(None)` when `--help` was requested (the caller should print [`USAGE`] and exit 0).
    ///
    /// # Errors
    /// Returns [`ConfigError`] for an unknown flag, a missing value, an unparseable value,
    /// or a `--listen` address equal to [`EMULATOR_HOST`].
    pub fn from_args_and_env<I>(args: I) -> Result<Option<Self>, ConfigError>
    where
        I: IntoIterator<Item = String>,
    {
        let mut listen = env("DANGLE_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.to_owned());
        let mut name_filter = env("DANGLE_NAME");
        let mut mac = env("DANGLE_MAC");
        let mut service = env("DANGLE_SERVICE");
        let mut write = env("DANGLE_WRITE");
        let mut notify = env("DANGLE_NOTIFY");
        let mut mtu = env("DANGLE_MTU");
        let mut scan = env("DANGLE_SCAN");
        let mut capture = env("DANGLE_CAPTURE");
        let mut capture_format = env("DANGLE_CAPTURE_FORMAT");

        let mut it = args.into_iter().peekable();
        let mode = take_mode(&mut it);
        while let Some(arg) = it.next() {
            let mut take = |flag: &str| -> Result<String, ConfigError> {
                it.next()
                    .ok_or_else(|| ConfigError(format!("{flag} needs a value")))
            };
            match arg.as_str() {
                "-h" | "--help" => return Ok(None),
                "--listen" => listen = take("--listen")?,
                "--name" => name_filter = Some(take("--name")?),
                "--mac" => mac = Some(take("--mac")?),
                "--service" => service = Some(take("--service")?),
                "--write" => write = Some(take("--write")?),
                "--notify" => notify = Some(take("--notify")?),
                "--mtu" => mtu = Some(take("--mtu")?),
                "--scan-timeout" => scan = Some(take("--scan-timeout")?),
                "--capture" => capture = Some(take("--capture")?),
                "--capture-format" => capture_format = Some(take("--capture-format")?),
                other => return Err(ConfigError(format!("unknown argument: {other:?}"))),
            }
        }

        let listen: SocketAddr = listen
            .parse()
            .map_err(|e| ConfigError(format!("invalid --listen {listen:?}: {e}")))?;
        if listen.ip() == IpAddr::V4(EMULATOR_HOST) {
            let port = listen.port();
            return Err(ConfigError(format!(
                "{listen} is the Android emulator's name for this host's IPv4 loopback. \
                 Bind 127.0.0.1:{port} and set the emulator's Wi-Fi host to {EMULATOR_HOST} port {port}"
            )));
        }
        let mac_filter = match mac {
            Some(m) => Some(parse_mac(&m)?),
            None => None,
        };
        let (service_uuid, write_uuid, notify_uuid) =
            resolve_serial(service.as_deref(), write.as_deref(), notify.as_deref())?;
        let max_write = match mtu {
            Some(n) => n
                .parse::<usize>()
                .map_err(|_| ConfigError(format!("invalid --mtu {n:?}")))
                .and_then(|n| {
                    if n == 0 {
                        Err(ConfigError("--mtu must be at least 1".to_owned()))
                    } else {
                        Ok(n)
                    }
                })?,
            None => DEFAULT_MAX_WRITE,
        };
        let scan_timeout = match scan {
            Some(s) => Duration::from_secs(
                s.parse::<u64>()
                    .map_err(|_| ConfigError(format!("invalid --scan-timeout {s:?}")))?,
            ),
            None => Duration::from_secs(DEFAULT_SCAN_SECS),
        };

        let format = match capture_format {
            Some(f) => Format::parse(&f).map_err(ConfigError)?,
            None => Format::Pcap,
        };
        let capture = capture.map(|p| CaptureConfig {
            path: PathBuf::from(p),
            format,
        });

        Ok(Some(Config {
            mode,
            listen,
            name_filter,
            mac_filter,
            service_uuid,
            write_uuid,
            notify_uuid,
            max_write,
            scan_timeout,
            capture,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_uuid_expands_onto_bluetooth_base() {
        // 0xFFF0 is the recovered `OBDLink` serial service; it must land on the SIG base UUID.
        assert_eq!(
            short_uuid(0xFFF0),
            0x0000_fff0_0000_1000_8000_0080_5f9b_34fb
        );
    }

    #[test]
    fn parse_uuid_accepts_short_and_full() {
        assert_eq!(parse_uuid("FFF0").unwrap(), DEFAULT_SERVICE_UUID);
        assert_eq!(
            parse_uuid("0000fff0-0000-1000-8000-00805f9b34fb").unwrap(),
            DEFAULT_SERVICE_UUID
        );
        assert!(parse_uuid("xyz").is_err());
        assert!(parse_uuid("fff").is_err());
    }

    #[test]
    fn parse_mac_handles_separators() {
        assert_eq!(
            parse_mac("AA:BB:CC:DD:EE:FF").unwrap(),
            0x0000_AABB_CCDD_EEFF
        );
        assert_eq!(parse_mac("aabbccddeeff").unwrap(), 0x0000_AABB_CCDD_EEFF);
        assert!(parse_mac("AA:BB").is_err());
    }

    #[test]
    fn args_override_defaults() {
        let args = [
            "--listen",
            "0.0.0.0:9000",
            "--name",
            "`OBDLink`",
            "--mtu",
            "40",
        ]
        .into_iter()
        .map(String::from);
        let cfg = Config::from_args_and_env(args).unwrap().unwrap();
        assert_eq!(cfg.listen.to_string(), "0.0.0.0:9000");
        assert_eq!(cfg.name_filter.as_deref(), Some("`OBDLink`"));
        assert_eq!(cfg.max_write, 40);
        assert_eq!(cfg.service_uuid, DEFAULT_SERVICE_UUID);
    }

    #[test]
    fn capture_defaults_off_and_flags_enable_it() {
        let none = Config::from_args_and_env(std::iter::empty())
            .unwrap()
            .unwrap();
        assert_eq!(none.capture, None);

        let args = ["--capture", "run.pcap"].into_iter().map(String::from);
        let cfg = Config::from_args_and_env(args).unwrap().unwrap();
        assert_eq!(
            cfg.capture,
            Some(CaptureConfig {
                path: PathBuf::from("run.pcap"),
                format: Format::Pcap
            })
        );

        let args = ["--capture", "x", "--capture-format", "btsnoop"]
            .into_iter()
            .map(String::from);
        let cfg = Config::from_args_and_env(args).unwrap().unwrap();
        assert_eq!(cfg.capture.unwrap().format, Format::Btsnoop);

        let args = ["--capture", "x", "--capture-format", "pcapng"]
            .into_iter()
            .map(String::from);
        assert!(Config::from_args_and_env(args).is_err());
    }

    #[test]
    fn help_returns_none() {
        let args = ["--help"].into_iter().map(String::from);
        assert!(Config::from_args_and_env(args).unwrap().is_none());
    }

    #[test]
    fn scan_subcommand_sets_doctor_mode() {
        for verb in ["scan", "doctor"] {
            let args = [verb, "--name", "OBDLink"].into_iter().map(String::from);
            let cfg = Config::from_args_and_env(args).unwrap().unwrap();
            assert_eq!(cfg.mode, Mode::Doctor);
            assert_eq!(cfg.name_filter.as_deref(), Some("OBDLink"));
        }
        let plain = ["--name", "OBDLink"].into_iter().map(String::from);
        assert_eq!(
            Config::from_args_and_env(plain).unwrap().unwrap().mode,
            Mode::Bridge
        );
    }

    #[test]
    fn format_addr_is_colon_hex() {
        assert_eq!(format_addr(0x0000_AABB_CCDD_EEFF), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn uuid_label_shortens_base_uuids_only() {
        assert_eq!(uuid_label(DEFAULT_SERVICE_UUID), "FFF0");
        assert_eq!(uuid_label(short_uuid(0x180A)), "180A");
        let full = 0x1234_5678_9abc_def0_1122_3344_5566_7788_u128;
        assert_eq!(uuid_label(full), "12345678-9abc-def0-1122-334455667788");
    }

    #[test]
    fn doctor_row_marks_the_serial_service() {
        let serial = DEFAULT_SERVICE_UUID;
        let with = Advert {
            address: 0x0000_AABB_CCDD_EEFF,
            name: "OBDLink MX".to_owned(),
            rssi: -63,
            services: vec![serial, short_uuid(0x180A)],
            has_serial_service: true,
        };
        let row = doctor_row(&with, serial);
        assert!(row.contains("AA:BB:CC:DD:EE:FF"), "{row}");
        assert!(row.contains("-63 dBm"), "{row}");
        assert!(row.contains("OBDLink MX"), "{row}");
        assert!(row.contains("FFF0"), "{row}");
        assert!(row.contains("<- serial service (FFF0)"), "{row}");

        let without = Advert {
            address: 0x0000_1122_3344_5566,
            name: String::new(),
            rssi: -90,
            services: vec![short_uuid(0x180A)],
            has_serial_service: false,
        };
        let row = doctor_row(&without, serial);
        assert!(row.contains("<unnamed>"), "{row}");
        assert!(!row.contains("<- serial service"), "{row}");
        assert!(advertises(&with.services, serial));
        assert!(!advertises(&without.services, serial));
    }

    #[test]
    fn rank_advertisers_filters_flags_and_orders() {
        let serial = DEFAULT_SERVICE_UUID;
        let seen = [
            Seen {
                addr: 0x0000_0000_0000_0001,
                name: "Weak serial".to_owned(),
                rssi: -80,
                services: vec![serial],
            },
            Seen {
                addr: 0x0000_0000_0000_0002,
                name: "Strong other".to_owned(),
                rssi: -40,
                services: vec![short_uuid(0x180A)],
            },
            Seen {
                addr: 0x0000_0000_0000_0003,
                name: "Strong serial".to_owned(),
                rssi: -55,
                services: vec![serial],
            },
            Seen {
                addr: 0x0000_0000_0000_0004,
                name: "Skipme".to_owned(),
                rssi: -30,
                services: vec![serial],
            },
        ];

        // No filter: serial-service carriers first, strongest within each group.
        let ranked = rank_advertisers(seen.iter().cloned(), &ScanFilter::default(), serial);
        let order: Vec<u64> = ranked.iter().map(|a| a.address).collect();
        assert_eq!(order, vec![4, 3, 1, 2]);
        assert!(ranked[0].has_serial_service);
        assert!(!ranked[3].has_serial_service);

        // Name filter is a case-insensitive substring.
        let filter = ScanFilter {
            name_contains: Some("serial".to_owned()),
            address: None,
        };
        let ranked = rank_advertisers(seen.iter().cloned(), &filter, serial);
        let order: Vec<u64> = ranked.iter().map(|a| a.address).collect();
        assert_eq!(order, vec![3, 1]);

        // Address filter keeps exactly one.
        let filter = ScanFilter {
            name_contains: None,
            address: Some(2),
        };
        let ranked = rank_advertisers(seen.iter().cloned(), &filter, serial);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].address, 2);
        assert!(!ranked[0].has_serial_service);
    }

    #[test]
    fn config_derives_serial_and_scan_filter() {
        let args = [
            "--name",
            "OBDLink",
            "--mac",
            "AA:BB:CC:DD:EE:FF",
            "--mtu",
            "40",
        ]
        .into_iter()
        .map(String::from);
        let cfg = Config::from_args_and_env(args).unwrap().unwrap();

        let serial = cfg.serial();
        assert_eq!(serial.service_uuid, DEFAULT_SERVICE_UUID);
        assert_eq!(serial.max_write, 40);
        assert_eq!(serial.write_uuid, None);

        let filter = cfg.scan_filter();
        assert_eq!(filter.name_contains.as_deref(), Some("OBDLink"));
        assert_eq!(filter.address, Some(0x0000_AABB_CCDD_EEFF));
        assert!(filter.matches("my obdlink mx", 0x0000_AABB_CCDD_EEFF));
        assert!(!filter.matches("other", 0x0000_AABB_CCDD_EEFF));
    }

    #[test]
    fn unknown_flag_errors() {
        let args = ["--nope"].into_iter().map(String::from);
        assert!(Config::from_args_and_env(args).is_err());
    }

    #[test]
    fn ipv4_loopback_and_wildcard_are_dialed_as_the_emulator_alias() {
        for text in ["127.0.0.1:35000", "127.0.0.1:9", "0.0.0.0:35000"] {
            let addr: SocketAddr = text.parse().unwrap();
            assert_eq!(
                emulator_wifi_target(addr),
                Some((EMULATOR_HOST.to_string(), addr.port()))
            );
            let notice = listen_notice(addr);
            assert!(notice.starts_with(&format!("listening on {addr}\n")));
            assert!(notice.contains(&format!(
                "emulator Wi-Fi adapter: host {EMULATOR_HOST} port {}",
                addr.port()
            )));
            assert!(notice.contains(&format!(
                "adb -s emulator-<digits> reverse tcp:{} tcp:{}",
                addr.port(),
                addr.port()
            )));
        }
    }

    #[test]
    fn other_binds_are_not_the_emulator_alias() {
        for text in ["127.0.0.2:35000", "[::1]:35000", "192.0.2.1:35000"] {
            let addr: SocketAddr = text.parse().unwrap();
            assert_eq!(emulator_wifi_target(addr), None);
            let notice = listen_notice(addr);
            assert!(notice.contains(&format!("Wi-Fi client host:port is {addr}")));
            assert!(!notice.contains("emulator Wi-Fi adapter:"));
        }
    }

    #[test]
    fn emulator_alias_is_refused_as_a_bind_address() {
        let args = ["--listen", "10.0.2.2:35000"].into_iter().map(String::from);
        let text = Config::from_args_and_env(args).unwrap_err().to_string();
        assert!(text.contains("10.0.2.2:35000"), "{text}");
        assert!(text.contains("127.0.0.1:35000"), "{text}");
        assert!(text.contains("Wi-Fi host to 10.0.2.2 port 35000"), "{text}");
    }

    #[test]
    fn nordic_uart_constants_round_trip_through_uuid_text() {
        // The three NUS UUIDs differ only in the first 32-bit field (…0001/0002/0003).
        assert_eq!(
            parse_uuid("6e400001-b5a3-f393-e0a9-e50e24dcca9e").unwrap(),
            NORDIC_UART_SERVICE_UUID
        );
        assert_eq!(
            parse_uuid("6e400002-b5a3-f393-e0a9-e50e24dcca9e").unwrap(),
            NORDIC_UART_WRITE_UUID
        );
        assert_eq!(
            parse_uuid("6e400003-b5a3-f393-e0a9-e50e24dcca9e").unwrap(),
            NORDIC_UART_NOTIFY_UUID
        );
        // NUS is a full 128-bit UUID, so it renders in the dashed long form, not a short code.
        assert_eq!(
            uuid_label(NORDIC_UART_SERVICE_UUID),
            "6e400001-b5a3-f393-e0a9-e50e24dcca9e"
        );
    }

    #[test]
    fn find_preset_matches_name_and_aliases_case_insensitively() {
        let fff0 = find_preset("fff0").unwrap();
        assert_eq!(fff0.service_uuid, DEFAULT_SERVICE_UUID);
        assert_eq!(fff0.write_uuid, None);
        assert_eq!(fff0.notify_uuid, None);
        // Aliases and case folding both resolve to the same preset.
        assert_eq!(find_preset("OBD"), Some(fff0));
        assert_eq!(find_preset("Elm327"), Some(fff0));

        let nus = find_preset("nordic-uart").unwrap();
        assert_eq!(nus.service_uuid, NORDIC_UART_SERVICE_UUID);
        assert_eq!(nus.write_uuid, Some(NORDIC_UART_WRITE_UUID));
        assert_eq!(nus.notify_uuid, Some(NORDIC_UART_NOTIFY_UUID));
        assert_eq!(find_preset("NUS"), Some(nus));
        assert_eq!(find_preset("nordic"), Some(nus));

        assert_eq!(find_preset("no-such-preset"), None);

        // The preset's SerialConfig carries its characteristics and the default write size.
        assert_eq!(
            nus.serial_config(),
            SerialConfig {
                service_uuid: NORDIC_UART_SERVICE_UUID,
                write_uuid: Some(NORDIC_UART_WRITE_UUID),
                notify_uuid: Some(NORDIC_UART_NOTIFY_UUID),
                max_write: DEFAULT_MAX_WRITE,
            }
        );
    }

    #[test]
    fn resolve_serial_defaults_to_fff0_by_property() {
        // No --service: the historical default, FFF0 with characteristics discovered by property.
        assert_eq!(
            resolve_serial(None, None, None).unwrap(),
            (DEFAULT_SERVICE_UUID, None, None)
        );
    }

    #[test]
    fn resolve_serial_expands_a_preset_name() {
        assert_eq!(
            resolve_serial(Some("nordic-uart"), None, None).unwrap(),
            (
                NORDIC_UART_SERVICE_UUID,
                Some(NORDIC_UART_WRITE_UUID),
                Some(NORDIC_UART_NOTIFY_UUID)
            )
        );
        // The fff0 preset is equivalent to the default.
        assert_eq!(
            resolve_serial(Some("fff0"), None, None).unwrap(),
            (DEFAULT_SERVICE_UUID, None, None)
        );
    }

    #[test]
    fn resolve_serial_accepts_a_raw_uuid_as_the_service() {
        // A raw service UUID sets only the service; characteristics stay by-property.
        assert_eq!(
            resolve_serial(Some("fff0"), None, None).unwrap().0,
            resolve_serial(Some("FFF0"), None, None).unwrap().0
        );
        let (service, write, notify) =
            resolve_serial(Some("1234-5678-9abc-def0-1122-3344-5566-7788"), None, None).unwrap();
        assert_eq!(service, 0x1234_5678_9abc_def0_1122_3344_5566_7788);
        assert_eq!((write, notify), (None, None));
    }

    #[test]
    fn resolve_serial_lets_explicit_chars_override_a_preset() {
        let (service, write, notify) = resolve_serial(
            Some("nordic-uart"),
            Some("FFE1"),
            Some("0000ffe2-0000-1000-8000-00805f9b34fb"),
        )
        .unwrap();
        assert_eq!(service, NORDIC_UART_SERVICE_UUID);
        assert_eq!(write, Some(short_uuid(0xFFE1)));
        assert_eq!(notify, Some(short_uuid(0xFFE2)));
    }

    #[test]
    fn resolve_serial_rejects_an_unknown_service() {
        // Neither a preset nor a valid UUID.
        assert!(resolve_serial(Some("not-a-thing"), None, None).is_err());
        // A preset service but an invalid override UUID.
        assert!(resolve_serial(Some("fff0"), Some("zzzz"), None).is_err());
    }

    #[test]
    fn service_preset_flows_through_from_args_and_env() {
        let args = ["--service", "nordic-uart"].into_iter().map(String::from);
        let cfg = Config::from_args_and_env(args).unwrap().unwrap();
        let serial = cfg.serial();
        assert_eq!(serial.service_uuid, NORDIC_UART_SERVICE_UUID);
        assert_eq!(serial.write_uuid, Some(NORDIC_UART_WRITE_UUID));
        assert_eq!(serial.notify_uuid, Some(NORDIC_UART_NOTIFY_UUID));

        // An explicit --write overrides the preset's write characteristic.
        let args = ["--service", "nus", "--write", "FFE1"]
            .into_iter()
            .map(String::from);
        let cfg = Config::from_args_and_env(args).unwrap().unwrap();
        let serial = cfg.serial();
        assert_eq!(serial.service_uuid, NORDIC_UART_SERVICE_UUID);
        assert_eq!(serial.write_uuid, Some(short_uuid(0xFFE1)));
        assert_eq!(serial.notify_uuid, Some(NORDIC_UART_NOTIFY_UUID));
    }
}
