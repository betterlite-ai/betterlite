use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::hosted_vllm_media::decisions::HostedVllmDecisionsConfig;
use serde::Deserialize;
use serde_json::{Value, json};

// LiteLLM's call also sets the environment, which the trait's plain fn pointer cannot read here
#[derive(Debug, Deserialize)]
pub(crate) struct KeyCall {
    api_key: Option<String>,
}

pub(super) fn key_value(call: &KeyCall) -> Value {
    let config = HostedVllmDecisionsConfig;
    let resolved = config.resolve_api_key(call.api_key.as_deref(), |_name| String::new());
    json!({
        "api_key_env": HostedVllmDecisionsConfig::API_KEY_ENV,
        "api_base_env": HostedVllmDecisionsConfig::API_BASE_ENV,
        "api_key_required": HostedVllmDecisionsConfig::API_KEY_REQUIRED,
        "resolved": resolved,
    })
}
