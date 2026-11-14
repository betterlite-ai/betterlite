use serde_json::{Map, Value, json};

use crate::base_helpers::append_tools;
use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

use super::PROVIDER;
use super::capabilities::GroqCapabilities;
use super::error::GroqChatError;

pub const COMPOUND_MODELS: [&str; 2] = ["compound", "compound-mini"];

const PROVIDER_PREFIX: &str = "groq/";
const MAX_RETRIES: &str = "max_retries";
const WEB_SEARCH_OPTIONS: &str = "web_search_options";
const REASONING_EFFORT: &str = "reasoning_effort";
const RESPONSE_FORMAT: &str = "response_format";
const RESPONSE_SCHEMA: &str = "response_schema";
const JSON_SCHEMA: &str = "json_schema";
const JSON_MODE: &str = "json_mode";
const JSON_TOOL: &str = "json_tool_call";
const TOOLS: &str = "tools";
const TOOL_CHOICE: &str = "tool_choice";
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
const MAX_TOKENS: &str = "max_tokens";
const FAKE_STREAM: &str = "fake_stream";
const BROWSER_SEARCH: &str = "browser_search";

#[must_use]
pub fn is_compound_model(model: &str) -> bool {
    match model.strip_prefix(PROVIDER_PREFIX) {
        Some(name) => COMPOUND_MODELS.contains(&name),
        None => COMPOUND_MODELS.contains(&model),
    }
}

pub struct SupportedParams<'request> {
    pub model: &'request str,
    pub config: &'request OpenAiConfig,
    pub catalog: &'request CatalogCapabilities,
    pub capabilities: &'request dyn GroqCapabilities,
}

pub fn get_supported_openai_params(
    request: &SupportedParams<'_>,
) -> Result<Vec<&'static str>, GroqChatError> {
    let mut params = request
        .config
        .supported_params(request.model, request.catalog);
    params.retain(|name| *name != MAX_RETRIES);
    let web_search = if is_compound_model(request.model) {
        true
    } else {
        request
            .capabilities
            .supports_web_search(request.model, PROVIDER)?
    };
    if !web_search {
        params.retain(|name| *name != WEB_SEARCH_OPTIONS);
    }
    if reasoning_supported(request.capabilities, request.model) {
        params.push(REASONING_EFFORT);
    }
    Ok(params)
}

// only this lookup is guarded, so a catalog failure leaves reasoning_effort out instead of failing the request
fn reasoning_supported(capabilities: &dyn GroqCapabilities, model: &str) -> bool {
    match capabilities.supports_reasoning(model, PROVIDER) {
        Ok(supported) => supported,
        Err(error) => {
            tracing::debug!(
                provider = PROVIDER,
                model,
                %error,
                "the reasoning capability lookup failed, so reasoning_effort is not advertised"
            );
            false
        }
    }
}

pub struct MapParams<'request> {
    pub model: &'request str,
    pub non_default: &'request Object,
    pub optional_params: &'request Object,
    pub config: &'request OpenAiConfig,
    pub catalog: &'request CatalogCapabilities,
    pub capabilities: &'request dyn GroqCapabilities,
    // the OpenAI-like base mapper ignores this flag, so it never changes the mapped options
    pub drop_params: bool,
}

pub struct MappedParams {
    pub optional_params: Object,
    pub non_default: Object,
}

pub fn map_openai_params(request: &MapParams<'_>) -> Result<MappedParams, GroqChatError> {
    let mut non_default = request.non_default.clone();
    let mut optional_params = request.optional_params.clone();
    map_response_format(
        request.model,
        &mut non_default,
        &mut optional_params,
        request.capabilities,
    )?;
    let web_search = non_default
        .remove(WEB_SEARCH_OPTIONS)
        .filter(|value| !value.is_null());
    base_map(request, &non_default, &mut optional_params)?;
    if let Some(web_search) = web_search {
        add_web_search(request.model, &mut optional_params, &web_search)?;
    }
    Ok(MappedParams {
        optional_params,
        non_default,
    })
}

