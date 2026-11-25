//! macOS monitor.
//!
//!  1. Bluetooth: the battery level macOS itself receives from the headset, as
//!     listed by `system_profiler SPBluetoothDataType` (the same data as System
//!     Information). Queried every 20 s because the tool takes ~1 s.
//!  2. SteelSeries 2.4 GHz dongle (Nova 3P / 3X) over USB HID, shared with Windows.

use crate::battery::{is_supported_bt, now_ms, BatteryState, DeviceEntry, Diagnostics, Msg};
use crate::hid::HidSource;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const BT_EVERY: Duration = Duration::from_secs(20);

#[derive(Clone)]
struct BtDevice {
    name: String,
    connected: bool,
    percent: Option<u8>,
}

struct BtScan {
    radio: String,
    device: Option<BtDevice>,
    all: Vec<DeviceEntry>,
}

pub struct Monitor {
    _tx: Sender<Msg>,
    hid: HidSource,
    bt: Option<BtScan>,
    bt_at: Option<Instant>,
    bt_good_at: Option<u64>,
    pub diag: Diagnostics,
}

/// "80%" / "80" → 80
fn parse_percent(v: &serde_json::Value) -> Option<u8> {
    let s = v.as_str()?.trim().trim_end_matches('%').trim();
    s.parse::<u8>().ok().filter(|p| *p <= 100)
}

fn scan_bt() -> Result<BtScan, String> {
    let out = Command::new("/usr/sbin/system_profiler")
        .args(["SPBluetoothDataType", "-json", "-detailLevel", "basic"])
        .output()
        .map_err(|e| format!("system_profiler failed: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("unexpected system_profiler output: {e}"))?;
    let root = &v["SPBluetoothDataType"][0];
    let radio = match root["controller_properties"]["controller_state"].as_str() {
        Some("attrib_on") => "on",
        Some("attrib_off") => "off",
        None if root.is_null() => "noAdapter",
        _ => "unknown",
    }
    .to_string();

    let mut device: Option<BtDevice> = None;
    let mut all = vec![];
    for (key, connected) in [("device_connected", true), ("device_not_connected", false)] {
        let Some(list) = root[key].as_array() else { continue };
        for item in list {
            let Some(obj) = item.as_object() else { continue };
            for (name, props) in obj {
                let matched = is_supported_bt(name);
                // Prefer the main level; fall back to any per-bud level, never the case.
                let percent = parse_percent(&props["device_batteryLevelMain"])
                    .or_else(|| parse_percent(&props["device_batteryLevel"]))
                    .or_else(|| {
                        props.as_object().and_then(|m| {
                            m.iter()
                                .filter(|(k, _)| k.starts_with("device_batteryLevel") && !k.contains("Case"))
                                .find_map(|(_, v)| parse_percent(v))
                        })
                    });
                all.push(DeviceEntry {
                    name: name.clone(),
                    id: props["device_address"].as_str().unwrap_or_default().to_string(),
                    connected: Some(connected),
                    matched,
                });
                let better = match &device {
                    None => true,
                    Some(d) => connected && !d.connected,
                };
                if matched && better {
                    device = Some(BtDevice { name: name.clone(), connected, percent });
                }
            }
        }
    }
    Ok(BtScan { radio, device, all })
}

impl Monitor {
    pub fn new(tx: Sender<Msg>) -> Self {
        Self { _tx: tx, hid: HidSource::new(), bt: None, bt_at: None, bt_good_at: None, diag: Diagnostics::default() }
    }

    /// A connected Bluetooth headset wins; otherwise a 2.4 GHz dongle if present.
    pub fn read(&mut self, force_full: bool) -> BatteryState {
        if force_full || self.bt_at.map_or(true, |t| t.elapsed() >= BT_EVERY) {
            self.bt_at = Some(Instant::now());
            self.diag.generated_at = now_ms();
            match scan_bt() {
                Ok(scan) => {
                    self.diag.radios = vec![format!("Bluetooth — {}", scan.radio)];
                    self.diag.classic_devices = scan.all.clone();
                    self.diag.hfp = match &scan.device {
                        Some(d) => match d.percent {
                            Some(p) => format!("macOS reports {p}% for \"{}\"", d.name),
                            None => format!("macOS reports no battery level for \"{}\"", d.name),
                        },
                        None => "No paired Arctis 3 / Nova 3P / Nova 3X".into(),
                    };
                    self.bt = Some(scan);
                }
                Err(e) => {
                    self.diag.push_log(e.clone());
                    self.diag.hfp = e;
                }
            }
            self.diag.gatt = "Not used on macOS (battery comes from the macOS Bluetooth stack)".into();
        }

        let mut bt = BatteryState::initial();
        bt.last_check = now_ms();
        if let Some(scan) = &self.bt {
            bt.bluetooth = scan.radio.clone();
            if let Some(d) = &scan.device {
                bt.detected = true;
                bt.transport = Some("bluetooth".into());
                bt.device_name = Some(d.name.clone());
                bt.connection = if d.connected { "connected" } else { "disconnected" }.into();
                if d.connected {
                    if let Some(p) = d.percent {
                        bt.percent = Some(p);
                        bt.source = Some("macOS Bluetooth".into());
                        bt.precision = Some(if p % 10 == 0 { "10% steps".into() } else { "1%".into() });
                        // Timestamp of the system_profiler scan that produced it.
                        let at = self.bt_good_at.filter(|_| self.bt_at.map_or(false, |t| t.elapsed() < BT_EVERY));
                        bt.last_update = Some(at.unwrap_or(bt.last_check));
                        self.bt_good_at = bt.last_update;
                    }
                }
            } else {
                bt.connection = "notFound".into();
            }
        } else {
            bt.connection = "notFound".into();
        }
        crate::hid::prefer(bt, &mut self.hid, &mut self.diag, force_full)
    }
}
