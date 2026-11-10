use crate::probe::ProbeResult;
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

const LOGITECH_VENDOR: u16 = 0x046d;
const HIDPP_USAGE_PAGE: u16 = 0xff00;
const HIDPP_LONG_USAGE: u16 = 0x0002;
const X3_RECEIVER: u16 = 0xc54f;
const X3_USAGE_PAGE: u16 = 0xff43;
const X3_LONG_USAGE: u16 = 0x0302;
const SUPERLIGHT_WIRED: u16 = 0xc09f;
const SUPERLIGHT_RECEIVER: u16 = 0xc54d;
const BATTERY_STATUS_FEATURE: u16 = 0x1000;
const UNIFIED_BATTERY_FEATURE: u16 = 0x1004;
const DEVICE_NAME_FEATURE: u16 = 0x0005;
const SOFTWARE_ID: u8 = 0x08;
const LOGITECH_TIMEOUT: u64 = 650;
const LOGITECH_SOURCE: &str = "Logitech LIGHTSPEED";
const LOGITECH_FALLBACK_NAME: &str = "Logitech wireless device";
const SLEEPING: &str = "Sleeping or unavailable";
const DEVICE_PRIORITY: i32 = 30;

const HP_VENDOR: u16 = 0x03f0;
const CLOUD_III_S: u16 = 0x02cc;
const CLOUD_III_S_ALTERNATE: u16 = 0x06be;
const HYPERX_USAGE_PAGE: u16 = 0x01c0;
const HYPERX_USAGE: u16 = 0x0001;
const HYPERX_TIMEOUT: u64 = 1500;

pub trait Exchange {
    fn exchange(
        &mut self,
        request: &[u8],
        matches: &dyn Fn(&[u8]) -> bool,
        timeout_ms: u64,
        feature_fallback: bool,
    ) -> Result<Option<Vec<u8>>, String>;
}

struct HidExchange<'a>(&'a HidDevice);

impl Exchange for HidExchange<'_> {
    fn exchange(
        &mut self,
        request: &[u8],
        matches: &dyn Fn(&[u8]) -> bool,
        timeout_ms: u64,
        feature_fallback: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        if let Err(error) = self.0.write(request) {
            if !feature_fallback {
                return Err(format!("Could not write HID output report: {error}"));
            }
            self.0
                .send_feature_report(request)
                .map_err(|error| format!("Could not write HID feature report: {error}"))?;
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1));
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let mut reply = [0u8; 64];
            let length = self
                .0
                .read_timeout(&mut reply, remaining.as_millis().clamp(1, i32::MAX as u128) as i32)
                .map_err(|error| format!("Could not read HID report: {error}"))?;
            if length == 0 {
                return Ok(None);
            }
            if matches(&reply[..length]) {
                return Ok(Some(reply[..length].to_vec()));
            }
        }
    }
}

struct Interface {
    path: String,
    product_id: u16,
    x3_receiver: bool,
}

pub fn collect_logitech(errors: &mut Vec<String>) -> Result<Vec<ProbeResult>, String> {
    let api = HidApi::new().map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() != LOGITECH_VENDOR {
            continue;
        }
        let x3_receiver = info.product_id() == X3_RECEIVER
            && info.usage_page() == X3_USAGE_PAGE
            && info.usage() == X3_LONG_USAGE;
        let hidpp = info.usage_page() == HIDPP_USAGE_PAGE && info.usage() == HIDPP_LONG_USAGE;
        if !x3_receiver && !hidpp {
            continue;
        }
        let interface = Interface {
            path: info.path().to_string_lossy().into_owned(),
            product_id: info.product_id(),
            x3_receiver,
        };
        let outcome = info
            .open_device(&api)
            .map_err(|_| "Could not open HID interface.".to_owned())
            .and_then(|device| read_logitech_devices(&interface, &mut HidExchange(&device), &mut results));
        if let Err(message) = outcome {
            errors.push(format!("Logitech device: {message}"));
        }
    }
    Ok(results)
}

