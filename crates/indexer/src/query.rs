use ipc_types::Entry;
use memchr::memmem::Finder;

#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub terms: Vec<String>,
    pub ext: Option<Vec<String>>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub is_dir: Option<bool>,
    pub path_prefix: Option<String>,
}

impl QueryFilter {
    pub fn parse(input: &str) -> Self {
        let mut filter = QueryFilter::default();
        let mut terms = Vec::new();

        for part in input.split_whitespace() {
            if let Some((k, v)) = part.split_once(':') {
                let key = k.to_lowercase();
                let val = v.trim();
                match key.as_str() {
                    "ext" => {
                        let exts: Vec<String> = val
                            .split(',')
                            .map(|s| s.trim().trim_start_matches('.').to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                        if !exts.is_empty() {
                            filter.ext = Some(exts);
                        }
                    }
                    "size" => {
                        if let Some(rest) = val.strip_prefix('>') {
                            if let Some(bytes) = parse_size(rest) {
                                filter.min_size = Some(bytes);
                            }
                        } else if let Some(rest) = val.strip_prefix('<') {
                            if let Some(bytes) = parse_size(rest) {
                                filter.max_size = Some(bytes);
                            }
                        }
                    }
                    "kind" => match val.to_lowercase().as_str() {
                        "folder" | "dir" => filter.is_dir = Some(true),
                        "file" => filter.is_dir = Some(false),
                        "doc" | "document" => {
                            filter.ext = Some(vec![
                                "doc".into(), "docx".into(), "pdf".into(), "txt".into(), "rtf".into(),
                                "odt".into(), "xls".into(), "xlsx".into(), "ppt".into(), "pptx".into(),
                            ]);
                        }
                        "picture" | "photo" | "image" => {
                            filter.ext = Some(vec![
                                "png".into(), "jpg".into(), "jpeg".into(), "gif".into(), "bmp".into(),
                                "webp".into(), "svg".into(), "ico".into(), "tiff".into(),
                            ]);
                        }
                        "video" => {
                            filter.ext = Some(vec![
                                "mp4".into(), "mkv".into(), "avi".into(), "mov".into(), "wmv".into(),
                                "flv".into(), "webm".into(),
                            ]);
                        }
                        "music" | "audio" => {
                            filter.ext = Some(vec![
                                "mp3".into(), "wav".into(), "flac".into(), "m4a".into(), "wma".into(),
                                "aac".into(), "ogg".into(),
                            ]);
                        }
                        _ => {}
                    },
                    "path" => {
                        filter.path_prefix = Some(val.to_lowercase());
                    }
                    _ => {
                        terms.push(part.to_lowercase());
                    }
                }
            } else {
                terms.push(part.to_lowercase());
            }
        }

        filter.terms = terms;
        filter
    }
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if let Some(num) = s.strip_suffix("gb") {
        num.trim().parse::<f64>().ok().map(|n| (n * 1024.0 * 1024.0 * 1024.0) as u64)
    } else if let Some(num) = s.strip_suffix("mb") {
        num.trim().parse::<f64>().ok().map(|n| (n * 1024.0 * 1024.0) as u64)
    } else if let Some(num) = s.strip_suffix("kb") {
        num.trim().parse::<f64>().ok().map(|n| (n * 1024.0) as u64)
    } else if let Some(num) = s.strip_suffix('b') {
        num.trim().parse::<u64>().ok()
    } else {
        s.parse::<u64>().ok()
    }
}
