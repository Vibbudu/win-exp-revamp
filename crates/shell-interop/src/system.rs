//! System facts used by the UI theme and indexer.

use ipc_types::SystemInfo;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::IsUserAnAdmin;
use windows::core::w;

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
