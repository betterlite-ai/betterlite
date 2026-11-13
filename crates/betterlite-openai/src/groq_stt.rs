use serde_json::{Map, Value};
use thiserror::Error;

mod tests;

const RESPONSE_FORMAT: &str = "response_format";

pub const SUPPORTED_PARAMS: [&str; 4] = ["prompt", "response_format", "temperature", "language"];
pub const SUPPORTED_RESPONSE_FORMATS: [&str; 3] = ["json", "verbose_json", "text"];

#[derive(Debug, Error)]
pub enum GroqSttError {
    #[error(
        "Groq doesn't support response_format={value}. To drop unsupported openai params from the call, set `litellm.drop_params = True`"
    )]
    UnsupportedResponseFormat { value: String },
}

impl GroqSttError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedResponseFormat { .. } => 400,
        }
    }
}

pub struct MapParams<'request> {
    pub non_default: &'request Map<String, Value>,
    pub optional_params: &'request Map<String, Value>,
    pub drop_params: bool,
    pub global_drop_params: bool,
}

pub fn map_openai_params_stt(request: &MapParams<'_>) -> Result<Map<String, Value>, GroqSttError> {
    let mut optional_params = request.optional_params.clone();
    for (name, value) in request.non_default {
        if name == RESPONSE_FORMAT {
            let supported = value
                .as_str()
                .is_some_and(|format| SUPPORTED_RESPONSE_FORMATS.contains(&format));
            if supported {
                optional_params.insert(name.clone(), value.clone());
            } else if !(request.global_drop_params || request.drop_params) {
                return Err(GroqSttError::UnsupportedResponseFormat {
                    value: rendered(value),
                });
            }
        } else {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    Ok(optional_params)
}

fn rendered(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
