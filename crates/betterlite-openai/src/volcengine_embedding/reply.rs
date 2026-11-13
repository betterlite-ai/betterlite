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

const NOT_OBJECT: &str = "the Volcengine embedding reply is not a JSON object";
const MODEL_NOT_STRING: &str = "the Volcengine embedding reply has a model that is not a string";

// the two optional parts are read as a typed record, where an absent or null part takes its default
#[derive(Debug, Deserialize)]
struct WireReply {
    #[serde(default)]
    data: Option<Vec<EmbeddingRecord>>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

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
    let model_name = reply_model(&fields, model)?;
    let reply = serde_json::from_value::<WireReply>(Value::Object(fields)).map_err(|source| {
        malformed(format!(
            "the Volcengine embedding reply is malformed: {source}"
        ))
    })?;
    Ok(EmbeddingResponse {
        object: String::from("list"),
        data: reply.data.unwrap_or_default(),
        model: model_name,
        usage: reply.usage.unwrap_or_default().into_usage(),
    })
}

fn reply_fields(raw: &[u8]) -> Result<Object, VolcEngineError> {
    let parsed: Value = serde_json::from_slice(raw).map_err(|source| {
        malformed(format!(
            "the Volcengine embedding reply is malformed: {source}"
        ))
    })?;
    let Value::Object(fields) = parsed else {
        return Err(malformed(String::from(NOT_OBJECT)));
    };
    Ok(fields)
}

fn reply_model(fields: &Object, model: &str) -> Result<String, VolcEngineError> {
    match fields.get("model") {
        None => Ok(model.to_owned()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        ) => Err(malformed(String::from(MODEL_NOT_STRING))),
    }
}

const fn malformed(message: String) -> VolcEngineError {
    VolcEngineError {
        status_code: FAILURE_STATUS,
        message,
        headers: Headers::new(),
    }
}
