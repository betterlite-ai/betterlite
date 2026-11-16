use std::collections::BTreeMap;

use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexSet;

use crate::responses::error::ResponsesError;

const IMAGE_BLOCK_KEYS: [&str; 2] = ["content", "output"];
const IMAGE_BLOCK_TYPES: [&str; 2] = ["input_image", "computer_screenshot"];
const REMOTE_IMAGE_URL_SCHEMES: [&str; 2] = ["http://", "https://"];

fn remote_image_url(block: &Json) -> Option<&str> {
    let fields = block.as_object()?;
    let kind = fields.get("type")?.as_str()?;
    let url = fields.get("image_url")?.as_str()?;
    let remote = REMOTE_IMAGE_URL_SCHEMES
        .iter()
        .any(|scheme| url.starts_with(scheme));
    (IMAGE_BLOCK_TYPES.contains(&kind) && remote).then_some(url)
}

fn blocks_under(value: &Json) -> &[Json] {
    match value {
        Json::List(blocks) => blocks,
        Json::Object(_) => std::slice::from_ref(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => &[],
    }
}

#[must_use]
pub fn collect_remote_image_urls(input: &ResponsesInput) -> Vec<String> {
    let ResponsesInput::Items(items) = input else {
        return Vec::new();
    };
    let mut urls = IndexSet::new();
    for item in items {
        let Some(fields) = item.0.as_object() else {
            continue;
        };
        for key in IMAGE_BLOCK_KEYS {
            let Some(value) = fields.get(key) else {
                continue;
            };
            let blocks = blocks_under(value);
            urls.extend(
                blocks
                    .iter()
                    .filter_map(remote_image_url)
                    .map(str::to_owned),
            );
        }
    }
    urls.into_iter().collect()
}

fn inline_block(block: &Json, inlined: &BTreeMap<String, String>) -> Result<Json, ResponsesError> {
    let (Some(url), Json::Object(fields)) = (remote_image_url(block), block) else {
        return Ok(block.clone());
    };
    let data_url = inlined
        .get(url)
        .ok_or_else(|| ResponsesError::MissingInlined {
            url: url.to_owned(),
        })?;
    let mut replaced = fields.clone();
    replaced.insert("image_url".to_owned(), Json::Text(data_url.clone()));
    Ok(Json::Object(replaced))
}

fn inline_item(item: &Json, inlined: &BTreeMap<String, String>) -> Result<Json, ResponsesError> {
    let Json::Object(fields) = item else {
        return Ok(item.clone());
    };
    let mut replaced = fields.clone();
    for key in IMAGE_BLOCK_KEYS {
        let value = match fields.get(key) {
            Some(Json::List(blocks)) => {
                let mut inlined_blocks = Vec::with_capacity(blocks.len());
                for block in blocks {
                    inlined_blocks.push(inline_block(block, inlined)?);
                }
                Json::List(inlined_blocks)
            }
            Some(single @ Json::Object(_)) => inline_block(single, inlined)?,
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) | None => continue,
        };
        replaced.insert(key.to_owned(), value);
    }
    Ok(Json::Object(replaced))
}

// A missing mapping entry is an error, never a fresh download
pub fn inline_remote_image_urls(
    input: &ResponsesInput,
    inlined: &BTreeMap<String, String>,
) -> Result<ResponsesInput, ResponsesError> {
    let ResponsesInput::Items(items) = input else {
        return Ok(input.clone());
    };
    if inlined.is_empty() {
        return Ok(input.clone());
    }
    let mut replaced = Vec::with_capacity(items.len());
    for item in items {
        replaced.push(ResponsesItem(inline_item(&item.0, inlined)?));
    }
    Ok(ResponsesInput::Items(replaced))
}
