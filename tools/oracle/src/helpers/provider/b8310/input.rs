use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Chat,
    Embedding,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    FireworksAi,
    TogetherAi,
    Groq,
    Cloudflare,
}

impl Provider {
    pub(super) const fn slug(self) -> &'static str {
        match self {
            Self::FireworksAi => "fireworks_ai",
            Self::TogetherAi => "together_ai",
            Self::Groq => "groq",
            Self::Cloudflare => "cloudflare",
        }
    }

    pub(super) const fn chat_tag(self) -> ChatConfigTag {
        match self {
            Self::FireworksAi => ChatConfigTag::FireworksAi,
            Self::TogetherAi => ChatConfigTag::TogetherAiChat,
            Self::Groq => ChatConfigTag::GroqChat,
            Self::Cloudflare => ChatConfigTag::CloudflareChat,
        }
    }

    pub(super) const fn provider_id(self) -> ProviderId {
        match self {
            Self::FireworksAi => ProviderId::FireworksAi,
            Self::TogetherAi => ProviderId::TogetherAi,
            Self::Groq => ProviderId::Groq,
            Self::Cloudflare => ProviderId::Cloudflare,
        }
    }
}

// the module-level litellm settings the arms read, as a case names them; litellm.headers is untyped in LiteLLM, so no case sets it
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Globals {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) groq_key: Option<String>,
    #[serde(default)]
    pub(super) cloudflare_api_key: Option<String>,
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
    // what litellm.GroqChatConfig.get_config() returns, built from the class defaults
    #[serde(default)]
    pub(super) groq_config: Option<Json>,
    // the model's supports_reasoning answer, which B2221's lookup owns
    #[serde(default)]
    pub(super) reasoning: bool,
    #[serde(default)]
    pub(super) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) answer: Option<Answer>,
}

impl Case {
    pub(super) fn optional_params(&self) -> Result<Object, Box<HelperError>> {
        match &self.optional_params {
            None | Some(Json::Null) => Ok(Object::new()),
            Some(given) => Ok(serde_json::from_value::<Object>(serde_json::to_value(
                given,
            )?)?),
        }
    }

    // what litellm.GroqChatConfig.get_config() returns for the call
    pub(super) fn groq_config(&self) -> Result<Object, Box<HelperError>> {
        match &self.groq_config {
            None | Some(Json::Null) => Ok(Object::new()),
            Some(given) => Ok(serde_json::from_value::<Object>(serde_json::to_value(
                given,
            )?)?),
        }
    }
}
