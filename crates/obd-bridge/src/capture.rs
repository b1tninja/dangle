//! Optional capture tap: record the relayed serial traffic in an existing interchange format.
//!
//! The bridge sees GATT *values* (the bytes of a write to, or a notification from, the adapter's
//! serial characteristics), not HCI traffic. To let a stock tool open the capture, each value is
//! wrapped in **synthesized** HCI/L2CAP/ATT framing and written as a pcap or btsnoop record. The
//! framing is invented by this module; only its layout follows the specification (`docs/capture.md`
//! lists every chosen value):
//!
//! - HCI ACL Data packet in the UART transport form: indicator octet `0x02`, then the ACL header
//!   (12-bit `Connection_Handle`, 2-bit `Packet_Boundary_Flag`, 2-bit `Broadcast_Flag`, 16-bit
//!   `Data_Total_Length`) - Core Specification 6.3, Vol 4, Part E, section 5.4.2.
//! - L2CAP basic header (16-bit PDU Length, 16-bit Channel ID), little-endian - Vol 3, Part A,
//!   section 3 and 3.1; the ATT fixed channel on LE-U is CID `0x0004` - Vol 3, Part A, section 2.1.
//! - ATT PDU: `ATT_WRITE_CMD` `0x52` (host to adapter) and `ATT_HANDLE_VALUE_NTF` `0x1B` (adapter to
//!   host), each opcode, 2-octet attribute handle, value - Vol 3, Part F, sections 3.4.5.3, 3.4.7.1
//!   and the opcode table in 3.4.8.
//!
//! The pcap link type numbers (201, `LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR`) and file layout are from
//! tcpdump.org (`linktypes.html`, `pcap-savefile(5)`); the 4-byte direction pseudo-header is from
//! libpcap's `pcap/bluetooth.h`. btsnoop is not a Bluetooth SIG or tcpdump format; its layout here
//! follows Wireshark's `wiretap/btsnoop.c`.
//!
//! The tap is a passive observer. Every write failure is reported once on stderr and then the tap
//! switches itself off; it can never return an error to, or otherwise break, the relay.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// pcap link type `LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR` (tcpdump.org link-layer header types).
pub const LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR: u32 = 201;
/// btsnoop datalink code for HCI UART (H4) framing (Wireshark `wiretap/btsnoop.c`).
pub const BTSNOOP_DATALINK_H4: u32 = 1002;

/// HCI UART Transport Layer packet indicator for an HCI ACL Data packet (Core 6.3, Vol 4, Part A).
const H4_ACL_DATA: u8 = 0x02;
/// L2CAP fixed channel ID of ATT on an LE-U logical link (Core 6.3, Vol 3, Part A, section 2.1).
const CID_ATT: u16 = 0x0004;
/// `ATT_WRITE_CMD` opcode (Core 6.3, Vol 3, Part F, section 3.4.8, Table 3.42).
const ATT_WRITE_CMD: u8 = 0x52;
/// `ATT_HANDLE_VALUE_NTF` opcode (Core 6.3, Vol 3, Part F, section 3.4.7.1).
const ATT_HANDLE_VALUE_NTF: u8 = 0x1B;

/// `Connection_Handle` written into every synthesized ACL header. Arbitrary: the bridge has no HCI
/// connection handle. It is within the 0x000..=0xEFF range of Vol 4, Part E, section 5.4.2.
pub const CONNECTION_HANDLE: u16 = 0x0040;
/// ATT attribute handle shown for the write characteristic's value. Arbitrary and fixed.
pub const WRITE_VALUE_HANDLE: u16 = 0x0011;
/// ATT attribute handle shown for the notify characteristic's value. Arbitrary and fixed.
pub const NOTIFY_VALUE_HANDLE: u16 = 0x0013;

