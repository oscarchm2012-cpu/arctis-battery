//! Dynamic tray icon: duck headphones (see `icon.rs`) with the battery
//! percentage in the middle, rendered at the tray's native size.

use crate::battery::BatteryState;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub const TRAY_ID: &str = "main";

type Rgb = (u8, u8, u8);
const GREY: Rgb = (0xd0, 0xd0, 0xd6);

/// Colour band per spec; `true` = must blink (1–9 %).
fn level_color(p: u8) -> (Rgb, bool) {
    match p {
        50..=255 => ((0x3d, 0xe0, 0x6e), false),
        20..=49 => ((0xff, 0xf0, 0x40), false),
        10..=19 => ((0xff, 0x4d, 0x4d), false),
        _ => ((0xff, 0x4d, 0x4d), true),
    }
}

pub fn needs_pulse(st: &BatteryState) -> bool {
    matches!(st.percent, Some(p) if p < 10)
}

/// The artwork has its own dark outline, so it needs no light/dark variant.
pub fn theme_changed() -> bool {
    false
}

/// Small-icon size for the system DPI (16 at 96 DPI, 24 at 144, 32 at 192 …).
fn icon_px() -> usize {
    // macOS menu bar: icons are shown 18 pt tall; 36 px is crisp on Retina.
    #[cfg(target_os = "macos")]
    {
        36
    }
    #[cfg(windows)]
    {
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
        ((16 * dpi.max(96) + 48) / 96) as usize
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        16
    }
}

/// `pulse_phase` = second half of the blink cycle (digits dimmed).
pub fn icon_for(st: &BatteryState, pulse_phase: bool) -> Image<'static> {
    let (text, color, dim) = match st.percent {
        Some(p) => {
            let (c, blink) = level_color(p.min(100));
            (p.min(100).to_string(), c, blink && pulse_phase)
        }
        None if st.connection == "connected" => ("?".to_string(), GREY, false),
        None => ("-".to_string(), GREY, false),
    };
    let n = icon_px();
    let px = crate::icon::render(n, &text, color, if dim { 0.25 } else { 1.0 })
        .unwrap_or_else(|| vec![0u8; n * n * 4]);
    Image::new_owned(px, n as u32, n as u32)
}

// ---- keep the icon next to the arrow, not in the overflow -----------------

/// Windows 11 hides new tray icons in the overflow flyout. Explorer records each
/// icon under HKCU\Control Panel\NotifyIconSettings; IsPromoted = 1 shows it on
/// the taskbar. Returns true once our entry was found (and promoted).
#[cfg(windows)]
pub fn promote_icon() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    let Ok(exe) = std::env::current_exe() else { return false };
    let full = exe.to_string_lossy().to_lowercase();
    // Explorer may record a different (e.g. redirected) path than current_exe()
    // reports, so also match on "<install folder>\<file name>".
    let tail = match (exe.parent().and_then(|p| p.file_name()), exe.file_name()) {
        (Some(dir), Some(file)) => format!(r"\{}\{}", dir.to_string_lossy(), file.to_string_lossy()).to_lowercase(),
        _ => full.clone(),
    };
    let root = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let Ok(settings) = root.open_subkey(r"Control Panel\NotifyIconSettings") else { return false };
    let mut found = false;
    for name in settings.enum_keys().flatten() {
        let Ok(k) = settings.open_subkey_with_flags(&name, KEY_READ | KEY_SET_VALUE) else { continue };
        let path: String = k.get_value("ExecutablePath").unwrap_or_default();
        let path = path.to_lowercase();
        if path == full || path.ends_with(&tail) {
            let promoted: u32 = k.get_value("IsPromoted").unwrap_or(0);
            if promoted != 1 {
                let _ = k.set_value("IsPromoted", &1u32);
            }
            found = true;
        }
    }
    found
}

#[cfg(not(windows))]
pub fn promote_icon() -> bool {
    true
}

pub fn tooltip_for(st: &BatteryState) -> String {
    let name = match st.device_name.as_deref() {
        Some(n) if crate::battery::is_arctis3(n) => "Arctis 3".to_string(),
        Some(n) => n.trim_start_matches("SteelSeries ").to_string(),
        None => "Arctis 3".to_string(),
    };
    match (st.percent, st.connection.as_str()) {
        (Some(p), _) => format!("SteelSeries {name} – Battery: {p}%"),
        (None, "notFound") => "SteelSeries Arctis – Headset not found".into(),
        (None, "disconnected") => format!("SteelSeries {name} – Disconnected"),
        _ if st.bluetooth == "off" => format!("SteelSeries {name} – Bluetooth is off"),
        _ => format!("SteelSeries {name} – Battery percentage unavailable"),
    }
}

pub fn update(app: &AppHandle, st: &BatteryState, pulse_phase: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon_for(st, pulse_phase)));
        let _ = tray.set_tooltip(Some(tooltip_for(st)));
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &refresh, &sep, &quit])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon_for(&BatteryState::initial(), false))
        .tooltip("SteelSeries Arctis 3 – Searching…")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "show" => crate::show_main(app),
            "refresh" => crate::request_refresh(app, true),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) {
                        let _ = w.hide();
                    } else {
                        crate::show_main(app);
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

