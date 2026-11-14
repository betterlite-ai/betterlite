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
                request.model = extra_value("model", "a string", value)?.ok_or(
                    ResponsesTransformError::FieldType {
                        field: "model",
                        expected: "a string",
                        source: None,
                    },
                )?;
            }
            "input" => request.input = explicit_value(value),
            "instructions" => {
                request.instructions = extra_value("instructions", "a string", value)?;
            }
            "tools" => {
                request.tools =
                    extra_value::<Vec<JsonValue>>("tools", "a list", value)?.map(JsonValue::Array);
            }
            "temperature" => {
                request.temperature = extra_value("temperature", "a number", value)?;
            }
            "top_p" => request.top_p = extra_value("top_p", "a number", value)?,
            "max_output_tokens" => {
                request.max_output_tokens = extra_value("max_output_tokens", "an integer", value)?;
            }
            "stream" => request.stream = extra_value("stream", "a boolean", value)?,
            "store" => request.store = extra_value("store", "a boolean", value)?,
            "metadata" => request.metadata = explicit_value(value),
            "user" => request.user = extra_value("user", "a string", value)?,
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

fn extra_value<T>(
    field: &'static str,
    expected: &'static str,
    value: &JsonValue,
) -> Result<Option<T>, ResponsesTransformError>
where
    T: DeserializeOwned,
{
    if matches!(value, JsonValue::Null) {
        return Ok(None);
    }
    let encoded =
        serde_json::to_vec(value).map_err(|source| ResponsesTransformError::Encode { source })?;
    serde_json::from_slice(&encoded)
        .map(Some)
        .map_err(|source| ResponsesTransformError::FieldType {
            field,
            expected,
            source: Some(source),
        })
}
