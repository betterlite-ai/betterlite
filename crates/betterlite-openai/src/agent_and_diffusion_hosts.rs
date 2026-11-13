#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "serde_json test fixtures are concise and type-directed"
    )
)]

pub mod gradient;
pub mod heroku;
pub mod inception_fim;

mod tests;

use betterlite_config::sections::agent_host_settings::Settings;
use secrecy::{ExposeSecret as _, SecretString};

pub const HYPERBOLIC_API_BASE: &str = "https://api.hyperbolic.xyz/v1";
pub const LAMBDA_API_BASE: &str = "https://api.lambda.ai/v1";
pub const INCEPTION_API_BASE: &str = "https://api.inceptionlabs.ai/v1";

pub const HYPERBOLIC_PARAMS: &[&str] = &[
    "messages",
    "model",
    "stream",
    "temperature",
    "top_p",
    "max_tokens",
    "frequency_penalty",
    "presence_penalty",
    "stop",
    "n",
    "tools",
    "tool_choice",
    "response_format",
    "seed",
    "user",
];

pub const INCEPTION_PARAMS: &[&str] = &[
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "stop",
    "tools",
    "tool_choice",
    "stream",
    "stream_options",
    "response_format",
    "reasoning_effort",
    "reasoning_summary",
    "reasoning_summary_wait",
    "diffusing",
    "realtime",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHost {
    Gradient,
    Heroku,
    Hyperbolic,
    LambdaAi,
    Inception,
}

impl AgentHost {
    // LiteLLM's config property names a provider tag on only these three classes
    #[must_use]
    pub const fn custom_llm_provider(self) -> Option<&'static str> {
        match self {
            Self::Hyperbolic => Some("hyperbolic"),
            Self::LambdaAi => Some("lambda_ai"),
            Self::Inception => Some("inception"),
            Self::Gradient | Self::Heroku => None,
        }
    }

    // only a host that overrides get_supported_openai_params names a list here; the others inherit B668's
    #[must_use]
    pub const fn supported_params(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Gradient => Some(gradient::SUPPORTED_PARAMS),
            Self::Hyperbolic => Some(HYPERBOLIC_PARAMS),
            Self::Inception => Some(INCEPTION_PARAMS),
            Self::Heroku | Self::LambdaAi => None,
        }
    }

    #[must_use]
    pub fn provider_info(
        self,
        settings: &Settings,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (Option<String>, Option<SecretString>) {
        match self {
            Self::Inception => inception_provider_info(api_base, api_key, settings),
            Self::Gradient | Self::Heroku | Self::Hyperbolic | Self::LambdaAi => {
                let (base_env, key_env, default_base) = self.compatible_env(settings);
                resolve_provider_info(base_env, key_env, default_base, api_base, api_key)
            }
        }
    }

    fn compatible_env(
        self,
        settings: &Settings,
    ) -> (Option<&str>, Option<&SecretString>, Option<&'static str>) {
        match self {
            Self::Gradient => (
                settings.gradient_ai_agent_endpoint.as_deref(),
                settings.gradient_ai_api_key.as_ref(),
                Some(gradient::SERVERLESS_ENDPOINT),
            ),
            Self::Heroku => (
                settings.heroku_api_base.as_deref(),
                settings.heroku_api_key.as_ref(),
                None,
            ),
            Self::Hyperbolic => (
                settings.hyperbolic_api_base.as_deref(),
                settings.hyperbolic_api_key.as_ref(),
                Some(HYPERBOLIC_API_BASE),
            ),
            Self::LambdaAi => (
                settings.lambda_api_base.as_deref(),
                settings.lambda_api_key.as_ref(),
                Some(LAMBDA_API_BASE),
            ),
            Self::Inception => (None, None, None),
        }
    }
}

// LiteLLM chains explicit, then the named environment value, then the default, and each step skips an empty string
#[must_use]
pub fn resolve_provider_info(
    base_env: Option<&str>,
    key_env: Option<&SecretString>,
    default_base: Option<&str>,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (Option<String>, Option<SecretString>) {
    let explicit_base = api_base.filter(|value| !value.is_empty());
    let environment_base = base_env.filter(|value| !value.is_empty());
    let chosen_base = explicit_base.map_or_else(|| environment_base.or(default_base), Some);
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| key_env.cloned());
    (chosen_base.map(str::to_owned), key)
}

// Inception borrows litellm.inception_key and the environment key only while no explicit base was passed, or the caller passed a key
#[must_use]
pub fn inception_provider_info(
    explicit_base: Option<&str>,
    explicit_key: Option<&SecretString>,
    settings: &Settings,
) -> (Option<String>, Option<SecretString>) {
    let given_key = explicit_key.filter(|key| !key.expose_secret().is_empty());
    let key = if explicit_base.is_none() || given_key.is_some() {
        let global = settings
            .inception_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        given_key
            .or(global)
            .or(settings.inception_api_key.as_ref())
            .cloned()
    } else {
        explicit_key.cloned()
    };
    let environment_base = settings
        .inception_api_base
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(INCEPTION_API_BASE);
    let base = explicit_base
        .filter(|value| !value.is_empty())
        .unwrap_or(environment_base);
    (Some(base.to_owned()), key)
}
