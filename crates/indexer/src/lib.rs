pub mod arena;
pub mod crawl;
pub mod mft;
pub mod query;

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
                    st.message = "Detecting drives...".into();
                }

                // Get drive roots using shell_interop::info::drives()
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

                let mut new_index = CompactIndex::with_capacity(500_000);
                let is_elevated = shell_interop::system::is_elevated();
                let mut total_indexed = 0;
                let mut used_mode = "crawl".to_string();

                for vol in &vol_names {
                    if stop_signal.load(Ordering::Relaxed) {
                        break;
                    }

                    let drive_char = vol.chars().next().unwrap_or('C').to_ascii_uppercase();
                    let mut indexed_mft = false;

                    if is_elevated {
                        {
                            let mut st = status_arc.lock();
                            st.message = format!("Reading NTFS MFT on {}:...", drive_char);
                        }
                        match mft::index_volume_mft(drive_char, &mut new_index) {
                            Ok(count) => {
                                total_indexed += count;
                                indexed_mft = true;
                                used_mode = "mft".into();
                            }
                            Err(e) => {
                                log::warn!("MFT failed for {}: {}. Falling back to crawl.", drive_char, e);
                            }
                        }
                    }

                    if !indexed_mft {
                        {
                            let mut st = status_arc.lock();
                            st.message = format!("Scanning {}...", vol);
                        }
                        let count = crawl::crawl_directory_tree(vol, &mut new_index, &stop_signal);
                        total_indexed += count;
                    }

                    {
                        let mut st = status_arc.lock();
                        st.files = total_indexed as u64;
                        st.elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                    }
                }

                let elapsed = start_time.elapsed();
                *index_arc.write() = new_index;

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
