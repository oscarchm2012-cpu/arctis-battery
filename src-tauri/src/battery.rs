//! Reads the real battery level of a SteelSeries Arctis 3 Bluetooth headset.
//!
//! Two sources, both provided by the device itself through Windows:
//!  1. Bluetooth LE GATT Battery Service (0x180F / Battery Level 0x2A19), if the
//!     headset exposes it. Subscribed for change notifications.
//!  2. The battery value Windows' Bluetooth stack receives from a Classic
//!     (Hands-Free profile) headset. Windows stores it on the device node as the
//!     property {104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2 — the same value shown in
//!     Settings > Bluetooth & devices.
//!
//! Nothing is estimated or simulated: if neither source has a value, the
//! percentage is reported as unavailable.

use serde::Serialize;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatteryState {
    /// A paired Arctis 3 was found.
    pub detected: bool,
    pub device_name: Option<String>,
    /// "connected" | "disconnected" | "unknown" | "notFound"
    pub connection: String,
    /// "on" | "off" | "disabled" | "noAdapter" | "unknown"
    pub bluetooth: String,
    pub percent: Option<u8>,
    /// Where the value came from.
    pub source: Option<String>,
    /// Resolution of the value as reported by the protocol.
    pub precision: Option<String>,
    /// "charging" | "discharging" | "notReported"
    pub charging: String,
    /// "bluetooth" | "usbDongle"
    pub transport: Option<String>,
    /// Ajazz AJ179 mouse on its 2.4 GHz receiver, if present.
    pub mouse: Option<crate::mouse::MouseState>,
    /// Unix ms of the last successful battery read.
    pub last_update: Option<u64>,
    /// Unix ms of the last check (successful or not).
    pub last_check: u64,
}

impl BatteryState {
    pub fn initial() -> Self {
        Self {
            detected: false,
            device_name: None,
            connection: "unknown".into(),
            bluetooth: "unknown".into(),
            percent: None,
            source: None,
            precision: None,
            charging: "notReported".into(),
            transport: None,
            mouse: None,
            last_update: None,
            last_check: 0,
        }
    }

    /// Equal apart from timestamps of the check.
    pub fn same_reading(&self, other: &Self) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.last_check = 0;
        b.last_check = 0;
        a.last_update = None;
        b.last_update = None;
        a == b
    }
}

#[derive(Clone, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEntry {
    pub name: String,
    pub id: String,
    pub connected: Option<bool>,
    pub matched: bool,
}

#[derive(Clone, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatteryNode {
    pub name: String,
    pub instance_id: String,
    pub percent: u8,
    pub matched: bool,
}

#[derive(Clone, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub generated_at: u64,
    pub radios: Vec<String>,
    pub classic_devices: Vec<DeviceEntry>,
    pub le_devices: Vec<DeviceEntry>,
    pub battery_nodes: Vec<BatteryNode>,
    pub gatt: String,
    pub hfp: String,
    /// SteelSeries 2.4 GHz dongle (HID) status.
    pub hid: String,
    pub hid_devices: Vec<String>,
    /// Ajazz mouse receiver status (incl. raw bytes).
    pub mouse: String,
    pub log: Vec<String>,
}

impl Diagnostics {
    pub fn push_log(&mut self, s: impl Into<String>) {
        if self.log.len() > 60 {
            self.log.remove(0);
        }
        self.log.push(s.into());
    }
}

/// SteelSeries 2.4 GHz dongles that answer the 0xB0 status request on the
/// vendor HID collection (usage page 0xFFC0, interface 3).
pub const HID_VENDOR: u16 = 0x1038;
pub const HID_DEVICES: &[(u16, &str)] = &[
    (0x2269, "Arctis Nova 3P Wireless"),
    (0x226d, "Arctis Nova 3X Wireless"),
    // Same protocol and offsets per HeadsetControl (steelseries_arctis_nova_5.hpp).
    (0x2232, "Arctis Nova 5 Wireless"),
    (0x2253, "Arctis Nova 5X Wireless"),
];

