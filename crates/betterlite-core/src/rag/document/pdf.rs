use std::panic::{AssertUnwindSafe, catch_unwind};

use lopdf::LoadOptions;

const MAX_PDF_INPUT_BYTES: usize = 50 * 1024 * 1024;
const MAX_PDF_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 10_000;

#[must_use]
pub fn extract_text_from_pdf(file_content: &[u8]) -> Option<String> {
    if file_content.len() > MAX_PDF_INPUT_BYTES {
        return None;
    }
    let extracted = catch_unwind(AssertUnwindSafe(|| {
        let options = LoadOptions::with_max_decompressed_size(MAX_PDF_STREAM_BYTES);
        let document = lopdf::Document::load_mem_with_options(file_content, options).ok()?;
        let page_numbers: Vec<u32> = document.get_pages().keys().copied().collect();
        if page_numbers.len() > MAX_PDF_PAGES {
            return None;
        }
        let mut text_parts: Vec<String> = Vec::new();
        for page_number in page_numbers {
            let raw_text = document
                .extract_text_with_limit(&[page_number], MAX_PDF_STREAM_BYTES)
                .ok()?;
            // lopdf's ET handler appends a synthetic newline that pypdf never adds.
            let text = raw_text.strip_suffix('\n').unwrap_or(&raw_text);
            if !text.is_empty() {
                text_parts.push(text.to_owned());
            }
        }
        if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n\n"))
        }
    }));
    extracted.unwrap_or_default()
}
