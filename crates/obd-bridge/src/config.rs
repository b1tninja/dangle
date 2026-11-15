//! Runtime configuration, parsed from command-line arguments and environment variables.
//!
//! Every option has an environment-variable fallback so the bridge can be configured either way;
//! an explicit command-line flag wins over the environment. See [`Config::from_args_and_env`].

use std::net::SocketAddr;
use std::time::Duration;

/// The Bluetooth SIG base UUID `00000000-0000-1000-8000-00805F9B34FB`, into which a 16-bit
/// "short" UUID is slotted at bits 96..=111. Recovered 16-bit assignments for the `OBDLink`
/// family's BLE serial profile are in `docs/ble-re.md`.
const BLUETOOTH_BASE_UUID: u128 = 0x0000_0000_0000_1000_8000_0080_5f9b_34fb;

/// Expand a 16-bit "short" Bluetooth UUID into its full 128-bit form.
#[must_use]
pub const fn short_uuid(short: u16) -> u128 {
    BLUETOOTH_BASE_UUID | ((short as u128) << 96)
}

/// Default GATT service that carries the ELM327 serial stream on `OBDLink`-class BLE adapters
/// (16-bit `0xFFF0`). Overridable because other adapters (e.g. Nordic-UART clones) differ.
pub const DEFAULT_SERVICE_UUID: u128 = short_uuid(0xFFF0);

/// Default TCP endpoint the bridge listens on. Loopback-only by default so the republished
/// adapter is not exposed to the network without a deliberate choice.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:35000";

/// Default maximum number of payload bytes written to the adapter in a single GATT write.
/// 20 bytes is the safe ATT payload for the default 23-byte MTU; raise it only if the adapter
/// and stack negotiate a larger MTU.
pub const DEFAULT_MAX_WRITE: usize = 20;

/// Default time to scan for a matching adapter before giving up one attempt.
pub const DEFAULT_SCAN_SECS: u64 = 20;

/// Fully resolved configuration for one run of the bridge.
#[derive(Debug, Clone)]
pub struct Config {
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
obd-bridge - republish a Bluetooth LE ELM327/STN OBD adapter as an ELM327-over-TCP server

USAGE:
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

A UUID may be a 4-hex-digit short form (e.g. FFF0) or a full 128-bit UUID
(e.g. 0000fff0-0000-1000-8000-00805f9b34fb).";

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

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

impl Config {
    /// Build a [`Config`] from process arguments and environment variables.
    ///
    /// Returns `Ok(None)` when `--help` was requested (the caller should print [`USAGE`] and exit 0).
    ///
    /// # Errors
    /// Returns [`ConfigError`] for an unknown flag, a missing value, or an unparseable value.
    pub fn from_args_and_env<I>(args: I) -> Result<Option<Self>, ConfigError>
    where
        I: IntoIterator<Item = String>,
    {
        let mut listen = env("OBD_BRIDGE_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.to_owned());
        let mut name_filter = env("OBD_BRIDGE_NAME");
        let mut mac = env("OBD_BRIDGE_MAC");
        let mut service = env("OBD_BRIDGE_SERVICE");
        let mut write = env("OBD_BRIDGE_WRITE");
        let mut notify = env("OBD_BRIDGE_NOTIFY");
        let mut mtu = env("OBD_BRIDGE_MTU");
        let mut scan = env("OBD_BRIDGE_SCAN");

        let mut it = args.into_iter();
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
                other => return Err(ConfigError(format!("unknown argument: {other:?}"))),
            }
        }

        let listen: SocketAddr = listen
            .parse()
            .map_err(|e| ConfigError(format!("invalid --listen {listen:?}: {e}")))?;
        let mac_filter = match mac {
            Some(m) => Some(parse_mac(&m)?),
            None => None,
        };
        let service_uuid = match service {
            Some(u) => parse_uuid(&u)?,
            None => DEFAULT_SERVICE_UUID,
        };
        let write_uuid = match write {
            Some(u) => Some(parse_uuid(&u)?),
            None => None,
        };
        let notify_uuid = match notify {
            Some(u) => Some(parse_uuid(&u)?),
            None => None,
        };
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

        Ok(Some(Config {
            listen,
            name_filter,
            mac_filter,
            service_uuid,
            write_uuid,
            notify_uuid,
            max_write,
            scan_timeout,
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
    fn help_returns_none() {
        let args = ["--help"].into_iter().map(String::from);
        assert!(Config::from_args_and_env(args).unwrap().is_none());
    }

    #[test]
    fn unknown_flag_errors() {
        let args = ["--nope"].into_iter().map(String::from);
        assert!(Config::from_args_and_env(args).is_err());
    }
}
