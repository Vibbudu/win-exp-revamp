use anyhow::Result;
use ipc_types::Preview;
use pulldown_cmark::{Parser, Options, html};

pub fn preview_text_or_code(path: &str, ext: &str) -> Result<Preview> {
    if ext == "md" || ext == "markdown" {
        let content = std::fs::read_to_string(path)?;
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TASKLISTS);
        let parser = Parser::new_ext(&content, options);
        let mut html_output = String::new();
        html::push_html(&mut html_output, parser);
        return Ok(Preview::Html {
            html: format!("<div class=\"markdown-body\">{}</div>", html_output),
        });
    }

    // Read up to 256KB
    let metadata = std::fs::metadata(path)?;
    let file_size = metadata.len();
    let max_read = 256 * 1024;
    let truncated = file_size > max_read as u64;

    let bytes = std::fs::read(path)?;
    let slice = if bytes.len() > max_read {
        &bytes[..max_read]
    } else {
        &bytes[..]
    };

    let content = String::from_utf8_lossy(slice).to_string();
    let language = detect_language(ext);

    Ok(Preview::Text {
        content,
        language,
        truncated,
    })
}

fn detect_language(ext: &str) -> String {
    match ext {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        "py" => "python",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "cxx" => "cpp",
        "cs" => "csharp",
        "go" => "go",
        "sh" | "bash" | "zsh" => "bash",
        "ps1" | "psm1" => "powershell",
        "bat" | "cmd" => "batch",
        "sql" => "sql",
        "xml" | "svg" => "xml",
        "ini" | "cfg" | "conf" => "ini",
        "log" => "log",
        _ => "plaintext",
    }.into()
}
