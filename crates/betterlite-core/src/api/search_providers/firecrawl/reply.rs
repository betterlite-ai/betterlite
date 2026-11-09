use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde_json::Deserializer;
use serde_json::value::RawValue;

use super::error::FirecrawlError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

type RawFields = BTreeMap<String, Box<RawValue>>;

// the cloud v2 shape nests web and news under one object; a null list fails like the source does
#[derive(Deserialize)]
struct FirecrawlCloud {
    #[serde(default)]
    web: Vec<RawFields>,
    #[serde(default)]
    news: Vec<RawFields>,
}

pub fn decode_firecrawl_reply(bytes: &[u8]) -> Result<SearchResponse, FirecrawlError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(FirecrawlError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut decoder = Deserializer::from_slice(bytes);
    // a bare list or scalar document fails here, as response_json.get does in the source
    let reply: RawFields = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| FirecrawlError::Reply { source })?;
    decoder.end().map_err(FirecrawlError::TrailingBytes)?;
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

fn project_data(text: &str) -> Result<Vec<SearchResult>, FirecrawlError> {
    match text.as_bytes().first().copied() {
        Some(b'[') => flat_results(text),
        Some(b'{') => cloud_results(text),
        _ => Ok(Vec::new()),
    }
}

// the self-hosted v1 shape and the cloud web list share one projection
fn flat_results(text: &str) -> Result<Vec<SearchResult>, FirecrawlError> {
    let items: Vec<RawFields> =
        serde_json::from_str(text).map_err(|source| FirecrawlError::Result { source })?;
    items
        .iter()
        .map(|fields| project_cloud(fields, CloudSection::Web))
        .collect()
}

fn cloud_results(text: &str) -> Result<Vec<SearchResult>, FirecrawlError> {
    let cloud: FirecrawlCloud =
        serde_json::from_str(text).map_err(|source| FirecrawlError::Result { source })?;
    let web = cloud
        .web
        .iter()
        .map(|fields| project_cloud(fields, CloudSection::Web))
        .collect::<Result<Vec<_>, _>>()?;
    let news = cloud
        .news
        .iter()
        .map(|fields| project_cloud(fields, CloudSection::News))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(web.into_iter().chain(news).collect())
}

#[derive(Clone, Copy)]
enum CloudSection {
    Web,
    News,
}

// a web item ignores its date and falls back to description, a news item keeps its date and snippet
fn project_cloud(
    fields: &RawFields,
    section: CloudSection,
) -> Result<SearchResult, FirecrawlError> {
    let (fallback, date) = match section {
        CloudSection::Web => ("description", None),
        CloudSection::News => (
            "snippet",
            optional_text(fields.get("date").map(Box::as_ref), "date")?,
        ),
    };
    Ok(SearchResult {
        title: text_field(fields.get("title").map(Box::as_ref), "title")?,
        url: text_field(fields.get("url").map(Box::as_ref), "url")?,
        snippet: markdown_then(
            fields.get("markdown").map(Box::as_ref),
            fields.get(fallback).map(Box::as_ref),
            fallback,
        )?,
        date,
        last_updated: None,
        extra: Object::new(),
    })
}

fn markdown_then(
    markdown: Option<&RawValue>,
    fallback: Option<&RawValue>,
    field: &'static str,
) -> Result<String, FirecrawlError> {
    markdown
        .and_then(markdown_text)
        .map_or_else(|| text_field(fallback, field), Ok)
}

fn markdown_text(raw: &RawValue) -> Option<String> {
    serde_json::from_str::<String>(raw.get())
        .ok()
        .filter(|text| !text.is_empty())
}

fn text_field(value: Option<&RawValue>, field: &'static str) -> Result<String, FirecrawlError> {
    let Some(raw) = value else {
        return Ok(String::new());
    };
    serde_json::from_str::<String>(raw.get())
        .map_err(|source| FirecrawlError::NotText { field, source })
}

fn optional_text(
    value: Option<&RawValue>,
    field: &'static str,
) -> Result<Option<String>, FirecrawlError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    serde_json::from_str::<Option<String>>(raw.get())
        .map_err(|source| FirecrawlError::NotText { field, source })
}
