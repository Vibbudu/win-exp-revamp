#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use indexer::IndexerEngine;
use ipc_types::{ATTR_HIDDEN, ATTR_READONLY, ATTR_REPARSE_POINT, ATTR_SYSTEM, Entry, Preview};
use lru::LruCache;
use parking_lot::Mutex;
use shell_interop::icons::ImageMode;
use slint::{
    Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString,
    Timer, TimerMode, VecModel,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, HTCAPTION, IsZoomed, PostMessageW, SW_MAXIMIZE,
    SW_MINIMIZE, SW_RESTORE, SendMessageW, ShowWindow, WM_CLOSE, WM_LBUTTONUP, WM_NCLBUTTONDOWN,
};

slint::include_modules!();

type ThumbCache = Arc<Mutex<LruCache<String, (u32, u32, Vec<u8>)>>>;

struct TabState {
    id: i32,
    title: String,
    path: PathBuf,
    is_this_pc: bool,
    is_network: bool,
    history_back: Vec<PathBuf>,
    history_forward: Vec<PathBuf>,
}

struct ExplorerState {
    current_path: PathBuf,
    is_network_view: bool,
    is_this_pc_view: bool,
    history_back: Vec<PathBuf>,
    history_forward: Vec<PathBuf>,
    tabs: Vec<TabState>,
    active_tab_idx: usize,
    next_tab_id: i32,
    entries: Vec<Entry>,
    sort_column: String,
    sort_asc: bool,
    view_mode: String,
    show_preview: bool,
    show_details: bool,
    show_hidden: bool,
    show_extensions: bool,
    compact_view: bool,
    selected_idx: Option<usize>,
    cloud_roots: Vec<String>,
}

fn get_active_hwnd() -> HWND {
    unsafe { GetForegroundWindow() }
}

fn get_cursor_point() -> POINT {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    pt
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 KB".to_string();
    }
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let b = bytes as f64;
    if b < KB {
        "1 KB".to_string()
    } else if b < MB {
        format!("{:.0} KB", (b / KB).ceil())
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else if b < TB {
        format!("{:.2} GB", b / GB)
    } else {
        format!("{:.2} TB", b / TB)
    }
}

/// Converts Unix epoch milliseconds to `DD-MM-YYYY HH:MM` using Hinnant's civil calendar algorithm
/// with saturating arithmetic so invalid or pre-1970 cloud timestamps never overflow or panic.
fn format_mtime(millis: i64) -> String {
    if millis <= 0 {
        return String::new();
    }
    let secs = millis / 1000;
    let days = secs.div_euclid(86400);
    let rem_secs = secs.rem_euclid(86400);
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097).max(0) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe.saturating_sub(365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy.saturating_sub((153 * mp + 2) / 5) + 1;
    let m = if mp < 10 { mp + 3 } else { mp.saturating_sub(9) };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{d:02}-{m:02}-{y:04} {hours:02}:{minutes:02}")
}

fn format_attrs(attrs: u32) -> String {
    let mut parts = Vec::new();
    if attrs & ATTR_READONLY != 0 {
        parts.push("Read-only");
    }
    if attrs & ATTR_HIDDEN != 0 {
        parts.push("Hidden");
    }
    if attrs & ATTR_SYSTEM != 0 {
        parts.push("System");
    }
    if attrs & ATTR_REPARSE_POINT != 0 {
        parts.push("Cloud / Reparse Point");
    }
    if parts.is_empty() {
        "Normal".to_string()
    } else {
        parts.join(", ")
    }
}

/// Resolves a crisp Segoe Fluent Icons codepoint, tint color, and Windows 11 Type description.
fn resolve_icon_and_type(entry: &Entry) -> (&'static str, Color, String) {
    if entry.is_dir {
        let label = if !entry.type_name.is_empty() {
            entry.type_name.clone()
        } else {
            "File folder".to_string()
        };
        return ("\u{E8B7}", Color::from_rgb_u8(255, 206, 61), label);
    }

    let ext = entry.ext.to_lowercase();
    match ext.as_str() {
        "xlsx" | "xls" | "xlsb" | "csv" | "ods" => (
            "\u{F0E3}",
            Color::from_rgb_u8(33, 163, 102),
            "Microsoft Excel Worksheet".to_string(),
        ),
        "gsheet" => (
            "\u{F0E3}",
            Color::from_rgb_u8(52, 168, 83),
            "Google Sheets Spreadsheet".to_string(),
        ),
        "docx" | "doc" | "rtf" => (
            "\u{E8A5}",
            Color::from_rgb_u8(65, 132, 243),
            "Microsoft Word Document".to_string(),
        ),
        "gdoc" => (
            "\u{E8A5}",
            Color::from_rgb_u8(66, 133, 244),
            "Google Docs Document".to_string(),
        ),
        "pptx" | "ppt" => (
            "\u{E8A5}",
            Color::from_rgb_u8(235, 108, 71),
            "Microsoft PowerPoint Presentation".to_string(),
        ),
        "gslides" => (
            "\u{E8A5}",
            Color::from_rgb_u8(251, 188, 4),
            "Google Slides Presentation".to_string(),
        ),
        "gform" => (
            "\u{E8A5}",
            Color::from_rgb_u8(161, 66, 244),
            "Google Forms Survey".to_string(),
        ),
        "pdf" => (
            "\u{EA90}",
            Color::from_rgb_u8(244, 67, 54),
            "PDF Document".to_string(),
        ),
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "svg" | "ico" => (
            "\u{EB9F}",
            Color::from_rgb_u8(178, 136, 255),
            format!("{} File", ext.to_uppercase()),
        ),
        "html" | "htm" => (
            "\u{E774}",
            Color::from_rgb_u8(76, 194, 255),
            "HTML Document".to_string(),
        ),
        "zip" | "7z" | "rar" | "tar" | "gz" => (
            "\u{F012}",
            Color::from_rgb_u8(245, 186, 24),
            "Compressed (zipped) Folder".to_string(),
        ),
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" => (
            "\u{E8B2}",
            Color::from_rgb_u8(0, 207, 232),
            format!("{} Video", ext.to_uppercase()),
        ),
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => (
            "\u{E8D6}",
            Color::from_rgb_u8(255, 107, 139),
            format!("{} Audio", ext.to_uppercase()),
        ),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "c" | "cpp" | "h" | "json" | "toml"
        | "yaml" | "yml" | "xml" | "css" | "slint" => (
            "\u{E943}",
            Color::from_rgb_u8(96, 205, 255),
            format!("{} Source File", ext.to_uppercase()),
        ),
        "exe" | "msi" | "bat" | "cmd" | "ps1" => (
            "\u{E756}",
            Color::from_rgb_u8(120, 210, 255),
            "Application".to_string(),
        ),
        "txt" | "md" | "log" | "ini" => (
            "\u{E8A5}",
            Color::from_rgb_u8(210, 210, 210),
            "Text Document".to_string(),
        ),
        _ => (
            "\u{E7C3}",
            Color::from_rgb_u8(180, 180, 180),
            if ext.is_empty() {
                "File".to_string()
            } else {
                format!("{} File", ext.to_uppercase())
            },
        ),
    }
}

