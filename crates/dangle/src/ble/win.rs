//! Bluetooth LE access on Windows via the first-party `windows` crate (`WinRT`
//! `Windows.Devices.Bluetooth`). This is the Windows backend behind the platform-neutral [`crate::ble`]
//! facade.
//!
//! The `WinRT` asynchronous operations are driven synchronously with
//! [`IAsyncOperation::get`](windows::Foundation::IAsyncOperation) on ordinary worker threads, so no
//! async runtime is pulled in. The projected `WinRT` types are agile (`Send + Sync`), which lets a
//! connected [`BleSerial`] be handed to the bridge's pump threads.
//!
//! COM apartment: the process relies on the Windows *implicit MTA*. `WinRT` activation and the async
//! `get()` calls join it automatically on a thread that has not initialised COM, and notification
//! callbacks are delivered on MTA thread-pool threads. No explicit `RoInitialize`/`CoInitializeEx`
//! is needed (and avoiding it keeps this crate free of `unsafe`).

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattCharacteristicProperties,
    GattClientCharacteristicConfigurationDescriptorValue, GattCommunicationStatus,
    GattDeviceService, GattValueChangedEventArgs, GattWriteOption,
};
use windows::Devices::Bluetooth::{BluetoothAdapter, BluetoothLEDevice};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Storage::Streams::{DataReader, DataWriter};
use windows::core::GUID;

use crate::BridgeResult;
use crate::config::{
    Advert, Config, ScanFilter, Seen, SerialConfig, doctor_row, format_addr, rank_advertisers,
    uuid_label,
};
use crate::link::{SerialLink, write_chunked};

/// Convert a `WinRT` [`GUID`] to its 128-bit value (the inverse of [`GUID::from_u128`]).
fn guid_to_u128(g: GUID) -> u128 {
    (u128::from(g.data1) << 96)
        | (u128::from(g.data2) << 80)
        | (u128::from(g.data3) << 64)
        | u128::from(u64::from_be_bytes(g.data4))
}

