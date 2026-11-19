use betterlite_prompts::arguments::json::{Json, Object};

use crate::chat_content::{ChatContent, Mode};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object as ParamsObject, OpenAiConfig,
    OpenAiSettings,
};
use crate::reasoning_models::copy_supported;
use crate::sail_common::{PROVIDER, SailSettings, chat_request_for_sail};

// ast-grep-ignore: no-long-names
use crate::sail_common::completion_window_for_service_tier as window_for_tier;

pub mod error;
mod tests;

use self::error::SailChatError;

const REJECTED_BY_SAIL: [&str; 7] = [
    "stop",
    "seed",
    "frequency_penalty",
    "presence_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
];
const ACCEPTED_BY_SAIL: [&str; 2] = ["reasoning_effort", "user"];
const SERVICE_TIER: &str = "service_tier";

#[derive(Debug, Clone, Default)]
pub struct SailChatConfig {
    pub settings: SailSettings,
}

pub struct MapRequest<'request> {
    pub model: &'request str,
    pub non_default_params: &'request ParamsObject,
    pub optional_params: &'request mut ParamsObject,
    pub catalog: &'request CatalogCapabilities,
    pub drop_params: bool,
}

// the base config is OpenAIGPTConfig, so ConfigFamily::Other keeps its list and not OpenAIConfig's reasoning_effort
#[must_use]
pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let base = OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    let mut params: Vec<&'static str> = base
        .supported_params(model, catalog)
        .into_iter()
        .filter(|name| !REJECTED_BY_SAIL.contains(name))
        .collect();
    for name in ACCEPTED_BY_SAIL {
        if !params.contains(&name) {
            params.push(name);
        }
    }
    params
}

impl SailChatConfig {
    pub fn map_openai_params(&self, request: MapRequest<'_>) -> Result<(), SailChatError> {
        let MapRequest {
            model,
            non_default_params,
            optional_params,
            catalog,
            drop_params,
        } = request;
        // LiteLLM checks the tier and throws the window away, so a bad one fails the mapping and nothing else
        let tier = non_default_params
            .get(SERVICE_TIER)
            // any serde_json value is representable as Json, so the value fallback never runs
            .map(|value| serde_json::from_value(value.clone()).unwrap_or(Json::Null));
        let _window = window_for_tier(tier.as_ref(), model, drop_params, &self.settings)?;
        copy_supported(
            non_default_params,
            optional_params,
            &supported_params(model, catalog),
        );
        Ok(())
    }

    pub fn transform_request(
        &self,
        content: &ChatContent<'_>,
        drop_params: bool,
    ) -> Result<Object, SailChatError> {
        let parent = ChatContent {
            provider: Some(PROVIDER),
            ..*content
        };
        let body = parent.transform(Mode::Sync)?.to_body()?;
        Ok(chat_request_for_sail(
            &body,
            content.model,
            drop_params,
            &self.settings,
        )?)
    }
}
