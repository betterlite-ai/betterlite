use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::request_form::scalars::scalar_float;

use super::error::CreateError;

// The two Json enums and the provider contract's maps convert through their serde forms
pub(crate) fn round_trip<Source, Target>(value: &Source) -> Result<Target, CreateError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    serde_json::from_str(&serde_json::to_string(value).map_err(CreateError::Encode)?)
        .map_err(CreateError::Encode)
}

// Python's type names, as float() and the type errors print them
pub(crate) trait ValueKind {
    fn value_kind(&self) -> &'static str;
}

// The two Json enums carry the same variants, so one body implements both
macro_rules! value_kinds {
    ($($json:ty),+ $(,)?) => {$(
        impl ValueKind for $json {
            fn value_kind(&self) -> &'static str {
                match self {
                    Self::Null => "NoneType",
                    Self::Bool(_) => "bool",
                    Self::Text(_) => "str",
                    Self::List(_) => "list",
                    Self::Object(_) => "dict",
                    Self::Number(number) => number_kind(&number.to_string()),
                }
            }
        }
    )+};
}

value_kinds!(Json, PromptJson);

fn number_kind(text: &str) -> &'static str {
    if text.contains(['.', 'e', 'E']) {
        "float"
    } else {
        "int"
    }
}

// float(value) for the shapes an optional timer can carry; Display is Python's own message
#[derive(Debug, thiserror::Error)]
pub(crate) enum FloatError {
    #[error("float() argument must be a string or a real number, not '{kind}'")]
    NotANumber { kind: &'static str },
    #[error("could not convert string to float: '{text}'")]
    Text { text: String },
}

pub(crate) fn float_value(value: &PromptJson) -> Result<f64, FloatError> {
    match value {
        PromptJson::Bool(flag) => Ok(if *flag { 1.0_f64 } else { 0.0_f64 }),
        PromptJson::Number(number) => Ok(number.as_f64().unwrap_or(f64::INFINITY)),
        PromptJson::Text(text) => {
            scalar_float(text).ok_or_else(|| FloatError::Text { text: text.clone() })
        }
        other @ (PromptJson::Null | PromptJson::List(_) | PromptJson::Object(_)) => {
            Err(FloatError::NotANumber {
                kind: other.value_kind(),
            })
        }
    }
}
