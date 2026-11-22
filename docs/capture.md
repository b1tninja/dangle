# Capture tap

`--capture <path>` records the relayed serial traffic in a file that an existing tool opens, instead
of a bespoke log. It is off by default.

```
dangle --capture run.pcap                             # pcap, link type 201 (default)
dangle --capture run.btsnoop --capture-format btsnoop
dangle --capture run.log --capture-format text
```

Environment fallbacks: `DANGLE_CAPTURE`, `DANGLE_CAPTURE_FORMAT`. A flag wins. The file is
created (truncated) at startup and stays open across TCP clients and BLE reconnects.

## The framing is synthesized

The bridge never sees HCI. It sees **GATT values**: the bytes a client wrote, and the bytes of each
notification. Each value is wrapped in made-up HCI ACL / L2CAP / ATT framing so that Wireshark and
similar tools decode it as an ATT write or notification. Nothing in the file is real controller
traffic: there is no connection setup, no MTU exchange, no discovery, no acknowledgement, no
encryption, and no Write Response even though the real backend may use a write with response.

Chosen values (constants in `capture.rs`):

| Item | Value | Note |
| --- | --- | --- |
| HCI `Connection_Handle` | `0x0040` | arbitrary, within the `0x000`..`0xEFF` range |
| ATT handle, write value | `0x0011` | arbitrary |
| ATT handle, notify value | `0x0013` | arbitrary |
| Host to adapter | `ATT_WRITE_CMD` (`0x52`), `Packet_Boundary_Flag` `0b00` | direction "sent" |
| Adapter to host | `ATT_HANDLE_VALUE_NTF` (`0x1B`), `Packet_Boundary_Flag` `0b10` | direction "received" |
| `Broadcast_Flag` | `0b00` | point-to-point |

A host-to-adapter relay larger than `--mtu` is recorded as several writes of at most `--mtu` bytes,
which is what the adapter receives. A notification is recorded whole. Records carry the time the
bridge observed the value, taken before it was relayed.

## Layout

Packet bytes, in order (all little-endian), as built by `h4_acl_packet`:

| Bytes | Field |
| --- | --- |
| 1 | H4 packet indicator `0x02` (HCI ACL Data) |
| 2 | `Connection_Handle` bits 0..11, `Packet_Boundary_Flag` bits 12..13, `Broadcast_Flag` bits 14..15 |
| 2 | `Data_Total_Length` (L2CAP header + ATT PDU) |
| 2 | L2CAP PDU Length (ATT PDU length) |
| 2 | L2CAP Channel ID `0x0004` |
| 1 | ATT opcode |
| 2 | ATT attribute handle |
| n | value |

Specification anchors (Bluetooth Core Specification 6.3, read through the primarysource `bluetooth_*`
tools; clause numbers only):

- HCI ACL Data packet, handle/flag/length fields and flag values: Vol 4, Part E, section 5.4.2
  (Figure 5.2). Its flag table is why controller-to-host uses `0b10` rather than `0b00`.
- Packet indicator for an ACL data packet over UART: Vol 4, Part A (HCI UART transport layer).
  The packet indicator table (HCI ACL Data packet = `0x02`) was read in the spec text; the exact
  section and table number were not recorded.
- L2CAP basic header and little-endian rule: Vol 3, Part A, sections 3 and 3.1. LE-U fixed channel
  CID `0x0004` for ATT: Vol 3, Part A, section 2.1.
- `ATT_WRITE_CMD`: Vol 3, Part F, section 3.4.5.3. `ATT_HANDLE_VALUE_NTF`: section 3.4.7.1. Opcode
  values: section 3.4.8, Table 3.42.

### pcap

Link type 201, `LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR` (tcpdump.org, `linktypes.html`). File and record
headers follow `pcap-savefile(5)` on tcpdump.org: magic `0xa1b2c3d4` written little-endian, version
2.4, snaplen 262144, microsecond timestamps. Each record is a 4-byte direction pseudo-header in
network byte order (bit 0 set means received by the host; libpcap `pcap/bluetooth.h`) followed by
the H4 packet. Link type 187 (`LINKTYPE_BLUETOOTH_HCI_H4`, no direction) is not written.

### btsnoop

Magic `btsnoop\0`, version 1, datalink 1002 (H4), big-endian, with 24-byte record headers
(original length, included length, flags, drops, timestamp). Flags bit 0 is set for controller to
host. The timestamp is Unix microseconds plus `0x00dcddb30f2f8000`. btsnoop is not a Bluetooth SIG or
tcpdump format; the layout follows Wireshark's `wiretap/btsnoop.c`.

### text

One line per value: `secs.micros`, `>` (to adapter) or `<` (from adapter), hex bytes, then ASCII.
No Bluetooth framing.

## Failure behaviour

The tap is a passive observer. It records after a value has been delivered (the adapter write
returned, or the socket write succeeded), so it cannot alter the relayed bytes. A capture write
failure is reported once on stderr and the tap switches itself off; the relay continues. A file that
cannot be created at startup is a startup error, since the capture was asked for.

The tap shares one mutex between both relay threads and flushes after each value, so a live reader
or a crash sees whole records. A stalled disk would therefore stall the relay for its duration; a
channel to a writer thread would remove that, at the cost of more machinery.

## Verified and unverified

Verified: the byte layout is asserted by unit tests in `capture.rs` (global header, record header,
H4 indicator, ACL header with flags, L2CAP header, ATT opcode and handle, exact bytes for one write
and one notification in pcap and for one notification in btsnoop) and a round trip through a small
pcap reader in the test module. The relay is shown unchanged with the tap on and with a failing tap
in `bridge.rs`.

Checked with Wireshark's `tshark`: a pcap and a btsnoop file, each holding one write and one
notification written by the tap, open without malformed-packet warnings. The dissector chain is
`bluetooth:hci_h4:bthci_acl:btl2cap:btatt`. Directions show as Sent and Rcvd; the connection handle,
the packet boundary and broadcast flags, lengths, CID `0x0004`, the ATT opcodes (Write Command,
Handle Value Notification), the handles and the values all decode as written, and the timestamps
read back exactly. Wireshark flags each frame "out of any connection handle session" and shows the
handles as "Unknown", because the capture has no connection setup or discovery.

Unverified: no real adapter was used, so a capture of live traffic has not been looked at. The
btsnoop layout rests on Wireshark's reader source, not on a Bluetooth clause.
