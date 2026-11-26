//! Ajazz AJ179 (PRO / APEX) mouse battery over its 2.4 GHz receiver (USB HID).
//!
//! Method used by the vendor driver's heartbeat, as documented by
//! Teristans/ajazz-aj179-battery-tray and Aiacos/ajazz-control-center:
//!   1. SET_FEATURE on the vendor collection (usage page 0xFFFF, usage 0x02):
//!      report 0x00, payload `F7 00 …` (64 bytes) — wakes the status report.
//!   2. wait ~50 ms
//!   3. GET_FEATURE report 0x05 → `05 00 00 <percent> …`; percent 1–100,
//!      0 / garbage = link not ready (mouse asleep), never shown as 0 %.
//! The AJ179 PRO receiver (3151:402D) is not documented by those projects; it
//! is matched by the same collection and parsed tolerantly (numbered or
//! unnumbered reports). Raw bytes are kept in the diagnostics.
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
/// Heartbeats per reading and the pause between them (~2.5 s in total).
const HEARTBEATS: u32 = 6;
const HEARTBEAT_GAP: Duration = Duration::from_millis(450);
const RESCAN: Duration = Duration::from_secs(60);
const BACKOFF_MIN: Duration = Duration::from_secs(120);
const BACKOFF_MAX: Duration = Duration::from_secs(900);
/// How long a reading stays valid while the mouse sleeps.
const KEEP_WHILE_ASLEEP_MS: u64 = 30 * 60 * 1000;

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MouseState {
    pub name: String,
    pub percent: Option<u8>,
    pub charging: bool,
    /// Receiver present but the mouse did not answer (asleep / out of range).
    pub asleep: bool,
    pub last_update: Option<u64>,
}

pub struct MouseSource {
    api: Option<hidapi::HidApi>,
    next_at: Option<Instant>,
    backoff: Duration,
    last: Option<MouseState>,
    pub status: String,
}

/// Finds the battery percent in a status feature report.
///
/// The frame is `[05] 00 00 P 01 xx 01 02 …` — P = percent, then a link-status
/// block. Depending on the OS / report numbering the leading `05` and zeros
/// shift by a byte (macOS returned `00 00 00 01 00 01 02` for report 05 and
/// `00 00 00 00 01 00 01 02` for report 00), so P is located as the byte right
/// before the `01 xx 01 02` block, preceded only by zeros.
/// P = 0 means "link not ready" (mouse asleep / not yet polled) → None.
pub fn parse_frame(data: &[u8]) -> Option<u8> {
    let body = match data {
        [0x05, rest @ ..] => rest,
        [0x00, 0x05, rest @ ..] => rest,
        _ => data,
    };
    for i in 1..=4 {
        if body.len() > i + 4
            && body[..i].iter().all(|&b| b == 0)
            && body[i + 1] == 0x01
            && body[i + 3] == 0x01
            && body[i + 4] == 0x02
        {
            let p = body[i];
            return (1..=100).contains(&p).then_some(p);
        }
    }
    // Fallback: plain numbered frame without the status block.
    if let [0x05, 0x00, 0x00, p, ..] = data {
        return (1..=100).contains(p).then_some(*p);
    }
    None
}

fn hex(b: &[u8]) -> String {
    b.iter().take(10).map(|v| format!("{v:02x}")).collect::<Vec<_>>().join(" ")
}

impl MouseSource {
    pub fn new() -> Self {
        Self {
            api: hidapi::HidApi::new().ok(),
            next_at: None,
            backoff: Duration::ZERO,
            last: None,
            status: String::new(),
        }
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
        let result = self.query();
        match result {
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
                self.status = format!("{name}: no battery reading ({detail}); next try in {} s", self.backoff.as_secs());
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
            Err(e) => return Query::Asleep { name, detail: format!("cannot open: {e}") },
        };

        // The vendor driver sends the F7 heartbeat about once a second, and the
        // receiver fills in the percent only once the mouse has answered the
        // relayed request. A single poll + 50 ms read gave "00" (link not
        // ready) on an AJ179 PRO, so send a short burst and read after each.
        let mut tried = vec![];
        for round in 0..HEARTBEATS {
            let mut poll = [0u8; 65];
            poll[1] = 0xf7;
            if let Err(e) = dev.send_feature_report(&poll) {
                return Query::Asleep { name, detail: format!("poll failed: {e}") };
            }
            std::thread::sleep(if round == 0 { Duration::from_millis(50) } else { HEARTBEAT_GAP });

            // Status report: numbered 0x05 first, then unnumbered.
            tried.clear();
            for id in [0x05u8, 0x00] {
                let mut buf = [0u8; 65];
                buf[0] = id;
                match dev.get_feature_report(&mut buf) {
                    Ok(n) => {
                        let data = &buf[..n.min(buf.len())];
                        tried.push(format!("id {id:02x}: {}", hex(data)));
                        if let Some(p) = parse_frame(data) {
                            self.status = format!("{name}: {p}% after {} heartbeat(s) (raw {})", round + 1, hex(data));
                            return Query::Reading(MouseState {
                                name,
                                percent: Some(p),
                                charging,
                                asleep: false,
                                last_update: Some(crate::battery::now_ms()),
                            });
                        }
                    }
                    Err(e) => tried.push(format!("id {id:02x}: {e}")),
                }
            }
        }
        Query::Asleep { name, detail: format!("after {HEARTBEATS} heartbeats: {}", tried.join(" | ")) }
    }
}

enum Query {
    Absent,
    Reading(MouseState),
    Asleep { name: String, detail: String },
}

#[cfg(test)]
mod tests {
    use super::parse_frame;

    #[test]
    fn parses_status_frames() {
        // Examples from Teristans/ajazz-aj179-battery-tray.
        assert_eq!(parse_frame(&[0x05, 0, 0, 0x38, 1, 0, 1, 2]), Some(56));
        assert_eq!(parse_frame(&[0x05, 0, 0, 0x64, 1, 1, 1, 2]), Some(100));
        assert_eq!(parse_frame(&[0x00, 0x05, 0, 0, 0x1f, 1, 0, 1, 2]), Some(31));
        // Real AJ179 PRO receiver on macOS, mouse link not ready (percent byte 0).
        assert_eq!(parse_frame(&[0, 0, 0, 1, 0, 1, 2, 0, 0, 0]), None);
        assert_eq!(parse_frame(&[0, 0, 0, 0, 1, 0, 1, 2, 0, 0]), None);
        // The same two layouts with a real percent in place.
        assert_eq!(parse_frame(&[0, 0, 0x4b, 1, 0, 1, 2, 0, 0, 0]), Some(75));
        assert_eq!(parse_frame(&[0, 0, 0, 0x4b, 1, 0, 1, 2, 0, 0]), Some(75));
        // 1 % must not be confused with the status block.
        assert_eq!(parse_frame(&[0, 0, 0, 1, 1, 0, 1, 2, 0, 0]), Some(1));
        assert_eq!(parse_frame(&[0x05, 0, 0, 0, 0, 0]), None); // link not ready
        assert_eq!(parse_frame(&[0x05, 3, 0, 0x38]), None); // reconnect garbage
        assert_eq!(parse_frame(&[0x05, 0, 0, 0xff]), None);
        assert_eq!(parse_frame(&[0; 65]), None);
    }
}