/// Run the `WinRT` advertisement watcher in active-scan mode for `scan_timeout`, accumulating one
/// [`Seen`] per device address (strongest RSSI, union of names and advertised service UUIDs).
///
/// # Errors
/// Returns an error if the advertisement watcher cannot be created or started.
fn collect_seen(scan_timeout: Duration) -> BridgeResult<BTreeMap<u64, Seen>> {
    let watcher = BluetoothLEAdvertisementWatcher::new()?;
    watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;

    let (tx, rx) = mpsc::channel::<(u64, String, i16, Vec<u128>)>();
    let tx = Arc::new(Mutex::new(tx));
    let handler = TypedEventHandler::new(
        move |_sender, args: &Option<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(args) = args.as_ref() {
                let addr = args.BluetoothAddress()?;
                let rssi = args.RawSignalStrengthInDBm().unwrap_or(0);
                let adv = args.Advertisement()?;
                let name = adv.LocalName().map(|h| h.to_string()).unwrap_or_default();
                let mut services = Vec::new();
                if let Ok(uuids) = adv.ServiceUuids() {
                    for i in 0..uuids.Size().unwrap_or(0) {
                        if let Ok(g) = uuids.GetAt(i) {
                            services.push(guid_to_u128(g));
                        }
                    }
                }
                if let Ok(tx) = tx.lock() {
                    let _ = tx.send((addr, name, rssi, services));
                }
            }
            Ok(())
        },
    );

    let token = watcher.Received(&handler)?;
    watcher.Start()?;

    let deadline = Instant::now() + scan_timeout;
    let mut seen: BTreeMap<u64, Seen> = BTreeMap::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok((addr, name, rssi, services)) => {
                let entry = seen.entry(addr).or_insert_with(|| Seen {
                    addr,
                    name: String::new(),
                    rssi: i16::MIN,
                    services: Vec::new(),
                });
                if !name.is_empty() {
                    entry.name = name;
                }
                if rssi > entry.rssi {
                    entry.rssi = rssi;
                }
                for s in services {
                    if !entry.services.contains(&s) {
                        entry.services.push(s);
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);
    Ok(seen)
}

/// Scan the host radio for BLE advertisers and return them as typed [`Advert`] rows.
///
/// Runs a `WinRT` [`BluetoothLEAdvertisementWatcher`] in active-scan mode for the whole of
/// `scan_timeout` (so weak and slow advertisers are seen), keeps those passing `filter`, marks the
/// ones advertising `serial` (the serial GATT service, default [`crate::DEFAULT_SERVICE_UUID`] =
/// `0xFFF0`), and orders them serial-service first then strongest signal. This opens no GATT
/// connection and binds no socket.
///
/// This is the reusable form of the `scan` doctor: the binary's doctor just formats the result, and
/// a sibling crate can read the rows to present a chooser before calling [`connect`] with the chosen
/// [`Advert::address`].
///
/// # Errors
/// Returns an error if the advertisement watcher cannot be created or started.
pub fn scan_advertisers(
    scan_timeout: Duration,
    filter: &ScanFilter,
    serial: u128,
) -> BridgeResult<Vec<Advert>> {
    let seen = collect_seen(scan_timeout)?;
    Ok(rank_advertisers(seen.into_values(), filter, serial))
}

/// Pre-flight doctor: report the host adapter and list the BLE advertisers the radio sees, marking
/// the one that carries the serial service, then return. This is the `scan` subcommand: it formats
/// the result of [`scan_advertisers`]. It opens no GATT connection and binds no socket; it is the
/// read-only check an operator runs before a capture to confirm the adapter is visible and to read
/// off the `--mac`/`--service` to use.
///
/// # Errors
/// Returns an error only if the advertisement watcher cannot be created or started.
pub fn doctor(cfg: &Config) -> BridgeResult<()> {
    match BluetoothAdapter::GetDefaultAsync().and_then(|op| op.get()) {
        Ok(adapter) => {
            let le = adapter.IsLowEnergySupported().unwrap_or(false);
            println!(
                "host Bluetooth adapter: present, Bluetooth LE {}",
                if le { "supported" } else { "NOT supported" }
            );
        }
        Err(e) => {
            println!("host Bluetooth adapter: could not query ({e}); is Bluetooth turned on?");
        }
    }

    println!("scanning for {:?} ...", cfg.scan_timeout);
    let rows = scan_advertisers(cfg.scan_timeout, &cfg.scan_filter(), cfg.service_uuid)?;

    if rows.is_empty() {
        println!(
            "no matching BLE advertisers seen. Is the dongle powered (e.g. an OBD adapter plugged into the OBD port) \
             and in range? Pairing it once in Windows Bluetooth settings helps."
        );
    } else {
        println!(
            "{} BLE advertiser(s) seen (serial service {} marked):",
            rows.len(),
            uuid_label(cfg.service_uuid)
        );
        for s in &rows {
            println!("  {}", doctor_row(s, cfg.service_uuid));
        }
    }
    Ok(())
}

/// Scan for a BLE adapter matching the configured name/address filters and return its 48-bit
/// Bluetooth address.
///
/// Uses a [`BluetoothLEAdvertisementWatcher`] in active-scan mode and returns the first matching
/// advertiser, or an error if none is seen within [`Config::scan_timeout`].
///
/// # Errors
/// Returns an error if the watcher cannot start or no matching adapter advertises in time.
pub fn scan_for_adapter(cfg: &Config) -> BridgeResult<(u64, String)> {
    let watcher = BluetoothLEAdvertisementWatcher::new()?;
    watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;

    let (tx, rx) = mpsc::channel::<(u64, String)>();
    let tx = Arc::new(Mutex::new(tx));
    let name = cfg.name_filter.clone();
    let mac = cfg.mac_filter;

    let handler = TypedEventHandler::new(
        move |_sender, args: &Option<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(args) = args.as_ref() {
                let addr = args.BluetoothAddress()?;
                let local = args
                    .Advertisement()
                    .and_then(|a| a.LocalName())
                    .map(|h| h.to_string())
                    .unwrap_or_default();
                let name_ok = name
                    .as_deref()
                    .is_none_or(|n| local.to_lowercase().contains(&n.to_lowercase()));
                let mac_ok = mac.is_none_or(|m| m == addr);
                if name_ok && mac_ok {
                    if let Ok(tx) = tx.lock() {
                        let _ = tx.send((addr, local));
                    }
                }
            }
            Ok(())
        },
    );

    let token = watcher.Received(&handler)?;
    watcher.Start()?;
    let result = rx.recv_timeout(cfg.scan_timeout);
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);

    result.map_err(|_| {
        format!(
            "no matching BLE adapter seen within {:?} (name filter {:?}, mac filter {:?})",
            cfg.scan_timeout,
            cfg.name_filter,
            cfg.mac_filter.map(format_addr)
        )
        .into()
    })
}

