use betterlite_models::controls::json_value::JsonValue;
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
    pub const fn supports_encrypted_tasks(self) -> bool {
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
#[must_use]
pub const fn bridges_to_chat_completions(
    config: Option<&ResponsesApiConfig>,
    use_chat_completions_api: bool,
) -> bool {
    match config {
        None => true,
        Some(_native) => use_chat_completions_api,
    }
}
#[must_use]
pub const fn encrypted_task_support_failure(
    config: Option<&ResponsesApiConfig>,
    use_chat_completions_api: bool,
) -> Option<CompatibilityFailure> {
    let Some(native) = config else {
        return Some(CompatibilityFailure::EncryptedTaskUnsupported);
    };
    if native.supports_encrypted_tasks() && !use_chat_completions_api {
        return None;
    }
    Some(CompatibilityFailure::EncryptedTaskUnsupported)
}
#[must_use]
pub fn passes_responses_through(model_info: &JsonValue) -> bool {
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
pub trait ConfigLookup {
    fn responses_api_config(
        &self,
        model: &str,
        provider: &str,
        api_base: Option<&str>,
    ) -> Option<NativeConfig>;
}
#[must_use]
pub fn resolve_provider_config(
    lookup: &dyn ConfigLookup,
    model: &str,
    provider: &str,
    model_info: &JsonValue,
    api_base: Option<&str>,
) -> Option<ResponsesApiConfig> {
    if let Some(native) = lookup.responses_api_config(model, provider, api_base) {
        return Some(ResponsesApiConfig::Native(native));
    }
    passes_responses_through(model_info).then_some(ResponsesApiConfig::OpenAiLike)
}