/// Largest ATT value that fits one ACL packet: `Data_Total_Length` (16 bits) minus the 4-octet L2CAP
/// header and the 3-octet ATT header.
const MAX_VALUE: usize = 0xFFFF - 4 - 3;
/// Snapshot length written to the pcap global header (a bound; no record is truncated).
const PCAP_SNAPLEN: u32 = 262_144;
/// Offset btsnoop adds to Unix microseconds (`KUnixTimeBase` in Wireshark's btsnoop reader).
const BTSNOOP_EPOCH_OFFSET_US: i64 = 0x00dc_ddb3_0f2f_8000;

/// Which way a value travelled, from the bridge host's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Written to the adapter (TCP client to adapter).
    HostToAdapter,
    /// Notified by the adapter (adapter to TCP client).
    AdapterToHost,
}

/// File format a capture is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// pcap, link type 201 (H4 with direction pseudo-header). The default.
    Pcap,
    /// btsnoop, datalink 1002 (H4).
    Btsnoop,
    /// Plain timestamped hex and ASCII lines; no Bluetooth framing.
    Text,
}

impl Format {
    /// Parse a `--capture-format` value (`pcap`, `btsnoop` or `text`, case-insensitive).
    ///
    /// # Errors
    /// Returns a message naming the accepted values.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "pcap" => Ok(Self::Pcap),
            "btsnoop" => Ok(Self::Btsnoop),
            "text" => Ok(Self::Text),
            _ => Err(format!(
                "invalid capture format {s:?}: expected pcap, btsnoop or text"
            )),
        }
    }
}

/// The synthesized ATT PDU for one value: opcode, little-endian attribute handle, value.
fn att_pdu(dir: Direction, value: &[u8]) -> Vec<u8> {
    let (opcode, handle) = match dir {
        Direction::HostToAdapter => (ATT_WRITE_CMD, WRITE_VALUE_HANDLE),
        Direction::AdapterToHost => (ATT_HANDLE_VALUE_NTF, NOTIFY_VALUE_HANDLE),
    };
    let mut pdu = Vec::with_capacity(3 + value.len());
    pdu.push(opcode);
    pdu.extend_from_slice(&handle.to_le_bytes());
    pdu.extend_from_slice(value);
    pdu
}

/// The synthesized H4 packet (indicator, HCI ACL header, L2CAP basic header, ATT PDU) for one value
/// of at most [`MAX_VALUE`] octets.
///
/// `Packet_Boundary_Flag` is `0b00` host to controller and `0b10` controller to host: the table in
/// Vol 4, Part E, section 5.4.2 allows `0b00` toward the host on neither LE-U nor ACL-U (loopback
/// aside), so the "first automatically flushable" value is used for the adapter-to-host direction.
/// `Broadcast_Flag` is `0b00` (point-to-point).
fn h4_acl_packet(dir: Direction, value: &[u8]) -> Vec<u8> {
    debug_assert!(value.len() <= MAX_VALUE);
    let att = att_pdu(dir, value);
    let pb: u16 = match dir {
        Direction::HostToAdapter => 0b00,
        Direction::AdapterToHost => 0b10,
    };
    // Handle in bits 0..=11, PB in bits 12..=13, BC in bits 14..=15 of the first two octets.
    let handle_and_flags = CONNECTION_HANDLE | (pb << 12);
    let l2cap_len = u16::try_from(att.len()).unwrap_or(u16::MAX);
    let acl_len = l2cap_len.saturating_add(4);

    let mut pkt = Vec::with_capacity(1 + 4 + 4 + att.len());
    pkt.push(H4_ACL_DATA);
    pkt.extend_from_slice(&handle_and_flags.to_le_bytes());
    pkt.extend_from_slice(&acl_len.to_le_bytes());
    pkt.extend_from_slice(&l2cap_len.to_le_bytes());
    pkt.extend_from_slice(&CID_ATT.to_le_bytes());
    pkt.extend_from_slice(&att);
    pkt
}

