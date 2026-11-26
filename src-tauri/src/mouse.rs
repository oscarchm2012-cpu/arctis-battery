//! Ajazz AJ179 (PRO / APEX) mouse battery over its 2.4 GHz receiver (USB HID).
//!
//! Receiver protocol ("GearHub" family), as implemented by
//! OpenMouse-Project/mouse-protocol (AJ179 PRO, receiver 3151:402D) and
//! Teristans/ajazz-aj179-battery-tray (AJ179 APEX, receiver 3151:5007):
//!   - vendor collection: usage page 0xFFFF, usage 0x02, 64-byte feature report
//!   - `F6 05`  select the mouse as target (sent once per opened handle)
//!   - `F7`     status poll, then GET_FEATURE → 7-byte status:
//!       [0] can read (1)   [1] keyboard battery   [2] MOUSE BATTERY %
//!       [3] keyboard online (0 = online)   [4] MOUSE ONLINE (0 = online)
//!       [5] can send (1)   [6] 0x02
//!     e.g. `01 00 55 01 00 01 02` = mouse online, 85 %.
//! Depending on OS and report numbering the status is preceded by a report-id
//! byte (`00` or `05`), so it is located by its shape, not a fixed offset.
//! Battery 0 = not populated yet (or mouse asleep) → never shown as 0 %.
//! Queried at most once a minute, with a growing pause after failures.

use serde::Serialize;
use std::time::{Duration, Instant};

const VID: u16 = 0x3151;
/// Receivers (mouse on 2.4 GHz).
const WIRELESS: &[(u16, &str)] = &[
    (0x402d, "Ajazz AJ179 PRO"),
    (0x5007, "Ajazz AJ179 APEX"),
    (0x4027, "Ajazz AJ179"),
];
/// The mouse itself on the USB cable (charging).
const WIRED: &[(u16, &str)] = &[
    (0x4026, "Ajazz AJ179"),
    (0x4006, "Ajazz AJ179"),
    (0x5008, "Ajazz AJ179"),
];

const POLL: Duration = Duration::from_secs(60);
const RESCAN: Duration = Duration::from_secs(60);
const BACKOFF_MIN: Duration = Duration::from_secs(120);
const BACKOFF_MAX: Duration = Duration::from_secs(900);
/// Status polls per reading (OpenMouse waits up to 8 × 100 ms for readiness).
const POLLS: u32 = 10;
const POLL_GAP: Duration = Duration::from_millis(150);
/// How long a reading stays valid while the mouse sleeps.
const KEEP_WHILE_ASLEEP_MS: u64 = 30 * 60 * 1000;

const CMD_SELECT_TARGET: u8 = 0xf6;
const TARGET_MOUSE: u8 = 0x05;
const CMD_GET_STATUS: u8 = 0xf7;

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MouseState {
    pub name: String,
    pub percent: Option<u8>,
    pub charging: bool,
    /// Receiver present but the mouse is not linked / did not report (asleep).
    pub asleep: bool,
    pub last_update: Option<u64>,
}

/// Decoded receiver status.
#[derive(Debug, PartialEq)]
pub struct Status {
    pub can_read: bool,
    pub mouse_battery: u8,
    pub mouse_online: bool,
}

impl Status {
    pub fn percent(&self) -> Option<u8> {
        (self.mouse_online && (1..=100).contains(&self.mouse_battery)).then_some(self.mouse_battery)
    }
}

/// Finds the 7-byte status in a feature-report buffer (with or without a
/// leading report-id byte) by its shape: flags at [3], [4], [5] are 0/1 and
/// [6] is 0x02.
pub fn parse_status(data: &[u8]) -> Option<Status> {
    for o in 0..=2 {
        let s = data.get(o..o + 7)?;
        if s[6] == 0x02 && s[3] <= 1 && s[4] <= 1 && s[5] <= 1 && s[0] <= 1 {
            return Some(Status { can_read: s[0] == 1, mouse_battery: s[2], mouse_online: s[4] == 0 });
        }
    }
    None
}

fn hex(b: &[u8]) -> String {
    b.iter().take(10).map(|v| format!("{v:02x}")).collect::<Vec<_>>().join(" ")
}

