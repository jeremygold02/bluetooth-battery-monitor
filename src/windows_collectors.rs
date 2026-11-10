use crate::probe::ProbeResult;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristicUuids, GattCommunicationStatus, GattDeviceService, GattServiceUuids,
};
use windows::Devices::Bluetooth::{
    BluetoothCacheMode, BluetoothConnectionStatus, BluetoothLEDevice,
};
use windows::Devices::Enumeration::{DeviceInformation, DeviceWatcher, DeviceWatcherStatus};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, IBuffer};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO, SP_DEVINFO_DATA, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW,
};
use windows::Win32::Devices::Properties::DEVPROPTYPE;
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::core::{Error, GUID, Owned, PCWSTR, RuntimeType};

const DEVICE_PROPERTY_FORMAT: GUID = GUID::from_u128(0xA45C254E_DF1C_4EFD_8020_67D146A850E0);
const BATTERY_LIFE_KEY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x49CD1F76_5626_4B17_A4E8_18B4AA1A2213),
    pid: 10,
};
const FRIENDLY_NAME_KEY: DEVPROPKEY = DEVPROPKEY {
    fmtid: DEVICE_PROPERTY_FORMAT,
    pid: 14,
};
const DEVICE_DESCRIPTION_KEY: DEVPROPKEY = DEVPROPKEY {
    fmtid: DEVICE_PROPERTY_FORMAT,
    pid: 2,
};
const BATTERY_LEVEL_STATUS_UUID: GUID = GUID::from_u128(0x00002BED_0000_1000_8000_00805F9B34FB);

const WIRELESS_INSTANCE_PREFIXES: [&str; 4] = ["BTH", "HID", "USB", "SWD"];
const BATTERY_PROPERTY_BUFFER_BYTES: usize = 8;
const STRING_PROPERTY_BUFFER_BYTES: usize = 2048;
const INSTANCE_ID_BUFFER_CHARACTERS: usize = 1024;
const MAXIMUM_BATTERY_PERCENT: u8 = 100;
const BATTERY_LEVEL_MINIMUM_BYTES: u32 = 1;
const BATTERY_LEVEL_STATUS_MINIMUM_BYTES: u32 = 3;

const CONNECTED_STATUS: &str = "Connected";
const WINDOWS_SOURCE: &str = "Windows";
const WINDOWS_PRIORITY: i32 = 10;
const BLUETOOTH_LE_SOURCE: &str = "Bluetooth LE";
const BLUETOOTH_LE_PRIORITY: i32 = 20;
const MISSING_OBJECT_MESSAGE: &str = "Object reference not set to an instance of an object.";

static MULTITHREADED_APARTMENT_HELD: Mutex<bool> = Mutex::new(false);

struct BatteryLevelReading {
    percent: u8,
    status: &'static str,
}

pub fn collect_battery_properties() -> Result<Vec<ProbeResult>, String> {
    let mut results = Vec::new();
    let Ok(device_info_handle) = (unsafe {
        SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_ALLCLASSES)
    }) else {
        return Ok(results);
    };
    let device_info_set = unsafe { Owned::new(device_info_handle) };

    for member_index in 0..=u32::MAX {
        let mut device = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        if unsafe { SetupDiEnumDeviceInfo(*device_info_set, member_index, &mut device) }.is_err() {
            break;
        }
        let Some(percent) = read_battery_percent(*device_info_set, &device) else {
            continue;
        };
        let Some(instance_id) = read_instance_id(*device_info_set, &device) else {
            continue;
        };
        if !is_wireless_device_instance(&instance_id) {
            continue;
        }
        let friendly_name = read_string_property(*device_info_set, &device, &FRIENDLY_NAME_KEY);
        let name = if is_blank(&friendly_name) {
            read_string_property(*device_info_set, &device, &DEVICE_DESCRIPTION_KEY)
        } else {
            friendly_name
        };
        results.push(windows_battery_result(&instance_id, name, percent));
    }

    Ok(results)
}

pub fn collect_gatt_batteries(
    mut on_device_error: impl FnMut(String),
) -> Result<Vec<ProbeResult>, String> {
    collect_connected_gatt_batteries(&mut on_device_error).map_err(|error| describe_failure(&error))
}

