use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicPtr, Ordering as AtomicOrdering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::Networking::WinHttp::{
    ERROR_WINHTTP_TIMEOUT, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_FLAG_SECURE,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP, WINHTTP_OPTION_URL,
    WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpQueryOption, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::Win32::Security::Cryptography::{
    BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptGenRandom, BCryptHashData,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::core::{HRESULT, PCWSTR, PWSTR, w};

macro_rules! github_owner {
    () => {
        "jeremygold02"
    };
}

macro_rules! github_repo {
    () => {
        "bluetooth-battery-monitor"
    };
}

macro_rules! identity_prefix {
    () => {
        "BluetoothBatteryMonitorRust.identity{"
    };
}

pub const RELEASE_ASSET_NAME: &str = "BluetoothBatteryMonitorRust.exe";

const GITHUB_OWNER: &str = github_owner!();
const GITHUB_REPO: &str = github_repo!();
const IDENTITY_PREFIX: &str = identity_prefix!();
const IDENTITY_SUFFIX: &str = "}";
const BASE_VERSION: &str = "0.1.0";
const USER_AGENT: &str = "Bluetooth-Battery-Monitor";
const SKIP_STARTUP_UPDATE_ARGUMENT: &str = "--skip-startup-update";
const UPDATE_COMPLETE_ARGUMENT: &str = "--update-complete";
const RELAUNCH_ARGUMENTS: [&str; 2] = [SKIP_STARTUP_UPDATE_ARGUMENT, UPDATE_COMPLETE_ARGUMENT];
const GITHUB_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RELEASE_ASSET_BYTES: u64 = 100 * 1024 * 1024;
const MAX_RELEASE_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STATE_BYTES: u64 = 4096;
const MAX_URL_UNITS: usize = 32 * 1024;
const TRANSFER_BUFFER_BYTES: usize = 81920;
const MACHINE_AMD64: u16 = 0x8664;
const OPTIONAL_HEADER_PE32_PLUS: u16 = 0x020B;
const IMAGE_EXECUTABLE: u16 = 0x0002;
const IMAGE_DLL: u16 = 0x2000;
const CLR_DIRECTORY_INDEX: usize = 14;
const NO_RELEASES_MESSAGE: &str = "No published releases were found.";
const NO_UPDATE_MESSAGE: &str = "No update is available.";
const ROLLBACK_MESSAGE: &str =
    "The previous update failed. The app was restored; check for updates to retry.";
const RESTORE_FAILED_MESSAGE: &str = "The update failed and the previous executable could not be restored. Reinstall the app from the release page.";
const UNREADABLE_METADATA_MESSAGE: &str =
    "The downloaded app metadata could not be verified. No app files were replaced.";

#[used]
static BUILD_IDENTITY: &str = concat!(
    identity_prefix!(),
    github_owner!(),
    "/",
    github_repo!(),
    "@",
    env!("BBM_VERSION"),
    "}"
);

const GITHUB_API: HttpEndpoint = HttpEndpoint {
    host: "api.github.com",
    port: INTERNET_DEFAULT_HTTPS_PORT,
    secure: true,
    proxy: WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
};

const GITHUB_DOWNLOADS: HttpEndpoint = HttpEndpoint {
    host: "github.com",
    port: INTERNET_DEFAULT_HTTPS_PORT,
    secure: true,
    proxy: WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferError {
    TimedOut,
    Failed,
}

pub struct AssetDownload {
    pub final_url: String,
    pub content_length: Option<u64>,
    pub body: Box<dyn Read>,
}

pub trait ReleaseSource {
    fn latest_release(&self) -> Result<Option<Vec<u8>>, TransferError>;
    fn open_asset(&self, url: &str) -> Result<AssetDownload, TransferError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupWatch {
    StillRunning,
    Exited,
}

pub trait ProcessHost {
    fn other_instance_running(&self, executable: &Path) -> bool;
    fn launch_and_watch(
        &self,
        executable: &Path,
        arguments: &[&str],
        watch: Duration,
    ) -> io::Result<StartupWatch>;
}

pub struct GitHubReleaseSource;

pub struct WindowsProcessHost;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub update_available: bool,
    pub can_install: bool,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub asset_sha256: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheckAction {
    None,
    InstallAutomatically,
    NotifyFailure,
    PromptInstall,
    PromptRelease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStage {
    Check,
    Unavailable,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralReason {
    UnsavedSettings,
    OtherInstanceRunning,
}

#[derive(Debug)]
pub enum UpdateOutcome {
    UpToDate { status_text: String },
    InstalledAndRelaunching { version: String },
    Deferred { reason: DeferralReason, pending: PreparedUpdate },
    Failed { stage: FailureStage, message: String },
    RolledBack { message: String },
    PreviouslyRolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualCheck {
    pub status_text: String,
    pub dialog_text: String,
    pub action: UpdateCheckAction,
    pub update: Option<UpdateInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicCheck {
    pub status_text: String,
    pub notification: Option<String>,
}

#[derive(Debug)]
pub struct PreparedUpdate {
    version: String,
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    asset_size: u64,
    asset_sha256: String,
    original_target_hash: String,
    _lock: UpdateLock,
}

pub struct Updater<S = GitHubReleaseSource, H = WindowsProcessHost> {
    current_version: String,
    executable_path: PathBuf,
    failed_state_path: PathBuf,
    download_root: PathBuf,
    timing: SwapTiming,
    reported_notices: Mutex<HashSet<String>>,
    source: S,
    host: H,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateCheckMode {
    Startup,
    Periodic,
    Manual,
}

#[derive(Debug, Clone, Copy)]
struct SwapTiming {
    replace_attempts: u32,
    replace_delay: Duration,
    rollback_attempts: u32,
    rollback_delay: Duration,
    startup_watch: Duration,
}

#[derive(Debug, PartialEq, Eq)]
struct UpdateError(String);

#[derive(Debug)]
enum PrepareError {
    Update(UpdateError),
    Transfer(TransferError),
    Io(io::Error),
}

struct InstallContext {
    target: PathBuf,
    app_dir: PathBuf,
    file_name: String,
}

#[derive(Debug)]
struct UpdateLock {
    file: Option<File>,
    path: PathBuf,
}

struct AssetLocation {
    path_and_query: String,
}

#[derive(Deserialize)]
struct ReleaseDocument {
    #[serde(default)]
    tag_name: Value,
    #[serde(default)]
    assets: Value,
}

#[derive(Deserialize)]
struct AssetDocument {
    #[serde(default)]
    name: Value,
    #[serde(default)]
    browser_download_url: Value,
    #[serde(default)]
    size: Value,
    #[serde(default)]
    digest: Value,
}

#[derive(Serialize, Deserialize)]
struct FailedUpdateState {
    failed_version: String,
}

struct Sha256Hasher {
    handle: BCRYPT_HASH_HANDLE,
}

struct HttpEndpoint {
    host: &'static str,
    port: u16,
    secure: bool,
    proxy: WINHTTP_ACCESS_TYPE,
}

struct HttpHandle(AtomicPtr<c_void>);

struct ResponseDeadline {
    completed: Sender<()>,
    watcher: JoinHandle<bool>,
}

struct HttpResponse {
    request: Arc<HttpHandle>,
    _connection: HttpHandle,
    _session: HttpHandle,
}

pub fn build_version() -> &'static str {
    BUILD_IDENTITY
        .strip_suffix(IDENTITY_SUFFIX)
        .and_then(|identity| identity.rsplit_once('@'))
        .map_or(BASE_VERSION, |(_, version)| version)
}

pub fn release_page_url() -> String {
    format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest")
}

pub fn should_check_for_startup_update(arguments: &[String]) -> bool {
    !has_argument(arguments, SKIP_STARTUP_UPDATE_ARGUMENT)
}

pub fn update_complete_status(arguments: &[String], current_version: &str) -> Option<String> {
    has_argument(arguments, UPDATE_COMPLETE_ARGUMENT)
        .then(|| format!("Updated to version {}.", clean_version(current_version)))
}

pub fn remove_stale_update_files(executable_path: &Path) {
    let Some(context) = install_context(executable_path) else {
        return;
    };
    let prefix = format!(".{}.update-", context.file_name).to_lowercase();
    let stale_files = || -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&context.app_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().to_lowercase().starts_with(&prefix))
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect()
    };
    if stale_files().is_empty() {
        return;
    }
    let Ok(_lock) = UpdateLock::acquire(&context) else {
        return;
    };
    for path in stale_files() {
        let _ = fs::remove_file(path);
    }
}

impl UpdateInfo {
    fn unavailable(message: &str) -> Self {
        Self {
            latest_version: String::new(),
            update_available: false,
            can_install: false,
            asset_name: String::new(),
            asset_url: String::new(),
            asset_size: 0,
            asset_sha256: String::new(),
            message: message.to_owned(),
        }
    }

    fn status_text(&self) -> String {
        if self.update_available && self.can_install {
            format!("Version {} available", self.latest_version)
        } else if self.update_available || self.latest_version.is_empty() {
            self.message.clone()
        } else {
            format!("Up to date: {}", self.latest_version)
        }
    }
}

impl DeferralReason {
    pub fn status_text(self) -> &'static str {
        match self {
            Self::UnsavedSettings => {
                "Update ready; waiting for unsaved settings to be saved or closed."
            }
            Self::OtherInstanceRunning => {
                "Update ready; waiting for other running instances to close."
            }
        }
    }
}

impl UpdateOutcome {
    pub fn status_text(&self) -> String {
        match self {
            Self::UpToDate { status_text } => status_text.clone(),
            Self::InstalledAndRelaunching { version } => format!("Installing version {version}..."),
            Self::Deferred { reason, .. } => reason.status_text().to_owned(),
            Self::Failed { stage: FailureStage::Check, message } => {
                format!("Update check failed: {message}")
            }
            Self::Failed { stage: FailureStage::Unavailable, message } => message.clone(),
            Self::Failed { stage: FailureStage::Install, message } => {
                format!("Update installation failed: {message}")
            }
            Self::RolledBack { message } => message.clone(),
            Self::PreviouslyRolledBack => ROLLBACK_MESSAGE.to_owned(),
        }
    }

    pub fn notification(&self) -> Option<String> {
        match self {
            Self::Failed { stage: FailureStage::Unavailable, message } => {
                Some(unavailable_notification(message))
            }
            Self::Failed { stage: FailureStage::Install, .. } => Some(
                "Automatic update failed. Open Settings and check for updates to retry.".to_owned(),
            ),
            Self::RolledBack { message } => Some(message.clone()),
            _ => None,
        }
    }

    pub fn dialog_text(&self) -> Option<String> {
        match self {
            Self::Failed { stage: FailureStage::Check, message } => {
                Some(format!("Could not check for updates:\r\n\r\n{message}"))
            }
            Self::Failed { stage: FailureStage::Install, message } => {
                Some(format!("Could not install the update:\r\n\r\n{message}"))
            }
            Self::Failed { stage: FailureStage::Unavailable, message }
            | Self::RolledBack { message } => Some(message.clone()),
            _ => None,
        }
    }
}

impl Drop for PreparedUpdate {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.staged);
    }
}

impl SwapTiming {
    const STANDARD: Self = Self {
        replace_attempts: 30,
        replace_delay: Duration::from_secs(1),
        rollback_attempts: 20,
        rollback_delay: Duration::from_millis(500),
        startup_watch: Duration::from_secs(5),
    };
}

impl Updater<GitHubReleaseSource, WindowsProcessHost> {
    pub fn new(current_version: &str, executable_path: &Path, failed_state_path: &Path) -> Self {
        Self {
            current_version: current_version.to_owned(),
            executable_path: executable_path.to_path_buf(),
            failed_state_path: failed_state_path.to_path_buf(),
            download_root: std::env::temp_dir(),
            timing: SwapTiming::STANDARD,
            reported_notices: Mutex::new(HashSet::new()),
            source: GitHubReleaseSource,
            host: WindowsProcessHost,
        }
    }
}

impl<S: ReleaseSource, H: ProcessHost> Updater<S, H> {
    pub fn startup_update(&self, can_restart: &dyn Fn() -> bool) -> UpdateOutcome {
        remove_stale_update_files(&self.executable_path);
        let info = match self.check_for_update() {
            Ok(info) => info,
            Err(error) => {
                return UpdateOutcome::Failed { stage: FailureStage::Check, message: error.0 };
            }
        };
        match decide_update_action(&info, UpdateCheckMode::Startup) {
            UpdateCheckAction::InstallAutomatically => {}
            UpdateCheckAction::NotifyFailure => {
                self.mark_uninstallable_update_notice(&info);
                return UpdateOutcome::Failed {
                    stage: FailureStage::Unavailable,
                    message: info.message,
                };
            }
            _ => {
                return UpdateOutcome::UpToDate { status_text: info.status_text() };
            }
        }
        let previously_rolled_back = read_failed_version(&self.failed_state_path)
            .is_some_and(|failed| compare_versions(&failed, &info.latest_version).is_eq());
        if previously_rolled_back {
            return UpdateOutcome::PreviouslyRolledBack;
        }
        self.install_update(&info, can_restart)
    }

    pub fn periodic_check(&self) -> PeriodicCheck {
        match self.check_for_update() {
            Ok(info) => {
                let notify = decide_update_action(&info, UpdateCheckMode::Periodic)
                    == UpdateCheckAction::NotifyFailure
                    && self.mark_uninstallable_update_notice(&info);
                PeriodicCheck {
                    status_text: info.status_text(),
                    notification: notify.then(|| unavailable_notification(&info.message)),
                }
            }
            Err(error) => PeriodicCheck {
                status_text: format!("Update check failed: {}", error.0),
                notification: None,
            },
        }
    }

    pub fn manual_check(&self) -> ManualCheck {
        match self.check_for_update() {
            Ok(info) => {
                let action = decide_update_action(&info, UpdateCheckMode::Manual);
                let question = match action {
                    UpdateCheckAction::PromptInstall => {
                        "\r\n\r\nInstall now? Bluetooth Battery Monitor will restart automatically."
                    }
                    UpdateCheckAction::PromptRelease => "\r\n\r\nOpen the release page?",
                    _ => "",
                };
                ManualCheck {
                    status_text: info.status_text(),
                    dialog_text: format!("{}{question}", info.message),
                    action,
                    update: Some(info),
                }
            }
            Err(error) => ManualCheck {
                status_text: format!("Update check failed: {}", error.0),
                dialog_text: format!("Could not check for updates:\r\n\r\n{}", error.0),
                action: UpdateCheckAction::None,
                update: None,
            },
        }
    }

    pub fn install_update(
        &self,
        info: &UpdateInfo,
        can_restart: &dyn Fn() -> bool,
    ) -> UpdateOutcome {
        match self.prepare_update(info) {
            Ok(pending) => self.finish_update(pending, can_restart),
            Err(error) => UpdateOutcome::Failed { stage: FailureStage::Install, message: error.0 },
        }
    }

    pub fn finish_update(
        &self,
        pending: PreparedUpdate,
        can_restart: &dyn Fn() -> bool,
    ) -> UpdateOutcome {
        if !can_restart() {
            return UpdateOutcome::Deferred { reason: DeferralReason::UnsavedSettings, pending };
        }
        if self.host.other_instance_running(&pending.target) {
            return UpdateOutcome::Deferred {
                reason: DeferralReason::OtherInstanceRunning,
                pending,
            };
        }
        if let Err(error) = self.replace_installed_executable(&pending) {
            return UpdateOutcome::Failed { stage: FailureStage::Install, message: error.0 };
        }
        let launch = self.host.launch_and_watch(
            &pending.target,
            &RELAUNCH_ARGUMENTS,
            self.timing.startup_watch,
        );
        if matches!(launch, Ok(StartupWatch::StillRunning)) {
            clear_failed_version(&self.failed_state_path);
            return UpdateOutcome::InstalledAndRelaunching { version: pending.version.clone() };
        }
        let _ = record_failed_version(&self.failed_state_path, &pending.version);
        match self.restore_previous_executable(&pending) {
            Ok(()) => UpdateOutcome::RolledBack { message: ROLLBACK_MESSAGE.to_owned() },
            Err(_) => UpdateOutcome::Failed {
                stage: FailureStage::Install,
                message: RESTORE_FAILED_MESSAGE.to_owned(),
            },
        }
    }

    fn check_for_update(&self) -> Result<UpdateInfo, UpdateError> {
        let document = match self.source.latest_release() {
            Ok(Some(document)) => document,
            Ok(None) => return Ok(UpdateInfo::unavailable(NO_RELEASES_MESSAGE)),
            Err(TransferError::TimedOut) => {
                return Err(UpdateError::new(
                    "GitHub took too long to respond. Try again in a moment.",
                ));
            }
            Err(TransferError::Failed) => {
                return Err(UpdateError::new(
                    "Could not check GitHub for updates. Check your connection and try again.",
                ));
            }
        };
        let unreadable = |_| {
            UpdateError::new(
                "GitHub returned update information that could not be read safely. Try again later.",
            )
        };
        let parsed: Value = serde_json::from_slice(&document).map_err(unreadable)?;
        if !parsed.is_object() {
            return Ok(UpdateInfo::unavailable(NO_RELEASES_MESSAGE));
        }
        let release = ReleaseDocument::deserialize(&parsed).map_err(unreadable)?;
        let Some(asset) = release_asset(&release.assets) else {
            return Ok(UpdateInfo::unavailable(NO_UPDATE_MESSAGE));
        };
        let Some(latest_version) = parse_release_version(text(&release.tag_name)) else {
            return Err(UpdateError::new(
                "The latest GitHub release has an invalid version tag. Open the release page to review it manually.",
            ));
        };

        let asset_url = text(&asset.browser_download_url).to_owned();
        let asset_size = whole_number(&asset.size);
        let asset_sha256 = parse_sha256_digest(text(&asset.digest));
        let current_version = clean_version(&self.current_version);
        let update_available = compare_versions(&latest_version, &current_version).is_gt();
        let has_install_asset = !asset_url.trim().is_empty();
        let has_valid_asset_url =
            has_install_asset && parse_release_asset_url(&asset_url).is_some();
        let has_integrity_metadata = asset_size_is_valid(asset_size) && asset_sha256.is_some();
        let install_supported = install_context(&self.executable_path).is_some();
        let can_install =
            update_available && has_valid_asset_url && has_integrity_metadata && install_supported;

        let message = if !update_available {
            format!("You are on the latest published version ({latest_version}).")
        } else if !has_install_asset {
            format!("Update available: {latest_version}, but no Windows exe asset was attached.")
        } else if !has_valid_asset_url {
            format!(
                "Update available: {latest_version}, but its download address is not valid for automatic installation."
            )
        } else if !has_integrity_metadata {
            format!(
                "Update available: {latest_version}, but automatic installation is unavailable because the release has no valid size and SHA-256 metadata."
            )
        } else if !install_supported {
            format!(
                "Update available: {latest_version}. Automatic installation is not available in this copy."
            )
        } else {
            format!("Update available: {latest_version}.")
        };

        Ok(UpdateInfo {
            latest_version,
            update_available,
            can_install,
            asset_name: text(&asset.name).to_owned(),
            asset_url,
            asset_size,
            asset_sha256: asset_sha256.unwrap_or_default(),
            message,
        })
    }

    fn prepare_update(&self, info: &UpdateInfo) -> Result<PreparedUpdate, UpdateError> {
        if !info.update_available {
            return Err(UpdateError::new(NO_UPDATE_MESSAGE));
        }
        let current_version = clean_version(&self.current_version);
        let Some(version) = parse_release_version(&info.latest_version)
            .filter(|version| compare_versions(version, &current_version).is_gt())
        else {
            return Err(UpdateError::new(
                "The update version is invalid or is not newer than this app.",
            ));
        };
        if info.asset_url.trim().is_empty() {
            return Err(UpdateError::new(
                "The latest release does not include a Windows exe asset.",
            ));
        }
        if !info.asset_name.eq_ignore_ascii_case(RELEASE_ASSET_NAME) {
            return Err(UpdateError::new(
                "The release asset name is not valid for automatic installation.",
            ));
        }
        if !asset_size_is_valid(info.asset_size) || !is_sha256_hex(&info.asset_sha256) {
            return Err(UpdateError::new(
                "The release does not include valid integrity metadata. Open the release page to install it manually.",
            ));
        }
        if parse_release_asset_url(&info.asset_url).is_none() {
            return Err(UpdateError::new(
                "The release download address is not valid for automatic installation.",
            ));
        }
        let Some(context) = install_context(&self.executable_path) else {
            return Err(UpdateError::new(
                "Self-update is only available in the packaged Windows app.",
            ));
        };

        let lock = UpdateLock::acquire(&context).map_err(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                prepare_failure(PrepareError::Io(error))
            } else {
                UpdateError::new("Another update is still in progress.")
            }
        })?;
        let update_id = unique_id().map_err(|error| prepare_failure(PrepareError::Io(error)))?;
        verify_install_directory(&context, &update_id)?;

        let staged = context.sibling(&format!(".update-{update_id}.tmp"));
        let download_dir =
            self.download_root.join(format!("bluetooth_battery_monitor_update_{update_id}"));
        let staging = self.stage_release(info, &version, &context.target, &staged, &download_dir);
        let _ = fs::remove_dir_all(&download_dir);
        match staging {
            Ok(original_target_hash) => Ok(PreparedUpdate {
                version,
                target: context.target,
                backup: path_with_suffix(&staged, ".bak"),
                staged,
                asset_size: info.asset_size,
                asset_sha256: info.asset_sha256.clone(),
                original_target_hash,
                _lock: lock,
            }),
            Err(error) => {
                let _ = fs::remove_file(&staged);
                Err(prepare_failure(error))
            }
        }
    }

    fn stage_release(
        &self,
        info: &UpdateInfo,
        version: &str,
        target: &Path,
        staged: &Path,
        download_dir: &Path,
    ) -> Result<String, PrepareError> {
        let original_target_hash = sha256_file(target)?;
        fs::create_dir_all(download_dir)?;
        let downloaded = download_dir.join(format!("{RELEASE_ASSET_NAME}.download"));
        download_file(&self.source, &info.asset_url, &downloaded, info.asset_size)?;
        verify_asset_file(&downloaded, info.asset_size, &info.asset_sha256)?;
        verify_native_release(&downloaded, version)?;
        copy_to_new_file(&downloaded, staged)?;
        verify_asset_file(staged, info.asset_size, &info.asset_sha256)?;
        Ok(original_target_hash)
    }

    fn replace_installed_executable(&self, pending: &PreparedUpdate) -> Result<(), UpdateError> {
        if !pending.staged.is_file() {
            return Err(UpdateError::new("The staged update file is missing."));
        }
        if !pending.target.is_file() {
            return Err(UpdateError::new("The installed executable is missing."));
        }
        verify_asset_file(&pending.staged, pending.asset_size, &pending.asset_sha256)
            .map_err(prepare_failure)?;
        let target_unchanged =
            sha256_file(&pending.target).is_ok_and(|hash| hash == pending.original_target_hash);
        if !target_unchanged {
            return Err(UpdateError::new(
                "The installed executable changed after this update was prepared.",
            ));
        }
        if pending.backup.exists() {
            return Err(UpdateError::new("The update backup path already exists."));
        }

        let attempts = self.timing.replace_attempts;
        let replaced = retry(attempts, self.timing.replace_delay, || {
            swap_files(&pending.target, &pending.staged, &pending.backup)
        });
        if replaced.is_ok() {
            return Ok(());
        }
        if !pending.target.exists() && fs::rename(&pending.backup, &pending.target).is_err() {
            return Err(UpdateError::new(RESTORE_FAILED_MESSAGE));
        }
        Err(UpdateError(format!("Could not replace the app after {attempts} attempts.")))
    }

    fn restore_previous_executable(&self, pending: &PreparedUpdate) -> io::Result<()> {
        let failed = path_with_suffix(&pending.staged, ".failed");
        let attempts = self.timing.rollback_attempts;
        let delay = self.timing.rollback_delay;
        retry(attempts, delay, || restore_files(&pending.target, &pending.backup, &failed))?;
        let _ = retry(attempts, delay, || remove_file_if_present(&failed));
        Ok(())
    }

    fn mark_uninstallable_update_notice(&self, info: &UpdateInfo) -> bool {
        let mut notices = match self.reported_notices.lock() {
            Ok(notices) => notices,
            Err(poisoned) => poisoned.into_inner(),
        };
        try_mark_uninstallable_update_notice(info, &mut notices)
    }
}

impl UpdateError {
    fn new(message: &str) -> Self {
        Self(message.to_owned())
    }
}

impl From<UpdateError> for PrepareError {
    fn from(error: UpdateError) -> Self {
        Self::Update(error)
    }
}

impl From<TransferError> for PrepareError {
    fn from(error: TransferError) -> Self {
        Self::Transfer(error)
    }
}

impl From<io::Error> for PrepareError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl InstallContext {
    fn sibling(&self, suffix: &str) -> PathBuf {
        self.app_dir.join(format!(".{}{suffix}", self.file_name))
    }
}

impl UpdateLock {
    fn acquire(context: &InstallContext) -> io::Result<Self> {
        let path = context.sibling(".update.lock");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)?;
        Ok(Self { file: Some(file), path })
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

impl Sha256Hasher {
    fn new() -> io::Result<Self> {
        let mut handle = BCRYPT_HASH_HANDLE::default();
        unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut handle, None, None, 0) }
            .ok()
            .map_err(io::Error::from)?;
        Ok(Self { handle })
    }

    fn update(&mut self, data: &[u8]) -> io::Result<()> {
        unsafe { BCryptHashData(self.handle, data, 0) }.ok().map_err(io::Error::from)
    }

    fn finish(self) -> io::Result<[u8; 32]> {
        let mut digest = [0u8; 32];
        unsafe { BCryptFinishHash(self.handle, &mut digest, 0) }.ok().map_err(io::Error::from)?;
        Ok(digest)
    }
}

