use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::credentials::is_openai_backed_api_base;
use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use thiserror::Error;

mod tests;

const MODE: &str = "mode";
const RESPONSES_MODE: &str = "responses";
const RESPONSES_PREFIX: &str = "responses/";

#[derive(Debug, Error)]
pub enum ModelInfoError {
    #[error("model info is not known for {model}")]
    Unknown { model: String },
}

pub trait ResponsesBridgeCapabilities {
    fn model_info(&self, model: &str, custom_llm_provider: &str) -> Result<Object, ModelInfoError>;
    fn resolved_openai_api_base(&self, api_base: Option<&str>) -> String;
    fn foundry_supports_responses(&self, model: &str, api_base: Option<&str>) -> bool;
    fn foundry_chat_rejects_tools(
        &self,
        model: &str,
        reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool;
    fn is_gpt_5_model(&self, model: &str) -> bool;
    fn is_gpt_5_4_plus_model(&self, model: &str) -> bool;
    fn is_gpt_5_search_model(&self, model: &str) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ReasoningEffort {
    Text(String),
    Object(Object),
}

#[derive(Debug, Clone, Copy)]
pub struct BridgeRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub web_search_options: Option<&'request JsonValue>,
    pub tools: &'request [JsonValue],
    pub reasoning_effort: Option<&'request ReasoningEffort>,
    pub reasoning_summary: Option<&'request JsonValue>,
    pub api_base: Option<&'request str>,
    // the LiteLLM setting keeps its longer name, route_all_chat_openai_to_responses
    pub route_all_openai_to_responses: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCheck {
    pub model_info: Object,
    pub model: String,
}

#[must_use]
pub fn responses_api_bridge_check<C>(request: &BridgeRequest<'_>, capabilities: &C) -> BridgeCheck
where
    C: ResponsesBridgeCapabilities + ?Sized,
{
    let mut model = request.model.to_owned();
    if request.route_all_openai_to_responses && request.custom_llm_provider == "openai" {
        model = model.replace(RESPONSES_PREFIX, "");
        let mut model_info = Object::new();
        model_info.insert(
            MODE.to_owned(),
            JsonValue::String(RESPONSES_MODE.to_owned()),
        );
        return BridgeCheck { model_info, model };
    }
    let mut model_info = lookup_model_info(request, capabilities, &mut model);
    if request.web_search_options.is_some() && request.custom_llm_provider == "xai" {
        model_info.insert(
            MODE.to_owned(),
            JsonValue::String(RESPONSES_MODE.to_owned()),
        );
        model = model.replace(RESPONSES_PREFIX, "");
    }
    if bridges_gpt_5(request, capabilities, &model_info, &model) {
        model_info.insert(
            MODE.to_owned(),
            JsonValue::String(RESPONSES_MODE.to_owned()),
        );
        model = model.replace(RESPONSES_PREFIX, "");
    }
    BridgeCheck { model_info, model }
}

fn lookup_model_info<C>(request: &BridgeRequest<'_>, capabilities: &C, model: &mut String) -> Object
where
    C: ResponsesBridgeCapabilities + ?Sized,
{
    let mut model_info = match capabilities.model_info(request.model, request.custom_llm_provider) {
        Ok(model_info) => model_info,
        Err(error) => {
            tracing::debug!(error = %error, "model info lookup failed");
            Object::new()
        }
    };
    let mode_missing = model_info.get(MODE).is_none_or(JsonValue::is_null);
    if mode_missing && request.model.starts_with(RESPONSES_PREFIX) {
        *model = model.replace(RESPONSES_PREFIX, "");
        model_info.insert(
            MODE.to_owned(),
            JsonValue::String(RESPONSES_MODE.to_owned()),
        );
    }
    model_info
}

fn bridges_gpt_5<C>(
    request: &BridgeRequest<'_>,
    capabilities: &C,
    model_info: &Object,
    model: &str,
) -> bool
where
    C: ResponsesBridgeCapabilities + ?Sized,
{
    let on_foundry = request.custom_llm_provider == "azure_ai"
        && capabilities.foundry_supports_responses(model, request.api_base);
    let provider_bridged = matches!(request.custom_llm_provider, "openai" | "azure") || on_foundry;
    if !provider_bridged
        || matches!(model_info.get(MODE), Some(JsonValue::String(mode)) if mode == RESPONSES_MODE)
        || !capabilities.is_gpt_5_model(model)
        || capabilities.is_gpt_5_search_model(model)
    {
        return false;
    }
    (request.reasoning_effort.is_some() && request.reasoning_summary.is_some())
        || chat_rejects_function_tools(request, capabilities, model, on_foundry)
}

fn chat_rejects_function_tools<C>(
    request: &BridgeRequest<'_>,
    capabilities: &C,
    model: &str,
    on_foundry: bool,
) -> bool
where
    C: ResponsesBridgeCapabilities + ?Sized,
{
    if !request.tools.iter().any(is_function_tool) || !reasoning_is_active(request.reasoning_effort)
    {
        return false;
    }
    if on_foundry {
        return capabilities.foundry_chat_rejects_tools(model, request.reasoning_effort);
    }
    let resolved = capabilities.resolved_openai_api_base(request.api_base);
    let api_base = resolved.trim();
    let enforcing = request.custom_llm_provider == "azure"
        || api_base.is_empty()
        || is_openai_backed_api_base(api_base).unwrap_or(false);
    capabilities.is_gpt_5_4_plus_model(model) && (request.reasoning_effort.is_some() || enforcing)
}

fn reasoning_is_active(reasoning_effort: Option<&ReasoningEffort>) -> bool {
    match reasoning_effort {
        Some(ReasoningEffort::Text(effort)) => effort != "none",
        Some(ReasoningEffort::Object(fields)) => {
            !matches!(fields.get("effort"), Some(JsonValue::String(effort)) if effort == "none")
                || fields
                    .get("summary")
                    .is_some_and(|summary| !summary.is_null())
        }
        None => true,
    }
}

fn is_function_tool(tool: &JsonValue) -> bool {
    let Some(fields) = tool.as_object() else {
        return false;
    };
    matches!(fields.get("type"), Some(JsonValue::String(kind)) if kind == "function")
        && (fields.get("function").is_some_and(JsonValue::is_object) || fields.contains_key("name"))
}