pub struct ConnectionWatcher {
    watcher: DeviceWatcher,
    shared: Arc<WatcherShared>,
    registrations: WatcherRegistrations,
}

impl ConnectionWatcher {
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        hold_multithreaded_apartment().ok()?;
        let selector = BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(
            BluetoothConnectionStatus::Connected,
        )
        .ok()?;
        let watcher = DeviceInformation::CreateWatcherAqsFilter(&selector).ok()?;
        let mut connection_watcher = Self {
            watcher,
            shared: Arc::new(WatcherShared {
                gate: Mutex::default(),
                on_change: Box::new(on_change),
            }),
            registrations: WatcherRegistrations::default(),
        };
        connection_watcher.register_and_start().ok()?;
        Some(connection_watcher)
    }

    fn register_and_start(&mut self) -> windows::core::Result<()> {
        self.registrations.added = Some(
            self.watcher
                .Added(&watcher_event_handler(&self.shared, WatcherEvent::Added))?,
        );
        self.registrations.removed = Some(
            self.watcher
                .Removed(&watcher_event_handler(&self.shared, WatcherEvent::Removed))?,
        );
        self.registrations.enumeration_completed = Some(self.watcher.EnumerationCompleted(
            &watcher_event_handler(&self.shared, WatcherEvent::EnumerationCompleted),
        )?);
        self.watcher.Start()
    }
}

impl Drop for ConnectionWatcher {
    fn drop(&mut self) {
        lock_ignoring_poison(&self.shared.gate).stopped = true;
        if let Some(token) = self.registrations.added {
            let _ = self.watcher.RemoveAdded(token);
        }
        if let Some(token) = self.registrations.removed {
            let _ = self.watcher.RemoveRemoved(token);
        }
        if let Some(token) = self.registrations.enumeration_completed {
            let _ = self.watcher.RemoveEnumerationCompleted(token);
        }
        let is_running = self.watcher.Status().is_ok_and(|status| {
            status == DeviceWatcherStatus::Started
                || status == DeviceWatcherStatus::EnumerationCompleted
        });
        if is_running {
            let _ = self.watcher.Stop();
        }
    }
}

#[derive(Default)]
struct WatcherRegistrations {
    added: Option<i64>,
    removed: Option<i64>,
    enumeration_completed: Option<i64>,
}

struct WatcherShared {
    gate: Mutex<WatcherGate>,
    on_change: Box<dyn Fn() + Send + Sync>,
}

impl WatcherShared {
    fn handle(&self, event: WatcherEvent) {
        let should_notify = lock_ignoring_poison(&self.gate).should_notify(event);
        if should_notify {
            let _ = catch_unwind(AssertUnwindSafe(|| (self.on_change)()));
        }
    }
}

#[derive(Clone, Copy)]
enum WatcherEvent {
    Added,
    Removed,
    EnumerationCompleted,
}

#[derive(Default)]
struct WatcherGate {
    initialized: bool,
    removed_during_enumeration: bool,
    stopped: bool,
}

impl WatcherGate {
    fn should_notify(&mut self, event: WatcherEvent) -> bool {
        if self.stopped {
            return false;
        }
        match event {
            WatcherEvent::Added => self.initialized,
            WatcherEvent::Removed => {
                if !self.initialized {
                    self.removed_during_enumeration = true;
                }
                self.initialized
            }
            WatcherEvent::EnumerationCompleted => {
                self.initialized = true;
                std::mem::take(&mut self.removed_during_enumeration)
            }
        }
    }
}

fn watcher_event_handler<EventArguments: RuntimeType + 'static>(
    shared: &Arc<WatcherShared>,
    event: WatcherEvent,
) -> TypedEventHandler<DeviceWatcher, EventArguments> {
    let shared = Arc::clone(shared);
    TypedEventHandler::new(move |_, _| {
        shared.handle(event);
        Ok(())
    })
}

fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn hold_multithreaded_apartment() -> windows::core::Result<()> {
    let mut held = lock_ignoring_poison(&MULTITHREADED_APARTMENT_HELD);
    if !*held {
        unsafe { CoIncrementMTAUsage() }?;
        *held = true;
    }
    Ok(())
}