impl Drop for Sha256Hasher {
    fn drop(&mut self) {
        let _ = unsafe { BCryptDestroyHash(self.handle) };
    }
}

impl HttpHandle {
    fn open(handle: *mut c_void) -> Result<Self, TransferError> {
        if handle.is_null() {
            Err(transfer_error(&windows::core::Error::from_thread()))
        } else {
            Ok(Self(AtomicPtr::new(handle)))
        }
    }

    fn raw(&self) -> *mut c_void {
        self.0.load(AtomicOrdering::SeqCst)
    }

    fn close(&self) {
        let handle = self.0.swap(std::ptr::null_mut(), AtomicOrdering::SeqCst);
        if !handle.is_null() {
            let _ = unsafe { WinHttpCloseHandle(handle) };
        }
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        self.close();
    }
}

impl ResponseDeadline {
    fn start(request: Arc<HttpHandle>, timeout: Duration) -> io::Result<Self> {
        let (completed, completion) = mpsc::channel::<()>();
        let watcher = std::thread::Builder::new().spawn(move || {
            let expired = completion.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout);
            if expired {
                request.close();
            }
            expired
        })?;
        Ok(Self { completed, watcher })
    }

    fn expired(self) -> bool {
        drop(self.completed);
        self.watcher.join().unwrap_or(false)
    }
}

impl HttpResponse {
    fn status_code(&self) -> Result<u32, TransferError> {
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        unsafe {
            WinHttpQueryHeaders(
                self.request.raw(),
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&raw mut status).cast()),
                &mut size,
                std::ptr::null_mut(),
            )
        }
        .map_err(|error| transfer_error(&error))?;
        Ok(status)
    }

    fn content_length(&self) -> Option<u64> {
        let mut value = [0u16; 32];
        let mut size = size_of_val(&value) as u32;
        unsafe {
            WinHttpQueryHeaders(
                self.request.raw(),
                WINHTTP_QUERY_CONTENT_LENGTH,
                PCWSTR::null(),
                Some(value.as_mut_ptr().cast()),
                &mut size,
                std::ptr::null_mut(),
            )
        }
        .ok()?;
        text_until_nul(&value).trim().parse().ok()
    }

    fn final_url(&self) -> String {
        let request = self.request.raw();
        let mut size = 0u32;
        let _ = unsafe { WinHttpQueryOption(request, WINHTTP_OPTION_URL, None, &mut size) };
        let mut value = vec![0u16; (size as usize).div_ceil(2).min(MAX_URL_UNITS) + 1];
        let mut capacity = u32::try_from(value.len() * 2).unwrap_or(u32::MAX);
        let queried = unsafe {
            WinHttpQueryOption(
                request,
                WINHTTP_OPTION_URL,
                Some(value.as_mut_ptr().cast()),
                &mut capacity,
            )
        };
        if queried.is_ok() { text_until_nul(&value) } else { String::new() }
    }
}

impl Read for HttpResponse {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let wanted = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let mut received = 0u32;
        unsafe {
            WinHttpReadData(self.request.raw(), buffer.as_mut_ptr().cast(), wanted, &mut received)
        }
        .map_err(|error| match transfer_error(&error) {
            TransferError::TimedOut => io::ErrorKind::TimedOut.into(),
            TransferError::Failed => io::Error::from(error),
        })?;
        Ok(received as usize)
    }
}

impl ReleaseSource for GitHubReleaseSource {
    fn latest_release(&self) -> Result<Option<Vec<u8>>, TransferError> {
        fetch_latest_release(&GITHUB_API, GITHUB_TIMEOUT)
    }

    fn open_asset(&self, url: &str) -> Result<AssetDownload, TransferError> {
        let location = parse_release_asset_url(url).ok_or(TransferError::Failed)?;
        open_download(&GITHUB_DOWNLOADS, &location.path_and_query, DOWNLOAD_TIMEOUT)
    }
}

impl ProcessHost for WindowsProcessHost {
    fn other_instance_running(&self, executable: &Path) -> bool {
        instance_running_outside_family(executable, std::process::id())
    }

