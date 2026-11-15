use betterlite_models::projections::reasoning_effort::{
    ThinkingBudgetThresholds, effort_from_thinking_budget,
};
use secrecy::SecretString;
use serde_json::Value;

use self::error::HostedVllmChatError;
use crate::local_servers::{ProviderEnvironment, resolve_api_base, resolve_api_key};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

pub mod error;
pub mod messages;
mod tests;
mod tools;
mod video;

pub const REASONING_EFFORT: &str = "reasoning_effort";
pub const THINKING: &str = "thinking";
const PLACEHOLDER_KEY: &str = "fake-api-key";

// the base config is OpenAIGPTConfig, so its family is Other and the subclass adds reasoning_effort and thinking
#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub base: OpenAiConfig,
    pub thresholds: ThinkingBudgetThresholds,
}

#[derive(Debug)]
pub struct ProviderInfo {
    pub api_base: Option<String>,
    pub api_key: SecretString,
}

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &ProviderEnvironment,
) -> ProviderInfo {
    ProviderInfo {
        api_base: resolve_api_base(api_base, environment),
        api_key: resolve_api_key(api_key, environment, PLACEHOLDER_KEY),
    }
}

#[must_use]
pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let base = OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    let mut params = base.supported_params(model, catalog);
    params.push(REASONING_EFFORT);
    params.push(THINKING);
    params
}

impl ChatConfig {
    pub fn map_openai_params(
        &self,
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) -> Result<(), HostedVllmChatError> {
        let mut params = non_default_params.clone();
        match params.remove("tools") {
            None | Some(Value::Null) => {}
            Some(tools) => {
                params.insert(String::from("tools"), tools::clean_and_convert(tools)?);
            }
        }
        if let Some(thinking) = params.remove(THINKING)
            && let Value::Object(thinking_fields) = &thinking
            && thinking_fields.get("type").and_then(Value::as_str) == Some("enabled")
            && !params.contains_key(REASONING_EFFORT)
        {
            let budget = match thinking_fields.get("budget_tokens") {
                None | Some(Value::Null) => 0,
                Some(Value::Number(number)) => {
                    number.as_i64().ok_or(HostedVllmChatError::ThinkingBudget)?
                }
                Some(Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_)) => {
                    return Err(HostedVllmChatError::ThinkingBudget);
                }
            };
            let effort = effort_from_thinking_budget(budget, self.thresholds);
            let label = serde_json::to_value(effort)
                .map_err(|source| HostedVllmChatError::Json { source })?;
            params.insert(String::from(REASONING_EFFORT), label);
        }
        self.base
            .map_params(model, &params, optional_params, catalog);
        if let Some(effort) = params.get(REASONING_EFFORT) {
            optional_params.insert(String::from(REASONING_EFFORT), effort.clone());
        }
        Ok(())
    }
}