fn display_name_for_entry(e: &Entry, show_extensions: bool) -> String {
    if e.is_dir || show_extensions || e.ext.is_empty() {
        return e.name.clone();
    }
    let suffix = format!(".{}", e.ext);
    if e.name.len() > suffix.len() && e.name.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase()) {
        e.name[..e.name.len() - suffix.len()].to_string()
    } else {
        e.name.clone()
    }
}

fn columns_for_view_mode(view_mode: &str) -> usize {
    match view_mode {
        "content" => 1,
        "tiles" => 3,
        "list" | "small_icons" => 4,
        "xl_icons" => 5,
        "large_icons" => 6,
        "icons" => 7,
        _ => 4,
    }
}

fn build_grid_rows(items: &[EntryItem], view_mode: &str) -> Vec<EntryGridRow> {
    let cols = columns_for_view_mode(view_mode).max(1);
    items
        .chunks(cols)
        .map(|chunk| EntryGridRow {
            items: ModelRc::new(Rc::new(VecModel::from(chunk.to_vec()))),
        })
        .collect()
}

fn build_crumbs(path: &Path, is_network: bool, is_this_pc: bool) -> Vec<CrumbItem> {
    if is_this_pc {
        return vec![CrumbItem {
            label: "This PC".into(),
            path: "this_pc".into(),
        }];
    }
    if is_network {
        return vec![CrumbItem {
            label: "Network".into(),
            path: "network".into(),
        }];
    }

    let mut crumbs = Vec::new();
    let mut current = PathBuf::new();

    for comp in path.components() {
        match comp {
            std::path::Component::Prefix(prefix) => {
                let p = prefix.as_os_str().to_string_lossy().to_string();
                let full = if p.starts_with("\\\\") {
                    p.clone()
                } else {
                    format!("{}\\", p)
                };
                let label = if p.starts_with("\\\\") {
                    p.trim_start_matches('\\').to_string()
                } else {
                    format!("Local Disk ({p})")
                };
                crumbs.push(CrumbItem {
                    label: label.into(),
                    path: full.clone().into(),
                });
                current = PathBuf::from(full);
            }
            std::path::Component::Normal(n) => {
                let s = n.to_string_lossy().to_string();
                current.push(&s);
                crumbs.push(CrumbItem {
                    label: s.into(),
                    path: current.to_string_lossy().to_string().into(),
                });
            }
            _ => {}
        }
    }
    crumbs
}

fn detect_nav_key(path: &Path, is_network: bool, is_this_pc: bool) -> &'static str {
    if is_this_pc {
        return "this_pc";
    }
    if is_network {
        return "network";
    }
    let s = path.to_string_lossy().to_lowercase();
    if s.starts_with("\\\\") {
        "network"
    } else if s.ends_with("\\desktop") {
        "desktop"
    } else if s.ends_with("\\downloads") {
        "downloads"
    } else if s.ends_with("\\documents") {
        "documents"
    } else if s.ends_with("\\pictures") {
        "pictures"
    } else if s.ends_with("\\music") {
        "music"
    } else if s.ends_with("\\videos") {
        "videos"
    } else if let Some(up) = std::env::var_os("USERPROFILE") {
        if Path::new(&up) == path {
            "home"
        } else {
            ""
        }
    } else {
        ""
    }
}

fn resolve_special_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    match trimmed.to_lowercase().as_str() {
        "home" => {
            if let Some(user_profile) = std::env::var_os("USERPROFILE") {
                PathBuf::from(user_profile)
            } else {
                PathBuf::from("C:\\")
            }
        }
        "desktop" => resolve_known_folder("Desktop"),
        "downloads" => resolve_known_folder("Downloads"),
        "documents" => resolve_known_folder("Documents"),
        "pictures" | "gallery" => resolve_known_folder("Pictures"),
        "music" => resolve_known_folder("Music"),
        "videos" => resolve_known_folder("Videos"),
        _ => PathBuf::from(trimmed),
    }
}

fn resolve_known_folder(name: &str) -> PathBuf {
    let known = shell_interop::info::known_folders();
    for kf in known {
        if kf.name.eq_ignore_ascii_case(name) {
            return PathBuf::from(kf.path);
        }
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        PathBuf::from(user_profile).join(name)
    } else {
        PathBuf::from("C:\\")
    }
}

/// Converts a slice of `Entry` into `Vec<EntryItem>` using the bounded `ThumbCache`
/// and schedules asynchronous thumbnail extraction on `StaPool` for any uncached visual items.
fn map_entries_and_schedule_thumbs(
    entries: &[Entry],
    show_extensions: bool,
    cloud_roots: &[String],
    view_mode: &str,
    thumb_cache: &ThumbCache,
    sta_pool: &Arc<shell_interop::StaPool>,
    ui_weak: slint::Weak<MainWindow>,
    dir_gen: u64,
    active_dir_gen: &Arc<AtomicU64>,
) -> Vec<EntryItem> {
    let mut uncached_requests: Vec<(usize, String, String)> = Vec::new();
    let target_px: u32 = match view_mode {
        "xl_icons" | "large_icons" | "icons" | "tiles" | "content" => 96,
        _ => 32,
    };

    let items: Vec<EntryItem> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (icon, color, type_name) = resolve_icon_and_type(e);
            let is_cloud = shell_interop::icons::is_cloud_synced_item(&e.path, e.attrs, cloud_roots);
            let display_name = display_name_for_entry(e, show_extensions);
            let ext_lower = e.ext.to_ascii_lowercase();

            let mut has_thumb = false;
            let mut thumb_img = Image::default();

            if !e.is_dir && shell_interop::icons::has_visual_thumbnail(&ext_lower) {
                let cache_key = format!("{}@{target_px}", e.path);
                if let Some((w, h, rgba)) = thumb_cache.lock().get(&cache_key).cloned() {
                    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h);
                    thumb_img = Image::from_rgba8(buf);
                    has_thumb = true;
                } else if uncached_requests.len() < 80 {
                    uncached_requests.push((i, e.path.clone(), ext_lower));
                }
            }

            EntryItem {
                id: format!("{i}").into(),
                name: display_name.into(),
                path: e.path.clone().into(),
                is_dir: e.is_dir,
                size_str: if e.is_dir && e.size == 0 {
                    "".into()
                } else {
                    format_bytes(e.size).into()
                },
                mtime_str: format_mtime(e.modified).into(),
                type_str: type_name.into(),
                icon_glyph: icon.into(),
                icon_color: color,
                has_thumb,
                thumb: thumb_img,
                is_cloud,
                orig_index: i as i32,
            }
        })
        .collect();

    if !uncached_requests.is_empty() {
        let cache = Arc::clone(thumb_cache);
        let gen_arc = Arc::clone(active_dir_gen);
        let view_mode_owned = view_mode.to_string();

        sta_pool.spawn(move || {
            for (row_idx, path, ext) in uncached_requests {
                if gen_arc.load(Ordering::Relaxed) != dir_gen {
                    break;
                }
                let cache_key = format!("{path}@{target_px}");
                let is_img_or_vid = matches!(
                    ext.as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "mp4" | "mov" | "mkv" | "avi" | "webm"
                );
                let mode = if is_img_or_vid {
                    ImageMode::Thumbnail
                } else {
                    ImageMode::Icon
                };
                if let Ok((w, h, rgba)) = shell_interop::icons::shell_image_rgba(&path, target_px, mode) {
                    cache.lock().put(cache_key, (w, h, rgba.clone()));
                    let ui_weak_inner = ui_weak.clone();
                    let gen_check = Arc::clone(&gen_arc);
                    let vm_clone = view_mode_owned.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if gen_check.load(Ordering::Relaxed) != dir_gen {
                            return;
                        }
                        if let Some(ui) = ui_weak_inner.upgrade() {
                            let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h);
                            let img = Image::from_rgba8(buf);

                            let model = ui.get_entries();
                            if let Some(mut item) = model.row_data(row_idx) {
                                item.has_thumb = true;
                                item.thumb = img;
                                model.set_row_data(row_idx, item);
                            }

                            if ui.get_view_mode().as_str() != "details" {
                                let all_items: Vec<EntryItem> = (0..model.row_count())
                                    .filter_map(|idx| model.row_data(idx))
                                    .collect();
                                let grid = build_grid_rows(&all_items, &vm_clone);
                                ui.set_grid_rows(ModelRc::new(Rc::new(VecModel::from(grid))));
                            }
                        }
                    });
                }
            }
        });
    }

    items
}

