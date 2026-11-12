use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Object;

#[derive(Debug, Error)]
pub enum ResponseDecodeError {
    #[error("the Responses provider response is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Responses provider response is not a JSON object")]
    NotObject,
}

// ast-grep-ignore: no-long-names
pub fn decode_response_capability_error(bytes: &[u8]) -> Result<Object, ResponseDecodeError> {
    match serde_json::from_slice(bytes) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(ResponseDecodeError::NotObject),
        Err(source) => Err(ResponseDecodeError::Json { source }),
    }
}
