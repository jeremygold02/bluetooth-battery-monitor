use crate::rate::LearnedBatteryRate;
use crate::win;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CURRENT_SETTINGS_VERSION: i32 = 9;
pub const DEFAULT_TRAY_BOX_COLOR: &str = "#0078D7";
pub const DEFAULT_TRAY_TEXT_COLOR: &str = "#FFFFFF";
pub const ICON_METRIC_LOWEST: &str = "Lowest";
pub const ICON_METRIC_AVERAGE: &str = "Average";
pub const ICON_METRIC_DEVICE: &str = "Device";
pub const THEME_SYSTEM: &str = "System Default";
pub const THEME_DARK: &str = "Dark Mode";
pub const THEME_LIGHT: &str = "Light Mode";
pub const MINIMUM_OBSERVED_AT: i64 = -62_135_596_800_000;
const SETTINGS_FOLDER: &str = "BluetoothBatteryMonitor";
const SETTINGS_FILE: &str = "settings.json";
const OPTIONAL_KEYS: [&str; 2] = ["LearnedBatteryRates", "CachedBatteryReadings"];

pub fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn serialize_date<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("/Date({value})/"))
}

fn deserialize_date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let text = Option::<String>::deserialize(deserializer)?.unwrap_or_default();
    Ok(parse_date(&text).unwrap_or(MINIMUM_OBSERVED_AT))
}

fn parse_date(text: &str) -> Option<i64> {
    let inner = text.strip_prefix("/Date(")?.strip_suffix(")/")?;
    let digits_end = inner
        .char_indices()
        .skip(1)
        .find(|(_, character)| !character.is_ascii_digit())
        .map_or(inner.len(), |(index, _)| index);
    inner[..digits_end].parse().ok()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct CachedBatteryReading {
    #[serde(deserialize_with = "null_as_default")]
    pub id: String,
    pub physical_id: Option<String>,
    #[serde(deserialize_with = "null_as_default")]
    pub name: String,
    #[serde(deserialize_with = "null_as_default")]
    pub source: String,
    pub percent: i32,
    pub priority: i32,
    #[serde(serialize_with = "serialize_date", deserialize_with = "deserialize_date")]
    pub observed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Settings {
    pub settings_version: i32,
    pub overlay_number: bool,
    pub critical_threshold: i32,
    pub low_threshold: i32,
    #[serde(deserialize_with = "null_as_default")]
    pub icon_metric: String,
    #[serde(deserialize_with = "null_as_default")]
    pub icon_device_id: String,
    #[serde(deserialize_with = "null_as_default")]
    pub icon_device_name: String,
    pub color_bars: bool,
    pub show_popup_reset_times: bool,
    pub show_popup_last_updated: bool,
    pub show_additional_limits: bool,
    pub show_reset_availability: bool,
    pub start_with_windows: bool,
    pub threshold_notifications: bool,
    pub refresh_seconds: i32,
    pub idle_refresh_seconds: i32,
    pub show_tray_box: bool,
    pub use_custom_tray_box_color: bool,
    #[serde(deserialize_with = "null_as_default")]
    pub tray_box_color: String,
    #[serde(deserialize_with = "null_as_default")]
    pub tray_text_color: String,
    #[serde(deserialize_with = "null_as_default")]
    pub theme: String,
    #[serde(deserialize_with = "null_as_default")]
    pub learned_battery_rates: Vec<LearnedBatteryRate>,
    #[serde(deserialize_with = "null_as_default")]
    pub cached_battery_readings: Vec<CachedBatteryReading>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_version: CURRENT_SETTINGS_VERSION,
            overlay_number: true,
            critical_threshold: 15,
            low_threshold: 25,
            icon_metric: ICON_METRIC_LOWEST.to_owned(),
            icon_device_id: String::new(),
            icon_device_name: String::new(),
            color_bars: true,
            show_popup_reset_times: true,
            show_popup_last_updated: true,
            show_additional_limits: true,
            show_reset_availability: true,
            start_with_windows: false,
            threshold_notifications: false,
            refresh_seconds: 60,
            idle_refresh_seconds: 0,
            show_tray_box: true,
            use_custom_tray_box_color: false,
            tray_box_color: DEFAULT_TRAY_BOX_COLOR.to_owned(),
            tray_text_color: DEFAULT_TRAY_TEXT_COLOR.to_owned(),
            theme: THEME_SYSTEM.to_owned(),
            learned_battery_rates: Vec::new(),
            cached_battery_readings: Vec::new(),
        }
    }
}