/// Seconds and microseconds since the Unix epoch (zero for a clock before it).
fn unix_parts(ts: SystemTime) -> (u64, u32) {
    ts.duration_since(UNIX_EPOCH)
        .map_or((0, 0), |d| (d.as_secs(), d.subsec_micros()))
}

/// One `u32` field of a pcap header, little-endian (the writer's byte order; the magic number tells
/// readers which that is - `pcap-savefile(5)`).
fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The 24-octet pcap global header for `linktype`.
fn pcap_global_header(linktype: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(24);
    le32(&mut h, 0xa1b2_c3d4); // magic, microsecond timestamps
    h.extend_from_slice(&2u16.to_le_bytes()); // major version
    h.extend_from_slice(&4u16.to_le_bytes()); // minor version
    le32(&mut h, 0); // reserved1
    le32(&mut h, 0); // reserved2
    le32(&mut h, PCAP_SNAPLEN);
    le32(&mut h, linktype);
    h
}

/// One pcap record (16-octet record header + 4-octet direction pseudo-header + H4 packet).
fn pcap_record(ts: SystemTime, dir: Direction, h4: &[u8]) -> Vec<u8> {
    let (secs, micros) = unix_parts(ts);
    let len = u32::try_from(4 + h4.len()).unwrap_or(u32::MAX);
    let mut r = Vec::with_capacity(16 + 4 + h4.len());
    le32(&mut r, u32::try_from(secs).unwrap_or(u32::MAX));
    le32(&mut r, micros);
    le32(&mut r, len); // captured length
    le32(&mut r, len); // original length
    // The pseudo-header is in network byte order; bit 0 set means received by the host.
    let direction: u32 = match dir {
        Direction::HostToAdapter => 0,
        Direction::AdapterToHost => 1,
    };
    r.extend_from_slice(&direction.to_be_bytes());
    r.extend_from_slice(h4);
    r
}

/// The 16-octet btsnoop file header: magic, version 1, datalink (all big-endian).
fn btsnoop_header() -> Vec<u8> {
    let mut h = Vec::with_capacity(16);
    h.extend_from_slice(b"btsnoop\0");
    h.extend_from_slice(&1u32.to_be_bytes());
    h.extend_from_slice(&BTSNOOP_DATALINK_H4.to_be_bytes());
    h
}

/// One btsnoop record: original length, included length, flags, cumulative drops, timestamp, data.
/// Flags bit 0 is set for controller to host; bit 1 (command/event) is clear for ACL data.
fn btsnoop_record(ts: SystemTime, dir: Direction, h4: &[u8]) -> Vec<u8> {
    let (secs, micros) = unix_parts(ts);
    let us = i64::try_from(secs).unwrap_or(i64::MAX / 2_000_000) * 1_000_000
        + i64::from(micros)
        + BTSNOOP_EPOCH_OFFSET_US;
    let len = u32::try_from(h4.len()).unwrap_or(u32::MAX);
    let flags: u32 = match dir {
        Direction::HostToAdapter => 0,
        Direction::AdapterToHost => 1,
    };
    let mut r = Vec::with_capacity(24 + h4.len());
    r.extend_from_slice(&len.to_be_bytes());
    r.extend_from_slice(&len.to_be_bytes());
    r.extend_from_slice(&flags.to_be_bytes());
    r.extend_from_slice(&0u32.to_be_bytes());
    r.extend_from_slice(&us.to_be_bytes());
    r.extend_from_slice(h4);
    r
}

/// One text-log line: `secs.micros`, an arrow (`>` to the adapter, `<` from it), hex, ASCII.
fn text_line(ts: SystemTime, dir: Direction, value: &[u8]) -> String {
    let (secs, micros) = unix_parts(ts);
    let arrow = match dir {
        Direction::HostToAdapter => '>',
        Direction::AdapterToHost => '<',
    };
    let mut line = format!("{secs}.{micros:06} {arrow}");
    for b in value {
        let _ = write!(line, " {b:02x}");
    }
    line.push_str("  |");
    line.extend(value.iter().map(|&b| {
        if b.is_ascii_graphic() || b == b' ' {
            char::from(b)
        } else {
            '.'
        }
    }));
    line.push_str("|\n");
    line
}

