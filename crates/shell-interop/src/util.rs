use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{ILFree, SHParseDisplayName};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::core::PCWSTR;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Window handle passed across threads as an integer.
pub fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

/// Owned absolute PIDL.
pub struct Pidl(pub *mut ITEMIDLIST);

impl Pidl {
    pub fn parse(path: &str) -> windows::core::Result<Self> {
        let w = wide(path);
        let mut p: *mut ITEMIDLIST = std::ptr::null_mut();
        unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut p, 0, None)? };
        Ok(Pidl(p))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ILFree(Some(self.0)) };
        }
    }
}