/// A connected BLE adapter exposed as a raw serial byte sink.
///
/// [`write`](Self::write) sends bytes to the adapter (chunked to the configured MTU). Bytes the
/// adapter sends back arrive on the [`Receiver`] that [`connect`] returns alongside this value. The
/// bridge relays both directions verbatim; no OBD/ELM327 parsing happens here.
///
/// The notification [`Receiver`] is kept out of this struct deliberately: that keeps `BleSerial`
/// `Sync`, so a shared `&BleSerial` can be used from the bridge's TCP-reader thread while the
/// receiver is drained on another thread.
pub struct BleSerial {
    write_char: GattCharacteristic,
    write_option: GattWriteOption,
    max_write: usize,
    notify_char: GattCharacteristic,
    value_changed_token: EventRegistrationToken,
    // Held so the underlying GATT connection stays open for this value's lifetime.
    _service: GattDeviceService,
    _device: BluetoothLEDevice,
}

fn has(props: GattCharacteristicProperties, flag: GattCharacteristicProperties) -> bool {
    props.contains(flag)
}

/// Open the configured serial GATT service on a connected device.
///
/// # Errors
/// Returns an error if the service query fails or the service is absent (adapter out of range or
/// not advertising the expected service).
fn open_service(device: &BluetoothLEDevice, service_uuid: u128) -> BridgeResult<GattDeviceService> {
    let services = device
        .GetGattServicesForUuidAsync(GUID::from_u128(service_uuid))?
        .get()?;
    if services.Status()? != GattCommunicationStatus::Success {
        return Err(format!(
            "GATT service query failed (status {:?}); is the adapter paired/in range?",
            services.Status()?
        )
        .into());
    }
    let service_list = services.Services()?;
    if service_list.Size()? == 0 {
        return Err(format!("serial GATT service {service_uuid:032x} not found on adapter").into());
    }
    Ok(service_list.GetAt(0)?)
}

/// The characteristics chosen for the serial stream.
struct Selected {
    write_char: GattCharacteristic,
    write_option: GattWriteOption,
    notify_char: GattCharacteristic,
    /// `true` if the notify characteristic is subscribed via Notify, `false` via Indicate.
    is_notify: bool,
}

/// Enumerate the service's characteristics and pick the write and notify characteristics, by
/// explicit UUID from `cfg` when given, otherwise by GATT property.
///
/// # Errors
/// Returns an error if the characteristic query fails or either required characteristic is absent.
fn select_characteristics(
    service: &GattDeviceService,
    cfg: &SerialConfig,
) -> BridgeResult<Selected> {
    let chars_result = service.GetCharacteristicsAsync()?.get()?;
    if chars_result.Status()? != GattCommunicationStatus::Success {
        return Err(format!(
            "GATT characteristic query failed (status {:?})",
            chars_result.Status()?
        )
        .into());
    }
    let chars = chars_result.Characteristics()?;

    let mut write: Option<(GattCharacteristic, GattWriteOption)> = None;
    let mut notify: Option<(GattCharacteristic, bool)> = None;

    for i in 0..chars.Size()? {
        let ch = chars.GetAt(i)?;
        let uuid = ch.Uuid()?;
        let props = ch.CharacteristicProperties()?;

        let want_write = cfg.write_uuid.map_or_else(
            || {
                has(props, GattCharacteristicProperties::Write)
                    || has(props, GattCharacteristicProperties::WriteWithoutResponse)
            },
            |u| uuid == GUID::from_u128(u),
        );
        if want_write && write.is_none() {
            let option = if has(props, GattCharacteristicProperties::WriteWithoutResponse) {
                GattWriteOption::WriteWithoutResponse
            } else {
                GattWriteOption::WriteWithResponse
            };
            write = Some((ch.clone(), option));
        }

        let want_notify = cfg.notify_uuid.map_or_else(
            || {
                has(props, GattCharacteristicProperties::Notify)
                    || has(props, GattCharacteristicProperties::Indicate)
            },
            |u| uuid == GUID::from_u128(u),
        );
        if want_notify && notify.is_none() {
            notify = Some((ch.clone(), has(props, GattCharacteristicProperties::Notify)));
        }
    }

    let (write_char, write_option) =
        write.ok_or("no writable characteristic found in serial service")?;
    let (notify_char, is_notify) =
        notify.ok_or("no notify/indicate characteristic found in serial service")?;
    Ok(Selected {
        write_char,
        write_option,
        notify_char,
        is_notify,
    })
}

