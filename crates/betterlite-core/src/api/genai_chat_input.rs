#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "GenAI parts are caller JSON, and serde_json's json! macro builds them with Into"
)]
use std::collections::BTreeSet;

use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiConfigTag, ParameterCapabilities, supported_parameters,
};
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::chat::{ChatMessage, ChatRequest, ToolChoice};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::schema::normalize::normalize_json_schema_types;
use crate::schema::{SchemaError, SchemaLimits};

mod tests;

const DEFAULT_IMAGE_MIME_TYPE: &str = "image/jpeg";
const JSON_MIME_TYPE: &str = "application/json";
const GEMINI_ONLY_SCHEMA_KEYS: [&str; 2] = ["propertyOrdering", "property_ordering"];
// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH, the bound its schema-key filter stops at
const MAX_SCHEMA_FILTER_DEPTH: u32 = 100;
const RESPONSE_SCHEMA_KEYS: [&str; 4] = [
    "responseJsonSchema",
    "response_json_schema",
    "responseSchema",
    "response_schema",
];

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateContentRequest {
    pub model: String,
    pub contents: Value,
    #[serde(rename = "systemInstruction", alias = "system_instruction")]
    pub system_instruction: Option<Value>,
    pub tools: Option<Vec<Value>>,
    #[serde(rename = "toolConfig", alias = "tool_config")]
    pub tool_config: Option<Value>,
    #[serde(rename = "generationConfig", alias = "config")]
    pub generation_config: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GenerateLiteLLMParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub max_retries: Option<u32>,
    pub organization: Option<String>,
}

pub struct TranslationContext<'call> {
    pub resolution: &'call ResolutionContext<'call>,
    pub limits: SchemaLimits,
}

#[derive(Debug, Clone)]
pub struct TranslatedCompletion {
    pub request: ChatRequest,
    // LiteLLM merges GenericLiteLLMParams into the completion kwargs; betterlite keeps its secrets typed here
    pub litellm_params: GenerateLiteLLMParams,
}

#[derive(Debug, Error)]
pub enum GenAiInputError {
    #[error("the GenAI schema cannot be used: {source}")]
    Schema {
        #[from]
        source: SchemaError,
    },
    #[error("the GenAI request cannot be encoded: {source}")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
}

pub fn translate_generate_content(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> Result<TranslatedCompletion, GenAiInputError> {
    let config = request
        .generation_config
        .as_ref()
        .and_then(Value::as_object);
    let mut response_format = response_format(config, &context.limits)?;
    if response_format.is_some() && !supports_response_format(request, litellm_params, context) {
        response_format = None;
    }
    let tools = match request.tools.as_deref() {
        Some(tools) => tools_to_openai(tools, &context.limits)?,
        None => Vec::new(),
    };
    let completion = json!({
        "model": request.model.clone(),
        "messages": messages(&request.contents, request.system_instruction.as_ref())?,
        "temperature": config.and_then(|fields| find(fields, &["temperature"])),
        "top_p": config.and_then(|fields| find(fields, &["topP"])),
        "max_tokens": config.and_then(|fields| find(fields, &["maxOutputTokens"])),
        "stop": config.and_then(|fields| find(fields, &["stopSequences"])),
        "tools": (!tools.is_empty()).then_some(tools),
        "tool_choice": tool_choice(request.tool_config.as_ref()),
        "response_format": response_format,
    });
    Ok(TranslatedCompletion {
        request: serde_json::from_value(completion)?,
        litellm_params: litellm_params.cloned().unwrap_or_default(),
    })
}

// _first_present: the first key that is present with a value other than null
fn find<'call>(
    fields: &'call serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'call Value> {
    keys.iter()
        .filter_map(|key| fields.get(*key))
        .find(|value| !value.is_null())
}

fn messages(
    contents: &Value,
    system_instruction: Option<&Value>,
) -> Result<Vec<ChatMessage>, GenAiInputError> {
    let mut messages = Vec::new();
    if let Some(text) = system_instruction
        .and_then(Value::as_object)
        .and_then(|instruction| find(instruction, &["parts"]))
        .and_then(Value::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(Value::as_str)
    {
        let system = json!({"role": "system", "content": text});
        messages.push(serde_json::from_value(system)?);
    }
    let items: Vec<&Value> = match contents {
        Value::Array(items) => items.iter().collect(),
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_)) => {
            vec![other]
        }
    };
    for content in items {
        let role = content
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let parts = content.get("parts").and_then(Value::as_array);
        let parts_slice = parts.map_or(&[][..], Vec::as_slice);
        match role {
            "user" => user_messages(parts_slice, &mut messages)?,
            "model" => model_message(parts_slice, &mut messages)?,
            // LiteLLM transforms only the user and model roles and drops every other role without a word
            _other => {}
        }
    }
    Ok(messages)
}

