//! The byte pipe: relay raw bytes between one TCP client and a connected [`BleSerial`].
//!
//! Two directions run concurrently inside a [`std::thread::scope`] so they can borrow the same
//! adapter:
//! - TCP -> BLE: read from the socket and [`BleSerial::write`] the bytes (chunked to the MTU).
//! - BLE -> TCP: drain the notification [`Receiver`] and write each chunk to the socket.
//!
//! The relay is transport-only: ELM327/OBD bytes pass through untouched, with no parsing, framing on
//! the `>` prompt, or command synthesis.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::BridgeResult;
use crate::ble::BleSerial;

/// Poll interval for the socket read timeout and notification receive, so both directions notice a
/// shutdown request promptly without busy-waiting.
const POLL: Duration = Duration::from_millis(200);

/// Relay bytes between `stream` and `ble` until the client disconnects or the BLE link fails.
///
/// Returns `Ok(())` when the TCP client closed its side cleanly (the adapter stays connected for the
/// next client). Returns `Err` when a BLE write failed, signalling the caller to drop the adapter
/// and reconnect.
///
/// # Errors
/// Returns an error if the socket cannot be configured or the BLE link failed during the session.
pub fn run(stream: TcpStream, ble: &BleSerial, notify: &Receiver<Vec<u8>>) -> BridgeResult<()> {
    let read_stream = stream.try_clone()?;
    read_stream.set_read_timeout(Some(POLL))?;

    let stop = AtomicBool::new(false);
    let ble_err: Mutex<Option<String>> = Mutex::new(None);

    thread::scope(|s| {
        s.spawn(|| tcp_to_ble(read_stream, ble, &stop, &ble_err));
        ble_to_tcp(stream, notify, &stop);
    });

    if let Some(message) = ble_err.into_inner().unwrap_or(None) {
        return Err(message.into());
    }
    Ok(())
}

/// Read from the socket and forward to the adapter. Sets `stop` and records any BLE error on exit.
fn tcp_to_ble(
    mut stream: TcpStream,
    ble: &BleSerial,
    stop: &AtomicBool,
    ble_err: &Mutex<Option<String>>,
) {
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buf) {
            Ok(0) => break, // client closed the connection
            Ok(n) => {
                if let Err(e) = ble.write(&buf[..n]) {
                    if let Ok(mut slot) = ble_err.lock() {
                        *slot = Some(e.to_string());
                    }
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // read timeout elapsed: loop to re-check the stop flag
            }
            Err(_) => break, // socket error: treat as client gone
        }
    }
    stop.store(true, Ordering::Relaxed);
}

/// Drain adapter notifications and forward them to the socket. Exits when `stop` is set.
fn ble_to_tcp(mut stream: TcpStream, notify: &Receiver<Vec<u8>>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match notify.recv_timeout(POLL) {
            Ok(bytes) => {
                if stream.write_all(&bytes).is_err() {
                    break; // client gone
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
}