fn read_device_property(
    device_info_set: HDEVINFO,
    device: &SP_DEVINFO_DATA,
    key: &DEVPROPKEY,
    buffer: &mut [u8],
) -> Option<u32> {
    let mut property_type = DEVPROPTYPE::default();
    let mut required_size = 0;
    unsafe {
        SetupDiGetDevicePropertyW(
            device_info_set,
            device,
            key,
            &mut property_type,
            Some(buffer),
            Some(&mut required_size),
            0,
        )
    }
    .ok()?;
    Some(required_size)
}

fn read_battery_percent(device_info_set: HDEVINFO, device: &SP_DEVINFO_DATA) -> Option<u8> {
    let mut buffer = [0u8; BATTERY_PROPERTY_BUFFER_BYTES];
    let required_size =
        read_device_property(device_info_set, device, &BATTERY_LIFE_KEY, &mut buffer)?;
    battery_percent_from_property(&buffer, required_size)
}

fn read_string_property(
    device_info_set: HDEVINFO,
    device: &SP_DEVINFO_DATA,
    key: &DEVPROPKEY,
) -> String {
    let mut buffer = [0u8; STRING_PROPERTY_BUFFER_BYTES];
    read_device_property(device_info_set, device, key, &mut buffer)
        .map(|required_size| decode_string_property(&buffer, required_size))
        .unwrap_or_default()
}

fn read_instance_id(device_info_set: HDEVINFO, device: &SP_DEVINFO_DATA) -> Option<String> {
    let mut buffer = [0u16; INSTANCE_ID_BUFFER_CHARACTERS];
    unsafe { SetupDiGetDeviceInstanceIdW(device_info_set, device, Some(&mut buffer), None) }
        .ok()?;
    Some(decode_terminated_wide_string(&buffer))
}

fn battery_percent_from_property(buffer: &[u8], required_size: u32) -> Option<u8> {
    if required_size < 1 {
        return None;
    }
    buffer.first().copied().and_then(valid_battery_percent)
}

fn valid_battery_percent(level: u8) -> Option<u8> {
    (level <= MAXIMUM_BATTERY_PERCENT).then_some(level)
}

fn decode_string_property(buffer: &[u8], required_size: u32) -> String {
    if required_size < 2 {
        return String::new();
    }
    let length = usize::try_from(required_size).map_or(buffer.len(), |size| size.min(buffer.len()));
    let code_units = buffer[..length].chunks_exact(2);
    let has_dangling_byte = !code_units.remainder().is_empty();
    let mut text: String =
        char::decode_utf16(code_units.map(|pair| u16::from_le_bytes([pair[0], pair[1]])))
            .map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
    if has_dangling_byte {
        text.push(char::REPLACEMENT_CHARACTER);
    }
    text.trim_end_matches('\0').to_owned()
}

fn decode_terminated_wide_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

fn is_wireless_device_instance(instance_id: &str) -> bool {
    WIRELESS_INSTANCE_PREFIXES.iter().any(|prefix| {
        instance_id
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    })
}

fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

fn windows_battery_result(instance_id: &str, name: String, percent: u8) -> ProbeResult {
    ProbeResult {
        id: format!("windows:{instance_id}"),
        physical_id: None,
        name,
        percent: Some(percent),
        source: WINDOWS_SOURCE.to_owned(),
        status: CONNECTED_STATUS.to_owned(),
        priority: WINDOWS_PRIORITY,
    }
}

fn gatt_battery_result(
    device_id: &str,
    name: String,
    reading: &BatteryLevelReading,
) -> ProbeResult {
    ProbeResult {
        id: format!("ble:{device_id}"),
        physical_id: None,
        name,
        percent: Some(reading.percent),
        source: BLUETOOTH_LE_SOURCE.to_owned(),
        status: reading.status.to_owned(),
        priority: BLUETOOTH_LE_PRIORITY,
    }
}

fn describe_failure(error: &Error) -> String {
    let code = error.code();
    if code.is_ok() {
        return MISSING_OBJECT_MESSAGE.to_owned();
    }
    let message = error.message();
    if is_blank(&message) {
        format!("Exception from HRESULT: 0x{:08X}", code.0)
    } else {
        message
    }
}

