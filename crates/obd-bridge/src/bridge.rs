//! The byte pipe: relay raw bytes between one TCP client and a connected [`SerialLink`].
//!
//! Two directions run concurrently inside a [`std::thread::scope`] so they can borrow the same
//! adapter:
//! - TCP -> BLE: read from the socket and [`SerialLink::write`] the bytes (chunked to the MTU).
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
use std::time::{Duration, SystemTime};

use crate::BridgeResult;
use crate::capture::{Direction, Tap};
use crate::link::SerialLink;

/// Poll interval for the socket read timeout and notification receive, so both directions notice a
/// shutdown request promptly without busy-waiting.
const POLL: Duration = Duration::from_millis(200);

/// Relay bytes between `stream` and `ble` until the client disconnects or the BLE link fails.
///
/// Returns `Ok(())` when the TCP client closed its side cleanly (the adapter stays connected for the
/// next client). Returns `Err` when a BLE write failed, signalling the caller to drop the adapter
/// and reconnect.
///
/// `tap` observes the relayed bytes after they have been delivered; it never alters or fails the
/// relay (see [`crate::capture`]).
///
/// # Errors
/// Returns an error if the socket cannot be configured or the BLE link failed during the session.
pub fn run<L: SerialLink>(
    stream: TcpStream,
    ble: &L,
    notify: &Receiver<Vec<u8>>,
    tap: &Tap,
) -> BridgeResult<()> {
    let read_stream = stream.try_clone()?;
    read_stream.set_read_timeout(Some(POLL))?;

    let stop = AtomicBool::new(false);
    let ble_err: Mutex<Option<String>> = Mutex::new(None);

    thread::scope(|s| {
        s.spawn(|| tcp_to_ble(read_stream, ble, tap, &stop, &ble_err));
        ble_to_tcp(stream, notify, tap, &stop);
    });

    if let Some(message) = ble_err.into_inner().unwrap_or(None) {
        return Err(message.into());
    }
    Ok(())
}