/// Selects item `idx` and updates the Details and Preview panes WITHOUT holding any RefCell borrow
/// across filesystem, preview, or COM calls.
/// For `.pdf` files, renders the actual visual first page via `Windows.Data.Pdf.PdfDocument`.
fn select_entry_safe(
    ui: &MainWindow,
    state: &Rc<RefCell<ExplorerState>>,
    sta_pool: &Arc<shell_interop::StaPool>,
    idx: usize,
) {
    let (entry, show_details, show_preview) = {
        let mut st = state.borrow_mut();
        if idx >= st.entries.len() {
            return;
        }
        st.selected_idx = Some(idx);
        (st.entries[idx].clone(), st.show_details, st.show_preview)
    };

    ui.set_selected_index(idx as i32);
    ui.set_selected_path(entry.path.clone().into());

    let size_str = if entry.is_dir {
        "".to_string()
    } else {
        format!("  {}", format_bytes(entry.size))
    };
    ui.set_status_selection_info(format!("1 item selected{size_str}").into());

    if !show_details && !show_preview {
        return;
    }

    let (_, _, type_desc) = resolve_icon_and_type(&entry);
    ui.set_preview_title(entry.name.clone().into());
    ui.set_preview_subtitle(type_desc.clone().into());
    ui.set_has_preview_image(false);

    // 1. Immediate structured properties for the Details pane
    if show_details {
        let mut base_props = vec![
            PropertyItem {
                name: "Name".into(),
                value: entry.name.clone().into(),
            },
            PropertyItem {
                name: "Type".into(),
                value: type_desc.clone().into(),
            },
            PropertyItem {
                name: "Folder path".into(),
                value: entry.path.clone().into(),
            },
        ];
        if !entry.is_dir {
            base_props.push(PropertyItem {
                name: "Size".into(),
                value: format!("{} ({} bytes)", format_bytes(entry.size), entry.size).into(),
            });
        }
        let mtime = format_mtime(entry.modified);
        if !mtime.is_empty() {
            base_props.push(PropertyItem {
                name: "Date modified".into(),
                value: mtime.into(),
            });
        }
        let ctime = format_mtime(entry.created);
        if !ctime.is_empty() {
            base_props.push(PropertyItem {
                name: "Date created".into(),
                value: ctime.into(),
            });
        }
        base_props.push(PropertyItem {
            name: "Attributes".into(),
            value: format_attrs(entry.attrs).into(),
        });

        ui.set_item_properties(ModelRc::new(Rc::new(VecModel::from(base_props.clone()))));

        let path_for_props = entry.path.clone();
        let ui_weak = ui.as_weak();
        sta_pool.spawn(move || {
            let extra = shell_interop::props::properties(&path_for_props);
            if !extra.is_empty() {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        if ui.get_selected_path().as_str() != path_for_props {
                            return;
                        }
                        let mut merged = base_props;
                        for pr in extra {
                            if !merged.iter().any(|m| m.name.as_str() == pr.label) {
                                merged.push(PropertyItem {
                                    name: pr.label.into(),
                                    value: pr.value.into(),
                                });
                            }
                        }
                        ui.set_item_properties(ModelRc::new(Rc::new(VecModel::from(merged))));
                    }
                });
            }
        });
    }

    // 2. High-resolution visual preview (WinRT PdfDocument for PDFs; ThumbnailOnly for images/videos)
    if !entry.is_dir {
        let path_for_img = entry.path.clone();
        let ext_lower = entry.ext.to_lowercase();
        let ui_weak = ui.as_weak();

        sta_pool.spawn(move || {
            let rendered = if ext_lower == "pdf" {
                // Render the real PDF page via Windows.Data.Pdf.PdfDocument (never falls back to app icon!)
                preview::pdf::render_pdf_first_page_rgba(&path_for_img, 520).or_else(|_| {
                    shell_interop::icons::shell_image_rgba(&path_for_img, 420, ImageMode::ThumbnailOnly)
                })
            } else if matches!(
                ext_lower.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "ico" | "mp4" | "mkv" | "mov" | "avi" | "webm"
            ) {
                shell_interop::icons::shell_image_rgba(&path_for_img, 420, ImageMode::Thumbnail)
            } else {
                shell_interop::icons::shell_image_rgba(&path_for_img, 420, ImageMode::ThumbnailOnly)
            };

            if let Ok((w, h, rgba)) = rendered {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        if ui.get_selected_path().as_str() != path_for_img {
                            return;
                        }
                        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h);
                        ui.set_preview_image(Image::from_rgba8(buffer));
                        ui.set_has_preview_image(true);
                    }
                });
            }
        });
    }

    // 3. Universal File Content Preview (PDF text, Excel grid, Word, PPTX, Google Docs, Code, Archives)
    if show_preview {
        if entry.is_dir {
            let sub_count = fs_core::list_dir(&entry.path).map(|v| v.len()).unwrap_or(0);
            ui.set_preview_type("text".into());
            ui.set_preview_text_content(
                format!(
                    "Folder: {}\nLocation: {}\nContains: {} items\nModified: {}",
                    entry.name,
                    entry.path,
                    sub_count,
                    format_mtime(entry.modified)
                )
                .into(),
            );
            return;
        }

        let prev = preview::generate_preview(&entry.path);
        match prev {
            Preview::Sheet { rows, .. } => {
                ui.set_preview_type("table".into());
                let mut row_models = Vec::new();
                let mut header_model = Vec::new();
                for (r_idx, r) in rows.iter().enumerate() {
                    if r_idx == 0 {
                        header_model = r.clone();
                    } else {
                        let cell_strings: Vec<SharedString> =
                            r.iter().map(|c| c.clone().into()).collect();
                        row_models.push(PreviewRow {
                            cells: ModelRc::new(Rc::new(VecModel::from(cell_strings))),
                        });
                    }
                }
                let headers: Vec<SharedString> =
                    header_model.into_iter().map(|h| h.into()).collect();
                ui.set_preview_table_headers(ModelRc::new(Rc::new(VecModel::from(headers))));
                ui.set_preview_table_rows(ModelRc::new(Rc::new(VecModel::from(row_models))));
            }
            Preview::Text { content, .. } => {
                ui.set_preview_type("text".into());
                ui.set_preview_text_content(content.into());
            }
            Preview::Image => {
                ui.set_preview_type("image".into());
                ui.set_preview_text_content(
                    format!(
                        "{}\nSize: {}\nModified: {}",
                        type_desc,
                        format_bytes(entry.size),
                        format_mtime(entry.modified)
                    )
                    .into(),
                );
            }
            Preview::Html { html } => {
                let plain = html
                    .replace("<p>", "")
                    .replace("</p>", "\n\n")
                    .replace("<br>", "\n")
                    .replace("<h1>", "# ")
                    .replace("</h1>", "\n\n")
                    .replace("<h2>", "## ")
                    .replace("</h2>", "\n\n")
                    .replace("<strong>", "")
                    .replace("</strong>", "")
                    .replace("<em>", "")
                    .replace("</em>", "");
                ui.set_preview_type("text".into());
                ui.set_preview_text_content(plain.into());
            }
            Preview::Slides { slides } => {
                let mut text = String::new();
                for (slide_idx, s) in slides.iter().enumerate() {
                    text.push_str(&format!("─── Slide {} ───\n", slide_idx + 1));
                    for line in s {
                        text.push_str(line);
                        text.push('\n');
                    }
                    text.push('\n');
                }
                ui.set_preview_type("text".into());
                ui.set_preview_text_content(text.into());
            }
            Preview::Archive { entries, total } => {
                let mut text = format!("Archive contains {total} items:\n\n");
                for arch in entries {
                    text.push_str(&format!(
                        "{}  {}\n",
                        if arch.is_dir { "[DIR]" } else { "[FILE]" },
                        arch.name
                    ));
                }
                ui.set_preview_type("text".into());
                ui.set_preview_text_content(text.into());
            }
            _ => {
                ui.set_preview_type("text".into());
                ui.set_preview_text_content(
                    format!(
                        "File: {}\nType: {}\nSize: {}\nModified: {}\nPath: {}",
                        entry.name,
                        type_desc,
                        format_bytes(entry.size),
                        format_mtime(entry.modified),
                        entry.path
                    )
                    .into(),
                );
            }
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    // 1. Initialize STA worker pool & bounded 256-entry LruCache (<15 MB RAM)
    let sta_pool = Arc::new(shell_interop::StaPool::new("sta-pool", 3));
    let thumb_cache: ThumbCache = Arc::new(Mutex::new(LruCache::new(
        NonZeroUsize::new(256).unwrap(),
    )));
    let active_dir_gen = Arc::new(AtomicU64::new(1));
    let search_gen = Arc::new(AtomicU64::new(0));

    let main_window = MainWindow::new()?;

    // 2. Detect Windows 11 OS Dark / Light Mode and apply to UI + Win32 menus
    let initial_dark = shell_interop::system::is_dark_mode();
    main_window.set_is_dark_mode(initial_dark);
    shell_interop::system::apply_window_theme(get_active_hwnd(), initial_dark);

    // 3. Discover local & cloud drives (e.g., Google Drive `DriveFS`)
    let drives = shell_interop::info::drives();
    let mut cloud_roots = Vec::new();
    let drive_items: Vec<DriveItem> = drives
        .iter()
        .map(|d| {
            let total = d.total;
            let free = d.free;
            let used = total.saturating_sub(free);
            let percent = if total > 0 {
                (used as f32) / (total as f32)
            } else {
                0.0
            };
            let is_cloud = d.file_system.eq_ignore_ascii_case("DriveFS")
                || d.label.to_ascii_lowercase().contains("google drive")
                || d.label.to_ascii_lowercase().contains("onedrive")
                || d.kind == "remote";
            if is_cloud {
                cloud_roots.push(d.path.clone());
            }
            DriveItem {
                letter: d.path.clone().into(),
                name: if d.label.is_empty() {
                    format!("Local Disk ({})", d.path.trim_end_matches('\\')).into()
                } else {
                    format!("{} ({})", d.label, d.path.trim_end_matches('\\')).into()
                },
                free_str: format_bytes(free).into(),
                total_str: format_bytes(total).into(),
                percent_used: percent,
                is_cloud,
            }
        })
        .collect();
    main_window.set_drives(ModelRc::new(Rc::new(VecModel::from(drive_items))));

    // 4. Set initial path (Documents or User profile)
    let initial_path = resolve_known_folder("Documents");
    let initial_title = initial_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "Documents".to_string());

    let state = Rc::new(RefCell::new(ExplorerState {
        current_path: initial_path.clone(),
        is_network_view: false,
        is_this_pc_view: false,
        history_back: Vec::new(),
        history_forward: Vec::new(),
        tabs: vec![TabState {
            id: 0,
            title: initial_title,
            path: initial_path.clone(),
            is_this_pc: false,
            is_network: false,
            history_back: Vec::new(),
            history_forward: Vec::new(),
        }],
        active_tab_idx: 0,
        next_tab_id: 1,
        entries: Vec::new(),
        sort_column: "name".to_string(),
        sort_asc: true,
        view_mode: "details".to_string(),
        show_preview: false,
        show_details: false,
        show_hidden: true,
        show_extensions: true,
        compact_view: true,
        selected_idx: None,
        cloud_roots,
    }));

    // 5. Start Priority + Background Indexer (indexes Desktop/Documents/Downloads immediately)
    let indexer = Arc::new(IndexerEngine::new());
    indexer.start_background_indexing();

    // Helper to refresh UI state for the current folder
    let update_ui = {
        let main_window_weak = main_window.as_weak();
        let state = Rc::clone(&state);
        let thumb_cache = Arc::clone(&thumb_cache);
        let sta_pool = Arc::clone(&sta_pool);
        let active_dir_gen = Arc::clone(&active_dir_gen);

        move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };

            let dir_gen = active_dir_gen.fetch_add(1, Ordering::Relaxed) + 1;

            let (path, is_network, is_this_pc, sort_col, asc, show_hidden, show_ext, view_mode, cloud_roots) = {
                let st = state.borrow();
                (
                    st.current_path.clone(),
                    st.is_network_view,
                    st.is_this_pc_view,
                    st.sort_column.clone(),
                    st.sort_asc,
                    st.show_hidden,
                    st.show_extensions,
                    st.view_mode.clone(),
                    st.cloud_roots.clone(),
                )
            };

            // 1. Enumerate folder or network resources WITHOUT holding RefCell borrow
            let mut entries = if is_this_pc {
                Vec::new()
            } else if is_network {
                shell_interop::info::network_resources()
            } else {
                match fs_core::list_dir(&path.to_string_lossy()) {
                    Ok(entries) => entries,
                    Err(_) => Vec::new(),
                }
            };

            if !show_hidden && !is_network && !is_this_pc {
                entries.retain(|e| (e.attrs & ATTR_HIDDEN == 0) && !e.name.starts_with('.'));
            }

            // 2. Sort entries (folders first, natural StrCmpLogicalW order)
            entries.sort_by(|a, b| {
                if a.is_dir != b.is_dir {
                    return b.is_dir.cmp(&a.is_dir);
                }
                let ord = match sort_col.as_str() {
                    "date" => a.modified.cmp(&b.modified),
                    "type" => a.ext.to_lowercase().cmp(&b.ext.to_lowercase()),
                    "size" => a.size.cmp(&b.size),
                    _ => fs_core::natural_cmp(&a.name, &b.name),
                };
                if asc { ord } else { ord.reverse() }
            });

            // 3. Map entries to Slint models and schedule background thumbnail loading
            let entry_items = map_entries_and_schedule_thumbs(
                &entries,
                show_ext,
                &cloud_roots,
                &view_mode,
                &thumb_cache,
                &sta_pool,
                main_window_weak.clone(),
                dir_gen,
                &active_dir_gen,
            );
            let grid_rows = build_grid_rows(&entry_items, &view_mode);

            let folder_name = if is_this_pc {
                "This PC".to_string()
            } else if is_network {
                "Network".to_string()
            } else {
                path.file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string())
            };

            let (tab_items, active_idx, can_back, can_fwd) = {
                let mut st = state.borrow_mut();
                st.entries = entries.clone();
                st.selected_idx = None;

                let active_idx = st.active_tab_idx;
                let back_clone = st.history_back.clone();
                let fwd_clone = st.history_forward.clone();
                if let Some(tab) = st.tabs.get_mut(active_idx) {
                    tab.path = path.clone();
                    tab.title = folder_name.clone();
                    tab.is_this_pc = is_this_pc;
                    tab.is_network = is_network;
                    tab.history_back = back_clone;
                    tab.history_forward = fwd_clone;
                }

                let tabs: Vec<TabItem> = st
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(idx, t)| TabItem {
                        id: t.id,
                        title: t.title.clone().into(),
                        path: t.path.to_string_lossy().to_string().into(),
                        is_active: idx == active_idx,
                    })
                    .collect();

                (
                    tabs,
                    active_idx,
                    !st.history_back.is_empty(),
                    !st.history_forward.is_empty(),
                )
            };

            ui.set_entries(ModelRc::new(Rc::new(VecModel::from(entry_items))));
            ui.set_grid_rows(ModelRc::new(Rc::new(VecModel::from(grid_rows))));
            ui.set_selected_index(-1);
            ui.set_status_item_count(
                if is_this_pc {
                    format!("{} items", ui.get_drives().row_count()).into()
                } else {
                    format!("{} items", entries.len()).into()
                },
            );
            ui.set_status_selection_info("".into());
            ui.set_is_renaming(false);
            ui.set_active_flyout("".into());
            ui.set_show_sub_flyout(false);

            // 4. Update breadcrumbs, active nav highlight, and folder title
            let crumbs = build_crumbs(&path, is_network, is_this_pc);
            ui.set_crumbs(ModelRc::new(Rc::new(VecModel::from(crumbs))));
            ui.set_current_path(
                if is_this_pc {
                    "This PC".to_string().into()
                } else if is_network {
                    "\\\\".to_string().into()
                } else {
                    path.to_string_lossy().to_string().into()
                },
            );
            ui.set_active_nav_key(detect_nav_key(&path, is_network, is_this_pc).into());
            ui.set_current_folder_name(folder_name.into());
            ui.set_can_go_back(can_back);
            ui.set_can_go_forward(can_fwd);

            ui.set_tabs(ModelRc::new(Rc::new(VecModel::from(tab_items))));
            ui.set_active_tab_index(active_idx as i32);

            // 5. Reset preview / details
            ui.set_preview_type("none".into());
            ui.set_has_preview_image(false);
            ui.set_item_properties(ModelRc::new(Rc::new(VecModel::from(Vec::new()))));
        }
    };

    // Initial render
    update_ui();

    // Poll Windows 11 registry theme every 2s AND install the native Win32 Mouse Back/Forward hook
    let theme_timer = Timer::default();
    {
        let main_window_weak = main_window.as_weak();
        let hook_installed = Arc::new(AtomicBool::new(false));
        theme_timer.start(
            TimerMode::Repeated,
            std::time::Duration::from_millis(600),
            move || {
                let hwnd = get_active_hwnd();
                if !hwnd.0.is_null() && !hook_installed.swap(true, Ordering::Relaxed) {
                    let ui_weak_nav = main_window_weak.clone();
                    shell_interop::system::install_mouse_nav_hook(hwnd, move |is_back| {
                        let ui_weak_cb = ui_weak_nav.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak_cb.upgrade() {
                                if is_back {
                                    ui.invoke_nav_back();
                                } else {
                                    ui.invoke_nav_forward();
                                }
                            }
                        });
                    });
                }

                if let Some(ui) = main_window_weak.upgrade() {
                    let os_dark = shell_interop::system::is_dark_mode();
                    if ui.get_is_dark_mode() != os_dark {
                        ui.set_is_dark_mode(os_dark);
                        shell_interop::system::apply_window_theme(hwnd, os_dark);
                    }
                }
            },
        );
    }

    // 6. Frameless Window Caption Callbacks
    main_window.on_window_drag(move || {
        let hwnd = get_active_hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = ReleaseCapture();
                SendMessageW(
                    hwnd,
                    WM_NCLBUTTONDOWN,
                    Some(WPARAM(HTCAPTION as usize)),
                    Some(LPARAM(0)),
                );
                let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), LPARAM(0));
            }
        }
    });

    main_window.on_window_minimize(move || {
        let hwnd = get_active_hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
    });

    main_window.on_window_maximize(move || {
        let hwnd = get_active_hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                if IsZoomed(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                } else {
                    let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                }
            }
        }
    });

    main_window.on_window_close(move || {
        let hwnd = get_active_hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        let _ = slint::quit_event_loop();
    });

    // 7. Navigation Callbacks (supports "This PC", Local Drives, Google Drive G:\, and NAS \\server\share)
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();
        main_window.on_nav_to_path(move |target| {
            let trimmed = target.trim();
            if trimmed.eq_ignore_ascii_case("this_pc") || trimmed.eq_ignore_ascii_case("this pc") {
                {
                    let mut st = state.borrow_mut();
                    let curr = st.current_path.clone();
                    st.history_back.push(curr);
                    st.history_forward.clear();
                    st.is_this_pc_view = true;
                    st.is_network_view = false;
                }
                if let Some(ui) = main_window_weak.upgrade() {
                    ui.set_search_text("".into());
                }
                update_ui();
                return;
            }

            if trimmed.eq_ignore_ascii_case("network") || trimmed == "\\\\" {
                {
                    let mut st = state.borrow_mut();
                    let curr = st.current_path.clone();
                    st.history_back.push(curr);
                    st.history_forward.clear();
                    st.is_network_view = true;
                    st.is_this_pc_view = false;
                }
                if let Some(ui) = main_window_weak.upgrade() {
                    ui.set_search_text("".into());
                }
                update_ui();
                return;
            }

            let p = resolve_special_path(trimmed);
            if p.exists() {
                if p.is_dir() {
                    {
                        let mut st = state.borrow_mut();
                        let curr = st.current_path.clone();
                        if curr != p || st.is_network_view || st.is_this_pc_view {
                            st.history_back.push(curr);
                            st.history_forward.clear();
                            st.current_path = p;
                            st.is_network_view = false;
                            st.is_this_pc_view = false;
                        }
                    }
                    if let Some(ui) = main_window_weak.upgrade() {
                        ui.set_search_text("".into());
                    }
                    update_ui();
                } else {
                    let hwnd = get_active_hwnd();
                    let _ = shell_interop::ops::shell_open(hwnd, &p.to_string_lossy(), None);
                }
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();
        main_window.on_nav_back(move || {
            let prev_opt = {
                let mut st = state.borrow_mut();
                if let Some(prev) = st.history_back.pop() {
                    let curr = st.current_path.clone();
                    st.history_forward.push(curr);
                    st.current_path = prev.clone();
                    st.is_network_view = false;
                    st.is_this_pc_view = false;
                    Some(prev)
                } else {
                    None
                }
            };
            if prev_opt.is_some() {
                if let Some(ui) = main_window_weak.upgrade() {
                    ui.set_search_text("".into());
                }
                update_ui();
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();
        main_window.on_nav_forward(move || {
            let next_opt = {
                let mut st = state.borrow_mut();
                if let Some(next) = st.history_forward.pop() {
                    let curr = st.current_path.clone();
                    st.history_back.push(curr);
                    st.current_path = next.clone();
                    st.is_network_view = false;
                    st.is_this_pc_view = false;
                    Some(next)
                } else {
                    None
                }
            };
            if next_opt.is_some() {
                if let Some(ui) = main_window_weak.upgrade() {
                    ui.set_search_text("".into());
                }
                update_ui();
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();
        main_window.on_nav_up(move || {
            let moved = {
                let mut st = state.borrow_mut();
                if let Some(parent) = st.current_path.parent().map(|p| p.to_path_buf()) {
                    let curr = st.current_path.clone();
                    if curr != parent {
                        st.history_back.push(curr);
                        st.history_forward.clear();
                        st.current_path = parent;
                        st.is_network_view = false;
                        st.is_this_pc_view = false;
                        true
                    } else {
                        st.history_back.push(curr);
                        st.history_forward.clear();
                        st.is_this_pc_view = true;
                        true
                    }
                } else if !st.is_this_pc_view {
                    st.is_this_pc_view = true;
                    true
                } else {
                    false
                }
            };
            if moved {
                if let Some(ui) = main_window_weak.upgrade() {
                    ui.set_search_text("".into());
                }
                update_ui();
            }
        });
    }

    {
        let update_ui = update_ui.clone();
        main_window.on_nav_refresh(move || {
            update_ui();
        });
    }

    // 8. Zero-Lag Asynchronous Search Callback (Runs off UI thread with AtomicU64 generation counter)
    {
        let indexer = Arc::clone(&indexer);
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();
        let update_ui_clone = update_ui.clone();
        let search_gen = Arc::clone(&search_gen);

        main_window.on_search_changed(move |query| {
            let q = query.trim().to_string();
            let my_gen = search_gen.fetch_add(1, Ordering::SeqCst) + 1;

            if q.is_empty() {
                update_ui_clone();
                return;
            }

            let (curr_dir, show_ext, cloud_roots, view_mode) = {
                let st = state.borrow();
                (
                    st.current_path.to_string_lossy().to_string(),
                    st.show_extensions,
                    st.cloud_roots.clone(),
                    st.view_mode.clone(),
                )
            };

            let indexer_bg = Arc::clone(&indexer);
            let search_gen_bg = Arc::clone(&search_gen);
            let ui_weak = main_window_weak.clone();

            std::thread::spawn(move || {
                // Fast SIMD search in RAM (<5ms) + bounded local check
                let results = indexer_bg.search_combined(&curr_dir, &q, 200);
                if search_gen_bg.load(Ordering::SeqCst) != my_gen {
                    return;
                }

                let _ = slint::invoke_from_event_loop(move || {
                    if search_gen_bg.load(Ordering::SeqCst) != my_gen {
                        return;
                    }
                    let Some(ui) = ui_weak.upgrade() else { return };

                    let entry_items: Vec<EntryItem> = results
                        .iter()
                        .enumerate()
                        .map(|(i, e)| {
                            let (icon, color, type_name) = resolve_icon_and_type(e);
                            let is_cloud = shell_interop::icons::is_cloud_synced_item(
                                &e.path,
                                e.attrs,
                                &cloud_roots,
                            );
                            let display_name = display_name_for_entry(e, show_ext);
                            EntryItem {
                                id: format!("{i}").into(),
                                name: display_name.into(),
                                path: e.path.clone().into(),
                                is_dir: e.is_dir,
                                size_str: if e.is_dir {
                                    "".into()
                                } else {
                                    format_bytes(e.size).into()
                                },
                                mtime_str: format_mtime(e.modified).into(),
                                type_str: type_name.into(),
                                icon_glyph: icon.into(),
                                icon_color: color,
                                has_thumb: false,
                                thumb: Image::default(),
                                is_cloud,
                                orig_index: i as i32,
                            }
                        })
                        .collect();

                    let grid_rows = build_grid_rows(&entry_items, &view_mode);
                    let count = entry_items.len();

                    ui.set_entries(ModelRc::new(Rc::new(VecModel::from(entry_items))));
                    ui.set_grid_rows(ModelRc::new(Rc::new(VecModel::from(grid_rows))));
                    ui.set_selected_index(-1);
                    ui.set_status_item_count(format!("{count} search results").into());
                    ui.set_status_selection_info("".into());
                });
            });
        });
    }

    // 9. Item Selection & Universal Previews
    {
        let state = Rc::clone(&state);
        let sta_pool = Arc::clone(&sta_pool);
        let main_window_weak = main_window.as_weak();

        main_window.on_item_clicked(move |idx| {
            if idx < 0 {
                return;
            }
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            select_entry_safe(&ui, &state, &sta_pool, idx as usize);
        });
    }

    // 9b. Native Cross-Application File Drag-and-Drop (`SHDoDragDrop`)
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();

        main_window.on_item_drag_started(move |idx| {
            if idx < 0 {
                return;
            }
            let i = idx as usize;
            let path_opt = {
                let st = state.borrow();
                st.entries.get(i).map(|e| e.path.clone())
            };
            if let Some(path) = path_opt {
                let hwnd = get_active_hwnd();
                unsafe {
                    let _ = ReleaseCapture();
                }
                let _ = shell_interop::ops::start_drag_drop(hwnd, &[path]);
                if !hwnd.0.is_null() {
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), LPARAM(0));
                    }
                }
                update_ui();
            }
        });
    }

    // 10. Double-click or Enter to open folder or launch file
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_item_double_clicked(move |idx| {
            if idx < 0 {
                return;
            }
            let i = idx as usize;
            let path_opt = {
                let st = state.borrow();
                st.entries.get(i).map(|e| (e.path.clone(), e.is_dir))
            };
            // Fallback to UI model if user clicked a search result
            let resolved = path_opt.or_else(|| {
                main_window_weak.upgrade().and_then(|ui| {
                    ui.get_entries()
                        .row_data(i)
                        .map(|item| (item.path.to_string(), item.is_dir))
                })
            });

            if let Some((path_str, is_dir)) = resolved {
                if is_dir {
                    {
                        let mut st = state.borrow_mut();
                        let curr = st.current_path.clone();
                        st.history_back.push(curr);
                        st.history_forward.clear();
                        st.current_path = PathBuf::from(&path_str);
                        st.is_network_view = false;
                        st.is_this_pc_view = false;
                    }
                    if let Some(ui) = main_window_weak.upgrade() {
                        ui.set_search_text("".into());
                    }
                    update_ui();
                } else {
                    let hwnd = get_active_hwnd();
                    let _ = shell_interop::ops::shell_open(hwnd, &path_str, None);
                }
            }
        });
    }

    // 11. Native Dark/Light Win32 Context Menu on Right Click
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_item_right_clicked(move |idx| {
            if idx < 0 {
                return;
            }
            let i = idx as usize;
            let entry_opt = {
                let st = state.borrow();
                st.entries.get(i).cloned()
            };

            if let Some(entry) = entry_opt {
                let hwnd = get_active_hwnd();
                let pt = get_cursor_point();
                if let Ok(action) = shell_interop::context_menu::show(
                    hwnd,
                    std::slice::from_ref(&entry.path),
                    None,
                    Some((pt.x, pt.y)),
                    false,
                ) {
                    match action.as_str() {
                        "open_new_tab" => {
                            let new_path = PathBuf::from(&entry.path);
                            let title = new_path
                                .file_name()
                                .map(|f| f.to_string_lossy().to_string())
                                .unwrap_or_else(|| entry.name.clone());
                            {
                                let mut st = state.borrow_mut();
                                let new_id = st.next_tab_id;
                                st.next_tab_id += 1;
                                st.tabs.push(TabState {
                                    id: new_id,
                                    title,
                                    path: new_path.clone(),
                                    is_this_pc: false,
                                    is_network: false,
                                    history_back: Vec::new(),
                                    history_forward: Vec::new(),
                                });
                                st.active_tab_idx = st.tabs.len() - 1;
                                st.current_path = new_path;
                                st.is_network_view = false;
                                st.is_this_pc_view = false;
                                st.history_back.clear();
                                st.history_forward.clear();
                            }
                            update_ui();
                        }
                        "open_new_window" => {
                            if let Ok(exe) = std::env::current_exe() {
                                let _ = std::process::Command::new(exe).spawn();
                            }
                        }
                        "rename" => {
                            if let Some(ui) = main_window_weak.upgrade() {
                                ui.set_rename_text(entry.name.into());
                                ui.set_is_renaming(true);
                            }
                        }
                        _ => {
                            update_ui();
                        }
                    }
                }
            }
        });
    }

    // 12. Command Bar & Flyout Actions
    {
        let state = Rc::clone(&state);
        let sta_pool = Arc::clone(&sta_pool);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_new_folder(move || {
            let (hwnd, curr) = {
                let st = state.borrow();
                (get_active_hwnd(), st.current_path.clone())
            };

            let mut candidate = "New folder".to_string();
            let mut counter = 2;
            while curr.join(&candidate).exists() {
                candidate = format!("New folder ({counter})");
                counter += 1;
            }

            if shell_interop::ops::create_new(hwnd, &curr.to_string_lossy(), &candidate, true)
                .is_ok()
            {
                update_ui();
                if let Some(ui) = main_window_weak.upgrade() {
                    let pos_opt = {
                        let st = state.borrow();
                        st.entries.iter().position(|e| e.name == candidate)
                    };
                    if let Some(pos) = pos_opt {
                        select_entry_safe(&ui, &state, &sta_pool, pos);
                        ui.set_rename_text(candidate.into());
                        ui.set_is_renaming(true);
                    }
                }
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let sta_pool = Arc::clone(&sta_pool);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_new_file(move |template_name| {
            let (hwnd, curr) = {
                let st = state.borrow();
                (get_active_hwnd(), st.current_path.clone())
            };

            let base_str = template_name.as_str();
            let (stem, ext) = base_str.rsplit_once('.').unwrap_or((base_str, "txt"));
            let mut candidate = format!("{stem}.{ext}");
            let mut counter = 2;
            while curr.join(&candidate).exists() {
                candidate = format!("{stem} ({counter}).{ext}");
                counter += 1;
            }

            if shell_interop::ops::create_new(hwnd, &curr.to_string_lossy(), &candidate, false)
                .is_ok()
            {
                update_ui();
                if let Some(ui) = main_window_weak.upgrade() {
                    let pos_opt = {
                        let st = state.borrow();
                        st.entries.iter().position(|e| e.name == candidate)
                    };
                    if let Some(pos) = pos_opt {
                        select_entry_safe(&ui, &state, &sta_pool, pos);
                        ui.set_rename_text(candidate.into());
                        ui.set_is_renaming(true);
                    }
                }
            }
        });
    }

    {
        let update_ui = update_ui.clone();
        main_window.on_command_map_network_drive(move || {
            let hwnd = get_active_hwnd();
            shell_interop::info::map_network_drive_dialog(hwnd);
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();

        main_window.on_command_rename(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let name_opt = {
                let st = state.borrow();
                st.selected_idx
                    .and_then(|idx| st.entries.get(idx))
                    .map(|e| e.name.clone())
            };
            if let Some(name) = name_opt {
                ui.set_rename_text(name.into());
                ui.set_is_renaming(true);
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();

        main_window.on_command_commit_rename(move |new_name| {
            let trimmed = new_name.trim();
            if trimmed.is_empty() {
                return;
            }
            let (hwnd, old_path_opt) = {
                let st = state.borrow();
                let p = st
                    .selected_idx
                    .and_then(|i| st.entries.get(i))
                    .map(|e| e.path.clone());
                (get_active_hwnd(), p)
            };
            if let Some(old_path) = old_path_opt {
                if shell_interop::ops::rename(hwnd, &old_path, trimmed).is_ok() {
                    update_ui();
                }
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();

        main_window.on_command_delete(move || {
            let (hwnd, path_opt) = {
                let st = state.borrow();
                let p = st
                    .selected_idx
                    .and_then(|i| st.entries.get(i))
                    .map(|e| e.path.clone());
                (get_active_hwnd(), p)
            };
            if let Some(p) = path_opt {
                let _ = shell_interop::ops::delete(hwnd, &[p], false);
                update_ui();
            }
        });
    }

    {
        let state = Rc::clone(&state);

        main_window.on_command_copy(move || {
            let (hwnd, path_opt) = {
                let st = state.borrow();
                let p = st
                    .selected_idx
                    .and_then(|i| st.entries.get(i))
                    .map(|e| e.path.clone());
                (get_active_hwnd(), p)
            };
            if let Some(p) = path_opt {
                let _ = shell_interop::clipboard::set_files(hwnd, &[p], false);
            }
        });
    }

    {
        let state = Rc::clone(&state);

        main_window.on_command_cut(move || {
            let (hwnd, path_opt) = {
                let st = state.borrow();
                let p = st
                    .selected_idx
                    .and_then(|i| st.entries.get(i))
                    .map(|e| e.path.clone());
                (get_active_hwnd(), p)
            };
            if let Some(p) = path_opt {
                let _ = shell_interop::clipboard::set_files(hwnd, &[p], true);
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();

        main_window.on_command_paste(move || {
            let (hwnd, dest) = {
                let st = state.borrow();
                (
                    get_active_hwnd(),
                    st.current_path.to_string_lossy().to_string(),
                )
            };
            if let Ok((files, is_cut)) = shell_interop::clipboard::get_files(hwnd) {
                let mode = if is_cut {
                    shell_interop::ops::Transfer::Move
                } else {
                    shell_interop::ops::Transfer::Copy
                };
                let _ = shell_interop::ops::transfer(hwnd, &files, &dest, mode);
                update_ui();
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_view(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let new_mode = {
                let mut st = state.borrow_mut();
                if st.view_mode == "details" {
                    st.view_mode = "tiles".to_string();
                } else {
                    st.view_mode = "details".to_string();
                }
                st.view_mode.clone()
            };
            ui.set_view_mode(new_mode.into());
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_set_view(move |mode| {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            {
                let mut st = state.borrow_mut();
                st.view_mode = mode.to_string();
            }
            ui.set_view_mode(mode);
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_hidden(move || {
            let show = {
                let mut st = state.borrow_mut();
                st.show_hidden = !st.show_hidden;
                st.show_hidden
            };
            if let Some(ui) = main_window_weak.upgrade() {
                ui.set_show_hidden(show);
            }
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_compact(move || {
            let compact = {
                let mut st = state.borrow_mut();
                st.compact_view = !st.compact_view;
                st.compact_view
            };
            if let Some(ui) = main_window_weak.upgrade() {
                ui.set_compact_view(compact);
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_extensions(move || {
            let show_ext = {
                let mut st = state.borrow_mut();
                st.show_extensions = !st.show_extensions;
                st.show_extensions
            };
            if let Some(ui) = main_window_weak.upgrade() {
                ui.set_show_file_extensions(show_ext);
            }
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let sta_pool = Arc::clone(&sta_pool);
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_preview(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let (show_prev, sel_opt) = {
                let mut st = state.borrow_mut();
                st.show_preview = !st.show_preview;
                if st.show_preview {
                    st.show_details = false;
                }
                (st.show_preview, st.selected_idx)
            };
            if show_prev {
                ui.set_show_details(false);
            }
            ui.set_show_preview(show_prev);
            if let Some(sel) = sel_opt {
                select_entry_safe(&ui, &state, &sta_pool, sel);
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let sta_pool = Arc::clone(&sta_pool);
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_details(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let (show_det, sel_opt) = {
                let mut st = state.borrow_mut();
                st.show_details = !st.show_details;
                if st.show_details {
                    st.show_preview = false;
                }
                (st.show_details, st.selected_idx)
            };
            if show_det {
                ui.set_show_preview(false);
            }
            ui.set_show_details(show_det);
            if let Some(sel) = sel_opt {
                select_entry_safe(&ui, &state, &sta_pool, sel);
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_sort_by(move |col| {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let (col_str, asc) = {
                let mut st = state.borrow_mut();
                let c = col.as_str();
                if st.sort_column == c {
                    st.sort_asc = !st.sort_asc;
                } else {
                    st.sort_column = c.to_string();
                    st.sort_asc = true;
                }
                (st.sort_column.clone(), st.sort_asc)
            };
            ui.set_sort_column(col_str.into());
            ui.set_sort_asc(asc);
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_sort_direction(move |asc| {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            {
                let mut st = state.borrow_mut();
                st.sort_asc = asc;
            }
            ui.set_sort_asc(asc);
            update_ui();
        });
    }

    // 13. Keyboard Arrow Navigation
    {
        let state = Rc::clone(&state);
        let sta_pool = Arc::clone(&sta_pool);
        let main_window_weak = main_window.as_weak();

        main_window.on_key_shortcut(move |action| {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let next_idx_opt = {
                let st = state.borrow();
                let len = st.entries.len();
                if len == 0 {
                    None
                } else {
                    match (action.as_str(), st.selected_idx) {
                        ("down", Some(curr)) => Some((curr + 1).min(len - 1)),
                        ("down", None) => Some(0),
                        ("up", Some(curr)) => Some(curr.saturating_sub(1)),
                        ("up", None) => Some(0),
                        _ => None,
                    }
                }
            };
            if let Some(next_idx) = next_idx_opt {
                select_entry_safe(&ui, &state, &sta_pool, next_idx);
            }
        });
    }

    // 14. Tab Management (Select, New Tab +, Close Tab)
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_tab_selected(move |idx| {
            let i = idx as usize;
            let changed = {
                let mut st = state.borrow_mut();
                if i < st.tabs.len() && i != st.active_tab_idx {
                    st.active_tab_idx = i;
                    st.current_path = st.tabs[i].path.clone();
                    st.is_this_pc_view = st.tabs[i].is_this_pc;
                    st.is_network_view = st.tabs[i].is_network;
                    st.history_back = st.tabs[i].history_back.clone();
                    st.history_forward = st.tabs[i].history_forward.clone();
                    true
                } else {
                    false
                }
            };
            if changed {
                if let Some(ui) = main_window_weak.upgrade() {
                    ui.set_search_text("".into());
                }
                update_ui();
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_tab_new(move || {
            {
                let mut st = state.borrow_mut();
                let new_id = st.next_tab_id;
                st.next_tab_id += 1;
                let path = st.current_path.clone();
                let is_this_pc = st.is_this_pc_view;
                let is_network = st.is_network_view;
                let title = if is_this_pc {
                    "This PC".to_string()
                } else if is_network {
                    "Network".to_string()
                } else {
                    path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Home".to_string())
                };
                st.tabs.push(TabState {
                    id: new_id,
                    title,
                    path: path.clone(),
                    is_this_pc,
                    is_network,
                    history_back: Vec::new(),
                    history_forward: Vec::new(),
                });
                st.active_tab_idx = st.tabs.len() - 1;
                st.history_back.clear();
                st.history_forward.clear();
            }
            if let Some(ui) = main_window_weak.upgrade() {
                ui.set_search_text("".into());
            }
            update_ui();
        });
    }

    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();

        main_window.on_tab_closed(move |idx| {
            let i = idx as usize;
            let closed = {
                let mut st = state.borrow_mut();
                if st.tabs.len() > 1 && i < st.tabs.len() {
                    st.tabs.remove(i);
                    if st.active_tab_idx >= st.tabs.len() {
                        st.active_tab_idx = st.tabs.len() - 1;
                    }
                    let active = st.active_tab_idx;
                    st.current_path = st.tabs[active].path.clone();
                    st.is_this_pc_view = st.tabs[active].is_this_pc;
                    st.is_network_view = st.tabs[active].is_network;
                    st.history_back = st.tabs[active].history_back.clone();
                    st.history_forward = st.tabs[active].history_forward.clone();
                    true
                } else {
                    false
                }
            };
            if closed {
                update_ui();
            }
        });
    }

    // Run the native Windows application loop
    main_window.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_mtime_dates_and_leap_years() {
        assert!(!format_mtime(1709164800_000).is_empty()); // Feb 29, 2024
        assert!(!format_mtime(1772668800_000).is_empty()); // Mar 05, 2026
        assert!(!format_mtime(1078012800_000).is_empty()); // Feb 29, 2004
        assert_eq!(format_mtime(0), "");
        assert_eq!(format_mtime(-5000), "");
    }
}
