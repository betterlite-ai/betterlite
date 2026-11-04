use std::collections::BTreeMap;

use serde_json::{Map, Value};
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
    Utf8(#[from] std::str::Utf8Error),
    #[error("failed to decode Cohere stream JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid Cohere stream index")]
    Index,
    #[error("failed to parse Cohere v2 chunk: {0}")]
    V2Transform(String),
}

#[derive(Debug, Error)]
pub enum CohereIteratorError {
    #[error("Error receiving chunk from stream: {source}")]
    Receiving {
        #[source]
        source: std::io::Error,
    },
    #[error("Error parsing chunk: {message},\nReceived chunk: {received}")]
    Parsing { message: String, received: String },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CohereUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
        let decoded = std::str::from_utf8(input)?;
        match decoded.find("data:") {
            Some(position) => &decoded[position..],
            None => decoded,
        }
    } else {
        std::str::from_utf8(input)?
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
    item: Result<Option<Vec<u8>>, std::io::Error>,
) -> Result<Option<CohereChunk>, CohereIteratorError> {
    let Some(bytes) = item.map_err(|source| CohereIteratorError::Receiving { source })? else {
        return Ok(None);
    };
    let received = String::from_utf8_lossy(&bytes).into_owned();
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
        Some(Value::String(text)) => text.parse::<i64>().map_err(|_| CohereStreamError::Index),
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
    if chunk_type == "content-delta" {
        let content = value.pointer("/delta/message/content");
        chunk.text = content
            .and_then(Value::as_str)
            .or_else(|| {
                content
                    .and_then(|item| item.get("text"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_owned();
    } else if chunk_type == "tool-call-delta" {
        if let Some(tool) = value
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        {
            chunk.tool_use = Some(
                serde_json::json!({"id": tool.get("id").and_then(Value::as_str).unwrap_or_default(), "type":"function", "function":{"name":tool.get("name").and_then(Value::as_str).unwrap_or_default(), "arguments":tool.get("arguments").and_then(Value::as_str).unwrap_or_default()}}),
            );
        }
    } else if event == "tool-plan-delta" {
        if let Some(plan) = value
            .pointer("/data/delta/message/tool_plan")
            .filter(|item| !item.is_null() && item.as_str() != Some(""))
        {
            chunk.provider_specific_fields = Some(serde_json::json!({"tool_plan":plan}));
        }
    } else if event == "citation-start" {
        if let Some(citations) = value
            .pointer("/data/delta/message/citations")
            .filter(|item| item.is_object() && !item.as_object().is_some_and(Map::is_empty))
        {
            chunk.provider_specific_fields = Some(
                serde_json::json!({"citations":[{"start":citations.get("start").cloned().unwrap_or(Value::from(0)),"end":citations.get("end").cloned().unwrap_or(Value::from(0)),"text":citations.get("text").cloned().unwrap_or(Value::from("")),"sources":citations.get("sources").cloned().unwrap_or(Value::Array(Vec::new())),"type":citations.get("type").cloned().unwrap_or(Value::from("TEXT_CONTENT"))}]}),
            );
        }
    } else if event == "message-end" {
        let delta = value.get("data").and_then(|data| data.get("delta"));
        chunk.is_finished = true;
        chunk.finish_reason = delta
            .and_then(|data| data.get("finish_reason"))
            .and_then(Value::as_str)
            .unwrap_or("stop")
            .to_owned();
        if let Some(tokens) = delta
            .and_then(|data| data.get("usage"))
            .filter(|usage| !usage.is_null() && !usage.as_object().is_some_and(Map::is_empty))
            .and_then(|usage| usage.get("tokens"))
        {
            let prompt = tokens
                .get("input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let completion = tokens
                .get("output_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            chunk.usage = Some(CohereUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt.saturating_add(completion),
            });
        }
    }
    if let Some(citations) = value.get("citations") {
        let fields = chunk
            .provider_specific_fields
            .get_or_insert_with(|| Value::Object(Map::new()));
        if let Some(fields) = fields.as_object_mut() {
            fields.insert("citations".to_owned(), citations.clone());
        }
    }
    Ok(chunk)
}
