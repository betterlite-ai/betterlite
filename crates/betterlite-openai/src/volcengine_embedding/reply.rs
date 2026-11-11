use serde::Deserialize;
use serde_json::Value;

use crate::embedding::{
    CompletionTokensDetails, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage,
    PromptTokensDetails,
};
use crate::openai_base_config::{Headers, Object};
use crate::volcengine_common::VolcEngineError;

// LiteLLM raises a bare exception on a reply it cannot read, which its proxy answers with a 500
const FAILURE_STATUS: u16 = 500;

// the counters default to zero and the two detail records stay absent, as LiteLLM's Usage() does
#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

impl WireUsage {
    fn into_usage(self) -> EmbeddingUsage {
        EmbeddingUsage {
            prompt_tokens: self.prompt_tokens.unwrap_or_default(),
            completion_tokens: self.completion_tokens.unwrap_or_default(),
            completion_tokens_details: self.completion_tokens_details,
            prompt_tokens_details: self.prompt_tokens_details,
            total_tokens: self.total_tokens.unwrap_or_default(),
        }
    }
}

// the reply's id and object are dropped, and a missing model falls back to the requested one
pub fn decode_reply(raw: &[u8], model: &str) -> Result<EmbeddingResponse, VolcEngineError> {
    let fields = reply_fields(raw)?;
    Ok(EmbeddingResponse {
        object: String::from("list"),
        data: reply_data(&fields)?,
        model: reply_model(&fields, model)?,
        usage: reply_usage(&fields)?,
    })
}

fn reply_model(fields: &Object, model: &str) -> Result<String, VolcEngineError> {
    match fields.get("model") {
        None => Ok(model.to_owned()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        ) => Err(malformed(String::from(
            "the Volcengine embedding reply has a model that is not a string",
        ))),
    }
}

fn reply_data(fields: &Object) -> Result<Vec<EmbeddingRecord>, VolcEngineError> {
    match fields.get("data") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            serde_json::from_value(Value::Array(items.clone())).map_err(|source| {
                malformed(format!(
                    "the Volcengine embedding reply has a malformed data list: {source}"
                ))
            })
        }
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            Err(malformed(String::from(
                "the Volcengine embedding reply has a data field that is not a list",
            )))
        }
    }
}

fn reply_usage(fields: &Object) -> Result<EmbeddingUsage, VolcEngineError> {
    match fields.get("usage") {
        None | Some(Value::Null) => Ok(WireUsage::default().into_usage()),
        Some(value @ Value::Object(_)) => serde_json::from_value(value.clone())
            .map(WireUsage::into_usage)
            .map_err(|source| {
                malformed(format!(
                    "the Volcengine embedding reply has a malformed usage: {source}"
                ))
            }),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            Err(malformed(String::from(
                "the Volcengine embedding reply has a usage that is not an object",
            )))
        }
    }
}

fn reply_fields(raw: &[u8]) -> Result<Object, VolcEngineError> {
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(fields)) => Ok(fields),
        Ok(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        ) => Err(malformed(String::from(
            "the Volcengine embedding reply is not a JSON object",
        ))),
        Err(source) => Err(malformed(format!(
            "the Volcengine embedding reply is malformed: {source}"
        ))),
    }
}

const fn malformed(message: String) -> VolcEngineError {
    VolcEngineError {
        status_code: FAILURE_STATUS,
        message,
        headers: Headers::new(),
    }
}
