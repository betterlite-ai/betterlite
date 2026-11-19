use betterlite_models::controls::json_value::JsonValue;

use super::ParameterMap;

pub const PASSTHROUGH_ENDPOINT: &str = "/v1/responses";
pub const ASYNC_PROMPT_MERGED_PARAMS: &str = "_async_prompt_merged_params";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeConfig {
    pub encrypted_agent_messages: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesApiConfig {
    Native(NativeConfig),
    OpenAiLike,
}

impl ResponsesApiConfig {
    #[must_use]
    pub const fn supports_encrypted_agent_messages(self) -> bool {
        match self {
            Self::Native(native) => native.encrypted_agent_messages,
            Self::OpenAiLike => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityFailure {
    EncryptedTaskUnsupported,
}

impl CompatibilityFailure {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::EncryptedTaskUnsupported => "encrypted_task_unsupported",
        }
    }
}

// LiteLLM's _bridges_to_chat_completions: no resolved config, or an explicit chat-completions request
#[must_use]
pub const fn bridges_to_chat_completions(
    config: Option<&ResponsesApiConfig>,
    use_chat_completions_api: bool,
) -> bool {
    config.is_none() || use_chat_completions_api
}

// LiteLLM's _encrypted_task_support_failure: every route but a capable native deployment refuses the task
#[must_use]
pub const fn encrypted_task_support_failure(
    config: Option<&ResponsesApiConfig>,
    use_chat_completions_api: bool,
) -> Option<CompatibilityFailure> {
    match config {
        Some(native) => {
            if native.supports_encrypted_agent_messages() && !use_chat_completions_api {
                return None;
            }
            Some(CompatibilityFailure::EncryptedTaskUnsupported)
        }
        None => Some(CompatibilityFailure::EncryptedTaskUnsupported),
    }
}

// LiteLLM's _deployment_passes_through_responses: only a list carrying the exact endpoint opts in
#[must_use]
pub fn deployment_passes_through_responses(model_info: &JsonValue) -> bool {
    let JsonValue::Object(fields) = model_info else {
        return false;
    };
    match fields.get("supported_endpoints") {
        Some(JsonValue::Array(endpoints)) => endpoints.iter().any(
            |endpoint| matches!(endpoint, JsonValue::Text(text) if text == PASSTHROUGH_ENDPOINT),
        ),
        Some(
            JsonValue::Text(_)
            | JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Object(_),
        )
        | None => false,
    }
}

// B157's typed provider config lookup, injected so this module never reaches the provider registry
pub trait ConfigLookup {
    fn responses_api_config(
        &self,
        model: &str,
        provider: &str,
        api_base: Option<&str>,
    ) -> Option<NativeConfig>;
}

pub struct ConfigRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub model_info: &'request JsonValue,
    pub api_base: Option<&'request str>,
}

// LiteLLM's _resolve_responses_api_provider_config: the lookup wins; supported_endpoints then opts into OpenAI-like
#[must_use]
pub fn resolve_responses_api_provider_config(
    lookup: &dyn ConfigLookup,
    request: &ConfigRequest<'_>,
) -> Option<ResponsesApiConfig> {
    if let Some(native) =
        lookup.responses_api_config(request.model, request.provider, request.api_base)
    {
        return Some(ResponsesApiConfig::Native(native));
    }
    deployment_passes_through_responses(request.model_info)
        .then_some(ResponsesApiConfig::OpenAiLike)
}

// LiteLLM's _AsyncPromptManagementOutcome: an async prompt pass hands its result forward through kwargs
#[derive(Debug, Clone, PartialEq)]
pub struct AsyncPromptManagementOutcome {
    pub merged_optional_params: ParameterMap,
    pub deployment_model_info: Option<JsonValue>,
}
