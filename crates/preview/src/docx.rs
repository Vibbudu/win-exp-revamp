use std::fs::File;
use std::io::Read;
use anyhow::{Result, anyhow};
use ipc_types::Preview;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

pub fn preview_docx(path: &str) -> Result<Preview> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut doc_entry = archive
        .by_name("word/document.xml")
        .map_err(|e| anyhow!("Not a valid docx or missing word/document.xml: {}", e))?;

    let mut xml = String::new();
    doc_entry.read_to_string(&mut xml)?;

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);

    let mut html = String::from("<div class=\"docx-body\">");
    let mut in_p = false;
    let mut in_t = false;
    let mut is_bold = false;
    let mut is_italic = false;
    let mut p_text = String::new();

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"w:p" => {
                    in_p = true;
                    p_text.clear();
                }
                b"w:b" => is_bold = true,
                b"w:i" => is_italic = true,
                b"w:t" => in_t = true,
                b"w:tbl" => html.push_str("<table class=\"docx-table\">"),
                b"w:tr" => html.push_str("<tr>"),
                b"w:tc" => html.push_str("<td>"),
                _ => {}
            },
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"w:p" => {
                    in_p = false;
                    let trimmed = p_text.trim();
                    if !trimmed.is_empty() {
                        html.push_str("<p>");
                        html.push_str(&html_escape(trimmed));
                        html.push_str("</p>\n");
                    }
                    p_text.clear();
                }
                b"w:t" => in_t = false,
                b"w:tbl" => html.push_str("</table>\n"),
                b"w:tr" => html.push_str("</tr>\n"),
                b"w:tc" => html.push_str("</td>\n"),
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if in_t {
                    if let Ok(t) = std::str::from_utf8(e.as_ref()) {
                        p_text.push_str(t);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    html.push_str("</div>");

    Ok(Preview::Html { html })
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
