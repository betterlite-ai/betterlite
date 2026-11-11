#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "GenAI caller JSON"
)]
use crate::schema::{SchemaError, SchemaLimits, normalize::normalize_json_schema_types};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiConfigTag, ParameterCapabilities, supported_parameters,
};
use betterlite_openai::chat::{ChatRequest, ToolChoice};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, slice};
use thiserror::Error;
mod tests;
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequest {
    pub model: String,
    pub contents: Value,
    pub system_instruction: Option<Value>,
    pub tools: Option<Vec<Value>>,
    pub tool_config: Option<Value>,
    pub generation_config: Option<Value>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateLiteLLMParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub max_retries: Option<u32>,
}
// LiteLLM merges GenericLiteLLMParams into the completion kwargs; betterlite returns it typed beside the request
pub type TranslatedCompletion = (ChatRequest, Option<GenerateLiteLLMParams>);
#[derive(Debug, Error)]
pub enum GenAiInputError {
    #[error("the GenAI schema cannot be used: {0}")]
    Schema(#[from] SchemaError),
    #[error("the GenAI request cannot be encoded: {0}")]
    Encode(#[from] serde_json::Error),
}
pub fn translate_generate_content(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    limits: SchemaLimits,
) -> Result<TranslatedCompletion, GenAiInputError> {
    let config = request.generation_config.as_ref();
    let custom = litellm_params.and_then(|params| params.custom_llm_provider.as_deref());
    let mut response_format = response_format(config, &limits)?;
    if response_format.is_some() && !supports_response_format(&request.model, custom) {
        response_format = None;
    }
    let tools = match request.tools.as_deref() {
        Some(tools) => tools_to_openai(tools, &limits)?,
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
    Ok((serde_json::from_value(completion)?, litellm_params.cloned()))
}
// _first_present: the first key that is present with a value other than null
fn find<'call>(fields: &'call Value, keys: &[&str]) -> Option<&'call Value> {
    keys.iter()
        .filter_map(|key| fields.get(*key))
        .find(|value| !value.is_null())
}
fn messages(
    contents: &Value,
    system_instruction: Option<&Value>,
) -> Result<Vec<Value>, GenAiInputError> {
    let mut messages = Vec::new();
    let system_parts = system_instruction.and_then(|given| find(given, &["parts"]));
    let first = system_parts
        .and_then(Value::as_array)
        .and_then(|parts| parts.first());
    if let Some(text) = first
        .and_then(|part| part.get("text"))
        .and_then(Value::as_str)
    {
        messages.push(json!({"role": "system", "content": text}));
    }
    let items = contents
        .as_array()
        .map_or_else(|| slice::from_ref(contents), Vec::as_slice);
    for content in items {
        let parts = content
            .get("parts")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let role = content
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        match role {
            "user" => user_messages(parts, &mut messages)?,
            "model" => model_message(parts, &mut messages)?,
            // LiteLLM transforms only the user and model roles and drops every other role without a word
            _other => {}
        }
    }
    Ok(messages)
}
fn user_messages(parts: &[Value], messages: &mut Vec<Value>) -> Result<(), GenAiInputError> {
    let mut content_parts = Vec::new();
    let mut tool_messages = Vec::new();
    for part in parts {
        match part {
            Value::String(text) => content_parts.push(json!({"type": "text", "text": text})),
            Value::Object(fields) => match (
                fields.get("text"),
                fields.get("inline_data"),
                fields.get("functionResponse"),
            ) {
                (Some(Value::String(text)), _, _) => {
                    content_parts.push(json!({"type": "text", "text": text}));
                }
                (_, Some(inline), _) => {
                    let mime = inline.get("mime_type").and_then(Value::as_str);
                    let data = inline
                        .get("data")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let url = format!("data:{};base64,{data}", mime.unwrap_or("image/jpeg"));
                    content_parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
                }
                (_, _, Some(response)) => tool_messages.push(tool_message(response)?),
                (Some(_) | None, None, None) => {}
            },
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {}
        }
    }
    if !content_parts.is_empty() {
        let lone_text = match content_parts.as_slice() {
            [part] if part.get("type").and_then(Value::as_str) == Some("text") => {
                part.get("text").and_then(Value::as_str).map(str::to_owned)
            }
            _ => None,
        };
        let content = lone_text.map_or_else(|| Value::Array(content_parts), Value::String);
        messages.push(json!({"role": "user", "content": content}));
    }
    messages.extend(tool_messages);
    Ok(())
}
fn tool_message(response: &Value) -> Result<Value, GenAiInputError> {
    let name = response
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let content = encode_json(response.get("response").filter(|value| !value.is_null()))?;
    Ok(json!({"role": "tool", "tool_call_id": format!("call_{name}"), "content": content}))
}
fn encode_json(value: Option<&Value>) -> Result<String, GenAiInputError> {
    let Some(given) = value else {
        return Ok(String::from("{}"));
    };
    Ok(serde_json::to_string(given)?)
}
fn model_message(parts: &[Value], messages: &mut Vec<Value>) -> Result<(), GenAiInputError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            Value::String(part_text) => text.push_str(part_text),
            Value::Object(fields) => match (fields.get("text"), fields.get("functionCall")) {
                (Some(Value::String(part_text)), _) => text.push_str(part_text),
                (_, Some(call)) => tool_calls.push(tool_call(call)?),
                (Some(_) | None, None) => {}
            },
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {}
        }
    }
    let content = (!text.is_empty()).then_some(Value::String(text));
    let calls = (!tool_calls.is_empty()).then_some(tool_calls);
    messages.push(json!({"role": "assistant", "content": content, "tool_calls": calls}));
    Ok(())
}
fn tool_call(call: &Value) -> Result<Value, GenAiInputError> {
    let named = call.get("name").and_then(Value::as_str);
    let name = named.unwrap_or_default();
    let arguments = encode_json(call.get("args").filter(|value| !value.is_null()))?;
    let id = format!("call_{}", named.unwrap_or("unknown"));
    Ok(json!({"id": id, "type": "function", "function": {"name": name, "arguments": arguments}}))
}
fn tool_choice(config: Option<&Value>) -> Option<ToolChoice> {
    let given = config.filter(|value| !value.is_null())?;
    let fcc = find(given, &["functionCallingConfig"]);
    let mode = fcc
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
    let declarations = tools
        .iter()
        .filter_map(|tool| tool.get("functionDeclarations"))
        .filter_map(Value::as_array)
        .flatten();
    for declaration in declarations {
        let given = find(declaration, &["parametersJsonSchema", "parameters"]);
        let parameters = match given.filter(|candidate| candidate.is_object()) {
            Some(candidate) => Some(normalize_json_schema_types(candidate, limits)?),
            None => None,
        };
        let name = declaration
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let description = declaration.get("description").and_then(Value::as_str);
        let function = json!({"name": name, "description": description, "parameters": parameters});
        openai_tools.push(json!({"type": "function", "function": function}));
    }
    Ok(openai_tools)
}
fn response_format(
    config: Option<&Value>,
    limits: &SchemaLimits,
) -> Result<Option<Value>, GenAiInputError> {
    let Some(fields) = config else {
        return Ok(None);
    };
    let mime = find(fields, &["responseMimeType", "response_mime_type"]);
    if mime.is_some_and(|given| given.as_str() != Some("application/json")) {
        return Ok(None);
    }
    let keys = [
        "responseJsonSchema",
        "response_json_schema",
        "responseSchema",
        "response_schema",
    ];
    let found = find(fields, &keys);
    let Some(schema) = found.filter(|candidate| candidate.is_object()) else {
        return Ok(None);
    };
    let mut cleaned = normalize_json_schema_types(schema, limits)?;
    if cleaned.get("type").and_then(Value::as_str) != Some("object") {
        return Ok(None);
    }
    without_gemini_keys(&mut cleaned, 0);
    let json_schema = json!({"name": "response", "schema": cleaned});
    Ok(Some(
        json!({"type": "json_schema", "json_schema": json_schema}),
    ))
}
// LiteLLM's filter_value_from_dict walks dicts and the dicts a list carries, and nothing deeper
fn without_gemini_keys(schema: &mut Value, depth: u32) {
    if depth > 100 {
        return;
    }
    if let Some(fields) = schema.as_object_mut() {
        fields.retain(|key, _| !["propertyOrdering", "property_ordering"].contains(&key.as_str()));
        for value in fields.values_mut() {
            without_gemini_keys(value, depth.saturating_add(1));
        }
    }
    if let Some(items) = schema.as_array_mut() {
        for item in items.iter_mut().filter(|item| item.is_object()) {
            without_gemini_keys(item, depth.saturating_add(1));
        }
    }
}
fn supports_response_format(model: &str, custom_provider: Option<&str>) -> bool {
    if custom_provider.is_some_and(|name| !name.is_empty() && name != "openai") {
        return true;
    }
    let name = model.rsplit('/').next().unwrap_or(model);
    match supported_parameters(&CapabilityQuery {
        provider: ProviderId::Openai,
        call_type: CallType::ChatCompletion,
        config: OpenAiConfigTag::UnknownModel,
        model: ModelProjection {
            model: name,
            capabilities: &BTreeSet::new(),
        },
        base_model: None,
    }) {
        ParameterCapabilities::Absent => true,
        ParameterCapabilities::Listed(names) => names.contains(&"response_format"),
    }
}
