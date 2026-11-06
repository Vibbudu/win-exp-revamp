use anyhow::{Result, bail};
use ipc_types::{ATTR_DIRECTORY, Entry};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, FILETIME, GetLastError};
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, GetFileAttributesExW, GetFileExInfoStandard, WIN32_FILE_ATTRIBUTE_DATA,
    WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

/// FILETIME (100ns since 1601) → unix epoch milliseconds.
#[inline]
pub fn filetime_to_unix_ms(ft: FILETIME) -> i64 {
    let v = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    if v == 0 {
        return 0;
    }
    (v as i64 - 116_444_736_000_000_000) / 10_000
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn name_from(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Enumerate one directory. `type_name` is left empty; the shell layer fills it in.
pub fn list_dir(dir: &str) -> Result<Vec<Entry>> {
    // "C:\" and "C:" both become "C:\"; "C:\foo\" becomes "C:\foo\".
    let mut base = dir.trim_end_matches(['\\', '/']).to_string();
    base.push('\\');
    let pattern = wide(&format!("{base}*"));
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() {
                return Ok(Vec::new());
            }
            bail!("cannot open {dir}: {}", e.message());
        }
    };
    let mut out = Vec::with_capacity(256);
    loop {
        let name = name_from(&data.cFileName);
        if name != "." && name != ".." {
            let is_dir = data.dwFileAttributes & ATTR_DIRECTORY != 0;
            let ext = if is_dir { String::new() } else { crate::extension_of(&name) };
            out.push(Entry {
                path: format!("{base}{name}"),
                is_dir,
                size: if is_dir { 0 } else { ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64 },
                modified: filetime_to_unix_ms(data.ftLastWriteTime),
                created: filetime_to_unix_ms(data.ftCreationTime),
                attrs: data.dwFileAttributes,
                type_name: String::new(),
                ext,
                name,
            });
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            let err = unsafe { GetLastError() };
            if err != ERROR_NO_MORE_FILES {
                // Partial listing is still useful; stop quietly.
            }
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    Ok(out)
}

/// Metadata for a single path (used to hydrate search results that only carry names).
pub fn stat_path(path: &str) -> Option<Entry> {
    let w = wide(path);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe {
        GetFileAttributesExW(PCWSTR(w.as_ptr()), GetFileExInfoStandard, &mut data as *mut _ as *mut _).ok()?;
    }
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
    let is_dir = data.dwFileAttributes & ATTR_DIRECTORY != 0;
    Some(Entry {
        path: path.to_string(),
        is_dir,
        size: if is_dir { 0 } else { ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64 },
        modified: filetime_to_unix_ms(data.ftLastWriteTime),
        created: filetime_to_unix_ms(data.ftCreationTime),
        attrs: data.dwFileAttributes,
        type_name: String::new(),
        ext: if is_dir { String::new() } else { crate::extension_of(&name) },
        name,
    })
}