pub fn parse_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |text: &str| u8::from_str_radix(text, 16).ok();
    match hex.len() {
        6 => Some([channel(&hex[0..2])?, channel(&hex[2..4])?, channel(&hex[4..6])?]),
        3 => {
            let double = |index: usize| channel(&hex[index..=index]).map(|value| value * 17);
            Some([double(0)?, double(1)?, double(2)?])
        }
        _ => None,
    }
}

pub fn format_color(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

pub fn is_dark_theme(theme: &str) -> bool {
    if theme.eq_ignore_ascii_case(THEME_DARK) {
        true
    } else if theme.eq_ignore_ascii_case(THEME_LIGHT) {
        false
    } else {
        win::apps_use_dark_theme()
    }
}

impl Settings {
    pub fn tray_box_rgb(&self) -> [u8; 3] {
        parse_color(&self.tray_box_color).unwrap_or([0, 120, 215])
    }

    pub fn tray_text_rgb(&self) -> [u8; 3] {
        parse_color(&self.tray_text_color).unwrap_or([255, 255, 255])
    }

    pub fn editable_equals(&self, other: &Settings) -> bool {
        let device_metric = self.icon_metric.eq_ignore_ascii_case(ICON_METRIC_DEVICE);
        self.critical_threshold == other.critical_threshold
            && self.low_threshold == other.low_threshold
            && self.icon_metric.eq_ignore_ascii_case(&other.icon_metric)
            && (!device_metric || self.icon_device_id.to_lowercase() == other.icon_device_id.to_lowercase())
            && self.color_bars == other.color_bars
            && self.show_popup_reset_times == other.show_popup_reset_times
            && self.show_popup_last_updated == other.show_popup_last_updated
            && self.show_reset_availability == other.show_reset_availability
            && self.start_with_windows == other.start_with_windows
            && self.threshold_notifications == other.threshold_notifications
            && self.show_tray_box == other.show_tray_box
            && self.use_custom_tray_box_color == other.use_custom_tray_box_color
            && (!self.use_custom_tray_box_color
                || self.tray_box_color.eq_ignore_ascii_case(&other.tray_box_color))
            && self.tray_text_color.eq_ignore_ascii_case(&other.tray_text_color)
            && self.refresh_seconds == other.refresh_seconds
            && self.idle_refresh_seconds == other.idle_refresh_seconds
            && self.theme.eq_ignore_ascii_case(&other.theme)
    }

    fn normalize(&mut self, source_version: i32) {
        self.overlay_number = true;
        if source_version < 6 {
            self.show_additional_limits = true;
            self.show_reset_availability = true;
        }
        if !(1..=99).contains(&self.critical_threshold) {
            self.critical_threshold = 15;
        }
        if !(1..=99).contains(&self.low_threshold) {
            self.low_threshold = 25;
        }
        if self.critical_threshold > self.low_threshold {
            self.critical_threshold = self.low_threshold;
        }
        if self.icon_metric.is_empty() {
            self.icon_metric = ICON_METRIC_LOWEST.to_owned();
        }
        self.refresh_seconds = self.refresh_seconds.clamp(30, 3600);
        if self.idle_refresh_seconds < 0 {
            self.idle_refresh_seconds = 0;
        } else if self.idle_refresh_seconds > 7200 {
            self.idle_refresh_seconds = 7200;
        } else if self.idle_refresh_seconds > 0 && self.idle_refresh_seconds < self.refresh_seconds {
            self.idle_refresh_seconds = self.refresh_seconds;
        }
        if parse_color(&self.tray_box_color).is_none() {
            self.tray_box_color = DEFAULT_TRAY_BOX_COLOR.to_owned();
        }
        if parse_color(&self.tray_text_color).is_none() {
            self.tray_text_color = DEFAULT_TRAY_TEXT_COLOR.to_owned();
        }
        self.theme = if self.theme.eq_ignore_ascii_case(THEME_DARK) {
            THEME_DARK
        } else if self.theme.eq_ignore_ascii_case(THEME_LIGHT) {
            THEME_LIGHT
        } else {
            THEME_SYSTEM
        }
        .to_owned();
    }
}

#[derive(PartialEq)]
enum FileResult {
    Missing,
    Invalid,
    Loaded,
    NewerVersion,
}

pub struct SettingsStore {
    pub values: Settings,
    path: PathBuf,
    protected_version: Option<i32>,
    protected_path: PathBuf,
    observed_path: Option<PathBuf>,
    observed_contents: String,
    observed_exists: bool,
    automatic_save_blocked: bool,
}

pub fn default_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(SETTINGS_FOLDER).join(SETTINGS_FILE))
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
}

