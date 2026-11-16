use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("request serialization failed: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("payload {bytes}B exceeds max_payload_bytes {limit}")]
    TooLarge { bytes: usize, limit: usize },
}

pub fn encode<Value>(value: &Value, max_payload_bytes: usize) -> Result<String, PayloadError>
where
    Value: Serialize,
{
    let body = serde_json::to_string(value).map_err(|source| PayloadError::Serialize { source })?;
    if body.len() > max_payload_bytes {
        return Err(PayloadError::TooLarge {
            bytes: body.len(),
            limit: max_payload_bytes,
        });
    }
    Ok(body)
}