    fn launch_and_watch(
        &self,
        executable: &Path,
        arguments: &[&str],
        watch: Duration,
    ) -> io::Result<StartupWatch> {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW.0);
        if let Some(app_dir) = executable.parent() {
            command.current_dir(app_dir);
        }
        let mut child = command.spawn()?;
        let deadline = Instant::now() + watch;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(StartupWatch::Exited),
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(StartupWatch::StillRunning);
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

fn has_argument(arguments: &[String], expected: &str) -> bool {
    arguments.iter().any(|argument| argument.eq_ignore_ascii_case(expected))
}

fn decide_update_action(info: &UpdateInfo, mode: UpdateCheckMode) -> UpdateCheckAction {
    if !info.update_available {
        return UpdateCheckAction::None;
    }
    match mode {
        UpdateCheckMode::Manual if info.can_install => UpdateCheckAction::PromptInstall,
        UpdateCheckMode::Manual => UpdateCheckAction::PromptRelease,
        _ if !info.can_install => UpdateCheckAction::NotifyFailure,
        UpdateCheckMode::Startup => UpdateCheckAction::InstallAutomatically,
        UpdateCheckMode::Periodic => UpdateCheckAction::None,
    }
}

fn try_mark_uninstallable_update_notice(info: &UpdateInfo, notices: &mut HashSet<String>) -> bool {
    if !info.update_available || info.can_install {
        return false;
    }
    notices.insert(format!("{}\n{}", info.latest_version, info.message))
}

fn unavailable_notification(message: &str) -> String {
    if message.trim().is_empty() {
        "The update could not be installed automatically. Open Settings for details.".to_owned()
    } else {
        message.to_owned()
    }
}

fn clean_version(value: &str) -> String {
    let bytes = value.as_bytes();
    let Some(start) = bytes.iter().position(u8::is_ascii_digit) else {
        return BASE_VERSION.to_owned();
    };
    let digits_end =
        |from: usize| from + bytes[from..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    let mut end = digits_end(start);
    for _ in 0..2 {
        let next_part_end = digits_end((end + 1).min(bytes.len()));
        if bytes.get(end) != Some(&b'.') || next_part_end == end + 1 {
            break;
        }
        end = next_part_end;
    }
    value[start..end].to_owned()
}

fn version_parts(value: &str) -> [i32; 3] {
    let mut parts = [0; 3];
    for (part, text) in parts.iter_mut().zip(clean_version(value).split('.')) {
        *part = text.parse().unwrap_or(0);
    }
    parts
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    version_parts(left).cmp(&version_parts(right))
}

fn parse_release_version(tag: &str) -> Option<String> {
    let value = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = value.split('.').collect();
    let part_is_valid = |part: &&str| {
        !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'))
            && part.parse::<i32>().is_ok()
    };
    (parts.len() == 3 && parts.iter().all(part_is_valid)).then(|| value.to_owned())
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn whole_number(value: &Value) -> u64 {
    const LARGEST_EXACT_FLOAT: f64 = 9_007_199_254_740_992.0;
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| {
                number
                    .as_f64()
                    .filter(|float| {
                        float.fract() == 0.0 && (0.0..=LARGEST_EXACT_FLOAT).contains(float)
                    })
                    .map(|float| float as u64)
            })
            .unwrap_or(0),
        Value::String(digits) => digits.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn release_asset(assets: &Value) -> Option<AssetDocument> {
    assets
        .as_array()?
        .iter()
        .filter(|item| item.is_object())
        .filter_map(|item| AssetDocument::deserialize(item).ok())
        .find(|asset| text(&asset.name).eq_ignore_ascii_case(RELEASE_ASSET_NAME))
}

fn asset_size_is_valid(size: u64) -> bool {
    size > 0 && size <= MAX_RELEASE_ASSET_BYTES
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_sha256_digest(digest: &str) -> Option<String> {
    const PREFIX: &str = "sha256:";
    let value = digest.trim();
    let prefix = value.get(..PREFIX.len())?;
    let hash = &value[PREFIX.len()..];
    (prefix.eq_ignore_ascii_case(PREFIX) && is_sha256_hex(hash)).then(|| hash.to_ascii_lowercase())
}

fn parse_release_asset_url(value: &str) -> Option<AssetLocation> {
    let is_plain = value.bytes().all(|byte| byte.is_ascii_graphic());
    if !is_plain || value.contains(['\\', '#']) {
        return None;
    }
    let (scheme, rest) = value.split_once("://")?;
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path_and_query) = rest.split_at(authority_end);
    let host = authority.strip_suffix(":443").unwrap_or(authority);
    if !scheme.eq_ignore_ascii_case("https") || !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let path = path_and_query.split('?').next().unwrap_or("");
    let expected_prefix = format!("/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/");
    let has_expected_prefix = path
        .get(..expected_prefix.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&expected_prefix));
    let has_dot_segment = path.split('/').any(|segment| {
        let decoded = segment.to_ascii_lowercase().replace("%2e", ".");
        decoded == "." || decoded == ".."
    });
    (has_expected_prefix && !has_dot_segment)
        .then(|| AssetLocation { path_and_query: path_and_query.to_owned() })
}

fn install_context(executable_path: &Path) -> Option<InstallContext> {
    let target = std::path::absolute(executable_path).ok()?;
    let metadata = fs::metadata(&target).ok()?;
    let is_executable =
        target.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !metadata.is_file() || metadata.permissions().readonly() || !is_executable {
        return None;
    }
    let app_dir = target.parent().filter(|directory| directory.is_dir())?.to_path_buf();
    let file_name = target.file_name()?.to_str()?.to_owned();
    Some(InstallContext { target, app_dir, file_name })
}

fn verify_install_directory(context: &InstallContext, probe_id: &str) -> Result<(), UpdateError> {
    let probe = |role: &str| context.sibling(&format!(".update-write-{probe_id}.{role}"));
    let probe_target = probe("target");
    let probe_replacement = probe("replacement");
    let probe_backup = probe("backup");
    let replace_probe = || -> io::Result<bool> {
        write_new_file(&probe_target, &[0])?;
        write_new_file(&probe_replacement, &[1])?;
        swap_files(&probe_target, &probe_replacement, &probe_backup)?;
        Ok(probe_target.is_file() && probe_backup.is_file())
    };
    let replaced = replace_probe();
    let probes = [&probe_target, &probe_replacement, &probe_backup];
    for path in probes {
        let _ = fs::remove_file(path);
    }
    if !matches!(replaced, Ok(true)) {
        return Err(UpdateError::new(
            "The app folder does not support safe automatic replacement. Install the release manually instead.",
        ));
    }
    if probes.iter().any(|path| path.exists()) {
        return Err(UpdateError::new(
            "The app folder could not clean up an update preflight file. Automatic installation was stopped.",
        ));
    }
    Ok(())
}

fn download_file(
    source: &impl ReleaseSource,
    url: &str,
    target: &Path,
    expected_size: u64,
) -> Result<(), PrepareError> {
    let mut download = source.open_asset(url)?;
    let is_secure =
        download.final_url.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    if !is_secure {
        return Err(UpdateError::new(
            "The update download was redirected to an insecure address. No files were replaced.",
        )
        .into());
    }
    if download.content_length.is_some_and(|length| length != expected_size) {
        return Err(UpdateError::new(
            "The update download size did not match the release metadata. No files were replaced.",
        )
        .into());
    }

    let mut destination = OpenOptions::new().write(true).create_new(true).open(target)?;
    let mut buffer = vec![0u8; TRANSFER_BUFFER_BYTES];
    let mut total_bytes = 0u64;
    loop {
        let bytes_read = match download.body.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(read_failure(&error).into()),
        };
        if bytes_read as u64 > expected_size - total_bytes {
            return Err(UpdateError::new(
                "The update download exceeded the expected size. No files were replaced.",
            )
            .into());
        }
        destination.write_all(&buffer[..bytes_read])?;
        total_bytes += bytes_read as u64;
    }
    destination.flush()?;
    if total_bytes != expected_size {
        return Err(UpdateError::new(
            "The update download was incomplete. No files were replaced.",
        )
        .into());
    }
    Ok(())
}

fn read_failure(error: &io::Error) -> TransferError {
    if error.kind() == io::ErrorKind::TimedOut {
        TransferError::TimedOut
    } else {
        TransferError::Failed
    }
}

fn verify_asset_file(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), PrepareError> {
    let has_expected_size = fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == expected_size);
    if !has_expected_size {
        return Err(UpdateError::new(
            "The update file size failed verification. No files were replaced.",
        )
        .into());
    }
    if !sha256_file(path)?.eq_ignore_ascii_case(expected_sha256) {
        return Err(UpdateError::new(
            "The update failed SHA-256 verification. No files were replaced.",
        )
        .into());
    }
    Ok(())
}

fn verify_native_release(path: &Path, expected_version: &str) -> Result<(), UpdateError> {
    let Some(version) = parse_release_version(expected_version) else {
        return Err(UpdateError::new("The release version is invalid."));
    };
    let image = fs::read(path).map_err(|_| UpdateError::new(UNREADABLE_METADATA_MESSAGE))?;
    if !image.starts_with(b"MZ") {
        return Err(UpdateError::new(
            "The verified release asset is not a Windows executable. No files were replaced.",
        ));
    }
    if !is_native_x64_executable(&image).unwrap_or(false) {
        return Err(UpdateError::new(UNREADABLE_METADATA_MESSAGE));
    }
    let identity = release_identity(&version);
    let has_identity = image.windows(identity.len()).any(|window| window == identity.as_bytes());
    if !has_identity {
        return Err(UpdateError::new(
            "The downloaded app identity or version does not match the GitHub release.",
        ));
    }
    Ok(())
}

fn release_identity(version: &str) -> String {
    format!("{IDENTITY_PREFIX}{GITHUB_OWNER}/{GITHUB_REPO}@{version}{IDENTITY_SUFFIX}")
}

fn is_native_x64_executable(image: &[u8]) -> Option<bool> {
    let signature_offset = read_u32(image, 0x3C)? as usize;
    let signature = image.get(signature_offset..signature_offset.checked_add(4)?)?;
    let file_header = signature_offset.checked_add(4)?;
    let machine = read_u16(image, file_header)?;
    let section_count = usize::from(read_u16(image, file_header.checked_add(2)?)?);
    let optional_header_size = usize::from(read_u16(image, file_header.checked_add(16)?)?);
    let characteristics = read_u16(image, file_header.checked_add(18)?)?;
    let optional_header = file_header.checked_add(20)?;
    let optional_magic = read_u16(image, optional_header)?;
    let is_native_image = signature == b"PE\0\0"
        && machine == MACHINE_AMD64
        && optional_magic == OPTIONAL_HEADER_PE32_PLUS
        && characteristics & IMAGE_EXECUTABLE != 0
        && characteristics & IMAGE_DLL == 0
        && section_count > 0;
    if !is_native_image {
        return Some(false);
    }

    let directory_count = read_u32(image, optional_header.checked_add(108)?)? as usize;
    if directory_count > CLR_DIRECTORY_INDEX {
        let clr_directory = optional_header.checked_add(112 + CLR_DIRECTORY_INDEX * 8)?;
        let clr_address = read_u32(image, clr_directory)?;
        let clr_size = read_u32(image, clr_directory.checked_add(4)?)?;
        if clr_address != 0 || clr_size != 0 {
            return Some(false);
        }
    }

    let section_table = optional_header.checked_add(optional_header_size)?;
    for index in 0..section_count {
        let section = section_table.checked_add(index.checked_mul(40)?)?;
        let raw_size = u64::from(read_u32(image, section.checked_add(16)?)?);
        let raw_offset = u64::from(read_u32(image, section.checked_add(20)?)?);
        if raw_size != 0 && raw_offset + raw_size > image.len() as u64 {
            return Some(false);
        }
    }
    Some(true)
}