fn read_logitech_devices(
    interface: &Interface,
    exchange: &mut dyn Exchange,
    results: &mut Vec<ProbeResult>,
) -> Result<(), String> {
    let last_index = if interface.product_id == SUPERLIGHT_WIRED { 1 } else { 6 };
    let mut any_response = false;
    for index in 1..=last_index {
        let (result, responded) = if interface.x3_receiver {
            read_x3_device(interface, exchange, index)?
        } else {
            read_device(interface, exchange, index)?
        };
        if !responded {
            if interface.x3_receiver {
                continue;
            }
            break;
        }
        any_response = true;
        results.extend(result);
    }
    if !any_response && !interface.x3_receiver {
        results.push(ProbeResult {
            id: logitech_id(interface, 1),
            name: LOGITECH_FALLBACK_NAME.to_owned(),
            source: LOGITECH_SOURCE.to_owned(),
            status: SLEEPING.to_owned(),
            priority: DEVICE_PRIORITY,
            ..Default::default()
        });
    }
    Ok(())
}

fn logitech_result(interface: &Interface, index: u8, name: Option<String>, battery: Option<(u8, u16, u8)>) -> ProbeResult {
    ProbeResult {
        id: logitech_id(interface, index),
        physical_id: None,
        name: name.filter(|name| !name.trim().is_empty()).unwrap_or_else(|| LOGITECH_FALLBACK_NAME.to_owned()),
        percent: battery.map(|(percent, _, _)| percent),
        source: LOGITECH_SOURCE.to_owned(),
        status: match battery {
            Some((_, feature, status)) => logitech_status(feature, status).to_owned(),
            None => SLEEPING.to_owned(),
        },
        priority: DEVICE_PRIORITY,
    }
}

fn read_x3_device(
    interface: &Interface,
    exchange: &mut dyn Exchange,
    index: u8,
) -> Result<(Option<ProbeResult>, bool), String> {
    let root = request(exchange, index, 0, 0, &UNIFIED_BATTERY_FEATURE.to_be_bytes())?;
    let Some(root) = root.filter(|root| root.len() >= 7 && !is_protocol_error(root)) else {
        return Ok((None, false));
    };
    let mut responded = true;
    let mut battery = None;
    if root[4] != 0 {
        battery = read_battery_by_index(exchange, index, root[4], 0x10)?
            .map(|(percent, status)| (percent, UNIFIED_BATTERY_FEATURE, status));
    }
    if battery.is_none() {
        battery = read_battery(exchange, index, BATTERY_STATUS_FEATURE, 0x00, &mut responded)?
            .map(|(percent, status)| (percent, BATTERY_STATUS_FEATURE, status));
    }
    let name = match read_device_name(exchange, index, &mut responded) {
        Ok(name) => name,
        Err(error) if battery.is_none() => return Err(error),
        Err(_) => None,
    };
    Ok((Some(logitech_result(interface, index, name, battery)), true))
}

fn read_device(
    interface: &Interface,
    exchange: &mut dyn Exchange,
    index: u8,
) -> Result<(Option<ProbeResult>, bool), String> {
    let mut responded = false;
    let mut battery = read_battery(exchange, index, BATTERY_STATUS_FEATURE, 0x00, &mut responded)?
        .map(|(percent, status)| (percent, BATTERY_STATUS_FEATURE, status));
    if !responded {
        return Ok((None, false));
    }
    if battery.is_none() {
        battery = read_battery(exchange, index, UNIFIED_BATTERY_FEATURE, 0x10, &mut responded)?
            .map(|(percent, status)| (percent, UNIFIED_BATTERY_FEATURE, status));
    }
    let name = read_device_name(exchange, index, &mut responded)?;
    let mut result = logitech_result(interface, index, name, battery);
    if battery.is_some() && matches!(interface.product_id, SUPERLIGHT_WIRED | SUPERLIGHT_RECEIVER) {
        result.physical_id = read_physical_id(exchange, index).ok().flatten();
    }
    Ok((Some(result), true))
}

