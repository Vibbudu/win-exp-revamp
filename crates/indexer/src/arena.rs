use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use ipc_types::{ATTR_DIRECTORY, Entry};
use memchr::memmem::Finder;
use rayon::prelude::*;
use crate::query::QueryFilter;

#[derive(Default)]
pub struct CompactIndex {
    pub names_arena: Vec<u8>,
    pub names_lower: Vec<u8>,
    pub offsets: Vec<u32>,
    pub lengths: Vec<u16>,
    pub parents: Vec<u32>,
    pub sizes: Vec<u64>,
    pub modified: Vec<i64>,
    pub attrs: Vec<u32>,
    pub ext_ids: Vec<u16>,
    pub ext_table: Vec<String>,
    pub ext_map: HashMap<String, u16>,
    pub root_indices: Vec<(String, u32)>, // (Root prefix e.g. "C:\", root index)
}

impl CompactIndex {
    pub fn new() -> Self {
        let mut idx = Self::default();
        // Index 0 in ext_table is empty extension ""
        idx.ext_table.push(String::new());
        idx.ext_map.insert(String::new(), 0);
        idx
    }

    pub fn with_capacity(est_entries: usize) -> Self {
        let mut idx = Self::new();
        idx.names_arena.reserve(est_entries * 16);
        idx.names_lower.reserve(est_entries * 16);
        idx.offsets.reserve(est_entries);
        idx.lengths.reserve(est_entries);
        idx.parents.reserve(est_entries);
        idx.sizes.reserve(est_entries);
        idx.modified.reserve(est_entries);
        idx.attrs.reserve(est_entries);
        idx.ext_ids.reserve(est_entries);
        idx
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn intern_ext(&mut self, ext: &str) -> u16 {
        let lower = ext.to_ascii_lowercase();
        if let Some(&id) = self.ext_map.get(&lower) {
            id
        } else {
            let id = self.ext_table.len() as u16;
            self.ext_table.push(lower.clone());
            self.ext_map.insert(lower, id);
            id
        }
    }

    pub fn add_entry(
        &mut self,
        name: &str,
        parent: u32,
        size: u64,
        modified: i64,
        attrs: u32,
    ) -> u32 {
        let id = self.offsets.len() as u32;
        let offset = self.names_arena.len() as u32;
        let bytes = name.as_bytes();
        let len = bytes.len().min(u16::MAX as usize) as u16;

        self.names_arena.extend_from_slice(&bytes[..len as usize]);
        // Also keep lowercased bytes for fast SIMD case-insensitive substring search
        let lower = name.to_lowercase();
        self.names_lower.extend_from_slice(lower.as_bytes());

        self.offsets.push(offset);
        self.lengths.push(len);
        self.parents.push(parent);
        self.sizes.push(size);
        self.modified.push(modified);
        self.attrs.push(attrs);

        let ext = if attrs & ATTR_DIRECTORY != 0 {
            ""
        } else {
            match name.rfind('.') {
                Some(i) if i > 0 && i + 1 < name.len() => &name[i + 1..],
                _ => "",
            }
        };
        let ext_id = self.intern_ext(ext);
        self.ext_ids.push(ext_id);

        id
    }

    pub fn get_name(&self, idx: u32) -> &str {
        let idx = idx as usize;
        if idx >= self.offsets.len() {
            return "";
        }
        let off = self.offsets[idx] as usize;
        let len = self.lengths[idx] as usize;
        std::str::from_utf8(&self.names_arena[off..off + len]).unwrap_or("")
    }

    pub fn reconstruct_path(&self, mut idx: u32) -> String {
        let mut segments = Vec::new();
        while idx != u32::MAX && (idx as usize) < self.offsets.len() {
            let name = self.get_name(idx);
            segments.push(name);
            idx = self.parents[idx as usize];
        }
        segments.reverse();

        let mut path = String::new();
        for (i, seg) in segments.iter().enumerate() {
            if i > 0 && !path.ends_with('\\') && !path.ends_with('/') {
                path.push('\\');
            }
            path.push_str(seg);
        }
        path
    }

    pub fn to_entry(&self, idx: u32) -> Entry {
        let i = idx as usize;
        let name = self.get_name(idx).to_string();
        let path = self.reconstruct_path(idx);
        let is_dir = (self.attrs[i] & ATTR_DIRECTORY) != 0;
        let ext_id = self.ext_ids[i] as usize;
        let ext = if ext_id < self.ext_table.len() {
            self.ext_table[ext_id].clone()
        } else {
            String::new()
        };

        Entry {
            name,
            path,
            is_dir,
            size: self.sizes[i],
            modified: self.modified[i],
            created: 0,
            attrs: self.attrs[i],
            type_name: String::new(),
            ext,
        }
    }

    pub fn search(&self, query_str: &str, limit: usize) -> Vec<Entry> {
        let filter = QueryFilter::parse(query_str);
        if filter.terms.is_empty() && filter.ext.is_none() && filter.min_size.is_none() && filter.max_size.is_none() && filter.is_dir.is_none() && filter.path_prefix.is_none() {
            return Vec::new();
        }

        let total = self.offsets.len();
        if total == 0 {
            return Vec::new();
        }

        let finders: Vec<Finder> = filter.terms.iter().map(|t| Finder::new(t.as_bytes())).collect();

        // Check extensions filter
        let target_ext_ids: Option<Vec<u16>> = filter.ext.as_ref().map(|exts| {
            exts.iter().filter_map(|e| self.ext_map.get(e).copied()).collect()
        });

        // Parallel chunk search
        let chunk_size = 32768;
        let num_chunks = (total + chunk_size - 1) / chunk_size;

        let matched_chunks: Vec<Vec<u32>> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| {
                let start = chunk_idx * chunk_size;
                let end = (start + chunk_size).min(total);
                let mut local = Vec::new();

                for i in start..end {
                    let attrs = self.attrs[i];
                    let is_dir = (attrs & ATTR_DIRECTORY) != 0;

                    // Filter is_dir
                    if let Some(want_dir) = filter.is_dir {
                        if is_dir != want_dir {
                            continue;
                        }
                    }

                    // Filter size: if size filter is specified, directories do not match
                    if filter.min_size.is_some() || filter.max_size.is_some() {
                        if is_dir {
                            continue;
                        }
                        let size = self.sizes[i];
                        if let Some(min) = filter.min_size {
                            if size < min {
                                continue;
                            }
                        }
                        if let Some(max) = filter.max_size {
                            if size > max {
                                continue;
                            }
                        }
                    }

                    // Filter extension
                    if let Some(ref target_ids) = target_ext_ids {
                        let eid = self.ext_ids[i];
                        if !target_ids.contains(&eid) {
                            continue;
                        }
                    }

                    // Match all terms against lower name
                    let off = self.offsets[i] as usize;
                    let len = self.lengths[i] as usize;
                    let name_lower = &self.names_lower[off..off + len];

                    let mut matches_all = true;
                    for finder in &finders {
                        if finder.find(name_lower).is_none() {
                            matches_all = false;
                            break;
                        }
                    }

                    if matches_all {
                        local.push(i as u32);
                        if local.len() >= limit {
                            break;
                        }
                    }
                }
                local
            })
            .collect();

        let matched_indices: Vec<u32> = matched_chunks
            .into_iter()
            .flatten()
            .take(limit)
            .collect();

        // Check path prefix if specified
        let mut results = Vec::with_capacity(matched_indices.len().min(limit));
        for idx in matched_indices {
            let mut entry = self.to_entry(idx);
            if let Some(ref prefix) = filter.path_prefix {
                if !entry.path.to_lowercase().starts_with(prefix) {
                    continue;
                }
            }
            results.push(entry);
            if results.len() >= limit {
                break;
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_index_add_and_search() {
        let mut idx = CompactIndex::new();
        let root = idx.add_entry("C:\\", u32::MAX, 0, 0, ATTR_DIRECTORY);
        let docs = idx.add_entry("Documents", root, 0, 0, ATTR_DIRECTORY);
        let f1 = idx.add_entry("Financial_Report_2026.xlsx", docs, 45000, 1000, 0);
        let f2 = idx.add_entry("budget_draft.xlsx", docs, 12000, 2000, 0);
        let f3 = idx.add_entry("profile_picture.png", docs, 850000, 3000, 0);

        assert_eq!(idx.reconstruct_path(f1), "C:\\Documents\\Financial_Report_2026.xlsx");
        assert_eq!(idx.reconstruct_path(f3), "C:\\Documents\\profile_picture.png");

        // Search by query
        let res1 = idx.search("report", 10);
        assert_eq!(res1.len(), 1);
        assert_eq!(res1[0].name, "Financial_Report_2026.xlsx");

        // Search by extension filter
        let res2 = idx.search("ext:xlsx", 10);
        assert_eq!(res2.len(), 2);

        // Search by size filter
        let res3 = idx.search("size:>50kb", 10);
        assert_eq!(res3.len(), 1);
        assert_eq!(res3[0].name, "profile_picture.png");
    }
}
