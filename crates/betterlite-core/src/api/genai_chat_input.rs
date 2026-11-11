#![expect(
    clippy::disallowed_types,
    reason = "GenAI parts may carry vendor JSON, and function args, responses and JSON schemas are arbitrary caller JSON"
)]
use std::collections::BTreeSet;
use std::slice;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::capabilities::{CapabilityFlag, CapabilityView};
use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::chat::{
    ChatMessage, ChatRequest, ContentPart, FunctionCall, FunctionDefinition, MessageContent,
    ResponseFormat, Role, Stop, Tool, ToolCall, ToolChoice,
};
use betterlite_openai::openai_base_config::{CatalogCapabilities, ModelFamily, model_family};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Number, Value};
use thiserror::Error;

use crate::schema::normalize::normalize_json_schema_types;
use crate::schema::{SchemaError, SchemaLimits};

mod tests;

const JSON_MIME_TYPE: &str = "application/json";

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Contents {
    One(Content),
    Many(Vec<Content>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Content {
    pub role: Option<String>,
    #[serde(default)]
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text(String),
    TextObject {
        text: String,
    },
    Inline {
        inline_data: InlineData,
    },
    Response {
        #[serde(rename = "functionResponse")]
        function_response: Call,
    },
    Call {
        #[serde(rename = "functionCall")]
        function_call: Call,
    },
    Other(Value),
}

#[derive(Debug, Clone, Deserialize)]
pub struct InlineData {
    pub mime_type: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Call {
    pub name: Option<String>,
    pub args: Option<Value>,
    pub response: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenAiTool {
    #[serde(rename = "functionDeclarations", default)]
    pub function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameters: Option<Value>,
    pub parameters_json_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: Option<FunctionCallingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemInstruction {
    #[serde(default)]
    pub parts: Vec<SystemPart>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemPart {
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    pub temperature: Option<Number>,
    pub max_output_tokens: Option<u32>,
    pub top_p: Option<Number>,
    pub stop_sequences: Option<Vec<String>>,
    #[serde(alias = "response_mime_type")]
    pub response_mime_type: Option<String>,
    #[serde(alias = "response_json_schema")]
    pub response_json_schema: Option<Value>,
    #[serde(alias = "response_schema")]
    pub response_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateLiteLLMParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub max_retries: Option<u32>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateContentRequest {
    pub model: String,
    pub contents: Contents,
    #[serde(rename = "systemInstruction")]
    pub system_instruction: Option<SystemInstruction>,
    #[serde(rename = "system_instruction")]
    pub system_instruction_snake: Option<SystemInstruction>,
    #[serde(default)]
    pub tools: Vec<GenAiTool>,
    #[serde(rename = "toolConfig")]
    pub tool_config: Option<ToolConfig>,
    #[serde(rename = "tool_config")]
    pub tool_config_snake: Option<ToolConfig>,
    #[serde(rename = "generationConfig")]
    pub generation_config: Option<GenerationConfig>,
}

pub struct TranslationContext<'call> {
    pub resolution: &'call ResolutionContext<'call>,
    pub model_capabilities: &'call CatalogCapabilities,
    pub limits: SchemaLimits,
}

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
    context: &TranslationContext<'_>,
) -> Result<TranslatedCompletion, GenAiInputError> {
    let system = request
        .system_instruction
        .as_ref()
        .or(request.system_instruction_snake.as_ref());
    let response_format = supported_response_format(request, litellm_params, context)?;
    let tools = tools_to_openai(&request.tools, &context.limits)?;
    let completion = completion_request(request, system, tools, response_format)?;
    Ok((completion, litellm_params.cloned()))
}

fn completion_request(
    request: &GenerateContentRequest,
    system: Option<&SystemInstruction>,
    tools: Vec<Tool>,
    response_format: Option<ResponseFormat>,
) -> Result<ChatRequest, GenAiInputError> {
    let config = request.generation_config.as_ref();
    Ok(ChatRequest {
        model: Some(request.model.clone()),
        messages: messages(&request.contents, system)?,
        stream: false,
        stream_options: None,
        temperature: config.and_then(|fields| fields.temperature.clone()),
        top_p: config.and_then(|fields| fields.top_p.clone()),
        presence_penalty: None,
        frequency_penalty: None,
        choice_count: None,
        max_tokens: config.and_then(|fields| fields.max_output_tokens),
        max_completion_tokens: None,
        seed: None,
        stop: config
            .and_then(|fields| fields.stop_sequences.clone())
            .map(Stop::Many),
        logprobs: None,
        top_logprobs: None,
        user: None,
        service_tier: None,
        reasoning_effort: None,
        tools: (!tools.is_empty()).then_some(tools),
        tool_choice: tool_choice(
            request
                .tool_config
                .as_ref()
                .or(request.tool_config_snake.as_ref()),
        ),
        parallel_tool_calls: None,
        response_format,
        passthrough: IndexMap::new(),
    })
}

fn supported_response_format(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> Result<Option<ResponseFormat>, GenAiInputError> {
    let mut format = response_format(request.generation_config.as_ref(), &context.limits)?;
    if format.is_some() && !supports_response_format(request, litellm_params, context) {
        format = None;
    }
    Ok(format)
}

fn messages(
    contents: &Contents,
    system: Option<&SystemInstruction>,
) -> Result<Vec<ChatMessage>, GenAiInputError> {
    let mut messages = Vec::new();
    let system_text = system
        .and_then(|instruction| instruction.parts.first())
        .and_then(|part| part.text.as_deref());
    if let Some(text) = system_text {
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(MessageContent::Text(text.to_owned())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        });
    }
    let items = match contents {
        Contents::One(content) => slice::from_ref(content),
        Contents::Many(many) => many,
    };
    for content in items {
        match content.role.as_deref().unwrap_or("user") {
            "user" => user_messages(content, &mut messages)?,
            "model" => messages.push(model_message(content)?),
            // LiteLLM transforms only the user and model roles and drops every other role without a word
            _other => {}
        }
    }
    Ok(messages)
}

fn user_messages(
    content: &Content,
    messages: &mut Vec<ChatMessage>,
) -> Result<(), GenAiInputError> {
    let mut parts = Vec::new();
    let mut tool_messages = Vec::new();
    for part in &content.parts {
        match part {
            Part::Text(text) | Part::TextObject { text } => parts.push(ContentPart {
                kind: String::from("text"),
                fields: IndexMap::from([(String::from("text"), Value::String(text.clone()))]),
            }),
            Part::Inline { inline_data } => parts.push(image_part(inline_data)),
            Part::Response { function_response } => {
                tool_messages.push(tool_message(function_response)?);
            }
            Part::Call { .. } | Part::Other(_) => {}
        }
    }
    if !parts.is_empty() {
        let message_content = match parts.as_slice() {
            [part] if part.kind == "text" => MessageContent::Text(
                part.fields
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
            _ => MessageContent::Parts(parts),
        };
        messages.push(ChatMessage {
            role: Role::User,
            content: Some(message_content),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        });
    }
    messages.extend(tool_messages);
    Ok(())
}

fn image_part(inline: &InlineData) -> ContentPart {
    let mime = inline.mime_type.as_deref().unwrap_or("image/jpeg");
    let data = inline.data.as_deref().unwrap_or_default();
    let mut image_url = serde_json::Map::new();
    drop(image_url.insert(
        String::from("url"),
        Value::String(format!("data:{mime};base64,{data}")),
    ));
    ContentPart {
        kind: String::from("image_url"),
        fields: IndexMap::from([(String::from("image_url"), Value::Object(image_url))]),
    }
}

fn tool_message(response: &Call) -> Result<ChatMessage, GenAiInputError> {
    let name = response.name.as_deref().unwrap_or("unknown");
    Ok(ChatMessage {
        role: Role::Tool,
        content: Some(MessageContent::Text(encode_json(
            response.response.as_ref(),
        )?)),
        name: None,
        tool_calls: None,
        tool_call_id: Some(format!("call_{name}")),
        passthrough: IndexMap::new(),
    })
}

fn encode_json(value: Option<&Value>) -> Result<String, GenAiInputError> {
    let Some(given) = value else {
        return Ok(String::from("{}"));
    };
    Ok(serde_json::to_string(given)?)
}

fn model_message(content: &Content) -> Result<ChatMessage, GenAiInputError> {
    let text: String = content
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) | Part::TextObject { text } => Some(text.as_str()),
            Part::Inline { .. } | Part::Response { .. } | Part::Call { .. } | Part::Other(_) => {
                None
            }
        })
        .collect();
    let mut tool_calls = Vec::new();
    for part in &content.parts {
        if let Part::Call { function_call } = part {
            tool_calls.push(tool_call(function_call)?);
        }
    }
    Ok(ChatMessage {
        role: Role::Assistant,
        content: (!text.is_empty()).then(|| MessageContent::Text(text)),
        name: None,
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        tool_call_id: None,
        passthrough: IndexMap::new(),
    })
}

fn tool_call(call: &Call) -> Result<ToolCall, GenAiInputError> {
    let name = call.name.clone().unwrap_or_default();
    let id = format!("call_{}", call.name.as_deref().unwrap_or("unknown"));
    Ok(ToolCall {
        id,
        kind: String::from("function"),
        function: FunctionCall {
            name,
            arguments: encode_json(call.args.as_ref())?,
        },
    })
}

fn tool_choice(config: Option<&ToolConfig>) -> Option<ToolChoice> {
    let mode = config?
        .function_calling_config
        .as_ref()
        .and_then(|function_calling_config| function_calling_config.mode.as_deref());
    let choice = match mode {
        Some("ANY") => "required",
        Some("NONE") => "none",
        // LiteLLM's map holds AUTO and falls back to auto for a mode it does not know
        Some(_) | None => "auto",
    };
    Some(ToolChoice::Mode(String::from(choice)))
}

fn tools_to_openai(
    tools: &[GenAiTool],
    limits: &SchemaLimits,
) -> Result<Vec<Tool>, GenAiInputError> {
    let mut openai_tools = Vec::new();
    for tool in tools {
        for declaration in &tool.function_declarations {
            let given = declaration
                .parameters_json_schema
                .as_ref()
                .or(declaration.parameters.as_ref());
            let parameters = match given.filter(|value| value.is_object()) {
                Some(value) => Some(normalize_json_schema_types(value, limits)?),
                None => None,
            };
            openai_tools.push(Tool {
                kind: String::from("function"),
                function: Some(FunctionDefinition {
                    name: declaration.name.clone().unwrap_or_default(),
                    description: declaration.description.clone(),
                    parameters,
                    strict: None,
                }),
                passthrough: IndexMap::new(),
            });
        }
    }
    Ok(openai_tools)
}

fn response_format(
    config: Option<&GenerationConfig>,
    limits: &SchemaLimits,
) -> Result<Option<ResponseFormat>, GenAiInputError> {
    let Some(fields) = config else {
        return Ok(None);
    };
    if fields
        .response_mime_type
        .as_deref()
        .is_some_and(|mime| mime != JSON_MIME_TYPE)
    {
        return Ok(None);
    }
    let schema = fields
        .response_json_schema
        .as_ref()
        .or(fields.response_schema.as_ref());
    let Some(given) = schema.filter(|value| value.is_object()) else {
        return Ok(None);
    };
    let mut cleaned = normalize_json_schema_types(given, limits)?;
    if cleaned.get("type").and_then(Value::as_str) != Some("object") {
        return Ok(None);
    }
    without_gemini_keys(&mut cleaned, 0);
    let json_schema = serde_json::Map::from_iter([
        (
            String::from("name"),
            Value::String(String::from("response")),
        ),
        (String::from("schema"), cleaned),
    ]);
    Ok(Some(ResponseFormat {
        kind: String::from("json_schema"),
        json_schema: Some(Value::Object(json_schema)),
    }))
}

// LiteLLM's filter_value_from_dict visits dict values and the dicts a list carries at one level below the dict
fn without_gemini_keys(schema: &mut Value, depth: u32) {
    if depth > 100 {
        return;
    }
    let Some(fields) = schema.as_object_mut() else {
        return;
    };
    fields.retain(|key, _| !["propertyOrdering", "property_ordering"].contains(&key.as_str()));
    for value in fields.values_mut() {
        match value {
            Value::Object(_) => without_gemini_keys(value, depth.saturating_add(1)),
            Value::Array(items) => {
                for item in items.iter_mut().filter(|item| item.is_object()) {
                    without_gemini_keys(item, depth.saturating_add(1));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

fn supports_response_format(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> bool {
    let custom = litellm_params.and_then(|params| params.custom_llm_provider.as_deref());
    let resolution = ResolutionRequest {
        model: &request.model,
        // an empty declaration names no provider, as LiteLLM's falsy check treats it
        custom_llm_provider: custom.filter(|name| !name.is_empty()),
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
    let capabilities = response_schema_capabilities(context.resolution.catalog, &model);
    let query = CapabilityQuery {
        provider,
        call_type: CallType::ChatCompletion,
        config: openai_config_tag(&model, context.model_capabilities),
        model: ModelProjection {
            model: &model,
            capabilities: &capabilities,
        },
        base_model: None,
    };
    match supported_parameters(&query) {
        // no parameter list describes a provider betterlite has not built, and LiteLLM keeps the field
        ParameterCapabilities::Absent => true,
        ParameterCapabilities::Listed(names) => names.contains(&"response_format"),
    }
}

fn openai_config_tag(model: &str, catalog: &CatalogCapabilities) -> OpenAiConfigTag {
    match model_family(model, catalog) {
        ModelFamily::OSeries => OpenAiConfigTag::OSeries,
        ModelFamily::GptReasoning => OpenAiConfigTag::Gpt5,
        ModelFamily::Audio | ModelFamily::Gpt => OpenAiConfigTag::Gpt,
        ModelFamily::Unknown => OpenAiConfigTag::UnknownModel,
    }
}

fn response_schema_capabilities(catalog: &ModelCatalog, model: &str) -> BTreeSet<OpenAiCapability> {
    let mut capabilities = BTreeSet::new();
    let found = catalog
        .lookup
        .get(model)
        .and_then(|canonical| catalog.entries.get(canonical.as_ref()));
    if let Some(entry) = found
        && CapabilityView::from_fields(&entry.fields.capabilities)
            .get(CapabilityFlag::ResponseSchema)
            == Some(true)
    {
        capabilities.insert(OpenAiCapability::ResponseSchema);
    }
    capabilities
}
