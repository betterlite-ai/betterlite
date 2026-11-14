use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Deserializer, Number};
use serde_with::rust::double_option;

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

// the extra kwargs are the Responses request's own keys, so an absent one differs from an explicit null
#[derive(Deserialize)]
struct ExtraFields {
    #[serde(default, with = "double_option")]
    model: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    input: Option<Option<JsonValue>>,
    #[serde(default, with = "double_option")]
    instructions: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    tools: Option<Option<Vec<JsonValue>>>,
    #[serde(default, with = "double_option")]
    temperature: Option<Option<Number>>,
    #[serde(default, with = "double_option")]
    top_p: Option<Option<Number>>,
    #[serde(default, with = "double_option")]
    max_output_tokens: Option<Option<i64>>,
    #[serde(default, with = "double_option")]
    stream: Option<Option<bool>>,
    #[serde(default, with = "double_option")]
    store: Option<Option<bool>>,
    #[serde(default, with = "double_option")]
    metadata: Option<Option<JsonValue>>,
    #[serde(default, with = "double_option")]
    user: Option<Option<String>>,
}

const EXTRA_FIELDS: [&str; 11] = [
    "model",
    "input",
    "instructions",
    "tools",
    "temperature",
    "top_p",
    "max_output_tokens",
    "stream",
    "store",
    "metadata",
    "user",
];

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

fn request_fields<T: DeserializeOwned>(params: &Params) -> Result<T, ResponsesTransformError> {
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

fn apply_extras(
    request: &mut ResponsesRequest,
    extras: &Params,
) -> Result<(), ResponsesTransformError> {
    let sent: ExtraFields = request_fields(extras)?;
    apply_extra_fields(request, sent)?;
    for (name, value) in extras {
        if !EXTRA_FIELDS.contains(&name.as_str()) {
            request.passthrough.insert(name.clone(), value.clone());
        }
    }
    Ok(())
}

fn apply_extra_fields(
    request: &mut ResponsesRequest,
    sent: ExtraFields,
) -> Result<(), ResponsesTransformError> {
    if let Some(model) = sent.model {
        request.model = model.ok_or(ResponsesTransformError::NotText { field: "model" })?;
    }
    if let Some(input) = sent.input {
        request.input = input;
    }
    if let Some(instructions) = sent.instructions {
        request.instructions = instructions;
    }
    if let Some(tools) = sent.tools {
        request.tools = tools.map(JsonValue::Array);
    }
    if let Some(temperature) = sent.temperature {
        request.temperature = temperature;
    }
    if let Some(top_p) = sent.top_p {
        request.top_p = top_p;
    }
    if let Some(max_output_tokens) = sent.max_output_tokens {
        request.max_output_tokens = max_output_tokens;
    }
    if let Some(stream) = sent.stream {
        request.stream = stream;
    }
    if let Some(store) = sent.store {
        request.store = store;
    }
    if let Some(metadata) = sent.metadata {
        request.metadata = metadata;
    }
    if let Some(user) = sent.user {
        request.user = user;
    }
    Ok(())
}