fn read_u16(image: &[u8], offset: usize) -> Option<u16> {
    let bytes = image.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32(image: &[u8], offset: usize) -> Option<u32> {
    let bytes = image.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn prepare_failure(error: PrepareError) -> UpdateError {
    match error {
        PrepareError::Update(error) => error,
        PrepareError::Transfer(TransferError::TimedOut) => {
            UpdateError::new("The update download timed out. Try again in a moment.")
        }
        PrepareError::Transfer(TransferError::Failed) => UpdateError::new(
            "The update could not be downloaded securely. Check your connection and try again.",
        ),
        PrepareError::Io(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            UpdateError::new(
                "The app folder does not allow automatic updates. Install the release manually instead.",
            )
        }
        PrepareError::Io(_) => {
            UpdateError::new("The update could not be prepared safely. No app files were replaced.")
        }
    }
}

fn swap_files(target: &Path, replacement: &Path, backup: &Path) -> io::Result<()> {
    if target.exists() {
        fs::rename(target, backup)?;
    }
    fs::rename(replacement, target).inspect_err(|_| {
        let _ = fs::rename(backup, target);
    })
}

fn restore_files(target: &Path, backup: &Path, failed: &Path) -> io::Result<()> {
    if !backup.is_file() {
        return Err(io::ErrorKind::NotFound.into());
    }
    if target.exists() {
        fs::rename(target, failed)?;
    }
    fs::rename(backup, target)
}

fn retry(
    attempts: u32,
    delay: Duration,
    mut operation: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let mut result = operation();
    for _ in 1..attempts {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(delay);
        result = operation();
    }
    result
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_new_file(path: &Path, content: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(content)?;
    file.flush()
}

fn copy_to_new_file(source: &Path, destination: &Path) -> io::Result<()> {
    let mut reader = File::open(source)?;
    let mut writer = OpenOptions::new().write(true).create_new(true).open(destination)?;
    io::copy(&mut reader, &mut writer)?;
    writer.flush()
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn read_failed_version(state_path: &Path) -> Option<String> {
    let metadata = fs::metadata(state_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_STATE_BYTES {
        return None;
    }
    let state: FailedUpdateState = serde_json::from_slice(&fs::read(state_path).ok()?).ok()?;
    parse_release_version(&state.failed_version)
}

fn record_failed_version(state_path: &Path, version: &str) -> io::Result<()> {
    if let Some(directory) = state_path.parent() {
        fs::create_dir_all(directory)?;
    }
    let state = FailedUpdateState { failed_version: version.to_owned() };
    let document = serde_json::to_vec(&state).map_err(io::Error::other)?;
    let temporary = path_with_suffix(state_path, ".tmp");
    fs::write(&temporary, document)?;
    fs::rename(&temporary, state_path)
}

fn clear_failed_version(state_path: &Path) {
    let _ = fs::remove_file(state_path);
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256Hasher::new()?;
    let mut buffer = vec![0u8; TRANSFER_BUFFER_BYTES];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => hasher.update(&buffer[..bytes_read])?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(hex(&hasher.finish()?))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unique_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .map_err(io::Error::from)?;
    Ok(hex(&bytes))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn text_until_nul(units: &[u16]) -> String {
    let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn transfer_error(error: &windows::core::Error) -> TransferError {
    if error.code() == HRESULT::from_win32(ERROR_WINHTTP_TIMEOUT) {
        TransferError::TimedOut
    } else {
        TransferError::Failed
    }
}

fn http_get(
    endpoint: &HttpEndpoint,
    path: &str,
    headers: &str,
    timeout: Duration,
) -> Result<(HttpResponse, ResponseDeadline), TransferError> {
    let agent = wide(USER_AGENT);
    let host = wide(endpoint.host);
    let object = wide(path);
    let header_block: Vec<u16> = headers.encode_utf16().collect();
    let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let flags = if endpoint.secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
    let failed = |error: windows::core::Error| transfer_error(&error);
    unsafe {
        let session = HttpHandle::open(WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            endpoint.proxy,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.raw(), milliseconds, milliseconds, milliseconds, milliseconds)
            .map_err(failed)?;
        let connection = HttpHandle::open(WinHttpConnect(
            session.raw(),
            PCWSTR(host.as_ptr()),
            endpoint.port,
            0,
        ))?;
        let request = Arc::new(HttpHandle::open(WinHttpOpenRequest(
            connection.raw(),
            w!("GET"),
            PCWSTR(object.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ))?);
        WinHttpSetOption(
            Some(request.raw().cast_const()),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP.to_ne_bytes()),
        )
        .map_err(failed)?;
        WinHttpAddRequestHeaders(request.raw(), &header_block, WINHTTP_ADDREQ_FLAG_ADD)
            .map_err(failed)?;
        let deadline = ResponseDeadline::start(Arc::clone(&request), timeout)
            .map_err(|_| TransferError::Failed)?;
        let exchanged = WinHttpSendRequest(request.raw(), None, None, 0, 0, 0)
            .and_then(|()| WinHttpReceiveResponse(request.raw(), std::ptr::null_mut()));
        if let Err(error) = exchanged {
            return Err(if deadline.expired() { TransferError::TimedOut } else { failed(error) });
        }
        Ok((HttpResponse { request, _connection: connection, _session: session }, deadline))
    }
}

fn fetch_latest_release(
    endpoint: &HttpEndpoint,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, TransferError> {
    let path = format!("/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    let headers = "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n";
    let (mut response, deadline) = http_get(endpoint, &path, headers, timeout)?;
    let fetched = read_release_document(&mut response);
    if deadline.expired() { Err(TransferError::TimedOut) } else { fetched }
}

fn read_release_document(response: &mut HttpResponse) -> Result<Option<Vec<u8>>, TransferError> {
    match response.status_code()? {
        404 => Ok(None),
        200..=299 => {
            let mut document = Vec::new();
            response
                .take(MAX_RELEASE_DOCUMENT_BYTES)
                .read_to_end(&mut document)
                .map_err(|error| read_failure(&error))?;
            Ok(Some(document))
        }
        _ => Err(TransferError::Failed),
    }
}

fn open_download(
    endpoint: &HttpEndpoint,
    path: &str,
    timeout: Duration,
) -> Result<AssetDownload, TransferError> {
    let headers = "Accept: application/octet-stream\r\n";
    let (response, deadline) = http_get(endpoint, path, headers, timeout)?;
    if deadline.expired() {
        return Err(TransferError::TimedOut);
    }
    if !(200..=299).contains(&response.status_code()?) {
        return Err(TransferError::Failed);
    }
    Ok(AssetDownload {
        final_url: response.final_url(),
        content_length: response.content_length(),
        body: Box::new(response),
    })
}

fn comparable_path(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    absolute.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase()
}

fn instance_running_outside_family(executable: &Path, family_process_id: u32) -> bool {
    let Some(file_name) = executable.file_name() else {
        return true;
    };
    let file_name = file_name.to_string_lossy().to_lowercase();
    let image_path = comparable_path(executable);
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return true;
    };
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.0) };
    let snapshot_handle = HANDLE(snapshot.as_raw_handle());
    let mut entry =
        PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut has_entry = unsafe { Process32FirstW(snapshot_handle, &mut entry) }.is_ok();
    while has_entry {
        let outside_family = entry.th32ProcessID != family_process_id
            && entry.th32ParentProcessID != family_process_id;
        if outside_family
            && text_until_nul(&entry.szExeFile).to_lowercase() == file_name
            && process_runs_image(entry.th32ProcessID, &image_path)
        {
            return true;
        }
        has_entry = unsafe { Process32NextW(snapshot_handle, &mut entry) }.is_ok();
    }
    false
}

fn process_runs_image(process_id: u32, image_path: &str) -> bool {
    let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
    {
        Ok(process) => unsafe { OwnedHandle::from_raw_handle(process.0) },
        Err(error) => return error.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0),
    };
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            HANDLE(process.as_raw_handle()),
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    };
    if queried.is_err() {
        return true;
    }
    let length = (length as usize).min(path.len());
    comparable_path(Path::new(&String::from_utf16_lossy(&path[..length]))) == image_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::io::Cursor;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::process::Child;
    use windows::Win32::Networking::WinHttp::WINHTTP_ACCESS_TYPE_NO_PROXY;

    const ORIGINAL_APP: &[u8] = b"installed application image";
    const INSTALL_FAILURE_NOTICE: &str =
        "Automatic update failed. Open Settings and check for updates to retry.";
    const FIXTURE_HOLD_FILE: &str = "fixture-hold";
    const FIXTURE_ARGUMENTS: [&str; 2] =
        ["fixture_process_waits_while_hold_file_exists", "--ignored"];
    const EXISTING_RELEASE_JSON: &str = r###"{
  "url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/251873964",
  "assets_url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/251873964/assets",
  "upload_url": "https://uploads.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/251873964/assets{?name,label}",
  "html_url": "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/tag/0.1.57",
  "id": 251873964,
  "author": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot", "site_admin": false },
  "node_id": "RE_kwDOQfixture4PA0ms",
  "tag_name": "0.1.57",
  "target_commitish": "4f0c2a9d6f1e4b7a8c3d2e1f0a9b8c7d6e5f4a3b",
  "name": "0.1.57",
  "draft": false,
  "immutable": false,
  "prerelease": false,
  "created_at": "2026-11-02T18:04:11Z",
  "updated_at": "2026-11-02T18:09:37Z",
  "published_at": "2026-11-02T18:09:37Z",
  "assets": [
    {
      "url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/assets/310045571",
      "id": 310045571,
      "node_id": "RA_kwDOQfixture4Seuwj",
      "name": "BluetoothBatteryMonitor.exe",
      "label": "",
      "uploader": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot", "site_admin": false },
      "content_type": "application/x-msdownload",
      "state": "uploaded",
      "size": 4698112,
      "digest": "sha256:5d9c1f0f3a6f4f2b8a1e7c6d5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a3921",
      "download_count": 12,
      "created_at": "2026-11-02T18:09:35Z",
      "updated_at": "2026-11-02T18:09:36Z",
      "browser_download_url": "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.1.57/BluetoothBatteryMonitor.exe"
    },
    {
      "url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/assets/310045572",
      "id": 310045572,
      "node_id": "RA_kwDOQfixture4Seuwk",
      "name": "BluetoothBatteryMonitor-Setup.exe",
      "label": "",
      "uploader": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot", "site_admin": false },
      "content_type": "application/x-msdownload",
      "state": "uploaded",
      "size": 3391488,
      "digest": "sha256:0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9",
      "download_count": 31,
      "created_at": "2026-11-02T18:09:36Z",
      "updated_at": "2026-11-02T18:09:36Z",
      "browser_download_url": "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.1.57/BluetoothBatteryMonitor-Setup.exe"
    },
    {
      "url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/assets/310045573",
      "id": 310045573,
      "node_id": "RA_kwDOQfixture4Seuwl",
      "name": "ThirdPartyNotices.zip",
      "label": "",
      "uploader": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot", "site_admin": false },
      "content_type": "application/zip",
      "state": "uploaded",
      "size": 5120,
      "digest": "sha256:9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0",
      "download_count": 2,
      "created_at": "2026-11-02T18:09:37Z",
      "updated_at": "2026-11-02T18:09:37Z",
      "browser_download_url": "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.1.57/ThirdPartyNotices.zip"
    }
  ],
  "tarball_url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/tarball/0.1.57",
  "zipball_url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/zipball/0.1.57",
  "body": "## Changes\n\n- fix: keep the tray tooltip current (4f0c2a9)\n\n## Download\n- `BluetoothBatteryMonitor-Setup.exe` installs the Windows tray app.\n- `BluetoothBatteryMonitor.exe` is the portable Windows tray app.\n- `ThirdPartyNotices.zip` contains the included third-party license notices."
}"###;

    struct Sandbox {
        root: PathBuf,
    }

    struct FakeSource {
        release: RefCell<Result<Option<Vec<u8>>, TransferError>>,
        asset: RefCell<Vec<u8>>,
        declared_length: Cell<Option<u64>>,
        final_url: RefCell<String>,
        open_failure: Cell<Option<TransferError>>,
        body_failure: Cell<Option<io::ErrorKind>>,
        checks: Cell<u32>,
        downloads: Cell<u32>,
    }

    struct FakeBody {
        data: Cursor<Vec<u8>>,
        failure: Option<io::ErrorKind>,
    }

    struct Launch {
        executable: PathBuf,
        arguments: Vec<String>,
        watch: Duration,
        image: Vec<u8>,
    }

    struct FakeHost {
        other_instance: Cell<bool>,
        startup: Cell<Result<StartupWatch, io::ErrorKind>>,
        launches: RefCell<Vec<Launch>>,
    }

    struct FixtureHost {
        keep_running: bool,
        requested_arguments: RefCell<Vec<Vec<String>>>,
    }

    struct LoopbackSource {
        endpoint: HttpEndpoint,
    }

    struct LoopbackServer {
        endpoint: HttpEndpoint,
        release: Sender<()>,
        requests: std::thread::JoinHandle<Vec<String>>,
    }

    type MetadataCase = (&'static str, fn(&mut Value), &'static str);

    type DownloadCase = (&'static str, Vec<u8>, Vec<u8>, fn(&FakeSource), &'static str);

    enum Reply {
        Send(Vec<u8>),
        Stall(Vec<u8>),
        Drip(Vec<u8>, Vec<u8>),
    }

    impl LoopbackServer {
        fn finish(self) -> Vec<String> {
            drop(self.release);
            self.requests.join().unwrap()
        }
    }

    impl Sandbox {
        fn new() -> Self {
            let temp_root = std::env::temp_dir();
            let root =
                temp_root.join(format!("BluetoothUpdaterRustTests-{}", unique_id().unwrap()));
            assert_eq!(root.parent(), Some(temp_root.as_path()));
            fs::create_dir_all(root.join("app")).unwrap();
            fs::create_dir_all(root.join("downloads")).unwrap();
            Self { root }
        }

        fn app_dir(&self) -> PathBuf {
            self.root.join("app")
        }

        fn target(&self) -> PathBuf {
            self.app_dir().join(RELEASE_ASSET_NAME)
        }

        fn state_path(&self) -> PathBuf {
            self.root.join("state").join("update-state.json")
        }

        fn download_root(&self) -> PathBuf {
            self.root.join("downloads")
        }

        fn install(&self, image: &[u8]) -> PathBuf {
            fs::write(self.target(), image).unwrap();
            self.target()
        }

        fn app_files(&self) -> Vec<String> {
            file_names(&self.app_dir())
        }

        fn updater<S: ReleaseSource, H: ProcessHost>(
            &self,
            current_version: &str,
            source: S,
            host: H,
        ) -> Updater<S, H> {
            Updater {
                current_version: current_version.to_owned(),
                executable_path: self.target(),
                failed_state_path: self.state_path(),
                download_root: self.download_root(),
                timing: SwapTiming {
                    replace_attempts: 3,
                    replace_delay: Duration::from_millis(20),
                    rollback_attempts: 40,
                    rollback_delay: Duration::from_millis(25),
                    startup_watch: Duration::from_millis(300),
                },
                reported_notices: Mutex::new(HashSet::new()),
                source,
                host,
            }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            for _ in 0..100 {
                if fs::remove_dir_all(&self.root).is_ok() || !self.root.exists() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    impl FakeSource {
        fn new(release: Result<Option<Vec<u8>>, TransferError>, asset: &[u8]) -> Self {
            Self {
                release: RefCell::new(release),
                asset: RefCell::new(asset.to_vec()),
                declared_length: Cell::new(Some(asset.len() as u64)),
                final_url: RefCell::new(
                    "https://release-assets.githubusercontent.com/github-production-release-asset/1"
                        .to_owned(),
                ),
                open_failure: Cell::new(None),
                body_failure: Cell::new(None),
                checks: Cell::new(0),
                downloads: Cell::new(0),
            }
        }

        fn publishing(version: &str, image: &[u8]) -> Self {
            Self::new(Ok(Some(rust_release(version, image))), image)
        }

        fn publish(&self, version: &str, image: &[u8]) {
            *self.release.borrow_mut() = Ok(Some(rust_release(version, image)));
            *self.asset.borrow_mut() = image.to_vec();
            self.declared_length.set(Some(image.len() as u64));
        }
    }

    impl ReleaseSource for FakeSource {
        fn latest_release(&self) -> Result<Option<Vec<u8>>, TransferError> {
            self.checks.set(self.checks.get() + 1);
            self.release.borrow().clone()
        }

        fn open_asset(&self, _url: &str) -> Result<AssetDownload, TransferError> {
            self.downloads.set(self.downloads.get() + 1);
            if let Some(failure) = self.open_failure.get() {
                return Err(failure);
            }
            Ok(AssetDownload {
                final_url: self.final_url.borrow().clone(),
                content_length: self.declared_length.get(),
                body: Box::new(FakeBody {
                    data: Cursor::new(self.asset.borrow().clone()),
                    failure: self.body_failure.get(),
                }),
            })
        }
    }

    impl Read for FakeBody {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let bytes_read = self.data.read(buffer)?;
            match self.failure {
                Some(kind) if bytes_read == 0 => Err(kind.into()),
                _ => Ok(bytes_read),
            }
        }
    }

    impl FakeHost {
        fn new(startup: Result<StartupWatch, io::ErrorKind>) -> Self {
            Self {
                other_instance: Cell::new(false),
                startup: Cell::new(startup),
                launches: RefCell::new(Vec::new()),
            }
        }

        fn staying_up() -> Self {
            Self::new(Ok(StartupWatch::StillRunning))
        }
    }

    impl ProcessHost for FakeHost {
        fn other_instance_running(&self, _executable: &Path) -> bool {
            self.other_instance.get()
        }

        fn launch_and_watch(
            &self,
            executable: &Path,
            arguments: &[&str],
            watch: Duration,
        ) -> io::Result<StartupWatch> {
            self.launches.borrow_mut().push(Launch {
                executable: executable.to_path_buf(),
                arguments: arguments.iter().map(|argument| (*argument).to_owned()).collect(),
                watch,
                image: fs::read(executable).unwrap(),
            });
            self.startup.get().map_err(io::Error::from)
        }
    }

    impl ProcessHost for FixtureHost {
        fn other_instance_running(&self, executable: &Path) -> bool {
            WindowsProcessHost.other_instance_running(executable)
        }

        fn launch_and_watch(
            &self,
            executable: &Path,
            arguments: &[&str],
            watch: Duration,
        ) -> io::Result<StartupWatch> {
            self.requested_arguments
                .borrow_mut()
                .push(arguments.iter().map(|argument| (*argument).to_owned()).collect());
            let used_arguments = if self.keep_running { &FIXTURE_ARGUMENTS[..] } else { arguments };
            WindowsProcessHost.launch_and_watch(executable, used_arguments, watch)
        }
    }

    impl ReleaseSource for LoopbackSource {
        fn latest_release(&self) -> Result<Option<Vec<u8>>, TransferError> {
            fetch_latest_release(&self.endpoint, Duration::from_secs(10))
        }

        fn open_asset(&self, url: &str) -> Result<AssetDownload, TransferError> {
            let location = parse_release_asset_url(url).ok_or(TransferError::Failed)?;
            open_download(&self.endpoint, &location.path_and_query, Duration::from_secs(10))
        }
    }

    fn file_names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn sha256_bytes(data: &[u8]) -> String {
        let mut hasher = Sha256Hasher::new().unwrap();
        hasher.update(data).unwrap();
        hex(&hasher.finish().unwrap())
    }

    fn native_image(machine: u16, characteristics: u16, managed: bool, identity: &str) -> Vec<u8> {
        let mut image = vec![0u8; 0x400];
        let mut put = |offset: usize, bytes: &[u8]| {
            image[offset..offset + bytes.len()].copy_from_slice(bytes);
        };
        put(0, b"MZ");
        put(0x3C, &0x80u32.to_le_bytes());
        put(0x80, b"PE\0\0");
        put(0x84, &machine.to_le_bytes());
        put(0x86, &1u16.to_le_bytes());
        put(0x94, &240u16.to_le_bytes());
        put(0x96, &characteristics.to_le_bytes());
        put(0x98, &OPTIONAL_HEADER_PE32_PLUS.to_le_bytes());
        put(0x98 + 108, &16u32.to_le_bytes());
        if managed {
            put(0x98 + 112 + CLR_DIRECTORY_INDEX * 8, &0x2000u32.to_le_bytes());
            put(0x98 + 112 + CLR_DIRECTORY_INDEX * 8 + 4, &72u32.to_le_bytes());
        }
        put(0x188, b".rdata\0\0");
        put(0x188 + 16, &0x200u32.to_le_bytes());
        put(0x188 + 20, &0x200u32.to_le_bytes());
        put(0x200, identity.as_bytes());
        image
    }

    fn release_image(version: &str) -> Vec<u8> {
        native_image(MACHINE_AMD64, 0x0022, false, &release_identity(version))
    }

    fn asset_json(name: &str, tag: &str, content: &[u8]) -> Value {
        json!({
            "url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/assets/310045580",
            "id": 310045580,
            "name": name,
            "label": "",
            "content_type": "application/x-msdownload",
            "state": "uploaded",
            "size": content.len(),
            "digest": format!("sha256:{}", sha256_bytes(content)),
            "download_count": 0,
            "browser_download_url": format!(
                "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/{tag}/{name}"
            ),
        })
    }

    fn release_json(tag: &str, assets: Vec<Value>) -> Vec<u8> {
        let release = json!({
            "url": "https://api.github.com/repos/jeremygold02/bluetooth-battery-monitor/releases/251873970",
            "html_url": format!(
                "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/tag/{tag}"
            ),
            "id": 251873970,
            "tag_name": tag,
            "name": tag,
            "draft": false,
            "prerelease": false,
            "assets": assets,
            "body": "## Changes",
        });
        serde_json::to_vec(&release).unwrap()
    }

    fn rust_release(version: &str, image: &[u8]) -> Vec<u8> {
        release_json(
            version,
            vec![
                asset_json("BluetoothBatteryMonitor.exe", version, b"managed tray app"),
                asset_json("BluetoothBatteryMonitor-Setup.exe", version, b"managed installer"),
                asset_json(RELEASE_ASSET_NAME, version, image),
                asset_json("ThirdPartyNotices.zip", version, b"notices"),
            ],
        )
    }

    fn release_with_rust_asset(version: &str, change: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut asset = asset_json(RELEASE_ASSET_NAME, version, &release_image(version));
        change(&mut asset);
        release_json(version, vec![asset])
    }

    fn checked(sandbox: &Sandbox, current_version: &str, release: Vec<u8>) -> UpdateInfo {
        sandbox
            .updater(
                current_version,
                FakeSource::new(Ok(Some(release)), b""),
                FakeHost::staying_up(),
            )
            .check_for_update()
            .unwrap()
    }

    fn check_error(release: Result<Option<Vec<u8>>, TransferError>) -> String {
        let sandbox = Sandbox::new();
        sandbox.install(ORIGINAL_APP);
        sandbox
            .updater("0.1.0", FakeSource::new(release, b""), FakeHost::staying_up())
            .check_for_update()
            .unwrap_err()
            .0
    }

    fn install_failure(outcome: &UpdateOutcome) -> &str {
        match outcome {
            UpdateOutcome::Failed { stage: FailureStage::Install, message } => message,
            other => panic!("expected an installation failure, got {other:?}"),
        }
    }

    fn pending_update(outcome: UpdateOutcome, expected_reason: DeferralReason) -> PreparedUpdate {
        match outcome {
            UpdateOutcome::Deferred { reason, pending } if reason == expected_reason => pending,
            other => panic!("expected a deferred update, got {other:?}"),
        }
    }

    fn http_response(status: &str, headers: &[(&str, String)], body: &[u8]) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    fn sized_response(status: &str, body: &[u8]) -> Vec<u8> {
        http_response(status, &[("Content-Length", body.len().to_string())], body)
    }

    fn accept_within(listener: &TcpListener, limit: Duration) -> Option<TcpStream> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Ok((stream, _)) = listener.accept() {
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                return Some(stream);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn read_request_head(stream: &mut TcpStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => break,
            }
        }
        String::from_utf8_lossy(&head).into_owned()
    }

    fn loopback(port: u16) -> HttpEndpoint {
        HttpEndpoint { host: "127.0.0.1", port, secure: false, proxy: WINHTTP_ACCESS_TYPE_NO_PROXY }
    }

    fn serve(replies: Vec<Reply>) -> LoopbackServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let (release, released) = mpsc::channel::<()>();
        let requests = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let Some(mut stream) = accept_within(&listener, Duration::from_secs(10)) else {
                    break;
                };
                requests.push(read_request_head(&mut stream));
                match reply {
                    Reply::Send(bytes) => {
                        let _ = stream.write_all(&bytes);
                        let _ = stream.shutdown(Shutdown::Write);
                    }
                    Reply::Stall(bytes) => {
                        let _ = stream.write_all(&bytes);
                        let _ = released.recv_timeout(Duration::from_secs(60));
                    }
                    Reply::Drip(sent_at_once, trickled) => {
                        let _ = stream.write_all(&sent_at_once);
                        for byte in trickled.iter().cycle().take(600) {
                            let waiting = released.recv_timeout(Duration::from_millis(100));
                            if stream.write_all(&[*byte]).is_err()
                                || waiting != Err(RecvTimeoutError::Timeout)
                            {
                                break;
                            }
                        }
                    }
                }
            }
            requests
        });
        LoopbackServer { endpoint: loopback(port), release, requests }
    }

    fn start_fixture(sandbox: &Sandbox) -> (Vec<u8>, Child) {
        let harness = fs::read(std::env::current_exe().unwrap()).unwrap();
        let target = sandbox.install(&harness);
        fs::write(sandbox.app_dir().join(FIXTURE_HOLD_FILE), "hold").unwrap();
        let child = Command::new(&target)
            .args(FIXTURE_ARGUMENTS)
            .current_dir(sandbox.app_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap();
        (harness, child)
    }

    fn release_fixtures(sandbox: &Sandbox, mut old_instance: Child) {
        fs::remove_file(sandbox.app_dir().join(FIXTURE_HOLD_FILE)).unwrap();
        old_instance.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while instance_running_outside_family(&sandbox.target(), u32::MAX)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn runnable_release(harness: &[u8], version: &str) -> Vec<u8> {
        let mut image = harness.to_vec();
        image.extend_from_slice(release_identity(version).as_bytes());
        image
    }

    fn assert_send<T: Send>() {}

    fn assert_send_and_sync<T: Send + Sync>() {}

    #[test]
    #[ignore = "runs only as a scratch process started by the updater tests"]
    fn fixture_process_waits_while_hold_file_exists() {
        let Ok(executable) = std::env::current_exe() else {
            return;
        };
        let hold_file = executable.with_file_name(FIXTURE_HOLD_FILE);
        let deadline = Instant::now() + Duration::from_secs(60);
        while hold_file.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn versions_compare_like_the_original_app() {
        for (value, cleaned) in [
            ("0.1.2", "0.1.2"),
            ("v1.2.3-beta", "1.2.3"),
            ("release 4", "4"),
            ("1.2.3.4", "1.2.3"),
            ("1..2", "1"),
            ("1.2.", "1.2"),
            ("", "0.1.0"),
            ("unknown", "0.1.0"),
        ] {
            assert_eq!(clean_version(value), cleaned, "{value}");
        }
        assert_eq!(compare_versions("0.1.10", "0.1.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.2", "0.1.99"), Ordering::Greater);
        assert_eq!(compare_versions("1", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("v0.1.2", "0.1.2"), Ordering::Equal);
        assert_eq!(compare_versions("0.1.2", "0.1.3"), Ordering::Less);
        assert_eq!(compare_versions("2147483648.0.0", "1.0.0"), Ordering::Less);
    }

    #[test]
    fn release_tags_must_be_strict_three_part_versions() {
        assert_eq!(parse_release_version("0.1.2").as_deref(), Some("0.1.2"));
        assert_eq!(parse_release_version("v0.1.2").as_deref(), Some("0.1.2"));
        assert_eq!(parse_release_version("10.0.2147483647").as_deref(), Some("10.0.2147483647"));
        for invalid in [
            "",
            "invalid",
            "release-0.1.2",
            "1.2",
            "1.2.3-beta",
            "01.2.3",
            "1.2.2147483648",
            "V1.2.3",
            "1.2.3.4",
            "1.+2.3",
            "1. 2.3",
        ] {
            assert_eq!(parse_release_version(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn asset_verification_checks_size_and_sha256() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let sandbox = Sandbox::new();
        let file = sandbox.root.join("asset.bin");
        fs::write(&file, b"verified asset").unwrap();
        let size = b"verified asset".len() as u64;
        let hash = sha256_file(&file).unwrap();
        assert_eq!(hash, sha256_bytes(b"verified asset"));
        assert!(verify_asset_file(&file, size, &hash).is_ok());
        assert!(verify_asset_file(&file, size, &hash.to_uppercase()).is_ok());
        let wrong_size = prepare_failure(verify_asset_file(&file, size + 1, &hash).unwrap_err());
        assert_eq!(
            wrong_size.0,
            "The update file size failed verification. No files were replaced."
        );
        let wrong_hash =
            prepare_failure(verify_asset_file(&file, size, &"0".repeat(64)).unwrap_err());
        assert_eq!(wrong_hash.0, "The update failed SHA-256 verification. No files were replaced.");
        assert!(verify_asset_file(&sandbox.root.join("missing.bin"), size, &hash).is_err());

        let large = sandbox.root.join("large.bin");
        let content: Vec<u8> = (0..300_000u32).map(|value| (value % 251) as u8).collect();
        fs::write(&large, &content).unwrap();
        assert_eq!(sha256_file(&large).unwrap(), sha256_bytes(&content));
    }

    #[test]
    fn sha256_digests_are_parsed_strictly() {
        let hash = "5D9C1F0F3A6F4F2B8A1E7C6D5B4A39281706F5E4D3C2B1A09F8E7D6C5B4A3921";
        assert_eq!(
            parse_sha256_digest(&format!(" SHA256:{hash} ")),
            Some(hash.to_ascii_lowercase())
        );
        for invalid in [
            String::new(),
            hash.to_owned(),
            format!("sha512:{hash}"),
            format!("sha256:{}", &hash[1..]),
            format!("sha256:{hash}0"),
            format!("sha256:{}g", &hash[1..]),
            "sha256:".to_owned(),
        ] {
            assert_eq!(parse_sha256_digest(&invalid), None, "{invalid}");
        }
    }

    #[test]
    fn release_asset_urls_must_point_at_this_repository() {
        let valid = "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/BluetoothBatteryMonitorRust.exe";
        let location = parse_release_asset_url(valid).unwrap();
        assert_eq!(
            location.path_and_query,
            "/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/BluetoothBatteryMonitorRust.exe"
        );
        for accepted in [
            "HTTPS://GitHub.com:443/JeremyGold02/Bluetooth-Battery-Monitor/releases/download/0.2.0/a.exe",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe?download=1",
        ] {
            assert!(parse_release_asset_url(accepted).is_some(), "{accepted}");
        }
        for rejected in [
            "",
            "github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "http://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://example.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://github.com.example.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://github.com:8443/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://user@github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://github.com@example.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a.exe#fragment",
            "https://github.com/other/bluetooth-battery-monitor/releases/download/0.2.0/a.exe",
            "https://github.com/jeremygold02/other/releases/download/0.2.0/a.exe",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/archive/0.2.0.zip",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/../../../../other/repo/a.exe",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/%2E%2e/a.exe",
            "https://github.com\\@example.com/jeremygold02/bluetooth-battery-monitor/releases/download/a.exe",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a b.exe",
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/a\r\nHost: example.com",
        ] {
            assert!(parse_release_asset_url(rejected).is_none(), "{rejected}");
        }
    }

    #[test]
    fn native_release_identity_is_verified_without_running_the_file() {
        let sandbox = Sandbox::new();
        let write = |name: &str, image: Vec<u8>| {
            let path = sandbox.root.join(name);
            fs::write(&path, image).unwrap();
            path
        };
        let rejection =
            |path: &Path, version: &str| verify_native_release(path, version).unwrap_err().0;
        let mismatch = "The downloaded app identity or version does not match the GitHub release.";

        let valid = write("valid.exe", release_image("1.2.3"));
        assert_eq!(verify_native_release(&valid, "1.2.3"), Ok(()));
        assert_eq!(verify_native_release(&valid, "v1.2.3"), Ok(()));
        assert_eq!(rejection(&valid, "1.2.4"), mismatch);
        assert_eq!(rejection(&valid, "11.2.3"), mismatch);
        assert_eq!(rejection(&valid, "release-1.2.3"), "The release version is invalid.");

        let wrong_owner = write(
            "wrong-owner.exe",
            native_image(
                MACHINE_AMD64,
                0x0022,
                false,
                "BluetoothBatteryMonitorRust.identity{other/bluetooth-battery-monitor@1.2.3}",
            ),
        );
        assert_eq!(rejection(&wrong_owner, "1.2.3"), mismatch);
        let wrong_identity = write(
            "wrong-identity.exe",
            native_image(
                MACHINE_AMD64,
                0x0022,
                false,
                "OtherApplication.identity{jeremygold02/bluetooth-battery-monitor@1.2.3}",
            ),
        );
        assert_eq!(rejection(&wrong_identity, "1.2.3"), mismatch);
        let unmarked = write("unmarked.exe", native_image(MACHINE_AMD64, 0x0022, false, ""));
        assert_eq!(rejection(&unmarked, "1.2.3"), mismatch);

        let identity = release_identity("1.2.3");
        let mut truncated_image = release_image("1.2.3");
        truncated_image.truncate(0x300);
        for (name, image) in [
            ("malformed.exe", b"MZ but not a native executable".to_vec()),
            ("x86.exe", native_image(0x014C, 0x0022, false, &identity)),
            ("arm64.exe", native_image(0xAA64, 0x0022, false, &identity)),
            ("library.exe", native_image(MACHINE_AMD64, 0x2022, false, &identity)),
            ("managed.exe", native_image(MACHINE_AMD64, 0x0022, true, &identity)),
            ("truncated.exe", truncated_image),
        ] {
            assert_eq!(
                rejection(&write(name, image), "1.2.3"),
                UNREADABLE_METADATA_MESSAGE,
                "{name}"
            );
        }
        let not_executable = write("text.exe", identity.into_bytes());
        assert_eq!(
            rejection(&not_executable, "1.2.3"),
            "The verified release asset is not a Windows executable. No files were replaced."
        );
        assert_eq!(
            rejection(&sandbox.root.join("missing.exe"), "1.2.3"),
            UNREADABLE_METADATA_MESSAGE
        );

        fs::remove_file(&valid).unwrap();
        assert!(!valid.exists());
    }

    #[test]
    fn own_test_binary_embeds_the_build_identity() {
        assert_eq!(build_version(), env!("BBM_VERSION"));
        let executable = std::env::current_exe().unwrap();
        assert_eq!(verify_native_release(&executable, build_version()), Ok(()));
        assert!(verify_native_release(&executable, "999.999.999").is_err());
    }

    #[test]
    fn update_decisions_match_the_original_app() {
        let info = |update_available: bool, can_install: bool| UpdateInfo {
            update_available,
            can_install,
            ..UpdateInfo::unavailable("")
        };
        let available = info(true, true);
        let mut unavailable = info(true, false);
        let current = info(false, false);
        let decide = decide_update_action;
        assert_eq!(
            decide(&available, UpdateCheckMode::Startup),
            UpdateCheckAction::InstallAutomatically
        );
        assert_eq!(
            decide(&unavailable, UpdateCheckMode::Startup),
            UpdateCheckAction::NotifyFailure
        );
        assert_eq!(decide(&available, UpdateCheckMode::Periodic), UpdateCheckAction::None);
        assert_eq!(
            decide(&unavailable, UpdateCheckMode::Periodic),
            UpdateCheckAction::NotifyFailure
        );
        assert_eq!(decide(&available, UpdateCheckMode::Manual), UpdateCheckAction::PromptInstall);
        assert_eq!(decide(&unavailable, UpdateCheckMode::Manual), UpdateCheckAction::PromptRelease);
        assert_eq!(decide(&current, UpdateCheckMode::Startup), UpdateCheckAction::None);
        assert_eq!(decide(&current, UpdateCheckMode::Manual), UpdateCheckAction::None);

        let mut notices = HashSet::new();
        unavailable.latest_version = "0.1.3".to_owned();
        unavailable.message = "The release asset is missing.".to_owned();
        assert!(try_mark_uninstallable_update_notice(&unavailable, &mut notices));
        assert!(!try_mark_uninstallable_update_notice(&unavailable, &mut notices));
        unavailable.message = "The release digest is missing.".to_owned();
        assert!(try_mark_uninstallable_update_notice(&unavailable, &mut notices));
        unavailable.message = "The release asset is missing.".to_owned();
        assert!(!try_mark_uninstallable_update_notice(&unavailable, &mut notices));
        unavailable.latest_version = "0.1.4".to_owned();
        assert!(try_mark_uninstallable_update_notice(&unavailable, &mut notices));
        assert!(!try_mark_uninstallable_update_notice(&available, &mut notices));
        assert!(!try_mark_uninstallable_update_notice(&current, &mut notices));

        let arguments = |values: &[&str]| -> Vec<String> {
            values.iter().map(|value| (*value).to_owned()).collect()
        };
        let relaunch = arguments(&RELAUNCH_ARGUMENTS);
        assert!(!should_check_for_startup_update(&relaunch));
        assert!(!should_check_for_startup_update(&arguments(&["--SKIP-STARTUP-UPDATE"])));
        assert!(should_check_for_startup_update(&arguments(&["--show-settings"])));
        assert!(should_check_for_startup_update(&[]));
        assert_eq!(
            update_complete_status(&relaunch, "0.1.5").as_deref(),
            Some("Updated to version 0.1.5.")
        );
        assert_eq!(update_complete_status(&arguments(&["--popup"]), "0.1.5"), None);
        assert_eq!(
            release_page_url(),
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/latest"
        );
    }

    #[test]
    fn failed_version_state_survives_bad_files() {
        let sandbox = Sandbox::new();
        let state = sandbox.state_path();
        assert_eq!(read_failed_version(&state), None);
        record_failed_version(&state, "1.2.3").unwrap();
        assert_eq!(read_failed_version(&state).as_deref(), Some("1.2.3"));
        assert_eq!(fs::read_to_string(&state).unwrap(), r#"{"failed_version":"1.2.3"}"#);
        assert_eq!(file_names(state.parent().unwrap()), ["update-state.json"]);
        record_failed_version(&state, "1.2.4").unwrap();
        assert_eq!(read_failed_version(&state).as_deref(), Some("1.2.4"));

        for content in [
            "",
            "not json",
            "[]",
            r#"{"failed_version":7}"#,
            r#"{"failed_version":"latest"}"#,
            r#"{"other":"1.2.3"}"#,
        ] {
            fs::write(&state, content).unwrap();
            assert_eq!(read_failed_version(&state), None, "{content}");
        }
        let oversized = format!(r#"{{"failed_version":"1.2.3","padding":"{}"}}"#, "x".repeat(5000));
        fs::write(&state, oversized).unwrap();
        assert_eq!(read_failed_version(&state), None);
        clear_failed_version(&state);
        assert!(!state.exists());
        clear_failed_version(&state);
        fs::create_dir_all(&state).unwrap();
        assert_eq!(read_failed_version(&state), None);
        assert!(record_failed_version(&state, "1.2.3").is_err());
    }

    #[test]
    fn updater_types_can_cross_threads() {
        assert_send_and_sync::<Updater>();
        assert_send::<UpdateOutcome>();
        assert_send::<PreparedUpdate>();
        assert_send::<ManualCheck>();
        assert_send::<PeriodicCheck>();
        let sandbox = Sandbox::new();
        let updater = Updater::new(build_version(), &sandbox.target(), &sandbox.state_path());
        assert_eq!(updater.current_version, env!("BBM_VERSION"));
        assert_eq!(updater.executable_path, sandbox.target());
        assert_eq!(updater.failed_state_path, sandbox.state_path());
        assert_eq!(updater.download_root, std::env::temp_dir());
        assert_eq!(updater.timing.replace_attempts, 30);
        assert_eq!(updater.timing.replace_delay, Duration::from_secs(1));
        assert_eq!(updater.timing.rollback_attempts, 20);
        assert_eq!(updater.timing.rollback_delay, Duration::from_millis(500));
        assert_eq!(updater.timing.startup_watch, Duration::from_secs(5));
        assert_eq!(GITHUB_API.host, "api.github.com");
        assert_eq!(GITHUB_DOWNLOADS.host, "github.com");
        const { assert!(GITHUB_API.secure && GITHUB_DOWNLOADS.secure) };
        assert_eq!((GITHUB_API.port, GITHUB_DOWNLOADS.port), (443, 443));
        assert_eq!(GITHUB_API.proxy, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY);
        assert_eq!(GITHUB_DOWNLOADS.proxy, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY);
    }

    #[test]
    fn existing_csharp_releases_are_never_an_update() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let release = EXISTING_RELEASE_JSON.as_bytes().to_vec();
        let parsed: Value = serde_json::from_slice(&release).unwrap();
        assert_eq!(parsed["tag_name"], "0.1.57");
        assert_eq!(parsed["assets"].as_array().unwrap().len(), 3);
        assert!(release_asset(&parsed["assets"]).is_none());
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::new(Ok(Some(release)), b"managed tray app"),
            FakeHost::staying_up(),
        );

        let info = updater.check_for_update().unwrap();
        assert_eq!(info, UpdateInfo::unavailable("No update is available."));

        let outcome = updater.startup_update(&|| true);
        assert!(
            matches!(
                &outcome,
                UpdateOutcome::UpToDate { status_text } if status_text == "No update is available."
            ),
            "{outcome:?}"
        );
        assert_eq!(outcome.status_text(), "No update is available.");
        assert_eq!(outcome.notification(), None);
        assert_eq!(outcome.dialog_text(), None);

        let manual = updater.manual_check();
        assert_eq!(manual.action, UpdateCheckAction::None);
        assert_eq!(manual.dialog_text, "No update is available.");
        assert_eq!(manual.status_text, "No update is available.");
        let periodic = updater.periodic_check();
        assert_eq!(periodic.notification, None);
        assert_eq!(periodic.status_text, "No update is available.");

        let forced = UpdateInfo {
            latest_version: "0.1.57".to_owned(),
            update_available: true,
            can_install: true,
            asset_name: "BluetoothBatteryMonitor.exe".to_owned(),
            asset_url: "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.1.57/BluetoothBatteryMonitor.exe".to_owned(),
            asset_size: 16,
            asset_sha256: sha256_bytes(b"managed tray app"),
            message: String::new(),
        };
        let refused = updater.install_update(&forced, &|| true);
        assert_eq!(
            install_failure(&refused),
            "The release asset name is not valid for automatic installation."
        );
        let setup = UpdateInfo {
            asset_name: "BluetoothBatteryMonitor-Setup.exe".to_owned(),
            ..forced.clone()
        };
        assert_eq!(
            install_failure(&updater.install_update(&setup, &|| true)),
            "The release asset name is not valid for automatic installation."
        );

        assert_eq!(updater.source.checks.get(), 4);
        assert_eq!(updater.source.downloads.get(), 0);
        assert!(updater.host.launches.borrow().is_empty());
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);
        assert!(file_names(&sandbox.download_root()).is_empty());
        assert!(!sandbox.state_path().exists());
    }

    #[test]
    fn release_checks_report_the_original_messages() {
        let sandbox = Sandbox::new();
        sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let no_releases = UpdateInfo::unavailable("No published releases were found.");
        let no_update = UpdateInfo::unavailable("No update is available.");

        let unpublished = sandbox
            .updater("0.1.0", FakeSource::new(Ok(None), b""), FakeHost::staying_up())
            .check_for_update()
            .unwrap();
        assert_eq!(unpublished, no_releases);
        assert_eq!(unpublished.status_text(), "No published releases were found.");
        assert_eq!(checked(&sandbox, "0.1.0", b"[]".to_vec()), no_releases);
        assert_eq!(checked(&sandbox, "0.1.0", b"null".to_vec()), no_releases);
        assert_eq!(checked(&sandbox, "0.1.0", release_json("0.2.0", Vec::new())), no_update);
        assert_eq!(
            checked(&sandbox, "0.1.0", br#"{"tag_name":"0.2.0","assets":"none"}"#.to_vec()),
            no_update
        );
        assert_eq!(
            checked(
                &sandbox,
                "0.1.0",
                br#"{"tag_name":"broken","assets":[7,null,{"name":3}]}"#.to_vec()
            ),
            no_update
        );

        let newer = checked(&sandbox, "0.1.0", rust_release("0.2.0", &image));
        assert!(newer.update_available && newer.can_install);
        assert_eq!(newer.latest_version, "0.2.0");
        assert_eq!(newer.asset_name, RELEASE_ASSET_NAME);
        assert_eq!(
            newer.asset_url,
            "https://github.com/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/BluetoothBatteryMonitorRust.exe"
        );
        assert_eq!(newer.asset_size, image.len() as u64);
        assert_eq!(newer.asset_sha256, sha256_bytes(&image));
        assert_eq!(newer.message, "Update available: 0.2.0.");
        assert_eq!(newer.status_text(), "Version 0.2.0 available");
        let prefixed_release =
            release_json("v0.2.0", vec![asset_json(RELEASE_ASSET_NAME, "v0.2.0", &image)]);
        let prefixed = checked(&sandbox, "0.1.0", prefixed_release);
        assert_eq!(prefixed.latest_version, "0.2.0");
        assert!(prefixed.can_install);

        for current_version in ["0.2.0", "0.3.1"] {
            let current = checked(&sandbox, current_version, rust_release("0.2.0", &image));
            assert!(!current.update_available && !current.can_install);
            assert_eq!(current.message, "You are on the latest published version (0.2.0).");
            assert_eq!(current.status_text(), "Up to date: 0.2.0");
        }

        let no_asset = "Update available: 0.2.0, but no Windows exe asset was attached.";
        let bad_address = "Update available: 0.2.0, but its download address is not valid for automatic installation.";
        let no_integrity = "Update available: 0.2.0, but automatic installation is unavailable because the release has no valid size and SHA-256 metadata.";
        let cases: [MetadataCase; 10] = [
            ("missing address", |asset| asset["browser_download_url"] = Value::Null, no_asset),
            ("numeric address", |asset| asset["browser_download_url"] = json!(7), no_asset),
            (
                "foreign address",
                |asset| {
                    asset["browser_download_url"] =
                        json!("https://example.com/BluetoothBatteryMonitorRust.exe");
                },
                bad_address,
            ),
            ("missing digest", |asset| asset["digest"] = Value::Null, no_integrity),
            (
                "weak digest",
                |asset| asset["digest"] = json!("md5:0123456789abcdef0123456789abcdef"),
                no_integrity,
            ),
            ("missing size", |asset| asset["size"] = Value::Null, no_integrity),
            ("zero size", |asset| asset["size"] = json!(0), no_integrity),
            ("negative size", |asset| asset["size"] = json!(-5), no_integrity),
            ("fractional size", |asset| asset["size"] = json!(10.5), no_integrity),
            (
                "oversized asset",
                |asset| asset["size"] = json!(MAX_RELEASE_ASSET_BYTES + 1),
                no_integrity,
            ),
        ];
        for (name, change, message) in cases {
            let info = checked(&sandbox, "0.1.0", release_with_rust_asset("0.2.0", change));
            assert!(info.update_available && !info.can_install, "{name}");
            assert_eq!(info.message, message, "{name}");
            assert_eq!(info.status_text(), message, "{name}");
        }
        for (size, expected) in [(json!("1024"), 1024), (json!(2048.0), 2048), (json!(true), 0)] {
            let release = release_with_rust_asset("0.2.0", |asset| asset["size"] = size);
            assert_eq!(checked(&sandbox, "0.1.0", release).asset_size, expected);
        }

        let unreadable =
            "GitHub returned update information that could not be read safely. Try again later.";
        assert_eq!(check_error(Ok(Some(b"{ not json".to_vec()))), unreadable);
        assert_eq!(check_error(Ok(Some(vec![0xFF, 0xFE, 0x00]))), unreadable);
        assert_eq!(check_error(Ok(Some(Vec::new()))), unreadable);
        assert_eq!(
            check_error(Err(TransferError::TimedOut)),
            "GitHub took too long to respond. Try again in a moment."
        );
        assert_eq!(
            check_error(Err(TransferError::Failed)),
            "Could not check GitHub for updates. Check your connection and try again."
        );
        for tag in ["release-0.2.0", "0.2", "0.2.0-beta", ""] {
            let release = release_json(tag, vec![asset_json(RELEASE_ASSET_NAME, "0.2.0", &image)]);
            assert_eq!(
                check_error(Ok(Some(release))),
                "The latest GitHub release has an invalid version tag. Open the release page to review it manually.",
                "{tag}"
            );
        }
    }

    #[test]
    fn installation_requires_a_writable_installed_executable() {
        let sandbox = Sandbox::new();
        let image = release_image("0.2.0");
        let unsupported =
            "Update available: 0.2.0. Automatic installation is not available in this copy.";

        let missing = checked(&sandbox, "0.1.0", rust_release("0.2.0", &image));
        assert!(missing.update_available && !missing.can_install);
        assert_eq!(missing.message, unsupported);

        let target = sandbox.install(ORIGINAL_APP);
        let writable = fs::metadata(&target).unwrap().permissions();
        let mut read_only = writable.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&target, read_only).unwrap();
        let checked_read_only = checked(&sandbox, "0.1.0", rust_release("0.2.0", &image));
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        let forced = UpdateInfo { can_install: true, ..checked_read_only.clone() };
        let refused = updater.install_update(&forced, &|| true);
        fs::set_permissions(&target, writable).unwrap();
        assert!(checked_read_only.update_available && !checked_read_only.can_install);
        assert_eq!(checked_read_only.message, unsupported);
        assert_eq!(
            install_failure(&refused),
            "Self-update is only available in the packaged Windows app."
        );
        assert_eq!(updater.source.downloads.get(), 0);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);

        let renamed = sandbox.app_dir().join("BluetoothBatteryMonitorRust.bin");
        fs::rename(&target, &renamed).unwrap();
        let mut other_extension = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        other_extension.executable_path = renamed;
        assert!(!other_extension.check_for_update().unwrap().can_install);
    }

    #[test]
    fn startup_update_installs_and_relaunches() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let unrelated = path_with_suffix(&target, ".bak");
        fs::write(&unrelated, "unrelated user file").unwrap();
        record_failed_version(&sandbox.state_path(), "0.1.4").unwrap();
        let image = release_image("0.1.5");
        let updater = sandbox.updater(
            "0.1.4",
            FakeSource::publishing("0.1.5", &image),
            FakeHost::staying_up(),
        );

        let outcome = updater.startup_update(&|| true);

        assert!(
            matches!(
                &outcome,
                UpdateOutcome::InstalledAndRelaunching { version } if version == "0.1.5"
            ),
            "{outcome:?}"
        );
        assert_eq!(outcome.status_text(), "Installing version 0.1.5...");
        assert_eq!(outcome.notification(), None);
        assert_eq!(outcome.dialog_text(), None);
        assert_eq!(fs::read(&target).unwrap(), image);
        assert_eq!(fs::read_to_string(&unrelated).unwrap(), "unrelated user file");
        let launches = updater.host.launches.borrow();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].executable, target);
        assert_eq!(launches[0].arguments, ["--skip-startup-update", "--update-complete"]);
        assert_eq!(launches[0].watch, updater.timing.startup_watch);
        assert_eq!(launches[0].image, image);
        assert_eq!(updater.source.checks.get(), 1);
        assert_eq!(updater.source.downloads.get(), 1);
        assert_eq!(read_failed_version(&sandbox.state_path()), None);
        assert!(file_names(&sandbox.download_root()).is_empty());

        let backups: Vec<String> =
            sandbox.app_files().into_iter().filter(|name| name.ends_with(".tmp.bak")).collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(sandbox.app_dir().join(&backups[0])).unwrap(), ORIGINAL_APP);
        assert_eq!(sandbox.app_files().len(), 3);
        remove_stale_update_files(&target);
        assert_eq!(
            sandbox.app_files(),
            [RELEASE_ASSET_NAME.to_owned(), format!("{RELEASE_ASSET_NAME}.bak")]
        );
        assert_eq!(fs::read(&target).unwrap(), image);
    }

    #[test]
    fn failed_startup_rolls_back_and_blocks_retry() {
        let expected =
            "The previous update failed. The app was restored; check for updates to retry.";
        for startup in [Ok(StartupWatch::Exited), Err(io::ErrorKind::PermissionDenied)] {
            let sandbox = Sandbox::new();
            let target = sandbox.install(ORIGINAL_APP);
            let unrelated = path_with_suffix(&target, ".bak");
            fs::write(&unrelated, "unrelated user file").unwrap();
            let broken = release_image("0.1.5");
            let updater = sandbox.updater(
                "0.1.4",
                FakeSource::publishing("0.1.5", &broken),
                FakeHost::new(startup),
            );

            let outcome = updater.startup_update(&|| true);

            assert!(
                matches!(&outcome, UpdateOutcome::RolledBack { message } if message == expected),
                "{outcome:?}"
            );
            assert_eq!(outcome.status_text(), expected);
            assert_eq!(outcome.notification().as_deref(), Some(expected));
            assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
            assert_eq!(fs::read_to_string(&unrelated).unwrap(), "unrelated user file");
            assert_eq!(
                sandbox.app_files(),
                [RELEASE_ASSET_NAME.to_owned(), format!("{RELEASE_ASSET_NAME}.bak")]
            );
            assert_eq!(updater.host.launches.borrow().len(), 1);
            assert_eq!(updater.host.launches.borrow()[0].image, broken);
            assert_eq!(
                updater.host.launches.borrow()[0].arguments,
                ["--skip-startup-update", "--update-complete"]
            );
            assert_eq!(read_failed_version(&sandbox.state_path()).as_deref(), Some("0.1.5"));

            for _ in 0..2 {
                let retry = updater.startup_update(&|| true);
                assert!(matches!(retry, UpdateOutcome::PreviouslyRolledBack), "{retry:?}");
                assert_eq!(retry.status_text(), expected);
                assert_eq!(retry.notification(), None);
                assert_eq!(retry.dialog_text(), None);
            }
            assert_eq!(updater.source.checks.get(), 3);
            assert_eq!(updater.source.downloads.get(), 1);
            assert_eq!(updater.host.launches.borrow().len(), 1);
            assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);

            let fixed = release_image("0.1.6");
            updater.source.publish("0.1.6", &fixed);
            updater.host.startup.set(Ok(StartupWatch::StillRunning));
            let next = updater.startup_update(&|| true);
            assert!(
                matches!(
                    &next,
                    UpdateOutcome::InstalledAndRelaunching { version } if version == "0.1.6"
                ),
                "{next:?}"
            );
            assert_eq!(fs::read(&target).unwrap(), fixed);
            assert_eq!(read_failed_version(&sandbox.state_path()), None);
            assert!(!sandbox.state_path().exists());
        }
    }

    #[test]
    fn manual_install_may_retry_a_rolled_back_version() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        record_failed_version(&sandbox.state_path(), "0.1.5").unwrap();
        let image = release_image("0.1.5");
        let updater = sandbox.updater(
            "0.1.4",
            FakeSource::publishing("0.1.5", &image),
            FakeHost::new(Ok(StartupWatch::Exited)),
        );
        assert!(matches!(updater.startup_update(&|| true), UpdateOutcome::PreviouslyRolledBack));
        assert_eq!(updater.source.downloads.get(), 0);

        let manual = updater.manual_check();
        assert_eq!(manual.action, UpdateCheckAction::PromptInstall);
        assert_eq!(manual.status_text, "Version 0.1.5 available");
        assert_eq!(
            manual.dialog_text,
            "Update available: 0.1.5.\r\n\r\nInstall now? Bluetooth Battery Monitor will restart automatically."
        );
        let update = manual.update.unwrap();

        let failed_again = updater.install_update(&update, &|| true);
        assert!(matches!(failed_again, UpdateOutcome::RolledBack { .. }), "{failed_again:?}");
        assert_eq!(
            failed_again.dialog_text().as_deref(),
            Some("The previous update failed. The app was restored; check for updates to retry.")
        );
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert_eq!(read_failed_version(&sandbox.state_path()).as_deref(), Some("0.1.5"));

        updater.host.startup.set(Ok(StartupWatch::StillRunning));
        let installed = updater.install_update(&update, &|| true);
        assert!(
            matches!(
                &installed,
                UpdateOutcome::InstalledAndRelaunching { version } if version == "0.1.5"
            ),
            "{installed:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), image);
        assert_eq!(read_failed_version(&sandbox.state_path()), None);
    }

    #[test]
    fn manual_check_reports_the_original_wording() {
        let sandbox = Sandbox::new();
        sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let manual = |current_version: &str, release: Result<Option<Vec<u8>>, TransferError>| {
            sandbox
                .updater(current_version, FakeSource::new(release, &image), FakeHost::staying_up())
                .manual_check()
        };

        let current = manual("0.2.0", Ok(Some(rust_release("0.2.0", &image))));
        assert_eq!(current.action, UpdateCheckAction::None);
        assert_eq!(current.status_text, "Up to date: 0.2.0");
        assert_eq!(current.dialog_text, "You are on the latest published version (0.2.0).");
        assert!(current.update.is_some_and(|update| !update.update_available));

        let unpublished = manual("0.2.0", Ok(None));
        assert_eq!(unpublished.action, UpdateCheckAction::None);
        assert_eq!(unpublished.status_text, "No published releases were found.");
        assert_eq!(unpublished.dialog_text, "No published releases were found.");

        let installable = manual("0.1.0", Ok(Some(rust_release("0.2.0", &image))));
        assert_eq!(installable.action, UpdateCheckAction::PromptInstall);
        assert_eq!(installable.status_text, "Version 0.2.0 available");
        assert_eq!(
            installable.dialog_text,
            "Update available: 0.2.0.\r\n\r\nInstall now? Bluetooth Battery Monitor will restart automatically."
        );

        let without_digest =
            release_with_rust_asset("0.2.0", |asset| asset["digest"] = Value::Null);
        let unverifiable = manual("0.1.0", Ok(Some(without_digest)));
        let message = "Update available: 0.2.0, but automatic installation is unavailable because the release has no valid size and SHA-256 metadata.";
        assert_eq!(unverifiable.action, UpdateCheckAction::PromptRelease);
        assert_eq!(unverifiable.status_text, message);
        assert_eq!(unverifiable.dialog_text, format!("{message}\r\n\r\nOpen the release page?"));

        let offline = manual("0.1.0", Err(TransferError::Failed));
        assert_eq!(offline.action, UpdateCheckAction::None);
        assert_eq!(offline.update, None);
        assert_eq!(
            offline.status_text,
            "Update check failed: Could not check GitHub for updates. Check your connection and try again."
        );
        assert_eq!(
            offline.dialog_text,
            "Could not check for updates:\r\n\r\nCould not check GitHub for updates. Check your connection and try again."
        );
        let slow = manual("0.1.0", Err(TransferError::TimedOut));
        assert_eq!(
            slow.dialog_text,
            "Could not check for updates:\r\n\r\nGitHub took too long to respond. Try again in a moment."
        );
    }

    #[test]
    fn uninstallable_updates_notify_once_and_check_failures_stay_quiet() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let message = "Update available: 0.2.0, but automatic installation is unavailable because the release has no valid size and SHA-256 metadata.";
        let without_digest =
            release_with_rust_asset("0.2.0", |asset| asset["digest"] = Value::Null);
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::new(Ok(Some(without_digest.clone())), b""),
            FakeHost::staying_up(),
        );

        let outcome = updater.startup_update(&|| true);
        assert!(
            matches!(
                &outcome,
                UpdateOutcome::Failed { stage: FailureStage::Unavailable, message: text }
                    if text == message
            ),
            "{outcome:?}"
        );
        assert_eq!(outcome.status_text(), message);
        assert_eq!(outcome.notification().as_deref(), Some(message));
        assert_eq!(outcome.dialog_text().as_deref(), Some(message));
        let repeated = updater.periodic_check();
        assert_eq!(repeated.status_text, message);
        assert_eq!(repeated.notification, None);

        let later = sandbox.updater(
            "0.1.0",
            FakeSource::new(Ok(Some(without_digest)), b""),
            FakeHost::staying_up(),
        );
        assert_eq!(later.periodic_check().notification.as_deref(), Some(message));
        assert_eq!(later.periodic_check().notification, None);
        let without_address =
            release_with_rust_asset("0.2.0", |asset| asset["browser_download_url"] = Value::Null);
        *later.source.release.borrow_mut() = Ok(Some(without_address));
        assert_eq!(
            later.periodic_check().notification.as_deref(),
            Some("Update available: 0.2.0, but no Windows exe asset was attached.")
        );
        later.source.publish("0.2.0", &release_image("0.2.0"));
        let installable = later.periodic_check();
        assert_eq!(installable.status_text, "Version 0.2.0 available");
        assert_eq!(installable.notification, None);
        *later.source.release.borrow_mut() = Err(TransferError::TimedOut);
        let unreachable = later.periodic_check();
        assert_eq!(
            unreachable.status_text,
            "Update check failed: GitHub took too long to respond. Try again in a moment."
        );
        assert_eq!(unreachable.notification, None);
        assert_eq!(
            unavailable_notification(" "),
            "The update could not be installed automatically. Open Settings for details."
        );

        let offline = sandbox.updater(
            "0.1.0",
            FakeSource::new(Err(TransferError::Failed), b""),
            FakeHost::staying_up(),
        );
        let failed_check = offline.startup_update(&|| true);
        assert!(
            matches!(&failed_check, UpdateOutcome::Failed { stage: FailureStage::Check, .. }),
            "{failed_check:?}"
        );
        assert_eq!(
            failed_check.status_text(),
            "Update check failed: Could not check GitHub for updates. Check your connection and try again."
        );
        assert_eq!(failed_check.notification(), None);
        assert_eq!(
            failed_check.dialog_text().as_deref(),
            Some(
                "Could not check for updates:\r\n\r\nCould not check GitHub for updates. Check your connection and try again."
            )
        );
        assert_eq!(offline.source.downloads.get(), 0);
        assert!(offline.host.launches.borrow().is_empty());
        assert!(later.host.launches.borrow().is_empty());
        assert_eq!(later.source.downloads.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);
    }

    #[test]
    fn download_failures_leave_the_app_untouched() {
        let published = release_image("0.2.0");
        let mut tampered = published.clone();
        tampered[0x3FF] ^= 0x01;
        let mut longer = published.clone();
        longer.extend_from_slice(b"extra");
        let other_version = release_image("0.2.1");
        let text_asset = b"<html>not an executable</html>".to_vec();
        let insecure =
            "The update download was redirected to an insecure address. No files were replaced.";
        let incomplete = "The update download was incomplete. No files were replaced.";
        let unreachable =
            "The update could not be downloaded securely. Check your connection and try again.";
        let timed_out = "The update download timed out. Try again in a moment.";

        let cases: [DownloadCase; 13] = [
            (
                "declared length differs",
                published.clone(),
                published.clone(),
                |source| source.declared_length.set(Some(7)),
                "The update download size did not match the release metadata. No files were replaced.",
            ),
            (
                "body is longer than announced",
                published.clone(),
                longer,
                |source| source.declared_length.set(None),
                "The update download exceeded the expected size. No files were replaced.",
            ),
            (
                "body ends early",
                published.clone(),
                published[..0x100].to_vec(),
                |source| source.declared_length.set(None),
                incomplete,
            ),
            (
                "body is empty",
                published.clone(),
                Vec::new(),
                |source| source.declared_length.set(None),
                incomplete,
            ),
            (
                "redirected to plain http",
                published.clone(),
                published.clone(),
                |source| *source.final_url.borrow_mut() = "http://example.com/asset".to_owned(),
                insecure,
            ),
            (
                "final address unknown",
                published.clone(),
                published.clone(),
                |source| source.final_url.borrow_mut().clear(),
                insecure,
            ),
            (
                "connection times out",
                published.clone(),
                published.clone(),
                |source| source.open_failure.set(Some(TransferError::TimedOut)),
                timed_out,
            ),
            (
                "connection fails",
                published.clone(),
                published.clone(),
                |source| source.open_failure.set(Some(TransferError::Failed)),
                unreachable,
            ),
            (
                "body read times out",
                published.clone(),
                published[..0x200].to_vec(),
                |source| {
                    source.declared_length.set(None);
                    source.body_failure.set(Some(io::ErrorKind::TimedOut));
                },
                timed_out,
            ),
            (
                "connection drops mid-body",
                published.clone(),
                published[..0x200].to_vec(),
                |source| {
                    source.declared_length.set(None);
                    source.body_failure.set(Some(io::ErrorKind::ConnectionReset));
                },
                unreachable,
            ),
            (
                "content does not match the digest",
                published.clone(),
                tampered,
                |_| {},
                "The update failed SHA-256 verification. No files were replaced.",
            ),
            (
                "asset was built for another version",
                other_version.clone(),
                other_version,
                |_| {},
                "The downloaded app identity or version does not match the GitHub release.",
            ),
            (
                "asset is not an executable",
                text_asset.clone(),
                text_asset,
                |_| {},
                "The verified release asset is not a Windows executable. No files were replaced.",
            ),
        ];

        for (name, announced, served, configure, message) in cases {
            let sandbox = Sandbox::new();
            let target = sandbox.install(ORIGINAL_APP);
            let source = FakeSource::new(Ok(Some(rust_release("0.2.0", &announced))), &served);
            configure(&source);
            let updater = sandbox.updater("0.1.0", source, FakeHost::staying_up());

            let outcome = updater.startup_update(&|| true);

            assert_eq!(install_failure(&outcome), message, "{name}");
            assert_eq!(
                outcome.status_text(),
                format!("Update installation failed: {message}"),
                "{name}"
            );
            assert_eq!(outcome.notification().as_deref(), Some(INSTALL_FAILURE_NOTICE), "{name}");
            assert_eq!(
                outcome.dialog_text(),
                Some(format!("Could not install the update:\r\n\r\n{message}")),
                "{name}"
            );
            assert_eq!(updater.source.downloads.get(), 1, "{name}");
            assert!(updater.host.launches.borrow().is_empty(), "{name}");
            assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP, "{name}");
            assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME], "{name}");
            assert!(file_names(&sandbox.download_root()).is_empty(), "{name}");
            assert!(!sandbox.state_path().exists(), "{name}");
        }
    }

    #[test]
    fn prepared_update_is_validated_before_any_download() {
        let sandbox = Sandbox::new();
        sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        let valid = updater.check_for_update().unwrap();
        let refusal = |info: UpdateInfo| updater.prepare_update(&info).unwrap_err().0;
        let integrity = "The release does not include valid integrity metadata. Open the release page to install it manually.";

        assert_eq!(
            refusal(UpdateInfo { update_available: false, ..valid.clone() }),
            "No update is available."
        );
        for version in ["0.1.0", "0.0.9", "next", ""] {
            assert_eq!(
                refusal(UpdateInfo { latest_version: version.to_owned(), ..valid.clone() }),
                "The update version is invalid or is not newer than this app.",
                "{version}"
            );
        }
        assert_eq!(
            refusal(UpdateInfo { asset_url: " ".to_owned(), ..valid.clone() }),
            "The latest release does not include a Windows exe asset."
        );
        assert_eq!(
            refusal(UpdateInfo {
                asset_name: "BluetoothBatteryMonitor.exe".to_owned(),
                ..valid.clone()
            }),
            "The release asset name is not valid for automatic installation."
        );
        assert_eq!(refusal(UpdateInfo { asset_size: 0, ..valid.clone() }), integrity);
        assert_eq!(
            refusal(UpdateInfo { asset_size: MAX_RELEASE_ASSET_BYTES + 1, ..valid.clone() }),
            integrity
        );
        assert_eq!(
            refusal(UpdateInfo { asset_sha256: "abc".to_owned(), ..valid.clone() }),
            integrity
        );
        assert_eq!(
            refusal(UpdateInfo {
                asset_url: "https://example.com/BluetoothBatteryMonitorRust.exe".to_owned(),
                ..valid.clone()
            }),
            "The release download address is not valid for automatic installation."
        );
        assert_eq!(updater.source.downloads.get(), 0);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);
    }

    #[test]
    fn dropping_a_prepared_update_cleans_up() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        let info = updater.check_for_update().unwrap();

        let prepared = updater.prepare_update(&info).unwrap();

        assert_eq!(prepared.version, "0.2.0");
        assert_eq!(fs::read(&prepared.staged).unwrap(), image);
        assert_eq!(prepared.staged.parent(), Some(sandbox.app_dir().as_path()));
        assert!(!prepared.backup.exists());
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert!(updater.host.launches.borrow().is_empty());
        assert!(file_names(&sandbox.download_root()).is_empty());
        assert_eq!(sandbox.app_files().len(), 3);
        drop(prepared);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
    }

    #[test]
    fn existing_backup_stops_replacement() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        let prepared = updater.prepare_update(&updater.check_for_update().unwrap()).unwrap();
        let backup = prepared.backup.clone();
        fs::write(&backup, "existing file").unwrap();

        let outcome = updater.finish_update(prepared, &|| true);

        assert_eq!(install_failure(&outcome), "The update backup path already exists.");
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert_eq!(fs::read_to_string(&backup).unwrap(), "existing file");
        assert!(updater.host.launches.borrow().is_empty());
        assert!(!sandbox.state_path().exists());
        assert_eq!(sandbox.app_files().len(), 2);
    }

    #[test]
    fn concurrent_updates_cannot_both_replace_the_app() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let first_image = release_image("0.2.0");
        let first = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &first_image),
            FakeHost::staying_up(),
        );
        let second = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &first_image),
            FakeHost::staying_up(),
        );

        let winner = first.prepare_update(&first.check_for_update().unwrap()).unwrap();
        let blocked = second.startup_update(&|| true);
        assert_eq!(install_failure(&blocked), "Another update is still in progress.");
        assert_eq!(second.source.downloads.get(), 0);
        remove_stale_update_files(&target);
        assert!(winner.staged.is_file());
        let installed = first.finish_update(winner, &|| true);
        assert!(
            matches!(installed, UpdateOutcome::InstalledAndRelaunching { .. }),
            "{installed:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), first_image);
        assert!(second.host.launches.borrow().is_empty());

        let newer_image = release_image("0.3.0");
        let stale_updater = sandbox.updater(
            "0.2.0",
            FakeSource::publishing("0.3.0", &newer_image),
            FakeHost::staying_up(),
        );
        let stale =
            stale_updater.prepare_update(&stale_updater.check_for_update().unwrap()).unwrap();
        let winning_image = release_image("0.3.1");
        fs::write(&target, &winning_image).unwrap();
        let rejected = stale_updater.finish_update(stale, &|| true);
        assert_eq!(
            install_failure(&rejected),
            "The installed executable changed after this update was prepared."
        );
        assert_eq!(fs::read(&target).unwrap(), winning_image);
        assert!(stale_updater.host.launches.borrow().is_empty());
    }

    #[test]
    fn running_instance_defers_replacement() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        updater.host.other_instance.set(true);

        let outcome = updater.startup_update(&|| true);

        assert_eq!(
            outcome.status_text(),
            "Update ready; waiting for other running instances to close."
        );
        assert_eq!(outcome.notification(), None);
        let pending = pending_update(outcome, DeferralReason::OtherInstanceRunning);
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert!(updater.host.launches.borrow().is_empty());
        assert!(pending.staged.is_file());

        let still_waiting = updater.finish_update(pending, &|| true);
        let pending = pending_update(still_waiting, DeferralReason::OtherInstanceRunning);
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);

        updater.host.other_instance.set(false);
        let installed = updater.finish_update(pending, &|| true);
        assert!(
            matches!(installed, UpdateOutcome::InstalledAndRelaunching { .. }),
            "{installed:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), image);
        assert_eq!(updater.host.launches.borrow().len(), 1);
        assert_eq!(updater.source.downloads.get(), 1);
    }

    #[test]
    fn unsaved_settings_defer_restart_until_allowed() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        let questions = Cell::new(0);
        let unsaved = || {
            questions.set(questions.get() + 1);
            false
        };

        let outcome = updater.startup_update(&unsaved);

        assert_eq!(
            outcome.status_text(),
            "Update ready; waiting for unsaved settings to be saved or closed."
        );
        assert_eq!(outcome.notification(), None);
        assert_eq!(outcome.dialog_text(), None);
        let pending = pending_update(outcome, DeferralReason::UnsavedSettings);
        assert_eq!(pending.version, "0.2.0");
        assert_eq!(questions.get(), 1);
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert!(updater.host.launches.borrow().is_empty());
        assert!(!sandbox.state_path().exists());

        let still_unsaved = updater.finish_update(pending, &unsaved);
        let pending = pending_update(still_unsaved, DeferralReason::UnsavedSettings);
        assert_eq!(questions.get(), 2);
        assert!(updater.host.launches.borrow().is_empty());

        let installed = updater.finish_update(pending, &|| true);
        assert!(
            matches!(installed, UpdateOutcome::InstalledAndRelaunching { .. }),
            "{installed:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), image);
        assert_eq!(updater.host.launches.borrow().len(), 1);
        assert_eq!(updater.source.downloads.get(), 1);

        let abandoned_sandbox = Sandbox::new();
        abandoned_sandbox.install(ORIGINAL_APP);
        let abandoning = abandoned_sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        drop(abandoning.startup_update(&|| false));
        assert_eq!(abandoned_sandbox.app_files(), [RELEASE_ASSET_NAME]);
        assert!(abandoning.host.launches.borrow().is_empty());
    }

    #[test]
    fn swap_reports_missing_or_changed_files() {
        let image = release_image("0.2.0");
        let prepare = |sandbox: &Sandbox| {
            sandbox.install(ORIGINAL_APP);
            let updater = sandbox.updater(
                "0.1.0",
                FakeSource::publishing("0.2.0", &image),
                FakeHost::staying_up(),
            );
            let prepared = updater.prepare_update(&updater.check_for_update().unwrap()).unwrap();
            (updater, prepared)
        };

        let sandbox = Sandbox::new();
        let (updater, prepared) = prepare(&sandbox);
        fs::remove_file(&prepared.staged).unwrap();
        let outcome = updater.finish_update(prepared, &|| true);
        assert_eq!(install_failure(&outcome), "The staged update file is missing.");
        assert_eq!(fs::read(sandbox.target()).unwrap(), ORIGINAL_APP);

        let sandbox = Sandbox::new();
        let (updater, prepared) = prepare(&sandbox);
        fs::write(&prepared.staged, release_image("0.2.9")).unwrap();
        let outcome = updater.finish_update(prepared, &|| true);
        assert_eq!(
            install_failure(&outcome),
            "The update failed SHA-256 verification. No files were replaced."
        );
        assert_eq!(fs::read(sandbox.target()).unwrap(), ORIGINAL_APP);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);

        let sandbox = Sandbox::new();
        let (updater, prepared) = prepare(&sandbox);
        fs::remove_file(sandbox.target()).unwrap();
        let outcome = updater.finish_update(prepared, &|| true);
        assert_eq!(install_failure(&outcome), "The installed executable is missing.");
        assert!(updater.host.launches.borrow().is_empty());
        assert!(sandbox.app_files().is_empty());
    }

    #[test]
    fn stale_update_files_are_removed_only_when_no_update_is_active() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let app_dir = sandbox.app_dir();
        let stale = [
            ".BluetoothBatteryMonitorRust.exe.update-0123.tmp",
            ".BluetoothBatteryMonitorRust.exe.update-0123.tmp.bak",
            ".BluetoothBatteryMonitorRust.exe.update-0123.tmp.failed",
            ".bluetoothbatterymonitorrust.exe.UPDATE-write-0123.target",
        ];
        let kept = [
            "BluetoothBatteryMonitorRust.exe.bak",
            ".BluetoothBatteryMonitor.exe.update-0123.tmp",
            "settings.json",
        ];
        for name in stale.iter().chain(&kept) {
            fs::write(app_dir.join(name), name).unwrap();
        }
        let folder = ".BluetoothBatteryMonitorRust.exe.update-folder";
        fs::create_dir(app_dir.join(folder)).unwrap();

        let image = release_image("0.2.0");
        let updater = sandbox.updater(
            "0.1.0",
            FakeSource::publishing("0.2.0", &image),
            FakeHost::staying_up(),
        );
        let prepared = updater.prepare_update(&updater.check_for_update().unwrap()).unwrap();
        remove_stale_update_files(&target);
        assert!(prepared.staged.is_file());
        for name in stale {
            assert!(app_dir.join(name).is_file(), "{name}");
        }
        drop(prepared);

        remove_stale_update_files(&target);

        let mut expected: Vec<String> = kept.iter().map(|name| (*name).to_owned()).collect();
        expected.push(folder.to_owned());
        expected.push(RELEASE_ASSET_NAME.to_owned());
        expected.sort();
        assert_eq!(sandbox.app_files(), expected);
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        remove_stale_update_files(&sandbox.app_dir().join("missing.exe"));
        assert_eq!(sandbox.app_files(), expected);
    }

    #[test]
    fn real_process_swap_relaunches_and_cleans_up() {
        let sandbox = Sandbox::new();
        let (harness, mut old_instance) = start_fixture(&sandbox);
        let target = sandbox.target();
        let image = runnable_release(&harness, "9.9.9");
        let host =
            FixtureHost { keep_running: true, requested_arguments: RefCell::new(Vec::new()) };
        let detected_as_child = !host.other_instance_running(&target);
        let detected_as_stranger = instance_running_outside_family(&target, u32::MAX);
        let updater =
            sandbox.updater(build_version(), FakeSource::publishing("9.9.9", &image), host);

        let outcome = updater.startup_update(&|| true);

        let old_instance_survived = old_instance.try_wait().unwrap().is_none();
        let new_instance_running = instance_running_outside_family(&target, u32::MAX);
        let installed_image = fs::read(&target).unwrap();
        let requested = updater.host.requested_arguments.borrow().clone();
        release_fixtures(&sandbox, old_instance);
        remove_stale_update_files(&target);

        assert!(detected_as_child);
        assert!(detected_as_stranger);
        assert!(
            matches!(
                &outcome,
                UpdateOutcome::InstalledAndRelaunching { version } if version == "9.9.9"
            ),
            "{outcome:?}"
        );
        assert!(old_instance_survived);
        assert!(new_instance_running);
        assert_eq!(sha256_bytes(&installed_image), sha256_bytes(&image));
        assert_eq!(requested, [["--skip-startup-update", "--update-complete"]]);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);
        assert!(!instance_running_outside_family(&target, u32::MAX));
    }

    #[test]
    fn real_process_early_exit_rolls_back_without_stopping_the_old_instance() {
        let sandbox = Sandbox::new();
        let (harness, mut old_instance) = start_fixture(&sandbox);
        let target = sandbox.target();
        let image = runnable_release(&harness, "9.9.9");
        let host =
            FixtureHost { keep_running: false, requested_arguments: RefCell::new(Vec::new()) };
        let mut updater =
            sandbox.updater(build_version(), FakeSource::publishing("9.9.9", &image), host);
        updater.timing.startup_watch = Duration::from_secs(30);

        let outcome = updater.startup_update(&|| true);

        let old_instance_survived = old_instance.try_wait().unwrap().is_none();
        let restored_image = fs::read(&target).unwrap();
        let files_after_rollback = sandbox.app_files();
        let requested = updater.host.requested_arguments.borrow().clone();
        release_fixtures(&sandbox, old_instance);

        assert!(matches!(&outcome, UpdateOutcome::RolledBack { .. }), "{outcome:?}");
        assert!(old_instance_survived);
        assert_eq!(sha256_bytes(&restored_image), sha256_bytes(&harness));
        assert_eq!(files_after_rollback, [RELEASE_ASSET_NAME, FIXTURE_HOLD_FILE]);
        assert_eq!(requested, [["--skip-startup-update", "--update-complete"]]);
        assert_eq!(read_failed_version(&sandbox.state_path()).as_deref(), Some("9.9.9"));
        assert!(matches!(updater.startup_update(&|| true), UpdateOutcome::PreviouslyRolledBack));
    }

    #[test]
    fn real_process_host_reports_launch_failures() {
        let sandbox = Sandbox::new();
        let missing = sandbox.app_dir().join("missing.exe");
        let result = WindowsProcessHost.launch_and_watch(&missing, &[], Duration::from_millis(50));
        assert!(result.is_err());
        assert!(!WindowsProcessHost.other_instance_running(&missing));
        assert!(!instance_running_outside_family(&missing, u32::MAX));
    }

    #[test]
    fn winhttp_fetches_the_latest_release_document() {
        let document = rust_release("0.2.0", &release_image("0.2.0"));
        let server = serve(vec![
            Reply::Send(sized_response("200 OK", &document)),
            Reply::Send(sized_response("404 Not Found", b"{\"message\":\"Not Found\"}")),
            Reply::Send(sized_response("403 Forbidden", b"{\"message\":\"rate limited\"}")),
            Reply::Send(sized_response("500 Internal Server Error", b"")),
            Reply::Send(http_response("200 OK", &[], &document)),
        ]);
        let limit = Duration::from_secs(10);

        assert_eq!(fetch_latest_release(&server.endpoint, limit), Ok(Some(document.clone())));
        assert_eq!(fetch_latest_release(&server.endpoint, limit), Ok(None));
        assert_eq!(fetch_latest_release(&server.endpoint, limit), Err(TransferError::Failed));
        assert_eq!(fetch_latest_release(&server.endpoint, limit), Err(TransferError::Failed));
        assert_eq!(fetch_latest_release(&server.endpoint, limit), Ok(Some(document)));

        let requests = server.finish();
        assert_eq!(requests.len(), 5);
        for request in requests {
            let lowercase = request.to_lowercase();
            assert!(
                request.starts_with(
                    "GET /repos/jeremygold02/bluetooth-battery-monitor/releases/latest HTTP/1.1\r\n"
                ),
                "{request}"
            );
            assert!(
                lowercase.contains("\r\nuser-agent: bluetooth-battery-monitor\r\n"),
                "{request}"
            );
            assert!(lowercase.contains("\r\naccept: application/vnd.github+json\r\n"), "{request}");
            assert!(lowercase.contains("\r\nx-github-api-version: 2022-11-28\r\n"), "{request}");
        }
    }

    #[test]
    fn winhttp_follows_redirects_and_streams_the_asset() {
        let asset: Vec<u8> = (0..400_000u32).map(|value| (value % 253) as u8).collect();
        let redirect = http_response(
            "302 Found",
            &[("Location", "/storage/final-asset".to_owned()), ("Content-Length", "0".to_owned())],
            b"",
        );
        let server =
            serve(vec![Reply::Send(redirect), Reply::Send(sized_response("200 OK", &asset))]);
        let asset_path = "/jeremygold02/bluetooth-battery-monitor/releases/download/0.2.0/BluetoothBatteryMonitorRust.exe";

        let mut download =
            open_download(&server.endpoint, asset_path, Duration::from_secs(10)).unwrap();

        assert_eq!(
            download.final_url,
            format!("http://127.0.0.1:{}/storage/final-asset", server.endpoint.port)
        );
        assert_eq!(download.content_length, Some(asset.len() as u64));
        let mut received = Vec::new();
        download.body.read_to_end(&mut received).unwrap();
        assert_eq!(received.len(), asset.len());
        assert_eq!(sha256_bytes(&received), sha256_bytes(&asset));
        drop(download);

        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with(&format!("GET {asset_path} HTTP/1.1\r\n")), "{requests:?}");
        assert!(requests[0].to_lowercase().contains("\r\naccept: application/octet-stream\r\n"));
        assert!(requests[1].starts_with("GET /storage/final-asset HTTP/1.1\r\n"), "{requests:?}");
    }

    #[test]
    fn winhttp_times_out_when_the_server_never_answers() {
        let server = serve(vec![Reply::Stall(Vec::new())]);
        let started = Instant::now();
        let result = fetch_latest_release(&server.endpoint, Duration::from_millis(250));
        let elapsed = started.elapsed();
        assert_eq!(server.finish().len(), 1);
        assert_eq!(result, Err(TransferError::TimedOut));
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    }

    #[test]
    fn winhttp_times_out_when_response_headers_trickle() {
        let trickled = b"HTTP/1.1 200 OK\r\nX-Padding: ".to_vec();
        let server = serve(vec![Reply::Drip(Vec::new(), trickled)]);
        let started = Instant::now();
        let result = open_download(&server.endpoint, "/asset", Duration::from_millis(250));
        let elapsed = started.elapsed();
        assert_eq!(server.finish().len(), 1);
        assert!(matches!(result, Err(TransferError::TimedOut)), "{:?}", result.err());
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    }

    #[test]
    fn winhttp_times_out_when_the_release_document_trickles() {
        let headers = http_response("200 OK", &[], b"");
        let server = serve(vec![Reply::Drip(headers, b" ".to_vec())]);
        let started = Instant::now();
        let result = fetch_latest_release(&server.endpoint, Duration::from_millis(250));
        let elapsed = started.elapsed();
        assert_eq!(server.finish().len(), 1);
        assert_eq!(result, Err(TransferError::TimedOut));
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    }

    #[test]
    fn winhttp_times_out_when_the_body_stalls() {
        let headers_only = http_response("200 OK", &[("Content-Length", "1000".to_owned())], b"");
        let server = serve(vec![Reply::Stall(headers_only)]);
        let mut download =
            open_download(&server.endpoint, "/asset", Duration::from_millis(250)).unwrap();
        let started = Instant::now();
        let stalled_read = download.body.read(&mut [0u8; 64]);
        let elapsed = started.elapsed();
        drop(download);
        assert_eq!(server.finish().len(), 1);
        let failure = stalled_read.unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::TimedOut);
        assert_eq!(read_failure(&failure), TransferError::TimedOut);
        assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
    }

    #[test]
    fn winhttp_reports_refusals_and_bad_statuses() {
        let missing = serve(vec![Reply::Send(sized_response("404 Not Found", b"missing"))]);
        let result = open_download(&missing.endpoint, "/asset", Duration::from_secs(10));
        assert!(matches!(result, Err(TransferError::Failed)));
        assert_eq!(missing.finish().len(), 1);

        let closed_port = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        assert_eq!(
            fetch_latest_release(&loopback(closed_port), Duration::from_secs(20)),
            Err(TransferError::Failed)
        );
    }

    #[test]
    fn winhttp_truncated_downloads_are_rejected() {
        let partial =
            http_response("200 OK", &[("Content-Length", "1000".to_owned())], &[7u8; 100]);
        let server = serve(vec![Reply::Send(partial)]);
        let mut download =
            open_download(&server.endpoint, "/asset", Duration::from_secs(10)).unwrap();
        assert_eq!(download.content_length, Some(1000));
        let mut received = Vec::new();
        let finished = download.body.read_to_end(&mut received);
        assert!(finished.is_err() || received.len() < 1000, "{finished:?}");
        drop(download);
        assert_eq!(server.finish().len(), 1);
    }

    #[test]
    fn plain_http_downloads_are_refused_end_to_end() {
        let sandbox = Sandbox::new();
        let target = sandbox.install(ORIGINAL_APP);
        let image = release_image("0.2.0");
        let server = serve(vec![
            Reply::Send(sized_response("200 OK", &rust_release("0.2.0", &image))),
            Reply::Send(sized_response("200 OK", &image)),
        ]);
        let source = LoopbackSource { endpoint: loopback(server.endpoint.port) };
        let updater = sandbox.updater("0.1.0", source, FakeHost::staying_up());

        let outcome = updater.startup_update(&|| true);

        assert_eq!(
            install_failure(&outcome),
            "The update download was redirected to an insecure address. No files were replaced."
        );
        assert_eq!(server.finish().len(), 2);
        assert!(updater.host.launches.borrow().is_empty());
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_APP);
        assert_eq!(sandbox.app_files(), [RELEASE_ASSET_NAME]);
        assert!(file_names(&sandbox.download_root()).is_empty());
    }
}
