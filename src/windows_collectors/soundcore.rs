use super::{BatteryLevelReading, hold_multithreaded_apartment, present};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::Devices::Bluetooth::Rfcomm::{RfcommDeviceService, RfcommServiceId};
use windows::Devices::Bluetooth::{
    BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice, BluetoothError,
};
use windows::Networking::Sockets::StreamSocket;
use windows::Storage::Streams::{DataReader, DataWriter, InputStreamOptions};
use windows::Win32::Foundation::{E_FAIL, ERROR_TIMEOUT};
use windows::core::{Error, HRESULT, Result};

pub(super) const MODEL_NAME: &str = "Soundcore Life Dot 2 NC";
pub(super) const SOURCE: &str = "Soundcore RFCOMM";

// A3931 RequestState and reply framing, documented in OpenSCQ30 and verified on the earbuds.
const REQUEST_STATE: [u8; 10] = [0x08, 0xee, 0, 0, 0, 0x01, 0x01, 10, 0, 0x02];
const REPLY_PREFIX: [u8; 5] = [0x09, 0xff, 0, 0, 0x01];
const STATE_COMMAND: [u8; 2] = [0x01, 0x01];
const HEADER_LENGTH: usize = 9;
const MAX_PACKET_LENGTH: usize = 512;
const MAX_REPLIES: usize = 4;
const MAX_BATTERY_LEVEL: u8 = 5;
const QUERY_TIMEOUT: Duration = Duration::from_secs(4);

macro_rules! wait_for_bluetooth {
    ($operation:expr, $deadline:expr) => {{
        let operation = $operation?;
        let (send, receive) = mpsc::channel();
        operation.when(move |result| {
            let _ = send.send(result);
        })?;
        match receive.recv_timeout($deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(_) => {
                let _ = operation.Cancel();
                Err(Error::new(
                    HRESULT::from_win32(ERROR_TIMEOUT.0),
                    "Soundcore battery request timed out",
                ))
            }
        }
    }};
}

pub(super) fn read_battery(address: u64) -> Result<Option<BatteryLevelReading>> {
    hold_multithreaded_apartment()?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let Some(device) = present(wait_for_bluetooth!(
        BluetoothDevice::FromBluetoothAddressAsync(address),
        deadline
    ))? else {
        return Ok(None);
    };
    let outcome = read_from_device(&device, deadline);
    let _ = device.Close();
    outcome
}

fn read_from_device(device: &BluetoothDevice, deadline: Instant) -> Result<Option<BatteryLevelReading>> {
    if device.ConnectionStatus()? != BluetoothConnectionStatus::Connected {
        return Ok(None);
    }
    let service_id = RfcommServiceId::SerialPort()?;
    let result = wait_for_bluetooth!(
        device.GetRfcommServicesForIdWithCacheModeAsync(&service_id, BluetoothCacheMode::Cached),
        deadline
    )?;
    let services = result.Services()?;
    if result.Error()? != BluetoothError::Success || services.Size()? == 0 {
        return Err(Error::new(E_FAIL, "Soundcore serial service is unavailable"));
    }
    let service = services.GetAt(0)?;
    let outcome = read_from_service(&service, deadline);
    let _ = service.Close();
    outcome
}

fn read_from_service(service: &RfcommDeviceService, deadline: Instant) -> Result<Option<BatteryLevelReading>> {
    let socket = StreamSocket::new()?;
    let outcome = (|| {
        let host = service.ConnectionHostName()?;
        let name = service.ConnectionServiceName()?;
        let protection = service.ProtectionLevel()?;
        wait_for_bluetooth!(socket.ConnectWithProtectionLevelAsync(&host, &name, protection), deadline)?;
        request_state(&socket, deadline)
    })();
    let _ = socket.Close();
    outcome
}

fn request_state(socket: &StreamSocket, deadline: Instant) -> Result<Option<BatteryLevelReading>> {
    let writer = DataWriter::CreateDataWriter(&socket.OutputStream()?)?;
    writer.WriteBytes(&REQUEST_STATE)?;
    let sent = wait_for_bluetooth!(writer.StoreAsync(), deadline)?;
    if sent as usize != REQUEST_STATE.len() {
        return Err(Error::new(E_FAIL, "Could not send the Soundcore battery request"));
    }

    let reader = DataReader::CreateDataReader(&socket.InputStream()?)?;
    reader.SetInputStreamOptions(InputStreamOptions::Partial)?;
    let mut packet = [0; MAX_PACKET_LENGTH];
    for _ in 0..MAX_REPLIES {
        read_bytes(&reader, &mut packet[..HEADER_LENGTH], deadline)?;
        let length = usize::from(u16::from_le_bytes([packet[7], packet[8]]));
        if !packet.starts_with(&REPLY_PREFIX) || !(HEADER_LENGTH + 1..=packet.len()).contains(&length) {
            return Err(Error::new(E_FAIL, "Invalid Soundcore reply header"));
        }
        read_bytes(&reader, &mut packet[HEADER_LENGTH..length], deadline)?;
        let (body, checksum) = packet[..length].split_at(length - 1);
        if body.iter().copied().fold(0u8, u8::wrapping_add) != checksum[0] {
            return Err(Error::new(E_FAIL, "Invalid Soundcore reply checksum"));
        }
        let payload = &body[HEADER_LENGTH..];
        if packet[5..7] == STATE_COMMAND && payload.len() >= 6 {
            return Ok(parse_battery(payload));
        }
    }
    Err(Error::new(E_FAIL, "Soundcore did not return a battery state"))
}

fn read_bytes(reader: &DataReader, mut buffer: &mut [u8], deadline: Instant) -> Result<()> {
    while !buffer.is_empty() {
        let loaded = wait_for_bluetooth!(reader.LoadAsync(buffer.len() as u32), deadline)? as usize;
        if loaded == 0 || loaded > buffer.len() {
            return Err(Error::new(E_FAIL, "Incomplete Soundcore reply"));
        }
        let (loaded_bytes, remaining) = buffer.split_at_mut(loaded);
        reader.ReadBytes(loaded_bytes)?;
        buffer = remaining;
    }
    Ok(())
}

fn parse_battery(payload: &[u8]) -> Option<BatteryLevelReading> {
    let [_, _, left, right, left_charging, right_charging, ..] = payload else {
        return None;
    };
    // A missing earbud reports 0xff. Use the lower level for the shared device row.
    let (level, charging) = [(*left, *left_charging == 1), (*right, *right_charging == 1)]
        .into_iter()
        .filter(|(level, _)| *level <= MAX_BATTERY_LEVEL)
        .min()?;
    Some(BatteryLevelReading {
        percent: level * (100 / MAX_BATTERY_LEVEL),
        status: if charging { "Charging" } else { "Not charging" },
    })
}