fn read_physical_id(exchange: &mut dyn Exchange, index: u8) -> Result<Option<String>, String> {
    let Some(root) = request(exchange, index, 0, 0, &[0x00, 0x03])? else { return Ok(None) };
    if root.len() < 7 || is_protocol_error(&root) || root[4] == 0 || root[6] == 0 {
        return Ok(None);
    }
    let Some(info) = request(exchange, index, root[4], 0, &[])? else { return Ok(None) };
    if info.len() < 17 || is_protocol_error(&info) {
        return Ok(None);
    }
    let unit = &info[5..9];
    let model = &info[11..17];
    if unit.iter().all(|byte| *byte == 0) || model.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    let hex = |bytes: &[u8]| bytes.iter().map(|byte| format!("{byte:02X}")).collect::<String>();
    Ok(Some(format!("logitech-unit:{}{}", hex(model), hex(unit))))
}

fn read_battery(
    exchange: &mut dyn Exchange,
    index: u8,
    feature: u16,
    function: u8,
    responded: &mut bool,
) -> Result<Option<(u8, u8)>, String> {
    let Some(feature_index) = feature_index(exchange, index, feature, responded)? else { return Ok(None) };
    let battery = read_battery_by_index(exchange, index, feature_index, function)?;
    if battery.is_some() {
        *responded = true;
    }
    Ok(battery)
}

fn read_battery_by_index(
    exchange: &mut dyn Exchange,
    index: u8,
    feature_index: u8,
    function: u8,
) -> Result<Option<(u8, u8)>, String> {
    let Some(response) = request(exchange, index, feature_index, function, &[])? else { return Ok(None) };
    if response.len() < 7 || is_protocol_error(&response) || response[4] > 100 {
        return Ok(None);
    }
    Ok(Some((response[4], response[6])))
}

fn read_device_name(exchange: &mut dyn Exchange, index: u8, responded: &mut bool) -> Result<Option<String>, String> {
    let Some(feature_index) = feature_index(exchange, index, DEVICE_NAME_FEATURE, responded)? else {
        return Ok(None);
    };
    let Some(count) = request(exchange, index, feature_index, 0x00, &[])? else { return Ok(None) };
    if count.len() < 5 || is_protocol_error(&count) {
        return Ok(None);
    }
    *responded = true;
    let length = usize::from(count[4]).min(64);
    if length == 0 {
        return Ok(None);
    }
    let mut name = Vec::new();
    for offset in (0..length).step_by(16) {
        let Some(fragment) = request(exchange, index, feature_index, 0x10, &[offset as u8])? else { break };
        if fragment.len() <= 4 || is_protocol_error(&fragment) {
            break;
        }
        let available = (length - offset).min(fragment.len() - 4);
        name.extend_from_slice(&fragment[4..4 + available]);
    }
    Ok(Some(String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_owned()))
}

fn feature_index(
    exchange: &mut dyn Exchange,
    index: u8,
    feature: u16,
    responded: &mut bool,
) -> Result<Option<u8>, String> {
    let Some(response) = request(exchange, index, 0, 0, &feature.to_be_bytes())? else { return Ok(None) };
    *responded = true;
    if response.len() < 5 || is_protocol_error(&response) || response[4] == 0 {
        return Ok(None);
    }
    Ok(Some(response[4]))
}

fn request(
    exchange: &mut dyn Exchange,
    index: u8,
    feature_index: u8,
    function: u8,
    parameters: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let command = function | SOFTWARE_ID;
    let mut request = [0u8; 20];
    request[..4].copy_from_slice(&[0x11, index, feature_index, command]);
    let count = parameters.len().min(16);
    request[4..4 + count].copy_from_slice(&parameters[..count]);
    exchange.exchange(
        &request,
        &|response: &[u8]| {
            response.len() >= 7
                && response[1] == index
                && ((response[2] == feature_index && response[3] == command)
                    || (response[2] == 0xff && response[3] == feature_index && response[4] == command))
        },
        LOGITECH_TIMEOUT,
        false,
    )
}

fn is_protocol_error(response: &[u8]) -> bool {
    response.len() >= 5 && response[2] == 0xff
}

fn logitech_id(interface: &Interface, index: u8) -> String {
    format!("logitech:{}:{index}", interface.path)
}

