use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use serde_json::Number;

use super::error::ResponsesTransformError;
use super::input::transform_input;

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
    apply_instructions(&mut request, optional_params)?;
    apply_tools(&mut request, optional_params)?;
    apply_generation_config(&mut request, optional_params)?;
    apply_passthrough(&mut request, optional_params)?;
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

fn apply_instructions(
    request: &mut ResponsesRequest,
    optional_params: &Params,
) -> Result<(), ResponsesTransformError> {
    let Some(text) = optional_text(
        "system_instruction",
        optional_params.get("system_instruction"),
    )?
    else {
        return Ok(());
    };
    // LiteLLM only sets instructions for a non-empty system_instruction
    if !text.is_empty() {
        request.instructions = Some(text);
    }
    Ok(())
}

fn apply_tools(
    request: &mut ResponsesRequest,
    optional_params: &Params,
) -> Result<(), ResponsesTransformError> {
    match optional_params.get("tools") {
        None | Some(JsonValue::Null) => Ok(()),
        Some(JsonValue::Array(items)) => {
            // LiteLLM only forwards a non-empty tools list
            if !items.is_empty() {
                request.tools = Some(JsonValue::Array(items.clone()));
            }
            Ok(())
        }
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Object(_),
        ) => Err(ResponsesTransformError::NotList { field: "tools" }),
    }
}

fn apply_generation_config(
    request: &mut ResponsesRequest,
    optional_params: &Params,
) -> Result<(), ResponsesTransformError> {
    match optional_params.get("generation_config") {
        None | Some(JsonValue::Null) => Ok(()),
        Some(JsonValue::Object(fields)) => {
            request.temperature =
                optional_number("generation_config.temperature", fields.get("temperature"))?;
            request.top_p = optional_number("generation_config.top_p", fields.get("top_p"))?;
            // Responses has no top_k, so LiteLLM drops it along with every other generation key
            request.max_output_tokens = optional_integer(
                "generation_config.max_output_tokens",
                fields.get("max_output_tokens"),
            )?;
            Ok(())
        }
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => Err(ResponsesTransformError::NotObject {
            field: "generation_config",
        }),
    }
}

fn apply_passthrough(
    request: &mut ResponsesRequest,
    optional_params: &Params,
) -> Result<(), ResponsesTransformError> {
    request.stream = optional_bool("stream", optional_params.get("stream"))?;
    request.store = optional_bool("store", optional_params.get("store"))?;
    request.metadata = optional_value(optional_params.get("metadata"));
    request.user = optional_text("user", optional_params.get("user"))?;
    Ok(())
}

fn apply_extras(
    request: &mut ResponsesRequest,
    extras: &Params,
) -> Result<(), ResponsesTransformError> {
    for (name, value) in extras {
        match name.as_str() {
            "model" => request.model = required_text("model", value)?,
            "input" => request.input = optional_value(Some(value)),
            "instructions" => request.instructions = optional_text("instructions", Some(value))?,
            "tools" => request.tools = optional_list("tools", value)?,
            "temperature" => request.temperature = optional_number("temperature", Some(value))?,
            "top_p" => request.top_p = optional_number("top_p", Some(value))?,
            "max_output_tokens" => {
                request.max_output_tokens = optional_integer("max_output_tokens", Some(value))?;
            }
            "stream" => request.stream = optional_bool("stream", Some(value))?,
            "store" => request.store = optional_bool("store", Some(value))?,
            "metadata" => request.metadata = optional_value(Some(value)),
            "user" => request.user = optional_text("user", Some(value))?,
            _ => {
                request.passthrough.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(())
}

fn required_text(
    field: &'static str,
    value: &JsonValue,
) -> Result<String, ResponsesTransformError> {
    match value {
        JsonValue::Text(text) => Ok(text.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Err(ResponsesTransformError::NotText { field }),
    }
}

fn optional_text(
    field: &'static str,
    value: Option<&JsonValue>,
) -> Result<Option<String>, ResponsesTransformError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => Ok(Some(required_text(field, value)?)),
    }
}

fn optional_bool(
    field: &'static str,
    value: Option<&JsonValue>,
) -> Result<Option<bool>, ResponsesTransformError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(flag)) => Ok(Some(*flag)),
        Some(
            JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ResponsesTransformError::NotBool { field }),
    }
}

fn optional_integer(
    field: &'static str,
    value: Option<&JsonValue>,
) -> Result<Option<i64>, ResponsesTransformError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(ResponsesTransformError::NotInteger { field }),
        Some(
            JsonValue::Bool(_) | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ResponsesTransformError::NotInteger { field }),
    }
}

fn optional_number(
    field: &'static str,
    value: Option<&JsonValue>,
) -> Result<Option<Number>, ResponsesTransformError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => Ok(Some(number.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ResponsesTransformError::NotNumber { field }),
    }
}

fn optional_list(
    field: &'static str,
    value: &JsonValue,
) -> Result<Option<JsonValue>, ResponsesTransformError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Array(items) => Ok(Some(JsonValue::Array(items.clone()))),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Object(_) => {
            Err(ResponsesTransformError::NotList { field })
        }
    }
}

fn optional_value(value: Option<&JsonValue>) -> Option<JsonValue> {
    match value {
        None | Some(JsonValue::Null) => None,
        Some(value) => Some(value.clone()),
    }
}
