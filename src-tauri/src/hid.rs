//! SteelSeries 2.4 GHz dongles (Arctis Nova 3P / 3X) over USB HID.
//! Cross-platform (hidapi): used by both the Windows and the macOS monitor.
//!
//! The dongle also carries the headset's audio, so it is queried gently: one
//! request every 30 s, no retries, and a growing pause after any failure.
//! Hammering it can hang the dongle until it is replugged.

use crate::battery::{now_ms, parse_hid_status, BatteryState, Diagnostics, HID_DEVICES, HID_VENDOR};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(30);
/// How often to look for a dongle when none is plugged in.
const RESCAN: Duration = Duration::from_secs(60);
/// After a failed query wait at least this long, doubling up to BACKOFF_MAX.
const BACKOFF_MIN: Duration = Duration::from_secs(120);
const BACKOFF_MAX: Duration = Duration::from_secs(900);

#[derive(Clone)]
pub struct HidReading {
    pub name: String,
    pub online: bool,
    pub level: u8,
    pub charging: bool,
    pub at: u64,
}

pub struct HidSource {
    api: Option<hidapi::HidApi>,
    dev: Option<(String, hidapi::HidDevice)>,
    /// Last good reading, reused between (infrequent) queries.
    last: Option<HidReading>,
    /// Earliest moment the dongle may be touched again.
    next_at: Option<Instant>,
    /// Grows after every failed query; zero when healthy.
    backoff: Duration,
}

impl HidSource {
    pub fn new() -> Self {
        Self { api: hidapi::HidApi::new().ok(), dev: None, last: None, next_at: None, backoff: Duration::ZERO }
    }

    /// Manual refresh: may skip the 30 s wait, but never an error back-off.
    pub fn allow_now(&mut self) {
        if self.backoff.is_zero() {
            self.next_at = None;
        }
    }

    /// Talks to the dongle at most every POLL; otherwise returns the cached reading.
    pub fn read(&mut self, diag: &mut Diagnostics) -> Option<HidReading> {
        let now = Instant::now();
        if self.next_at.map_or(false, |t| now < t) {
            return self.last.clone();
        }
        if self.dev.is_none() {
            self.open(diag);
            if self.dev.is_none() {
                self.last = None;
                self.next_at = Some(now + RESCAN);
                return None;
            }
        }
        let (name, dev) = self.dev.as_ref()?;
        let name = name.clone();
        match query(dev) {
            Ok((online, level, charging)) => {
                self.backoff = Duration::ZERO;
                self.next_at = Some(now + POLL);
                diag.hid = if online {
                    format!("Battery {level}%, {}", if charging { "charging" } else { "on battery" })
                } else {
                    "Dongle present, headset is off or out of range".into()
                };
                self.last = Some(HidReading { name, online, level, charging, at: now_ms() });
                self.last.clone()
            }
            Err(e) => {
                // Close the handle and leave the dongle alone for a while.
                self.dev = None;
                self.last = None;
                self.backoff = (self.backoff * 2).clamp(BACKOFF_MIN, BACKOFF_MAX);
                self.next_at = Some(now + self.backoff);
                let msg = format!("Dongle did not answer ({e}); next try in {} s", self.backoff.as_secs());
                diag.push_log(msg.clone());
                diag.hid = msg;
                None
            }
        }
    }

    fn open(&mut self, diag: &mut Diagnostics) {
        let Some(api) = self.api.as_mut() else {
            diag.hid = "HID API unavailable".into();
            return;
        };
        let _ = api.refresh_devices();
        diag.hid_devices.clear();
        let mut opened = None;
        let mut open_err = None;
        for d in api.device_list().filter(|d| d.vendor_id() == HID_VENDOR) {
            diag.hid_devices.push(format!(
                "{} · PID {:04X} · interface {} · usage page {:04X}",
                d.product_string().unwrap_or("SteelSeries device"),
                d.product_id(),
                d.interface_number(),
                d.usage_page()
            ));
            if opened.is_some() || d.usage_page() != 0xffc0 {
                continue;
            }
            if let Some((_, name)) = HID_DEVICES.iter().find(|(pid, _)| *pid == d.product_id()) {
                match d.open_device(api) {
                    Ok(dev) => opened = Some((name.to_string(), dev)),
                    Err(e) => open_err = Some(format!("Cannot open {name}: {e}")),
                }
            }
        }
        match (&opened, open_err) {
            (Some((name, _)), _) => diag.push_log(format!("Opened {name} dongle")),
            (None, Some(e)) => diag.hid = e,
            (None, None) => diag.hid = "No supported SteelSeries 2.4 GHz dongle found".into(),
        }
        self.dev = opened;
    }
}

/// Sends one `00 B0` request and waits for the status reply (skipping unrelated
/// input reports). Never retries: a failure is handled by the caller's back-off.
fn query(dev: &hidapi::HidDevice) -> Result<(bool, u8, bool), String> {
    // Drop stale input reports so an old reply is not mistaken for this one.
    let mut buf = [0u8; 64];
    while matches!(dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
    dev.write(&[0x00, 0xb0]).map_err(|e| e.to_string())?;
    for _ in 0..3 {
        let n = dev.read_timeout(&mut buf, 1000).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("no reply (timeout)".into());
        }
        if let Some(r) = parse_hid_status(&buf[..n]) {
            return Ok(r);
        }
    }
    Err("no status reply".into())
}

/// Picks the source to show. A Bluetooth connection wins only when it actually
/// reports a percentage: Nova 3P/3X can be connected over Bluetooth and the
/// dongle at the same time, and the OS often has no battery level for the
/// Bluetooth link, while the dongle does.
pub fn prefer(bt: BatteryState, hid: &mut HidSource, diag: &mut Diagnostics, force_full: bool) -> BatteryState {
    if bt.connection == "connected" && bt.percent.is_some() {
        return bt;
    }
    if force_full {
        hid.allow_now();
    }
    match hid.read(diag) {
        // Dongle reading, unless the headset is off on the dongle but connected over Bluetooth.
        Some(r) if r.online || bt.connection != "connected" => to_state(r, bt.bluetooth),
        _ => bt,
    }
}

/// Battery state for a dongle reading (`bluetooth` = radio state to keep showing).
pub fn to_state(r: HidReading, bluetooth: String) -> BatteryState {
    let mut st = BatteryState::initial();
    st.last_check = now_ms();
    st.bluetooth = bluetooth;
    st.transport = Some("usbDongle".into());
    st.detected = true;
    st.device_name = Some(r.name);
    if r.online {
        st.connection = "connected".into();
        if r.level <= 100 {
            st.percent = Some(r.level);
            st.source = Some("SteelSeries 2.4 GHz dongle (USB HID)".into());
            st.precision = Some("1%".into());
            st.charging = if r.charging { "charging" } else { "discharging" }.into();
            st.last_update = Some(r.at);
        }
    } else {
        st.connection = "disconnected".into();
    }
    st
}