pub fn is_obsolete_superlight_wired_slot(id: &str) -> bool {
    let lower = id.to_lowercase();
    if !lower.starts_with("logitech:") || !lower.contains("vid_046d&pid_c09f") {
        return false;
    }
    lower
        .rsplit_once(':')
        .and_then(|(_, slot)| slot.parse::<i32>().ok())
        .is_some_and(|slot| (2..=6).contains(&slot))
}

fn logitech_status(feature: u16, status: u8) -> &'static str {
    match (feature, status) {
        (_, 0) => "Discharging",
        (_, 1) => "Charging",
        (UNIFIED_BATTERY_FEATURE, 2) => "Charging slowly",
        (UNIFIED_BATTERY_FEATURE, 3) => "Fully charged",
        (UNIFIED_BATTERY_FEATURE, 4) => "Charging error",
        (UNIFIED_BATTERY_FEATURE, _) => "Connected",
        (_, 2) => "Nearly charged",
        (_, 3) => "Fully charged",
        (_, 4) => "Charging slowly",
        (_, 5) => "Battery error",
        (_, 6) => "Temperature warning",
        (_, 7) => "Charging error",
        _ => "Connected",
    }
}

pub fn collect_hyperx(errors: &mut Vec<String>) -> Result<Vec<ProbeResult>, String> {
    let api = HidApi::new().map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() != HP_VENDOR
            || !matches!(info.product_id(), CLOUD_III_S | CLOUD_III_S_ALTERNATE)
            || info.usage_page() != HYPERX_USAGE_PAGE
            || info.usage() != HYPERX_USAGE
        {
            continue;
        }
        let mut result = ProbeResult {
            id: format!("hyperx:{}", info.path().to_string_lossy()),
            name: info
                .product_string()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("HyperX Cloud III S Wireless")
                .to_owned(),
            source: "HyperX 2.4 GHz".to_owned(),
            priority: DEVICE_PRIORITY,
            ..Default::default()
        };
        let outcome = info
            .open_device(&api)
            .map_err(|_| "Could not open HID interface.".to_owned())
            .and_then(|device| read_hyperx(&mut HidExchange(&device), &mut result));
        if let Err(message) = outcome {
            errors.push(format!("HyperX device: {message}"));
            result.status = if result.percent.is_some() { "Connected" } else { "Headset unavailable" }.to_owned();
        }
        results.push(result);
    }
    Ok(results)
}

fn read_hyperx(exchange: &mut dyn Exchange, result: &mut ProbeResult) -> Result<(), String> {
    let level = hyperx_request(exchange, 0x06)?.filter(|response| response.len() > 6).map(|response| response[6]);
    match level {
        None => result.status = "Headset unavailable".to_owned(),
        Some(0xff) => result.status = "Headset off or unavailable".to_owned(),
        Some(percent) if percent <= 100 => {
            result.percent = Some(percent);
            result.status = match hyperx_request(exchange, 0x48) {
                Ok(Some(response)) if response.len() > 6 => match response[6] {
                    0 => "Not charging",
                    1 => "Charging",
                    2 => "Fully charged",
                    _ => "Connected",
                },
                _ => "Connected",
            }
            .to_owned();
        }
        Some(_) => result.status = "Unexpected battery response".to_owned(),
    }
    Ok(())
}