/// Parses the dongle's reply to `00 B0`:
/// `[0]=0xB0 echo, [1]=link (0x02 = headset off), [3]=battery %, [4]=0x01 charging`.
pub fn parse_hid_status(buf: &[u8]) -> Option<(bool, u8, bool)> {
    if buf.len() < 5 || buf[0] != 0xb0 {
        return None;
    }
    Some((buf[1] != 0x02, buf[3], buf[4] == 0x01))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// "Arctis 3" but not "Arctis 3X", "Arctis 30…", "Arctis Nova 3".
pub fn is_arctis3(name: &str) -> bool {
    let lower = name.to_lowercase();
    for pat in ["arctis 3", "arctis3"] {
        let mut start = 0;
        while let Some(pos) = lower[start..].find(pat) {
            let end = start + pos + pat.len();
            let next = lower[end..].chars().next();
            if next.map_or(true, |c| !c.is_alphanumeric()) {
                return true;
            }
            start = end;
        }
    }
    false
}

/// Headsets whose battery is read over Bluetooth: Arctis 3, and Arctis Nova 3P /
/// 3X / 5 / 5X when used over Bluetooth instead of their 2.4 GHz dongle.
pub fn is_supported_bt(name: &str) -> bool {
    let lower = name.to_lowercase();
    is_arctis3(name) || lower.contains("nova 3p") || lower.contains("nova 3x") || lower.contains("nova 5")
}

/// Messages that wake the monitor thread.
pub enum Msg {
    /// Re-read now; `full` forces device re-enumeration.
    Refresh { full: bool },
}

#[cfg(windows)]
pub use win::Monitor;
#[cfg(target_os = "macos")]
pub use crate::mac::Monitor;

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::mpsc::Sender;
    use windows::core::{GUID, HSTRING, PCWSTR};
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristic, GattCharacteristicUuids, GattClientCharacteristicConfigurationDescriptorValue,
        GattCommunicationStatus, GattDeviceService, GattServiceUuids, GattValueChangedEventArgs,
    };
    use windows::Devices::Bluetooth::{
        BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
    };
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::DataReader;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
        SetupDiGetClassDevsW, SetupDiGetDevicePropertyW, SetupDiOpenDeviceInfoW, DIGCF_ALLCLASSES,
        DIGCF_PRESENT, HDEVINFO, SP_DEVINFO_DATA,
    };
    use windows::Win32::Devices::Properties::DEVPROPTYPE;
    use windows::Win32::Foundation::DEVPROPKEY;

    const KEY_BATTERY: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
        pid: 2,
    };
    const KEY_FRIENDLY_NAME: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };
    const KEY_NAME: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac),
        pid: 10,
    };
    const KEY_INSTANCE_ID: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0x78c34fc8_104a_4aca_9ea4_524d52996e57),
        pid: 256,
    };
    const DEVPROP_TYPE_BYTE: u32 = 0x3;
    const DEVPROP_TYPE_STRING: u32 = 0x12;
    /// GATT "Battery Level Status" (contains the charge state).
    const UUID_BATTERY_LEVEL_STATUS: GUID = GUID::from_u128(0x00002bed_0000_1000_8000_00805f9b34fb);

    const FULL_SCAN_EVERY: Duration = Duration::from_secs(60);
    const RESCAN_WHEN_MISSING: Duration = Duration::from_secs(15);

    struct LeLink {
        device: BluetoothLEDevice,
        _service: GattDeviceService,
        level: GattCharacteristic,
        status: Option<GattCharacteristic>,
        _token: Option<i64>,
    }

    pub struct Monitor {
        tx: Sender<Msg>,
        radios: Vec<Radio>,
        classic: Option<(String, BluetoothDevice)>,
        le: Option<(String, BluetoothLEDevice)>,
        le_link: Option<LeLink>,
        le_link_failed_at: Option<Instant>,
        /// (name, instance id) of matched device nodes that carry the battery property.
        battery_nodes: Vec<(String, String)>,
        last_full: Option<Instant>,
        last_good: Option<(u8, u64, String)>,
        hid: crate::hid::HidSource,
        pub diag: Diagnostics,
    }

    fn log(diag: &mut Diagnostics, s: impl Into<String>) {
        diag.push_log(s);
    }

    impl Monitor {
        pub fn new(tx: Sender<Msg>) -> Self {
            unsafe {
                use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            Self {
                tx,
                radios: vec![],
                classic: None,
                le: None,
                le_link: None,
                le_link_failed_at: None,
                battery_nodes: vec![],
                last_full: None,
                last_good: None,
                hid: crate::hid::HidSource::new(),
                diag: Diagnostics::default(),
            }
        }

        /// Arctis 3 over Bluetooth wins when it is connected; otherwise a
        /// SteelSeries 2.4 GHz dongle (Nova 3P/3X) if one is plugged in.
        pub fn read(&mut self, force_full: bool) -> BatteryState {
            let mut bt = self.read_bt(force_full);
            if bt.detected {
                bt.transport = Some("bluetooth".into());
            }
            crate::hid::prefer(bt, &mut self.hid, &mut self.diag, force_full)
        }

        fn read_bt(&mut self, force_full: bool) -> BatteryState {
            let detected_before = self.classic.is_some() || self.le.is_some() || !self.battery_nodes.is_empty();
            let due = match self.last_full {
                None => true,
                Some(t) => {
                    t.elapsed() >= FULL_SCAN_EVERY || (!detected_before && t.elapsed() >= RESCAN_WHEN_MISSING)
                }
            };
            if force_full || due {
                self.full_scan();
            }

            let mut st = BatteryState::initial();
            st.last_check = now_ms();
            st.bluetooth = self.radio_state();

            // Device identity and connection.
            let classic_conn = self.classic.as_ref().and_then(|(_, d)| {
                d.ConnectionStatus().ok().map(|s| s == BluetoothConnectionStatus::Connected)
            });
            let le_conn = self.le.as_ref().and_then(|(_, d)| {
                d.ConnectionStatus().ok().map(|s| s == BluetoothConnectionStatus::Connected)
            });
            let name = self
                .classic
                .as_ref()
                .map(|(n, _)| n.clone())
                .or_else(|| self.le.as_ref().map(|(n, _)| n.clone()))
                .or_else(|| self.battery_nodes.first().map(|(n, _)| n.clone()));
            st.detected = name.is_some();
            st.device_name = name;
            let connected = match (classic_conn, le_conn) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), _) | (_, Some(false)) => Some(false),
                _ => None,
            };
            st.connection = match (st.detected, connected) {
                (false, _) => "notFound",
                (true, Some(true)) => "connected",
                (true, Some(false)) => "disconnected",
                (true, None) => "unknown",
            }
            .into();

            if st.bluetooth == "off" || st.bluetooth == "disabled" || connected == Some(false) {
                // A disconnected headset cannot report anything; any stored value is stale.
                self.le_link = None;
                return st;
            }

            // 1) GATT Battery Service.
            if le_conn == Some(true) {
                self.ensure_le_link();
                if let Some(link) = &self.le_link {
                    match read_byte(&link.level) {
                        Ok(v) if v <= 100 => {
                            st.percent = Some(v);
                            st.source = Some("Bluetooth LE GATT Battery Service".into());
                            st.precision = Some("1%".into());
                            if let Some(c) = &link.status {
                                if let Some(ch) = read_charge_state(c) {
                                    st.charging = ch.into();
                                }
                            }
                            self.diag.gatt = format!("Battery Level read: {v}%");
                        }
                        Ok(v) => self.diag.gatt = format!("Invalid Battery Level value {v}"),
                        Err(e) => {
                            self.diag.gatt = format!("Read failed: {e}");
                            self.le_link = None;
                        }
                    }
                }
            }

            // 2) Hands-Free battery value held by Windows' Bluetooth stack.
            if st.percent.is_none() {
                let mut found = None;
                for (n, id) in &self.battery_nodes {
                    if let Some(v) = read_battery_by_id(id) {
                        found = Some((v, n.clone()));
                        break;
                    }
                }
                match found {
                    Some((v, node)) if v <= 100 => {
                        st.percent = Some(v);
                        st.source = Some("Windows Bluetooth (Hands-Free battery report)".into());
                        st.precision = Some(if v % 10 == 0 { "10% steps".into() } else { "1%".into() });
                        self.diag.hfp = format!("{v}% from \"{node}\"");
                    }
                    _ => {
                        if self.diag.hfp.is_empty() || self.battery_nodes.is_empty() {
                            self.diag.hfp = "No battery value published by Windows for this headset".into();
                        }
                    }
                }
            }

            if let Some(p) = st.percent {
                let src = st.source.clone().unwrap_or_default();
                let ts = match &self.last_good {
                    Some((lp, lts, ls)) if *lp == p && *ls == src => *lts,
                    _ => st.last_check,
                };
                self.last_good = Some((p, ts, src));
                st.last_update = Some(st.last_check);
            }
            st
        }

        fn radio_state(&self) -> String {
            if self.radios.is_empty() {
                return "noAdapter".into();
            }
            let mut best = "unknown";
            for r in &self.radios {
                match r.State() {
                    Ok(RadioState::On) => return "on".into(),
                    Ok(RadioState::Off) => best = "off",
                    Ok(RadioState::Disabled) if best != "off" => best = "disabled",
                    _ => {}
                }
            }
            best.into()
        }

        fn wake(&self) -> Sender<Msg> {
            self.tx.clone()
        }

        fn full_scan(&mut self) {
            self.last_full = Some(Instant::now());
            let mut d = Diagnostics {
                generated_at: now_ms(),
                log: std::mem::take(&mut self.diag.log),
                gatt: std::mem::take(&mut self.diag.gatt),
                hid: std::mem::take(&mut self.diag.hid),
                hid_devices: std::mem::take(&mut self.diag.hid_devices),
                hfp: String::new(),
                ..Default::default()
            };

            // Radios
            self.radios.clear();
            match Radio::GetRadiosAsync().and_then(|op| op.get()) {
                Ok(list) => {
                    for r in list {
                        if r.Kind().ok() == Some(RadioKind::Bluetooth) {
                            let state = match r.State() {
                                Ok(RadioState::On) => "On",
                                Ok(RadioState::Off) => "Off",
                                Ok(RadioState::Disabled) => "Disabled",
                                _ => "Unknown",
                            };
                            let name = r.Name().map(|s| s.to_string()).unwrap_or_default();
                            d.radios.push(format!("{name} — {state}"));
                            let tx = self.wake();
                            let _ = r.StateChanged(&TypedEventHandler::new(move |_, _| {
                                let _ = tx.send(Msg::Refresh { full: false });
                                Ok(())
                            }));
                            self.radios.push(r);
                        }
                    }
                    if self.radios.is_empty() {
                        log(&mut d, "No Bluetooth radio found");
                    }
                }
                Err(e) => log(&mut d, format!("Radio query failed: {e}")),
            }

            // Paired Classic devices
            let prev_classic = self.classic.as_ref().map(|(_, d)| d.DeviceId().ok()).flatten();
            self.classic = None;
            match enumerate(BluetoothDevice::GetDeviceSelectorFromPairingState(true)) {
                Ok(list) => {
                    for (name, id) in list {
                        let matched = is_supported_bt(&name);
                        let mut connected = None;
                        if let Ok(dev) = BluetoothDevice::FromIdAsync(&id).and_then(|o| o.get()) {
                            connected = dev
                                .ConnectionStatus()
                                .ok()
                                .map(|s| s == BluetoothConnectionStatus::Connected);
                            if matched && self.classic.is_none() {
                                if prev_classic.as_ref() != Some(&id) {
                                    let tx = self.wake();
                                    let _ = dev.ConnectionStatusChanged(&TypedEventHandler::new(move |_, _| {
                                        let _ = tx.send(Msg::Refresh { full: false });
                                        Ok(())
                                    }));
                                    log(&mut d, format!("Found Classic device \"{name}\""));
                                }
                                self.classic = Some((name.clone(), dev));
                            }
                        }
                        d.classic_devices.push(DeviceEntry { name, id: id.to_string(), connected, matched });
                    }
                }
                Err(e) => log(&mut d, format!("Classic enumeration failed: {e}")),
            }

            // Paired LE devices
            let prev_le = self.le.as_ref().map(|(_, d)| d.DeviceId().ok()).flatten();
            self.le = None;
            match enumerate(BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)) {
                Ok(list) => {
                    for (name, id) in list {
                        let matched = is_supported_bt(&name);
                        let mut connected = None;
                        if let Ok(dev) = BluetoothLEDevice::FromIdAsync(&id).and_then(|o| o.get()) {
                            connected = dev
                                .ConnectionStatus()
                                .ok()
                                .map(|s| s == BluetoothConnectionStatus::Connected);
                            if matched && self.le.is_none() {
                                if prev_le.as_ref() != Some(&id) {
                                    self.le_link = None;
                                    let tx = self.wake();
                                    let _ = dev.ConnectionStatusChanged(&TypedEventHandler::new(move |_, _| {
                                        let _ = tx.send(Msg::Refresh { full: false });
                                        Ok(())
                                    }));
                                    log(&mut d, format!("Found LE device \"{name}\""));
                                }
                                self.le = Some((name.clone(), dev));
                            }
                        }
                        d.le_devices.push(DeviceEntry { name, id: id.to_string(), connected, matched });
                    }
                }
                Err(e) => log(&mut d, format!("LE enumeration failed: {e}")),
            }
            if self.le.is_none() {
                self.le_link = None;
                d.gatt = "Headset is not paired as a Bluetooth LE device — GATT Battery Service not available".into();
            }

            // Device nodes carrying a battery value
            self.battery_nodes.clear();
            for node in scan_battery_nodes() {
                if node.matched {
                    self.battery_nodes.push((node.name.clone(), node.instance_id.clone()));
                }
                d.battery_nodes.push(node);
            }
            if self.battery_nodes.is_empty() {
                d.hfp = "Windows has not published a battery value for an Arctis 3 device node".into();
            }

            self.diag = d;
        }

        fn ensure_le_link(&mut self) {
            if self.le_link.is_some() {
                return;
            }
            if let Some(t) = self.le_link_failed_at {
                if t.elapsed() < Duration::from_secs(30) {
                    return;
                }
            }
            let Some((_, dev)) = &self.le else { return };
            match open_le_link(dev, self.wake()) {
                Ok(link) => {
                    log(&mut self.diag, "GATT Battery Service opened, notifications requested");
                    self.le_link = Some(link);
                    self.le_link_failed_at = None;
                }
                Err(e) => {
                    self.diag.gatt = e.clone();
                    log(&mut self.diag, format!("GATT: {e}"));
                    self.le_link_failed_at = Some(Instant::now());
                }
            }
        }
    }

    fn enumerate(selector: windows::core::Result<HSTRING>) -> windows::core::Result<Vec<(String, HSTRING)>> {
        let sel = selector?;
        let infos = DeviceInformation::FindAllAsyncAqsFilter(&sel)?.get()?;
        let mut out = vec![];
        for info in infos {
            let name = info.Name().map(|s| s.to_string()).unwrap_or_default();
            if let Ok(id) = info.Id() {
                out.push((name, id));
            }
        }
        Ok(out)
    }

    fn open_le_link(dev: &BluetoothLEDevice, tx: Sender<Msg>) -> Result<LeLink, String> {
        let svc_uuid = GattServiceUuids::Battery().map_err(|e| e.to_string())?;
        let res = dev
            .GetGattServicesForUuidWithCacheModeAsync(svc_uuid, BluetoothCacheMode::Uncached)
            .and_then(|o| o.get())
            .map_err(|e| format!("Service discovery failed: {e}"))?;
        let status = res.Status().map_err(|e| e.to_string())?;
        if status != GattCommunicationStatus::Success {
            return Err(format!("Service discovery status: {}", comm_status(status)));
        }
        let services = res.Services().map_err(|e| e.to_string())?;
        if services.Size().unwrap_or(0) == 0 {
            return Err("Device does not expose the GATT Battery Service (0x180F)".into());
        }
        let service = services.GetAt(0).map_err(|e| e.to_string())?;
        let ch_uuid = GattCharacteristicUuids::BatteryLevel().map_err(|e| e.to_string())?;
        let cres = service
            .GetCharacteristicsForUuidWithCacheModeAsync(ch_uuid, BluetoothCacheMode::Uncached)
            .and_then(|o| o.get())
            .map_err(|e| format!("Characteristic discovery failed: {e}"))?;
        if cres.Status().map_err(|e| e.to_string())? != GattCommunicationStatus::Success {
            return Err("Battery Level characteristic not accessible".into());
        }
        let chars = cres.Characteristics().map_err(|e| e.to_string())?;
        if chars.Size().unwrap_or(0) == 0 {
            return Err("Battery Level characteristic (0x2A19) missing".into());
        }
        let level = chars.GetAt(0).map_err(|e| e.to_string())?;

        // Change notifications (optional per spec).
        let mut token = None;
        let notify_ok = level
            .WriteClientCharacteristicConfigurationDescriptorAsync(
                GattClientCharacteristicConfigurationDescriptorValue::Notify,
            )
            .and_then(|o| o.get())
            .map(|s| s == GattCommunicationStatus::Success)
            .unwrap_or(false);
        if notify_ok {
            token = level
                .ValueChanged(&TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(
                    move |_, _| {
                        let _ = tx.send(Msg::Refresh { full: false });
                        Ok(())
                    },
                ))
                .ok();
        }

        let status = service
            .GetCharacteristicsForUuidWithCacheModeAsync(UUID_BATTERY_LEVEL_STATUS, BluetoothCacheMode::Uncached)
            .and_then(|o| o.get())
            .ok()
            .and_then(|r| r.Characteristics().ok())
            .and_then(|c| if c.Size().unwrap_or(0) > 0 { c.GetAt(0).ok() } else { None });

        Ok(LeLink { device: dev.clone(), _service: service, level, status, _token: token })
    }

    fn comm_status(s: GattCommunicationStatus) -> &'static str {
        match s {
            GattCommunicationStatus::Unreachable => "unreachable",
            GattCommunicationStatus::ProtocolError => "protocol error",
            GattCommunicationStatus::AccessDenied => "access denied",
            _ => "error",
        }
    }

    fn read_value(ch: &GattCharacteristic) -> Result<Vec<u8>, String> {
        let r = ch
            .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)
            .and_then(|o| o.get())
            .map_err(|e| e.to_string())?;
        let st = r.Status().map_err(|e| e.to_string())?;
        if st != GattCommunicationStatus::Success {
            return Err(comm_status(st).into());
        }
        let buf = r.Value().map_err(|e| e.to_string())?;
        let len = buf.Length().map_err(|e| e.to_string())? as usize;
        let reader = DataReader::FromBuffer(&buf).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; len];
        reader.ReadBytes(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn read_byte(ch: &GattCharacteristic) -> Result<u8, String> {
        read_value(ch)?.first().copied().ok_or_else(|| "empty value".into())
    }

    /// Battery Level Status (0x2BED): flags u8, power state u16 (LE).
    /// Bits 5–6 of power state: 1 = charging, 2/3 = discharging.
    fn read_charge_state(ch: &GattCharacteristic) -> Option<&'static str> {
        let v = read_value(ch).ok()?;
        if v.len() < 3 {
            return None;
        }
        let power = u16::from_le_bytes([v[1], v[2]]);
        match (power >> 5) & 0b11 {
            1 => Some("charging"),
            2 | 3 => Some("discharging"),
            _ => None,
        }
    }

    impl Drop for LeLink {
        fn drop(&mut self) {
            if let Some(t) = self._token {
                let _ = self.level.RemoveValueChanged(t);
            }
            let _ = &self.device;
        }
    }

    // ---- SetupAPI helpers ------------------------------------------------

    unsafe fn get_prop(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<(u32, Vec<u8>)> {
        let mut ty = DEVPROPTYPE(0);
        let mut size = 0u32;
        let _ = SetupDiGetDevicePropertyW(set, data, key, &mut ty, None, Some(&mut size), 0);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        SetupDiGetDevicePropertyW(set, data, key, &mut ty, Some(&mut buf), None, 0).ok()?;
        Some((ty.0, buf))
    }

    unsafe fn get_string(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
        let (ty, buf) = get_prop(set, data, key)?;
        if ty != DEVPROP_TYPE_STRING {
            return None;
        }
        let wide: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let s = String::from_utf16_lossy(&wide);
        Some(s.trim_end_matches('\0').to_string())
    }

    unsafe fn get_battery(set: HDEVINFO, data: &SP_DEVINFO_DATA) -> Option<u8> {
        let (ty, buf) = get_prop(set, data, &KEY_BATTERY)?;
        if ty == DEVPROP_TYPE_BYTE && !buf.is_empty() {
            Some(buf[0])
        } else {
            None
        }
    }

    /// Enumerates every present device node that carries a battery value.
    fn scan_battery_nodes() -> Vec<BatteryNode> {
        let mut out = vec![];
        unsafe {
            let Ok(set) = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT) else {
                return out;
            };
            let mut i = 0u32;
            loop {
                let mut data = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
                if SetupDiEnumDeviceInfo(set, i, &mut data).is_err() {
                    break;
                }
                i += 1;
                let Some(pct) = get_battery(set, &data) else { continue };
                let name = get_string(set, &data, &KEY_FRIENDLY_NAME)
                    .or_else(|| get_string(set, &data, &KEY_NAME))
                    .unwrap_or_default();
                let instance_id = get_string(set, &data, &KEY_INSTANCE_ID).unwrap_or_default();
                out.push(BatteryNode { matched: is_supported_bt(&name), name, instance_id, percent: pct });
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
        }
        out
    }

    /// Re-reads the battery value of one known device node (cheap, used every tick).
    fn read_battery_by_id(instance_id: &str) -> Option<u8> {
        unsafe {
            let set = SetupDiCreateDeviceInfoList(None, None).ok()?;
            let wide: Vec<u16> = instance_id.encode_utf16().chain(std::iter::once(0)).collect();
            let mut data = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            let r = if SetupDiOpenDeviceInfoW(set, PCWSTR(wide.as_ptr()), None, 0, Some(&mut data)).is_ok() {
                get_battery(set, &data)
            } else {
                None
            };
            let _ = SetupDiDestroyDeviceInfoList(set);
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_arctis3, is_supported_bt, parse_hid_status};

    #[test]
    fn parses_nova_status() {
        // Captured from a real Arctis Nova 3X dongle.
        let r = [0xb0, 0x03, 0x02, 0x16, 0x03, 0x64, 0x64, 0x00];
        assert_eq!(parse_hid_status(&r), Some((true, 22, false)));
        assert_eq!(parse_hid_status(&[0xb0, 0x02, 0x02, 0x16, 0x00]), Some((false, 22, false)));
        // Same dongle with the USB-C cable plugged in (charging).
        let c = [0xb0, 0x03, 0x02, 0x1c, 0x01, 0x64, 0x64, 0x00];
        assert_eq!(parse_hid_status(&c), Some((true, 28, true)));
        assert_eq!(parse_hid_status(&[0xb1, 0x03, 0x02, 0x50, 0x01]), None);
    }

    #[test]
    fn matches_only_arctis_3() {
        assert!(is_arctis3("Arctis 3 Bluetooth"));
        assert!(is_arctis3("Headset (Arctis 3 Bluetooth Hands-Free AG Audio)"));
        assert!(is_arctis3("SteelSeries Arctis 3"));
        assert!(!is_arctis3("Arctis Nova 3X Wireless"));
        assert!(!is_arctis3("Arctis Nova 3"));
        assert!(!is_arctis3("Arctis 3X"));
        assert!(!is_arctis3("Arctis 30"));
        assert!(!is_arctis3("Arctis 7"));
        assert!(is_supported_bt("Arctis Nova 3P Wireless"));
        assert!(is_supported_bt("Arctis Nova 3X Wireless"));
        assert!(is_supported_bt("Arctis 3 Bluetooth"));
        assert!(!is_supported_bt("Arctis Nova 7"));
        assert!(is_supported_bt("Arctis Nova 5"));
        assert!(is_supported_bt("Arctis Nova 5X"));
        assert!(!is_supported_bt("Arctis Nova 3"));
    }
}
