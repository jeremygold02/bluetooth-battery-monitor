use std::path::Path;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, FILETIME, GetLastError, LocalFree, POINT, SYSTEMTIME,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject};
use windows_sys::Win32::Globalization::{
    GetLocaleInfoEx, GetTimeFormatEx, LOCALE_SDECIMAL, TIME_NOSECONDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, GetSysColor, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    SYS_COLOR_INDEX,
};
use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Shell::{CommandLineToArgvW, ShellExecuteW};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, LoadImageW,
    SM_CXICON, SM_CXSMICON, SPI_GETHIGHCONTRAST, SW_SHOWNORMAL, SendMessageW, SetForegroundWindow,
    SystemParametersInfoW, WM_SETICON,
};

const APP_ICON_RESOURCE: *const u16 = 1 as *const u16;
const FALLBACK_WORK_AREA: PixelRect = PixelRect { left: 0, top: 0, right: 1920, bottom: 1040 };
const UNIX_EPOCH_AS_FILETIME_MS: i64 = 11_644_473_600_000;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const THEME_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelPoint {
    pub x: i32,
    pub y: i32,
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

pub fn enable_dpi_awareness() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

pub fn allow_foreground(process: u32) {
    unsafe { AllowSetForegroundWindow(process) };
}

pub fn set_foreground(window: isize) {
    unsafe { SetForegroundWindow(window as _) };
}

pub fn work_area(anchor: Option<PixelPoint>) -> PixelRect {
    let (point, flags) = match anchor {
        Some(point) => (POINT { x: point.x, y: point.y }, MONITOR_DEFAULTTONEAREST),
        None => (POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
    };
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    let found = unsafe {
        let monitor = MonitorFromPoint(point, flags);
        !monitor.is_null() && GetMonitorInfoW(monitor, &mut info) != 0
    };
    let work = info.rcWork;
    if found && work.right > work.left && work.bottom > work.top {
        PixelRect { left: work.left, top: work.top, right: work.right, bottom: work.bottom }
    } else {
        FALLBACK_WORK_AREA
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

fn local_time(unix_ms: i64) -> Option<SYSTEMTIME> {
    let ticks = unix_ms.checked_add(UNIX_EPOCH_AS_FILETIME_MS)?.checked_mul(10_000)?;
    let ticks = u64::try_from(ticks).ok()?;
    let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let converted = unsafe {
        FileTimeToSystemTime(&file_time, &mut utc) != 0
            && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) != 0
    };
    converted.then_some(local)
}

pub fn format_short_time(unix_ms: i64) -> String {
    let Some(time) = local_time(unix_ms) else { return String::new() };
    let mut buffer = [0u16; 80];
    let length = unsafe {
        GetTimeFormatEx(std::ptr::null(), TIME_NOSECONDS, &time, std::ptr::null(), buffer.as_mut_ptr(), buffer.len() as i32)
    };
    if length > 0 { from_wide(&buffer) } else { String::new() }
}

pub fn decimal_separator() -> String {
    let mut buffer = [0u16; 8];
    let length =
        unsafe { GetLocaleInfoEx(std::ptr::null(), LOCALE_SDECIMAL, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length > 0 { from_wide(&buffer) } else { ".".to_owned() }
}

pub fn registry_dword(key: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(value).as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut data).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

pub fn registry_string(key: &str, value: &str) -> Option<String> {
    let mut buffer = vec![0u16; 2048];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(value).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then(|| from_wide(&buffer))
}

pub fn set_registry_string(key: &str, value: &str, data: &str) -> Result<(), String> {
    let data = wide(data);
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(value).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS { Ok(()) } else { Err(std::io::Error::from_raw_os_error(status as i32).to_string()) }
}

pub fn delete_registry_value(key: &str, value: &str) -> Result<(), String> {
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(value).as_ptr()) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status as i32).to_string())
    }
}

pub fn apps_use_dark_theme() -> bool {
    registry_dword(THEME_KEY, "AppsUseLightTheme") == Some(0)
}

pub fn high_contrast() -> bool {
    let mut state: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
    state.cbSize = size_of::<HIGHCONTRASTW>() as u32;
    let read = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, state.cbSize, (&raw mut state).cast(), 0) };
    read != 0 && state.dwFlags & HCF_HIGHCONTRASTON != 0
}

