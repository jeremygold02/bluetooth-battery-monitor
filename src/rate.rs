use serde::{Deserialize, Serialize};

const MAX_GAP_SECONDS: f64 = 7200.0;
const MINIMUM_PERIOD_SECONDS: f64 = 1800.0;
const MINIMUM_CHANGE_PERCENT: i32 = 3;
const MAX_SEGMENT_SECONDS: f64 = 7.0 * 24.0 * 3600.0;
const MAX_RATE_PER_HOUR: f64 = 200.0;
const MAX_HISTORY_DEVICES: usize = 512;
const MAX_HISTORY_SAMPLES_PER_DIRECTION: usize = 20;
const MAX_DEVICE_ID_LENGTH: usize = 2048;
const PHYSICAL_ID_PREFIX: &str = "logitech-unit:";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LearnedBatteryRateSample {
    pub percent: f64,
    pub seconds: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LearnedBatteryRate {
    #[serde(deserialize_with = "crate::settings::null_as_default")]
    pub device_id: String,
    #[serde(deserialize_with = "crate::settings::null_as_default")]
    pub charge_samples: Vec<LearnedBatteryRateSample>,
    #[serde(deserialize_with = "crate::settings::null_as_default")]
    pub discharge_samples: Vec<LearnedBatteryRateSample>,
}

pub struct RateInput<'a> {
    pub id: &'a str,
    pub physical_id: Option<&'a str>,
    pub percent: Option<u8>,
    pub status: &'a str,
    pub cached: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Unknown,
    Charging,
    Discharging,
    FullOrError,
}

struct Reading {
    start_percent: i32,
    last_percent: i32,
    start_time: f64,
    last_time: f64,
    state: State,
    direction: i32,
    samples: u32,
}

impl Reading {
    fn new(percent: i32, state: State, now: f64) -> Self {
        Self {
            start_percent: percent,
            last_percent: percent,
            start_time: now,
            last_time: now,
            state,
            direction: 0,
            samples: 1,
        }
    }
}

#[derive(Default)]
pub struct RateTracker {
    readings: Vec<(String, Reading)>,
    history: Vec<LearnedBatteryRate>,
    revision: u64,
    decimal_separator: String,
}

