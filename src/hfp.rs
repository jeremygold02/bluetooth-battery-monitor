use crate::probe::ProbeResult;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Devices::Bluetooth::{
    BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS, BluetoothFindDeviceClose,
    BluetoothFindFirstDevice, BluetoothFindNextDevice, HBLUETOOTH_DEVICE_FIND,
};
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, GetLastError};
use windows_sys::Win32::System::Diagnostics::Etw::{
    CONTROLTRACE_HANDLE, CloseTrace, ENABLE_TRACE_PARAMETERS, ENABLE_TRACE_PARAMETERS_VERSION_2,
    EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, EnableTraceEx2, OpenTraceW, PROCESS_TRACE_MODE_EVENT_RECORD,
    PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE, PROPERTY_DATA_DESCRIPTOR, ProcessTrace,
    StartTraceW, StopTraceW, TdhGetProperty, TdhGetPropertySize, WNODE_FLAG_TRACED_GUID,
};
use windows_sys::core::GUID;

const BTHPORT_PROVIDER_GUID: GUID = GUID::from_u128(0x8A1F_9517_3A8C_4A9E_A018_4F17_A200_F277);
const HFP_FILTER_PROVIDER_GUID: GUID = GUID::from_u128(0xB0D4_845E_67D3_4F9E_9349_5862_EBF0_1844);
const BTHPORT_ACL_TRANSFER_EVENT_ID: u16 = 247;
const HFP_FILTER_BATTERY_EVENT_ID: u16 = 1;
const BTHPORT_L2CAP_KEYWORD: u64 = 0x2000_0000_0000_0000;
const HFP_FILTER_KEYWORD: u64 = 1;
const HFP_SOURCE: &str = "Bluetooth HFP";
const HFP_PRIORITY: i32 = 25;
const BLUETOOTH_ADDRESS_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const MAXIMUM_EVENT_PROPERTY_BYTES: usize = 8192;
const MAXIMUM_RECENT_PACKET_BYTES: usize = 512;
const LOGGER_NAME_CAPACITY: usize = 128;

struct HfpShared {
    pending_batteries: Mutex<HashMap<u64, u8>>,
    recent_packet_bytes: Mutex<HashMap<u64, Vec<u8>>>,
    runtime_error: Mutex<Option<String>>,
    on_update: Box<dyn Fn() + Send + Sync>,
}

pub struct HfpMonitor {
    shared: Arc<HfpShared>,
    trace_handle: PROCESSTRACE_HANDLE,
    worker: Option<JoinHandle<()>>,
}

impl HfpMonitor {
    pub fn start(on_update: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let shared = Arc::new(HfpShared {
            pending_batteries: Mutex::default(),
            recent_packet_bytes: Mutex::default(),
            runtime_error: Mutex::default(),
            on_update: Box::new(on_update),
        });
        let worker_shared = Arc::clone(&shared);
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("Bluetooth HFP ETW".to_owned())
            .spawn(move || {
                let (session_name, session_name_wide, session_guid) = new_session_identity();
                let (session, trace_handle) = match EtwSession::start(
                    session_name,
                    session_name_wide,
                    session_guid,
                    &worker_shared,
                ) {
                    Ok(session) => session,
                    Err(message) => {
                        let _ = started_tx.send(Err(message));
                        return;
                    }
                };
                if started_tx.send(Ok(trace_handle)).is_err() {
                    unsafe { CloseTrace(trace_handle) };
                    return;
                }

                let status = unsafe { ProcessTrace(&trace_handle, 1, null(), null()) };
                if status != 0 {
                    *lock(&worker_shared.runtime_error) = Some(trace_error("ProcessTrace", status));
                    (worker_shared.on_update)();
                }
                drop(session);
            })
            .map_err(|error| format!("Could not start the HFP event worker: {error}"))?;

        let trace_handle = match started_rx.recv() {
            Ok(Ok(handle)) => handle,
            Ok(Err(message)) => {
                let _ = worker.join();
                return Err(message);
            }
            Err(_) => {
                let _ = worker.join();
                return Err("The HFP event worker stopped before opening its trace".to_owned());
            }
        };

        Ok(Self {
            shared,
            trace_handle,
            worker: Some(worker),
        })
    }

