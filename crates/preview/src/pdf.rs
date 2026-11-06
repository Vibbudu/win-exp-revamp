//! Pure-Rust PDF text & metadata extractor for the Universal Preview Engine.
//! Decompresses `/FlateDecode` streams, builds `ToUnicode` CMap tables, and extracts
//! readable text from PDF `BT`..`ET` text blocks (`Tj`, `TJ`, `'`, `"`).

use std::collections::HashMap;
use std::io::{Cursor, Read};

use anyhow::{Result, anyhow};
use flate2::read::{DeflateDecoder, ZlibDecoder};
use ipc_types::Preview;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::core::HSTRING;

const MAX_PDF_BYTES: usize = 16 * 1024 * 1024; // Read up to 16 MB for fast previewing

pub fn preview_pdf(path: &str) -> Result<Preview> {
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let file_size = meta.len();

    let to_read = (file_size as usize).min(MAX_PDF_BYTES);
    let mut raw = vec![0u8; to_read];
    let n = file.read(&mut raw)?;
    raw.truncate(n);

    // 1. Decompress all PDF streams (both content streams and ToUnicode CMap streams)
    let streams = extract_streams(&raw);

    // 2. Build a combined ToUnicode CMap from any stream containing `begincmap`
    let mut cmap: HashMap<u16, String> = HashMap::new();
    for s in &streams {
        if let Ok(txt) = std::str::from_utf8(s) {
            if txt.contains("begincmap") || txt.contains("beginbfchar") || txt.contains("beginbfrange") {
                parse_tounicode_cmap(txt, &mut cmap);
            }
        }
    }

    // 3. Extract text from content streams (`BT` .. `ET`)
    let mut extracted = String::new();
    for s in &streams {
        let text_lossy = String::from_utf8_lossy(s);
        if text_lossy.contains("begincmap") {
            continue;
        }
        if text_lossy.contains("BT") && (text_lossy.contains("Tj") || text_lossy.contains("TJ")) {
            let chunk = extract_bt_et_text(&text_lossy, &cmap);
            let trimmed = chunk.trim();
            if !trimmed.is_empty() {
                if !extracted.is_empty() {
                    extracted.push_str("\n\n");
                }
                extracted.push_str(trimmed);
                if extracted.len() > 32_000 {
                    break;
                }
            }
        }
    }

    // Also check uncompressed BT..ET in the raw bytes if streams yielded nothing
    if extracted.trim().is_empty() {
        let raw_lossy = String::from_utf8_lossy(&raw);
        if raw_lossy.contains("BT") {
            extracted = extract_bt_et_text(&raw_lossy, &cmap);
        }
    }

    let pages = count_pages(&raw).max(1);
    let mut summary = format!("PDF Document • ~{pages} page(s)\n");
    summary.push_str("────────────────────────────────────────\n\n");

    let cleaned = clean_extracted_text(&extracted);
    if cleaned.len() >= 8 {
        summary.push_str(&cleaned);
    } else {
        summary.push_str("(Scanned or image-based PDF — visual page preview shown above)");
    }

    Ok(Preview::Text {
        content: summary,
        language: "pdf".into(),
        truncated: file_size as usize > MAX_PDF_BYTES,
    })
}