fn present<T>(outcome: windows::core::Result<T>) -> windows::core::Result<Option<T>> {
    match outcome {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code().is_ok() => Ok(None),
        Err(error) => Err(error),
    }
}

fn collect_connected_gatt_batteries(
    on_device_error: &mut dyn FnMut(String),
) -> windows::core::Result<Vec<ProbeResult>> {
    hold_multithreaded_apartment()?;
    let selector = BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(
        BluetoothConnectionStatus::Connected,
    )?;
    let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
    let mut results = Vec::new();
    for device_index in 0..devices.Size()? {
        let information = devices.GetAt(device_index)?;
        if let Err(error) = append_gatt_battery(&information, &mut results) {
            on_device_error(describe_failure(&error));
        }
    }
    Ok(results)
}

fn append_gatt_battery(
    information: &DeviceInformation,
    results: &mut Vec<ProbeResult>,
) -> windows::core::Result<()> {
    let device_id = information.Id()?;
    let Some(device) = present(BluetoothLEDevice::FromIdAsync(&device_id)?.join())? else {
        return Ok(());
    };
    let outcome =
        append_open_device_battery(&device, information, &device_id.to_string_lossy(), results);
    device.Close().and(outcome)
}

fn append_open_device_battery(
    device: &BluetoothLEDevice,
    information: &DeviceInformation,
    device_id: &str,
    results: &mut Vec<ProbeResult>,
) -> windows::core::Result<()> {
    let Some(reading) = read_battery_level(device)? else {
        return Ok(());
    };
    let device_name = device.Name()?.to_string_lossy();
    let name = if is_blank(&device_name) {
        information.Name()?.to_string_lossy()
    } else {
        device_name
    };
    results.push(gatt_battery_result(device_id, name, &reading));
    Ok(())
}

fn read_battery_level(
    device: &BluetoothLEDevice,
) -> windows::core::Result<Option<BatteryLevelReading>> {
    let services_result = device
        .GetGattServicesForUuidWithCacheModeAsync(
            GattServiceUuids::Battery()?,
            BluetoothCacheMode::Uncached,
        )?
        .join()?;
    if services_result.Status()? != GattCommunicationStatus::Success {
        return Ok(None);
    }
    let services = services_result.Services()?;
    for service_index in 0..services.Size()? {
        let service = services.GetAt(service_index)?;
        let outcome = read_service_battery_level(&service);
        if let Some(reading) = service.Close().and(outcome)? {
            return Ok(Some(reading));
        }
    }
    Ok(None)
}

fn read_service_battery_level(
    service: &GattDeviceService,
) -> windows::core::Result<Option<BatteryLevelReading>> {
    find_characteristic_value(
        service,
        GattCharacteristicUuids::BatteryLevel()?,
        BATTERY_LEVEL_MINIMUM_BYTES,
        |value| {
            with_data_reader(value, |reader| {
                let level = reader.ReadByte()?;
                Ok(
                    valid_battery_percent(level).map(|percent| BatteryLevelReading {
                        percent,
                        status: read_battery_level_status(service),
                    }),
                )
            })
        },
    )
}

fn read_battery_level_status(service: &GattDeviceService) -> &'static str {
    find_characteristic_value(
        service,
        BATTERY_LEVEL_STATUS_UUID,
        BATTERY_LEVEL_STATUS_MINIMUM_BYTES,
        |value| {
            with_data_reader(value, |reader| {
                let mut bytes = vec![0u8; usize::try_from(value.Length()?)?];
                reader.ReadBytes(&mut bytes)?;
                Ok(Some(format_battery_level_status(&bytes)))
            })
        },
    )
    .ok()
    .flatten()
    .unwrap_or(CONNECTED_STATUS)
}

