use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::num::ParseIntError;
use std::str::{self, Utf8Error};
use std::string::FromUtf8Error;

use serde_json::{Error as JsonError, Map, Value};
use thiserror::Error;

mod tests;
mod v2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CohereStreamVersion {
    V1,
    V2,
}

#[derive(Debug, Error)]
pub enum CohereStreamError {
    #[error("failed to decode UTF-8 Cohere stream line: {0}")]
    Utf8(#[from] Utf8Error),
    #[error("failed to decode Cohere stream JSON: {0}")]
    Json(#[from] JsonError),
    #[error("invalid Cohere stream index")]
    Index,
    #[error("invalid Cohere stream index: {source}")]
    InvalidIndex {
        #[source]
        source: ParseIntError,
    },
    #[error("invalid UTF-8 in received Cohere stream chunk: {source}")]
    ReceivedUtf8 {
        #[source]
        source: FromUtf8Error,
    },
    #[error("failed to parse Cohere v2 chunk: {0}")]
    V2Transform(String),
}

#[derive(Debug, Error)]
pub enum CohereIteratorError {
    #[error("Error receiving chunk from stream: {source}")]
    Receiving {
        #[source]
        source: IoError,
    },
    #[error("Error parsing chunk: {message},\nReceived chunk: {received}")]
    Parsing { message: String, received: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CohereUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CohereChunk {
    pub text: String,
    pub tool_use: Option<Value>,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<CohereUsage>,
    pub index: i64,
    pub provider_specific_fields: Option<Value>,
}

pub fn validate_environment(headers: &mut BTreeMap<String, String>, api_key: &str) {
    headers.insert(
        "Request-Source".to_owned(),
        "unspecified:litellm".to_owned(),
    );
    headers.insert("accept".to_owned(), "application/json".to_owned());
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    if !api_key.is_empty() {
        headers.insert("Authorization".to_owned(), format!("Bearer {api_key}"));
    }
}

pub fn decode_cohere_line(
    version: CohereStreamVersion,
    input: &[u8],
    is_bytes: bool,
) -> Result<CohereChunk, CohereStreamError> {
    let text = if is_bytes {
        let decoded = str::from_utf8(input)?;
        decoded.find("data:").map_or(decoded, |position| {
            decoded.get(position..).unwrap_or(decoded)
        })
    } else {
        str::from_utf8(input)?
    };
    let value: Value = serde_json::from_str(text)?;
    match version {
        CohereStreamVersion::V1 => parse_v1(&value),
        CohereStreamVersion::V2 => {
            v2::parse_v2(&value).map_err(|error| CohereStreamError::V2Transform(error.to_string()))
        }
    }
}

pub fn decode_cohere_item(
    version: CohereStreamVersion,
    item: Result<Option<Vec<u8>>, IoError>,
) -> Result<Option<CohereChunk>, CohereIteratorError> {
    let Some(bytes) = item.map_err(|source| CohereIteratorError::Receiving { source })? else {
        return Ok(None);
    };
    let received = match String::from_utf8(bytes.clone()) {
        Ok(received) => received,
        Err(source) => CohereStreamError::ReceivedUtf8 { source }.to_string(),
    };
    decode_cohere_line(version, &bytes, true)
        .map(Some)
        .map_err(|error| CohereIteratorError::Parsing {
            message: error.to_string(),
            received,
        })
}

pub(super) fn int_value(value: Option<&Value>) -> Result<i64, CohereStreamError> {
    match value {
        None => Ok(0),
        Some(Value::Number(number)) => number.as_i64().ok_or(CohereStreamError::Index),
        Some(Value::String(text)) => text
            .parse::<i64>()
            .map_err(|source| CohereStreamError::InvalidIndex { source }),
        Some(_) => Err(CohereStreamError::Index),
    }
}

fn parse_v1(value: &Value) -> Result<CohereChunk, CohereStreamError> {
    let index = int_value(value.get("index"))?;
    let mut chunk = v2::empty_chunk(index);
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .clone_into(&mut chunk.text);
    let has_text = value.get("text").is_some();
    chunk.is_finished = !has_text && value.get("is_finished") == Some(&Value::Bool(true));
    if chunk.is_finished {
        value
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .clone_into(&mut chunk.finish_reason);
    }
    chunk.provider_specific_fields = value.get("citations").map(|citations| {
        Value::Object(Map::from_iter([(
            String::from("citations"),
            citations.clone(),
        )]))
    });
    Ok(chunk)
}
