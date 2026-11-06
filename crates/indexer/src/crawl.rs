use std::sync::atomic::{AtomicUsize, Ordering};
use ipc_types::ATTR_DIRECTORY;
use crate::arena::CompactIndex;

pub fn crawl_directory_tree(
    root_path: &str,
    index: &mut CompactIndex,
    stop_signal: &std::sync::atomic::AtomicBool,
) -> usize {
    let mut clean_root = root_path.trim_end_matches(['\\', '/']).to_string();
    if clean_root.len() == 2 && clean_root.ends_with(':') {
        clean_root.push('\\');
    }
    let root_id = index.add_entry(&clean_root, u32::MAX, 0, 0, ATTR_DIRECTORY);

    // Queue of (path_u16_with_trailing_slash, parent_idx)
    let mut queue = Vec::new();
    let mut root_u16: Vec<u16> = clean_root.encode_utf16().collect();
    if !root_u16.ends_with(&['\\' as u16]) {
        root_u16.push('\\' as u16);
    }
    queue.push((root_u16, root_id));

    let mut count = 0;

    while let Some((dir_u16, parent_id)) = queue.pop() {
        if stop_signal.load(Ordering::Relaxed) {
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
                // Skip system/hidden junctions like "C:\Documents and Settings", "$RECYCLE.BIN"
                let skip = name.starts_with('$')
                    || name.eq_ignore_ascii_case("System Volume Information")
                    || (entry.attrs & 0x400 != 0); // Reparse point / junction

                if !skip {
                    let mut child_u16 = Vec::with_capacity(dir_u16.len() + entry.name.len() + 1);
                    child_u16.extend_from_slice(&dir_u16);
                    child_u16.extend_from_slice(entry.name);
                    child_u16.push('\\' as u16);
                    queue.push((child_u16, id));
                }
            }
        });
    }

    count
}
