#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod battery;
mod hid;
mod icon;
#[cfg(target_os = "macos")]
mod mac;
mod mouse;
mod settings;
mod tray;

use battery::{BatteryState, Diagnostics, Msg};
use settings::Settings;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_notification::NotificationExt;

struct AppState {
    battery: Mutex<BatteryState>,
    diag: Mutex<Diagnostics>,
    tx: Mutex<Option<Sender<Msg>>>,
    settings: Mutex<Settings>,
    pulse: Arc<AtomicBool>,
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn request_refresh(app: &AppHandle, full: bool) {
    if let Some(tx) = app.state::<AppState>().tx.lock().unwrap().as_ref() {
        let _ = tx.send(Msg::Refresh { full });
    }
}

// ---- commands -----------------------------------------------------------

#[tauri::command]
fn get_state(state: tauri::State<AppState>) -> BatteryState {
    state.battery.lock().unwrap().clone()
}

#[tauri::command]
fn get_diagnostics(state: tauri::State<AppState>) -> Diagnostics {
    state.diag.lock().unwrap().clone()
}

#[tauri::command]
fn refresh(app: AppHandle) {
    request_refresh(&app, true);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    notifications: bool,
    autostart: bool,
    version: String,
}

#[tauri::command]
fn get_settings(app: AppHandle, state: tauri::State<AppState>) -> SettingsView {
    SettingsView {
        notifications: state.settings.lock().unwrap().notifications,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
fn set_notifications(app: AppHandle, state: tauri::State<AppState>, enabled: bool) {
    let mut s = state.settings.lock().unwrap();
    s.notifications = enabled;
    settings::save(&app, &s);
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    al.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// ---- monitor thread -----------------------------------------------------

struct Alerts {
    sent20: bool,
    sent10: bool,
}

impl Alerts {
    /// Notify once when crossing 20% and 10%; re-arm after recovering by 5 points.
    fn check(&mut self, app: &AppHandle, p: u8, enabled: bool) {
        if p >= 25 {
            self.sent20 = false;
        }
        if p >= 15 {
            self.sent10 = false;
        }
        let msg = if p <= 10 && !self.sent10 {
            self.sent10 = true;
            self.sent20 = true;
            Some(("Headset battery critical", format!("Battery at {p}%. Charge your headset now.")))
        } else if p <= 20 && !self.sent20 {
            self.sent20 = true;
            Some(("Headset battery low", format!("Battery at {p}%.")))
        } else {
            None
        };
        if let (Some((title, body)), true) = (msg, enabled) {
            let _ = app.notification().builder().title(title).body(body).show();
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn spawn_monitor(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<Msg>();
    *app.state::<AppState>().tx.lock().unwrap() = Some(tx.clone());

    std::thread::Builder::new()
        .name("battery-monitor".into())
        .spawn(move || {
            let mut mon = battery::Monitor::new(tx);
            let mut mouse = mouse::MouseSource::new();
            let mut alerts = Alerts { sent20: false, sent10: false };
            let mut full = true;
            loop {
                let mut st = mon.read(full);
                st.mouse = mouse.read(full);
                let s = app.state::<AppState>();
                {
                    let mut d = mon.diag.clone();
                    d.mouse = mouse.status.clone();
                    *s.diag.lock().unwrap() = d;
                }
                let changed = {
                    let mut cur = s.battery.lock().unwrap();
                    let changed = !cur.same_reading(&st) || cur.last_check == 0;
                    *cur = st.clone();
                    changed
                };
                s.pulse.store(tray::needs_pulse(&st), Ordering::Relaxed);
                if changed {
                    tray::update(&app, &st, false);
                }
                if let Some(p) = st.percent {
                    let enabled = s.settings.lock().unwrap().notifications;
                    alerts.check(&app, p, enabled);
                }
                let _ = app.emit("battery-update", &st);

                full = false;
                match rx.recv_timeout(battery::POLL_INTERVAL) {
                    Ok(Msg::Refresh { full: f }) => {
                        full = f;
                        // Coalesce bursts of change events.
                        while let Ok(Msg::Refresh { full: f2 }) = rx.recv_timeout(Duration::from_millis(150)) {
                            full |= f2;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("spawn monitor");
}

/// Blinks the tray icon while the battery is below 10%.
fn spawn_pulser(app: AppHandle, pulse: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut phase = false;
        let mut was = false;
        let mut tick = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(600));
            tick = tick.wrapping_add(1);
            let on = pulse.load(Ordering::Relaxed);
            // Redraw for the new taskbar colours after a light/dark theme switch.
            if tick % 8 == 0 && !on && tray::theme_changed() {
                let st = app.state::<AppState>().battery.lock().unwrap().clone();
                tray::update(&app, &st, false);
            }
            if on {
                phase = !phase;
                let st = app.state::<AppState>().battery.lock().unwrap().clone();
                tray::update(&app, &st, phase);
            } else if was {
                let st = app.state::<AppState>().battery.lock().unwrap().clone();
                tray::update(&app, &st, false);
            }
            was = on;
        }
    });
}

fn main() {
    let pulse = Arc::new(AtomicBool::new(false));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .manage(AppState {
            battery: Mutex::new(BatteryState::initial()),
            diag: Mutex::new(Diagnostics::default()),
            tx: Mutex::new(None),
            settings: Mutex::new(Settings::default()),
            pulse: pulse.clone(),
        })
        .setup(move |app| {
            // Menu-bar app on macOS: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();

            let mut s = settings::load(&handle);
            if !s.autostart_initialized {
                let _ = handle.autolaunch().enable();
                s.autostart_initialized = true;
                settings::save(&handle, &s);
            }
            let pin_needed = !s.tray_pinned;
            *handle.state::<AppState>().settings.lock().unwrap() = s;

            tray::create(&handle)?;

            // Explorer registers the icon a moment after it appears; promote it once.
            if pin_needed {
                let h = handle.clone();
                std::thread::spawn(move || {
                    for _ in 0..60 {
                        std::thread::sleep(Duration::from_secs(2));
                        if tray::promote_icon() {
                            let st = h.state::<AppState>();
                            let mut s = st.settings.lock().unwrap();
                            s.tray_pinned = true;
                            settings::save(&h, &s);
                            break;
                        }
                    }
                });
            }

            if !std::env::args().any(|a| a == "--minimized") {
                show_main(&handle);
            }

            #[cfg(any(windows, target_os = "macos"))]
            spawn_monitor(handle.clone());
            spawn_pulser(handle, pulse.clone());
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Resized(_) => {
                if window.is_minimized().unwrap_or(false) {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_diagnostics,
            refresh,
            get_settings,
            set_notifications,
            set_autostart,
            hide_window,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arctis 3 Battery Monitor");
}