pub struct MouseSource {
    api: Option<hidapi::HidApi>,
    next_at: Option<Instant>,
    backoff: Duration,
    last: Option<MouseState>,
    pub status: String,
}

enum Query {
    Absent,
    Reading(MouseState),
    Asleep { name: String, detail: String },
}

impl MouseSource {
    pub fn new() -> Self {
        Self { api: hidapi::HidApi::new().ok(), next_at: None, backoff: Duration::ZERO, last: None, status: String::new() }
    }

    /// Current mouse state; talks to the receiver at most once per POLL.
    pub fn read(&mut self, force: bool) -> Option<MouseState> {
        let now = Instant::now();
        if force && self.backoff.is_zero() {
            self.next_at = None;
        }
        if self.next_at.map_or(false, |t| now < t) {
            return self.last.clone();
        }
        match self.query() {
            Query::Absent => {
                self.last = None;
                self.next_at = Some(now + RESCAN);
                self.status = "No Ajazz AJ179 receiver found".into();
            }
            Query::Reading(st) => {
                self.backoff = Duration::ZERO;
                self.next_at = Some(now + POLL);
                self.last = Some(st);
            }
            Query::Asleep { name, detail } => {
                self.backoff = (self.backoff * 2).clamp(BACKOFF_MIN, BACKOFF_MAX);
                self.next_at = Some(now + self.backoff);
                self.status = format!("{name}: {detail}; next try in {} s", self.backoff.as_secs());
                // Keep the last value for a while: a sleeping mouse still has that charge.
                let (percent, last_update) = match &self.last {
                    Some(l)
                        if l.name == name
                            && l.last_update.map_or(false, |t| crate::battery::now_ms().saturating_sub(t) < KEEP_WHILE_ASLEEP_MS) =>
                    {
                        (l.percent, l.last_update)
                    }
                    _ => (None, None),
                };
                self.last = Some(MouseState { name, percent, charging: false, asleep: true, last_update });
            }
        }
        self.last.clone()
    }