/// Read from the socket and forward to the adapter. Sets `stop` and records any BLE error on exit.
fn tcp_to_ble<L: SerialLink>(
    mut stream: TcpStream,
    ble: &L,
    tap: &Tap,
    stop: &AtomicBool,
    ble_err: &Mutex<Option<String>>,
) {
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buf) {
            Ok(0) => break, // client closed the connection
            Ok(n) => {
                let seen = SystemTime::now();
                if let Err(e) = ble.write(&buf[..n]) {
                    if let Ok(mut slot) = ble_err.lock() {
                        *slot = Some(e.to_string());
                    }
                    break;
                }
                tap.record(seen, Direction::HostToAdapter, &buf[..n]);
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
fn ble_to_tcp(mut stream: TcpStream, notify: &Receiver<Vec<u8>>, tap: &Tap, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match notify.recv_timeout(POLL) {
            Ok(bytes) => {
                let seen = SystemTime::now();
                if stream.write_all(&bytes).is_err() {
                    break; // client gone
                }
                tap.record(seen, Direction::AdapterToHost, &bytes);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::write_chunked;
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Sender};

    /// In-memory link: records each chunk it is asked to write, optionally failing from the Nth write.
    struct MockLink {
        max_write: usize,
        writes: Mutex<Vec<Vec<u8>>>,
        fail_after: Option<usize>,
    }

    impl MockLink {
        fn new(max_write: usize) -> Self {
            Self {
                max_write,
                writes: Mutex::new(Vec::new()),
                fail_after: None,
            }
        }
        fn failing() -> Self {
            Self {
                fail_after: Some(0),
                ..Self::new(20)
            }
        }
        fn written(&self) -> Vec<Vec<u8>> {
            self.writes.lock().unwrap().clone()
        }
        fn wait_for_bytes(&self, n: usize) {
            for _ in 0..500 {
                if self.written().concat().len() >= n {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("link never received {n} bytes");
        }
    }

    impl SerialLink for MockLink {
        fn write(&self, data: &[u8]) -> BridgeResult<()> {
            write_chunked(self.max_write, data, |chunk| {
                let mut w = self.writes.lock().unwrap();
                if self.fail_after.is_some_and(|n| w.len() >= n) {
                    return Err("mock link down".into());
                }
                w.push(chunk.to_vec());
                Ok(())
            })
        }
    }

    /// A loopback pair: the bridge's end and the "client" end.
    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        (server, client)
    }

    fn read_until_prompt(client: &mut TcpStream) -> Vec<u8> {
        let mut out = Vec::new();
        let mut byte = [0u8; 1];
        while client.read_exact(&mut byte).is_ok() {
            out.push(byte[0]);
            if byte[0] == b'>' {
                break;
            }
        }
        out
    }

    /// Run the relay on the test thread against a fresh loopback connection while `client_side`
    /// plays the TCP client on another thread (`Receiver` is not `Sync`, so it stays here). The
    /// client stream is dropped when `client_side` returns, which ends the session.
    fn session(
        link: &MockLink,
        rx: &Receiver<Vec<u8>>,
        client_side: impl FnOnce(&mut TcpStream) + Send,
    ) -> BridgeResult<()> {
        let (server, mut client) = pair();
        thread::scope(|s| {
            s.spawn(move || client_side(&mut client));
            run(server, link, rx, &Tap::disabled())
        })
    }

    #[test]
    fn client_bytes_reach_link_verbatim_and_chunked() {
        let link = MockLink::new(4);
        let (_tx, rx): (Sender<Vec<u8>>, _) = mpsc::channel();
        let payload = b"ATZ\r010C\r0123456789";
        session(&link, &rx, |c| {
            c.write_all(payload).unwrap();
            link.wait_for_bytes(payload.len());
        })
        .unwrap();
        let writes = link.written();
        assert!(writes.iter().all(|c| !c.is_empty() && c.len() <= 4));
        assert_eq!(writes.concat(), payload);
    }

    #[test]
    fn notifications_reach_client_in_order() {
        let link = MockLink::new(20);
        let (tx, rx) = mpsc::channel();
        for chunk in [&b"one"[..], b"two", b"three\r>"] {
            tx.send(chunk.to_vec()).unwrap();
        }
        session(&link, &rx, |c| {
            assert_eq!(read_until_prompt(c), b"onetwothree\r>");
        })
        .unwrap();
    }

    #[test]
    fn split_response_is_relayed_byte_exact_to_prompt() {
        let link = MockLink::new(20);
        let (tx, rx) = mpsc::channel();
        let response = b"SEARCHING...\r41 0C 1A F8\r\r>";
        // Split mid-token, including between the CR and the prompt.
        for part in [
            &response[..5],
            &response[5..14],
            &response[14..26],
            &response[26..],
        ] {
            tx.send(part.to_vec()).unwrap();
        }
        session(&link, &rx, |c| {
            assert_eq!(read_until_prompt(c), response);
        })
        .unwrap();
    }

    #[test]
    fn client_disconnect_is_ok_and_link_serves_next_client() {
        let link = MockLink::new(20);
        let (tx, rx) = mpsc::channel();
        session(&link, &rx, |c| {
            c.write_all(b"ATI\r").unwrap();
            link.wait_for_bytes(4);
        })
        .expect("clean disconnect is Ok");

        // Same link and receiver, new client.
        tx.send(b"OK\r>".to_vec()).unwrap();
        session(&link, &rx, |c| {
            c.write_all(b"ATE0\r").unwrap();
            link.wait_for_bytes(9);
            assert_eq!(read_until_prompt(c), b"OK\r>");
        })
        .expect("second client");
        assert_eq!(link.written().concat(), b"ATI\rATE0\r");
    }

    #[test]
    fn failing_link_write_makes_run_err() {
        let link = MockLink::failing();
        let (_tx, rx): (Sender<Vec<u8>>, _) = mpsc::channel();
        let err = session(&link, &rx, |c| {
            c.write_all(b"ATZ\r").unwrap();
            // Keep the connection open so only the link failure can end the session.
            thread::sleep(Duration::from_secs(2));
        })
        .unwrap_err();
        assert!(err.to_string().contains("mock link down"));
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::capture::Format;
    use crate::link::SerialLink;
    use std::io;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::mpsc;

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Sink(Mutex<Vec<u8>>);

    impl SerialLink for Sink {
        fn write(&self, data: &[u8]) -> BridgeResult<()> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(())
        }
    }

    /// Writer that accepts the text-format "header" (none) and then always fails.
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("broken"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (server, _) = l.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        (server, client)
    }

    /// Run one session: the client sends `ATZ\r`, the adapter answers `OK\r>`.
    fn exchange(tap: &Tap) -> Vec<u8> {
        let link = Sink(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        tx.send(b"OK\r>".to_vec()).unwrap();
        let (server, mut client) = pair();
        let mut got = Vec::new();
        thread::scope(|s| {
            let got = &mut got;
            s.spawn(move || {
                client.write_all(b"ATZ\r").unwrap();
                let mut b = [0u8; 1];
                while client.read_exact(&mut b).is_ok() {
                    got.push(b[0]);
                    if b[0] == b'>' {
                        break;
                    }
                }
            });
            run(server, &link, &rx, tap).unwrap();
        });
        assert_eq!(*link.0.lock().unwrap(), b"ATZ\r");
        got
    }

    #[test]
    fn tap_records_both_directions_and_relay_is_unchanged() {
        let buf = Shared::default();
        let tap = Tap::from_writer(Box::new(buf.clone()), Format::Text, 20).unwrap();
        assert_eq!(exchange(&tap), b"OK\r>");
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains("> 41 54 5a 0d  |ATZ.|"), "{text}");
        assert!(text.contains("< 4f 4b 0d 3e  |OK.>|"), "{text}");
    }

    #[test]
    fn broken_tap_does_not_break_the_relay() {
        let tap = Tap::from_writer(Box::new(Broken), Format::Text, 20).unwrap();
        assert_eq!(exchange(&tap), b"OK\r>");
        assert!(!tap.is_active());
    }
}
