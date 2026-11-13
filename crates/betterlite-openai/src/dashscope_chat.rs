use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

pub const DEFAULT_BASE: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
const CHAT_COMPLETIONS: &str = "/chat/completions";
const REASONING_EFFORT: &str = "reasoning_effort";

// one family's chat endpoint rule: the default, the environment value the caller resolved, and whether the URL reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatBasePolicy {
    pub default_base: String,
    pub env_base: Option<String>,
    pub env_in_complete_url: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct DashscopeChatConfig {
    pub policy: ChatBasePolicy,
    pub env_api_key: Option<SecretString>,
}

impl DashscopeChatConfig {
    // LiteLLM's DashScopeChatConfig overrides remove_cache_control_flag_from_messages_and_tools to return its inputs
    pub const PRESERVES_CACHE_CONTROL: bool = true;

    // DashScopeChatConfig extends OpenAIGPTConfig, so the base list runs with the OpenAI provider extras left out
    #[must_use]
    pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
        let mut params = OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        }
        .supported_params(model, catalog);
        params.push(REASONING_EFFORT);
        params
    }

    // the source chains its explicit value, the environment value and the default, and an empty string falls through
    #[must_use]
    pub fn provider_info(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> ProviderInfo {
        let environment_base = self
            .policy
            .env_base
            .as_deref()
            .filter(|value| !value.is_empty());
        let base = api_base
            .filter(|value| !value.is_empty())
            .or(environment_base)
            .map_or_else(|| self.policy.default_base.clone(), str::to_owned);
        let environment_key = self
            .env_api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .cloned()
            .or_else(|| environment_key.cloned());
        ProviderInfo {
            api_base: base,
            api_key: key,
        }
    }

    // the source never trims the base, so a trailing slash gives a doubled slash before /chat/completions
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        let environment_base = if self.policy.env_in_complete_url {
            self.policy
                .env_base
                .as_deref()
                .filter(|value| !value.is_empty())
        } else {
            None
        };
        let base = api_base
            .filter(|value| !value.is_empty())
            .or(environment_base)
            .map_or_else(|| self.policy.default_base.clone(), str::to_owned);
        if base.ends_with(CHAT_COMPLETIONS) {
            return base;
        }
        format!("{base}{CHAT_COMPLETIONS}")
    }
}

mod tests;