fn version_of(values: &Map<String, Value>) -> Option<i32> {
    let (_, raw) = values.iter().find(|(key, _)| key.eq_ignore_ascii_case("SettingsVersion"))?;
    let version = match raw {
        Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|value| value.round() as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        Value::Bool(flag) => i64::from(*flag),
        _ => return None,
    };
    i32::try_from(version).ok().filter(|version| *version >= 0)
}

fn read_version(path: &Path) -> Option<i32> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(values) => version_of(&values),
        _ => None,
    }
}

fn deserialize<T: DeserializeOwned>(values: &Map<String, Value>) -> Option<T> {
    serde_json::from_value(Value::Object(values.clone())).ok()
}

fn deserialize_settings(values: &Map<String, Value>) -> Option<(Settings, bool)> {
    if let Some(settings) = deserialize::<Settings>(values) {
        return Some((settings, false));
    }
    let optional: Vec<String> = values
        .keys()
        .filter(|key| OPTIONAL_KEYS.iter().any(|name| key.eq_ignore_ascii_case(name)))
        .cloned()
        .collect();
    for key in &optional {
        let mut without = values.clone();
        without.remove(key);
        if let Some(settings) = deserialize::<Settings>(&without) {
            return Some((settings, true));
        }
    }
    if optional.len() < 2 {
        return None;
    }
    let mut without = values.clone();
    for key in &optional {
        without.remove(key);
    }
    deserialize::<Settings>(&without).map(|settings| (settings, true))
}

fn newer_version_message(version: i32, path: &Path) -> String {
    format!(
        "Settings were created by a newer app version (schema {version}). This version supports schema {CURRENT_SETTINGS_VERSION} and will not overwrite the file: {}",
        path.display()
    )
}

impl SettingsStore {
    fn empty(path: &Path) -> Self {
        Self {
            values: Settings::default(),
            path: path.to_owned(),
            protected_version: None,
            protected_path: PathBuf::new(),
            observed_path: None,
            observed_contents: String::new(),
            observed_exists: false,
            automatic_save_blocked: false,
        }
    }

    pub fn load(path: &Path) -> Self {
        let (primary, loaded) = Self::try_load_file(path);
        if let Some(store) = loaded {
            return store;
        }
        let backup = backup_path(path);
        let (backup_result, loaded) = Self::try_load_file(&backup);
        if let Some(mut store) = loaded {
            store.path = path.to_owned();
            store.automatic_save_blocked = true;
            return store;
        }
        let mut store = Self::empty(path);
        store.observed_path = Some(path.to_owned());
        store.automatic_save_blocked = primary != FileResult::Missing || backup_result != FileResult::Missing;
        store
    }

