use crate::devices;
use crate::probe::ProbeResult;
use crate::windows_collectors;

pub fn collect_all() -> (Vec<ProbeResult>, Vec<String>) {
    let mut collected = Vec::new();
    let mut errors = Vec::new();
    let mut add = |label: &str, outcome: Result<Vec<ProbeResult>, String>, errors: &mut Vec<String>| match outcome {
        Ok(results) => collected.extend(results),
        Err(message) => errors.push(format!("{label}: {message}")),
    };

    add("Windows", windows_collectors::collect_battery_properties(), &mut errors);
    let mut device_errors = Vec::new();
    let gatt = windows_collectors::collect_gatt_batteries(|message| {
        device_errors.push(format!("Bluetooth LE device: {message}"));
    });
    errors.append(&mut device_errors);
    add("Bluetooth LE", gatt, &mut errors);
    let logitech = devices::collect_logitech(&mut errors);
    add("Logitech", logitech, &mut errors);
    let hyperx = devices::collect_hyperx(&mut errors);
    add("HyperX", hyperx, &mut errors);
    (collected, errors)
}
