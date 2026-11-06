//! The classic shell context menu ("Show more options"), including third-party handlers
//! such as 7-Zip, VS Code, Git, WizTree, WinRAR, etc., styled with Windows 11 Dark/Light theme.
//! Must be called on the thread that owns `owner`.

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_CANRENAME, CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_PTINVOKE,
    CMINVOKECOMMANDINFOEX, DefSubclassProc, GCS_VERBW, IContextMenu, IContextMenu2, IContextMenu3,
    IShellFolder, RemoveWindowSubclass, SHBindToObject, SHBindToParent, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, InsertMenuW, MF_BYPOSITION, MF_SEPARATOR,
    MF_STRING, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_DRAWITEM,
    WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR, WM_MENUSELECT,
};
use windows::core::{Interface, PCSTR, PCWSTR, PSTR, w};

use crate::util::Pidl;

const FIRST_ID: u32 = 1;
const LAST_ID: u32 = 0x7EFF;
const ID_OPEN_NEW_TAB: u32 = 0x7F01;
const ID_OPEN_NEW_WINDOW: u32 = 0x7F02;
const SUBCLASS_ID: usize = 0x48595045; // "HYPE"
const CMIC_MASK_UNICODE: u32 = 0x00004000;

/// Result of showing the menu: a verb the UI should handle itself (e.g. "rename", "open_new_tab",
/// "open_new_window"), "invoked" when the shell handled it, or "" when dismissed.
pub fn show(
    owner: HWND,
    paths: &[String],
    background_of: Option<&str>,
    at: Option<(i32, i32)>,
    extended: bool,
) -> Result<String> {
    // Ensure Win32 menu theme matches the current Windows 11 Dark / Light mode
    let is_dark = crate::system::is_dark_mode();
    crate::system::apply_window_theme(owner, is_dark);

    let menu: IContextMenu = if let Some(dir) = background_of {
        background_menu(owner, dir)?
    } else {
        items_menu(owner, paths)?
    };
    let pt = match at {
        Some((x, y)) => POINT { x, y },
        None => {
            let mut p = POINT::default();
            unsafe { GetCursorPos(&mut p)? };
            p
        }
    };

    let is_single_dir = background_of.is_none()
        && paths.len() == 1
        && std::path::Path::new(&paths[0]).is_dir();

    unsafe {
        let hmenu = CreatePopupMenu()?;
        let mut flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME;
        if extended {
            flags |= CMF_EXTENDEDVERBS;
        }
        menu.QueryContextMenu(hmenu, 0, FIRST_ID, LAST_ID, flags).ok()?;

        // Inject Windows 11 "Open in new tab" and "Open in new window" at the top for folders
        if is_single_dir {
            let _ = InsertMenuW(
                hmenu,
                0,
                MF_BYPOSITION | MF_STRING,
                ID_OPEN_NEW_TAB as usize,
                w!("Open in new tab"),
            );
            let _ = InsertMenuW(
                hmenu,
                1,
                MF_BYPOSITION | MF_STRING,
                ID_OPEN_NEW_WINDOW as usize,
                w!("Open in new window"),
            );
            let _ = InsertMenuW(hmenu, 2, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
        }

        // Forward owner-draw/submenu messages so "Send to", "Open with", etc. populate.
        let cm3: Option<IContextMenu3> = menu.cast().ok();
        let cm2: Option<IContextMenu2> = menu.cast().ok();
        let raw = cm3.as_ref().map(|m| m.as_raw() as usize).unwrap_or(0);
        let raw2 = cm2.as_ref().map(|m| m.as_raw() as usize).unwrap_or(0);
        let ref_data = Box::into_raw(Box::new((raw, raw2))) as usize;
        let _ = SetWindowSubclass(owner, Some(subclass_proc), SUBCLASS_ID, ref_data);

        let cmd = TrackPopupMenuEx(
            hmenu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            pt.x,
            pt.y,
            owner,
            None,
        );
        let _ = RemoveWindowSubclass(owner, Some(subclass_proc), SUBCLASS_ID);
        drop(Box::from_raw(ref_data as *mut (usize, usize)));

        let id = cmd.0 as u32;
        let result = if id == ID_OPEN_NEW_TAB {
            "open_new_tab".to_string()
        } else if id == ID_OPEN_NEW_WINDOW {
            "open_new_window".to_string()
        } else if id >= FIRST_ID && id <= LAST_ID {
            let offset = (id - FIRST_ID) as usize;
            let verb = command_verb(&menu, offset);
            if matches!(verb.as_str(), "rename") {
                verb
            } else {
                let info = CMINVOKECOMMANDINFOEX {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                    fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                    hwnd: owner,
                    lpVerb: PCSTR(offset as *const u8),
                    lpVerbW: PCWSTR(offset as *const u16),
                    nShow: SW_SHOWNORMAL.0,
                    ptInvoke: pt,
                    ..Default::default()
                };
                menu.InvokeCommand(&info as *const _ as *const _)?;
                if verb.is_empty() { "invoked".into() } else { verb }
            }
        } else {
            String::new()
        };
        let _ = DestroyMenu(hmenu);
        Ok(result)
    }
}

fn command_verb(menu: &IContextMenu, offset: usize) -> String {
    let mut buf = [0u16; 256];
    unsafe {
        if menu
            .GetCommandString(
                offset,
                GCS_VERBW,
                None,
                PSTR(buf.as_mut_ptr() as *mut u8),
                buf.len() as u32,
            )
            .is_ok()
        {
            return crate::util::from_wide(&buf).to_lowercase();
        }
    }
    String::new()
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if matches!(
            msg,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR | WM_MENUSELECT
        ) && data != 0
        {
            let (raw3, raw2) = *(data as *const (usize, usize));
            if raw3 != 0 {
                let cm3 = IContextMenu3::from_raw_borrowed(&(raw3 as *mut _)).cloned();
                if let Some(cm3) = cm3 {
                    let mut res = LRESULT(0);
                    if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut res)).is_ok() {
                        return res;
                    }
                }
            } else if raw2 != 0 {
                let cm2 = IContextMenu2::from_raw_borrowed(&(raw2 as *mut _)).cloned();
                if let Some(cm2) = cm2 {
                    if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                        return LRESULT(0);
                    }
                }
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

fn items_menu(owner: HWND, paths: &[String]) -> Result<IContextMenu> {
    if paths.is_empty() {
        return Err(anyhow!("no items"));
    }
    let pidls: Vec<Pidl> = paths
        .iter()
        .map(|p| Pidl::parse(p))
        .collect::<windows::core::Result<_>>()?;
    unsafe {
        let folder: IShellFolder = SHBindToParent(pidls[0].0, None)?;
        let children: Vec<*const ITEMIDLIST> = pidls
            .iter()
            .map(|p| windows::Win32::UI::Shell::ILFindLastID(p.0) as *const _)
            .collect();
        let menu: IContextMenu = folder.GetUIObjectOf(owner, &children, None)?;
        Ok(menu)
    }
}

fn background_menu(owner: HWND, dir: &str) -> Result<IContextMenu> {
    let pidl = Pidl::parse(dir)?;
    unsafe {
        let folder: IShellFolder = SHBindToObject(None, pidl.0, None)?;
        let menu: IContextMenu = folder.CreateViewObject(owner)?;
        Ok(menu)
    }
}