    fn try_load_file(path: &Path) -> (FileResult, Option<Self>) {
        if !path.exists() {
            return (FileResult::Missing, None);
        }
        let Ok(json) = std::fs::read_to_string(path) else { return (FileResult::Invalid, None) };
        let Ok(Value::Object(values)) = serde_json::from_str::<Value>(&json) else {
            return (FileResult::Invalid, None);
        };
        let version = version_of(&values);
        let source_version = version.unwrap_or(0);
        let mut store = Self::empty(path);
        store.observed_path = Some(path.to_owned());
        store.observed_contents = json;
        store.observed_exists = true;

        if source_version > CURRENT_SETTINGS_VERSION {
            let (mut settings, ignored) = deserialize_settings(&values).unwrap_or_default();
            settings.normalize(source_version);
            settings.settings_version = source_version;
            store.values = settings;
            store.protected_version = Some(source_version);
            store.protected_path = path.to_owned();
            store.automatic_save_blocked = ignored;
            return (FileResult::NewerVersion, Some(store));
        }

        let Some((mut settings, ignored)) = deserialize_settings(&values) else {
            return (FileResult::Invalid, None);
        };
        settings.normalize(source_version);
        if version.is_none() && settings.theme == THEME_DARK {
            settings.theme = THEME_SYSTEM.to_owned();
        }
        settings.settings_version = CURRENT_SETTINGS_VERSION;
        store.values = settings;
        store.automatic_save_blocked = ignored;
        (FileResult::Loaded, Some(store))
    }

    pub fn save_blocked_message(&self) -> Option<String> {
        self.protected_version.map(|version| newer_version_message(version, &self.protected_path))
    }

    pub fn save(&mut self, mut candidate: Settings) -> Result<(), String> {
        if let Some(message) = self.save_blocked_message() {
            return Err(message);
        }
        let backup = backup_path(&self.path);
        for existing in [&self.path, &backup] {
            if let Some(version) = read_version(existing).filter(|version| *version > CURRENT_SETTINGS_VERSION) {
                return Err(newer_version_message(version, existing));
            }
        }
        let directory = self.path.parent().ok_or("Could not determine the settings folder.")?;
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        candidate.settings_version = CURRENT_SETTINGS_VERSION;
        let json = serde_json::to_string(&candidate)
            .map_err(|error| error.to_string())?
            .replace("\"/Date(", "\"\\/Date(")
            .replace(")/\"", ")\\/\"");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let mut temp_name = self.path.as_os_str().to_owned();
        temp_name.push(format!(".{:032x}.tmp", unique ^ (u128::from(std::process::id()) << 96)));
        let temp = PathBuf::from(temp_name);

        let written = write_new_file(&temp, &json).and_then(|()| {
            if self.path.exists() {
                win::replace_file(&self.path, &temp, &backup)
            } else {
                std::fs::rename(&temp, &self.path)
            }
        });
        if temp.exists() {
            let _ = std::fs::remove_file(&temp);
        }
        written.map_err(|error| error.to_string())?;

        self.values = candidate;
        self.observed_path = Some(self.path.clone());
        self.observed_contents = json;
        self.observed_exists = true;
        self.automatic_save_blocked = false;
        Ok(())
    }

    fn automatic_save_allowed(&self) -> bool {
        if self.automatic_save_blocked || self.protected_version.is_some() {
            return false;
        }
        if !self.observed_path.as_deref().is_some_and(|observed| same_path(observed, &self.path)) {
            return false;
        }
        let exists = self.path.exists();
        if exists != self.observed_exists {
            return false;
        }
        !exists || std::fs::read_to_string(&self.path).is_ok_and(|contents| contents == self.observed_contents)
    }

    pub fn try_save_rates(&mut self, rates: Vec<LearnedBatteryRate>) -> bool {
        if !self.automatic_save_allowed() {
            return false;
        }
        let mut candidate = self.values.clone();
        candidate.learned_battery_rates = rates;
        self.save(candidate).is_ok()
    }

