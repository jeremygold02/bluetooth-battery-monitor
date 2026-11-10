use crate::win::{PixelPoint, wide};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, HICON, IDNO, IDYES, KillTimer, MB_ICONINFORMATION,
    MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, PostMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_LBUTTONUP, WM_NULL, WM_POWERBROADCAST,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
};

const TRAY_CALLBACK: u32 = WM_APP + 1;
const WAKE: u32 = WM_APP + 2;
const RESUME_AUTOMATIC: usize = 18;
const RESUME_SUSPEND: usize = 7;
const ICON_ID: u32 = 1;
const TOOLTIP_LIMIT: usize = 127;
const BALLOON_TEXT_LIMIT: usize = 255;
const BALLOON_TITLE_LIMIT: usize = 63;
const WINDOW_CLASS: &str = "BluetoothBatteryMonitorTray";
const ICON_RETRY_TIMER: usize = usize::MAX;
const ICON_RETRY_MS: u32 = 2000;

pub enum ShellEvent {
    Click(PixelPoint),
    Command(u32, PixelPoint),
    Timer(usize),
    Wake,
    Resume,
}

#[derive(Clone)]
pub struct MenuEntry {
    pub id: u32,
    pub label: String,
    pub enabled: bool,
}

impl MenuEntry {
    pub fn item(id: u32, label: &str) -> Self {
        Self { id, label: label.to_owned(), enabled: true }
    }

    pub fn separator() -> Self {
        Self { id: 0, label: String::new(), enabled: false }
    }
}

thread_local! {
    static EVENTS: RefCell<VecDeque<ShellEvent>> = const { RefCell::new(VecDeque::new()) };
    static MENU: RefCell<Vec<MenuEntry>> = const { RefCell::new(Vec::new()) };
    static ICON: Cell<HICON> = const { Cell::new(std::ptr::null_mut()) };
    static TOOLTIP: RefCell<String> = const { RefCell::new(String::new()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

#[derive(Clone)]
pub struct Waker(isize);

impl Waker {
    pub fn wake(&self) {
        unsafe { PostMessageW(self.0 as HWND, WAKE, 0, 0) };
    }
}

pub struct Shell {
    window: HWND,
}

fn copy_text(destination: &mut [u16], text: &str, limit: usize) {
    let units: Vec<u16> = text.encode_utf16().take(limit.min(destination.len() - 1)).collect();
    destination[..units.len()].copy_from_slice(&units);
    destination[units.len()] = 0;
}

fn notify_data(window: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = ICON_ID;
    data
}

fn add_icon(window: HWND) {
    let mut data = notify_data(window);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_CALLBACK;
    data.hIcon = ICON.with(Cell::get);
    TOOLTIP.with(|tooltip| copy_text(&mut data.szTip, &tooltip.borrow(), TOOLTIP_LIMIT));
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &data);
        if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
            KillTimer(window, ICON_RETRY_TIMER);
        } else {
            SetTimer(window, ICON_RETRY_TIMER, ICON_RETRY_MS, None);
        }
    }
}

fn cursor() -> PixelPoint {
    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point) };
    PixelPoint { x: point.x, y: point.y }
}

fn push(event: ShellEvent) {
    EVENTS.with(|events| events.borrow_mut().push_back(event));
}

fn show_menu(window: HWND) {
    let entries = MENU.with(|menu| menu.borrow().clone());
    let position = cursor();
    let command = unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for entry in &entries {
            if entry.id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            } else {
                let flags = if entry.enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                AppendMenuW(menu, flags, entry.id as usize, wide(&entry.label).as_ptr());
            }
        }
        SetForegroundWindow(window);
        let command =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, position.x, position.y, 0, window, std::ptr::null());
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
        command
    };
    if command > 0 {
        push(ShellEvent::Command(command as u32, position));
    }
}

