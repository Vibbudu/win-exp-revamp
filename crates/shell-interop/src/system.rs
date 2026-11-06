//! System facts used by the UI theme and indexer, plus Windows 11 Dark/Light theme
//! detection and native Win32 dark context menu registration (`uxtheme.dll`).

use ipc_types::SystemInfo;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::IsUserAnAdmin;
use windows::core::{PCSTR, w};

/// DWM accent colour as `#rrggbb` (falls back to the Windows default blue).
pub fn accent_color() -> String {
    let mut value: u32 = 0;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\DWM"),
            w!("AccentColor"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    if ok.is_err() {
        return "#0067c0".into();
    }
    // Stored as 0xAABBGGRR.
    let r = value & 0xff;
    let g = (value >> 8) & 0xff;
    let b = (value >> 16) & 0xff;
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Returns `true` when Windows 11 is in Dark Mode (`AppsUseLightTheme == 0`),
/// or `false` when Windows 11 is in Light Mode (`AppsUseLightTheme == 1`).
pub fn is_dark_mode() -> bool {
    let mut value: u32 = 0;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    if ok.is_err() {
        return true; // Default to dark mode
    }
    value == 0
}

/// Applies Windows 11 immersive dark/light mode to the window and Win32 popup menus (`HMENU`)
/// via `uxtheme.dll` ordinals 133 (`AllowDarkModeForWindow`), 135 (`SetPreferredAppMode`),
/// and 136 (`FlushMenuThemes`).
pub fn apply_window_theme(hwnd: HWND, dark: bool) {
    unsafe {
        if !hwnd.0.is_null() {
            let use_dark: u32 = if dark { 1 } else { 0 };
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &use_dark as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
            );
        }

        if let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) {
            // Ordinal 135: SetPreferredAppMode(PreferredAppMode)
            // 0 = Default, 1 = AllowDark, 2 = ForceDark, 3 = ForceLight
            if let Some(proc) = GetProcAddress(uxtheme, PCSTR(135usize as *const u8)) {
                let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 =
                    std::mem::transmute(proc);
                let _ = set_preferred_app_mode(if dark { 2 } else { 3 });
            }

            // Ordinal 133: AllowDarkModeForWindow(HWND, bool) -> bool
            if !hwnd.0.is_null() {
                if let Some(proc) = GetProcAddress(uxtheme, PCSTR(133usize as *const u8)) {
                    let allow_dark_for_window: unsafe extern "system" fn(HWND, bool) -> bool =
                        std::mem::transmute(proc);
                    let _ = allow_dark_for_window(hwnd, dark);
                }
            }

            // Ordinal 136: FlushMenuThemes()
            if let Some(proc) = GetProcAddress(uxtheme, PCSTR(136usize as *const u8)) {
                let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(proc);
                flush_menu_themes();
            }
        }
    }
}

pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

pub fn system_info() -> SystemInfo {
    SystemInfo {
        accent: accent_color(),
        elevated: is_elevated(),
        user_name: std::env::var("USERNAME").unwrap_or_default(),
        home: std::env::var("USERPROFILE").unwrap_or_default(),
    }
}