fn count_pages(raw: &[u8]) -> usize {
    let s = String::from_utf8_lossy(raw);
    let mut count = 0;
    let mut search_from = 0;
    while let Some(pos) = s[search_from..].find("/Type") {
        let idx = search_from + pos;
        let rest = &s[idx + 5..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with("/Page") && !trimmed.starts_with("/Pages") {
            count += 1;
        }
        search_from = idx + 5;
    }
    count
}

fn extract_streams(raw: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 10 < raw.len() {
        let Some(pos) = find_subslice(&raw[i..], b"stream") else {
            break;
        };
        let stream_kw = i + pos;
        // Ensure it's not `endstream`
        if stream_kw >= 3 && &raw[stream_kw - 3..stream_kw] == b"end" {
            i = stream_kw + 6;
            continue;
        }
        let mut data_start = stream_kw + 6;
        if data_start < raw.len() && raw[data_start] == b'\r' {
            data_start += 1;
        }
        if data_start < raw.len() && raw[data_start] == b'\n' {
            data_start += 1;
        } else {
            i = stream_kw + 6;
            continue;
        }

        let Some(end_rel) = find_subslice(&raw[data_start..], b"endstream") else {
            break;
        };
        let mut data_end = data_start + end_rel;
        while data_end > data_start && (raw[data_end - 1] == b'\n' || raw[data_end - 1] == b'\r') {
            data_end -= 1;
        }

        let slice = &raw[data_start..data_end];
        if !slice.is_empty() && slice.len() < 4 * 1024 * 1024 {
            // Try zlib decompression first, then raw deflate, then keep raw if plain text
            let mut decoded = Vec::new();
            if ZlibDecoder::new(slice).read_to_end(&mut decoded).is_ok() && !decoded.is_empty() {
                out.push(decoded);
            } else {
                decoded.clear();
                if DeflateDecoder::new(slice).read_to_end(&mut decoded).is_ok() && !decoded.is_empty() {
                    out.push(decoded);
                } else {
                    out.push(slice.to_vec());
                }
            }
        }

        i = data_start + end_rel + 9;
        if out.len() > 150 {
            break;
        }
    }
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_tounicode_cmap(cmap_text: &str, map: &mut HashMap<u16, String>) {
    // 1. Parse `beginbfchar` .. `endbfchar`
    let mut rest = cmap_text;
    while let Some(start) = rest.find("beginbfchar") {
        let after = &rest[start + 11..];
        let Some(end) = after.find("endbfchar") else { break };
        let block = &after[..end];
        for line in block.lines() {
            let hexes = extract_hex_tokens(line);
            if hexes.len() >= 2 {
                if let (Some(src), Some(dst)) = (u16::from_str_radix(&hexes[0], 16).ok(), decode_utf16_hex(&hexes[1])) {
                    map.insert(src, dst);
                }
            }
        }
        rest = &after[end + 9..];
    }

    // 2. Parse `beginbfrange` .. `endbfrange`
    let mut rest = cmap_text;
    while let Some(start) = rest.find("beginbfrange") {
        let after = &rest[start + 12..];
        let Some(end) = after.find("endbfrange") else { break };
        let block = &after[..end];
        for line in block.lines() {
            let hexes = extract_hex_tokens(line);
            if hexes.len() >= 3 {
                if let (Ok(lo), Ok(hi), Ok(mut dst_code)) = (
                    u16::from_str_radix(&hexes[0], 16),
                    u16::from_str_radix(&hexes[1], 16),
                    u32::from_str_radix(&hexes[2], 16),
                ) {
                    if hi >= lo && (hi - lo) <= 512 {
                        for code in lo..=hi {
                            if let Some(ch) = char::from_u32(dst_code) {
                                map.insert(code, ch.to_string());
                            }
                            dst_code += 1;
                        }
                    }
                }
            }
        }
        rest = &after[end + 10..];
    }
}

fn extract_hex_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut token = String::new();
            for ch in chars.by_ref() {
                if ch == '>' {
                    break;
                }
                if ch.is_ascii_hexdigit() {
                    token.push(ch);
                }
            }
            if !token.is_empty() {
                out.push(token);
            }
        }
    }
    out
}

fn decode_utf16_hex(hex: &str) -> Option<String> {
    if hex.len() == 2 {
        let b = u8::from_str_radix(hex, 16).ok()?;
        return Some((b as char).to_string());
    }
    let mut units = Vec::new();
    let mut idx = 0;
    while idx + 4 <= hex.len() {
        if let Ok(u) = u16::from_str_radix(&hex[idx..idx + 4], 16) {
            units.push(u);
        }
        idx += 4;
    }
    if units.is_empty() {
        None
    } else {
        Some(String::from_utf16_lossy(&units))
    }
}

