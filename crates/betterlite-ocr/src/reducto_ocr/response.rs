use std::collections::BTreeMap;

use serde::Deserialize;

use crate::json::{Json, Object};
use crate::ocr_config::decode::OcrFieldKind;
use crate::ocr_config::decode::decode_value;
use crate::ocr_error::OcrError;
use crate::response::{OCRPage, OCRResponse, OCRUsageInfo};

use super::numbers::{lax_i64, truncated_i64};

#[derive(Debug, Deserialize)]
pub(super) struct ReductoResponse {
    usage: Option<ReductoUsage>,
    chunks: Option<Vec<ReductoChunk>>,
    // result is kept here so an absent one (use the top-level chunks) stays distinct from null
    #[serde(flatten)]
    extra: Object,
}

#[derive(Debug, Deserialize)]
struct ReductoResult {
    chunks: Option<Vec<ReductoChunk>>,
}

#[derive(Debug, Default, Deserialize)]
struct ReductoUsage {
    num_pages: Option<Json>,
    credits: Option<Json>,
}

#[derive(Debug, Deserialize)]
struct ReductoChunk {
    content: Option<String>,
    blocks: Option<Vec<Object>>,
}

pub(super) fn normalize_response(
    model: &str,
    response: ReductoResponse,
) -> Result<OCRResponse, OcrError> {
    let ReductoResponse {
        usage,
        chunks: top_level_chunks,
        mut extra,
    } = response;
    let result = match extra.shift_remove("result") {
        None => ReductoResult {
            chunks: top_level_chunks,
        },
        Some(Json::Null) => ReductoResult { chunks: None },
        Some(value) => decode_value(&value, "result", OcrFieldKind::Response)?,
    };
    let chunks = result.chunks.unwrap_or_default();
    Ok(OCRResponse {
        pages: build_pages_from_reducto(&chunks)?,
        model: model.to_owned(),
        document_annotation: None,
        usage_info: Some(usage_info(usage)?),
        content: None,
        tables: None,
        key_value_pairs: None,
        object: "ocr".to_owned(),
        extra: Object::new(),
        provider_native_response: None,
    })
}

fn usage_info(reported: Option<ReductoUsage>) -> Result<OCRUsageInfo, OcrError> {
    let usage = reported.unwrap_or_default();
    let pages_processed = usage
        .num_pages
        .as_ref()
        .map(|value| {
            lax_i64(value).ok_or_else(|| OcrError::ResponseField {
                path: "usage.num_pages".to_owned(),
            })
        })
        .transpose()?;
    let credits = usage
        .credits
        .as_ref()
        .map(|value| {
            let parsed = match value {
                Json::Number(number) => number.as_f64(),
                Json::Text(text) => text.trim().parse::<f64>().ok(),
                Json::Bool(flag) => Some(f64::from(*flag)),
                Json::Null | Json::List(_) | Json::Object(_) => None,
            };
            parsed
                .filter(|candidate| candidate.is_finite())
                .ok_or_else(|| OcrError::ResponseField {
                    path: "usage.credits".to_owned(),
                })
        })
        .transpose()?;
    Ok(OCRUsageInfo {
        pages_processed,
        pages_processed_annotation: None,
        credits,
        doc_size_bytes: None,
        extra: Object::new(),
    })
}

fn build_pages_from_reducto(chunks: &[ReductoChunk]) -> Result<Vec<OCRPage>, OcrError> {
    let mut blocks_by_page: BTreeMap<i64, Vec<&Object>> = BTreeMap::new();
    for chunk in chunks {
        for block in chunk.blocks.iter().flatten() {
            if let Some(number) = block_page(block) {
                blocks_by_page.entry(number).or_default().push(block);
            }
        }
    }
    if blocks_by_page.is_empty() {
        return Ok(fallback_pages(chunks));
    }
    blocks_by_page
        .into_iter()
        .map(|(number, blocks)| page_from_blocks(number, &blocks))
        .collect()
}

// a page number LiteLLM cannot read leaves the block out instead of failing the whole response
fn block_page(block: &Object) -> Option<i64> {
    match block.get("bbox") {
        Some(Json::Object(bbox)) => bbox.get("page").and_then(block_page_number),
        Some(_) | None => None,
    }
}

fn block_page_number(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(truncated_i64)),
        Json::Text(text) => text.trim().parse().ok(),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn fallback_pages(chunks: &[ReductoChunk]) -> Vec<OCRPage> {
    let markdown = join_content(chunks.iter().map(|chunk| chunk.content.as_deref()));
    if markdown.is_empty() {
        return Vec::new();
    }
    vec![OCRPage {
        index: 0,
        markdown,
        images: None,
        dimensions: None,
        extra: Object::new(),
    }]
}

fn page_from_blocks(number: i64, blocks: &[&Object]) -> Result<OCRPage, OcrError> {
    let mut contents = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block.get("content") {
            None | Some(Json::Null) => contents.push(None),
            Some(Json::Text(content)) => contents.push(Some(content.as_str())),
            Some(_) => {
                return Err(OcrError::ResponseField {
                    path: "result.chunks.blocks.content".to_owned(),
                });
            }
        }
    }
    let markdown = join_content(contents.into_iter());
    let listed: Vec<Json> = blocks
        .iter()
        .map(|block| Json::Object((*block).clone()))
        .collect();
    let mut extra = Object::new();
    let _previous = extra.insert("blocks".to_owned(), Json::List(listed));
    Ok(OCRPage {
        index: number.saturating_sub(1).max(0),
        markdown,
        images: None,
        dimensions: None,
        extra,
    })
}

fn join_content<'source, Content>(content: Content) -> String
where
    Content: Iterator<Item = Option<&'source str>>,
{
    let mut parts = Vec::new();
    for text in content.flatten() {
        if !text.is_empty() {
            parts.push(text);
        }
    }
    parts.join("\n\n")
}
