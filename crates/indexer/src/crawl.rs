use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use ipc_types::{ATTR_DIRECTORY, ATTR_HIDDEN, Entry};
use memchr::memmem::Finder;
use crate::arena::CompactIndex;
use crate::query::QueryFilter;

fn is_low_priority_dir(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('$')
        || name.eq_ignore_ascii_case("AppData")
        || name.eq_ignore_ascii_case("node_modules")
        || name.eq_ignore_ascii_case("target")
        || name.eq_ignore_ascii_case("System Volume Information")
        || name.eq_ignore_ascii_case("Windows")
        || name.eq_ignore_ascii_case("WinSxS")
        || name.eq_ignore_ascii_case("ProgramData")
}

/// Fast BFS directory crawler that indexes normal user directories first
/// and defers hidden/cache/system directories so user files are indexed in <100ms.
pub fn crawl_directory_tree(
    root_path: &str,
    index: &mut CompactIndex,
    stop_signal: &std::sync::atomic::AtomicBool,
    max_entries: usize,
) -> usize {
    let mut clean_root = root_path.trim_end_matches(['\\', '/']).to_string();
    if clean_root.len() == 2 && clean_root.ends_with(':') {
        clean_root.push('\\');
    }
    if !std::path::Path::new(&clean_root).exists() {
        return 0;
    }

    let root_id = index.add_entry(&clean_root, u32::MAX, 0, 0, ATTR_DIRECTORY);

    let mut high_queue = VecDeque::new();
    let mut low_queue = VecDeque::new();

    let mut root_u16: Vec<u16> = clean_root.encode_utf16().collect();
    if !root_u16.ends_with(&['\\' as u16]) {
        root_u16.push('\\' as u16);
    }
    high_queue.push_back((root_u16, root_id));

    let mut count = 0;

    while let Some((dir_u16, parent_id)) = high_queue.pop_front().or_else(|| low_queue.pop_front()) {
        if stop_signal.load(Ordering::Relaxed) || count >= max_entries {
            break;
        }

        fs_core::raw::enumerate_raw(&dir_u16, |entry| {
            let name = String::from_utf16_lossy(entry.name);
            let is_dir = entry.attrs & ATTR_DIRECTORY != 0;
            let id = index.add_entry(
                &name,
                parent_id,
                entry.size,
                entry.modified,
                entry.attrs,
            );
            count += 1;

            if is_dir {
                // Always skip reparse points / junctions to avoid infinite loops
                let is_reparse = (entry.attrs & 0x400) != 0;
                let is_sys_vol = name.starts_with('$') || name.eq_ignore_ascii_case("System Volume Information");

                if !is_reparse && !is_sys_vol {
                    let mut child_u16 = Vec::with_capacity(dir_u16.len() + entry.name.len() + 1);
                    child_u16.extend_from_slice(&dir_u16);
                    child_u16.extend_from_slice(entry.name);
                    child_u16.push('\\' as u16);

                    if is_low_priority_dir(&name) || (entry.attrs & ATTR_HIDDEN != 0) {
                        low_queue.push_back((child_u16, id));
                    } else {
                        high_queue.push_back((child_u16, id));
                    }
                }
            }
        });
    }

    count
}

/// Instant live BFS search starting from `root_path` (for current folder + subfolders).
/// Prioritizes user-visible folders first so results in Documents/Downloads appear in <5ms.
pub fn live_search_directory(
    root_path: &str,
    query: &str,
    limit: usize,
    max_dirs_scanned: usize,
) -> Vec<Entry> {
    let filter = QueryFilter::parse(query);
    if filter.terms.is_empty()
        && filter.ext.is_none()
        && filter.min_size.is_none()
        && filter.max_size.is_none()
        && filter.is_dir.is_none()
    {
        return Vec::new();
    }

    let finders: Vec<Finder> = filter
        .terms
        .iter()
        .map(|t| Finder::new(t.as_bytes()))
        .collect();

    let mut clean_root = root_path.trim_end_matches(['\\', '/']).to_string();
    if clean_root.len() == 2 && clean_root.ends_with(':') {
        clean_root.push('\\');
    }

    let mut high_queue = VecDeque::new();
    let mut low_queue = VecDeque::new();

    let mut root_u16: Vec<u16> = clean_root.encode_utf16().collect();
    if !root_u16.ends_with(&['\\' as u16]) {
        root_u16.push('\\' as u16);
    }
    high_queue.push_back((root_u16, clean_root));

    let mut results = Vec::new();
    let mut dirs_scanned = 0;

    while let Some((dir_u16, dir_str)) = high_queue.pop_front().or_else(|| low_queue.pop_front()) {
        if results.len() >= limit || dirs_scanned >= max_dirs_scanned {
            break;
        }
        dirs_scanned += 1;

        fs_core::raw::enumerate_raw(&dir_u16, |entry| {
            let name = String::from_utf16_lossy(entry.name);
            let is_dir = (entry.attrs & ATTR_DIRECTORY) != 0;

            let full_path = if dir_str.ends_with('\\') {
                format!("{dir_str}{name}")
            } else {
                format!("{dir_str}\\{name}")
            };

            if is_dir {
                let is_reparse = (entry.attrs & 0x400) != 0;
                let is_sys = name.starts_with('$') || name.eq_ignore_ascii_case("System Volume Information");
                if !is_reparse && !is_sys {
                    let mut child_u16 = Vec::with_capacity(dir_u16.len() + entry.name.len() + 1);
                    child_u16.extend_from_slice(&dir_u16);
                    child_u16.extend_from_slice(entry.name);
                    child_u16.push('\\' as u16);

                    if is_low_priority_dir(&name) || (entry.attrs & ATTR_HIDDEN != 0) {
                        low_queue.push_back((child_u16, full_path.clone()));
                    } else {
                        high_queue.push_back((child_u16, full_path.clone()));
                    }
                }
            }

            if results.len() >= limit {
                return;
            }

            // Check filter criteria
            if let Some(want_dir) = filter.is_dir {
                if is_dir != want_dir {
                    return;
                }
            }

            if filter.min_size.is_some() || filter.max_size.is_some() {
                if is_dir {
                    return;
                }
                if let Some(min) = filter.min_size {
                    if entry.size < min {
                        return;
                    }
                }
                if let Some(max) = filter.max_size {
                    if entry.size > max {
                        return;
                    }
                }
            }

            let ext = if is_dir {
                String::new()
            } else {
                fs_core::extension_of(&name)
            };

            if let Some(ref exts) = filter.ext {
                if !exts.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
                    return;
                }
            }

            let name_lower = name.to_lowercase();
            let name_bytes = name_lower.as_bytes();
            let mut matches_all = true;
            for finder in &finders {
                if finder.find(name_bytes).is_none() {
                    matches_all = false;
                    break;
                }
            }

            if matches_all {
                results.push(Entry {
                    name,
                    path: full_path,
                    is_dir,
                    size: if is_dir { 0 } else { entry.size },
                    modified: entry.modified,
                    created: entry.modified,
                    attrs: entry.attrs,
                    type_name: String::new(),
                    ext,
                });
            }
        });
    }

    results
}
