pub mod archive;
pub mod docx;
pub mod excel;
pub mod pptx;
pub mod text;

use std::path::Path;
use anyhow::Result;
use ipc_types::Preview;

pub fn generate_preview(path: &str) -> Preview {
    let p = Path::new(path);
    if !p.exists() {
        return Preview::None {
            reason: "File does not exist".into(),
        };
    }

    if p.is_dir() {
        let count = match fs_core::list_dir(path) {
            Ok(entries) => entries.len() as u64,
            Err(_) => 0,
        };
        return Preview::Folder { items: count };
    }

    let ext = fs_core::extension_of(path);

    // 1. Image formats (loaded directly by browser via asset protocol or data URL)
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "avif"
    ) {
        return Preview::Image;
    }

    // 2. PDF
    if ext == "pdf" {
        return Preview::Pdf;
    }

    // 3. Excel & Spreadsheets
    if matches!(ext.as_str(), "xlsx" | "xls" | "xlsb" | "ods" | "csv" | "tsv") {
        if let Ok(res) = excel::preview_excel(path) {
            return res;
        }
    }

    // 4. Word Documents
    if ext == "docx" {
        if let Ok(res) = docx::preview_docx(path) {
            return res;
        }
    }

    // 5. PowerPoint Presentations
    if ext == "pptx" {
        if let Ok(res) = pptx::preview_pptx(path) {
            return res;
        }
    }

    // 6. Archives
    if matches!(ext.as_str(), "zip" | "jar" | "apk") {
        if let Ok(res) = archive::preview_zip(path) {
            return res;
        }
    }

    // 7. Video
    if matches!(ext.as_str(), "mp4" | "webm" | "mkv" | "mov" | "avi" | "wmv") {
        return Preview::Video;
    }

    // 8. Audio
    if matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "flac" | "m4a" | "wma" | "aac") {
        return Preview::Audio;
    }

    // 9. Web pages
    if ext == "html" || ext == "htm" {
        if let Ok(html) = std::fs::read_to_string(path) {
            return Preview::WebPage { html };
        }
    }

    // 10. Code & Text & Markdown
    if is_text_or_code(&ext) {
        if let Ok(res) = text::preview_text_or_code(path, &ext) {
            return res;
        }
    }

    // 11. Fallback: check if shell has a thumbnail
    Preview::Thumbnail {
        note: format!("Preview for .{} file", ext.to_uppercase()),
    }
}

fn is_text_or_code(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "mts"
            | "cts"
            | "jsx"
            | "tsx"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "xml"
            | "css"
            | "scss"
            | "less"
            | "py"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "cxx"
            | "cs"
            | "go"
            | "sh"
            | "bash"
            | "zsh"
            | "ps1"
            | "psm1"
            | "bat"
            | "cmd"
            | "sql"
            | "ini"
            | "cfg"
            | "conf"
            | "log"
            | "env"
            | "gitignore"
            | "lock"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_and_code_detection() {
        assert!(is_text_or_code("rs"));
        assert!(is_text_or_code("ts"));
        assert!(is_text_or_code("json"));
        assert!(is_text_or_code("md"));
        assert!(!is_text_or_code("exe"));
        assert!(!is_text_or_code("dll"));
    }

    #[test]
    fn test_markdown_preview_generation() {
        let tmp = std::env::temp_dir().join("hyperion_test.md");
        std::fs::write(&tmp, "# Test Heading\n\nParagraph text with **bold**.").unwrap();
        let prev = generate_preview(tmp.to_str().unwrap());
        if let Preview::Html { html } = prev {
            assert!(html.contains("<h1>Test Heading</h1>"));
            assert!(html.contains("<strong>bold</strong>"));
        } else {
            panic!("Expected Html preview for markdown");
        }
        let _ = std::fs::remove_file(tmp);
    }
}
