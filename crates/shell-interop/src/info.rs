//! Shell metadata: type names, display names, drives, known folders, recent files.

use std::collections::HashMap;
use std::sync::OnceLock;

use ipc_types::{ATTR_DIRECTORY, DriveInfo, Entry, NamedPath, RecentItem};
use parking_lot::Mutex;
use windows::Win32::Foundation::HWND;
use windows::Win32::NetworkManagement::WNet::{
    NETRESOURCEW, RESOURCE_CONNECTED, RESOURCE_REMEMBERED, RESOURCETYPE_DISK, WNetCloseEnum,
    WNetConnectionDialog, WNetEnumResourceW, WNetOpenEnumW,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile, STGM_READ};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
    FOLDERID_Recent, FOLDERID_Videos, IShellLinkW, KF_FLAG_DEFAULT, SHFILEINFOW, SHGFI_DISPLAYNAME,
    SHGFI_FLAGS, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SHGetKnownFolderPath,
    ShellLink,
};
use windows::core::{GUID, Interface, PCWSTR};

use crate::util::{from_wide, wide};

fn type_cache() -> &'static Mutex<HashMap<String, String>> {
    static C: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shgfi(path: &str, attrs: FILE_FLAGS_AND_ATTRIBUTES, flags: SHGFI_FLAGS) -> Option<SHFILEINFOW> {
    let w = wide(path);
    let mut info = SHFILEINFOW::default();
    let r = unsafe {
        SHGetFileInfoW(PCWSTR(w.as_ptr()), attrs, Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32, flags)
    };
    (r != 0).then_some(info)
}

/// Localized type name for an extension ("" = no extension). Cached; must be on an STA thread
/// the first time each extension is seen.
pub fn type_name(ext: &str, is_dir: bool) -> String {
    let key = if is_dir { "\u{0}dir".to_string() } else { ext.to_string() };
    if let Some(v) = type_cache().lock().get(&key) {
        return v.clone();
    }
    let (probe, attrs) = if is_dir {
        ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY)
    } else if ext.is_empty() {
        ("file".to_string(), FILE_ATTRIBUTE_NORMAL)
    } else {
        (format!("file.{ext}"), FILE_ATTRIBUTE_NORMAL)
    };
    let name = shgfi(&probe, attrs, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES)
        .map(|i| from_wide(&i.szTypeName))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| if ext.is_empty() { "File".into() } else { format!("{} File", ext.to_uppercase()) });
    type_cache().lock().insert(key, name.clone());
    name
}

/// Returns a cached type name without touching COM (None when not cached yet).
pub fn cached_type_name(ext: &str, is_dir: bool) -> Option<String> {
    let key = if is_dir { "\u{0}dir".to_string() } else { ext.to_string() };
    type_cache().lock().get(&key).cloned()
}

/// Shell display name, e.g. "Local Disk (C:)" or localized "Downloads".
pub fn display_name(path: &str) -> Option<String> {
    shgfi(path, FILE_FLAGS_AND_ATTRIBUTES(0), SHGFI_DISPLAYNAME)
        .map(|i| from_wide(&i.szDisplayName))
        .filter(|s| !s.is_empty())
}

pub fn drives() -> Vec<DriveInfo> {
    let mut buf = [0u16; 512];
    let n = unsafe { GetLogicalDriveStringsW(Some(&mut buf)) } as usize;
    let mut out = Vec::new();
    for root in buf[..n.min(buf.len())].split(|&c| c == 0).filter(|s| !s.is_empty()) {
        let root_s = String::from_utf16_lossy(root);
        let mut rw: Vec<u16> = root.to_vec();
        rw.push(0);
        let kind = match unsafe { GetDriveTypeW(PCWSTR(rw.as_ptr())) } {
            2 => "removable",
            3 => "fixed",
            4 => "network",
            5 => "cdrom",
            6 => "ram",
            _ => "unknown",
        };
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        let _ = unsafe {
            GetVolumeInformationW(PCWSTR(rw.as_ptr()), Some(&mut label), None, None, None, Some(&mut fs))
        };
        let (mut free, mut total) = (0u64, 0u64);
        let _ = unsafe { GetDiskFreeSpaceExW(PCWSTR(rw.as_ptr()), Some(&mut free), Some(&mut total), None) };
        let shown = display_name(&root_s).unwrap_or_else(|| {
            let l = from_wide(&label);
            format!("{} ({})", if l.is_empty() { "Local Disk".into() } else { l }, root_s.trim_end_matches('\\'))
        });
        out.push(DriveInfo { path: root_s, label: shown, file_system: from_wide(&fs), total, free, kind: kind.into() });
    }
    out
}

fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Desktop, Downloads, Documents, Pictures, Music, Videos — in Explorer's order.
pub fn known_folders() -> Vec<NamedPath> {
    let ids: [(&GUID, &str); 6] = [
        (&FOLDERID_Desktop, "Desktop"),
        (&FOLDERID_Downloads, "Downloads"),
        (&FOLDERID_Documents, "Documents"),
        (&FOLDERID_Pictures, "Pictures"),
        (&FOLDERID_Music, "Music"),
        (&FOLDERID_Videos, "Videos"),
    ];
    ids.iter()
        .filter_map(|(id, fallback)| {
            let path = known_folder(id)?;
            let name = display_name(&path).unwrap_or_else(|| fallback.to_string());
            Some(NamedPath { name, path })
        })
        .collect()
}

/// Resolve a .lnk target path.
pub fn resolve_link(lnk: &str) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let w = wide(lnk);
        pf.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let s = from_wide(&buf);
        (!s.is_empty()).then_some(s)
    }
}

/// Recently used files from the user's Recent folder (what Home → Recent shows).
pub fn recent_items(limit: usize) -> Vec<RecentItem> {
    let Some(dir) = known_folder(&FOLDERID_Recent) else { return vec![] };
    let Ok(mut entries) = fs_core::list_dir(&dir) else { return vec![] };
    entries.retain(|e| !e.is_dir && e.ext == "lnk");
    entries.sort_unstable_by(|a, b| b.modified.cmp(&a.modified));
    let mut out = Vec::new();
    for e in entries {
        if out.len() >= limit {
            break;
        }
        let Some(target) = resolve_link(&e.path) else { continue };
        let Some(meta) = fs_core::stat_path(&target) else { continue };
        let location = std::path::Path::new(&target)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push(RecentItem { name: meta.name, path: target, is_dir: meta.is_dir, accessed: e.modified, location });
    }
    out
}

/// Enumerate connected and remembered SMB / NAS network shares (`\\server\share`) and mapped network drives.
pub fn network_resources() -> Vec<Entry> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. Include mapped network drives from logical drives
    for d in drives() {
        if d.kind == "network" || d.file_system.eq_ignore_ascii_case("drivefs") {
            seen.insert(d.path.to_lowercase());
            results.push(Entry {
                name: d.label.clone(),
                path: d.path,
                is_dir: true,
                size: d.total,
                modified: 0,
                created: 0,
                attrs: ATTR_DIRECTORY,
                type_name: format!("Network / Cloud Drive ({})", d.file_system),
                ext: String::new(),
            });
        }
    }

    // 2. Enumerate connected & remembered WNet SMB/NAS disk shares
    for scope in [RESOURCE_CONNECTED, RESOURCE_REMEMBERED] {
        unsafe {
            let mut handle = Default::default();
            if WNetOpenEnumW(scope, RESOURCETYPE_DISK, Default::default(), None, &mut handle).is_ok() {
                let mut buf = vec![0u8; 16384];
                let mut count = u32::MAX;
                let mut buf_size = buf.len() as u32;
                if WNetEnumResourceW(
                    handle,
                    &mut count,
                    buf.as_mut_ptr() as *mut _,
                    &mut buf_size,
                )
                .is_ok()
                {
                    let slice = std::slice::from_raw_parts(
                        buf.as_ptr() as *const NETRESOURCEW,
                        count as usize,
                    );
                    for nr in slice {
                        let remote = if !nr.lpRemoteName.is_null() {
                            nr.lpRemoteName.to_string().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let local = if !nr.lpLocalName.is_null() {
                            nr.lpLocalName.to_string().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        if !remote.is_empty() && seen.insert(remote.to_lowercase()) {
                            let display = if !local.is_empty() {
                                format!("{remote} ({local})")
                            } else {
                                remote.clone()
                            };
                            results.push(Entry {
                                name: display,
                                path: remote,
                                is_dir: true,
                                size: 0,
                                modified: 0,
                                created: 0,
                                attrs: ATTR_DIRECTORY,
                                type_name: "Network Share (SMB / NAS)".into(),
                                ext: String::new(),
                            });
                        }
                    }
                }
                let _ = WNetCloseEnum(handle);
            }
        }
    }

    results
}

/// Opens the native Windows "Map Network Drive" dialog (`WNetConnectionDialog`) to connect to NAS / SMB shares.
pub fn map_network_drive_dialog(hwnd: HWND) {
    unsafe {
        let _ = WNetConnectionDialog(hwnd, RESOURCETYPE_DISK.0);
    }
}
