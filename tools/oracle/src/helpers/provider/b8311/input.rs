use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::vercel_ai_gateway::DEFAULT_API_BASE as VERCEL_DEFAULT_API_BASE;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Chat,
    Embedding,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    Openrouter,
    VercelAiGateway,
}

impl Provider {
    pub(super) const fn slug(self) -> &'static str {
        match self {
            Self::Openrouter => "openrouter",
            Self::VercelAiGateway => "vercel_ai_gateway",
        }
    }

    pub(super) const fn chat_tag(self) -> ChatConfigTag {
        match self {
            Self::Openrouter => ChatConfigTag::Openrouter,
            Self::VercelAiGateway => ChatConfigTag::VercelAiGateway,
        }
    }

    pub(super) const fn provider_id(self) -> ProviderId {
        match self {
            Self::Openrouter => ProviderId::Openrouter,
            Self::VercelAiGateway => ProviderId::VercelAiGateway,
        }
    }
}

// the module-level litellm values the arms read; litellm.headers is untyped in LiteLLM, so no case sets it
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Globals {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) openrouter_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(super) op: Operation,
    pub(super) provider: Provider,
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) input: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    pub(super) optional_params: Option<Json>,
    // what litellm.OpenrouterConfig.get_config() or litellm.VercelAIGatewayConfig.get_config() returns
    #[serde(default)]
    pub(super) class_config: Option<Json>,
    #[serde(default)]
    pub(super) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) globals: Globals,
}

pub(super) fn as_object(value: Option<&Json>) -> Result<Object, Box<HelperError>> {
    match value {
        None | Some(Json::Null) => Ok(Object::new()),
        Some(given) => Ok(serde_json::from_value::<Object>(serde_json::to_value(
            given,
        )?)?),
    }
}

// what get_llm_provider settles before the arm runs: Vercel fills the base and key, OIDC included, from the environment; OpenRouter passes the caller's values
pub(super) struct PreResolved {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
}

#[rustfmt::skip]
fn nonempty(value: Option<&String>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
        .map(String::as_str)
}

#[rustfmt::skip]
fn variable<'env>(env: &'env BTreeMap<String, String>, name: &str) -> Option<&'env str> {
    env.get(name).map(String::as_str)
        .filter(|text| !text.is_empty())
}

#[rustfmt::skip]
pub(super) fn pre_resolved(provider: Provider, api_base: Option<&String>, api_key: Option<&String>, env: &BTreeMap<String, String>) -> PreResolved {
    let (base, key) = match provider {
        Provider::Openrouter => (nonempty(api_base), nonempty(api_key)),
        Provider::VercelAiGateway => (
            Some(nonempty(api_base).or_else(|| variable(env, "VERCEL_AI_GATEWAY_API_BASE")).unwrap_or(VERCEL_DEFAULT_API_BASE)),
            nonempty(api_key).or_else(|| variable(env, "VERCEL_AI_GATEWAY_API_KEY")).or_else(|| variable(env, "VERCEL_OIDC_TOKEN")),
        ),
    };
    PreResolved { api_base: base.map(str::to_owned), api_key: key.map(SecretString::from) }
}
