//! `obd-bridge`: connect to a Bluetooth LE ELM327/STN OBD adapter with the host's radio and
//! republish it as an ELM327-over-TCP server, so a tool that can reach a TCP endpoint but not host
//! Bluetooth (e.g. an Android emulator) can drive a real BLE adapter.
//!
//! This binary is a transport bridge only: it moves raw ASCII command/response bytes between the TCP
//! client and the adapter's BLE serial characteristics. It performs no OBD/UDS logic and issues no
//! vehicle writes of its own.

// The relay and link trait are platform-independent (and tested everywhere); only the Windows
// build has a backend that uses them outside tests.
#[cfg_attr(not(windows), allow(dead_code))]
mod bridge;
#[cfg_attr(not(windows), allow(dead_code))]
mod capture;
mod config;
#[cfg_attr(not(windows), allow(dead_code))]
mod link;

#[cfg(windows)]
mod ble;

use std::process::ExitCode;

/// Crate-wide fallible result with a boxed, thread-safe error.
pub type BridgeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("obd-bridge: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn run() -> BridgeResult<()> {
    use std::net::TcpListener;
    use std::time::Duration;

    let cfg = match config::Config::from_args_and_env(std::env::args().skip(1)) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            println!("{}", config::USAGE);
            return Ok(());
        }
        Err(e) => {
            eprintln!("{}\n", config::USAGE);
            return Err(Box::new(e));
        }
    };

    let listener = TcpListener::bind(cfg.listen)?;
    let bound = listener.local_addr()?;
    for line in config::listen_notice(bound).lines() {
        eprintln!("obd-bridge: {line}");
    }

    // The tap outlives BLE reconnects and TCP clients, so one file holds the whole run. A tap that
    // cannot be created is fatal at startup (the user asked for it); a later write error only
    // disables it.
    let tap = match &cfg.capture {
        Some(c) => {
            let tap = capture::Tap::create(&c.path, c.format, cfg.max_write)
                .map_err(|e| format!("cannot create capture file {}: {e}", c.path.display()))?;
            eprintln!(
                "obd-bridge: capturing {:?} to {}",
                c.format,
                c.path.display()
            );
            tap
        }
        None => capture::Tap::disabled(),
    };

    // Outer loop: (re)connect the BLE adapter.
    loop {
        let (ble, notify_rx) = match establish(&cfg) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("obd-bridge: adapter not ready: {e}; retrying in 3s");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        eprintln!("obd-bridge: adapter connected; waiting for a TCP client");

        // Inner loop: serve clients one at a time while the adapter stays connected.
        loop {
            let (stream, peer) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) => {
                    eprintln!("obd-bridge: accept error: {e}");
                    continue;
                }
            };
            let _ = stream.set_nodelay(true);
            eprintln!("obd-bridge: client {peer} connected");

            match bridge::run(stream, &ble, &notify_rx, &tap) {
                Ok(()) => eprintln!("obd-bridge: client {peer} disconnected"),
                Err(e) => {
                    eprintln!("obd-bridge: BLE link lost ({e}); reconnecting adapter");
                    break; // drop `ble`, reconnect in the outer loop
                }
            }
        }
    }
}

/// Scan for the configured adapter and open a serial connection to it.
#[cfg(windows)]
fn establish(
    cfg: &config::Config,
) -> BridgeResult<(ble::BleSerial, std::sync::mpsc::Receiver<Vec<u8>>)> {
    let (addr, name) = ble::scan_for_adapter(cfg)?;
    let shown = if name.is_empty() { "<unnamed>" } else { &name };
    eprintln!(
        "obd-bridge: found adapter {shown} at {}",
        ble::format_addr(addr)
    );
    ble::connect(cfg, addr)
}

#[cfg(not(windows))]
fn run() -> BridgeResult<()> {
    // The only backend is WinRT Bluetooth LE. Still parse args so --help works anywhere.
    match config::Config::from_args_and_env(std::env::args().skip(1)) {
        Ok(None) => {
            println!("{}", config::USAGE);
            Ok(())
        }
        _ => Err("obd-bridge requires Windows: the Bluetooth LE backend uses WinRT GATT".into()),
    }
}