pub fn is_physical_id(value: Option<&str>) -> bool {
    let Some(value) = value else { return false };
    value.len() == PHYSICAL_ID_PREFIX.len() + 20
        && value.starts_with(PHYSICAL_ID_PREFIX)
        && value[PHYSICAL_ID_PREFIX.len()..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn has_stable_id(id: &str) -> bool {
    if id.trim().is_empty() || id.chars().count() > MAX_DEVICE_ID_LENGTH {
        return false;
    }
    let generated = id
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("device:"))
        && id.len() == 39
        && id[7..].bytes().all(|byte| byte.is_ascii_hexdigit());
    !generated
}

pub fn same_id(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

pub fn format_duration(total_seconds: i64) -> String {
    let total = total_seconds.max(0);
    let days = total / 86_400;
    let hours = total / 3600;
    let minutes = total / 60;
    if days >= 1 {
        format!("{days}d {}h", (total % 86_400) / 3600)
    } else if hours >= 1 {
        format!("{hours}h {}m", (total % 3600) / 60)
    } else if minutes >= 1 {
        format!("{minutes}m")
    } else {
        format!("{total}s")
    }
}

impl RateTracker {
    pub fn new(decimal_separator: &str) -> Self {
        Self { decimal_separator: decimal_separator.to_owned(), ..Self::default() }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn import_history(&mut self, values: &[LearnedBatteryRate]) {
        self.readings.clear();
        self.history.clear();
        self.revision = 0;
        for value in values {
            if !has_stable_id(&value.device_id) || self.history_index(&value.device_id).is_some() {
                continue;
            }
            let copy = LearnedBatteryRate {
                device_id: value.device_id.clone(),
                charge_samples: valid_samples(&value.charge_samples),
                discharge_samples: valid_samples(&value.discharge_samples),
            };
            if copy.charge_samples.is_empty() && copy.discharge_samples.is_empty() {
                continue;
            }
            self.history.push(copy);
            if self.history.len() >= MAX_HISTORY_DEVICES {
                break;
            }
        }
    }

    pub fn export_history(&self) -> Vec<LearnedBatteryRate> {
        let mut result = self.history.clone();
        result.sort_by_key(|value| value.device_id.to_lowercase());
        result
    }

    pub fn reset(&mut self) {
        self.readings.clear();
    }

    pub fn update(&mut self, devices: &[RateInput<'_>], now: f64) -> Vec<Option<String>> {
        let mut estimates = vec![None; devices.len()];
        if !now.is_finite() || now < 0.0 {
            self.readings.clear();
            return estimates;
        }
        for (index, device) in devices.iter().enumerate() {
            if device.id.trim().is_empty() {
                continue;
            }
            let rate_id = if is_physical_id(device.physical_id) {
                device.physical_id.unwrap_or(device.id)
            } else {
                device.id
            };
            let state = state_of(device.status);
            if state == State::FullOrError {
                self.readings.retain(|(id, _)| !same_id(id, rate_id));
                continue;
            }
            let Some(percent) = device.percent.filter(|value| *value <= 100 && !device.cached) else {
                continue;
            };
            let percent = i32::from(percent);

            if !same_id(rate_id, device.id)
                && self.history_index(rate_id).is_none()
                && let Some(legacy) = self.history_index(device.id)
            {
                self.history[legacy].device_id = rate_id.to_owned();
                self.revision += 1;
            }

            let position = self.readings.iter().position(|(id, _)| same_id(id, rate_id));
            let restart = match position {
                None => true,
                Some(position) => {
                    let reading = &self.readings[position].1;
                    state != reading.state
                        || now <= reading.last_time
                        || now - reading.last_time > MAX_GAP_SECONDS
                        || now - reading.start_time > MAX_SEGMENT_SECONDS
                }
            };
            if restart {
                self.set_reading(rate_id, Reading::new(percent, state, now));
                estimates[index] = self.historical_estimate(rate_id, state, 0, percent);
                continue;
            }
            let Some(position) = position else { continue };

            let direction = (percent - self.readings[position].1.last_percent).signum();
            let contradicts = direction != 0
                && ((state == State::Charging && direction < 0)
                    || (state == State::Discharging && direction > 0)
                    || (direction > 0 && device.status.eq_ignore_ascii_case("Not charging")));
            if contradicts {
                self.set_reading(rate_id, Reading::new(percent, state, now));
                continue;
            }
            let previous_direction = self.readings[position].1.direction;
            if direction != 0 && previous_direction != 0 && previous_direction != direction {
                let mut replacement = Reading::new(percent, state, now);
                replacement.direction = direction;
                self.set_reading(rate_id, replacement);
                estimates[index] = self.historical_estimate(rate_id, state, direction, percent);
                continue;
            }

            let (change, elapsed, samples, reading_direction) = {
                let reading = &mut self.readings[position].1;
                if direction != 0 {
                    reading.direction = direction;
                }
                reading.last_time = now;
                reading.last_percent = percent;
                reading.samples += 1;
                (
                    percent - reading.start_percent,
                    now - reading.start_time,
                    reading.samples,
                    reading.direction,
                )
            };
            if samples < 3 || elapsed < MINIMUM_PERIOD_SECONDS || change.abs() < MINIMUM_CHANGE_PERCENT {
                estimates[index] = self.historical_estimate(rate_id, state, reading_direction, percent);
                continue;
            }

            let qualified_direction = change.signum();
            let mut per_hour = f64::from(change.abs()) * 3600.0 / elapsed;
            let had_direction = self.history_index(rate_id).is_some_and(|found| {
                !direction_samples(&self.history[found], qualified_direction).is_empty()
            });
            if self.record_segment(rate_id, qualified_direction, f64::from(change.abs()), elapsed)
                && let Some(found) = self.history_index(rate_id)
            {
                per_hour = average_rate(direction_samples(&self.history[found], qualified_direction));
                let mut next = Reading::new(percent, state, now);
                next.direction = qualified_direction;
                self.set_reading(rate_id, next);
            }
            let label = if had_direction { "Avg." } else { "Est." };
            estimates[index] = Some(self.format_estimate(label, qualified_direction, per_hour, percent, true));
        }
        self.readings.retain(|(_, reading)| now - reading.last_time <= MAX_GAP_SECONDS);
        estimates
    }

    fn history_index(&self, id: &str) -> Option<usize> {
        self.history.iter().position(|value| same_id(&value.device_id, id))
    }

    fn set_reading(&mut self, id: &str, reading: Reading) {
        match self.readings.iter().position(|(existing, _)| same_id(existing, id)) {
            Some(position) => self.readings[position].1 = reading,
            None => self.readings.push((id.to_owned(), reading)),
        }
    }

    fn historical_estimate(&self, id: &str, state: State, direction: i32, percent: i32) -> Option<String> {
        if !has_stable_id(id) {
            return None;
        }
        let value = &self.history[self.history_index(id)?];
        match state {
            State::Charging if !value.charge_samples.is_empty() => {
                Some(self.format_estimate("Avg.", 1, average_rate(&value.charge_samples), percent, true))
            }
            State::Discharging if !value.discharge_samples.is_empty() => {
                Some(self.format_estimate("Avg.", -1, average_rate(&value.discharge_samples), percent, true))
            }
            State::Unknown if direction > 0 && !value.charge_samples.is_empty() => Some(self.format_estimate(
                "Avg. charge",
                1,
                average_rate(&value.charge_samples),
                percent,
                false,
            )),
            State::Unknown if direction <= 0 && !value.discharge_samples.is_empty() => Some(self.format_estimate(
                "Avg. use",
                -1,
                average_rate(&value.discharge_samples),
                percent,
                true,
            )),
            _ => None,
        }
    }

    fn format_estimate(&self, label: &str, direction: i32, per_hour: f64, percent: i32, allow_time: bool) -> String {
        let decimals = if per_hour < 0.01 { 4 } else { 2 };
        let sign = if direction > 0 { "+" } else { "\u{2212}" };
        let mut estimate = format!("{label} {sign}{}%/h", self.format_number(per_hour, decimals));
        if allow_time && per_hour > 0.0 {
            let remaining = if direction > 0 { 100 - percent } else { percent };
            let seconds = f64::from(remaining) / per_hour * 3600.0;
            if seconds > 0.0 && seconds < f64::from(i32::MAX) {
                estimate.push_str(" \u{00b7} ~");
                estimate.push_str(&format_duration(seconds.round() as i64));
                estimate.push_str(if direction > 0 { " to full" } else { " remaining" });
            }
        }
        estimate
    }

    fn format_number(&self, value: f64, decimals: usize) -> String {
        let scale = 10f64.powi(decimals as i32);
        let text = format!("{:.*}", decimals, (value * scale).round() / scale);
        let trimmed = text.trim_end_matches('0').trim_end_matches('.');
        let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
        if self.decimal_separator.is_empty() || self.decimal_separator == "." {
            trimmed.to_owned()
        } else {
            trimmed.replace('.', &self.decimal_separator)
        }
    }

    fn record_segment(&mut self, id: &str, direction: i32, change: f64, seconds: f64) -> bool {
        if !has_stable_id(id) {
            return false;
        }
        let index = match self.history_index(id) {
            Some(index) => index,
            None => {
                if self.history.len() >= MAX_HISTORY_DEVICES {
                    return false;
                }
                self.history.push(LearnedBatteryRate { device_id: id.to_owned(), ..Default::default() });
                self.history.len() - 1
            }
        };
        let value = &mut self.history[index];
        let samples = if direction > 0 { &mut value.charge_samples } else { &mut value.discharge_samples };
        samples.push(LearnedBatteryRateSample { percent: change, seconds });
        if samples.len() > MAX_HISTORY_SAMPLES_PER_DIRECTION {
            samples.remove(0);
        }
        self.revision += 1;
        true
    }
}

fn direction_samples(value: &LearnedBatteryRate, direction: i32) -> &[LearnedBatteryRateSample] {
    if direction > 0 { &value.charge_samples } else { &value.discharge_samples }
}

fn average_rate(samples: &[LearnedBatteryRateSample]) -> f64 {
    let percent: f64 = samples.iter().map(|sample| sample.percent).sum();
    let seconds: f64 = samples.iter().map(|sample| sample.seconds).sum();
    percent * 3600.0 / seconds
}

fn valid_samples(source: &[LearnedBatteryRateSample]) -> Vec<LearnedBatteryRateSample> {
    let mut destination = Vec::new();
    for sample in source {
        let valid = sample.percent.is_finite()
            && sample.seconds.is_finite()
            && sample.percent >= f64::from(MINIMUM_CHANGE_PERCENT)
            && sample.percent <= 100.0
            && sample.seconds >= MINIMUM_PERIOD_SECONDS
            && sample.seconds <= MAX_SEGMENT_SECONDS
            && sample.percent * 3600.0 / sample.seconds <= MAX_RATE_PER_HOUR;
        if !valid {
            continue;
        }
        destination.push(sample.clone());
        if destination.len() > MAX_HISTORY_SAMPLES_PER_DIRECTION {
            destination.remove(0);
        }
    }
    destination
}

fn state_of(status: &str) -> State {
    let is = |value: &str| status.eq_ignore_ascii_case(value);
    if is("Fully charged") || is("Battery error") || is("Temperature warning") || is("Charging error") {
        State::FullOrError
    } else if is("Nearly charged")
        || status.get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case("Charging"))
    {
        State::Charging
    } else if is("Discharging") {
        State::Discharging
    } else {
        State::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(id: &'a str, percent: u8, status: &'a str) -> RateInput<'a> {
        RateInput { id, physical_id: None, percent: Some(percent), status, cached: false }
    }

    fn step(tracker: &mut RateTracker, id: &str, percent: u8, status: &str, now: f64) -> Option<String> {
        tracker.update(&[input(id, percent, status)], now).remove(0)
    }

    #[test]
    fn durations_match_the_original_format() {
        assert_eq!(format_duration(-5), "0s");
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3_660), "1h 1m");
        assert_eq!(format_duration(90_000), "1d 1h");
    }

    #[test]
    fn an_estimate_needs_three_readings_thirty_minutes_and_three_percent() {
        let mut tracker = RateTracker::new(".");
        assert_eq!(step(&mut tracker, "mouse", 80, "Discharging", 0.0), None);
        assert_eq!(step(&mut tracker, "mouse", 79, "Discharging", 900.0), None);
        assert_eq!(step(&mut tracker, "mouse", 78, "Discharging", 1800.0), None);
        assert_eq!(
            step(&mut tracker, "mouse", 77, "Discharging", 3600.0),
            Some("Est. \u{2212}3%/h \u{00b7} ~1d 1h remaining".to_owned())
        );
        assert_eq!(tracker.revision(), 1);
        assert_eq!(tracker.export_history()[0].discharge_samples.len(), 1);
    }

    #[test]
    fn learned_history_gives_an_average_straight_away() {
        let mut tracker = RateTracker::new(".");
        tracker.import_history(&[LearnedBatteryRate {
            device_id: "Mouse".to_owned(),
            charge_samples: vec![LearnedBatteryRateSample { percent: 30.0, seconds: 3600.0 }],
            discharge_samples: vec![LearnedBatteryRateSample { percent: 5.0, seconds: 7200.0 }],
        }]);
        assert_eq!(
            step(&mut tracker, "mouse", 50, "Discharging", 10.0),
            Some("Avg. \u{2212}2.5%/h \u{00b7} ~20h 0m remaining".to_owned())
        );
        tracker.reset();
        assert_eq!(
            step(&mut tracker, "mouse", 40, "Charging", 20.0),
            Some("Avg. +30%/h \u{00b7} ~2h 0m to full".to_owned())
        );
        tracker.reset();
        assert_eq!(
            step(&mut tracker, "mouse", 40, "Connected", 30.0),
            Some("Avg. use \u{2212}2.5%/h \u{00b7} ~16h 0m remaining".to_owned())
        );
    }

    #[test]
    fn cached_and_full_readings_do_not_feed_learning() {
        let mut tracker = RateTracker::new(".");
        step(&mut tracker, "mouse", 80, "Discharging", 0.0);
        let cached = RateInput { id: "mouse", physical_id: None, percent: Some(60), status: "Discharging", cached: true };
        assert_eq!(tracker.update(&[cached], 900.0), vec![None]);
        step(&mut tracker, "mouse", 79, "Discharging", 1000.0);
        assert_eq!(step(&mut tracker, "mouse", 100, "Fully charged", 1100.0), None);
        assert_eq!(step(&mut tracker, "mouse", 77, "Discharging", 4000.0), None);
        assert_eq!(tracker.revision(), 0);
    }

    #[test]
    fn brief_gaps_keep_the_observation_but_long_gaps_restart_it() {
        let mut tracker = RateTracker::new(".");
        step(&mut tracker, "mouse", 80, "Discharging", 0.0);
        step(&mut tracker, "mouse", 79, "Discharging", 1000.0);
        assert!(step(&mut tracker, "mouse", 76, "Discharging", 1000.0 + MAX_GAP_SECONDS).is_some());
        let mut tracker = RateTracker::new(".");
        step(&mut tracker, "mouse", 80, "Discharging", 0.0);
        step(&mut tracker, "mouse", 79, "Discharging", 1000.0);
        assert_eq!(step(&mut tracker, "mouse", 70, "Discharging", 1001.0 + MAX_GAP_SECONDS), None);
        assert_eq!(tracker.revision(), 0);
    }

    #[test]
    fn invalid_history_samples_are_dropped_on_import() {
        let mut tracker = RateTracker::new(".");
        let sample = |percent: f64, seconds: f64| LearnedBatteryRateSample { percent, seconds };
        tracker.import_history(&[
            LearnedBatteryRate {
                device_id: "a".to_owned(),
                charge_samples: vec![sample(2.0, 3600.0), sample(50.0, 60.0), sample(f64::NAN, 3600.0)],
                discharge_samples: vec![sample(10.0, 3600.0), sample(101.0, 3600.0)],
            },
            LearnedBatteryRate { device_id: "A".to_owned(), ..Default::default() },
            LearnedBatteryRate {
                device_id: "device:0123456789abcdef0123456789abcdef".to_owned(),
                discharge_samples: vec![sample(10.0, 3600.0)],
                ..Default::default()
            },
        ]);
        let history = tracker.export_history();
        assert_eq!(history.len(), 1);
        assert!(history[0].charge_samples.is_empty());
        assert_eq!(history[0].discharge_samples, vec![sample(10.0, 3600.0)]);
    }

    #[test]
    fn history_follows_a_device_to_its_physical_identity() {
        let mut tracker = RateTracker::new(".");
        tracker.import_history(&[LearnedBatteryRate {
            device_id: "logitech:path:1".to_owned(),
            discharge_samples: vec![LearnedBatteryRateSample { percent: 6.0, seconds: 3600.0 }],
            ..Default::default()
        }]);
        let physical = "logitech-unit:0123456789ABCDEF0123";
        assert!(is_physical_id(Some(physical)));
        let device = RateInput {
            id: "logitech:path:1",
            physical_id: Some(physical),
            percent: Some(60),
            status: "Discharging",
            cached: false,
        };
        let estimate = tracker.update(&[device], 5.0).remove(0);
        assert_eq!(estimate, Some("Avg. \u{2212}6%/h \u{00b7} ~10h 0m remaining".to_owned()));
        assert_eq!(tracker.export_history()[0].device_id, physical);
        assert_eq!(tracker.revision(), 1);
    }

    #[test]
    fn rates_use_the_locale_decimal_separator() {
        let mut tracker = RateTracker::new(",");
        tracker.import_history(&[LearnedBatteryRate {
            device_id: "mouse".to_owned(),
            discharge_samples: vec![LearnedBatteryRateSample { percent: 5.0, seconds: 7200.0 }],
            ..Default::default()
        }]);
        assert_eq!(
            step(&mut tracker, "mouse", 40, "Connected", 1.0),
            Some("Avg. use \u{2212}2,5%/h \u{00b7} ~16h 0m remaining".to_owned())
        );
    }
}
