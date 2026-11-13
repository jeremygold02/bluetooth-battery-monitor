# Bluetooth Battery Monitor

Bluetooth Battery Monitor is a Windows tray app for checking the battery levels of supported Bluetooth and wireless devices. A compact popup shows each device's battery level and an estimated charging or usage rate when enough data is available.

<p align="center">
  <img src="docs/images/battery-popup.png" alt="Bluetooth Battery Monitor popup showing a wireless headset and mouse with their battery levels and rate-learning status">
</p>

_The devices and battery values in the screenshots are sample data._

## Download

Download `BluetoothBatteryMonitor-Setup.exe` to install the app for your Windows account, or run `BluetoothBatteryMonitorRust.exe` directly for a portable copy. Both run the native Rust app on Windows 10 or Windows 11 without a separate runtime. [GitHub Releases](https://github.com/jeremygold02/bluetooth-battery-monitor/releases/latest)

If you still use the old .NET version, run `BluetoothBatteryMonitor-Setup.exe` once to switch to the Rust app and keep your settings.

The per-user installer and in-app updater deliver the Soundcore battery reader in the native app executable. It uses the Bluetooth APIs built into Windows and reads paired, connected earbuds with standard user permissions.

## Features

- Show supported device battery levels in a compact tray popup, with details available on hover.
- Choose the lowest battery, average battery, or a specific device for the tray icon.
- Estimate charging and discharge rates from observed battery-level changes, with separate rolling histories for charging and usage.
- Customize the tray icon background and text colors, theme, notification thresholds, popup details, refresh interval, and Windows startup behavior.
- Refresh manually or automatically, including after a Bluetooth LE connection or disconnection.
- Start with Windows for your user account with standard permissions.
- Install available updates automatically at startup, waiting for unsaved settings to be saved or closed before restarting. Update failures are reported; you can also check manually from Settings or the tray menu.

## Supported devices

The monitor combines local Windows and device-protocol sources:

- The Windows `System.Devices.BatteryLife` property and Bluetooth LE Battery Service, including charging state when a device reports it.
- The HID Generic Device Controls Battery Strength usage (`0x06:0x20`) on Bluetooth and USB HID devices, including interfaces that are recognized before a battery report is available.
- Soundcore Life Dot 2 NC battery and charging state over Bluetooth RFCOMM. The popup uses the lower reported level when both earbuds are available.
- Connected classic Bluetooth Hands-Free Profile (HFP) devices are listed from Windows device information. For compatible headsets, enable **Settings → General → Listen for Hands-Free battery reports** to read battery messages transmitted by the headset. This is off by default, and its status appears in Settings. If Windows denies trace access, run the app as administrator or use an account in Performance Log Users.
- Logitech HID++ battery features `0x1000` and `0x1004`, used by compatible LIGHTSPEED receivers.
- The PRO X3 SUPERSTRIKE LIGHTSPEED receiver (`046D:C54F`), using its 20-byte HID++ interface; physical-device validation is pending.
- HyperX Cloud III S Wireless dongle battery and charging-state responses for HP USB product IDs `02CC` and `06BE`.

The current collectors have been verified with a Logitech G305 LIGHTSPEED mouse and a HyperX Cloud III S Wireless headset. The generic HID reader is based on the standard Battery Strength usage but still needs hardware validation with more devices. HFP event decoding also needs validation with more headsets. Other devices can work when they expose a supported Windows, Bluetooth, HID, or HFP battery interface; detection and available details vary by device.

Readings from different sources are merged when they identify the same device unambiguously. Devices with duplicate names from the same source remain separate. The popup lists known battery levels first, with the lowest level at the top.

## Refresh and battery estimates

The app checks devices at startup and at the interval selected in Settings, every 60 seconds by default. It also requests a check after a Bluetooth LE device connects or disconnects. Battery changes and USB receiver devices still use polling or manual refresh.

An initial rate needs at least three readings over 30 minutes and a change of at least three percentage points. Devices that report in coarse steps can take longer. Once learned, charging and discharge rates are kept separately as duration-weighted rolling averages of the latest 20 qualifying periods for each device. The learned rates are saved locally and can be reused after the app restarts; new observations gradually replace older periods.

Rates and time remaining are estimates based on past percentage changes, not electrical current measurements. They can change with use, charging conditions, battery aging, and the precision of a device's battery gauge. When the current direction is unknown, a saved usage rate is shown as an average without a live time-remaining prediction.

Cached and unavailable readings do not contribute to the estimate. An interrupted observation—such as a device disappearing, a long gap, a charge-state change, or Windows resuming—starts a fresh observation period while retaining qualified history. Windows may also expose a cached battery property without indicating how recently the hardware measured it.

## Using the tray app

- Left-click the tray icon to open or close the popup. Use its refresh button or **Refresh now** in the tray menu to check immediately.
- Open Settings from the popup gear or tray menu. A specific-device selection changes the tray icon; the popup can still show the other devices.
- Hover over a device for battery, status, and rate details. Scroll the list when more devices are present than fit in the popup.
- Rows for sleeping, disconnected, or unavailable devices are dimmed and can show the last known battery level. The app saves these readings locally so they remain available after a restart.

![Bluetooth Battery Monitor settings showing tray icon choices, preview, colors, and theme](docs/images/battery-settings.png)

## Development

The app is written in Rust. Building from source requires Windows and the stable Rust toolchain for `x86_64-pc-windows-msvc`.

From PowerShell in the project folder:

```powershell
cargo build --release
```

The app is written to `target\release\BluetoothBatteryMonitor.exe`. Only one instance runs at a time, so exit an existing tray instance before starting a local build. Local builds report version `0.0.0` and do not check for updates automatically.

Run the automated checks without showing app windows:

```powershell
cargo test
```

### Release packaging

When changes land on `main`, the release workflow builds and tests a versioned Rust app and publishes three assets. [Release workflow](.github/workflows/release.yml)

- `BluetoothBatteryMonitor-Setup.exe` installs the native app and license notices for the current Windows user.
- `BluetoothBatteryMonitorRust.exe` runs as a portable app and is the executable the native updater installs in place.
- `ThirdPartyNotices.zip` contains the license notices for portable copies.

The installer and updater preserve the user's Hands-Free setting. Listening uses the Windows event tracing API and stays off until a user enables it in Settings.

To collect the license notices that ship with a release into `artifacts\build`:

```powershell
.\tools\collect-licenses.ps1 -Destination .\artifacts\build
```

For a direct local device-collector check:

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\run-battery-collectors-probe.ps1
```

## Troubleshooting

- **HyperX is missing:** turn on the headset, reconnect its USB dongle, and refresh. The dongle reports `0xFF` when the headset is unavailable.
- **G305 is missing:** wake the mouse and refresh. HID++ queries may not answer while it is asleep.
- **A Bluetooth device is missing:** confirm Windows can display its battery level. Not every device implements the standard Battery Service or exposes a Windows battery property.
- **A selected device is unavailable:** reconnect or wake it. If its connection or receiver changes, its identifier can change and it may need to be selected again.
- **A local build exits immediately or cannot be rebuilt:** exit the existing tray process first.

## Privacy

Device readings are collected locally. Preferences, last-known battery readings, and learned rate samples, keyed by device identifier, are stored in `%APPDATA%\BluetoothBatteryMonitor\settings.json`. The app contacts GitHub for release checks and update downloads; it has no separate telemetry or analytics service.

## Attribution

The interface uses [egui](https://github.com/emilk/egui). The HyperX protocol compatibility work was informed by the MIT-licensed [HyperHeadset](https://github.com/LennardKittner/HyperHeadset) and [HyperX Cloud 2 Battery Monitor](https://github.com/auto94/HyperX-Cloud-2-Battery-Monitor) projects. License notices are included in `THIRD_PARTY_NOTICES.md` and the build output.

The Soundcore reader uses the A3931 protocol documented by the [OpenSCQ30 implementation](https://github.com/Oppzippy/OpenSCQ30/blob/master/lib/src/devices/soundcore/a3931.rs).

This project builds on [Codex Usage Tray](https://github.com/jeremygold02/codex-usage-tray-icon), which was inspired by the commercial [Bluetooth Battery Monitor](https://www.bluetoothgoodies.com/). It contains no code from that commercial application.
