use betterlite_models::controls::json_value::JsonValue;

use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prefixes::normalize_openai_chat_model;
use crate::api::responses_provider_selection::provider_config::{
    ConfigLookup, ConfigRequest, bridges_to_chat_completions, resolve_provider_config,
};

pub struct ProvisionalBridge<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub use_chat_completions_api: bool,
    pub model_info: &'request JsonValue,
    pub api_base: Option<&'request str>,
}

// LiteLLM's _will_bridge_to_chat_completions: the dispatch-time predicate, asked before the hook runs
#[must_use]
pub fn will_use_chat_bridge(lookup: &dyn ConfigLookup, bridge: &ProvisionalBridge<'_>) -> bool {
    let (normalized_model, prefixed) = normalize_openai_chat_model(bridge.model);
    let Some(provider) = bridge.custom_llm_provider else {
        return true;
    };
    let config = resolve_provider_config(
        lookup,
        &ConfigRequest {
            model: &normalized_model,
            provider,
            model_info: bridge.model_info,
            api_base: bridge.api_base,
        },
    );
    bridges_to_chat_completions(config.as_ref(), bridge.use_chat_completions_api || prefixed)
}

// LiteLLM's _api_base_kwarg: only a text api_base reaches the provider config lookup
#[must_use]
pub fn api_base_kwarg(kwargs: &ParameterMap) -> Option<&str> {
    let value = kwargs.get("api_base")?;
    let JsonValue::Text(base) = value else {
        return None;
    };
    Some(base)
}
