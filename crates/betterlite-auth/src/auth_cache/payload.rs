use betterlite_cache::codec::{CacheCodec, CodecError};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("the cached payload has no json representation")]
    Json(#[source] serde_json::Error),
    #[error("the cached value could not be serialized")]
    Codec(#[from] CodecError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOperation {
    Get,
    AsyncGet,
}

impl ReadOperation {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Get => "get_cache",
            Self::AsyncGet => "async_get_cache",
        }
    }
}

// the wrapper adds this line after the codec's own warning; the key and the payload stay out of it
pub(super) fn log_decode_miss(operation: ReadOperation, model: &'static str) {
    tracing::error!(
        model,
        "UserApiKeyCache.{} failed to deserialize the cached value",
        operation.label()
    );
}

// a model dumps straight to its json form; every write takes this one representation to both tiers
pub fn encode<Value>(value: &Value) -> Result<Json, PayloadError>
where
    Value: Serialize,
{
    serde_json::from_value(
        serde_json::to_value(CacheCodec::encode(value)?).map_err(PayloadError::Json)?,
    )
    .map_err(PayloadError::Json)
}

// a dictionary validates against the requested model first, and any other payload passes through
pub fn encode_validated<Model>(payload: &Json) -> Result<Json, PayloadError>
where
    Model: DeserializeOwned + Serialize,
{
    let wire = serde_json::to_value(payload).map_err(PayloadError::Json)?;
    let value = serde_json::from_value::<CacheValue>(wire).map_err(PayloadError::Json)?;
    let encoded = CacheCodec::encode_validated::<Model>(&value)?;
    serde_json::from_value(serde_json::to_value(encoded).map_err(PayloadError::Json)?)
        .map_err(PayloadError::Json)
}

pub fn decode<Model>(payload: &Json) -> Result<Option<Model>, PayloadError>
where
    Model: DeserializeOwned,
{
    let wire = serde_json::to_value(payload).map_err(PayloadError::Json)?;
    let value = serde_json::from_value::<CacheValue>(wire).map_err(PayloadError::Json)?;
    Ok(CacheCodec::decode(Some(&value)))
}
