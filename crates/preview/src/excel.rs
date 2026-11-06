use std::path::Path;
use anyhow::{Result, anyhow};
use calamine::{Reader, open_workbook_auto};
use ipc_types::Preview;

pub fn preview_excel(path: &str) -> Result<Preview> {
    let ext = fs_core::extension_of(path);
    if ext == "csv" || ext == "tsv" {
        return preview_csv(path, if ext == "tsv" { b'\t' } else { b',' });
    }

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| anyhow!("Failed to open workbook: {}", e))?;

    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Ok(Preview::Sheet {
            sheets: vec![],
            active: 0,
            rows: vec![],
            truncated: false,
        });
    }

    let active_sheet_name = sheet_names[0].clone();
    let mut rows_data = Vec::new();
    let mut truncated = false;

    if let Ok(range) = workbook.worksheet_range(&active_sheet_name) {
        let max_rows = 300;
        let max_cols = 40;

        for (r_idx, row) in range.rows().enumerate() {
            if r_idx >= max_rows {
                truncated = true;
                break;
            }
            let mut row_strs = Vec::with_capacity(row.len().min(max_cols));
            for (c_idx, cell) in row.iter().enumerate() {
                if c_idx >= max_cols {
                    break;
                }
                row_strs.push(cell.to_string());
            }
            rows_data.push(row_strs);
        }
    }

    Ok(Preview::Sheet {
        sheets: sheet_names,
        active: 0,
        rows: rows_data,
        truncated,
    })
}

fn preview_csv(path: &str, delimiter: u8) -> Result<Preview> {
    let content = std::fs::read_to_string(path)?;
    let mut rows_data = Vec::new();
    let max_rows = 500;
    let mut truncated = false;

    for (r_idx, line) in content.lines().enumerate() {
        if r_idx >= max_rows {
            truncated = true;
            break;
        }
        let cols: Vec<String> = if delimiter == b'\t' {
            line.split('\t').map(|s| s.trim().to_string()).collect()
        } else {
            // Simple comma split
            line.split(',').map(|s| s.trim().to_string()).collect()
        };
        rows_data.push(cols);
    }

    Ok(Preview::Sheet {
        sheets: vec!["Data".into()],
        active: 0,
        rows: rows_data,
        truncated,
    })
}
