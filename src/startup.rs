use crate::win;
use std::path::Path;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "BluetoothBatteryMonitor";

fn targets(command: &str, executable: &Path) -> bool {
    let Some(target) = win::first_argument(command) else { return false };
    let target = Path::new(&target);
    target.is_absolute()
        && target.to_string_lossy().to_lowercase() == executable.to_string_lossy().to_lowercase()
}

pub fn is_enabled() -> bool {
    let Ok(executable) = std::env::current_exe() else { return false };
    win::registry_string(RUN_KEY, VALUE_NAME).is_some_and(|command| targets(&command, &executable))
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if !enabled {
        return win::delete_registry_value(RUN_KEY, VALUE_NAME);
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    win::set_registry_string(RUN_KEY, VALUE_NAME, &format!("\"{}\"", executable.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_command_for_this_executable_counts_as_enabled() {
        let executable = Path::new("C:\\Apps\\Battery Monitor\\monitor.exe");
        assert!(targets("\"C:\\Apps\\Battery Monitor\\monitor.exe\"", executable));
        assert!(targets("\"c:\\apps\\battery monitor\\MONITOR.EXE\" --skip-startup-update", executable));
        assert!(!targets("\"C:\\Apps\\Other\\monitor.exe\"", executable));
        assert!(!targets("monitor.exe", executable));
        assert!(!targets("", executable));
    }
}