    pub fn try_save_cache(&mut self, readings: Vec<CachedBatteryReading>) -> bool {
        if !self.automatic_save_allowed() {
            return false;
        }
        let mut candidate = self.values.clone();
        candidate.cached_battery_readings = readings;
        self.save(candidate).is_ok()
    }
}

fn write_new_file(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate::LearnedBatteryRateSample;

    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(name: &str) -> Self {
            let folder = std::env::temp_dir().join(format!(
                "bbm-rust-settings-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).unwrap();
            Self(folder)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const ORIGINAL_JSON: &str = r##"{"SettingsVersion":9,"OverlayNumber":true,"CriticalThreshold":10,"LowThreshold":30,"IconMetric":"Device","IconDeviceId":"hyperx:\\\\?\\hid#vid_03f0","IconDeviceName":"HyperX Cloud III S Wireless","ColorBars":true,"ShowPopupResetTimes":true,"ShowPopupLastUpdated":false,"ShowAdditionalLimits":true,"ShowResetAvailability":true,"StartWithWindows":true,"ThresholdNotifications":true,"RefreshSeconds":120,"IdleRefreshSeconds":0,"ShowTrayBox":true,"UseCustomTrayBoxColor":false,"TrayBoxColor":"#0078D7","TrayTextColor":"#FFFFFF","Theme":"Dark Mode","LearnedBatteryRates":[{"DeviceId":"hyperx:path","ChargeSamples":[],"DischargeSamples":[{"Percent":4,"Seconds":3605.5}]}],"CachedBatteryReadings":[{"Id":"hyperx:path","PhysicalId":null,"Name":"HyperX Cloud III S Wireless","Source":"HyperX 2.4 GHz","Percent":53,"Priority":30,"ObservedAt":"\/Date(1762790400000)\/"}]}"##;

    #[test]
    fn original_app_settings_parse_and_round_trip() {
        let folder = TempFolder::new("roundtrip");
        let path = folder.file("settings.json");
        std::fs::write(&path, ORIGINAL_JSON).unwrap();
        let mut store = SettingsStore::load(&path);
        assert_eq!(store.values.critical_threshold, 10);
        assert_eq!(store.values.icon_metric, ICON_METRIC_DEVICE);
        assert_eq!(store.values.icon_device_id, "hyperx:\\\\?\\hid#vid_03f0");
        assert_eq!(store.values.theme, THEME_DARK);
        assert_eq!(
            store.values.learned_battery_rates[0].discharge_samples,
            vec![LearnedBatteryRateSample { percent: 4.0, seconds: 3605.5 }]
        );
        assert_eq!(store.values.cached_battery_readings[0].observed_at, 1_762_790_400_000);
        assert_eq!(store.values.cached_battery_readings[0].physical_id, None);

        let mut candidate = store.values.clone();
        candidate.low_threshold = 40;
        store.save(candidate).unwrap();
        assert!(backup_path(&path).exists());
        let reloaded = SettingsStore::load(&path);
        assert_eq!(reloaded.values, store.values);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains(r#""ObservedAt":"\/Date(1762790400000)\/""#));
        assert!(written.contains(r#""IconDeviceId":"hyperx:\\\\?\\hid#vid_03f0""#));
        assert!(store.values.start_with_windows);
    }

    #[test]
    fn a_missing_file_starts_from_defaults_and_is_created_on_save() {
        let folder = TempFolder::new("fresh");
        let path = folder.file("nested").join("settings.json");
        let mut store = SettingsStore::load(&path);
        assert_eq!(store.values, Settings::default());
        assert!(store.try_save_cache(Vec::new()));
        assert!(path.exists());
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn out_of_range_values_are_normalized() {
        let folder = TempFolder::new("normalize");
        let path = folder.file("settings.json");
        std::fs::write(
            &path,
            r#"{"SettingsVersion":3,"CriticalThreshold":80,"LowThreshold":20,"RefreshSeconds":5,"IdleRefreshSeconds":40,"TrayBoxColor":"nope","TrayTextColor":null,"Theme":"dark mode","IconMetric":"","ShowResetAvailability":false}"#,
        )
        .unwrap();
        let values = SettingsStore::load(&path).values;
        assert_eq!((values.critical_threshold, values.low_threshold), (20, 20));
        assert_eq!((values.refresh_seconds, values.idle_refresh_seconds), (30, 40));
        assert_eq!(values.tray_box_color, DEFAULT_TRAY_BOX_COLOR);
        assert_eq!(values.tray_text_color, DEFAULT_TRAY_TEXT_COLOR);
        assert_eq!(values.theme, THEME_DARK);
        assert_eq!(values.icon_metric, ICON_METRIC_LOWEST);
        assert!(values.show_reset_availability);
        assert_eq!(values.settings_version, CURRENT_SETTINGS_VERSION);
    }

    #[test]
    fn newer_schema_files_are_never_overwritten() {
        let folder = TempFolder::new("newer");
        let path = folder.file("settings.json");
        let json = r#"{"SettingsVersion":42,"RefreshSeconds":90}"#;
        std::fs::write(&path, json).unwrap();
        let mut store = SettingsStore::load(&path);
        assert_eq!(store.values.refresh_seconds, 90);
        assert!(store.save_blocked_message().unwrap().contains("schema 42"));
        assert!(store.save(Settings::default()).is_err());
        assert!(!store.try_save_rates(Vec::new()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), json);
    }

    #[test]
    fn automatic_saves_stop_when_the_file_changed_underneath() {
        let folder = TempFolder::new("observed");
        let path = folder.file("settings.json");
        let mut store = SettingsStore::load(&path);
        assert!(store.try_save_cache(Vec::new()));
        std::fs::write(&path, "{\"SettingsVersion\":9,\"RefreshSeconds\":45}").unwrap();
        assert!(!store.try_save_rates(Vec::new()));
        assert!(store.save(store.values.clone()).is_ok());
        assert!(store.try_save_rates(Vec::new()));
    }

    #[test]
    fn a_corrupt_file_falls_back_to_its_backup_and_blocks_automatic_saves() {
        let folder = TempFolder::new("backup");
        let path = folder.file("settings.json");
        std::fs::write(&path, "not json").unwrap();
        std::fs::write(backup_path(&path), "{\"SettingsVersion\":9,\"RefreshSeconds\":75}").unwrap();
        let mut store = SettingsStore::load(&path);
        assert_eq!(store.values.refresh_seconds, 75);
        assert!(!store.try_save_cache(Vec::new()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
    }

    #[test]
    fn malformed_history_is_ignored_but_preferences_survive() {
        let folder = TempFolder::new("malformed");
        let path = folder.file("settings.json");
        std::fs::write(&path, r#"{"SettingsVersion":9,"RefreshSeconds":200,"LearnedBatteryRates":"broken"}"#).unwrap();
        let mut store = SettingsStore::load(&path);
        assert_eq!(store.values.refresh_seconds, 200);
        assert!(store.values.learned_battery_rates.is_empty());
        assert!(!store.try_save_rates(Vec::new()));
    }

    #[test]
    fn colours_and_dates_parse_strictly() {
        assert_eq!(parse_color("#0078D7"), Some([0, 120, 215]));
        assert_eq!(parse_color("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_color("0078D7"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(format_color([0, 194, 186]), "#00C2BA");
        assert_eq!(parse_date("/Date(1762790400000)/"), Some(1_762_790_400_000));
        assert_eq!(parse_date("/Date(-62135596800000)/"), Some(MINIMUM_OBSERVED_AT));
        assert_eq!(parse_date("/Date(1762790400000-0500)/"), Some(1_762_790_400_000));
        assert_eq!(parse_date("2026-11-10"), None);
    }
}
