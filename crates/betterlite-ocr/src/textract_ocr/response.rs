use std::collections::BTreeSet;

use crate::json::Object;
use crate::response::{OCRPage, OCRResponse, OCRUsageInfo};
use crate::textract_ocr::wire::{Block, BlockType, DocumentMetadata};

pub(crate) fn lines_by_page(blocks: &[Block]) -> Vec<(i64, String)> {
    let pages: BTreeSet<i64> = blocks.iter().map(|block| block.page).collect();
    pages
        .into_iter()
        .map(|page| {
            let lines: Vec<&str> = blocks
                .iter()
                .filter(|block| block.kind == BlockType::Line && block.page == page)
                .filter_map(|block| block.text.as_deref())
                .collect();
            (page, lines.join("\n"))
        })
        .filter(|(_, markdown)| !markdown.is_empty())
        .collect()
}

pub(crate) fn ocr_response(
    model: &str,
    page_markdown: Vec<(i64, String)>,
    document_metadata: Option<DocumentMetadata>,
) -> OCRResponse {
    let pages: Vec<OCRPage> = page_markdown
        .into_iter()
        .map(|(page, markdown)| OCRPage {
            index: page.saturating_sub(1),
            markdown,
            images: None,
            dimensions: None,
            extra: Object::new(),
        })
        .collect();
    let pages_processed = document_metadata
        .and_then(|metadata| metadata.pages)
        .or_else(|| i64::try_from(pages.len()).ok());
    OCRResponse {
        pages,
        model: model.to_owned(),
        document_annotation: None,
        usage_info: Some(OCRUsageInfo {
            pages_processed,
            pages_processed_annotation: None,
            credits: None,
            doc_size_bytes: None,
            extra: Object::new(),
        }),
        content: None,
        tables: None,
        key_value_pairs: None,
        object: "ocr".to_owned(),
        extra: Object::new(),
        provider_native_response: None,
    }
}
