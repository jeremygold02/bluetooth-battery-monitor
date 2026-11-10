use crate::devices::is_obsolete_superlight_wired_slot;
use crate::probe::ProbeResult;
use crate::rate::{LearnedBatteryRate, RateInput, RateTracker, has_stable_id, is_physical_id, same_id};
use crate::settings::{CachedBatteryReading, MINIMUM_OBSERVED_AT};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

const MAXIMUM_CACHED_DEVICES: usize = 512;
const MAXIMUM_TEXT_LENGTH: usize = 512;
const CACHE_REVISION_INTERVAL_MS: i64 = 15 * 60 * 1000;
const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1000;
const LOGITECH_FALLBACK_NAME: &str = "Logitech wireless device";

static GENERATED_IDS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub physical_id: Option<String>,
    pub name: String,
    pub percent: Option<u8>,
    pub source: String,
    pub status: String,
    pub priority: i32,
    pub observed_at: i64,
    pub cached: bool,
    pub estimate: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchResult {
    pub devices: Vec<Device>,
    pub lowest: Option<u8>,
    pub average: Option<u8>,
    pub status_message: Option<String>,
}

pub struct Monitor {
    cache: Vec<Device>,
    last_cache_revision: Vec<(String, i64)>,
    cache_revision: u64,
    rates: RateTracker,
    format_date_time: fn(i64) -> String,
}

fn generated_id(attempted_at: i64) -> String {
    let counter = GENERATED_IDS.fetch_add(1, Ordering::Relaxed);
    format!("device:{:016x}{:016x}", attempted_at as u64, counter)
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn is_charging(status: &str) -> bool {
    status.get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case("Charging"))
}

fn score(device: &Device) -> i32 {
    (if device.percent.is_some() { 1000 } else { 0 }) + (if device.cached { -100 } else { 0 }) + device.priority
}

fn merge_key(device: &Device) -> String {
    let name = normalize_name(&device.name);
    if !name.is_empty() && !name.contains("wireless device") && !name.contains("usb receiver") {
        format!("name:{name}")
    } else {
        format!("id:{}", device.id.to_lowercase())
    }
}

fn round_half_even(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if (fraction - 0.5).abs() < f64::EPSILON {
        if floor % 2.0 == 0.0 { floor } else { floor + 1.0 }
    } else {
        value.round()
    }
}

impl Monitor {
    pub fn new(decimal_separator: &str, format_date_time: fn(i64) -> String) -> Self {
        Self {
            cache: Vec::new(),
            last_cache_revision: Vec::new(),
            cache_revision: 0,
            rates: RateTracker::new(decimal_separator),
            format_date_time,
        }
    }

    pub fn initialize_rates(&mut self, history: &[LearnedBatteryRate]) {
        self.rates.import_history(history);
    }

    pub fn reset_rates(&mut self) {
        self.rates.reset();
    }

    pub fn updated_rates(&self, saved_revision: u64) -> Option<(u64, Vec<LearnedBatteryRate>)> {
        let revision = self.rates.revision();
        (revision > saved_revision).then(|| (revision, self.rates.export_history()))
    }

    pub fn initialize_cache(&mut self, readings: &[CachedBatteryReading], now: i64) {
        self.cache.clear();
        self.last_cache_revision.clear();
        self.cache_revision = 0;
        for reading in readings {
            if is_obsolete_superlight_wired_slot(&reading.id) {
                self.cache_revision += 1;
                continue;
            }
            let valid = has_stable_id(&reading.id)
                && (0..=100).contains(&reading.percent)
                && !reading.name.trim().is_empty()
                && reading.name.chars().count() <= MAXIMUM_TEXT_LENGTH
                && !reading.source.trim().is_empty()
                && reading.source.chars().count() <= MAXIMUM_TEXT_LENGTH
                && (-100..=100).contains(&reading.priority)
                && reading.observed_at != MINIMUM_OBSERVED_AT
                && reading.observed_at <= now + ONE_DAY_MS
                && self.cached_index(&reading.id).is_none();
            if !valid {
                continue;
            }
            self.cache.push(Device {
                id: reading.id.clone(),
                physical_id: reading.physical_id.clone().filter(|value| is_physical_id(Some(value))),
                name: reading.name.clone(),
                percent: u8::try_from(reading.percent).ok(),
                source: reading.source.clone(),
                priority: reading.priority,
                observed_at: reading.observed_at,
                ..Default::default()
            });
            self.last_cache_revision.push((reading.id.clone(), reading.observed_at));
            if self.cache.len() >= MAXIMUM_CACHED_DEVICES {
                break;
            }
        }
    }

