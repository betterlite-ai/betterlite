use std::any::type_name;

use betterlite_context::local_ttl_cache::CacheValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[path = "codec/tests.rs"]
mod codec_tests;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("the value cannot be written as json")]
    Serialize(#[source] serde_json::Error),
    #[error("the payload does not match the model")]
    Validation(#[source] serde_json::Error),
}

pub type CodecResult<Value> = Result<Value, CodecError>;

pub struct CacheCodec;

impl CacheCodec {
    // a model instance dumps straight to its json form, without the validation round trip LiteLLM skips too
    pub fn encode<Model>(value: &Model) -> CodecResult<CacheValue>
    where
        Model: Serialize,
    {
        serde_json::from_value(serde_json::to_value(value).map_err(CodecError::Serialize)?)
            .map_err(CodecError::Serialize)
    }

    // an untyped payload is validated against the model first, and the validated fields are what gets dumped
    pub fn encode_validated<Model>(value: &CacheValue) -> CodecResult<CacheValue>
    where
        Model: DeserializeOwned + Serialize,
    {
        if !matches!(value, CacheValue::Object(_)) {
            // LiteLLM validates a mapping or a model and returns any other payload unchanged
            return Ok(value.clone());
        }
        let json = serde_json::to_string(value).map_err(CodecError::Serialize)?;
        let model: Model = serde_json::from_str(&json).map_err(CodecError::Validation)?;
        Self::encode(&model)
    }

    pub fn decode<Model>(cached: Option<&CacheValue>) -> Option<Model>
    where
        Model: DeserializeOwned,
    {
        let entry = cached?;
        if !matches!(entry, CacheValue::Object(_)) {
            // LiteLLM treats a payload that is not a mapping as a miss and logs nothing for it
            return None;
        }
        let json = serde_json::to_value(entry).ok()?;
        match serde_json::from_value(json) {
            Ok(model) => Some(model),
            Err(source) => {
                tracing::warn!(
                    model = type_name::<Model>(),
                    error = %source,
                    "CacheCodec.decode: validation failed"
                );
                None
            }
        }
    }
}
