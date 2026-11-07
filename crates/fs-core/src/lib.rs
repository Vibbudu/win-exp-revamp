//! fs-core: the hot path for opening folders.
//!
//! * [`list_dir`] enumerates a directory with a single `FindFirstFileExW` stream using
//!   `FindExInfoBasic` + `FIND_FIRST_EX_LARGE_FETCH` (no 8.3 names, 64 KiB batches), which
//!   returns name/size/times/attributes without any per-file `stat`.
//! * [`natural_cmp`] reproduces Explorer's `StrCmpLogicalW` ordering in pure Rust.
//! * [`sort_entries`] sorts with rayon for large folders.

pub mod natural;
pub mod raw;
mod win;

pub use natural::natural_cmp;
pub use raw::{RawEntry, enumerate_raw};
pub use win::{calculate_folder_size_fast, filetime_to_unix_ms, list_dir, stat_path};

use ipc_types::{Entry, SortKey, SortSpec};
use rayon::prelude::*;
use std::cmp::Ordering;

/// Sort entries the way Explorer does: folders first for ascending sorts, the whole order is
/// reversed for descending sorts, ties broken by natural name order.
pub fn sort_entries(entries: &mut [Entry], spec: SortSpec) {
    let cmp = |a: &Entry, b: &Entry| -> Ordering {
        let dirs = b.is_dir.cmp(&a.is_dir);
        let primary = match spec.key {
            SortKey::Name => dirs.then_with(|| natural_cmp(&a.name, &b.name)),
            SortKey::Modified => dirs.then(a.modified.cmp(&b.modified)),
            SortKey::Created => dirs.then(a.created.cmp(&b.created)),
            SortKey::Type => dirs
                .then_with(|| a.type_name.to_lowercase().cmp(&b.type_name.to_lowercase())),
            SortKey::Size => dirs.then(a.size.cmp(&b.size)),
            SortKey::Path => natural_cmp(&a.path, &b.path),
        };
        let ord = primary.then_with(|| natural_cmp(&a.name, &b.name));
        if spec.desc { ord.reverse() } else { ord }
    };
    if entries.len() > 4096 {
        entries.par_sort_unstable_by(cmp);
    } else {
        entries.sort_unstable_by(cmp);
    }
}

/// Lower-case extension (without dot) of a file name; "" for none.
pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, is_dir: bool, size: u64) -> Entry {
        Entry { name: name.into(), is_dir, size, ..Default::default() }
    }

    #[test]
    fn folders_first_and_natural() {
        let mut v = vec![e("file10.txt", false, 1), e("b", true, 0), e("file2.txt", false, 5), e("A", true, 0)];
        sort_entries(&mut v, SortSpec { key: SortKey::Name, desc: false });
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["A", "b", "file2.txt", "file10.txt"]);
        sort_entries(&mut v, SortSpec { key: SortKey::Name, desc: true });
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["file10.txt", "file2.txt", "b", "A"]);
    }

    #[test]
    fn extensions() {
        assert_eq!(extension_of("Report.XLSX"), "xlsx");
        assert_eq!(extension_of(".gitignore"), "");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("a.tar.gz"), "gz");
    }

    #[test]
    fn lists_windows_dir_quickly() {
        let t = std::time::Instant::now();
        let v = list_dir(r"C:\Windows\System32").expect("list");
        assert!(v.len() > 1000);
        assert!(v.iter().any(|e| e.name.eq_ignore_ascii_case("kernel32.dll") && e.size > 0));
        eprintln!("listed {} entries in {:?}", v.len(), t.elapsed());
    }
}