fn find_characteristic_value<T>(
    service: &GattDeviceService,
    characteristic_uuid: GUID,
    minimum_length: u32,
    mut interpret: impl FnMut(&IBuffer) -> windows::core::Result<Option<T>>,
) -> windows::core::Result<Option<T>> {
    let characteristics_result = service
        .GetCharacteristicsForUuidWithCacheModeAsync(
            characteristic_uuid,
            BluetoothCacheMode::Uncached,
        )?
        .join()?;
    if characteristics_result.Status()? != GattCommunicationStatus::Success {
        return Ok(None);
    }
    let characteristics = characteristics_result.Characteristics()?;
    for characteristic_index in 0..characteristics.Size()? {
        let read = characteristics
            .GetAt(characteristic_index)?
            .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
            .join()?;
        if read.Status()? != GattCommunicationStatus::Success {
            continue;
        }
        let Some(value) = present(read.Value())? else {
            continue;
        };
        if value.Length()? < minimum_length {
            continue;
        }
        if let Some(found) = interpret(&value)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn with_data_reader<T>(
    value: &IBuffer,
    read: impl FnOnce(&DataReader) -> windows::core::Result<T>,
) -> windows::core::Result<T> {
    let reader = DataReader::FromBuffer(value)?;
    let outcome = read(&reader);
    reader.Close().and(outcome)
}

fn format_battery_level_status(value: &[u8]) -> &'static str {
    let [flags, power_state, ..] = *value else {
        return CONNECTED_STATUS;
    };
    let required_length = 3
        + 2 * usize::from(flags & 1 != 0)
        + usize::from(flags & 2 != 0)
        + usize::from(flags & 4 != 0);
    if value.len() < required_length || power_state & 1 == 0 {
        return CONNECTED_STATUS;
    }
    match (power_state >> 5) & 3 {
        1 => "Charging",
        2 => "Discharging",
        3 => "Not charging",
        _ => CONNECTED_STATUS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};
    use windows::Storage::Streams::DataWriter;
    use windows::core::HRESULT;

    fn utf16_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn buffer_from(bytes: &[u8]) -> IBuffer {
        hold_multithreaded_apartment().expect("multithreaded apartment");
        let writer = DataWriter::new().expect("data writer");
        writer.WriteBytes(bytes).expect("written bytes");
        writer.DetachBuffer().expect("detached buffer")
    }

    fn read_bytes(buffer: &IBuffer, count: usize) -> windows::core::Result<Vec<u8>> {
        with_data_reader(buffer, |reader| {
            let mut bytes = vec![0u8; count];
            reader.ReadBytes(&mut bytes)?;
            Ok(bytes)
        })
    }

    #[test]
    fn wireless_instances_match_supported_enumerators_ignoring_case() {
        assert!(is_wireless_device_instance(
            "BTHENUM\\DEV_001122334455\\7&1A2B3C&0&BLUETOOTHDEVICE"
        ));
        assert!(is_wireless_device_instance(
            "BTHLE\\DEV_C1D2E3F4A5B6\\8&ABC&0"
        ));
        assert!(is_wireless_device_instance("hid\\vid_046d&pid_c539\\6&1&0"));
        assert!(is_wireless_device_instance("Usb\\VID_03F0&PID_02CC\\5&1"));
        assert!(is_wireless_device_instance(
            "SWD\\MMDEVAPI\\{0.0.0.00000000}"
        ));
        assert!(is_wireless_device_instance("SWD"));
    }

    #[test]
    fn other_instances_are_rejected() {
        assert!(!is_wireless_device_instance(""));
        assert!(!is_wireless_device_instance("   "));
        assert!(!is_wireless_device_instance("BT"));
        assert!(!is_wireless_device_instance(" BTHENUM\\DEV"));
        assert!(!is_wireless_device_instance("ACPI\\PNP0C0A\\1"));
        assert!(!is_wireless_device_instance("ROOT\\BTH\\0000"));
        assert!(!is_wireless_device_instance(
            "bd0c5f1e2a3b4c5d6e7f8091a2b3c4d5"
        ));
        assert!(!is_wireless_device_instance("ÜSB\\VID"));
    }

    #[test]
    fn battery_percent_requires_data_and_valid_range() {
        assert_eq!(
            battery_percent_from_property(&[0, 0, 0, 0, 0, 0, 0, 0], 1),
            Some(0)
        );
        assert_eq!(
            battery_percent_from_property(&[100, 9, 9, 9, 9, 9, 9, 9], 4),
            Some(100)
        );
        assert_eq!(
            battery_percent_from_property(&[101, 0, 0, 0, 0, 0, 0, 0], 1),
            None
        );
        assert_eq!(
            battery_percent_from_property(&[255, 0, 0, 0, 0, 0, 0, 0], 1),
            None
        );
        assert_eq!(
            battery_percent_from_property(&[55, 0, 0, 0, 0, 0, 0, 0], 0),
            None
        );
        assert_eq!(battery_percent_from_property(&[], 1), None);
    }

    #[test]
    fn string_property_decodes_utf16_and_trims_terminators() {
        let mut buffer = utf16_bytes("MX Master 3S\0");
        let required_size = buffer.len() as u32;
        buffer.resize(64, 0xAA);
        assert_eq!(
            decode_string_property(&buffer, required_size),
            "MX Master 3S"
        );
        assert_eq!(
            decode_string_property(&utf16_bytes("Kopfhörer 🎧\0\0"), 28),
            "Kopfhörer 🎧"
        );
        assert_eq!(decode_string_property(&utf16_bytes("  \0"), 6), "  ");
    }

    #[test]
    fn string_property_handles_short_oversized_and_malformed_data() {
        assert_eq!(decode_string_property(&utf16_bytes("A\0"), 0), "");
        assert_eq!(decode_string_property(&utf16_bytes("A\0"), 1), "");
        assert_eq!(decode_string_property(&utf16_bytes("AB"), 4096), "AB");
        assert_eq!(decode_string_property(&[0x41, 0x00, 0x42], 3), "A\u{FFFD}");
        assert_eq!(
            decode_string_property(&[0x00, 0xD8, 0x41, 0x00], 4),
            "\u{FFFD}A"
        );
        assert_eq!(decode_string_property(&[], 2), "");
    }

    #[test]
    fn instance_id_stops_at_first_terminator() {
        let mut buffer: Vec<u16> = "BTHLE\\DEV_AABBCCDDEEFF\\8&1&0".encode_utf16().collect();
        buffer.extend([0, u16::from(b'X'), 0]);
        assert_eq!(
            decode_terminated_wide_string(&buffer),
            "BTHLE\\DEV_AABBCCDDEEFF\\8&1&0"
        );
        assert_eq!(
            decode_terminated_wide_string(&[u16::from(b'A'), u16::from(b'B')]),
            "AB"
        );
        assert_eq!(decode_terminated_wide_string(&[0, 0]), "");
        assert_eq!(decode_terminated_wide_string(&[]), "");
    }

    #[test]
    fn blank_detection_matches_null_or_white_space() {
        assert!(is_blank(""));
        assert!(is_blank(" \t\r\n\u{00A0}\u{2003}\u{3000}"));
        assert!(!is_blank(" a "));
        assert!(!is_blank("\0"));
    }

    #[test]
    fn windows_result_uses_persisted_key_format() {
        let result =
            windows_battery_result("BTHLE\\DEV_AABBCCDDEEFF\\8&1&0", "Keyboard".to_owned(), 87);
        assert_eq!(
            result,
            ProbeResult {
                id: "windows:BTHLE\\DEV_AABBCCDDEEFF\\8&1&0".to_owned(),
                physical_id: None,
                name: "Keyboard".to_owned(),
                percent: Some(87),
                source: "Windows".to_owned(),
                status: "Connected".to_owned(),
                priority: 10,
            }
        );
    }

    #[test]
    fn gatt_result_uses_persisted_key_format() {
        let reading = BatteryLevelReading {
            percent: 42,
            status: "Charging",
        };
        let device_id = "BluetoothLE#BluetoothLE00:11:22:33:44:55-aa:bb:cc:dd:ee:ff";
        let result = gatt_battery_result(device_id, "Mouse".to_owned(), &reading);
        assert_eq!(
            result,
            ProbeResult {
                id: "ble:BluetoothLE#BluetoothLE00:11:22:33:44:55-aa:bb:cc:dd:ee:ff".to_owned(),
                physical_id: None,
                name: "Mouse".to_owned(),
                percent: Some(42),
                source: "Bluetooth LE".to_owned(),
                status: "Charging".to_owned(),
                priority: 20,
            }
        );
    }

    #[test]
    fn battery_level_accepts_zero_through_one_hundred() {
        assert_eq!(valid_battery_percent(0), Some(0));
        assert_eq!(valid_battery_percent(100), Some(100));
        assert_eq!(valid_battery_percent(101), None);
        assert_eq!(valid_battery_percent(255), None);
    }

    #[test]
    fn battery_level_status_reports_charge_state() {
        assert_eq!(format_battery_level_status(&[0x00, 0x21, 0x00]), "Charging");
        assert_eq!(
            format_battery_level_status(&[0x00, 0x41, 0x00]),
            "Discharging"
        );
        assert_eq!(
            format_battery_level_status(&[0x00, 0x61, 0x00]),
            "Not charging"
        );
        assert_eq!(
            format_battery_level_status(&[0x00, 0x01, 0x00]),
            "Connected"
        );
        assert_eq!(
            format_battery_level_status(&[0x00, 0xE3, 0xFF, 0x12]),
            "Not charging"
        );
    }

    #[test]
    fn battery_level_status_requires_present_battery_and_complete_fields() {
        assert_eq!(format_battery_level_status(&[]), "Connected");
        assert_eq!(format_battery_level_status(&[0x00, 0x21]), "Connected");
        assert_eq!(
            format_battery_level_status(&[0x00, 0x20, 0x00]),
            "Connected"
        );
        assert_eq!(
            format_battery_level_status(&[0x01, 0x21, 0x00, 0x00]),
            "Connected"
        );
        assert_eq!(
            format_battery_level_status(&[0x01, 0x21, 0x00, 0x00, 0x00]),
            "Charging"
        );
        assert_eq!(
            format_battery_level_status(&[0x02, 0x21, 0x00]),
            "Connected"
        );
        assert_eq!(
            format_battery_level_status(&[0x02, 0x21, 0x00, 0x00]),
            "Charging"
        );
        assert_eq!(
            format_battery_level_status(&[0x07, 0x41, 0x00, 0, 0, 0]),
            "Connected"
        );
        assert_eq!(
            format_battery_level_status(&[0x07, 0x41, 0x00, 0, 0, 0, 0]),
            "Discharging"
        );
    }

    #[test]
    fn watcher_stays_silent_until_enumeration_completes() {
        let mut gate = WatcherGate::default();
        assert!(!gate.should_notify(WatcherEvent::Added));
        assert!(!gate.should_notify(WatcherEvent::Added));
        assert!(!gate.should_notify(WatcherEvent::EnumerationCompleted));
        assert!(gate.should_notify(WatcherEvent::Added));
        assert!(gate.should_notify(WatcherEvent::Removed));
    }

    #[test]
    fn watcher_reports_removal_seen_during_enumeration_once() {
        let mut gate = WatcherGate::default();
        assert!(!gate.should_notify(WatcherEvent::Removed));
        assert!(!gate.should_notify(WatcherEvent::Removed));
        assert!(gate.should_notify(WatcherEvent::EnumerationCompleted));
        assert!(!gate.should_notify(WatcherEvent::EnumerationCompleted));
    }

    #[test]
    fn stopped_watcher_never_notifies() {
        let mut gate = WatcherGate::default();
        assert!(!gate.should_notify(WatcherEvent::Removed));
        gate.stopped = true;
        assert!(!gate.should_notify(WatcherEvent::EnumerationCompleted));
        assert!(!gate.should_notify(WatcherEvent::Added));
        assert!(!gate.should_notify(WatcherEvent::Removed));
        assert!(!gate.initialized);
    }

    #[test]
    fn shared_watcher_state_invokes_callback_and_survives_panics() {
        let notifications = Arc::new(Mutex::new(0));
        let counter = Arc::clone(&notifications);
        let shared = WatcherShared {
            gate: Mutex::default(),
            on_change: Box::new(move || {
                let mut count = lock_ignoring_poison(&counter);
                *count += 1;
                if *count == 2 {
                    std::panic::resume_unwind(Box::new("second notification fails"));
                }
            }),
        };
        shared.handle(WatcherEvent::Added);
        shared.handle(WatcherEvent::EnumerationCompleted);
        assert_eq!(*lock_ignoring_poison(&notifications), 0);
        shared.handle(WatcherEvent::Added);
        shared.handle(WatcherEvent::Removed);
        shared.handle(WatcherEvent::Added);
        assert_eq!(*lock_ignoring_poison(&notifications), 3);
    }

    #[test]
    fn missing_objects_are_separated_from_failures() {
        assert_eq!(present(Ok(7)), Ok(Some(7)));
        assert_eq!(present::<u8>(Err(Error::empty())), Ok(None));
        let failure = Error::from_hresult(HRESULT(0x8000_4005_u32 as i32));
        assert_eq!(present::<u8>(Err(failure.clone())), Err(failure));
    }

    #[test]
    fn data_reader_scope_returns_values_and_propagates_failures() {
        let buffer = buffer_from(&[87, 0x21, 0x00]);
        assert_eq!(buffer.Length(), Ok(3));
        assert_eq!(
            with_data_reader(&buffer, |reader| reader.ReadByte()),
            Ok(87)
        );
        assert_eq!(read_bytes(&buffer, 3), Ok(vec![87, 0x21, 0x00]));
        assert!(read_bytes(&buffer, 4).is_err());
        assert_eq!(
            with_data_reader(&buffer, |_| Err::<u8, _>(Error::empty())),
            Err(Error::empty())
        );
    }

    #[test]
    #[ignore = "queries the devices connected to this computer"]
    fn device_without_bluetooth_le_support_yields_no_result() {
        hold_multithreaded_apartment().expect("multithreaded apartment");
        let devices = DeviceInformation::FindAllAsync()
            .and_then(|operation| operation.join())
            .expect("device list");
        let information = devices.GetAt(0).expect("first device");
        let mut results = Vec::new();
        if let Err(failure) = append_gatt_battery(&information, &mut results) {
            assert!(!is_blank(&describe_failure(&failure)));
        }
        assert!(results.is_empty());
    }

    #[test]
    #[ignore = "queries the devices connected to this computer"]
    fn collectors_return_valid_readings_from_connected_hardware() {
        let windows_results = collect_battery_properties().expect("Windows battery properties");
        let mut device_errors = Vec::new();
        let gatt_results = collect_gatt_batteries(|message| device_errors.push(message))
            .expect("Bluetooth LE batteries");
        println!("{windows_results:#?}\n{gatt_results:#?}\n{device_errors:#?}");
        for result in &windows_results {
            let instance_id = result
                .id
                .strip_prefix("windows:")
                .expect("Windows id prefix");
            assert!(is_wireless_device_instance(instance_id));
            assert_eq!((result.source.as_str(), result.priority), ("Windows", 10));
            assert_eq!(result.status, "Connected");
        }
        for result in &gatt_results {
            assert!(result.id.starts_with("ble:"));
            assert_eq!(
                (result.source.as_str(), result.priority),
                ("Bluetooth LE", 20)
            );
        }
        for result in windows_results.iter().chain(&gatt_results) {
            assert!(result.percent.is_some_and(|percent| percent <= 100));
            assert_eq!(result.physical_id, None);
        }
    }

    #[test]
    #[ignore = "watches the devices connected to this computer"]
    fn watcher_completes_initial_enumeration_and_stops_on_drop() {
        let connection_watcher = ConnectionWatcher::start(|| {}).expect("connection watcher");
        let device_watcher = connection_watcher.watcher.clone();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !lock_ignoring_poison(&connection_watcher.shared.gate).initialized
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(lock_ignoring_poison(&connection_watcher.shared.gate).initialized);
        assert_eq!(
            device_watcher.Status(),
            Ok(DeviceWatcherStatus::EnumerationCompleted)
        );
        drop(connection_watcher);
        let deadline = Instant::now() + Duration::from_secs(10);
        while device_watcher.Status() != Ok(DeviceWatcherStatus::Stopped)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(device_watcher.Status(), Ok(DeviceWatcherStatus::Stopped));
    }

    #[test]
    fn failures_always_have_a_description() {
        assert_eq!(describe_failure(&Error::empty()), MISSING_OBJECT_MESSAGE);
        let unknown = Error::from_hresult(HRESULT(0xA123_4567_u32 as i32));
        assert_eq!(
            describe_failure(&unknown),
            "Exception from HRESULT: 0xA1234567"
        );
        let known = Error::from_hresult(HRESULT(0x8007_048F_u32 as i32));
        assert!(!is_blank(&describe_failure(&known)));
    }
}
