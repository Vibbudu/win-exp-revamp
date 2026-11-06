//! File operations through `IFileOperation`, so users get the native progress dialog,
//! conflict resolution, Recycle Bin semantics and shell undo records — identical to Explorer.

use anyhow::Result;
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::Shell::{
    FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_RENAMEONCOLLISION, FOFX_ADDUNDORECORD, FOFX_NOMINIMIZEBOX,
    FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
    SHOpenWithDialog, OPENASINFO, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT,
    FILEOPERATION_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

use crate::util::wide;

pub fn item(path: &str) -> windows::core::Result<IShellItem> {
    let w = wide(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
}

fn new_op(owner: HWND, flags: u32) -> Result<IFileOperation> {
    let op: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL)? };
    unsafe {
        op.SetOwnerWindow(owner)?;
        op.SetOperationFlags(FILEOPERATION_FLAGS(flags))?;
    }
    Ok(op)
}

const BASE: u32 = FOF_ALLOWUNDO.0 as u32 | FOFX_ADDUNDORECORD.0 as u32 | FOFX_NOMINIMIZEBOX.0 as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

/// Copy or move `sources` into `dest_dir`. Same-folder copies get "- Copy" names like Explorer.
pub fn transfer(owner: HWND, sources: &[String], dest_dir: &str, kind: Transfer) -> Result<bool> {
    let same_folder = sources.iter().all(|s| {
        std::path::Path::new(s).parent().map(|p| p.as_os_str().eq_ignore_ascii_case(dest_dir.trim_end_matches('\\'))).unwrap_or(false)
    });
    let mut flags = BASE;
    if same_folder && kind == Transfer::Copy {
        flags |= FOF_RENAMEONCOLLISION.0 as u32;
    }
    let op = new_op(owner, flags)?;
    let dest = item(dest_dir)?;
    unsafe {
        for s in sources {
            let it = item(s)?;
            match kind {
                Transfer::Copy => op.CopyItem(&it, &dest, PCWSTR::null(), None)?,
                Transfer::Move => op.MoveItem(&it, &dest, PCWSTR::null(), None)?,
            }
        }
        op.PerformOperations()?;
        Ok(!op.GetAnyOperationsAborted()?.as_bool())
    }
}

/// Delete to the Recycle Bin (or permanently when `permanent`, e.g. Shift+Delete).
pub fn delete(owner: HWND, paths: &[String], permanent: bool) -> Result<bool> {
    let mut flags = BASE;
    if permanent {
        flags &= !(FOF_ALLOWUNDO.0 as u32);
    } else {
        flags |= FOFX_RECYCLEONDELETE.0 as u32;
    }
    let op = new_op(owner, flags)?;
    unsafe {
        for p in paths {
            op.DeleteItem(&item(p)?, None)?;
        }
        op.PerformOperations()?;
        Ok(!op.GetAnyOperationsAborted()?.as_bool())
    }
}

pub fn rename(owner: HWND, path: &str, new_name: &str) -> Result<()> {
    let op = new_op(owner, BASE)?;
    let name = HSTRING::from(new_name);
    unsafe {
        op.RenameItem(&item(path)?, &name, None)?;
        op.PerformOperations()?;
    }
    Ok(())
}

/// Create a new folder (or empty file) with a unique name such as "New folder (2)".
/// Returns the created path.
pub fn create_new(owner: HWND, dir: &str, base_name: &str, is_dir: bool) -> Result<String> {
    let dir_trim = dir.trim_end_matches('\\');
    let (stem, ext) = match base_name.rfind('.') {
        Some(i) if !is_dir && i > 0 => (&base_name[..i], &base_name[i..]),
        _ => (base_name, ""),
    };
    let mut name = base_name.to_string();
    let mut n = 2;
    while std::path::Path::new(&format!("{dir_trim}\\{name}")).exists() {
        name = format!("{stem} ({n}){ext}");
        n += 1;
    }
    let op = new_op(owner, BASE | FOF_NOCONFIRMATION.0 as u32)?;
    let hname = HSTRING::from(name.as_str());
    let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY.0 } else { 0 };
    unsafe {
        op.NewItem(&item(dir)?, attrs, &hname, PCWSTR::null(), None)?;
        op.PerformOperations()?;
    }
    Ok(format!("{dir_trim}\\{name}"))
}

/// Open with the default handler (same as double-click in Explorer). `verb` e.g. "runas".
pub fn shell_open(owner: HWND, path: &str, verb: Option<&str>) -> Result<()> {
    let file = wide(path);
    let verb_w = verb.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        hwnd: owner,
        lpVerb: verb_w.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info)? };
    Ok(())
}

pub fn open_with_dialog(owner: HWND, path: &str) -> Result<()> {
    let file = wide(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
    };
    unsafe { SHOpenWithDialog(Some(owner), &info)? };
    Ok(())
}