    fn query(&mut self) -> Query {
        let Some(api) = self.api.as_mut() else {
            self.status = "HID API unavailable".into();
            return Query::Absent;
        };
        let _ = api.refresh_devices();
        let mut wireless = None;
        let mut wired = None;
        for d in api.device_list().filter(|d| d.vendor_id() == VID && d.usage_page() == 0xffff) {
            let pid = d.product_id();
            let pick = |list: &[(u16, &'static str)]| list.iter().find(|(p, _)| *p == pid).map(|(_, n)| *n);
            // Prefer the documented control collection (usage 0x02).
            let better = |cur: &Option<(&'static str, hidapi::DeviceInfo)>| {
                cur.as_ref().map_or(true, |(_, c)| c.usage() != 0x02 && d.usage() == 0x02)
            };
            if let Some(n) = pick(WIRELESS) {
                if better(&wireless) {
                    wireless = Some((n, d.clone()));
                }
            } else if let Some(n) = pick(WIRED) {
                if better(&wired) {
                    wired = Some((n, d.clone()));
                }
            }
        }
        let (name, info, charging) = match (wireless, wired) {
            (Some((n, d)), _) => (n, d, false),
            (None, Some((n, d))) => (n, d, true),
            (None, None) => return Query::Absent,
        };
        let name = name.to_string();
        let dev = match info.open_device(api) {
            Ok(d) => d,
            Err(e) => return Query::Asleep { name, detail: format!("cannot open receiver: {e}") },
        };

        let send = |cmd: &[u8]| -> Result<(), String> {
            let mut buf = [0u8; 65]; // [report id 0x00] + 64-byte payload
            buf[1..1 + cmd.len()].copy_from_slice(cmd);
            dev.send_feature_report(&buf).map_err(|e| e.to_string())
        };
        // Report 0 is what OpenMouse reads; 0x05 is the numbered variant (APEX).
        let read = |id: u8| -> Result<Vec<u8>, String> {
            let mut buf = [0u8; 65];
            buf[0] = id;
            let n = dev.get_feature_report(&mut buf).map_err(|e| e.to_string())?;
            Ok(buf[..n.min(buf.len())].to_vec())
        };

        // Select the mouse as target, as the vendor driver / OpenMouse do.
        if let Err(e) = send(&[CMD_SELECT_TARGET, TARGET_MOUSE]) {
            return Query::Asleep { name, detail: format!("select target failed: {e}") };
        }
        std::thread::sleep(Duration::from_millis(50));

        let mut last_seen = String::from("no reply");
        for round in 0..POLLS {
            if let Err(e) = send(&[CMD_GET_STATUS]) {
                return Query::Asleep { name, detail: format!("status poll failed: {e}") };
            }
            std::thread::sleep(Duration::from_millis(10));
            for id in [0x00u8, 0x05] {
                let data = match read(id) {
                    Ok(d) => d,
                    Err(e) => {
                        last_seen = format!("report {id:02x}: {e}");
                        continue;
                    }
                };
                let Some(st) = parse_status(&data) else {
                    last_seen = format!("report {id:02x}: unrecognised {}", hex(&data));
                    continue;
                };
                last_seen = format!(
                    "mouse {}, battery byte {}, ready {} (raw {})",
                    if st.mouse_online { "linked" } else { "not linked (asleep / Bluetooth / cable)" },
                    st.mouse_battery,
                    if st.can_read { "yes" } else { "no" },
                    hex(&data)
                );
                if let Some(p) = st.percent() {
                    self.status = format!("{name}: {p}% after {} poll(s) (raw {})", round + 1, hex(&data));
                    return Query::Reading(MouseState {
                        name,
                        percent: Some(p),
                        charging,
                        asleep: false,
                        last_update: Some(crate::battery::now_ms()),
                    });
                }
                break; // recognised but not populated yet: poll again
            }
            std::thread::sleep(POLL_GAP);
        }
        Query::Asleep { name, detail: format!("no battery reading after {POLLS} polls — {last_seen}") }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_status, Status};

    fn pct(d: &[u8]) -> Option<u8> {
        parse_status(d).and_then(|s| s.percent())
    }

    #[test]
    fn parses_receiver_status() {
        // OpenMouse fixture for the AJ179 PRO receiver: mouse online, 85 %.
        assert_eq!(pct(&[0x01, 0x00, 0x55, 0x01, 0x00, 0x01, 0x02, 0, 0]), Some(85));
        // Same with hidapi's leading report-id byte.
        assert_eq!(pct(&[0x00, 0x01, 0x00, 0x55, 0x01, 0x00, 0x01, 0x02, 0]), Some(85));
        // OpenMouse doc example `01 00 2D 01 00 01` + 02: 45 %.
        assert_eq!(pct(&[0x01, 0x00, 0x2d, 0x01, 0x00, 0x01, 0x02]), Some(45));
        // AJ179 APEX numbered report (Teristans): 05 + status, 56 %.
        assert_eq!(pct(&[0x05, 0x00, 0x00, 0x38, 0x01, 0x00, 0x01, 0x02]), Some(56));
        // Real AJ179 PRO on macOS: mouse linked but battery not populated yet.
        assert_eq!(
            parse_status(&[0, 0, 0, 1, 0, 1, 2, 0, 0, 0]),
            Some(Status { can_read: false, mouse_battery: 0, mouse_online: true })
        );
        assert_eq!(pct(&[0, 0, 0, 0, 1, 0, 1, 2, 0, 0]), None);
        // Mouse not linked (byte 4 = 1).
        assert_eq!(pct(&[0x01, 0x00, 0x55, 0x01, 0x01, 0x01, 0x02]), None);
        // Low battery stays unambiguous.
        assert_eq!(pct(&[0x01, 0x00, 0x01, 0x01, 0x00, 0x01, 0x02]), Some(1));
        assert_eq!(pct(&[0x00, 0x01, 0x00, 0x01, 0x01, 0x00, 0x01, 0x02]), Some(1));
        // Garbage.
        assert_eq!(parse_status(&[0; 65]), None);
        assert_eq!(pct(&[0x05, 0x03, 0x00, 0x38]), None);
    }
}