    pub fn updated_cache(&self, saved_revision: u64) -> Option<(u64, Vec<CachedBatteryReading>)> {
        if self.cache_revision <= saved_revision {
            return None;
        }
        let mut readings: Vec<CachedBatteryReading> = self
            .cache
            .iter()
            .filter_map(|device| {
                Some(CachedBatteryReading {
                    id: device.id.clone(),
                    physical_id: device.physical_id.clone(),
                    name: device.name.clone(),
                    source: device.source.clone(),
                    percent: i32::from(device.percent?),
                    priority: device.priority,
                    observed_at: device.observed_at,
                })
            })
            .collect();
        readings.sort_by_key(|reading| reading.id.to_lowercase());
        Some((self.cache_revision, readings))
    }

    pub fn fetch(
        &mut self,
        collected: Vec<ProbeResult>,
        errors: &[String],
        attempted_at: i64,
        monotonic_seconds: f64,
    ) -> FetchResult {
        let mut devices = self.merge_and_cache(collected, attempted_at);
        let inputs: Vec<RateInput<'_>> = devices
            .iter()
            .map(|device| RateInput {
                id: &device.id,
                physical_id: device.physical_id.as_deref(),
                percent: device.percent,
                status: &device.status,
                cached: device.cached,
            })
            .collect();
        let estimates = self.rates.update(&inputs, monotonic_seconds);
        for (device, estimate) in devices.iter_mut().zip(estimates) {
            device.estimate = estimate;
        }
        devices.sort_by(|left, right| {
            right
                .percent
                .is_some()
                .cmp(&left.percent.is_some())
                .then(left.percent.cmp(&right.percent))
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });

        let known: Vec<u8> = devices.iter().filter_map(|device| device.percent).collect();
        let total: u32 = known.iter().map(|percent| u32::from(*percent)).sum();
        let status_message = if devices.is_empty() {
            Some(if errors.is_empty() {
                "No supported wireless battery devices found."
            } else {
                "No supported devices found; one or more checks failed."
            })
        } else if errors.is_empty() {
            None
        } else {
            Some("Some device types could not be checked.")
        };
        FetchResult {
            lowest: known.iter().copied().min(),
            average: (!known.is_empty()).then(|| round_half_even(f64::from(total) / known.len() as f64) as u8),
            status_message: status_message.map(str::to_owned),
            devices,
        }
    }

    fn cached_index(&self, id: &str) -> Option<usize> {
        self.cache.iter().position(|device| same_id(&device.id, id))
    }

    fn cached_status(&self, observed_at: i64) -> String {
        format!("Last reported {} \u{00b7} sleeping or unavailable", (self.format_date_time)(observed_at))
    }

    fn remember(&mut self, value: &Device) {
        if !has_stable_id(&value.id)
            || value.name.chars().count() > MAXIMUM_TEXT_LENGTH
            || value.source.chars().count() > MAXIMUM_TEXT_LENGTH
            || !(-100..=100).contains(&value.priority)
        {
            return;
        }
        let existing = self.cached_index(&value.id);
        if existing.is_none() && self.cache.len() >= MAXIMUM_CACHED_DEVICES {
            return;
        }
        let revision_index = self.last_cache_revision.iter().position(|(id, _)| same_id(id, &value.id));
        let revision_due = match (existing, revision_index) {
            (Some(index), Some(revision)) => {
                let previous = &self.cache[index];
                previous.percent != value.percent
                    || previous.name != value.name
                    || previous.physical_id != value.physical_id
                    || previous.source != value.source
                    || previous.priority != value.priority
                    || value.observed_at - self.last_cache_revision[revision].1 >= CACHE_REVISION_INTERVAL_MS
            }
            _ => true,
        };
        match existing {
            Some(index) => self.cache[index] = value.clone(),
            None => self.cache.push(value.clone()),
        }
        if revision_due {
            self.cache_revision += 1;
            match revision_index {
                Some(revision) => self.last_cache_revision[revision].1 = value.observed_at,
                None => self.last_cache_revision.push((value.id.clone(), value.observed_at)),
            }
        }
    }