    pub fn take_battery_probes(&self) -> (Vec<ProbeResult>, Option<String>) {
        let mut readings: Vec<_> = std::mem::take(&mut *lock(&self.shared.pending_batteries))
            .into_iter()
            .collect();
        readings.sort_by_key(|(address, _)| *address);
        if readings.is_empty() {
            return (Vec::new(), lock(&self.shared.runtime_error).take());
        }
        let names = bluetooth_device_names();
        let probes = readings
            .into_iter()
            .map(|(address, percent)| ProbeResult {
                id: format!("hfp:{address:012X}"),
                physical_id: None,
                name: names
                    .get(&address)
                    .filter(|name| !name.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Wireless device {address:012X}")),
                percent: Some(percent),
                source: HFP_SOURCE.to_owned(),
                status: "Connected".to_owned(),
                priority: HFP_PRIORITY,
            })
            .collect();
        let runtime_error = lock(&self.shared.runtime_error).take();
        (probes, runtime_error)
    }
}

impl Drop for HfpMonitor {
    fn drop(&mut self) {
        unsafe { CloseTrace(self.trace_handle) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[repr(C)]
struct TracePropertiesBuffer {
    properties: EVENT_TRACE_PROPERTIES,
    logger_name: [u16; LOGGER_NAME_CAPACITY],
}

impl TracePropertiesBuffer {
    fn new(logger_name: &str, session_guid: GUID) -> Result<Self, String> {
        let mut buffer = Self {
            properties: EVENT_TRACE_PROPERTIES::default(),
            logger_name: [0; LOGGER_NAME_CAPACITY],
        };
        let encoded: Vec<u16> = logger_name.encode_utf16().collect();
        if encoded.len() + 1 > buffer.logger_name.len() {
            return Err("The HFP trace session name is too long".to_owned());
        }
        buffer.logger_name[..encoded.len()].copy_from_slice(&encoded);
        buffer.logger_name[encoded.len()] = 0;

        buffer.properties.Wnode.BufferSize = size_of::<Self>() as u32;
        buffer.properties.Wnode.Guid = session_guid;
        buffer.properties.Wnode.ClientContext = 1;
        buffer.properties.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        buffer.properties.BufferSize = 16;
        buffer.properties.MinimumBuffers = 2;
        buffer.properties.MaximumBuffers = 8;
        buffer.properties.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        buffer.properties.FlushTimer = 1;
        buffer.properties.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        Ok(buffer)
    }
}

struct EtwSession {
    control_handle: CONTROLTRACE_HANDLE,
    logger_name: Vec<u16>,
    properties: TracePropertiesBuffer,
}

impl EtwSession {
    fn start(
        logger_name: String,
        logger_name_wide: Vec<u16>,
        session_guid: GUID,
        shared: &Arc<HfpShared>,
    ) -> Result<(Self, PROCESSTRACE_HANDLE), String> {
        let mut properties = TracePropertiesBuffer::new(&logger_name, session_guid)?;
        let mut control_handle = CONTROLTRACE_HANDLE { Value: 0 };
        let status = unsafe {
            StartTraceW(
                &mut control_handle,
                logger_name_wide.as_ptr(),
                &mut properties.properties,
            )
        };
        if status != 0 {
            return Err(trace_error("StartTraceW", status));
        }

        let session = Self {
            control_handle,
            logger_name: logger_name_wide,
            properties,
        };
        let enable_parameters = ENABLE_TRACE_PARAMETERS {
            Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
            ..Default::default()
        };
        let status = unsafe {
            EnableTraceEx2(
                control_handle,
                &BTHPORT_PROVIDER_GUID,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER,
                5,
                BTHPORT_L2CAP_KEYWORD,
                0,
                0,
                &enable_parameters,
            )
        };
        if status != 0 {
            return Err(trace_error("EnableTraceEx2", status));
        }

        let status = unsafe {
            EnableTraceEx2(
                control_handle,
                &HFP_FILTER_PROVIDER_GUID,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER,
                5,
                HFP_FILTER_KEYWORD,
                0,
                0,
                &enable_parameters,
            )
        };
        if status != 0 {
            *lock(&shared.runtime_error) =
                Some(trace_error("EnableTraceEx2(HFP filter)", status));
        }

        let mut logfile = EVENT_TRACE_LOGFILEW::default();
        logfile.LoggerName = session.logger_name.as_ptr().cast_mut();
        logfile.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event_record);
        logfile.Context = Arc::as_ptr(shared).cast_mut().cast::<c_void>();
        let trace_handle = unsafe { OpenTraceW(&mut logfile) };
        if trace_handle.Value == u64::MAX {
            let error = unsafe { GetLastError() };
            return Err(trace_error("OpenTraceW", error));
        }
        Ok((session, trace_handle))
    }
}

impl Drop for EtwSession {
    fn drop(&mut self) {
        unsafe {
            StopTraceW(
                self.control_handle,
                self.logger_name.as_ptr(),
                &mut self.properties.properties,
            );
        }
    }
}

fn new_session_identity() -> (String, Vec<u16>, GUID) {
    let process_id = std::process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let name = format!("Bluetooth Battery Monitor HFP {process_id} {timestamp:016X}");
    let name_wide = name.encode_utf16().chain(std::iter::once(0)).collect();
    let guid = GUID::from_u128((u128::from(process_id) << 64) | u128::from(timestamp));
    (name, name_wide, guid)
}

unsafe extern "system" fn on_event_record(record: *mut EVENT_RECORD) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        process_event_record(record)
    }));
}

