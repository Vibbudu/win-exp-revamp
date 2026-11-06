pub mod arena;
pub mod crawl;
pub mod mft;
pub mod query;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use parking_lot::{Mutex, RwLock};
use ipc_types::{Entry, IndexStatus};
use crate::arena::CompactIndex;

#[derive(Clone)]
pub struct IndexerEngine {
    index: Arc<RwLock<CompactIndex>>,
    status: Arc<Mutex<IndexStatus>>,
    stop_signal: Arc<AtomicBool>,
}

impl IndexerEngine {
    pub fn new() -> Self {
        Self {
            index: Arc::new(RwLock::new(CompactIndex::new())),
            status: Arc::new(Mutex::new(IndexStatus {
                state: "idle".into(),
                mode: "none".into(),
                files: 0,
                elapsed_ms: 0.0,
                volumes: Vec::new(),
                message: "Ready to index".into(),
            })),
            stop_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn status(&self) -> IndexStatus {
        self.status.lock().clone()
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Entry> {
        self.index.read().search(query, limit)
    }

    /// Instant combined search:
    /// 1. Searches the active directory tree (BFS, prioritizing non-hidden user folders).
    /// 2. Searches the global in-memory index (which has Known Folders indexed in <100ms).
    /// 3. Deduplicates by case-insensitive path.
    pub fn search_combined(&self, current_dir: &str, query: &str, limit: usize) -> Vec<Entry> {
        let mut results = Vec::with_capacity(limit);
        let mut seen = HashSet::new();

        // 1. Live BFS search of current directory & subfolders
        if !current_dir.is_empty() && std::path::Path::new(current_dir).exists() {
            let local_matches = crawl::live_search_directory(current_dir, query, limit, 1200);
            for item in local_matches {
                let key = item.path.to_lowercase();
                if seen.insert(key) {
                    results.push(item);
                    if results.len() >= limit {
                        return results;
                    }
                }
            }
        }

        // 2. Search the in-memory global index
        let global_matches = self.index.read().search(query, limit);
        for item in global_matches {
            let key = item.path.to_lowercase();
            if seen.insert(key) {
                results.push(item);
                if results.len() >= limit {
                    break;
                }
            }
        }

        results
    }

    pub fn start_background_indexing(&self) {
        let index_arc = Arc::clone(&self.index);
        let status_arc = Arc::clone(&self.status);
        let stop_signal = Arc::clone(&self.stop_signal);

        std::thread::Builder::new()
            .name("indexer-worker".into())
            .spawn(move || {
                let start_time = Instant::now();
                {
                    let mut st = status_arc.lock();
                    st.state = "building".into();
                    st.message = "Indexing user folders...".into();
                }

                // PHASE 1: Immediately index Known Folders (Desktop, Downloads, Documents, Pictures, Music, Videos)
                // so user searches return results within the first 100ms!
                let mut priority_index = CompactIndex::with_capacity(100_000);
                let mut priority_count = 0;

                let known_folders = shell_interop::info::known_folders();
                for kf in &known_folders {
                    if stop_signal.load(Ordering::Relaxed) {
                        return;
                    }
                    priority_count += crawl::crawl_directory_tree(
                        &kf.path,
                        &mut priority_index,
                        &stop_signal,
                        50_000,
                    );
                }

                // Publish priority index immediately!
                *index_arc.write() = priority_index;
                {
                    let mut st = status_arc.lock();
                    st.files = priority_count as u64;
                    st.elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                    st.message = format!("Indexed {} user files", priority_count);
                }

                // PHASE 2: Full volume indexing (MFT if elevated, or priority BFS crawl)
                let drives = shell_interop::info::drives();
                let mut vol_names = Vec::new();
                for d in &drives {
                    if d.kind == "fixed" || d.kind == "removable" {
                        vol_names.push(d.path.clone());
                    }
                }

                {
                    let mut st = status_arc.lock();
                    st.volumes = vol_names.clone();
                }

                let mut full_index = CompactIndex::with_capacity(600_000);
                // Re-include known folders at the front of full_index
                for kf in &known_folders {
                    let _ = crawl::crawl_directory_tree(
                        &kf.path,
                        &mut full_index,
                        &stop_signal,
                        50_000,
                    );
                }

                let is_elevated = shell_interop::system::is_elevated();
                let mut total_indexed = full_index.len();
                let mut used_mode = "crawl".to_string();

                for vol in &vol_names {
                    if stop_signal.load(Ordering::Relaxed) {
                        break;
                    }

                    let drive_char = vol.chars().next().unwrap_or('C').to_ascii_uppercase();
                    let mut indexed_mft = false;

                    if is_elevated {
                        if let Ok(count) = mft::index_volume_mft(drive_char, &mut full_index) {
                            total_indexed += count;
                            indexed_mft = true;
                            used_mode = "mft".into();
                        }
                    }

                    if !indexed_mft {
                        let count = crawl::crawl_directory_tree(
                            vol,
                            &mut full_index,
                            &stop_signal,
                            500_000,
                        );
                        total_indexed += count;
                    }
                }

                let elapsed = start_time.elapsed();
                *index_arc.write() = full_index;

                {
                    let mut st = status_arc.lock();
                    st.state = "ready".into();
                    st.mode = used_mode;
                    st.files = total_indexed as u64;
                    st.elapsed_ms = elapsed.as_secs_f64() * 1000.0;
                    st.message = format!(
                        "Indexed {} files in {:.2}s",
                        total_indexed,
                        elapsed.as_secs_f64()
                    );
                }
            })
            .expect("failed to spawn indexer worker");
    }
}
