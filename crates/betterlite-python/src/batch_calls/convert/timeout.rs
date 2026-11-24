use betterlite_core::api::batch_provider_create::timeout::{HttpTimeout, RequestTimeout};
use betterlite_core::api::batch_provider_read::BatchReadTimeoutValue;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::batches::sdk::TimeoutPhases;
use serde::de::DeserializeOwned;

use super::ConvertError;
use super::encode;

const CREATE_SHAPES: &str =
    "expected seconds or the connect/read/write/pool phases of an httpx.Timeout";
const READ_SHAPES: &str =
    "expected seconds, a number written as text, or the phases of an httpx.Timeout";

// the phases are an HttpTimeout, so its own Deserialize reads them once the tree is clean JSON again
fn typed<T: DeserializeOwned>(value: &JsonValue) -> Result<T, ConvertError> {
    let text = encode(value)?;
    serde_json::from_str(&text).map_err(|source| ConvertError::Timeout(source.to_string()))
}

fn seconds(number: &serde_json::Number) -> Result<f64, ConvertError> {
    number
        .as_f64()
        .ok_or_else(|| ConvertError::Timeout(format!("{number} is not a number of seconds")))
}

pub(super) fn create(value: &JsonValue) -> Result<RequestTimeout, ConvertError> {
    match value {
        JsonValue::Number(number) => Ok(RequestTimeout::Seconds(seconds(number)?)),
        JsonValue::Object(_) => typed(value).map(RequestTimeout::Client),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Text(_) | JsonValue::Array(_) => {
            Err(ConvertError::Timeout(String::from(CREATE_SHAPES)))
        }
    }
}

pub(super) fn read(value: &JsonValue) -> Result<BatchReadTimeoutValue, ConvertError> {
    match value {
        JsonValue::Number(number) => Ok(BatchReadTimeoutValue::Seconds(seconds(number)?)),
        JsonValue::Text(text) => Ok(BatchReadTimeoutValue::Text(text.clone())),
        JsonValue::Object(_) => {
            let http: HttpTimeout = typed(value)?;
            Ok(BatchReadTimeoutValue::Phases(TimeoutPhases {
                connect: http.connect,
                read: http.read,
                write: http.write,
                pool: http.pool,
            }))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) => {
            Err(ConvertError::Timeout(String::from(READ_SHAPES)))
        }
    }
}