fn user_messages(parts: &[Value], messages: &mut Vec<ChatMessage>) -> Result<(), GenAiInputError> {
    let mut content_parts = Vec::new();
    let mut tool_messages = Vec::new();
    for part in parts {
        let Some(fields) = part.as_object() else {
            if let Some(text) = part.as_str() {
                content_parts.push(json!({"type": "text", "text": text}));
            }
            continue;
        };
        if let Some(text) = fields.get("text").and_then(Value::as_str) {
            content_parts.push(json!({"type": "text", "text": text}));
            continue;
        }
        if let Some(inline) = fields.get("inline_data") {
            content_parts.push(image_part(inline));
            continue;
        }
        if let Some(response) = fields.get("functionResponse") {
            tool_messages.push(tool_message(response)?);
        }
    }
    if !content_parts.is_empty() {
        let content = match content_parts.as_slice() {
            [part] if part.get("type").and_then(Value::as_str) == Some("text") => {
                part.get("text").cloned().unwrap_or_default()
            }
            _ => Value::Array(content_parts),
        };
        let user = json!({"role": "user", "content": content});
        messages.push(serde_json::from_value(user)?);
    }
    messages.extend(tool_messages);
    Ok(())
}

fn image_part(inline: &Value) -> Value {
    let mime_type = inline
        .get("mime_type")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_IMAGE_MIME_TYPE);
    let data = inline
        .get("data")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({"type": "image_url", "image_url": {"url": format!("data:{mime_type};base64,{data}")}})
}

fn tool_message(response: &Value) -> Result<ChatMessage, GenAiInputError> {
    let name = response
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let content = encode_json(response.get("response").filter(|value| !value.is_null()))?;
    let id = format!("call_{name}");
    let tool = json!({"role": "tool", "tool_call_id": id, "content": content});
    Ok(serde_json::from_value(tool)?)
}

fn encode_json(value: Option<&Value>) -> Result<String, GenAiInputError> {
    match value {
        Some(given) => Ok(serde_json::to_string(given)?),
        None => Ok(String::from("{}")),
    }
}

fn model_message(parts: &[Value], messages: &mut Vec<ChatMessage>) -> Result<(), GenAiInputError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        let Some(fields) = part.as_object() else {
            if let Some(part_text) = part.as_str() {
                text.push_str(part_text);
            }
            continue;
        };
        if let Some(part_text) = fields.get("text").and_then(Value::as_str) {
            text.push_str(part_text);
            continue;
        }
        if let Some(call) = fields.get("functionCall") {
            tool_calls.push(tool_call(call)?);
        }
    }
    let content = (!text.is_empty()).then_some(Value::String(text));
    let calls = (!tool_calls.is_empty()).then_some(tool_calls);
    let assistant = json!({"role": "assistant", "content": content, "tool_calls": calls});
    messages.push(serde_json::from_value(assistant)?);
    Ok(())
}

fn tool_call(call: &Value) -> Result<Value, GenAiInputError> {
    let named = call.get("name").and_then(Value::as_str);
    let name = named.unwrap_or_default();
    let arguments = encode_json(call.get("args").filter(|value| !value.is_null()))?;
    let id = format!("call_{}", named.unwrap_or("unknown"));
    Ok(json!({
        "id": id,
        "type": "function",
        "function": {"name": name, "arguments": arguments},
    }))
}

fn tool_choice(config: Option<&Value>) -> Option<ToolChoice> {
    let given = config
        .filter(|given| !given.is_null())
        .and_then(Value::as_object)?;
    let mode = find(given, &["functionCallingConfig"])
        .and_then(|fields| fields.get("mode"))
        .and_then(Value::as_str);
    let choice = match mode {
        Some("ANY") => "required",
        Some("NONE") => "none",
        // LiteLLM's map holds AUTO and falls back to auto for a mode it does not know
        Some(_) | None => "auto",
    };
    Some(ToolChoice::Mode(String::from(choice)))
}

