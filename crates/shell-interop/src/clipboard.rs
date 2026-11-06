//! File clipboard compatible with Explorer: `CF_HDROP` + "Preferred DropEffect".

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};
use windows::core::w;

const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;
const DROPEFFECT_LINK: u32 = 4;

fn open(owner: HWND) -> Result<()> {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    Err(anyhow!("clipboard busy"))
}

unsafe fn alloc_copy(bytes: &[u8]) -> Result<HGLOBAL> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len())?;
        let p = GlobalLock(h) as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

/// Put files on the clipboard. `cut` marks them for move on paste (Explorer dims them).
pub fn set_files(owner: HWND, paths: &[String], cut: bool) -> Result<()> {
    let header = std::mem::size_of::<DROPFILES>();
    let mut wide: Vec<u16> = Vec::new();
    for p in paths {
        wide.extend(p.encode_utf16());
        wide.push(0);
    }
    wide.push(0);
    let mut bytes = vec![0u8; header + wide.len() * 2];
    let df = DROPFILES { pFiles: header as u32, fWide: true.into(), ..Default::default() };
    unsafe {
        std::ptr::copy_nonoverlapping(&df as *const _ as *const u8, bytes.as_mut_ptr(), header);
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, bytes.as_mut_ptr().add(header), wide.len() * 2);
    }
    let effect: u32 = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY | DROPEFFECT_LINK };
    open(owner)?;
    let res = (|| -> Result<()> {
        unsafe {
            EmptyClipboard()?;
            let h = alloc_copy(&bytes)?;
            SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(h.0)))?;
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            let h2 = alloc_copy(&effect.to_le_bytes())?;
            SetClipboardData(fmt, Some(HANDLE(h2.0)))?;
        }
        Ok(())
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    res
}

/// Files currently on the clipboard and whether they were cut.
pub fn get_files(owner: HWND) -> Result<(Vec<String>, bool)> {
    if unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32) }.is_err() {
        return Ok((vec![], false));
    }
    open(owner)?;
    let res = (|| -> Result<(Vec<String>, bool)> {
        unsafe {
            let h = GetClipboardData(CF_HDROP.0 as u32)?;
            let hdrop = HDROP(h.0);
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                out.push(String::from_utf16_lossy(&buf[..len]));
            }
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            let mut cut = false;
            if let Ok(h2) = GetClipboardData(fmt) {
                let p = GlobalLock(HGLOBAL(h2.0)) as *const u32;
                if !p.is_null() {
                    cut = (*p & DROPEFFECT_MOVE) != 0;
                    let _ = GlobalUnlock(HGLOBAL(h2.0));
                }
            }
            Ok((out, cut))
        }
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    res
}

/// Copy plain text (used by "Copy as path").
pub fn set_text(owner: HWND, text: &str) -> Result<()> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
    open(owner)?;
    let res = (|| -> Result<()> {
        unsafe {
            EmptyClipboard()?;
            let h = alloc_copy(&bytes)?;
            SetClipboardData(13 /* CF_UNICODETEXT */, Some(HANDLE(h.0)))?;
        }
        Ok(())
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    res
}
