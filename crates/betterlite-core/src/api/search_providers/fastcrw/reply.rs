use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde_json::Deserializer;
use serde_json::value::RawValue;

use super::error::FastcrwError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

type RawFields = BTreeMap<String, Box<RawValue>>;

pub fn decode_fastcrw_reply(bytes: &[u8]) -> Result<SearchResponse, FastcrwError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(FastcrwError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut decoder = Deserializer::from_slice(bytes);
    // a bare list or scalar document fails here, as response_json.get does in the source
    let reply: RawFields = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| FastcrwError::Reply { source })?;
    decoder.end().map_err(FastcrwError::TrailingBytes)?;
    let results = match reply.get("data") {
        Some(data) => project_data(data.get())?,
        None => Vec::new(),
    };
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

// LiteLLM reads only a list here and lets the source validation reject a member that is not an object
fn project_data(text: &str) -> Result<Vec<SearchResult>, FastcrwError> {
    if !text.starts_with('[') {
        return Ok(Vec::new());
    }
    let items: Vec<RawFields> =
        serde_json::from_str(text).map_err(|source| FastcrwError::Result { source })?;
    items.iter().map(project).collect()
}

fn project(fields: &RawFields) -> Result<SearchResult, FastcrwError> {
    let snippet = fields
        .get("markdown")
        .map(Box::as_ref)
        .and_then(markdown_text)
        .map_or_else(
            || text_field(fields.get("description").map(Box::as_ref), "description"),
            Ok,
        )?;
    Ok(SearchResult {
        title: text_field(fields.get("title").map(Box::as_ref), "title")?,
        url: text_field(fields.get("url").map(Box::as_ref), "url")?,
        snippet,
        date: None,
        last_updated: None,
        extra: Object::new(),
    })
}

fn markdown_text(raw: &RawValue) -> Option<String> {
    serde_json::from_str::<String>(raw.get())
        .ok()
        .filter(|text| !text.is_empty())
}

fn text_field(value: Option<&RawValue>, field: &'static str) -> Result<String, FastcrwError> {
    let Some(raw) = value else {
        return Ok(String::new());
    };
    serde_json::from_str::<String>(raw.get())
        .map_err(|source| FastcrwError::NotText { field, source })
}
