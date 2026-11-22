//! `dangle` binary: connect to a Bluetooth LE serial dongle with the host's radio and republish
//! its GATT serial stream as a TCP server, so a tool that can reach a TCP endpoint but not host
//! Bluetooth (e.g. an Android emulator) can drive a real BLE dongle. A typical dongle is an OBD
//! adapter such as an `OBDLink` MX, but the bridge is protocol-agnostic: it moves bytes.
//!
//! This is a thin wrapper over the `dangle` library: the TCP accept loop, the BLE (re)connect
//! loop, and the `scan` doctor subcommand, all built on the library's host-OS BLE and byte-pipe
//! building blocks. It is a transport bridge only: it moves raw bytes between the TCP client and
//! the dongle's BLE serial characteristics, parsing none of them.

use std::process::ExitCode;

use dangle::BridgeResult;
use dangle::config;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dangle: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn run() -> BridgeResult<()> {
    use std::net::TcpListener;
    use std::time::Duration;

    use dangle::{ble, bridge, capture};

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

    if cfg.mode == config::Mode::Doctor {
        return ble::doctor(&cfg);
    }

    if cfg.mode == config::Mode::Pipe {
        return pipe(&cfg);
    }

    let listener = TcpListener::bind(cfg.listen)?;
    let bound = listener.local_addr()?;
    for line in config::listen_notice(bound).lines() {
        eprintln!("dangle: {line}");
    }

    // The tap outlives BLE reconnects and TCP clients, so one file holds the whole run. A tap that
    // cannot be created is fatal at startup (the user asked for it); a later write error only
    // disables it.
    let tap = match &cfg.capture {
        Some(c) => {
            let tap = capture::Tap::create(&c.path, c.format, cfg.max_write)
                .map_err(|e| format!("cannot create capture file {}: {e}", c.path.display()))?;
            eprintln!("dangle: capturing {:?} to {}", c.format, c.path.display());
            tap
        }
        None => capture::Tap::disabled(),
    };

    // Outer loop: (re)connect the BLE adapter.
    loop {
        let (ble, notify_rx) = match establish(&cfg) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("dangle: dongle not ready: {e}; retrying in 3s");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        eprintln!("dangle: dongle connected; waiting for a TCP client");

        // Inner loop: serve clients one at a time while the adapter stays connected.
        loop {
            let (stream, peer) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) => {
                    eprintln!("dangle: accept error: {e}");
                    continue;
                }
            };
            let _ = stream.set_nodelay(true);
            eprintln!("dangle: client {peer} connected");

            match bridge::run(stream, &ble, &notify_rx, &tap) {
                Ok(()) => eprintln!("dangle: client {peer} disconnected"),
                Err(e) => {
                    eprintln!("dangle: BLE link lost ({e}); reconnecting dongle");
                    break; // drop `ble`, reconnect in the outer loop
                }
            }
        }
    }
}

/// Connect to the dongle and relay its serial stream on this process's stdin/stdout, with no TCP
/// server (the `pipe` subcommand).
///
/// A connect-and-pipe demo that drives the library's scan/connect path directly: bytes read from
/// stdin go to the dongle, and bytes the dongle sends arrive on stdout. It returns when stdin
/// reaches EOF (Ctrl-Z then Enter on Windows) or the link drops on the next write.
#[cfg(windows)]
fn pipe(cfg: &config::Config) -> BridgeResult<()> {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;

    use dangle::link::SerialLink;

    let (ble, notify_rx) = establish(cfg)?;
    eprintln!("dangle: dongle connected; relaying stdin<->dongle (Ctrl-Z then Enter to end)");

    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        // stdin -> dongle
        s.spawn(|| {
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 1024];
            while !stop.load(Ordering::Relaxed) {
                match stdin.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) if ble.write(&buf[..n]).is_ok() => {}
                    _ => break, // read error or BLE write failed (link dropped)
                }
            }
            stop.store(true, Ordering::Relaxed);
        });

        // dongle -> stdout
        let mut stdout = std::io::stdout().lock();
        while !stop.load(Ordering::Relaxed) {
            match notify_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(bytes) => {
                    if stdout.write_all(&bytes).is_err() || stdout.flush().is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {} // re-check the stop flag
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        stop.store(true, Ordering::Relaxed);
    });

    Ok(())
}

/// Scan for the configured adapter and open a serial connection to it.
#[cfg(windows)]
fn establish(
    cfg: &config::Config,
) -> BridgeResult<(dangle::ble::BleSerial, std::sync::mpsc::Receiver<Vec<u8>>)> {
    use dangle::ble;

    let (addr, name) = ble::scan_for_adapter(cfg)?;
    let shown = if name.is_empty() { "<unnamed>" } else { &name };
    eprintln!(
        "dangle: found dongle {shown} at {}",
        config::format_addr(addr)
    );
    ble::connect(addr, &cfg.serial())
}

#[cfg(not(windows))]
fn run() -> BridgeResult<()> {
    // The only backend is WinRT Bluetooth LE. Still parse args so --help works anywhere.
    match config::Config::from_args_and_env(std::env::args().skip(1)) {
        Ok(None) => {
            println!("{}", config::USAGE);
            Ok(())
        }
        _ => Err("dangle requires Windows: the Bluetooth LE backend uses WinRT GATT".into()),
    }
}