fn extract_bt_et_text(stream: &str, cmap: &HashMap<u16, String>) -> String {
    let mut out = String::new();
    let mut rest = stream;

    while let Some(bt) = rest.find("BT") {
        let after_bt = &rest[bt + 2..];
        let et = after_bt.find("ET").unwrap_or(after_bt.len());
        let block = &after_bt[..et];

        let mut chars = block.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '(' {
                // Literal PDF string `(...)`
                let mut literal = String::new();
                let mut depth = 1;
                while let Some(ch) = chars.next() {
                    if ch == '\\' {
                        if let Some(esc) = chars.next() {
                            match esc {
                                'n' => literal.push('\n'),
                                'r' => literal.push('\r'),
                                't' => literal.push('\t'),
                                '(' | ')' | '\\' => literal.push(esc),
                                '0'..='7' => {
                                    let mut oct = String::from(esc);
                                    for _ in 0..2 {
                                        if let Some(&nc) = chars.peek() {
                                            if ('0'..='7').contains(&nc) {
                                                oct.push(nc);
                                                chars.next();
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                    if let Ok(code) = u8::from_str_radix(&oct, 8) {
                                        if let Some(mapped) = cmap.get(&(code as u16)) {
                                            literal.push_str(mapped);
                                        } else if code >= 32 {
                                            literal.push(code as char);
                                        }
                                    }
                                }
                                _ => literal.push(esc),
                            }
                        }
                    } else if ch == '(' {
                        depth += 1;
                        literal.push('(');
                    } else if ch == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        literal.push(')');
                    } else {
                        let code = ch as u32;
                        if code <= 0xFFFF {
                            if let Some(mapped) = cmap.get(&(code as u16)) {
                                literal.push_str(mapped);
                                continue;
                            }
                        }
                        literal.push(ch);
                    }
                }
                out.push_str(&literal);
            } else if c == '<' {
                if chars.peek() == Some(&'<') {
                    chars.next();
                    continue;
                }
                let mut hex = String::new();
                while let Some(ch) = chars.next() {
                    if ch == '>' {
                        break;
                    }
                    if ch.is_ascii_hexdigit() {
                        hex.push(ch);
                    }
                }
                if !hex.is_empty() {
                    out.push_str(&decode_hex_string_with_cmap(&hex, cmap));
                }
            } else if c == 'T' {
                if let Some(&next_ch) = chars.peek() {
                    if next_ch == 'D' || next_ch == 'd' || next_ch == '*' || next_ch == 'm' {
                        if !out.ends_with('\n') && !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                }
            } else if c == '\n' || c == '\r' {
                if !out.ends_with('\n') && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }

        if et < after_bt.len() {
            rest = &after_bt[et + 2..];
        } else {
            break;
        }
    }

    out
}

fn decode_hex_string_with_cmap(hex: &str, cmap: &HashMap<u16, String>) -> String {
    let mut out = String::new();
    // Try 4-hex-digit (2-byte) codes first if even multiple of 4 and CMap has 2-byte keys
    let has_two_byte = cmap.keys().any(|&k| k > 0xFF);
    if hex.len() % 4 == 0 && (has_two_byte || cmap.is_empty()) {
        let mut i = 0;
        let mut matched_any = false;
        let mut temp = String::new();
        while i + 4 <= hex.len() {
            if let Ok(code) = u16::from_str_radix(&hex[i..i + 4], 16) {
                if let Some(s) = cmap.get(&code) {
                    temp.push_str(s);
                    matched_any = true;
                } else if (32..=126).contains(&code) {
                    temp.push((code as u8) as char);
                }
            }
            i += 4;
        }
        if matched_any || !temp.is_empty() {
            return temp;
        }
    }

    // Fallback: 2-hex-digit (1-byte) codes
    let mut i = 0;
    while i + 2 <= hex.len() {
        if let Ok(code) = u8::from_str_radix(&hex[i..i + 2], 16) {
            if let Some(s) = cmap.get(&(code as u16)) {
                out.push_str(s);
            } else if (32..=126).contains(&code) {
                out.push(code as char);
            }
        }
        i += 2;
    }
    out
}

fn clean_extracted_text(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for ch in s.chars() {
        if ch == '\n' {
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
            prev_space = true;
        } else if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else if !ch.is_control() {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Renders the first page of a PDF file to an RGBA pixel buffer `(width, height, rgba_bytes)`
/// using Windows 11's built-in `Windows.Data.Pdf.PdfDocument` WinRT engine.
/// This renders the actual visual PDF page (even when no shell thumbnail handler is cached),
/// avoiding generic browser/PDF reader app icons.
pub fn render_pdf_first_page_rgba(path: &str, target_width: u32) -> Result<(u32, u32, Vec<u8>)> {
    let hpath = HSTRING::from(path);
    let file = StorageFile::GetFileFromPathAsync(&hpath)?.join()?;
    let doc = PdfDocument::LoadFromFileAsync(&file)?.join()?;
    if doc.PageCount()? == 0 {
        return Err(anyhow!("PDF has 0 pages"));
    }
    let page = doc.GetPage(0)?;
    let size = page.Size()?;
    let w = size.Width.max(1.0);
    let h = size.Height.max(1.0);
    let dst_w = target_width.max(120);
    let dst_h = ((dst_w as f32) * (h / w)).round().max(120.0) as u32;

    let opts = PdfPageRenderOptions::new()?;
    opts.SetDestinationWidth(dst_w)?;
    opts.SetDestinationHeight(dst_h)?;

    let stream = InMemoryRandomAccessStream::new()?;
    page.RenderWithOptionsToStreamAsync(&stream, &opts)?.join()?;

    let stream_size = stream.Size()? as u32;
    if stream_size == 0 {
        return Err(anyhow!("Rendered PDF stream is empty"));
    }
    let input = stream.GetInputStreamAt(0)?;
    let reader = DataReader::CreateDataReader(&input)?;
    reader.LoadAsync(stream_size)?.join()?;
    let mut png_bytes = vec![0u8; stream_size as usize];
    reader.ReadBytes(&mut png_bytes)?;

    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut png_reader = decoder.read_info()?;
    let out_size = png_reader
        .output_buffer_size()
        .ok_or_else(|| anyhow!("PNG output buffer size overflow"))?;
    let mut buf = vec![0u8; out_size];
    let info = png_reader.next_frame(&mut buf)?;
    let bytes = &buf[..info.buffer_size()];

    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in bytes.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in bytes.chunks_exact(2) {
                out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &g in bytes {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        _ => return Err(anyhow!("Unsupported PNG color type from PdfDocument")),
    };

    Ok((info.width, info.height, rgba))
}
