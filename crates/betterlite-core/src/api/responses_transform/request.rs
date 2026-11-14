use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Deserializer, Number};

use super::error::ResponsesTransformError;
use super::input::transform_input;

#[derive(Debug)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: Option<JsonValue>,
    pub instructions: Option<String>,
    pub tools: Option<JsonValue>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub max_output_tokens: Option<i64>,
    pub stream: Option<bool>,
    pub store: Option<bool>,
    pub metadata: Option<JsonValue>,
    pub user: Option<String>,
    // LiteLLM splats its **kwargs into the Responses call unchanged, so a key this bridge doesn't model stays as given
    pub passthrough: Params,
}

// the optional params LiteLLM reads from the Interactions request, typed as the Interactions spec declares them
#[derive(Deserialize)]
struct InteractionFields {
    system_instruction: Option<String>,
    tools: Option<Vec<JsonValue>>,
    generation_config: Option<GenerationConfig>,
    stream: Option<bool>,
    store: Option<bool>,
    metadata: Option<JsonValue>,
    user: Option<String>,
}

#[derive(Deserialize)]
struct GenerationConfig {
    temperature: Option<Number>,
    top_p: Option<Number>,
    max_output_tokens: Option<i64>,
}

pub fn to_responses_request(
    model: &str,
    input: Option<&JsonValue>,
    optional_params: &Params,
    extras: &Params,
) -> Result<ResponsesRequest, ResponsesTransformError> {
    let mut request = ResponsesRequest {
        model: model.to_owned(),
        input: requested_input(input)?,
        instructions: None,
        tools: None,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stream: None,
        store: None,
        metadata: None,
        user: None,
        passthrough: Params::new(),
    };
    apply_optional_params(&mut request, optional_params)?;
    apply_extras(&mut request, extras)?;
    Ok(request)
}

fn requested_input(
    input: Option<&JsonValue>,
) -> Result<Option<JsonValue>, ResponsesTransformError> {
    match input {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => Ok(Some(transform_input(value)?)),
    }
}

fn request_fields<T>(params: &Params) -> Result<T, ResponsesTransformError>
where
    T: DeserializeOwned,
{
    let encoded =
        serde_json::to_vec(params).map_err(|source| ResponsesTransformError::Encode { source })?;
    let mut decoder = Deserializer::from_slice(&encoded);
    serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| ResponsesTransformError::Fields { source })
}

fn apply_optional_params(
    request: &mut ResponsesRequest,
    optional_params: &Params,
) -> Result<(), ResponsesTransformError> {
    let sent: InteractionFields = request_fields(optional_params)?;
    // LiteLLM only sets instructions for a non-empty system_instruction
    request.instructions = sent.system_instruction.filter(|text| !text.is_empty());
    let tools = sent.tools.unwrap_or_default();
    // LiteLLM only forwards a non-empty tools list
    request.tools = (!tools.is_empty()).then_some(JsonValue::Array(tools));
    if let Some(config) = sent.generation_config {
        request.temperature = config.temperature;
        request.top_p = config.top_p;
        // Responses has no top_k, so LiteLLM drops it along with every other generation key
        request.max_output_tokens = config.max_output_tokens;
    }
    request.stream = sent.stream;
    request.store = sent.store;
    request.metadata = sent.metadata;
    request.user = sent.user;
    Ok(())
}

// LiteLLM updates the request dict with its extra kwargs, so an absent key keeps the mapped value while a null clears it
fn apply_extras(
    request: &mut ResponsesRequest,
    extras: &Params,
) -> Result<(), ResponsesTransformError> {
    for (name, value) in extras {
        match name.as_str() {
            "model" => {
                request.model =
                    nullable_text("model", value)?.ok_or(ResponsesTransformError::FieldType {
                        field: "model",
                        expected: "a string",
                    })?;
            }
            "input" => request.input = explicit_value(value),
            "instructions" => request.instructions = nullable_text("instructions", value)?,
            "tools" => request.tools = nullable_tools(value)?,
            "temperature" => request.temperature = nullable_number("temperature", value)?,
            "top_p" => request.top_p = nullable_number("top_p", value)?,
            "max_output_tokens" => {
                request.max_output_tokens = nullable_tokens(value)?;
            }
            "stream" => request.stream = nullable_bool("stream", value)?,
            "store" => request.store = nullable_bool("store", value)?,
            "metadata" => request.metadata = explicit_value(value),
            "user" => request.user = nullable_text("user", value)?,
            _ => {
                request.passthrough.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(())
}

fn explicit_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Text(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Some(value.clone()),
    }
}

fn nullable_text(
    field: &'static str,
    value: &JsonValue,
) -> Result<Option<String>, ResponsesTransformError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Text(text) => Ok(Some(text.clone())),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ResponsesTransformError::FieldType {
                field,
                expected: "a string",
            })
        }
    }
}

fn nullable_bool(
    field: &'static str,
    value: &JsonValue,
) -> Result<Option<bool>, ResponsesTransformError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Bool(flag) => Ok(Some(*flag)),
        JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ResponsesTransformError::FieldType {
                field,
                expected: "a boolean",
            })
        }
    }
}

fn nullable_number(
    field: &'static str,
    value: &JsonValue,
) -> Result<Option<Number>, ResponsesTransformError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Number(number) => Ok(Some(number.clone())),
        JsonValue::Bool(_) | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ResponsesTransformError::FieldType {
                field,
                expected: "a number",
            })
        }
    }
}

fn nullable_tokens(value: &JsonValue) -> Result<Option<i64>, ResponsesTransformError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Number(number) => match number.as_i64() {
            Some(tokens) => Ok(Some(tokens)),
            None => Err(ResponsesTransformError::FieldType {
                field: "max_output_tokens",
                expected: "an integer",
            }),
        },
        JsonValue::Bool(_) | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ResponsesTransformError::FieldType {
                field: "max_output_tokens",
                expected: "an integer",
            })
        }
    }
}

fn nullable_tools(value: &JsonValue) -> Result<Option<JsonValue>, ResponsesTransformError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Array(_) => Ok(Some(value.clone())),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Object(_) => {
            Err(ResponsesTransformError::FieldType {
                field: "tools",
                expected: "a list",
            })
        }
    }
}
