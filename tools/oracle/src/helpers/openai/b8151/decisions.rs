use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use betterlite_openai::hosted_vllm_media::decisions::HostedVllmDecisionsConfig;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

// LiteLLM's call also sets the environment, which the trait's plain fn pointer cannot read here
#[derive(Debug, Deserialize)]
pub(crate) struct KeyCall {
    api_key: Option<String>,
}

pub(super) fn key_value(call: &KeyCall) -> Result<Value, Box<HelperError>> {
    let config = HostedVllmDecisionsConfig;
    let resolved = config.resolve_api_key(call.api_key.as_deref(), |_name| String::new());
    Ok(json!({
        "api_key_env": HostedVllmDecisionsConfig::API_KEY_ENV,
        "api_base_env": HostedVllmDecisionsConfig::API_BASE_ENV,
        "api_key_required": HostedVllmDecisionsConfig::API_KEY_REQUIRED,
        "resolved": resolved,
    }))
}