pub fn system_color(index: i32) -> [u8; 3] {
    let color = unsafe { GetSysColor(index as SYS_COLOR_INDEX) };
    [color as u8, (color >> 8) as u8, (color >> 16) as u8]
}

pub fn replace_file(target: &Path, replacement: &Path, backup: &Path) -> std::io::Result<()> {
    let wide_path = |path: &Path| wide(&path.to_string_lossy());
    let replaced = unsafe {
        ReplaceFileW(
            wide_path(target).as_ptr(),
            wide_path(replacement).as_ptr(),
            wide_path(backup).as_ptr(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

pub fn first_argument(command_line: &str) -> Option<String> {
    let command_line = command_line.trim();
    if command_line.is_empty() {
        return None;
    }
    let mut count = 0;
    let arguments = unsafe { CommandLineToArgvW(wide(command_line).as_ptr(), &mut count) };
    if arguments.is_null() {
        return None;
    }
    let first = (count >= 1)
        .then(|| unsafe { *arguments })
        .filter(|pointer| !pointer.is_null())
        .map(|pointer| unsafe {
            let length = (0..).take_while(|offset| *pointer.add(*offset) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
        });
    unsafe { LocalFree(arguments.cast()) };
    first
}

pub struct SingleInstance(isize);

impl SingleInstance {
    pub fn acquire(name: &str) -> Option<Self> {
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, wide(name).as_ptr()) };
        if handle.is_null() {
            return None;
        }
        let instance = Self(handle as isize);
        (unsafe { GetLastError() } != ERROR_ALREADY_EXISTS).then_some(instance)
    }

    pub fn acquire_within(name: &str, patience: std::time::Duration) -> Option<Self> {
        let deadline = std::time::Instant::now() + patience;
        loop {
            if let Some(instance) = Self::acquire(name) {
                return Some(instance);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0 as _) };
    }
}

pub struct ExitSignal(isize);

impl ExitSignal {
    pub fn open(name: &str) -> Option<Self> {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide(name).as_ptr()) };
        (!handle.is_null()).then_some(Self(handle as isize))
    }

    pub fn request(&self) {
        unsafe { SetEvent(self.0 as _) };
    }

    pub fn wait(&self) -> bool {
        unsafe { WaitForSingleObject(self.0 as _, INFINITE) == WAIT_OBJECT_0 }
    }
}

impl Drop for ExitSignal {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0 as _) };
    }
}

pub fn apply_app_icon(window: isize) {
    for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
        unsafe {
            let size = GetSystemMetrics(metric);
            let icon = LoadImageW(GetModuleHandleW(std::ptr::null()), APP_ICON_RESOURCE, IMAGE_ICON, size, size, LR_SHARED);
            if !icon.is_null() {
                SendMessageW(window as _, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
}

pub fn open_url(url: &str) -> Result<(), String> {
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_area_is_never_empty() {
        let area = work_area(None);
        assert!(area.right > area.left && area.bottom > area.top);
    }

    #[test]
    fn times_format_in_the_user_locale() {
        let noon = 1_762_794_000_000;
        let time = format_short_time(noon);
        assert!(!time.is_empty());
        assert!(!time.contains(":00:00"));
        assert_eq!(format_short_time(i64::MAX), "");
    }

    #[test]
    fn command_lines_yield_their_executable() {
        assert_eq!(
            first_argument("\"C:\\Program Files\\App\\app.exe\" --flag"),
            Some("C:\\Program Files\\App\\app.exe".to_owned())
        );
        assert_eq!(first_argument("C:\\app.exe"), Some("C:\\app.exe".to_owned()));
        assert_eq!(first_argument("   "), None);
    }

    #[test]
    fn missing_registry_values_read_as_absent() {
        assert_eq!(registry_string("Software\\BluetoothBatteryMonitorTests", "Missing"), None);
        assert!(delete_registry_value("Software\\BluetoothBatteryMonitorTests", "Missing").is_ok());
        assert!(!decimal_separator().is_empty());
    }
}
