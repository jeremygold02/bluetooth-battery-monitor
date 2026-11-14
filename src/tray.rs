use crate::collect;
use crate::icon::{ICON_SIZE, IconContent, IconFonts, IconStyle, render};
use crate::hfp::HfpMonitor;
use crate::ipc::{self, FromPopup, FromSettings, PopupState, PopupView, SettingsInit, ToSettings, UpdateStatus};
use crate::monitor::{Device, FetchResult, Monitor};
use crate::rate::LearnedBatteryRate;
use crate::settings::{
    self, CachedBatteryReading, ICON_METRIC_AVERAGE, ICON_METRIC_DEVICE, Settings, SettingsStore,
};
use crate::shell::{MenuEntry, Shell, ShellEvent, Waker};
use crate::startup;
use crate::updater::{self, ManualCheck, PeriodicCheck, PreparedUpdate, UpdateCheckAction, UpdateOutcome, Updater};
use crate::win::{self, PixelPoint};
use crate::windows_collectors::ConnectionWatcher;
use serde::de::DeserializeOwned;
use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const TIMER_REFRESH: usize = 1;
const TIMER_CONNECTION: usize = 2;
const TIMER_STARTUP_UI: usize = 3;
const TIMER_UPDATE_PERIODIC: usize = 4;
const TIMER_UPDATE_WAIT: usize = 5;
const TIMER_UPDATE_CLEANUP: usize = 6;
const UPDATE_CLEANUP_DELAY_MS: u32 = 60_000;
const CONNECTION_DEBOUNCE_MS: u32 = 1500;
const STARTUP_UI_DELAY_MS: u32 = 2500;
const UPDATE_CHECK_INTERVAL_MS: u32 = 12 * 60 * 60 * 1000;
const UPDATE_WAIT_MS: u32 = 2000;
const UPDATE_HANDOVER_PATIENCE: Duration = Duration::from_secs(30);
const UPDATE_STATE_FILE: &str = "update-state.json";
const UPDATES_TITLE: &str = "Bluetooth Battery Monitor Updates";
const UPDATE_NOTIFICATION_TITLE: &str = "Bluetooth Battery Monitor Update";
const CHECKING: &str = "Checking...";
const INSTALLING: &str = "Installing...";
const REOPEN_GUARD: Duration = Duration::from_millis(300);
const MENU_REFRESH: u32 = 1;
const MENU_SHOW: u32 = 2;
const MENU_SETTINGS: u32 = 3;
const MENU_UPDATES: u32 = 4;
const MENU_EXIT: u32 = 5;
const SINGLE_INSTANCE_MUTEX: &str = "BluetoothBatteryMonitor.SingleInstance";
const EXIT_EVENT: &str = "Local\\BluetoothBatteryMonitor.ExitRequest";
const DEVELOPMENT_VERSION: &str = "0.0.0";
const APP_TITLE: &str = "Bluetooth Battery";
const SETTINGS_TITLE: &str = "Bluetooth Battery Settings";
const REFRESHING_STATUS: &str = "Checking device batteries...";
const REFRESH_FAILED_WITHOUT_DATA_STATUS: &str = "Unable to check device batteries. Try again.";
const RATES_WARNING: &str = "Could not save learned battery rates; check the settings file.";
const CACHE_WARNING: &str = "Could not save last known battery levels; check the settings file.";
const STATUS_LIMIT: usize = 120;

#[derive(Clone, Default)]
struct View {
    devices: Vec<Device>,
    lowest: Option<u8>,
    average: Option<u8>,
    last_updated: Option<i64>,
    error_message: Option<String>,
    status_message: Option<String>,
    stale: bool,
    refreshing: bool,
}

enum WorkerCommand {
    Fetch,
    SetHfpTrace(bool),
    ResetRates,
    RatesSaved(u64),
    CacheSaved(u64),
}

struct FetchOutcome {
    result: Option<FetchResult>,
    attempted_at: i64,
    rates: Option<(u64, Vec<LearnedBatteryRate>)>,
    cache: Option<(u64, Vec<CachedBatteryReading>)>,
}

enum Incoming {
    Fetched(Box<FetchOutcome>),
    HfpTraceStatus(Option<String>),
    Popup(u32, FromPopup),
    PopupClosed(u32),
    Settings(u32, FromSettings),
    SettingsClosed(u32),
    ConnectionChanged,
    ExitRequested,
    Update(Box<UpdateEvent>),
}

enum UpdateEvent {
    Periodic(PeriodicCheck),
    Manual(ManualCheck),
    Outcome { outcome: UpdateOutcome, interactive: bool },
}

struct Window {
    process: Child,
    input: ChildStdin,
}

