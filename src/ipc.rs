use crate::monitor::Device;
use crate::settings::Settings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PopupView {
    pub theme: String,
    pub color_bars: bool,
    pub low_threshold: i32,
    pub critical_threshold: i32,
    pub show_details: bool,
    pub show_updated: bool,
    pub show_more: bool,
}

impl PopupView {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            theme: settings.theme.clone(),
            color_bars: settings.color_bars,
            low_threshold: settings.low_threshold,
            critical_threshold: settings.critical_threshold,
            show_details: settings.show_popup_reset_times,
            show_updated: settings.show_popup_last_updated,
            show_more: settings.show_reset_availability,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PopupState {
    pub devices: Vec<Device>,
    pub updated: String,
    pub status: Option<String>,
    pub refreshing: bool,
    pub has_snapshot: bool,
    pub view: PopupView,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FromPopup {
    Refresh,
    Settings,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub text: String,
    pub busy: bool,
    pub button: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsInit {
    pub settings: Settings,
    pub start_with_windows: bool,
    pub hfp_trace_status: Option<String>,
    pub devices: Vec<Device>,
    pub version: String,
    pub update: UpdateStatus,
    pub save_blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToSettings {
    Init(Box<SettingsInit>),
    Devices(Vec<Device>),
    HfpTraceStatus(Option<String>),
    Update(UpdateStatus),
    SaveResult(Option<String>),
    Activate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FromSettings {
    Save(Box<Settings>),
    Dirty(bool),
    CheckUpdates,
}

pub fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let line = serde_json::to_string(message).map_err(io::Error::other)?;
    writeln!(writer, "{line}")?;
    writer.flush()
}

pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Ok(message) = serde_json::from_str::<T>(line.trim()) {
            return Ok(Some(message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_the_pipe_and_malformed_lines_are_skipped() {
        let state = PopupState {
            devices: vec![Device {
                id: "logitech:\\\\?\\hid#x:1".to_owned(),
                name: "Mouse\twith \"quotes\"\nand lines".to_owned(),
                percent: Some(79),
                estimate: Some("Avg. \u{2212}1.5%/h \u{00b7} ~2d 4h remaining".to_owned()),
                ..Default::default()
            }],
            updated: "11:57 AM".to_owned(),
            status: None,
            refreshing: true,
            has_snapshot: true,
            view: PopupView::from_settings(&Settings::default()),
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &state).unwrap();
        bytes.extend_from_slice(b"not json\n{\"devices\":42}\n");
        write_message(&mut bytes, &state).unwrap();
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 4);
        let mut reader = bytes.as_slice();
        assert_eq!(read_message::<PopupState>(&mut reader).unwrap(), Some(state.clone()));
        assert_eq!(read_message::<PopupState>(&mut reader).unwrap(), Some(state));
        assert_eq!(read_message::<PopupState>(&mut reader).unwrap(), None);
    }

    #[test]
    fn commands_and_settings_round_trip() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &FromPopup::Settings).unwrap();
        write_message(&mut bytes, &FromSettings::Save(Box::new(Settings::default()))).unwrap();
        write_message(&mut bytes, &ToSettings::SaveResult(Some("Could not save settings".to_owned()))).unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(read_message::<FromPopup>(&mut reader).unwrap(), Some(FromPopup::Settings));
        assert_eq!(
            read_message::<FromSettings>(&mut reader).unwrap(),
            Some(FromSettings::Save(Box::new(Settings::default())))
        );
        assert_eq!(
            read_message::<ToSettings>(&mut reader).unwrap(),
            Some(ToSettings::SaveResult(Some("Could not save settings".to_owned())))
        );
    }
}
