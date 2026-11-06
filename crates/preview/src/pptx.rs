use std::fs::File;
use std::io::Read;
use anyhow::{Result, anyhow};
use ipc_types::Preview;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

pub fn preview_pptx(path: &str) -> Result<Preview> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut slides = Vec::new();
    let mut slide_num = 1;

    loop {
        let slide_name = format!("ppt/slides/slide{}.xml", slide_num);
        let mut slide_entry = match archive.by_name(&slide_name) {
            Ok(e) => e,
            Err(_) => break, // No more slides
        };

        let mut xml = String::new();
        if slide_entry.read_to_string(&mut xml).is_ok() {
            let mut reader = Reader::from_str(&xml);
            reader.config_mut().trim_text(true);

            let mut slide_lines = Vec::new();
            let mut in_t = false;
            let mut buf = Vec::new();

            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref e)) if e.name().as_ref() == b"a:t" => in_t = true,
                    Ok(Event::End(ref e)) if e.name().as_ref() == b"a:t" => in_t = false,
                    Ok(Event::Text(e)) => {
                        if in_t {
                            if let Ok(t) = std::str::from_utf8(e.as_ref()) {
                                let trimmed = t.trim();
                                if !trimmed.is_empty() {
                                    slide_lines.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                    Ok(Event::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }

            if !slide_lines.is_empty() {
                slides.push(slide_lines);
            }
        }

        slide_num += 1;
        if slide_num > 50 {
            break;
        }
    }

    if slides.is_empty() {
        return Ok(Preview::None {
            reason: "No readable slides found".into(),
        });
    }

    Ok(Preview::Slides { slides })
}