impl Window {
    fn close(mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

struct Tray {
    settings: SettingsStore,
    fonts: IconFonts,
    commands: Sender<WorkerCommand>,
    incoming: Sender<Incoming>,
    waker: Waker,
    last_successful: Option<View>,
    current: Option<View>,
    shown_icon: Option<(IconContent, IconStyle)>,
    first_threshold_level: Option<u8>,
    second_threshold_level: Option<u8>,
    last_refresh_started: Option<Instant>,
    refresh_in_progress: bool,
    refresh_feedback_requested: bool,
    connection_refresh_pending: bool,
    popup: Option<Window>,
    popup_closed_at: Option<Instant>,
    anchor: Option<PixelPoint>,
    settings_window: Option<Window>,
    hfp_trace_status: Option<String>,
    restart_allowed: Arc<AtomicBool>,
    updater: Option<Arc<Updater>>,
    update_status: String,
    update_busy: Option<&'static str>,
    pending_update: Option<(PreparedUpdate, bool)>,
    show_popup_on_start: bool,
    show_settings_on_start: bool,
}

pub fn run(arguments: &[String]) {
    let has = |flag: &str| arguments.iter().any(|argument| argument.eq_ignore_ascii_case(flag));
    let exit_signal = win::ExitSignal::open(EXIT_EVENT);
    let patience = if has("--update-complete") { UPDATE_HANDOVER_PATIENCE } else { Duration::ZERO };
    let Some(_instance) = win::SingleInstance::acquire_within(SINGLE_INSTANCE_MUTEX, patience) else {
        if has("--exit")
            && let Some(signal) = &exit_signal
        {
            signal.request();
        }
        return;
    };
    if has("--exit") {
        return;
    }
    win::enable_dpi_awareness();
    let Some(settings_path) = settings::default_path() else { return };
    let store = SettingsStore::load(&settings_path);
    let fonts = IconFonts::load();
    let unknown = render(IconContent::Unknown, &IconStyle::from_settings(&store.values), &fonts);
    let menu = vec![
        MenuEntry::item(MENU_REFRESH, "Refresh now"),
        MenuEntry::item(MENU_SHOW, "Show batteries"),
        MenuEntry::item(MENU_SETTINGS, "Settings"),
        MenuEntry::item(MENU_UPDATES, "Check for updates"),
        MenuEntry::separator(),
        MenuEntry::item(MENU_EXIT, "Exit"),
    ];
    let Some(shell) = Shell::create("", &unknown, ICON_SIZE as u32, menu) else { return };

    let (incoming, inbox) = mpsc::channel::<Incoming>();
    let (commands, worker_commands) = mpsc::channel::<WorkerCommand>();
    let mut monitor = Monitor::new(&win::decimal_separator());
    monitor.initialize_rates(&store.values.learned_battery_rates);
    monitor.initialize_cache(&store.values.cached_battery_readings, win::now_ms());
    let worker_incoming = incoming.clone();
    let worker_waker = shell.waker();
    std::thread::spawn(move || work(monitor, worker_commands, worker_incoming, worker_waker));
    if store.values.hfp_trace_enabled {
        let _ = commands.send(WorkerCommand::SetHfpTrace(true));
    }

    let mut tray = Tray {
        settings: store,
        fonts,
        commands,
        incoming,
        waker: shell.waker(),
        last_successful: None,
        current: None,
        shown_icon: None,
        first_threshold_level: None,
        second_threshold_level: None,
        last_refresh_started: None,
        refresh_in_progress: false,
        refresh_feedback_requested: false,
        connection_refresh_pending: false,
        popup: None,
        popup_closed_at: None,
        anchor: None,
        settings_window: None,
        hfp_trace_status: None,
        restart_allowed: Arc::new(AtomicBool::new(true)),
        updater: None,
        update_status: updater::update_complete_status(arguments, updater::build_version()).unwrap_or_default(),
        update_busy: None,
        pending_update: None,
        show_popup_on_start: has("--show-usage"),
        show_settings_on_start: has("--show-settings"),
    };
    if let (Ok(executable), Some(folder)) = (std::env::current_exe(), settings_path.parent()) {
        let state_path = folder.join(UPDATE_STATE_FILE);
        tray.updater = Some(Arc::new(Updater::new(updater::build_version(), &executable, &state_path)));
        let automatic = updater::build_version() != DEVELOPMENT_VERSION;
        if automatic {
            shell.set_timer(TIMER_UPDATE_PERIODIC, UPDATE_CHECK_INTERVAL_MS);
        }
        if automatic && updater::should_check_for_startup_update(arguments) {
            tray.update_status = "Checking for updates...".to_owned();
            tray.run_update(CHECKING, |updater, can_restart| UpdateEvent::Outcome {
                outcome: updater.startup_update(can_restart),
                interactive: false,
            });
        } else if !has("--update-complete") {
            updater::remove_stale_update_files(&executable);
        }
        shell.set_timer(TIMER_UPDATE_CLEANUP, UPDATE_CLEANUP_DELAY_MS);
    }
    if let Some(signal) = exit_signal {
        let exit_incoming = tray.incoming.clone();
        let exit_waker = shell.waker();
        std::thread::spawn(move || {
            if signal.wait() {
                let _ = exit_incoming.send(Incoming::ExitRequested);
                exit_waker.wake();
            }
        });
    }
    let watcher_incoming = tray.incoming.clone();
    let watcher_waker = shell.waker();
    let _watcher = ConnectionWatcher::start(move || {
        let _ = watcher_incoming.send(Incoming::ConnectionChanged);
        watcher_waker.wake();
    });
    tray.refresh_if_due(&shell, false);
    if tray.show_popup_on_start || tray.show_settings_on_start {
        shell.set_timer(TIMER_STARTUP_UI, STARTUP_UI_DELAY_MS);
    }
    shell.run(|shell, event| tray.handle(shell, event, &inbox));
    tray.shut_down();
}

fn work(mut monitor: Monitor, commands: Receiver<WorkerCommand>, incoming: Sender<Incoming>, waker: Waker) {
    let mut hfp_monitor = None;
    let started = Instant::now();
    let mut saved_rates = 0;
    let mut saved_cache = 0;
    for command in commands {
        match command {
            WorkerCommand::SetHfpTrace(enabled) => {
                let status = if enabled {
                    let hfp_incoming = incoming.clone();
                    let hfp_waker = waker.clone();
                    match HfpMonitor::start(move || {
                        let _ = hfp_incoming.send(Incoming::ConnectionChanged);
                        hfp_waker.wake();
                    }) {
                        Ok(trace) => {
                            hfp_monitor = Some(trace);
                            Some("Listening for Hands-Free battery reports.".to_owned())
                        }
                        Err(message) => Some(message),
                    }
                } else {
                    hfp_monitor = None;
                    None
                };
                let _ = incoming.send(Incoming::HfpTraceStatus(status));
                waker.wake();
            }
            WorkerCommand::ResetRates => monitor.reset_rates(),
            WorkerCommand::RatesSaved(revision) => saved_rates = revision,
            WorkerCommand::CacheSaved(revision) => saved_cache = revision,
            WorkerCommand::Fetch => {
                let attempted_at = win::now_ms();
                let collected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    collect::collect_all(hfp_monitor.as_ref())
                }))
                .ok();
                let outcome = match collected {
                    Some((probes, errors, trace_error)) => {
                        if let Some(message) = trace_error {
                            let _ = incoming.send(Incoming::HfpTraceStatus(Some(message)));
                        }
                        let result = monitor.fetch(probes, &errors, attempted_at, started.elapsed().as_secs_f64());
                        FetchOutcome {
                            result: Some(result),
                            attempted_at,
                            rates: monitor.updated_rates(saved_rates),
                            cache: monitor.updated_cache(saved_cache),
                        }
                    }
                    None => FetchOutcome { result: None, attempted_at, rates: None, cache: None },
                };
                if incoming.send(Incoming::Fetched(Box::new(outcome))).is_err() {
                    return;
                }
                waker.wake();
            }
        }
    }
}