/// Connect to the adapter at `address`, discover the serial service and its write/notify
/// characteristics, subscribe to notifications, and return a ready [`BleSerial`].
///
/// The write and notify characteristics are chosen by GATT property (Write/WriteWithoutResponse and
/// Notify/Indicate respectively) unless overridden by an explicit UUID in `cfg`; this mirrors how
/// the `OBDLink` app discovers its Tx/Rx characteristics rather than hard-coding them (see
/// `docs/ble-re.md`).
///
/// On success returns the [`BleSerial`] sink together with a [`Receiver`] that yields the byte
/// chunks the adapter sends back, in arrival order.
///
/// `address` is a 48-bit Bluetooth device address, such as an [`Advert::address`] from
/// [`scan_advertisers`]. `cfg` is the serial-service profile to open; [`SerialConfig::default`] is
/// the `OBDLink`-class profile (service `0xFFF0`, characteristics by property, 20-byte writes).
///
/// # Errors
/// Returns an error if the device cannot be reached, the service or a required characteristic is
/// absent, or the notification subscription fails.
pub fn connect(address: u64, cfg: &SerialConfig) -> BridgeResult<(BleSerial, Receiver<Vec<u8>>)> {
    let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
    let service = open_service(&device, cfg.service_uuid)?;
    let Selected {
        write_char,
        write_option,
        notify_char,
        is_notify,
    } = select_characteristics(&service, cfg)?;

    let cccd = if is_notify {
        GattClientCharacteristicConfigurationDescriptorValue::Notify
    } else {
        GattClientCharacteristicConfigurationDescriptorValue::Indicate
    };
    let status = notify_char
        .WriteClientCharacteristicConfigurationDescriptorAsync(cccd)?
        .get()?;
    if status != GattCommunicationStatus::Success {
        return Err(format!("failed to enable notifications (status {status:?})").into());
    }

    let (ntx, notify_rx) = mpsc::channel::<Vec<u8>>();
    let ntx = Arc::new(Mutex::new(ntx));
    let value_handler =
        TypedEventHandler::new(move |_ch, args: &Option<GattValueChangedEventArgs>| {
            if let Some(args) = args.as_ref() {
                let buf = args.CharacteristicValue()?;
                let len = buf.Length()? as usize;
                if len > 0 {
                    let reader = DataReader::FromBuffer(&buf)?;
                    let mut bytes = vec![0u8; len];
                    reader.ReadBytes(&mut bytes)?;
                    if let Ok(ntx) = ntx.lock() {
                        let _ = ntx.send(bytes);
                    }
                }
            }
            Ok(())
        });
    let value_changed_token = notify_char.ValueChanged(&value_handler)?;

    Ok((
        BleSerial {
            write_char,
            write_option,
            max_write: cfg.max_write,
            notify_char,
            value_changed_token,
            _service: service,
            _device: device,
        },
        notify_rx,
    ))
}

impl SerialLink for BleSerial {
    /// Write `data` to the adapter, split into MTU-sized GATT writes.
    ///
    /// # Errors
    /// Returns an error if any GATT write does not complete successfully (typically the adapter
    /// disconnected), which the caller treats as a signal to reconnect.
    fn write(&self, data: &[u8]) -> BridgeResult<()> {
        write_chunked(self.max_write, data, |chunk| {
            let writer = DataWriter::new()?;
            writer.WriteBytes(chunk)?;
            let buffer = writer.DetachBuffer()?;
            let status = self
                .write_char
                .WriteValueWithOptionAsync(&buffer, self.write_option)?
                .get()?;
            if status != GattCommunicationStatus::Success {
                return Err(format!("GATT write failed (status {status:?})").into());
            }
            Ok(())
        })
    }
}

impl Drop for BleSerial {
    fn drop(&mut self) {
        let _ = self
            .notify_char
            .RemoveValueChanged(self.value_changed_token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::short_uuid;

    #[test]
    fn guid_to_u128_inverts_from_u128() {
        let serial = short_uuid(0xFFF0);
        assert_eq!(guid_to_u128(GUID::from_u128(serial)), serial);
        let full = 0x1234_5678_9abc_def0_1122_3344_5566_7788_u128;
        assert_eq!(guid_to_u128(GUID::from_u128(full)), full);
    }
}