/// Live state of an enabled tap.
struct Sink {
    out: Box<dyn Write + Send>,
    format: Format,
    /// Largest value recorded as one host-to-adapter write (the bridge's `--mtu`).
    write_chunk: usize,
}

impl Sink {
    fn record(&mut self, ts: SystemTime, dir: Direction, data: &[u8]) -> io::Result<()> {
        let step = match dir {
            Direction::HostToAdapter => self.write_chunk.clamp(1, MAX_VALUE),
            Direction::AdapterToHost => MAX_VALUE,
        };
        for value in data.chunks(step) {
            let bytes = match self.format {
                Format::Pcap => pcap_record(ts, dir, &h4_acl_packet(dir, value)),
                Format::Btsnoop => btsnoop_record(ts, dir, &h4_acl_packet(dir, value)),
                Format::Text => text_line(ts, dir, value).into_bytes(),
            };
            self.out.write_all(&bytes)?;
        }
        // Flush per call so a live reader (or a crash) sees whole records.
        self.out.flush()
    }
}

/// A capture tap, shared by both relay directions. [`Tap::disabled`] is a no-op.
pub struct Tap {
    sink: Mutex<Option<Sink>>,
}

impl Tap {
    /// A tap that records nothing.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            sink: Mutex::new(None),
        }
    }

    /// Create `path` (truncating it) and write the file header for `format`.
    ///
    /// `write_chunk` is the bridge's per-write payload limit: a larger host-to-adapter relay is
    /// recorded as several ATT writes of at most that size, matching what the adapter receives.
    ///
    /// # Errors
    /// Returns the I/O error if the file cannot be created or the header written.
    pub fn create(path: &Path, format: Format, write_chunk: usize) -> io::Result<Self> {
        let file = File::create(path)?;
        Self::from_writer(Box::new(BufWriter::new(file)), format, write_chunk)
    }

    /// Like [`Tap::create`] over any writer.
    ///
    /// # Errors
    /// Returns the I/O error if the header cannot be written.
    pub fn from_writer(
        mut out: Box<dyn Write + Send>,
        format: Format,
        write_chunk: usize,
    ) -> io::Result<Self> {
        match format {
            Format::Pcap => {
                out.write_all(&pcap_global_header(LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR))?;
            }
            Format::Btsnoop => {
                out.write_all(&btsnoop_header())?;
            }
            Format::Text => {}
        }
        out.flush()?;
        Ok(Self {
            sink: Mutex::new(Some(Sink {
                out,
                format,
                write_chunk,
            })),
        })
    }

    /// Whether the tap is still recording.
    #[cfg(test)]
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.sink.lock().is_ok_and(|s| s.is_some())
    }

    /// Record `data`, observed at `ts`, travelling `dir`. Never fails: on an I/O error the tap logs
    /// once to stderr and turns itself off, leaving the relay untouched.
    pub fn record(&self, ts: SystemTime, dir: Direction, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let Ok(mut guard) = self.sink.lock() else {
            return;
        };
        let Some(sink) = guard.as_mut() else {
            return;
        };
        if let Err(e) = sink.record(ts, dir, data) {
            eprintln!("obd-bridge: capture write failed ({e}); capture disabled");
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// A writer whose bytes the test can read back.
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

    impl Shared {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Accepts `ok` bytes, then fails every write.
    struct FailAfter {
        ok: usize,
    }

    impl Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.ok < buf.len() {
                return Err(io::Error::other("disk full"));
            }
            self.ok -= buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn at(secs: u64, micros: u32) -> SystemTime {
        UNIX_EPOCH + Duration::new(secs, micros * 1000)
    }

    fn make(format: Format, chunk: usize) -> (Tap, Shared) {
        let buf = Shared::default();
        let tap = Tap::from_writer(Box::new(buf.clone()), format, chunk).unwrap();
        (tap, buf)
    }

    /// A decoded pcap record.
    #[derive(Debug, PartialEq)]
    struct Rec {
        secs: u32,
        micros: u32,
        received: bool,
        h4: Vec<u8>,
    }

    /// Small pcap reader: checks the global header and returns the records.
    fn read_pcap(file: &[u8]) -> Vec<Rec> {
        assert_eq!(&file[..4], &[0xd4, 0xc3, 0xb2, 0xa1], "magic as written LE");
        let u32le = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
        assert_eq!(&file[4..8], &[2, 0, 4, 0], "version 2.4");
        assert_eq!(u32le(&file[8..12]), 0);
        assert_eq!(u32le(&file[12..16]), 0);
        assert_eq!(u32le(&file[16..20]), PCAP_SNAPLEN);
        assert_eq!(u32le(&file[20..24]), 201);
        let mut recs = Vec::new();
        let mut i = 24;
        while i < file.len() {
            let caplen = u32le(&file[i + 8..i + 12]) as usize;
            assert_eq!(caplen, u32le(&file[i + 12..i + 16]) as usize);
            let body = &file[i + 16..i + 16 + caplen];
            recs.push(Rec {
                secs: u32le(&file[i..i + 4]),
                micros: u32le(&file[i + 4..i + 8]),
                received: u32::from_be_bytes(body[..4].try_into().unwrap()) & 1 == 1,
                h4: body[4..].to_vec(),
            });
            i += 16 + caplen;
        }
        assert_eq!(i, file.len(), "no trailing bytes");
        recs
    }

    /// A decoded ACL/L2CAP/ATT packet.
    #[derive(Debug, PartialEq)]
    struct Att {
        conn: u16,
        pb: u8,
        bc: u8,
        cid: u16,
        opcode: u8,
        handle: u16,
        value: Vec<u8>,
    }

    fn parse_h4(h4: &[u8]) -> Att {
        assert_eq!(h4[0], 0x02, "ACL indicator");
        let hf = u16::from_le_bytes([h4[1], h4[2]]);
        let acl_len = usize::from(u16::from_le_bytes([h4[3], h4[4]]));
        assert_eq!(h4.len(), 5 + acl_len, "`Data_Total_Length`");
        let l2_len = usize::from(u16::from_le_bytes([h4[5], h4[6]]));
        assert_eq!(acl_len, 4 + l2_len, "L2CAP PDU Length");
        Att {
            conn: hf & 0x0fff,
            pb: u8::try_from((hf >> 12) & 3).unwrap(),
            bc: u8::try_from(hf >> 14).unwrap(),
            cid: u16::from_le_bytes([h4[7], h4[8]]),
            opcode: h4[9],
            handle: u16::from_le_bytes([h4[10], h4[11]]),
            value: h4[12..].to_vec(),
        }
    }

    #[test]
    fn pcap_global_header_exact_bytes() {
        let (_tap, buf) = make(Format::Pcap, 20);
        assert_eq!(
            buf.bytes(),
            [
                0xd4, 0xc3, 0xb2, 0xa1, // magic
                0x02, 0x00, 0x04, 0x00, // version 2.4
                0, 0, 0, 0, 0, 0, 0, 0, // reserved
                0x00, 0x00, 0x04, 0x00, // snaplen 262144
                0xc9, 0x00, 0x00, 0x00, // link type 201
            ]
        );
    }

    #[test]
    fn pcap_host_write_record_exact_bytes() {
        let (tap, buf) = make(Format::Pcap, 20);
        tap.record(
            at(1_700_000_000, 123_456),
            Direction::HostToAdapter,
            b"ATZ\r",
        );
        let rec = &buf.bytes()[24..];
        let mut want = Vec::new();
        want.extend_from_slice(&1_700_000_000u32.to_le_bytes());
        want.extend_from_slice(&123_456u32.to_le_bytes());
        want.extend_from_slice(&[20, 0, 0, 0, 20, 0, 0, 0]); // caplen, len
        want.extend_from_slice(&[0, 0, 0, 0]); // direction: sent
        want.extend_from_slice(&[
            0x02, // H4 ACL data
            0x40, 0x00, // handle 0x040, PB 0b00, BC 0b00
            0x0b, 0x00, // `Data_Total_Length` 11
            0x07, 0x00, 0x04, 0x00, // L2CAP length 7, CID 0x0004
            0x52, 0x11, 0x00, // ATT_WRITE_CMD, handle 0x0011
            b'A', b'T', b'Z', b'\r',
        ]);
        assert_eq!(rec, want);
    }

    #[test]
    fn pcap_notification_record_exact_bytes() {
        let (tap, buf) = make(Format::Pcap, 20);
        tap.record(at(5, 7), Direction::AdapterToHost, b"OK\r>");
        let rec = &buf.bytes()[24..];
        let mut want = Vec::new();
        want.extend_from_slice(&5u32.to_le_bytes());
        want.extend_from_slice(&7u32.to_le_bytes());
        want.extend_from_slice(&[20, 0, 0, 0, 20, 0, 0, 0]);
        want.extend_from_slice(&[0, 0, 0, 1]); // direction: received
        want.extend_from_slice(&[
            0x02, //
            0x40, 0x20, // handle 0x040, PB 0b10
            0x0b, 0x00, //
            0x07, 0x00, 0x04, 0x00, //
            0x1b, 0x13, 0x00, // ATT_HANDLE_VALUE_NTF, handle 0x0013
            b'O', b'K', b'\r', b'>',
        ]);
        assert_eq!(rec, want);
    }

    #[test]
    fn pcap_round_trips_through_reader() {
        let (tap, buf) = make(Format::Pcap, 20);
        tap.record(at(10, 1), Direction::HostToAdapter, b"ATI\r");
        tap.record(at(10, 2), Direction::AdapterToHost, b"ELM327 v1.5\r\r>");
        let recs = read_pcap(&buf.bytes());
        assert_eq!(recs.len(), 2);
        assert_eq!(
            (recs[0].secs, recs[0].micros, recs[0].received),
            (10, 1, false)
        );
        assert_eq!(
            (recs[1].secs, recs[1].micros, recs[1].received),
            (10, 2, true)
        );
        let a = parse_h4(&recs[0].h4);
        assert_eq!(
            a,
            Att {
                conn: CONNECTION_HANDLE,
                pb: 0,
                bc: 0,
                cid: 4,
                opcode: 0x52,
                handle: WRITE_VALUE_HANDLE,
                value: b"ATI\r".to_vec()
            }
        );
        let b = parse_h4(&recs[1].h4);
        assert_eq!((b.pb, b.opcode, b.handle), (2, 0x1b, NOTIFY_VALUE_HANDLE));
        assert_eq!(b.value, b"ELM327 v1.5\r\r>");
    }

    #[test]
    fn host_writes_are_split_at_the_write_chunk() {
        let (tap, buf) = make(Format::Pcap, 20);
        let data: Vec<u8> = (0..45).collect();
        tap.record(at(1, 0), Direction::HostToAdapter, &data);
        let recs = read_pcap(&buf.bytes());
        let lens: Vec<usize> = recs.iter().map(|r| parse_h4(&r.h4).value.len()).collect();
        assert_eq!(lens, [20, 20, 5]);
        let joined: Vec<u8> = recs.iter().flat_map(|r| parse_h4(&r.h4).value).collect();
        assert_eq!(joined, data);
    }

    #[test]
    fn notifications_are_recorded_whole() {
        let (tap, buf) = make(Format::Pcap, 20);
        tap.record(at(1, 0), Direction::AdapterToHost, &[b'x'; 100]);
        let recs = read_pcap(&buf.bytes());
        assert_eq!(recs.len(), 1);
        assert_eq!(parse_h4(&recs[0].h4).value.len(), 100);
    }

    #[test]
    fn btsnoop_header_and_record_exact_bytes() {
        let (tap, buf) = make(Format::Btsnoop, 20);
        let file = buf.bytes();
        assert_eq!(
            file,
            [
                b'b', b't', b's', b'n', b'o', b'o', b'p', 0, // magic
                0, 0, 0, 1, // version 1
                0, 0, 0x03, 0xea, // datalink 1002
            ]
        );
        tap.record(at(2, 3), Direction::AdapterToHost, b"OK\r>");
        let rec = &buf.bytes()[16..];
        let us = 2_000_003i64 + 0x00dc_ddb3_0f2f_8000;
        let mut want = Vec::new();
        want.extend_from_slice(&16u32.to_be_bytes()); // original length (H4 packet, no pseudo-header)
        want.extend_from_slice(&16u32.to_be_bytes()); // included length
        want.extend_from_slice(&1u32.to_be_bytes()); // flags: controller to host, data
        want.extend_from_slice(&0u32.to_be_bytes()); // drops
        want.extend_from_slice(&us.to_be_bytes());
        want.extend_from_slice(&[
            0x02, 0x40, 0x20, 0x0b, 0x00, 0x07, 0x00, 0x04, 0x00, 0x1b, 0x13, 0x00, b'O', b'K',
            b'\r', b'>',
        ]);
        assert_eq!(rec, want);

        // Host to controller clears bit 0.
        let (tap, buf) = make(Format::Btsnoop, 20);
        tap.record(at(0, 0), Direction::HostToAdapter, b"A");
        assert_eq!(&buf.bytes()[16 + 8..16 + 12], &[0, 0, 0, 0]);
    }

    #[test]
    fn text_log_lines() {
        let (tap, buf) = make(Format::Text, 4);
        tap.record(at(9, 42), Direction::HostToAdapter, b"ATZ\r0");
        tap.record(at(9, 43), Direction::AdapterToHost, b"OK\r>");
        assert_eq!(
            String::from_utf8(buf.bytes()).unwrap(),
            "9.000042 > 41 54 5a 0d  |ATZ.|\n\
             9.000042 > 30  |0|\n\
             9.000043 < 4f 4b 0d 3e  |OK.>|\n"
        );
    }

    #[test]
    fn empty_data_records_nothing() {
        let (tap, buf) = make(Format::Pcap, 20);
        tap.record(at(1, 1), Direction::HostToAdapter, b"");
        assert_eq!(buf.bytes().len(), 24);
    }

    #[test]
    fn write_failure_disables_the_tap_without_panicking() {
        // Room for the 24-byte pcap header and nothing else.
        let tap = Tap::from_writer(Box::new(FailAfter { ok: 24 }), Format::Pcap, 20).unwrap();
        assert!(tap.is_active());
        tap.record(at(1, 1), Direction::HostToAdapter, b"ATZ\r");
        assert!(!tap.is_active());
        tap.record(at(1, 2), Direction::HostToAdapter, b"ATZ\r"); // no-op, no second log
    }

    #[test]
    fn header_failure_is_reported_by_create() {
        assert!(Tap::from_writer(Box::new(FailAfter { ok: 3 }), Format::Pcap, 20).is_err());
    }

    #[test]
    fn disabled_tap_is_inert() {
        let tap = Tap::disabled();
        assert!(!tap.is_active());
        tap.record(at(1, 1), Direction::AdapterToHost, b"x");
    }

    #[test]
    fn format_parse() {
        assert_eq!(Format::parse("PCAP"), Ok(Format::Pcap));
        assert_eq!(Format::parse("btsnoop"), Ok(Format::Btsnoop));
        assert_eq!(Format::parse("text"), Ok(Format::Text));
        assert!(Format::parse("pcapng").is_err());
    }
}