fn spawn_window<M>(
    arguments: &[String],
    incoming: &Sender<Incoming>,
    waker: &Waker,
    message: fn(u32, M) -> Incoming,
    closed: fn(u32) -> Incoming,
) -> Option<Window>
where
    M: DeserializeOwned + Send + 'static,
{
    let executable = std::env::current_exe().ok()?;
    let mut process = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let (Some(input), Some(output)) = (process.stdin.take(), process.stdout.take()) else {
        let _ = process.kill();
        let _ = process.wait();
        return None;
    };
    let id = process.id();
    win::allow_foreground(id);
    let incoming = incoming.clone();
    let waker = waker.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        while let Ok(Some(received)) = ipc::read_message::<M>(&mut reader) {
            if incoming.send(message(id, received)).is_err() {
                return;
            }
            waker.wake();
        }
        let _ = incoming.send(closed(id));
        waker.wake();
    });
    Some(Window { process, input })
}

fn compact(value: Option<&str>, maximum: usize) -> Option<String> {
    let collapsed = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else if collapsed.chars().count() > maximum {
        let shortened: String = collapsed.chars().take(maximum - 3).collect();
        Some(format!("{}...", shortened.trim_end()))
    } else {
        Some(collapsed)
    }
}

fn append_status(view: &mut View, warning: &str) {
    view.status_message = Some(match view.status_message.take().filter(|status| !status.is_empty()) {
        Some(status) => format!("{status} {warning}"),
        None => warning.to_owned(),
    });
}

fn display_name(device: &Device) -> &str {
    if device.name.trim().is_empty() { "Wireless device" } else { &device.name }
}

fn summary_line(device: Option<&Device>) -> String {
    match device {
        None => "No other supported device".to_owned(),
        Some(device) => match device.percent {
            Some(percent) => format!("{}: {percent}%", display_name(device)),
            None => format!("{}: unavailable", display_name(device)),
        },
    }
}