// fake_stream is set before the schema is read, so a json_object format forces replay without the tool workaround
fn map_response_format(
    model: &str,
    non_default: &mut Object,
    optional_params: &mut Object,
    capabilities: &dyn GroqCapabilities,
) -> Result<(), GroqChatError> {
    let format = non_default.get(RESPONSE_FORMAT).cloned();
    if format.as_ref().is_some_and(|value| !value.is_null()) {
        optional_params.insert(FAKE_STREAM.to_owned(), Value::Bool(true));
    }
    let Some(Value::Object(fields)) = format else {
        return Ok(());
    };
    let schema = match fields.get(RESPONSE_SCHEMA) {
        Some(schema) => schema,
        None => match fields.get(JSON_SCHEMA) {
            None => return Ok(()),
            Some(envelope) => envelope.get("schema").ok_or(GroqChatError::SchemaShape)?,
        },
    };
    if schema.is_null() || capabilities.supports_response_schema(model, PROVIDER)? {
        return Ok(());
    }
    if non_default.contains_key(TOOLS) {
        return Err(GroqChatError::BadRequest {
            message: tools_conflict_message(model),
        });
    }
    optional_params.insert(TOOLS.to_owned(), Value::Array(vec![json_tool(schema)]));
    let tool_choice = json!({"type": "function", "function": {"name": JSON_TOOL}});
    optional_params.insert(TOOL_CHOICE.to_owned(), tool_choice);
    optional_params.insert(JSON_MODE.to_owned(), Value::Bool(true));
    non_default.remove(RESPONSE_FORMAT);
    Ok(())
}

fn tools_conflict_message(model: &str) -> String {
    format!(
        "Groq model '{model}' does not support native structured outputs. LiteLLM uses a tool-calling workaround for structured outputs on this model, which is incompatible with user-provided tools. Either use a model that supports native structured outputs (e.g., gpt-oss-120b, llama-4, kimi-k2), or remove the tools parameter. See: https://console.groq.com/docs/structured-outputs#supported-models"
    )
}

fn json_tool(schema: &Value) -> Value {
    let mut function = Map::new();
    function.insert("name".to_owned(), Value::String(JSON_TOOL.to_owned()));
    function.insert("parameters".to_owned(), schema.clone());
    let mut tool = Map::new();
    tool.insert("type".to_owned(), Value::String("function".to_owned()));
    tool.insert("function".to_owned(), Value::Object(function));
    Value::Object(tool)
}

// the Groq supported list decides the copy, since the overridden list drops max_retries and can add reasoning_effort
fn base_map(
    request: &MapParams<'_>,
    non_default: &Object,
    optional_params: &mut Object,
) -> Result<(), GroqChatError> {
    let supported = get_supported_openai_params(&SupportedParams {
        model: request.model,
        config: request.config,
        catalog: request.catalog,
        capabilities: request.capabilities,
    })?;
    for (name, value) in non_default {
        if supported.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    // the OpenAI-like mapper runs with its default flag on, so the alias never reaches Groq
    if let Some(limit) = non_default.get(MAX_COMPLETION_TOKENS) {
        optional_params.insert(MAX_TOKENS.to_owned(), limit.clone());
        optional_params.remove(MAX_COMPLETION_TOKENS);
    }
    Ok(())
}

fn add_web_search(
    model: &str,
    optional_params: &mut Object,
    web_search: &Value,
) -> Result<(), GroqChatError> {
    // LiteLLM logs only a non-empty value, and an empty object still adds the tool
    if let Some(fields) = web_search.as_object().filter(|fields| !fields.is_empty()) {
        let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
        keys.sort_unstable();
        tracing::info!(
            provider = PROVIDER,
            fields = ?keys,
            "Groq web search enabled; ignoring unsupported web_search_options fields"
        );
    }
    if is_compound_model(model) || has_browser_search(optional_params) {
        return Ok(());
    }
    append_tools(optional_params, vec![json!({"type": BROWSER_SEARCH})])?;
    Ok(())
}

fn has_browser_search(optional_params: &Object) -> bool {
    let tools = optional_params.get(TOOLS).and_then(Value::as_array);
    tools.is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) == Some(BROWSER_SEARCH))
    })
}