fn tools_to_openai(tools: &[Value], limits: &SchemaLimits) -> Result<Vec<Value>, GenAiInputError> {
    let mut openai_tools = Vec::new();
    for tool in tools {
        let Some(declarations) = tool.get("functionDeclarations").and_then(Value::as_array) else {
            continue;
        };
        for declaration in declarations {
            let given = declaration
                .as_object()
                .and_then(|fields| find(fields, &["parametersJsonSchema", "parameters"]));
            let parameters = match given.filter(|candidate| candidate.is_object()) {
                Some(candidate) => Some(normalize_json_schema_types(candidate, limits)?),
                None => None,
            };
            let name = declaration
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let description = declaration.get("description").and_then(Value::as_str);
            let function =
                json!({"name": name, "description": description, "parameters": parameters});
            openai_tools.push(json!({"type": "function", "function": function}));
        }
    }
    Ok(openai_tools)
}

fn response_format(
    config: Option<&serde_json::Map<String, Value>>,
    limits: &SchemaLimits,
) -> Result<Option<Value>, GenAiInputError> {
    let Some(fields) = config else {
        return Ok(None);
    };
    if let Some(mime_type) = find(fields, &["responseMimeType", "response_mime_type"])
        && mime_type.as_str() != Some(JSON_MIME_TYPE)
    {
        return Ok(None);
    }
    let given = find(fields, &RESPONSE_SCHEMA_KEYS).filter(|schema| schema.is_object());
    let Some(schema) = given else {
        return Ok(None);
    };
    let normalized = normalize_json_schema_types(schema, limits)?;
    if normalized.get("type").and_then(Value::as_str) != Some("object") {
        return Ok(None);
    }
    let cleaned = without_gemini_keys(&normalized, 0);
    let json_schema = json!({"name": "response", "schema": cleaned});
    Ok(Some(
        json!({"type": "json_schema", "json_schema": json_schema}),
    ))
}

// LiteLLM's filter_value_from_dict walks dicts and the dicts a list carries, and nothing deeper
fn without_gemini_keys(schema: &Value, depth: u32) -> Value {
    match schema {
        Value::Object(_) | Value::Array(_) if depth > MAX_SCHEMA_FILTER_DEPTH => schema.clone(),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| !GEMINI_ONLY_SCHEMA_KEYS.contains(&key.as_str()))
                .map(|(key, value)| {
                    (
                        key.clone(),
                        without_gemini_keys(value, depth.saturating_add(1)),
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| match item {
                    Value::Object(_) => without_gemini_keys(item, depth.saturating_add(1)),
                    other @ (Value::Null
                    | Value::Bool(_)
                    | Value::Number(_)
                    | Value::String(_)
                    | Value::Array(_)) => other.clone(),
                })
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => schema.clone(),
    }
}

fn supports_response_format(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> bool {
    let custom_provider = litellm_params.and_then(|params| params.custom_llm_provider.as_deref());
    let resolution = ResolutionRequest {
        model: &request.model,
        // an empty declaration names no provider, as LiteLLM's falsy check treats it
        custom_llm_provider: custom_provider.filter(|name| !name.is_empty()),
        api_base: None,
        api_key: None,
    };
    let Ok(ResolvedModel {
        provider: ProviderIdentity::Known(provider),
        model,
        ..
    }) = resolve(&resolution, context.resolution)
    else {
        // LiteLLM keeps response_format when get_llm_provider raises BadRequestError
        return true;
    };
    let capabilities = BTreeSet::new();
    let query = CapabilityQuery {
        provider,
        call_type: CallType::ChatCompletion,
        config: OpenAiConfigTag::UnknownModel,
        model: ModelProjection {
            model: &model,
            capabilities: &capabilities,
        },
        base_model: None,
    };
    match supported_parameters(&query) {
        // a provider whose adapter betterlite has not built has no parameter list, and LiteLLM keeps the field
        ParameterCapabilities::Absent => true,
        ParameterCapabilities::Listed(names) => names.contains(&"response_format"),
    }
}
