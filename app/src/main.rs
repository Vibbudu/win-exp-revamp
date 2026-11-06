#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use indexer::IndexerEngine;
use ipc_types::{Entry, Preview};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, HTCAPTION, IsZoomed, PostMessageW, SW_MAXIMIZE,
    SW_MINIMIZE, SW_RESTORE, SendMessageW, ShowWindow, WM_CLOSE, WM_NCLBUTTONDOWN,
};

slint::include_modules!();

struct TabState {
    id: i32,
    title: String,
    path: PathBuf,
    history_back: Vec<PathBuf>,
    history_forward: Vec<PathBuf>,
}

struct ExplorerState {
    current_path: PathBuf,
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
    selected_idx: Option<usize>,
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

fn format_mtime(millis: i64) -> String {
    if millis <= 0 {
        return String::new();
    }
    let secs = millis / 1000;
    let days = secs / 86400;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1460 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{d:02}-{m:02}-{y:04} {hours:02}:{minutes:02}")
}

/// Resolves a crisp Segoe Fluent Icons codepoint, tint color, and Windows 11 Type description.
fn resolve_icon_and_type(entry: &Entry) -> (&'static str, Color, String) {
    if entry.is_dir {
        return (
            "\u{E8B7}",
            Color::from_rgb_u8(255, 206, 61),
            "File folder".to_string(),
        );
    }

    let ext = entry.ext.to_lowercase();
    match ext.as_str() {
        "xlsx" | "xls" | "xlsb" | "csv" | "ods" => (
            "\u{F0E3}",
            Color::from_rgb_u8(33, 163, 102),
            "Microsoft Excel Worksheet".to_string(),
        ),
        "docx" | "doc" | "rtf" => (
            "\u{E8A5}",
            Color::from_rgb_u8(65, 132, 243),
            "Microsoft Word Document".to_string(),
        ),
        "pptx" | "ppt" => (
            "\u{E8A5}",
            Color::from_rgb_u8(235, 108, 71),
            "Microsoft PowerPoint Presentation".to_string(),
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
        | "yaml" | "yml" | "xml" | "html" | "css" | "slint" => (
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

fn build_crumbs(path: &Path) -> Vec<CrumbItem> {
    let mut crumbs = Vec::new();
    let mut current = PathBuf::new();

    for comp in path.components() {
        match comp {
            std::path::Component::Prefix(prefix) => {
                let p = prefix.as_os_str().to_string_lossy().to_string();
                let full = format!("{}\\", p);
                crumbs.push(CrumbItem {
                    label: format!("Local Disk ({p})").into(),
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

fn detect_nav_key(path: &Path) -> &'static str {
    let s = path.to_string_lossy().to_lowercase();
    if s.ends_with("\\desktop") {
        "desktop"
    } else if s.ends_with("\\downloads") {
        "downloads"
    } else if s.ends_with("\\documents") {
        "documents"
    } else if s.ends_with("\\pictures") {
        "pictures_folder"
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
        "pictures" => resolve_known_folder("Pictures"),
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

fn select_entry_and_update_panes(ui: &MainWindow, st: &mut ExplorerState, idx: usize) {
    if idx >= st.entries.len() {
        return;
    }
    st.selected_idx = Some(idx);
    ui.set_selected_index(idx as i32);

    if let Some(entry) = st.entries.get(idx) {
        let p = entry.path.clone();
        ui.set_selected_path(p.clone().into());
        let size_str = if entry.is_dir {
            "".to_string()
        } else {
            format!("  {}", format_bytes(entry.size))
        };
        ui.set_status_selection_info(format!("1 item selected{size_str}").into());

        // Populate Details pane if visible
        if st.show_details {
            let props = shell_interop::props::properties(&p);
            let prop_items: Vec<PropertyItem> = props
                .into_iter()
                .map(|pr| PropertyItem {
                    name: pr.label.into(),
                    value: pr.value.into(),
                })
                .collect();
            ui.set_item_properties(ModelRc::new(Rc::new(VecModel::from(prop_items))));
        }

        // Populate Universal Preview pane if visible
        if st.show_preview {
            let (_, _, type_desc) = resolve_icon_and_type(entry);
            ui.set_preview_title(entry.name.clone().into());
            ui.set_preview_subtitle(type_desc.into());

            if entry.is_dir {
                let sub_count = fs_core::list_dir(&p).map(|v| v.len()).unwrap_or(0);
                ui.set_preview_type("text".into());
                ui.set_preview_text_content(
                    format!("Folder: {}\nPath: {}\nContains: {} items", entry.name, p, sub_count)
                        .into(),
                );
                return;
            }

            let prev = preview::generate_preview(&p);
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
                            "File: {}\nSize: {}\nPath: {}",
                            entry.name,
                            format_bytes(entry.size),
                            entry.path
                        )
                        .into(),
                    );
                }
            }
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    // 1. Initialize STA worker pool for Windows Shell COM operations
    let _sta_pool = shell_interop::StaPool::new("sta-pool", 2);

    let main_window = MainWindow::new()?;

    // 2. Discover local drives and bind them to the UI
    let drives = shell_interop::info::drives();
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
            DriveItem {
                letter: d.path.clone().into(),
                name: if d.label.is_empty() {
                    "Local Disk".into()
                } else {
                    d.label.clone().into()
                },
                free_str: format_bytes(free).into(),
                total_str: format_bytes(total).into(),
                percent_used: percent,
            }
        })
        .collect();
    main_window.set_drives(ModelRc::new(Rc::new(VecModel::from(drive_items))));

    // 3. Set initial path (User profile or C:\)
    let initial_path = if let Some(up) = std::env::var_os("USERPROFILE") {
        PathBuf::from(up)
    } else {
        PathBuf::from("C:\\")
    };

    let initial_title = initial_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "Home".to_string());

    let state = Rc::new(RefCell::new(ExplorerState {
        current_path: initial_path.clone(),
        history_back: Vec::new(),
        history_forward: Vec::new(),
        tabs: vec![TabState {
            id: 0,
            title: initial_title,
            path: initial_path.clone(),
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
        selected_idx: None,
    }));

    // 4. Start Priority + Background Indexer (indexes Desktop/Documents/Downloads immediately)
    let indexer = Arc::new(IndexerEngine::new());
    indexer.start_background_indexing();

    // Helper to refresh UI state for the current folder
    let update_ui = {
        let main_window_weak = main_window.as_weak();
        let state = Rc::clone(&state);

        move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };

            let mut st = state.borrow_mut();
            let path = st.current_path.clone();

            // 1. Enumerate folder using fs_core large-fetch Win32 API
            let mut entries = match fs_core::list_dir(&path.to_string_lossy()) {
                Ok(entries) => entries,
                Err(_) => Vec::new(),
            };

            // 2. Sort entries (folders first, natural StrCmpLogicalW order)
            let sort_col = st.sort_column.clone();
            let asc = st.sort_asc;
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

            st.entries = entries.clone();
            st.selected_idx = None;

            // 3. Map entries to Slint models
            let entry_items: Vec<EntryItem> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let (icon, color, type_name) = resolve_icon_and_type(e);
                    EntryItem {
                        id: format!("{i}").into(),
                        name: e.name.clone().into(),
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
                    }
                })
                .collect();

            ui.set_entries(ModelRc::new(Rc::new(VecModel::from(entry_items))));
            ui.set_selected_index(-1);
            ui.set_status_item_count(format!("{} items", entries.len()).into());
            ui.set_status_selection_info("".into());
            ui.set_is_renaming(false);

            // 4. Update breadcrumbs, active nav highlight, and folder title
            let crumbs = build_crumbs(&path);
            ui.set_crumbs(ModelRc::new(Rc::new(VecModel::from(crumbs))));
            ui.set_current_path(path.to_string_lossy().to_string().into());
            ui.set_active_nav_key(detect_nav_key(&path).into());

            let folder_name = path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            ui.set_current_folder_name(folder_name.clone().into());

            ui.set_can_go_back(!st.history_back.is_empty());
            ui.set_can_go_forward(!st.history_forward.is_empty());

            // 5. Sync active tab state
            let active_idx = st.active_tab_idx;
            let back_clone = st.history_back.clone();
            let fwd_clone = st.history_forward.clone();
            if let Some(tab) = st.tabs.get_mut(active_idx) {
                tab.path = path.clone();
                tab.title = folder_name;
                tab.history_back = back_clone;
                tab.history_forward = fwd_clone;
            }

            let tab_items: Vec<TabItem> = st
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
            ui.set_tabs(ModelRc::new(Rc::new(VecModel::from(tab_items))));
            ui.set_active_tab_index(active_idx as i32);

            // 6. Reset preview / details
            ui.set_preview_type("none".into());
            ui.set_item_properties(ModelRc::new(Rc::new(VecModel::from(Vec::new()))));
        }
    };

    // Initial render
    update_ui();

    // 5. Frameless Window Caption Callbacks (Drag, Minimize, Maximize/Restore, Close)
    main_window.on_window_drag(move || {
        let hwnd = get_active_hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = ReleaseCapture();
                SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
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

    // 6. Navigation Callbacks
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();
        main_window.on_nav_to_path(move |target| {
            let p = resolve_special_path(target.as_str());
            if p.exists() {
                if p.is_dir() {
                    let mut st = state.borrow_mut();
                    let curr = st.current_path.clone();
                    if curr != p {
                        st.history_back.push(curr);
                        st.history_forward.clear();
                        st.current_path = p;
                    }
                    drop(st);
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
            let mut st = state.borrow_mut();
            if let Some(prev) = st.history_back.pop() {
                let curr = st.current_path.clone();
                st.history_forward.push(curr);
                st.current_path = prev;
                drop(st);
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
            let mut st = state.borrow_mut();
            if let Some(next) = st.history_forward.pop() {
                let curr = st.current_path.clone();
                st.history_back.push(curr);
                st.current_path = next;
                drop(st);
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
            let mut st = state.borrow_mut();
            if let Some(parent) = st.current_path.parent().map(|p| p.to_path_buf()) {
                let curr = st.current_path.clone();
                if curr != parent {
                    st.history_back.push(curr);
                    st.history_forward.clear();
                    st.current_path = parent;
                    drop(st);
                    if let Some(ui) = main_window_weak.upgrade() {
                        ui.set_search_text("".into());
                    }
                    update_ui();
                }
            }
        });
    }

    {
        let update_ui = update_ui.clone();
        main_window.on_nav_refresh(move || {
            update_ui();
        });
    }

    // 7. Instant Search Callback (Live Current Subtree BFS + SIMD Global Arena Index)
    {
        let indexer = Arc::clone(&indexer);
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();
        let update_ui_clone = update_ui.clone();

        main_window.on_search_changed(move |query| {
            let q = query.trim().to_string();
            if q.is_empty() {
                update_ui_clone();
                return;
            }

            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };

            let curr_dir = {
                let st = state.borrow();
                st.current_path.to_string_lossy().to_string()
            };

            // Search both the active directory tree (instant live BFS) and the global SIMD index
            let results = indexer.search_combined(&curr_dir, &q, 300);

            let entry_items: Vec<EntryItem> = results
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let (icon, color, type_name) = resolve_icon_and_type(e);
                    EntryItem {
                        id: format!("{i}").into(),
                        name: e.name.clone().into(),
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
                    }
                })
                .collect();

            let count = entry_items.len();
            {
                let mut st = state.borrow_mut();
                st.entries = results;
                st.selected_idx = None;
            }

            ui.set_entries(ModelRc::new(Rc::new(VecModel::from(entry_items))));
            ui.set_selected_index(-1);
            ui.set_status_item_count(format!("{count} search results").into());
            ui.set_status_selection_info("".into());
        });
    }

    // 8. Item Selection & Universal Previews
    {
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();

        main_window.on_item_clicked(move |idx| {
            if idx < 0 {
                return;
            }
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let mut st = state.borrow_mut();
            select_entry_and_update_panes(&ui, &mut st, idx as usize);
        });
    }

    // 9. Double-click or Enter to open folder or launch file with default program
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_item_double_clicked(move |idx| {
            if idx < 0 {
                return;
            }
            let i = idx as usize;
            let entry_opt = {
                let st = state.borrow();
                st.entries.get(i).cloned()
            };

            if let Some(entry) = entry_opt {
                if entry.is_dir {
                    let mut st = state.borrow_mut();
                    let curr = st.current_path.clone();
                    st.history_back.push(curr);
                    st.history_forward.clear();
                    st.current_path = PathBuf::from(entry.path);
                    drop(st);
                    if let Some(ui) = main_window_weak.upgrade() {
                        ui.set_search_text("".into());
                    }
                    update_ui();
                } else {
                    let hwnd = get_active_hwnd();
                    let _ = shell_interop::ops::shell_open(hwnd, &entry.path, None);
                }
            }
        });
    }

    // 10. Native Win32 Context Menu on Right Click (IContextMenu3)
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();

        main_window.on_item_right_clicked(move |idx| {
            let i = idx as usize;
            let path_opt = {
                let st = state.borrow();
                st.entries.get(i).map(|e| e.path.clone())
            };

            if let Some(path) = path_opt {
                let hwnd = get_active_hwnd();
                let pt = get_cursor_point();
                let _ = shell_interop::context_menu::show(
                    hwnd,
                    &[path],
                    None,
                    Some((pt.x, pt.y)),
                    false,
                );
                update_ui();
            }
        });
    }

    // 11. Command Bar Actions (New Folder, Rename, Delete, Cut, Copy, Paste, View, Panes)
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_command_new_folder(move || {
            let (hwnd, curr) = {
                let st = state.borrow();
                (get_active_hwnd(), st.current_path.clone())
            };

            // Find an available "New folder" name
            let mut candidate = "New folder".to_string();
            let mut counter = 2;
            while curr.join(&candidate).exists() {
                candidate = format!("New folder ({counter})");
                counter += 1;
            }

            if shell_interop::ops::create_new(
                hwnd,
                &curr.to_string_lossy(),
                &candidate,
                true,
            )
            .is_ok()
            {
                update_ui();
                // Automatically select the newly created folder and open inline rename
                if let Some(ui) = main_window_weak.upgrade() {
                    let mut st = state.borrow_mut();
                    if let Some(pos) = st.entries.iter().position(|e| e.name == candidate) {
                        select_entry_and_update_panes(&ui, &mut st, pos);
                        ui.set_rename_text(candidate.into());
                        ui.set_is_renaming(true);
                    }
                }
            }
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
            let st = state.borrow();
            if let Some(idx) = st.selected_idx {
                if let Some(entry) = st.entries.get(idx) {
                    ui.set_rename_text(entry.name.clone().into());
                    ui.set_is_renaming(true);
                }
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
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_view(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let mut st = state.borrow_mut();
            if st.view_mode == "details" {
                st.view_mode = "icons".to_string();
            } else {
                st.view_mode = "details".to_string();
            }
            ui.set_view_mode(st.view_mode.clone().into());
        });
    }

    {
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_preview(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let mut st = state.borrow_mut();
            st.show_preview = !st.show_preview;
            if st.show_preview {
                st.show_details = false;
                ui.set_show_details(false);
            }
            ui.set_show_preview(st.show_preview);
            if let Some(sel) = st.selected_idx {
                select_entry_and_update_panes(&ui, &mut st, sel);
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();

        main_window.on_command_toggle_details(move || {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let mut st = state.borrow_mut();
            st.show_details = !st.show_details;
            if st.show_details {
                st.show_preview = false;
                ui.set_show_preview(false);
            }
            ui.set_show_details(st.show_details);
            if let Some(sel) = st.selected_idx {
                select_entry_and_update_panes(&ui, &mut st, sel);
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
            let mut st = state.borrow_mut();
            let c = col.as_str();
            if st.sort_column == c {
                st.sort_asc = !st.sort_asc;
            } else {
                st.sort_column = c.to_string();
                st.sort_asc = true;
            }
            ui.set_sort_column(st.sort_column.clone().into());
            ui.set_sort_asc(st.sort_asc);
            drop(st);
            update_ui();
        });
    }

    // 12. Keyboard Arrow Navigation
    {
        let state = Rc::clone(&state);
        let main_window_weak = main_window.as_weak();

        main_window.on_key_shortcut(move |action| {
            let ui = match main_window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let mut st = state.borrow_mut();
            let len = st.entries.len();
            if len == 0 {
                return;
            }
            let next_idx = match (action.as_str(), st.selected_idx) {
                ("down", Some(curr)) => (curr + 1).min(len - 1),
                ("down", None) => 0,
                ("up", Some(curr)) => curr.saturating_sub(1),
                ("up", None) => 0,
                _ => return,
            };
            select_entry_and_update_panes(&ui, &mut st, next_idx);
        });
    }

    // 13. Tab Management (Select, New Tab +, Close Tab)
    {
        let state = Rc::clone(&state);
        let update_ui = update_ui.clone();
        let main_window_weak = main_window.as_weak();

        main_window.on_tab_selected(move |idx| {
            let i = idx as usize;
            let mut st = state.borrow_mut();
            if i < st.tabs.len() && i != st.active_tab_idx {
                st.active_tab_idx = i;
                st.current_path = st.tabs[i].path.clone();
                st.history_back = st.tabs[i].history_back.clone();
                st.history_forward = st.tabs[i].history_forward.clone();
                drop(st);
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
            let mut st = state.borrow_mut();
            let new_id = st.next_tab_id;
            st.next_tab_id += 1;
            let path = st.current_path.clone();
            let title = path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "Home".to_string());
            st.tabs.push(TabState {
                id: new_id,
                title,
                path: path.clone(),
                history_back: Vec::new(),
                history_forward: Vec::new(),
            });
            st.active_tab_idx = st.tabs.len() - 1;
            st.history_back.clear();
            st.history_forward.clear();
            drop(st);
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
            let mut st = state.borrow_mut();
            if st.tabs.len() > 1 && i < st.tabs.len() {
                st.tabs.remove(i);
                if st.active_tab_idx >= st.tabs.len() {
                    st.active_tab_idx = st.tabs.len() - 1;
                }
                let active = st.active_tab_idx;
                st.current_path = st.tabs[active].path.clone();
                st.history_back = st.tabs[active].history_back.clone();
                st.history_forward = st.tabs[active].history_forward.clone();
                drop(st);
                update_ui();
            }
        });
    }

    // Run the native Windows application loop
    main_window.run()
}
