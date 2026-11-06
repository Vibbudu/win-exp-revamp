use windows::Win32::Foundation::ERROR_NO_MORE_FILES;
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

/// One raw directory record handed to [`enumerate_raw`] callbacks.
pub struct RawEntry<'a> {
    /// UTF-16 file name (no terminator).
    pub name: &'a [u16],
    pub attrs: u32,
    pub size: u64,
    pub modified: i64,
}

/// Allocation-free enumeration for the indexer: calls `f` for each child of `dir`
/// (`dir` must already end with a backslash). Returns false if the folder could not be opened.
pub fn enumerate_raw(dir_with_slash: &[u16], mut f: impl FnMut(RawEntry<'_>)) -> bool {
    let mut pattern: Vec<u16> = Vec::with_capacity(dir_with_slash.len() + 2);
    pattern.extend_from_slice(dir_with_slash);
    pattern.push('*' as u16);
    pattern.push(0);
    let mut data = WIN32_FIND_DATAW::default();
    let Ok(handle) = (unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    }) else {
        return false;
    };
    loop {
        let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(260);
        let name = &data.cFileName[..len];
        let dot = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
        if !dot {
            f(RawEntry {
                name,
                attrs: data.dwFileAttributes,
                size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
                modified: crate::filetime_to_unix_ms(data.ftLastWriteTime),
            });
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            let _ = ERROR_NO_MORE_FILES;
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    true
}