fn hyperx_request(exchange: &mut dyn Exchange, command: u8) -> Result<Option<Vec<u8>>, String> {
    let mut request = [0u8; 64];
    request[..4].copy_from_slice(&[0x0c, 0x02, 0x03, 0x01]);
    request[5] = command;
    exchange.exchange(
        &request,
        &|response: &[u8]| response.len() > 6 && response[0] == 0x0c && response[5] == command,
        HYPERX_TIMEOUT,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    type Reply = Result<Vec<u8>, String>;

    struct Scripted {
        replies: Vec<([u8; 3], Vec<u8>, Reply)>,
        requests: Vec<Vec<u8>>,
    }

    impl Scripted {
        fn new() -> Self {
            Self { replies: Vec::new(), requests: Vec::new() }
        }

        fn on(mut self, index: u8, feature_index: u8, function: u8, parameters: &[u8], payload: &[u8]) -> Self {
            self.replies.push(([index, feature_index, function], parameters.to_vec(), Ok(payload.to_vec())));
            self
        }

        fn fail(mut self, index: u8, feature_index: u8, function: u8, parameters: &[u8]) -> Self {
            self.replies.push(([index, feature_index, function], parameters.to_vec(), Err("device removed".to_owned())));
            self
        }
    }

    impl Exchange for Scripted {
        fn exchange(
            &mut self,
            request: &[u8],
            matches: &dyn Fn(&[u8]) -> bool,
            _timeout_ms: u64,
            _feature_fallback: bool,
        ) -> Result<Option<Vec<u8>>, String> {
            self.requests.push(request.to_vec());
            let key = [request[1], request[2], request[3] & 0xf0];
            let found = self
                .replies
                .iter()
                .find(|(expected, parameters, _)| *expected == key && request[4..4 + parameters.len()] == parameters[..]);
            let Some((_, _, reply)) = found else { return Ok(None) };
            let payload = reply.clone()?;
            let mut response = vec![0x11, request[1], request[2], request[3]];
            response.extend_from_slice(&payload);
            response.resize(20, 0);
            assert!(matches(&response));
            Ok(Some(response))
        }
    }

    fn interface(product_id: u16, x3_receiver: bool) -> Interface {
        Interface { path: "\\\\?\\hid#vid_046d&pid_test".to_owned(), product_id, x3_receiver }
    }

    fn name_replies(script: Scripted, index: u8) -> Scripted {
        script
            .on(index, 0, 0x00, &[0x00, 0x05], &[3])
            .on(index, 3, 0x00, &[], &[5])
            .on(index, 3, 0x10, &[0], b"Mouse")
    }

    #[test]
    fn a_wired_superlight_is_read_once_with_its_physical_identity() {
        let mut script = name_replies(Scripted::new(), 1)
            .on(1, 0, 0x00, &[0x10, 0x00], &[2])
            .on(1, 2, 0x00, &[], &[81, 0, 1])
            .on(1, 0, 0x00, &[0x00, 0x03], &[4, 0, 1])
            .on(1, 4, 0x00, &[], &[0, 0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 1, 2, 3, 4, 5, 6]);
        for index in 2..=6 {
            script = name_replies(script, index)
                .on(index, 0, 0x00, &[0x10, 0x00], &[2])
                .on(index, 2, 0x00, &[], &[81, 0, 1]);
        }
        let mut results = Vec::new();
        read_logitech_devices(&interface(SUPERLIGHT_WIRED, false), &mut script, &mut results).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "logitech:\\\\?\\hid#vid_046d&pid_test:1");
        assert_eq!(results[0].name, "Mouse");
        assert_eq!(results[0].percent, Some(81));
        assert_eq!(results[0].status, "Charging");
        assert_eq!(results[0].physical_id.as_deref(), Some("logitech-unit:010203040506AABBCCDD"));
        assert!(script.requests.iter().all(|request| request[1] == 1 && request[3] & 0x0f == SOFTWARE_ID));
    }

    #[test]
    fn a_silent_receiver_reports_one_sleeping_device() {
        let mut script = Scripted::new();
        let mut results = Vec::new();
        read_logitech_devices(&interface(0xc547, false), &mut script, &mut results).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].percent, None);
        assert_eq!(results[0].status, SLEEPING);
        assert_eq!(results[0].name, LOGITECH_FALLBACK_NAME);
        assert_eq!(script.requests.len(), 1);
    }

    #[test]
    fn the_unified_battery_feature_is_the_fallback_on_older_receivers() {
        let mut script = name_replies(Scripted::new(), 1)
            .on(1, 0, 0x00, &[0x10, 0x00], &[0])
            .on(1, 0, 0x00, &[0x10, 0x04], &[6])
            .on(1, 6, 0x10, &[], &[55, 0, 2]);
        let mut results = Vec::new();
        read_logitech_devices(&interface(0xc547, false), &mut script, &mut results).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!((results[0].percent, results[0].status.as_str()), (Some(55), "Charging slowly"));
        assert_eq!(results[0].physical_id, None);
    }

    #[test]
    fn an_x3_receiver_skips_empty_slots_and_keeps_a_battery_without_a_name() {
        let mut script = Scripted::new()
            .on(3, 0, 0x00, &[0x10, 0x04], &[7])
            .on(3, 7, 0x10, &[], &[97, 0, 0])
            .fail(3, 0, 0x00, &[0x00, 0x05]);
        let mut results = Vec::new();
        read_logitech_devices(&interface(X3_RECEIVER, true), &mut script, &mut results).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].id.ends_with(":3"));
        assert_eq!(results[0].name, LOGITECH_FALLBACK_NAME);
        assert_eq!((results[0].percent, results[0].status.as_str()), (Some(97), "Discharging"));
    }

    #[test]
    fn a_failing_interface_keeps_earlier_devices_and_reports_the_error() {
        let mut script = name_replies(Scripted::new(), 1)
            .on(1, 0, 0x00, &[0x10, 0x00], &[2])
            .on(1, 2, 0x00, &[], &[40, 0, 0])
            .fail(2, 0, 0x00, &[0x10, 0x00]);
        let mut results = Vec::new();
        let outcome = read_logitech_devices(&interface(0xc547, false), &mut script, &mut results);
        assert_eq!(outcome, Err("device removed".to_owned()));
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn obsolete_wired_slots_and_status_codes_are_recognised() {
        assert!(is_obsolete_superlight_wired_slot("logitech:\\\\?\\HID#VID_046D&PID_C09F&MI_02#x:4"));
        assert!(!is_obsolete_superlight_wired_slot("logitech:\\\\?\\hid#vid_046d&pid_c09f&mi_02#x:1"));
        assert!(!is_obsolete_superlight_wired_slot("logitech:\\\\?\\hid#vid_046d&pid_c54d#x:4"));
        assert_eq!(logitech_status(BATTERY_STATUS_FEATURE, 2), "Nearly charged");
        assert_eq!(logitech_status(UNIFIED_BATTERY_FEATURE, 7), "Connected");
        assert_eq!(logitech_status(BATTERY_STATUS_FEATURE, 7), "Charging error");
    }

    #[test]
    fn hyperx_levels_map_to_the_original_statuses() {
        struct Headset(u8, Option<u8>);
        impl Exchange for Headset {
            fn exchange(
                &mut self,
                request: &[u8],
                _matches: &dyn Fn(&[u8]) -> bool,
                _timeout_ms: u64,
                feature_fallback: bool,
            ) -> Result<Option<Vec<u8>>, String> {
                assert!(feature_fallback);
                let value = if request[5] == 0x06 { Some(self.0) } else { self.1 };
                Ok(value.map(|value| vec![0x0c, 0, 0, 0, 0, request[5], value]))
            }
        }
        let read = |level: u8, charging: Option<u8>| {
            let mut result = ProbeResult::default();
            read_hyperx(&mut Headset(level, charging), &mut result).unwrap();
            (result.percent, result.status)
        };
        assert_eq!(read(53, Some(0)), (Some(53), "Not charging".to_owned()));
        assert_eq!(read(100, Some(2)), (Some(100), "Fully charged".to_owned()));
        assert_eq!(read(40, None), (Some(40), "Connected".to_owned()));
        assert_eq!(read(0xff, Some(0)), (None, "Headset off or unavailable".to_owned()));
        assert_eq!(read(150, Some(0)), (None, "Unexpected battery response".to_owned()));
    }

    #[test]
    #[ignore = "queries the HID devices connected to this computer"]
    fn reads_connected_hardware() {
        let started = Instant::now();
        let mut errors = Vec::new();
        let mut devices = collect_logitech(&mut errors).unwrap();
        devices.extend(collect_hyperx(&mut errors).unwrap());
        println!("collect took {} ms", started.elapsed().as_millis());
        for device in &devices {
            println!(
                "{} | {} | {:?} | {} | physical={:?}",
                device.name, device.source, device.percent, device.status, device.physical_id
            );
        }
        for error in &errors {
            println!("error: {error}");
        }
        assert!(devices.iter().all(|device| device.percent.is_none_or(|percent| percent <= 100)));
    }
}
