use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::Number;
use serde_json::value::RawValue;

use super::GeminiIngestionError;
use super::reply::ReplyError;
use crate::rag::ingestion::config::VectorStoreConfig;
use crate::rag::ingestion::upload::DEFAULT_CONTENT_TYPE;

pub(super) const API_VERSION: &str = "v1beta";

// LiteLLM strips every version segment from the base and then adds the upload prefix and version back
pub(super) fn initiate_url(base_url: &str, store_id: &str) -> String {
    let stripped = base_url.replace("/v1beta", "");
    format!("{stripped}/upload/{API_VERSION}/{store_id}:uploadToFileSearchStore")
}

pub(super) fn json_headers(api_key: &str) -> Result<HeaderMap, GeminiIngestionError> {
    let mut headers = HeaderMap::new();
    insert(&mut headers, "content-type", "application/json")?;
    insert(&mut headers, "x-goog-api-key", api_key)?;
    Ok(headers)
}

pub(super) fn initiate_headers(
    api_key: &str,
    file_size: usize,
    content_type: Option<&str>,
) -> Result<HeaderMap, GeminiIngestionError> {
    let mut headers = HeaderMap::new();
    insert(&mut headers, "x-goog-upload-protocol", "resumable")?;
    insert(&mut headers, "x-goog-upload-command", "start")?;
    insert(
        &mut headers,
        "x-goog-upload-header-content-length",
        &file_size.to_string(),
    )?;
    insert(
        &mut headers,
        "x-goog-upload-header-content-type",
        content_type
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CONTENT_TYPE),
    )?;
    insert(&mut headers, "content-type", "application/json")?;
    insert(&mut headers, "x-goog-api-key", api_key)?;
    Ok(headers)
}

pub(super) fn put_headers(file_size: usize) -> Result<HeaderMap, GeminiIngestionError> {
    let mut headers = HeaderMap::new();
    insert(&mut headers, "content-length", &file_size.to_string())?;
    insert(&mut headers, "x-goog-upload-offset", "0")?;
    insert(&mut headers, "x-goog-upload-command", "upload, finalize")?;
    Ok(headers)
}

pub(super) fn initiate_body(
    filename: &str,
    chunking_strategy: &RawValue,
    config: &VectorStoreConfig,
) -> Result<Json, GeminiIngestionError> {
    let mut body = IndexMap::new();
    body.insert("displayName".to_owned(), Json::Text(filename.to_owned()));
    if let Some(chunking) = chunking_config(chunking_strategy) {
        body.insert("chunkingConfig".to_owned(), chunking);
    }
    if let Some(metadata) = custom_metadata(config)? {
        body.insert("customMetadata".to_owned(), metadata);
    }
    Ok(Json::Object(body))
}

// IngestionState parsed the strategy once, so this text is valid JSON
fn chunking_config(strategy: &RawValue) -> Option<Json> {
    let Json::Object(fields) = serde_json::from_str::<Json>(strategy.get()).ok()? else {
        return None;
    };
    let Some(Json::Object(white_space)) = fields.get("white_space_config") else {
        return None;
    };
    if white_space.is_empty() {
        return None;
    }
    let max_tokens = white_space
        .get("max_tokens_per_chunk")
        .map_or_else(|| Json::Number(Number::from(800_u32)), Clone::clone);
    let max_overlap = white_space
        .get("max_overlap_tokens")
        .map_or_else(|| Json::Number(Number::from(400_u32)), Clone::clone);
    let mut config = IndexMap::new();
    config.insert("maxTokensPerChunk".to_owned(), max_tokens);
    config.insert("maxOverlapTokens".to_owned(), max_overlap);
    let mut chunking = IndexMap::new();
    chunking.insert("whiteSpaceConfig".to_owned(), Json::Object(config));
    Some(Json::Object(chunking))
}

pub(super) fn custom_metadata(
    config: &VectorStoreConfig,
) -> Result<Option<Json>, GeminiIngestionError> {
    let Some(raw) = config.get("custom_metadata") else {
        return Ok(None);
    };
    let value = serde_json::from_str::<Json>(raw.get()).map_err(|_source| {
        GeminiIngestionError::Setting {
            field: "custom_metadata",
            expected: "a list",
        }
    })?;
    match value {
        Json::Null => Ok(None),
        Json::List(items) => Ok((!items.is_empty()).then_some(Json::List(items))),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(GeminiIngestionError::Setting {
                field: "custom_metadata",
                expected: "a list",
            })
        }
    }
}

pub(super) fn store_name(body: &[u8]) -> Result<String, GeminiIngestionError> {
    let Json::Object(fields) = serde_json::from_slice::<Json>(body).map_err(not_json)? else {
        return Err(GeminiIngestionError::BadReply {
            source: ReplyError::NotObject,
        });
    };
    match fields.get("name") {
        Some(Json::Text(name)) => Ok(name.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Ok(String::new()),
    }
}

pub(super) fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_invalid| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

const fn not_json(source: serde_json::Error) -> GeminiIngestionError {
    GeminiIngestionError::BadReply {
        source: ReplyError::NotJson(source),
    }
}

fn insert(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
) -> Result<(), GeminiIngestionError> {
    let header_value = HeaderValue::from_str(value)
        .map_err(|source| GeminiIngestionError::Header { name, source })?;
    headers.insert(HeaderName::from_static(name), header_value);
    Ok(())
}