fn process_event_record(record: *mut EVENT_RECORD) {
    let Some(record) = (unsafe { record.as_ref() }) else {
        return;
    };
    if record.UserContext.is_null() {
        return;
    }
    let shared = unsafe { &*record.UserContext.cast::<HfpShared>() };

    if same_guid(&record.EventHeader.ProviderId, &HFP_FILTER_PROVIDER_GUID) {
        if record.EventHeader.EventDescriptor.Id != HFP_FILTER_BATTERY_EVENT_ID
            || record.UserData.is_null()
            || record.UserDataLength < 9
        {
            return;
        }
        let payload = unsafe {
            std::slice::from_raw_parts(
                record.UserData.cast::<u8>(),
                usize::from(record.UserDataLength),
            )
        };
        let Some(address) = read_bluetooth_address(&payload[..8]) else {
            return;
        };
        let percent = payload[8];
        if percent > 100 {
            return;
        }
        lock(&shared.pending_batteries).insert(address, percent);
        (shared.on_update)();
        return;
    }

    if !same_guid(&record.EventHeader.ProviderId, &BTHPORT_PROVIDER_GUID)
        || record.EventHeader.EventDescriptor.Id != BTHPORT_ACL_TRANSFER_EVENT_ID
    {
        return;
    }
    let Some(address_property) = read_event_property(record, "BtAddress") else {
        return;
    };
    let Some(address) = read_bluetooth_address(&address_property) else {
        return;
    };
    for property_name in ["BufferStart", "BufferEnd"] {
        let Some(packet_bytes) = read_event_property(record, property_name) else {
            continue;
        };
        let Some(percent) = parse_packet_for_battery(shared, address, &packet_bytes) else {
            continue;
        };
        lock(&shared.pending_batteries).insert(address, percent);
        (shared.on_update)();
    }
}

fn read_event_property(record: &EVENT_RECORD, name: &str) -> Option<Vec<u8>> {
    let property_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let descriptor = PROPERTY_DATA_DESCRIPTOR {
        PropertyName: property_name.as_ptr() as usize as u64,
        ArrayIndex: u32::MAX,
        Reserved: 0,
    };
    let mut size = 0u32;
    let status = unsafe { TdhGetPropertySize(record, 0, null(), 1, &descriptor, &mut size) };
    if status != 0 || size == 0 || size as usize > MAXIMUM_EVENT_PROPERTY_BYTES {
        return None;
    }

    let mut value = vec![0; size as usize];
    let status =
        unsafe { TdhGetProperty(record, 0, null(), 1, &descriptor, size, value.as_mut_ptr()) };
    (status == 0).then_some(value)
}

fn parse_packet_for_battery(shared: &HfpShared, address: u64, packet_bytes: &[u8]) -> Option<u8> {
    if packet_bytes.is_empty() {
        return None;
    }
    let mut recent_packets = lock(&shared.recent_packet_bytes);
    let recent = recent_packets.entry(address).or_default();
    if packet_bytes.len() >= MAXIMUM_RECENT_PACKET_BYTES {
        recent.clear();
        recent.extend_from_slice(&packet_bytes[packet_bytes.len() - MAXIMUM_RECENT_PACKET_BYTES..]);
    } else {
        recent.extend_from_slice(packet_bytes);
        if recent.len() > MAXIMUM_RECENT_PACKET_BYTES {
            let excess = recent.len() - MAXIMUM_RECENT_PACKET_BYTES;
            recent.drain(..excess);
        }
    }
    let percent = parse_hfp_battery_report(recent);
    if percent.is_some() {
        recent.clear();
    }
    percent
}