unsafe extern "system" fn window_procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        TRAY_CALLBACK => {
            match lparam as u32 {
                WM_LBUTTONUP => push(ShellEvent::Click(cursor())),
                WM_RBUTTONUP => show_menu(window),
                _ => {}
            }
            0
        }
        WM_TIMER => {
            if wparam == ICON_RETRY_TIMER {
                add_icon(window);
            } else {
                push(ShellEvent::Timer(wparam));
            }
            0
        }
        WAKE => {
            push(ShellEvent::Wake);
            0
        }
        WM_POWERBROADCAST => {
            if wparam == RESUME_AUTOMATIC || wparam == RESUME_SUSPEND {
                push(ShellEvent::Resume);
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ => {
            if message != 0 && message == TASKBAR_CREATED.with(Cell::get) {
                add_icon(window);
                return 0;
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

fn create_icon(rgba: &[u8], size: u32) -> HICON {
    let pixels = (size * size) as usize;
    if size == 0 || rgba.len() < pixels * 4 {
        return std::ptr::null_mut();
    }
    let stride = size.div_ceil(16) as usize * 2;
    let mut mask = vec![0u8; stride * size as usize];
    let mut bgra = Vec::with_capacity(pixels * 4);
    for (index, pixel) in rgba.chunks_exact(4).take(pixels).enumerate() {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        if pixel[3] == 0 {
            let (x, y) = (index % size as usize, index / size as usize);
            mask[y * stride + x / 8] |= 0x80 >> (x % 8);
        }
    }
    unsafe {
        CreateIcon(GetModuleHandleW(std::ptr::null()), size as i32, size as i32, 1, 32, mask.as_ptr(), bgra.as_ptr())
    }
}

impl Shell {
    pub fn create(tooltip: &str, rgba: &[u8], size: u32, menu: Vec<MenuEntry>) -> Option<Self> {
        let class_name = wide(WINDOW_CLASS);
        let window = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let mut class: WNDCLASSW = std::mem::zeroed();
            class.lpfnWndProc = Some(window_procedure);
            class.hInstance = instance;
            class.lpszClassName = class_name.as_ptr();
            if RegisterClassW(&class) == 0 {
                return None;
            }
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            return None;
        }
        MENU.with(|entries| *entries.borrow_mut() = menu);
        TOOLTIP.with(|text| *text.borrow_mut() = tooltip.to_owned());
        ICON.with(|icon| icon.set(create_icon(rgba, size)));
        TASKBAR_CREATED.with(|message| {
            message.set(unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) });
        });
        add_icon(window);
        Some(Self { window })
    }

    pub fn waker(&self) -> Waker {
        Waker(self.window as isize)
    }

    pub fn set_icon(&self, rgba: &[u8], size: u32) {
        let icon = create_icon(rgba, size);
        if icon.is_null() {
            return;
        }
        let mut data = notify_data(self.window);
        data.uFlags = NIF_ICON;
        data.hIcon = icon;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
        let previous = ICON.with(|current| current.replace(icon));
        if !previous.is_null() {
            unsafe { DestroyIcon(previous) };
        }
    }

    pub fn set_tooltip(&self, text: &str) {
        TOOLTIP.with(|tooltip| *tooltip.borrow_mut() = text.to_owned());
        let mut data = notify_data(self.window);
        data.uFlags = NIF_TIP;
        copy_text(&mut data.szTip, text, TOOLTIP_LIMIT);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    pub fn notify(&self, title: &str, message: &str, warning: bool) {
        let mut data = notify_data(self.window);
        data.uFlags = NIF_INFO;
        copy_text(&mut data.szInfo, message, BALLOON_TEXT_LIMIT);
        copy_text(&mut data.szInfoTitle, title, BALLOON_TITLE_LIMIT);
        data.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    pub fn message(&self, title: &str, text: &str, warning: bool) {
        let icon = if warning { MB_ICONWARNING } else { MB_ICONINFORMATION };
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(text).as_ptr(),
                wide(title).as_ptr(),
                MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST,
            )
        };
    }

    pub fn confirm(&self, title: &str, text: &str) -> bool {
        let answer = unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(text).as_ptr(),
                wide(title).as_ptr(),
                MB_YESNO | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST,
            )
        };
        answer == IDYES && answer != IDNO
    }

    pub fn set_timer(&self, id: usize, milliseconds: u32) {
        unsafe { SetTimer(self.window, id, milliseconds.max(1), None) };
    }

    pub fn kill_timer(&self, id: usize) {
        unsafe { KillTimer(self.window, id) };
    }

    pub fn run(&self, mut handle: impl FnMut(&Shell, ShellEvent) -> bool) {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        'pump: while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            loop {
                let event = EVENTS.with(|events| events.borrow_mut().pop_front());
                let Some(event) = event else { break };
                if !handle(self, event) {
                    break 'pump;
                }
            }
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let data = notify_data(self.window);
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            DestroyWindow(self.window);
        }
        let icon = ICON.with(|current| current.replace(std::ptr::null_mut()));
        if !icon.is_null() {
            unsafe { DestroyIcon(icon) };
        }
    }
}
