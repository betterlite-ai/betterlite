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

// the upstream answer: litellm's handler succeeds on a 2xx status and raises on any other
#[derive(Deserialize)]
pub(super) struct Answer {
    pub(super) status: u16,
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
    #[serde(default)]
    pub(super) answer: Option<Answer>,
}

pub(super) fn as_object(value: Option<&Json>) -> Result<Object, Box<HelperError>> {
    match value {
        None | Some(Json::Null) => Ok(Object::new()),
        Some(given) => Ok(serde_json::from_value::<Object>(serde_json::to_value(
            given,
        )?)?),
    }
}

// what get_llm_provider settles before main.py hands the call to an arm
pub(super) struct PreResolved {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
}

// Vercel goes through _get_openai_compatible_provider_info, which fills the base from the environment and
// the key from VERCEL_OIDC_TOKEN too; OpenRouter has no such branch and reaches its arm with the caller's own values
pub(super) fn pre_resolved(
    provider: Provider,
    api_base: Option<&String>,
    api_key: Option<&String>,
    env: &BTreeMap<String, String>,
) -> PreResolved {
    match provider {
        Provider::Openrouter => PreResolved {
            api_base: api_base.filter(|text| !text.is_empty()).cloned(),
            api_key: api_key
                .filter(|text| !text.is_empty())
                .cloned()
                .map(SecretString::from),
        },
        Provider::VercelAiGateway => PreResolved {
            api_base: Some(
                api_base
                    .filter(|text| !text.is_empty())
                    .map(String::as_str)
                    .or_else(|| {
                        env.get("VERCEL_AI_GATEWAY_API_BASE")
                            .map(String::as_str)
                            .filter(|text| !text.is_empty())
                    })
                    .unwrap_or(VERCEL_DEFAULT_API_BASE)
                    .to_owned(),
            ),
            api_key: api_key
                .filter(|text| !text.is_empty())
                .map(|key| key.as_str())
                .or_else(|| {
                    env.get("VERCEL_AI_GATEWAY_API_KEY")
                        .map(String::as_str)
                        .filter(|text| !text.is_empty())
                })
                .or_else(|| {
                    env.get("VERCEL_OIDC_TOKEN")
                        .map(String::as_str)
                        .filter(|text| !text.is_empty())
                })
                .map(SecretString::from),
        },
    }
}