fn parse_hfp_battery_report(bytes: &[u8]) -> Option<u8> {
    for prefix in [b"AT+IPHONEACCEV=".as_slice(), b"+IPHONEACCEV=".as_slice()] {
        if let Some(start) = find_ascii_case_insensitive(bytes, prefix) {
            if let Some(percent) = parse_iphone_acce_event(&bytes[start + prefix.len()..]) {
                return Some(percent);
            }
        }
    }

    for prefix in [b"AT+BIEV=2,".as_slice(), b"+BIEV=2,".as_slice()] {
        if let Some(start) = find_ascii_case_insensitive(bytes, prefix) {
            if let Some(percent) = parse_decimal_value(&bytes[start + prefix.len()..]) {
                return Some(percent);
            }
        }
    }

    for prefix in [
        b"AT+XEVENT=BATTERY".as_slice(),
        b"+XEVENT=BATTERY".as_slice(),
        b"+XEVENT:BATTERY".as_slice(),
    ] {
        if let Some(start) = find_ascii_case_insensitive(bytes, prefix) {
            if let Some(percent) = parse_decimal_value(&bytes[start + prefix.len()..]) {
                return Some(percent);
            }
        }
    }
    None
}

fn parse_iphone_acce_event(bytes: &[u8]) -> Option<u8> {
    let mut cursor = 0usize;
    let pair_count = parse_decimal_field(bytes, &mut cursor)??;
    if pair_count == 0 || pair_count > 16 {
        return None;
    }
    for _ in 0..pair_count {
        let key = parse_decimal_field(bytes, &mut cursor)??;
        let value = parse_decimal_field(bytes, &mut cursor)??;
        if key == 1 && value <= 100 {
            return Some(value as u8);
        }
    }
    None
}

fn parse_decimal_value(bytes: &[u8]) -> Option<u8> {
    let mut cursor = 0usize;
    while matches!(bytes.get(cursor), Some(b'=' | b':' | b',' | b' ' | b'\t')) {
        cursor += 1;
    }
    let value = parse_decimal_field(bytes, &mut cursor)??;
    (value <= 100).then_some(value as u8)
}

fn parse_decimal_field(bytes: &[u8], cursor: &mut usize) -> Option<Option<u32>> {
    while matches!(bytes.get(*cursor), Some(b' ' | b'\t')) {
        *cursor += 1;
    }
    let start = *cursor;
    let mut value = 0u32;
    while let Some(byte @ b'0'..=b'9') = bytes.get(*cursor) {
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(*byte - b'0'))?;
        *cursor += 1;
    }
    if *cursor == start {
        return None;
    }
    match bytes.get(*cursor) {
        Some(b',') => {
            *cursor += 1;
            Some(Some(value))
        }
        Some(b'\r' | b'\n' | b' ' | b'\t' | 0) => Some(Some(value)),
        Some(_) => None,
        None => Some(None),
    }
}

fn find_ascii_case_insensitive(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn same_guid(left: &GUID, right: &GUID) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}

fn trace_error(operation: &str, code: u32) -> String {
    if code == ERROR_ACCESS_DENIED {
        "Windows denied access to Hands-Free battery reports. Run the monitor as administrator or use an account in Performance Log Users.".to_owned()
    } else {
        format!("{operation} failed with Windows error {code}")
    }
}

fn read_bluetooth_address(bytes: &[u8]) -> Option<u64> {
    let address = u64::from_le_bytes(bytes.try_into().ok()?) & BLUETOOTH_ADDRESS_MASK;
    (address != 0).then_some(address)
}

fn bluetooth_device_names() -> HashMap<u64, String> {
    let search = BLUETOOTH_DEVICE_SEARCH_PARAMS {
        dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
        fReturnAuthenticated: 1,
        fReturnRemembered: 1,
        fReturnUnknown: 0,
        fReturnConnected: 1,
        fIssueInquiry: 0,
        cTimeoutMultiplier: 0,
        hRadio: null_mut(),
    };
    let mut info = BLUETOOTH_DEVICE_INFO {
        dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
        ..Default::default()
    };
    let search_handle = unsafe { BluetoothFindFirstDevice(&search, &mut info) };
    if search_handle.is_null() {
        return HashMap::new();
    }
    let _search_guard = BluetoothDeviceSearch(search_handle);
    let mut names = HashMap::new();
    loop {
        let address = unsafe { info.Address.Anonymous.ullLong } & BLUETOOTH_ADDRESS_MASK;
        let end = info
            .szName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(info.szName.len());
        let name = String::from_utf16_lossy(&info.szName[..end]);
        if address != 0 && !name.trim().is_empty() {
            names.insert(address, name);
        }
        info = BLUETOOTH_DEVICE_INFO {
            dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        if unsafe { BluetoothFindNextDevice(search_handle, &mut info) } == 0 {
            break;
        }
    }
    names
}

struct BluetoothDeviceSearch(HBLUETOOTH_DEVICE_FIND);

impl Drop for BluetoothDeviceSearch {
    fn drop(&mut self) {
        unsafe { BluetoothFindDeviceClose(self.0) };
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
