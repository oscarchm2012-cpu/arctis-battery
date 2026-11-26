# Arctis Battery Monitor

Tray / menu-bar app showing the real battery level of **SteelSeries Arctis 3 Bluetooth** and
**Arctis Nova 3P / 3X / 5 / 5X Wireless** as a duck-headphones icon with the percentage.
Tauri 2 + React + TypeScript + Rust.

## Download

**[Latest release](https://github.com/oscarchm2012-cpu/arctis-battery/releases/latest)**

- Windows 10 / 11: [Arctis-Battery-Monitor-1.0.2-Windows-setup.exe](https://github.com/oscarchm2012-cpu/arctis-battery/releases/download/v1.0.2/Arctis-Battery-Monitor-1.0.2-Windows-setup.exe)
- macOS 11+ (Apple Silicon and Intel): [Arctis-Battery-Monitor-1.0.2-macOS.dmg](https://github.com/oscarchm2012-cpu/arctis-battery/releases/download/v1.0.2/Arctis-Battery-Monitor-1.0.2-macOS.dmg)

Not code-signed yet: on Windows click *More info → Run anyway* if SmartScreen appears; on macOS
right-click the app → *Open* the first time.

## How the battery is read

No values are estimated or simulated. Sources, in order:

1. **Bluetooth LE GATT Battery Service** (0x180F, Battery Level 0x2A19) – read uncached every 5 s,
   plus change notifications. Charging state from Battery Level Status (0x2BED) if present.
2. **Windows Bluetooth Hands-Free battery report** – the value the headset sends over HFP, which
   Windows stores on the device node as `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2`
   (the same number Settings › Bluetooth & devices shows). Usually in 10 % steps.

3. **SteelSeries 2.4 GHz dongle (USB HID)** for Arctis Nova 3P / 3X / 5 / 5X Wireless (1038:2269 / 226D / 2232 / 2253):
   request `00 B0` on interface 3 (usage page 0xFFC0); reply byte 1 = link (0x02 headset off),
   byte 3 = battery %, byte 4 = 0x01 charging. Used when no Arctis 3 is connected over Bluetooth.
   The dongle also carries the headset's audio, so it is queried gently: one request every 30 s,
   no retries, and after a failed query it is left alone for 2 min (doubling up to 15 min).
   Hammering it with requests (e.g. while plugging in the charging cable) can hang the dongle until
   it is replugged. `cargo run --example probe` (in `src-tauri`) sends a single request and dumps the reply.

If neither is available the app shows **"Battery percentage unavailable"**. Charging state is shown
only when the device reports it; Classic Bluetooth headsets do not, so it shows "Not reported by device".

Detection matches paired devices whose name contains "Arctis 3" (not "Arctis Nova 3"/"3X").
Connection and Bluetooth radio changes are event-driven; battery is re-read every 5 s.

## Features

- Dynamic tray icon (headphones + %), green 50–100, yellow 20–49, red 10–19, blinking red 1–9.
- Tooltip `SteelSeries Arctis 3 – Battery: XX%`.
- Notifications at 20 % and 10 % (once per crossing).
- Starts with Windows (enabled on first launch, toggle in the window), starts hidden in the tray.
- Close/minimize hides to tray; left-click tray icon toggles window; right-click menu.
- Bluetooth diagnostics panel with copyable report.

## macOS

Same app, menu-bar icon (no Dock icon). Battery sources on macOS:
- Bluetooth (Arctis 3, Nova 3P/3X over Bluetooth): the level macOS itself shows, read from
  `system_profiler SPBluetoothDataType -json` every 20 s.
- Nova 3P/3X 2.4 GHz dongle: same USB HID protocol as on Windows (`src-tauri/src/hid.rs`).

Building needs a Mac (Xcode command line tools) or GitHub Actions: `.github/workflows/build.yml`
builds a universal `.dmg` (Apple Silicon + Intel) and the Windows installer on every push to `main`.
The app is not signed, so on first launch right-click it → Open (or `xattr -cr "/Applications/Arctis 3 Battery Monitor.app"`).

## Build

Requires Node 18+, Rust (MSVC) and Visual Studio C++ Build Tools.

```
npm install
npx tauri build
```

Installer: `src-tauri/target/release/bundle/nsis/Arctis 3 Battery Monitor_1.0.0_x64-setup.exe`

Dev: `npx tauri dev`