    fn merge_and_cache(&mut self, collected: Vec<ProbeResult>, attempted_at: i64) -> Vec<Device> {
        let mut hydrated: Vec<Device> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for probe in collected {
            let percent = probe.percent.map(|percent| percent.min(100));
            let blank = |text: &str| text.trim().is_empty();
            let mut current = Device {
                id: if blank(&probe.id) { generated_id(attempted_at) } else { probe.id },
                physical_id: probe.physical_id,
                name: if blank(&probe.name) { "Wireless device".to_owned() } else { probe.name },
                percent,
                source: if blank(&probe.source) { "Windows".to_owned() } else { probe.source },
                status: if blank(&probe.status) {
                    if percent.is_some() { "Connected" } else { "Unavailable" }.to_owned()
                } else {
                    probe.status
                },
                priority: probe.priority,
                observed_at: attempted_at,
                cached: false,
                estimate: None,
            };
            seen.push(current.id.to_lowercase());
            if current.percent.is_some() {
                self.remember(&current);
            } else if let Some(index) = self.cached_index(&current.id) {
                let cached = &self.cache[index];
                current.percent = cached.percent;
                current.observed_at = cached.observed_at;
                current.cached = true;
                current.physical_id = cached.physical_id.clone();
                current.status = self.cached_status(cached.observed_at);
                if current.name.eq_ignore_ascii_case(LOGITECH_FALLBACK_NAME) && !blank(&cached.name) {
                    current.name = cached.name.clone();
                }
            }
            hydrated.push(current);
        }
        for cached in &self.cache {
            if seen.contains(&cached.id.to_lowercase()) {
                continue;
            }
            let mut missing = cached.clone();
            missing.cached = true;
            missing.status = self.cached_status(cached.observed_at);
            missing.estimate = None;
            hydrated.push(missing);
        }

        let mut first_ids: Vec<(String, String)> = Vec::new();
        let mut ambiguous: Vec<String> = Vec::new();
        for device in &hydrated {
            let name = normalize_name(&device.name);
            let key = format!("{name}\0{}", device.source.to_lowercase());
            match first_ids.iter().find(|(existing, _)| *existing == key) {
                Some((_, id)) if !same_id(id, &device.id) => ambiguous.push(name),
                Some(_) => {}
                None => first_ids.push((key, device.id.clone())),
            }
        }

        let mut merged: Vec<(String, Device)> = Vec::new();
        for candidate in hydrated {
            let physical = candidate.physical_id.as_deref().filter(|value| is_physical_id(Some(value)));
            let key = match physical {
                Some(physical) => format!("physical:{}", physical.to_lowercase()),
                None if ambiguous.contains(&normalize_name(&candidate.name)) => {
                    format!("id:{}", candidate.id.to_lowercase())
                }
                None => merge_key(&candidate),
            };
            match merged.iter().position(|(existing, _)| *existing == key) {
                None => merged.push((key, candidate)),
                Some(index) => {
                    let existing = &merged[index].1;
                    let same_unit = physical.is_some()
                        && existing
                            .physical_id
                            .as_deref()
                            .is_some_and(|value| physical.is_some_and(|physical| same_id(value, physical)));
                    let replaces = score(&candidate) > score(existing)
                        || (score(&candidate) == score(existing)
                            && same_unit
                            && !candidate.cached
                            && is_charging(&candidate.status)
                            && !is_charging(&existing.status));
                    if replaces {
                        merged[index].1 = candidate;
                    }
                }
            }
        }
        merged.into_iter().map(|(_, device)| device).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_762_790_400_000;

    fn stamp(value: i64) -> String {
        format!("t{value}")
    }

    fn monitor() -> Monitor {
        Monitor::new(".", stamp)
    }

    fn probe(id: &str, name: &str, percent: Option<u8>, status: &str) -> ProbeResult {
        ProbeResult {
            id: id.to_owned(),
            physical_id: None,
            name: name.to_owned(),
            percent,
            source: "Logitech LIGHTSPEED".to_owned(),
            status: status.to_owned(),
            priority: 30,
        }
    }

    #[test]
    fn a_higher_live_reading_replaces_the_cache_and_survives_absence() {
        let mut monitor = monitor();
        monitor.fetch(vec![probe("logitech:a:1", "Mouse", Some(35), "Discharging")], &[], NOW, 1.0);
        let live = monitor.fetch(vec![probe("logitech:a:1", "Mouse", Some(100), "Charging")], &[], NOW + 60_000, 61.0);
        assert_eq!(live.devices[0].percent, Some(100));
        assert!(!live.devices[0].cached);
        let (revision, cache) = monitor.updated_cache(0).unwrap();
        assert_eq!(cache[0].percent, 100);
        assert_eq!(cache[0].observed_at, NOW + 60_000);
        assert!(monitor.updated_cache(revision).is_none());

        let absent = monitor.fetch(Vec::new(), &[], NOW + 120_000, 121.0);
        assert_eq!(absent.devices.len(), 1);
        assert_eq!(absent.devices[0].percent, Some(100));
        assert!(absent.devices[0].cached);
        assert_eq!(absent.devices[0].status, format!("Last reported t{} \u{00b7} sleeping or unavailable", NOW + 60_000));
        assert_eq!(absent.status_message, None);
    }

    #[test]
    fn a_sleeping_receiver_shows_its_last_reading_and_name() {
        let mut monitor = monitor();
        monitor.fetch(vec![probe("logitech:a:1", "PRO X SUPERLIGHT 2c", Some(64), "Discharging")], &[], NOW, 1.0);
        let asleep = monitor.fetch(
            vec![probe("logitech:a:1", LOGITECH_FALLBACK_NAME, None, "Sleeping or unavailable")],
            &[],
            NOW + 60_000,
            61.0,
        );
        assert_eq!(asleep.devices.len(), 1);
        assert_eq!(asleep.devices[0].name, "PRO X SUPERLIGHT 2c");
        assert_eq!(asleep.devices[0].percent, Some(64));
        assert!(asleep.devices[0].cached);
        assert_eq!(asleep.devices[0].observed_at, NOW);
    }

    #[test]
    fn wired_and_wireless_readings_of_one_unit_merge_and_prefer_charging() {
        let mut monitor = monitor();
        let unit = Some("logitech-unit:010203040506AABBCCDD".to_owned());
        let mut wireless = probe("logitech:receiver:1", "PRO X SUPERLIGHT 2c", Some(70), "Discharging");
        wireless.physical_id = unit.clone();
        let mut wired = probe("logitech:cable:1", "PRO X SUPERLIGHT 2c", Some(70), "Charging");
        wired.physical_id = unit;
        let result = monitor.fetch(vec![wireless, wired], &[], NOW, 1.0);
        assert_eq!(result.devices.len(), 1);
        assert_eq!(result.devices[0].id, "logitech:cable:1");
        assert_eq!(result.devices[0].status, "Charging");
    }

    #[test]
    fn two_devices_with_one_name_and_source_stay_separate() {
        let mut monitor = monitor();
        let result = monitor.fetch(
            vec![
                probe("logitech:a:1", "G305", Some(40), "Discharging"),
                probe("logitech:b:1", "G305", Some(90), "Discharging"),
            ],
            &[],
            NOW,
            1.0,
        );
        assert_eq!(result.devices.len(), 2);
        assert_eq!((result.lowest, result.average), (Some(40), Some(65)));
    }

    #[test]
    fn one_device_seen_through_two_sources_keeps_the_better_reading() {
        let mut monitor = monitor();
        let mut windows = probe("windows:x", "Headset", Some(50), "Connected");
        windows.source = "Windows".to_owned();
        windows.priority = 10;
        let result = monitor.fetch(vec![windows, probe("hyperx:y", "headset", Some(52), "Not charging")], &[], NOW, 1.0);
        assert_eq!(result.devices.len(), 1);
        assert_eq!(result.devices[0].id, "hyperx:y");
    }

    #[test]
    fn devices_sort_by_level_then_name_with_unknown_levels_last() {
        let mut monitor = monitor();
        let result = monitor.fetch(
            vec![
                probe("c", "Zed", Some(20), "Discharging"),
                probe("a", "Unknown", None, "Sleeping or unavailable"),
                probe("b", "alpha", Some(20), "Discharging"),
                probe("d", "Mid", Some(5), "Discharging"),
            ],
            &["Bluetooth LE: failed".to_owned()],
            NOW,
            1.0,
        );
        let names: Vec<&str> = result.devices.iter().map(|device| device.name.as_str()).collect();
        assert_eq!(names, ["Mid", "alpha", "Zed", "Unknown"]);
        assert_eq!(result.status_message.as_deref(), Some("Some device types could not be checked."));
        assert_eq!((result.lowest, result.average), (Some(5), Some(15)));
    }

    #[test]
    fn empty_results_explain_whether_checks_failed() {
        let mut monitor = monitor();
        assert_eq!(
            monitor.fetch(Vec::new(), &[], NOW, 1.0).status_message.as_deref(),
            Some("No supported wireless battery devices found.")
        );
        assert_eq!(
            monitor.fetch(Vec::new(), &["Windows: failed".to_owned()], NOW, 2.0).status_message.as_deref(),
            Some("No supported devices found; one or more checks failed.")
        );
    }

    #[test]
    fn saved_cache_entries_are_validated_and_obsolete_wired_slots_are_dropped() {
        let reading = |id: &str, percent: i32, observed_at: i64| CachedBatteryReading {
            id: id.to_owned(),
            physical_id: Some("not-a-unit".to_owned()),
            name: "Mouse".to_owned(),
            source: "Logitech LIGHTSPEED".to_owned(),
            percent,
            priority: 30,
            observed_at,
        };
        let mut monitor = monitor();
        monitor.initialize_cache(
            &[
                reading("logitech:\\\\?\\hid#vid_046d&pid_c09f#x:3", 80, NOW),
                reading("logitech:\\\\?\\hid#vid_046d&pid_c09f#x:1", 80, NOW),
                reading("LOGITECH:\\\\?\\HID#VID_046D&PID_C09F#X:1", 70, NOW),
                reading("device:0123456789abcdef0123456789abcdef", 50, NOW),
                reading("too-high", 101, NOW),
                reading("never-seen", 50, MINIMUM_OBSERVED_AT),
                reading("from-the-future", 50, NOW + 2 * ONE_DAY_MS),
            ],
            NOW,
        );
        let (_, cache) = monitor.updated_cache(0).unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].percent, 80);
        assert_eq!(cache[0].physical_id, None);
        let shown = monitor.fetch(Vec::new(), &[], NOW, 1.0);
        assert_eq!(shown.devices.len(), 1);
        assert!(shown.devices[0].cached);
    }

    #[test]
    fn unchanged_readings_only_revise_the_cache_every_fifteen_minutes() {
        let mut monitor = monitor();
        let reading = || vec![probe("logitech:a:1", "Mouse", Some(50), "Discharging")];
        monitor.fetch(reading(), &[], NOW, 1.0);
        let (first, _) = monitor.updated_cache(0).unwrap();
        monitor.fetch(reading(), &[], NOW + 60_000, 61.0);
        assert!(monitor.updated_cache(first).is_none());
        monitor.fetch(reading(), &[], NOW + CACHE_REVISION_INTERVAL_MS, 901.0);
        assert!(monitor.updated_cache(first).is_some());
    }

    #[test]
    fn averages_round_half_to_even_like_the_original() {
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(3.5), 4.0);
        assert_eq!(round_half_even(2.4), 2.0);
        assert_eq!(round_half_even(2.6), 3.0);
    }
}
