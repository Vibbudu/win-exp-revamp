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

/// Render the shell image for `path` (a filesystem path or a `::{CLSID}` parsing name)
/// at `size`×`size` pixels and return PNG bytes. Must run on an STA thread.
pub fn shell_image_png(path: &str, size: u32, mode: ImageMode) -> Result<Vec<u8>> {
    let w = wide(path);
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None)? };
    let flags: SIIGBF = match mode {
        ImageMode::Icon => SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        ImageMode::Thumbnail => SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK,
        ImageMode::ThumbnailOnly => SIIGBF_THUMBNAILONLY | SIIGBF_RESIZETOFIT,
    };
    let sz = SIZE { cx: size as i32, cy: size as i32 };
    let hbmp = unsafe { factory.GetImage(sz, flags)? };
    let res = hbitmap_to_png(hbmp);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
    }
    res
}

fn hbitmap_to_png(hbmp: HBITMAP) -> Result<Vec<u8>> {
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
    let mut out = Vec::with_capacity(buf.len() / 3);
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header()?;
        writer.write_image_data(&buf)?;
    }
    Ok(out)
}

/// Extensions whose icon differs per file, so they cannot be cached by extension.
pub fn icon_is_per_file(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "ico" | "url" | "msi" | "scr" | "cpl" | "appref-ms" | "cur" | "ani" | "dll" | "website")
}
