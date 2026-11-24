use betterlite_core::api::batch_provider_create::timeout::{HttpTimeout, RequestTimeout};
use betterlite_core::api::batch_provider_read::BatchReadTimeoutValue;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::batches::sdk::TimeoutPhases;

use super::ConvertError;

const CREATE_SHAPES: &str = "seconds or the connect/read/write/pool phases of an httpx.Timeout";
const READ_SHAPES: &str = "seconds, a number written as text, or the phases of an httpx.Timeout";

enum Shape {
    Seconds(f64),
    Text(String),
    Phases(HttpTimeout),
}

fn seconds(number: &serde_json::Number) -> Result<f64, ConvertError> {
    number
        .as_f64()
        .ok_or_else(|| ConvertError::Timeout(format!("{number} is not a number of seconds")))
}

// the timeout crosses the payload as a number, as text or as phases, and each call takes some of them
fn shape(value: &JsonValue, refusal: &'static str) -> Result<Shape, ConvertError> {
    match value {
        JsonValue::Number(number) => Ok(Shape::Seconds(seconds(number)?)),
        JsonValue::Text(text) => Ok(Shape::Text(text.clone())),
        JsonValue::Object(_) => {
            let text =
                serde_json::to_string(value).map_err(|source| ConvertError::Write { source })?;
            let http: HttpTimeout = serde_json::from_str(&text)
                .map_err(|source| ConvertError::Timeout(source.to_string()))?;
            Ok(Shape::Phases(http))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) => {
            Err(ConvertError::Shapes(refusal))
        }
    }
}

pub(super) fn create(value: &JsonValue) -> Result<RequestTimeout, ConvertError> {
    match shape(value, CREATE_SHAPES)? {
        Shape::Seconds(seconds) => Ok(RequestTimeout::Seconds(seconds)),
        Shape::Phases(http) => Ok(RequestTimeout::Client(http)),
        Shape::Text(_) => Err(ConvertError::Shapes(CREATE_SHAPES)),
    }
}

pub(super) fn read(value: &JsonValue) -> Result<BatchReadTimeoutValue, ConvertError> {
    match shape(value, READ_SHAPES)? {
        Shape::Seconds(seconds) => Ok(BatchReadTimeoutValue::Seconds(seconds)),
        Shape::Text(text) => Ok(BatchReadTimeoutValue::Text(text)),
        Shape::Phases(http) => Ok(BatchReadTimeoutValue::Phases(TimeoutPhases {
            connect: http.connect,
            read: http.read,
            write: http.write,
            pool: http.pool,
        })),
    }
}
