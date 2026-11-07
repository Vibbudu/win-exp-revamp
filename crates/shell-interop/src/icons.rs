//! System icons and thumbnails through `IShellItemImageFactory` — the exact images Explorer
//! shows, including overlays-free file-type icons, folder icons, drive icons and thumbnails.

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
};
use windows::core::PCWSTR;

use crate::util::wide;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageMode {
    /// File-type / folder icon only (fast, cacheable per extension).
    Icon,
    /// Thumbnail if the shell has one, otherwise the icon.
    Thumbnail,
    /// Thumbnail only; fails if none is available.
    ThumbnailOnly,
}

/// Render the shell image for `path` at `size`×`size` pixels and return `(width, height, rgba_bytes)`.
/// Must run on an STA thread.
pub fn shell_image_rgba(path: &str, size: u32, mode: ImageMode) -> Result<(u32, u32, Vec<u8>)> {
    let w = wide(path);
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None)? };
    let flags: SIIGBF = match mode {
        ImageMode::Icon => SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        ImageMode::Thumbnail => SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK,
        ImageMode::ThumbnailOnly => SIIGBF_THUMBNAILONLY | SIIGBF_RESIZETOFIT,
    };
    let sz = SIZE { cx: size as i32, cy: size as i32 };
    let hbmp = unsafe { factory.GetImage(sz, flags)? };
    let res = hbitmap_to_rgba(hbmp);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
    }
    res
}

/// Render the shell image for `path` (a filesystem path or a `::{CLSID}` parsing name)
/// at `size`×`size` pixels and return PNG bytes. Must run on an STA thread.
pub fn shell_image_png(path: &str, size: u32, mode: ImageMode) -> Result<Vec<u8>> {
    let (w, h, buf) = shell_image_rgba(path, size, mode)?;
    let mut out = Vec::with_capacity(buf.len() / 3);
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header()?;
        writer.write_image_data(&buf)?;
    }
    Ok(out)
}

fn hbitmap_to_rgba(hbmp: HBITMAP) -> Result<(u32, u32, Vec<u8>)> {
    let mut bm = BITMAP::default();
    let got = unsafe {
        GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _))
    };
    if got == 0 {
        return Err(anyhow!("GetObject failed"));
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return Err(anyhow!("GetDIBits failed"));
        }
    }
    // BGRA (premultiplied) → RGBA (straight). Some thumbnails carry no alpha at all.
    let has_alpha = buf.chunks_exact(4).any(|p| p[3] != 0);
    for p in buf.chunks_exact_mut(4) {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        if !has_alpha {
            p[0] = r;
            p[2] = b;
            p[3] = 255;
        } else if a == 0 {
            p.copy_from_slice(&[0, 0, 0, 0]);
        } else {
            let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            p[0] = un(r);
            p[1] = un(g);
            p[2] = un(b);
        }
    }
    Ok((w as u32, h as u32, buf))
}

/// Extensions whose icon differs per file, so they cannot be cached by extension.
pub fn icon_is_per_file(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "ico" | "url" | "msi" | "scr" | "cpl" | "appref-ms" | "cur" | "ani" | "dll" | "website")
}

/// Extensions that represent images/photos/videos/documents where per-item shell thumbnails
/// are beneficial in directory listings.
pub fn has_visual_thumbnail(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "mp4"
            | "mov"
            | "mkv"
            | "avi"
            | "webm"
            | "html"
            | "htm"
            | "exe"
            | "lnk"
            | "url"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudSyncState {
    None,
    Cloud,    // Online only: white cloud with blue outline
    Pinned,   // Available offline: solid circle with checkmark
    Syncing,  // Sync in progress: spinning sync arrows
    Error,    // Sync warning: warning icon
}

impl CloudSyncState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CloudSyncState::None => "none",
            CloudSyncState::Cloud => "cloud",
            CloudSyncState::Pinned => "pinned",
            CloudSyncState::Syncing => "syncing",
            CloudSyncState::Error => "error",
        }
    }
}

/// Detects specific cloud sync state (cloud-only, pinned/available offline, syncing, or error).
pub fn detect_cloud_sync_state(path: &str, attrs: u32, cloud_roots: &[String]) -> CloudSyncState {
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
    const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
    const FILE_ATTRIBUTE_UNPINNED: u32 = 0x0010_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

    let p_lower = path.to_ascii_lowercase();
    let is_in_cloud = p_lower.contains("\\onedrive")
        || p_lower.contains("\\my drive")
        || p_lower.contains("\\shared drives")
        || p_lower.contains("\\google drive")
        || p_lower.contains("\\icloud")
        || p_lower.contains("\\dropbox")
        || cloud_roots.iter().any(|r| path.len() >= r.len() && path[..r.len()].eq_ignore_ascii_case(r));

    if !is_in_cloud && (attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_PINNED)) == 0 {
        return CloudSyncState::None;
    }

    if (attrs & FILE_ATTRIBUTE_PINNED) != 0 {
        return CloudSyncState::Pinned;
    }
    if (attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_UNPINNED)) != 0 {
        return CloudSyncState::Cloud;
    }

    if p_lower.ends_with(".tmp") || p_lower.ends_with(".part") || p_lower.contains("~$") {
        return CloudSyncState::Syncing;
    }

    if is_in_cloud {
        if p_lower.ends_with(".gdoc") || p_lower.ends_with(".gsheet") || p_lower.ends_with(".gslides") {
            CloudSyncState::Cloud
        } else {
            CloudSyncState::Pinned
        }
    } else {
        CloudSyncState::None
    }
}

/// Detects whether a file/folder resides on a cloud-backed filesystem (Google Drive `DriveFS`,
/// OneDrive, iCloud, Dropbox) or carries cloud placeholder/reparse attributes so the UI can
/// render the Windows 11 cloud sync overlay badge on the bottom-left of its icon/thumbnail.
pub fn is_cloud_synced_item(path: &str, attrs: u32, cloud_roots: &[String]) -> bool {
    detect_cloud_sync_state(path, attrs, cloud_roots) != CloudSyncState::None
}
