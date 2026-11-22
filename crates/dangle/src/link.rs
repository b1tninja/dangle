//! The transport abstraction the byte pipe is generic over.
//!
//! [`SerialLink`] is the write half of a serial-over-GATT connection. The read half is a plain
//! `Receiver<Vec<u8>>` of notification chunks, kept separate so a link can be shared (`&Self`)
//! between the TCP-reader thread and the thread draining notifications. The real implementation
//! is `ble::BleSerial` (Windows-only); tests use an in-memory mock so [`crate::bridge`] is
//! exercised on any platform without a radio.

use crate::BridgeResult;

/// The write half of a serial link to a BLE serial dongle.
pub trait SerialLink: Sync {
    /// Write `data` to the adapter. Implementations split it into link-sized writes (see
    /// [`write_chunked`]).
    ///
    /// # Errors
    /// Returns an error if the link failed (typically the adapter disconnected), which the caller
    /// treats as a signal to reconnect.
    fn write(&self, data: &[u8]) -> BridgeResult<()>;
}

/// Split `data` into pieces of at most `max_write` bytes and pass each, in order, to `send`,
/// stopping at the first error.
///
/// Shared by the BLE backend and the test mock so the chunking rule is tested without hardware.
///
/// # Errors
/// Returns the first error `send` returns; later chunks are not sent.
///
/// # Panics
/// Panics if `max_write` is zero (config rejects `--mtu 0`).
pub fn write_chunked(
    max_write: usize,
    data: &[u8],
    mut send: impl FnMut(&[u8]) -> BridgeResult<()>,
) -> BridgeResult<()> {
    for chunk in data.chunks(max_write) {
        send(chunk)?;
    }
    Ok(())
}
