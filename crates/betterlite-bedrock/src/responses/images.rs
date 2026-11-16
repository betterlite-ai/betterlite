use std::collections::BTreeMap;
use std::slice::from_ref;

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
        Json::Object(_) => from_ref(value),
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
    let Some(url) = remote_image_url(block) else {
        return Ok(block.clone());
    };
    let Some(data_url) = inlined.get(url) else {
        return Err(ResponsesError::MissingInlined {
            url: url.to_owned(),
        });
    };
    let mut replaced = block.clone();
    if let Json::Object(fields) = &mut replaced {
        fields.insert("image_url".to_owned(), Json::Text(data_url.clone()));
    }
    Ok(replaced)
}

fn inline_field(
    key: &str,
    value: &Json,
    inlined: &BTreeMap<String, String>,
) -> Result<Json, ResponsesError> {
    if !IMAGE_BLOCK_KEYS.contains(&key) {
        return Ok(value.clone());
    }
    match value {
        Json::List(blocks) => blocks
            .iter()
            .map(|block| inline_block(block, inlined))
            .collect::<Result<Vec<_>, _>>()
            .map(Json::List),
        Json::Object(_) => inline_block(value, inlined),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Ok(value.clone()),
    }
}

fn inline_item(item: &Json, inlined: &BTreeMap<String, String>) -> Result<Json, ResponsesError> {
    match item {
        Json::Object(fields) => fields
            .iter()
            .map(|(key, value)| Ok((key.clone(), inline_field(key, value, inlined)?)))
            .collect::<Result<_, _>>()
            .map(Json::Object),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Ok(item.clone())
        }
    }
}

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