fn native_tooltip(view: &View) -> String {
    view.devices
        .iter()
        .map(|device| {
            let name = display_name(device).replace(['\r', '\n'], " ");
            match device.percent {
                Some(percent) => format!("{} {percent}%", name.trim()),
                None => format!("{} ?%", name.trim()),
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn threshold_level(device: Option<&Device>, settings: &Settings) -> u8 {
    let Some(device) = device.filter(|device| !device.cached) else { return 0 };
    let remaining = i32::from(device.percent.unwrap_or(0));
    if remaining <= settings.critical_threshold {
        2
    } else if remaining <= settings.low_threshold {
        1
    } else {
        0
    }
}

fn icon_content(view: &View, settings: &Settings) -> IconContent {
    let percent = if settings.icon_metric.eq_ignore_ascii_case(ICON_METRIC_DEVICE) {
        view.devices
            .iter()
            .find(|device| {
                device.percent.is_some()
                    && !settings.icon_device_id.is_empty()
                    && device.id.to_lowercase() == settings.icon_device_id.to_lowercase()
            })
            .and_then(|device| device.percent)
    } else if settings.icon_metric.eq_ignore_ascii_case(ICON_METRIC_AVERAGE) {
        view.average
    } else {
        view.lowest
    };
    percent.map_or(IconContent::Unknown, IconContent::Percent)
}

impl Tray {
    fn handle(&mut self, shell: &Shell, event: ShellEvent, inbox: &Receiver<Incoming>) -> bool {
        match event {
            ShellEvent::Click(point) => {
                self.anchor = Some(point);
                if self.popup.is_some() {
                    self.close_popup();
                } else if self.popup_closed_at.is_none_or(|at| at.elapsed() > REOPEN_GUARD) {
                    self.open_popup(shell);
                }
            }
            ShellEvent::Command(MENU_REFRESH, _) => self.refresh(shell, true),
            ShellEvent::Command(MENU_SHOW, point) => {
                self.anchor = Some(point);
                self.open_popup(shell);
            }
            ShellEvent::Command(MENU_SETTINGS, _) => self.show_settings(),
            ShellEvent::Command(MENU_UPDATES, _) => self.check_for_updates(shell),
            ShellEvent::Command(MENU_EXIT, _) => return false,
            ShellEvent::Command(_, _) => {}
            ShellEvent::Timer(TIMER_REFRESH) => {
                shell.kill_timer(TIMER_REFRESH);
                self.refresh_if_due(shell, false);
            }
            ShellEvent::Timer(TIMER_CONNECTION) => {
                shell.kill_timer(TIMER_CONNECTION);
                if self.refresh_in_progress {
                    self.connection_refresh_pending = true;
                } else {
                    self.refresh(shell, false);
                }
            }
            ShellEvent::Timer(TIMER_STARTUP_UI) => {
                shell.kill_timer(TIMER_STARTUP_UI);
                if std::mem::take(&mut self.show_popup_on_start) {
                    self.open_popup(shell);
                }
                if std::mem::take(&mut self.show_settings_on_start) {
                    self.show_settings();
                }
            }
            ShellEvent::Timer(TIMER_UPDATE_PERIODIC) => {
                if self.update_busy.is_none() {
                    self.set_update_status("Checking for updates...");
                    self.run_update(CHECKING, |updater, _| UpdateEvent::Periodic(updater.periodic_check()));
                }
            }
            ShellEvent::Timer(TIMER_UPDATE_WAIT) => {
                shell.kill_timer(TIMER_UPDATE_WAIT);
                self.retry_pending_update();
            }
            ShellEvent::Timer(TIMER_UPDATE_CLEANUP) => {
                if self.update_busy.is_none() && self.pending_update.is_none() {
                    shell.kill_timer(TIMER_UPDATE_CLEANUP);
                    if let Ok(executable) = std::env::current_exe() {
                        updater::remove_stale_update_files(&executable);
                    }
                }
            }
            ShellEvent::Timer(_) => {}
            ShellEvent::Resume => {
                let _ = self.commands.send(WorkerCommand::ResetRates);
                shell.set_timer(TIMER_CONNECTION, CONNECTION_DEBOUNCE_MS);
            }
            ShellEvent::Wake => {
                while let Ok(message) = inbox.try_recv() {
                    if !self.receive(shell, message) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn receive(&mut self, shell: &Shell, message: Incoming) -> bool {
        match message {
            Incoming::ExitRequested => return false,
            Incoming::Update(event) => return self.handle_update(shell, *event),
            Incoming::Fetched(outcome) => self.complete_refresh(shell, *outcome),
            Incoming::HfpTraceStatus(status) => {
                if self.settings.values.hfp_trace_enabled {
                    self.hfp_trace_status = status;
                    if let Some(window) = &mut self.settings_window {
                        let status = self.hfp_trace_status.clone();
                        let _ = ipc::write_message(&mut window.input, &ToSettings::HfpTraceStatus(status));
                    }
                }
            }
            Incoming::Popup(id, request) => {
                if self.popup.as_ref().is_some_and(|popup| popup.process.id() == id) {
                    match request {
                        FromPopup::Refresh => self.refresh(shell, false),
                        FromPopup::Settings => {
                            self.close_popup();
                            self.show_settings();
                        }
                    }
                }
            }
            Incoming::PopupClosed(id) => {
                if self.popup.as_ref().is_some_and(|popup| popup.process.id() == id) {
                    self.close_popup();
                }
            }
            Incoming::Settings(id, request) => {
                if self.settings_window.as_ref().is_some_and(|window| window.process.id() == id) {
                    match request {
                        FromSettings::Dirty(dirty) => self.restart_allowed.store(!dirty, Ordering::SeqCst),
                        FromSettings::CheckUpdates => self.check_for_updates(shell),
                        FromSettings::Save(candidate) => self.save_settings(shell, *candidate),
                    }
                }
            }
            Incoming::SettingsClosed(id) => {
                if self.settings_window.as_ref().is_some_and(|window| window.process.id() == id) {
                    if let Some(window) = self.settings_window.take() {
                        window.close();
                    }
                    self.restart_allowed.store(true, Ordering::SeqCst);
                }
            }
            Incoming::ConnectionChanged => shell.set_timer(TIMER_CONNECTION, CONNECTION_DEBOUNCE_MS),
        }
        true
    }

    fn set_update_status(&mut self, status: &str) {
        self.update_status = status.to_owned();
        self.push_update_state();
    }

    fn push_update_state(&mut self) {
        let state = self.update_state();
        if let Some(window) = &mut self.settings_window {
            let _ = ipc::write_message(&mut window.input, &ToSettings::Update(state));
        }
    }

    fn run_update<F>(&mut self, busy: &'static str, job: F)
    where
        F: FnOnce(&Updater, &dyn Fn() -> bool) -> UpdateEvent + Send + 'static,
    {
        let Some(updater) = self.updater.clone() else { return };
        self.update_busy = Some(busy);
        self.push_update_state();
        let allowed = Arc::clone(&self.restart_allowed);
        let incoming = self.incoming.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let can_restart = || allowed.load(Ordering::SeqCst);
            let event = job(&updater, &can_restart);
            if incoming.send(Incoming::Update(Box::new(event))).is_ok() {
                waker.wake();
            }
        });
    }

    fn check_for_updates(&mut self, shell: &Shell) {
        if self.update_busy.is_some() {
            shell.message(UPDATES_TITLE, "An update check is already running.", false);
            return;
        }
        self.set_update_status("Checking for updates...");
        self.run_update(CHECKING, |updater, _| UpdateEvent::Manual(updater.manual_check()));
    }

    fn retry_pending_update(&mut self) {
        if let Some((pending, interactive)) = self.pending_update.take() {
            self.run_update(INSTALLING, move |updater, can_restart| UpdateEvent::Outcome {
                outcome: updater.finish_update(pending, can_restart),
                interactive,
            });
        }
    }

    fn handle_update(&mut self, shell: &Shell, event: UpdateEvent) -> bool {
        self.update_busy = None;
        match event {
            UpdateEvent::Periodic(check) => {
                self.set_update_status(&check.status_text);
                if let Some(notification) = check.notification {
                    shell.notify(UPDATE_NOTIFICATION_TITLE, &notification, true);
                }
            }
            UpdateEvent::Manual(check) => {
                self.set_update_status(&check.status_text);
                match (check.action, check.update) {
                    (UpdateCheckAction::PromptInstall, Some(info)) => {
                        if shell.confirm(UPDATES_TITLE, &check.dialog_text) {
                            self.set_update_status(&format!("Installing version {}...", info.latest_version));
                            self.run_update(INSTALLING, move |updater, can_restart| UpdateEvent::Outcome {
                                outcome: updater.install_update(&info, can_restart),
                                interactive: true,
                            });
                        }
                    }
                    (UpdateCheckAction::PromptRelease, _) => {
                        if shell.confirm(UPDATES_TITLE, &check.dialog_text)
                            && let Err(message) = win::open_url(&updater::release_page_url())
                        {
                            shell.message(UPDATES_TITLE, &format!("Could not open release page:\r\n\r\n{message}"), true);
                        }
                    }
                    (_, update) => shell.message(UPDATES_TITLE, &check.dialog_text, update.is_none()),
                }
            }
            UpdateEvent::Outcome { outcome, interactive } => {
                self.set_update_status(&outcome.status_text());
                match outcome {
                    UpdateOutcome::InstalledAndRelaunching { .. } => return false,
                    UpdateOutcome::Deferred { pending, .. } => {
                        self.pending_update = Some((pending, interactive));
                        self.update_busy = Some(INSTALLING);
                        self.push_update_state();
                        shell.set_timer(TIMER_UPDATE_WAIT, UPDATE_WAIT_MS);
                    }
                    other if interactive => {
                        if let Some(text) = other.dialog_text() {
                            shell.message(UPDATES_TITLE, &text, true);
                        }
                    }
                    other => {
                        if let Some(text) = other.notification() {
                            shell.notify(UPDATE_NOTIFICATION_TITLE, &text, true);
                        }
                    }
                }
            }
        }
        true
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(u64::try_from(self.settings.values.refresh_seconds.max(30)).unwrap_or(60))
    }

    fn refresh_if_due(&mut self, shell: &Shell, show_feedback: bool) {
        if self.refresh_in_progress {
            return;
        }
        let Some(started) = self.last_refresh_started else {
            self.refresh(shell, show_feedback);
            return;
        };
        let remaining = self.refresh_interval().saturating_sub(started.elapsed());
        if remaining.is_zero() {
            self.refresh(shell, show_feedback);
        } else {
            shell.set_timer(TIMER_REFRESH, u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX));
        }
    }

    fn refresh(&mut self, shell: &Shell, show_feedback: bool) {
        self.refresh_feedback_requested |= show_feedback;
        if self.refresh_in_progress {
            self.push_state();
            return;
        }
        self.refresh_in_progress = true;
        self.last_refresh_started = Some(Instant::now());
        shell.kill_timer(TIMER_REFRESH);

        let mut view = self.last_successful.clone().unwrap_or_default();
        view.status_message = Some(REFRESHING_STATUS.to_owned());
        view.stale = false;
        view.refreshing = true;
        shell.set_tooltip(&self.last_successful.as_ref().map(native_tooltip).unwrap_or_default());
        self.current = Some(view);
        self.push_state();
        if self.commands.send(WorkerCommand::Fetch).is_err() {
            self.refresh_in_progress = false;
        }
    }

    fn complete_refresh(&mut self, shell: &Shell, outcome: FetchOutcome) {
        self.refresh_in_progress = false;
        let show_feedback = std::mem::take(&mut self.refresh_feedback_requested);
        match outcome.result {
            None => self.apply_failure(shell, show_feedback),
            Some(result) => {
                let mut view = View {
                    devices: result.devices,
                    lowest: result.lowest,
                    average: result.average,
                    last_updated: Some(outcome.attempted_at),
                    status_message: result.status_message,
                    ..Default::default()
                };
                if let Some((revision, rates)) = outcome.rates {
                    if self.settings.try_save_rates(rates) {
                        let _ = self.commands.send(WorkerCommand::RatesSaved(revision));
                    } else {
                        append_status(&mut view, RATES_WARNING);
                    }
                }
                if let Some((revision, cache)) = outcome.cache {
                    if self.settings.try_save_cache(cache) {
                        let _ = self.commands.send(WorkerCommand::CacheSaved(revision));
                    } else {
                        append_status(&mut view, CACHE_WARNING);
                    }
                }
                self.last_successful = Some(view.clone());
                self.current = Some(view);
                self.render(shell, true, true, show_feedback);
            }
        }
        self.refresh_if_due(shell, false);
        if std::mem::take(&mut self.connection_refresh_pending) {
            shell.set_timer(TIMER_CONNECTION, CONNECTION_DEBOUNCE_MS);
        }
    }

    fn apply_failure(&mut self, shell: &Shell, show_feedback: bool) {
        let message = REFRESH_FAILED_WITHOUT_DATA_STATUS.to_owned();
        let mut view = self.last_successful.clone().unwrap_or_default();
        view.error_message = Some(message.clone());
        view.status_message = Some(message.clone());
        view.stale = true;
        view.refreshing = false;
        if self.last_successful.is_some() {
            shell.set_tooltip(&native_tooltip(&view));
        } else {
            self.set_icon(shell, IconContent::Error);
            shell.set_tooltip("");
        }
        self.current = Some(view);
        self.push_state();
        if show_feedback {
            shell.notify(APP_TITLE, &message, true);
        }
    }

    fn set_icon(&mut self, shell: &Shell, content: IconContent) {
        let style = IconStyle::from_settings(&self.settings.values);
        if self.shown_icon != Some((content, style)) {
            shell.set_icon(&render(content, &style, &self.fonts), ICON_SIZE as u32);
            self.shown_icon = Some((content, style));
        }
    }

    fn render(&mut self, shell: &Shell, render_icon: bool, update_thresholds: bool, show_feedback: bool) {
        let Some(view) = self.current.clone() else { return };
        if render_icon {
            self.set_icon(shell, icon_content(&view, &self.settings.values));
        }
        shell.set_tooltip(&native_tooltip(&view));
        self.push_state();
        let threshold_shown = update_thresholds && self.notify_thresholds(shell, &view);
        if show_feedback && !threshold_shown {
            let summary = format!("{} | {}", summary_line(view.devices.first()), summary_line(view.devices.get(1)));
            shell.notify(APP_TITLE, &summary, false);
        }
    }

    fn notify_thresholds(&mut self, shell: &Shell, view: &View) -> bool {
        let settings = &self.settings.values;
        let levels = [
            (view.devices.first(), threshold_level(view.devices.first(), settings), self.first_threshold_level),
            (view.devices.get(1), threshold_level(view.devices.get(1), settings), self.second_threshold_level),
        ];
        let notify = settings.threshold_notifications;
        self.first_threshold_level = Some(levels[0].1);
        self.second_threshold_level = Some(levels[1].1);
        let mut lines = Vec::new();
        let mut highest = 0;
        for (device, level, previous) in levels {
            let Some(device) = device else { continue };
            if notify && previous.is_some_and(|previous| level > previous) {
                let severity = if level >= 2 { "critical" } else { "low" };
                lines.push(format!(
                    "{} battery is {severity}: {}% remaining.",
                    device.name,
                    device.percent.unwrap_or(0)
                ));
                highest = highest.max(level);
            }
        }
        if lines.is_empty() {
            return false;
        }
        shell.notify(if highest >= 2 { "Battery Critical" } else { "Battery Low" }, &lines.join("\r\n"), true);
        true
    }

    fn popup_state(&self) -> PopupState {
        let refreshing = self.refresh_in_progress || self.current.as_ref().is_some_and(|view| view.refreshing);
        let status = self.current.as_ref().filter(|_| !refreshing).and_then(|view| {
            compact(view.status_message.as_deref(), STATUS_LIMIT)
                .or_else(|| compact(view.error_message.as_deref(), STATUS_LIMIT))
                .or_else(|| view.stale.then(|| "Battery levels may be stale".to_owned()))
        });
        PopupState {
            devices: self.current.as_ref().map(|view| view.devices.clone()).unwrap_or_default(),
            updated: self
                .current
                .as_ref()
                .and_then(|view| view.last_updated)
                .map(win::format_short_time)
                .unwrap_or_default(),
            status,
            refreshing,
            has_snapshot: self.current.is_some(),
            view: PopupView::from_settings(&self.settings.values),
        }
    }

    fn push_state(&mut self) {
        let state = self.popup_state();
        if let Some(popup) = &mut self.popup {
            let _ = ipc::write_message(&mut popup.input, &state);
        }
        if let Some(window) = &mut self.settings_window {
            let _ = ipc::write_message(&mut window.input, &ToSettings::Devices(state.devices));
        }
    }

    fn open_popup(&mut self, shell: &Shell) {
        if self.popup.is_some() {
            return;
        }
        let mut arguments = vec!["--popup".to_owned()];
        if let Some(anchor) = self.anchor {
            arguments.push("--anchor".to_owned());
            arguments.push(format!("{},{}", anchor.x, anchor.y));
        }
        let Some(popup) =
            spawn_window(&arguments, &self.incoming, &self.waker, Incoming::Popup, Incoming::PopupClosed)
        else {
            return;
        };
        self.popup = Some(popup);
        self.refresh(shell, false);
    }

    fn close_popup(&mut self) {
        if let Some(popup) = self.popup.take() {
            popup.close();
            self.popup_closed_at = Some(Instant::now());
        }
    }

    fn update_state(&self) -> UpdateStatus {
        UpdateStatus {
            text: if self.update_status.trim().is_empty() {
                "Updates have not been checked yet.".to_owned()
            } else {
                self.update_status.clone()
            },
            busy: self.update_busy.is_some(),
            button: self.update_busy.unwrap_or("Check for updates").to_owned(),
        }
    }

    fn show_settings(&mut self) {
        if let Some(window) = &mut self.settings_window {
            win::allow_foreground(window.process.id());
            let _ = ipc::write_message(&mut window.input, &ToSettings::Activate);
            return;
        }
        let Some(mut window) = spawn_window(
            &["--settings".to_owned()],
            &self.incoming,
            &self.waker,
            Incoming::Settings,
            Incoming::SettingsClosed,
        ) else {
            return;
        };
        let mut editable = self.settings.values.clone();
        editable.learned_battery_rates.clear();
        editable.cached_battery_readings.clear();
        let init = SettingsInit {
            settings: editable,
            start_with_windows: startup::is_enabled(),
            hfp_trace_status: self.hfp_trace_status.clone(),
            devices: self.current.as_ref().map(|view| view.devices.clone()).unwrap_or_default(),
            version: updater::build_version().to_owned(),
            update: self.update_state(),
            save_blocked: self.settings.save_blocked_message(),
        };
        let _ = ipc::write_message(&mut window.input, &ToSettings::Init(Box::new(init)));
        self.settings_window = Some(window);
    }

    fn save_settings(&mut self, shell: &Shell, mut candidate: Settings) {
        let trace_changed = candidate.hfp_trace_enabled != self.settings.values.hfp_trace_enabled;
        let startup_changed = candidate.start_with_windows != startup::is_enabled();
        candidate.learned_battery_rates = self.settings.values.learned_battery_rates.clone();
        candidate.cached_battery_readings = self.settings.values.cached_battery_readings.clone();
        let result = self.settings.save(candidate).err().map(|message| format!("Could not save settings:\n\n{message}"));
        let saved = result.is_none();
        if let Some(window) = &mut self.settings_window {
            let _ = ipc::write_message(&mut window.input, &ToSettings::SaveResult(result));
        }
        if !saved {
            return;
        }
        self.restart_allowed.store(true, Ordering::SeqCst);
        if startup_changed
            && let Err(message) = startup::set_enabled(self.settings.values.start_with_windows)
        {
            let mut corrected = self.settings.values.clone();
            corrected.start_with_windows = startup::is_enabled();
            let mut text = format!("Could not update Windows startup setting:\n\n{message}");
            if let Err(save_error) = self.settings.save(corrected) {
                text.push_str(&format!("\n\nThe corrected setting could not be saved: {save_error}"));
            }
            shell.message(SETTINGS_TITLE, &text, true);
        }
        if trace_changed {
            self.hfp_trace_status = None;
            let _ = self.commands.send(WorkerCommand::SetHfpTrace(self.settings.values.hfp_trace_enabled));
            shell.set_timer(TIMER_CONNECTION, CONNECTION_DEBOUNCE_MS);
        }
        self.first_threshold_level = None;
        self.second_threshold_level = None;
        if self.current.is_some() && self.last_successful.is_some() {
            self.render(shell, true, true, false);
        } else {
            self.push_state();
        }
        shell.kill_timer(TIMER_REFRESH);
        self.refresh_if_due(shell, false);
    }

    fn shut_down(&mut self) {
        if let Some(popup) = self.popup.take() {
            popup.close();
        }
        if let Some(window) = self.settings_window.take() {
            window.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, percent: Option<u8>, cached: bool) -> Device {
        Device { id: format!("id:{name}"), name: name.to_owned(), percent, cached, ..Default::default() }
    }

    fn view(devices: Vec<Device>) -> View {
        let known: Vec<u8> = devices.iter().filter_map(|device| device.percent).collect();
        View {
            lowest: known.iter().copied().min(),
            average: (!known.is_empty())
                .then(|| (known.iter().map(|value| u32::from(*value)).sum::<u32>() / known.len() as u32) as u8),
            devices,
            ..Default::default()
        }
    }

    #[test]
    fn the_tray_icon_follows_the_chosen_metric() {
        let view = view(vec![device("Headset", Some(40), false), device("Mouse", Some(80), false)]);
        let mut settings = Settings::default();
        assert_eq!(icon_content(&view, &settings), IconContent::Percent(40));
        settings.icon_metric = ICON_METRIC_AVERAGE.to_owned();
        assert_eq!(icon_content(&view, &settings), IconContent::Percent(60));
        settings.icon_metric = "device".to_owned();
        settings.icon_device_id = "ID:MOUSE".to_owned();
        assert_eq!(icon_content(&view, &settings), IconContent::Percent(80));
        settings.icon_device_id = "id:gone".to_owned();
        assert_eq!(icon_content(&view, &settings), IconContent::Unknown);
        assert_eq!(icon_content(&View::default(), &Settings::default()), IconContent::Unknown);
    }

    #[test]
    fn tooltips_and_summaries_match_the_original_wording() {
        let view = view(vec![device("Headset\r\nX", Some(40), false), device(" ", None, false)]);
        assert_eq!(native_tooltip(&view), "Headset  X 40%\r\nWireless device ?%");
        assert_eq!(summary_line(view.devices.first()), "Headset\r\nX: 40%");
        assert_eq!(summary_line(view.devices.get(1)), "Wireless device: unavailable");
        assert_eq!(summary_line(None), "No other supported device");
    }

    #[test]
    fn threshold_levels_ignore_cached_readings() {
        let settings = Settings::default();
        assert_eq!(threshold_level(Some(&device("a", Some(15), false)), &settings), 2);
        assert_eq!(threshold_level(Some(&device("a", Some(25), false)), &settings), 1);
        assert_eq!(threshold_level(Some(&device("a", Some(26), false)), &settings), 0);
        assert_eq!(threshold_level(Some(&device("a", Some(5), true)), &settings), 0);
        assert_eq!(threshold_level(Some(&device("a", None, false)), &settings), 2);
        assert_eq!(threshold_level(None, &settings), 0);
    }

    #[test]
    #[ignore = "downloads the latest release from GitHub"]
    fn latest_published_release_downloads_and_verifies() {
        let folder = std::env::temp_dir().join(format!("bbm-rust-update-download-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("BluetoothBatteryMonitor.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &target).unwrap();
        let updater = Updater::new(DEVELOPMENT_VERSION, &target, &folder.join(UPDATE_STATE_FILE));
        let check = updater.manual_check();
        let outcome = check.update.as_ref().map(|update| updater.install_update(update, &|| false));
        let prepared = matches!(outcome, Some(UpdateOutcome::Deferred { .. }));
        let description = format!("{} / {outcome:?}", check.dialog_text);
        drop(outcome);
        let unchanged = std::fs::read(&target).ok() == std::fs::read(std::env::current_exe().unwrap()).ok();
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(check.action, UpdateCheckAction::PromptInstall, "{description}");
        assert!(prepared, "{description}");
        assert!(unchanged);
    }

    #[test]
    fn status_text_is_compacted_and_warnings_are_appended() {
        assert_eq!(compact(Some("  a \r\n b\t c  "), 120), Some("a b c".to_owned()));
        assert_eq!(compact(Some("   "), 120), None);
        assert_eq!(compact(Some("abcdefghij"), 8), Some("abcde...".to_owned()));
        let mut view = View::default();
        append_status(&mut view, RATES_WARNING);
        append_status(&mut view, CACHE_WARNING);
        assert_eq!(view.status_message, Some(format!("{RATES_WARNING} {CACHE_WARNING}")));
    }
}
