//! Details pane properties via the Windows Property System (same formatting as Explorer),
//! and the native Properties dialog.

use ipc_types::Property;
use windows::Win32::Foundation::{HWND, PROPERTYKEY};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::PropertiesSystem::{
    GPS_BESTEFFORT, GPS_DEFAULT, IPropertyStore, PDFF_DEFAULT, PSFormatForDisplayAlloc,
    PSGetPropertyKeyFromName, SHGetPropertyStoreFromParsingName,
};
use windows::Win32::UI::Shell::{SHOP_FILEPATH, SHObjectProperties};
use windows::core::{HSTRING, PCWSTR};

use crate::util::wide;

/// Canonical property names shown in the details pane, with their English labels.
const PROPS: &[(&str, &str)] = &[
    ("System.ItemTypeText", "Type"),
    ("System.Size", "Size"),
    ("System.DateModified", "Date modified"),
    ("System.DateCreated", "Date created"),
    ("System.DateAccessed", "Date accessed"),
    ("System.Image.Dimensions", "Dimensions"),
    ("System.Photo.DateTaken", "Date taken"),
    ("System.Photo.CameraModel", "Camera model"),
    ("System.Media.Duration", "Length"),
    ("System.Video.FrameWidth", "Frame width"),
    ("System.Video.FrameHeight", "Frame height"),
    ("System.Audio.EncodingBitrate", "Bit rate"),
    ("System.Music.Artist", "Contributing artists"),
    ("System.Music.AlbumTitle", "Album"),
    ("System.Title", "Title"),
    ("System.Author", "Authors"),
    ("System.Document.PageCount", "Pages"),
    ("System.Document.WordCount", "Word count"),
    ("System.Company", "Company"),
    ("System.FileVersion", "File version"),
    ("System.FileOwner", "Owner"),
    ("System.ItemFolderPathDisplay", "Location"),
];

pub fn properties(path: &str) -> Vec<Property> {
    let mut out = Vec::new();
    let w = HSTRING::from(path);
    let store: IPropertyStore =
        match unsafe { SHGetPropertyStoreFromParsingName(&w, None, GPS_DEFAULT | GPS_BESTEFFORT) } {
            Ok(s) => s,
            Err(_) => return out,
        };
    for (name, label) in PROPS {
        let wn = wide(name);
        let mut key = PROPERTYKEY::default();
        if unsafe { PSGetPropertyKeyFromName(PCWSTR(wn.as_ptr()), &mut key) }.is_err() {
            continue;
        }
        let Ok(value) = (unsafe { store.GetValue(&key) }) else { continue };
        if value.is_empty() {
            continue;
        }
        if let Ok(text) = unsafe { PSFormatForDisplayAlloc(&key, &value, PDFF_DEFAULT) } {
            let s = unsafe { text.to_string() }.unwrap_or_default();
            unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
            let s = s.trim_matches(|c: char| c == '\u{200e}' || c == '\u{200f}' || c.is_whitespace()).to_string();
            if !s.is_empty() {
                out.push(Property { label: label.to_string(), value: s });
            }
        }
    }
    out
}

/// Native Properties dialog (Alt+Enter).
pub fn show_properties_dialog(owner: HWND, path: &str) {
    let w = wide(path);
    unsafe {
        let _ = SHObjectProperties(Some(owner), SHOP_FILEPATH, PCWSTR(w.as_ptr()), PCWSTR::null());
    }
}
