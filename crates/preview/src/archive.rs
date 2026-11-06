use std::fs::File;
use anyhow::{Result, anyhow};
use ipc_types::{ArchiveEntry, Preview};
use zip::ZipArchive;

pub fn preview_zip(path: &str) -> Result<Preview> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let total = archive.len();

    let mut entries = Vec::with_capacity(total.min(500));
    for i in 0..total.min(500) {
        if let Ok(entry) = archive.by_index(i) {
            let name = entry.name().map(|n| n.to_string()).unwrap_or_default();
            entries.push(ArchiveEntry {
                name,
                size: entry.size(),
                compressed: entry.compressed_size(),
                is_dir: entry.is_dir(),
            });
        }
    }

    Ok(Preview::Archive { entries, total })
}
