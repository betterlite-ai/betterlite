use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::num::ParseIntError;
use std::str::{self, Utf8Error};
use std::string::FromUtf8Error;

use serde_json::{Error as JsonError, Map, Value};
use thiserror::Error;

mod tests;

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
            parse_v2(&value).map_err(|error| CohereStreamError::V2Transform(error.to_string()))
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

fn int_value(value: Option<&Value>) -> Result<i64, CohereStreamError> {
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
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let has_text = value.get("text").is_some();
    let is_finished = !has_text && value.get("is_finished") == Some(&Value::Bool(true));
    let finish_reason = if is_finished {
        value
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    } else {
        String::new()
    };
    let fields = value.get("citations").map(|citations| {
        Value::Object(Map::from_iter([(
            String::from("citations"),
            citations.clone(),
        )]))
    });
    Ok(CohereChunk {
        text,
        tool_use: None,
        is_finished,
        finish_reason,
        usage: None,
        index,
        provider_specific_fields: fields,
    })
}

fn parse_v2(value: &Value) -> Result<CohereChunk, CohereStreamError> {
    let index = int_value(value.get("index"))?;
    let chunk_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let event = value
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut chunk = CohereChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index,
        provider_specific_fields: None,
    };
    match chunk_type {
        "content-delta" => parse_content_delta(value, &mut chunk),
        "tool-call-delta" => parse_tool_call_delta(value, &mut chunk),
        _ => parse_event(event, value, &mut chunk),
    }
    copy_top_level_citations(value, &mut chunk);
    Ok(chunk)
}

fn parse_content_delta(value: &Value, chunk: &mut CohereChunk) {
    let content = value.pointer("/delta/message/content");
    content
        .and_then(Value::as_str)
        .or_else(|| {
            content
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .clone_into(&mut chunk.text);
}

fn parse_tool_call_delta(value: &Value, chunk: &mut CohereChunk) {
    let first_tool = value
        .pointer("/delta/tool_calls")
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    if let Some(tool_value) = first_tool {
        let function = Map::from_iter([
            (String::from("name"), string_field(tool_value, "name")),
            (
                String::from("arguments"),
                string_field(tool_value, "arguments"),
            ),
        ]);
        chunk.tool_use = Some(Value::Object(Map::from_iter([
            (String::from("id"), string_field(tool_value, "id")),
            (
                String::from("type"),
                Value::String(String::from("function")),
            ),
            (String::from("function"), Value::Object(function)),
        ])));
    }
}

fn parse_event(event: &str, value: &Value, chunk: &mut CohereChunk) {
    match event {
        "tool-plan-delta" => parse_tool_plan(value, chunk),
        "citation-start" => parse_citation_start(value, chunk),
        "message-end" => parse_message_end(value, chunk),
        _ => {}
    }
}

fn parse_tool_plan(value: &Value, chunk: &mut CohereChunk) {
    if let Some(plan) = value
        .pointer("/data/delta/message/tool_plan")
        .filter(|item| !item.is_null() && item.as_str() != Some(""))
    {
        let fields = Map::from_iter([(String::from("tool_plan"), plan.clone())]);
        chunk.provider_specific_fields = Some(Value::Object(fields));
    }
}

fn parse_citation_start(value: &Value, chunk: &mut CohereChunk) {
    let citations_value = value.pointer("/data/delta/message/citations");
    if let Some(citations) = citations_value
        .filter(|item| item.is_object() && !item.as_object().is_some_and(Map::is_empty))
    {
        let citation = Map::from_iter([
            (
                String::from("start"),
                citations
                    .get("start")
                    .map_or(Value::from(0_u64), Clone::clone),
            ),
            (
                String::from("end"),
                citations
                    .get("end")
                    .map_or(Value::from(0_u64), Clone::clone),
            ),
            (
                String::from("text"),
                citations
                    .get("text")
                    .map_or(Value::String(String::new()), Clone::clone),
            ),
            (
                String::from("sources"),
                citations
                    .get("sources")
                    .map_or(Value::Array(Vec::new()), Clone::clone),
            ),
            (
                String::from("type"),
                citations
                    .get("type")
                    .map_or(Value::String(String::from("TEXT_CONTENT")), Clone::clone),
            ),
        ]);
        chunk.provider_specific_fields = Some(Value::Object(Map::from_iter([(
            String::from("citations"),
            Value::Array(vec![Value::Object(citation)]),
        )])));
    }
}

fn parse_message_end(value: &Value, chunk: &mut CohereChunk) {
    let delta = value.get("data").and_then(|data| data.get("delta"));
    chunk.is_finished = true;
    delta
        .and_then(|data| data.get("finish_reason"))
        .and_then(Value::as_str)
        .unwrap_or("stop")
        .clone_into(&mut chunk.finish_reason);
    chunk.usage = delta.and_then(usage_from_delta);
}

fn usage_from_delta(delta: &Value) -> Option<CohereUsage> {
    let usage = delta.get("usage")?;
    if usage.is_null() || usage.as_object().is_some_and(Map::is_empty) {
        return None;
    }
    let tokens = usage.get("tokens")?;
    let prompt = tokens
        .get("input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completion = tokens
        .get("output_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Some(CohereUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.saturating_add(completion),
    })
}

fn string_field(value: &Value, field: &str) -> Value {
    Value::String(
        value
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    )
}

fn copy_top_level_citations(value: &Value, chunk: &mut CohereChunk) {
    if let Some(citations) = value.get("citations") {
        let provider_fields = chunk
            .provider_specific_fields
            .get_or_insert_with(|| Value::Object(Map::new()));
        if let Some(object) = provider_fields.as_object_mut() {
            object.insert(String::from("citations"), citations.clone());
        }
    }
}
