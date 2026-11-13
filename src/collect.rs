use crate::devices;
use crate::hfp::HfpMonitor;
use crate::probe::ProbeResult;
use crate::windows_collectors;

pub fn collect_all(hfp_monitor: Option<&HfpMonitor>) -> (Vec<ProbeResult>, Vec<String>, Option<String>) {
    let mut collected = Vec::new();
    let mut errors = Vec::new();
    let mut add = |label: &str, outcome: Result<Vec<ProbeResult>, String>, errors: &mut Vec<String>| match outcome {
        Ok(results) => collected.extend(results),
        Err(message) => errors.push(format!("{label}: {message}")),
    };

    add("Windows", windows_collectors::collect_battery_properties(), &mut errors);
    let connected = windows_collectors::collect_connected_hfp_devices(&mut errors);
    let (mut reports, trace_error) = hfp_monitor.map(HfpMonitor::take_battery_probes).unwrap_or_default();
    if let Ok(devices) = &connected {
        reports.retain(|report| {
            !devices.iter().any(|device| device.id == report.id && device.percent.is_some())
        });
    }
    add("Bluetooth HFP", connected, &mut errors);
    add("Bluetooth HFP", Ok(reports), &mut errors);
    let mut device_errors = Vec::new();
    let gatt = windows_collectors::collect_gatt_batteries(|message| {
        device_errors.push(format!("Bluetooth LE device: {message}"));
    });
    errors.append(&mut device_errors);
    add("Bluetooth LE", gatt, &mut errors);
    let hid_batteries = devices::collect_hid_batteries();
    add("HID battery", hid_batteries, &mut errors);
    let logitech = devices::collect_logitech(&mut errors);
    add("Logitech", logitech, &mut errors);
    let hyperx = devices::collect_hyperx(&mut errors);
    add("HyperX", hyperx, &mut errors);
    (collected, errors, trace_error)
}
